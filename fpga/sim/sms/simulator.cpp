#include <fstream>
#include <iostream>
#include <iterator>

#include "simulator.hpp"

namespace {

constexpr int SDRAM_COLUMN_BITS = 9;
constexpr int SDRAM_ROW_BITS = 13;

size_t sdram_index(int bank, int row, int column)
{
    return (size_t(bank) << (SDRAM_ROW_BITS + SDRAM_COLUMN_BITS)) |
        (size_t(row) << SDRAM_COLUMN_BITS) | size_t(column);
}

} // namespace

Simulator::Simulator(std::filesystem::path rom_path, std::filesystem::path bios_path)
    : framebuffer(width(), height())
{
    (void)bios_path;
    std::ifstream file(rom_path, std::ios::binary);
    rom.assign(std::istreambuf_iterator<char>(file), {});
    std::cerr << "ROM: " << rom.size() << " bytes\n";

    top = new Vsms_top;
    sdram.resize(size_t(4) << (SDRAM_ROW_BITS + SDRAM_COLUMN_BITS), 0xFFFF);
}

Simulator::~Simulator()
{
    top->final();
    delete top;
}

void Simulator::reset()
{
    top->pal = 0;
    top->japan = 0;
    top->enable = 1;
    top->dl_en = 0;
    top->dl_wr = 0;
    top->save_req = 0;
    top->joy1 = 0;

    top->reset = 1;
    top->sdram_init = 1;
    simulate_cycles(100);
    top->sdram_init = 0;
    simulate_cycles(2000);
    top->reset = 0;

    // Load the game 32 bits at a time, as the host does.
    top->dl_en = 1;
    simulate_cycles(16);
    for (size_t i = 0; i < rom.size(); i += 4) {
        uint32_t word = 0;
        for (size_t b = 0; b < 4 && i + b < rom.size(); b++) word |= uint32_t(rom[i + b]) << (8 * b);
        top->dl_addr = i;
        top->dl_data = word;
        top->dl_wr = 1;
        simulate_cycles(1);
        top->dl_wr = 0;
        simulate_cycles(1);
        while (top->dl_busy) simulate_cycles(1);
    }
    // The core clears its RAMs while dl_en is high (32K cycles).
    simulate_cycles(40000);
    top->dl_en = 0;
    simulate_cycles(16);
}

void Simulator::set_joypad_state(JoypadState state)
{
    // {start, 2, 1, up, down, left, right}; buttons 1 and 2 are B and A.
    top->joy1 = (state.start << 6) | (state.a << 5) | (state.b << 4) | (state.up << 3) |
        (state.down << 2) | (state.left << 1) | (state.right << 0);
}

void Simulator::stepSdram()
{
    // Called after a clk_mem rising edge: the pins show the command registered
    // on this edge, which the SDRAM (clocked half a cycle later) acts on.
    if (!pendingReads.empty() && pendingReads.front().edge == memEdges) {
        pendingReads.pop_front();
    }

    if (top->sdram_cs_n) {
        return;
    }
    int bank = top->sdram_ba;
    int command = (top->sdram_ras_n << 2) | (top->sdram_cas_n << 1) | top->sdram_we_n;
    switch (command) {
        case 0b011: // ACTIVE
            openRow[bank] = top->sdram_a;
            break;
        case 0b101: { // READ, CAS latency 2: sampled by the controller 3 edges later
            int column = top->sdram_a & 0x1FF;
            pendingReads.push_back({memEdges + 3, sdram[sdram_index(bank, openRow[bank], column)]});
            break;
        }
        case 0b100: { // WRITE
            int column = top->sdram_a & 0x1FF;
            uint16_t& word = sdram[sdram_index(bank, openRow[bank], column)];
            uint16_t data = top->sdram_dq_out;
            if (!(top->sdram_dqm & 1)) word = (word & 0xFF00) | (data & 0x00FF);
            if (!(top->sdram_dqm & 2)) word = (word & 0x00FF) | (data & 0xFF00);
            break;
        }
        default:
            break;
    }
}

void Simulator::stepMem(int phase)
{
    // Data the SDRAM drives for this edge.
    if (!pendingReads.empty() && pendingReads.front().edge == memEdges + 1) {
        top->sdram_dq_in = pendingReads.front().data;
    }

    top->clk_mem = 1;
    if (phase == 0) top->clk_sys = 1;
    if (phase == 1) top->clk_sys = 0;
    top->eval();
    memEdges++;
    stepSdram();

    top->clk_mem = 0;
    top->eval();
}

void Simulator::simulate_cycles(uint64_t num_cycles)
{
    for (uint64_t i = 0; i < num_cycles; i++) {
        for (int phase = 0; phase < 2; phase++) {
            stepMem(phase);
        }
        stepFramebuffer();
        stepAudio();
        cycles++;
    }
}

void Simulator::stepFramebuffer()
{
    framebuffer.update(top->video_hblank, top->video_vblank);
    if (top->video_valid && !top->video_hblank && !top->video_vblank) {
        auto c5 = [](int c) { return (c << 1) | (c >> 3); };
        framebuffer.pushBGR(c5(top->video_r) | (c5(top->video_g) << 5) | (c5(top->video_b) << 10));
    }
}

void Simulator::simulate_frame()
{
    // 342 pixels x 262 lines, 10 master clock cycles per pixel
    simulate_cycles(3420 * 262);
}

void Simulator::stepAudio()
{
    audioAccumulator += audioSampleHz();
    if (audioAccumulator >= uint64_t(clockHz())) {
        audioAccumulator -= clockHz();
        audioSampleBuffer.push_back(int16_t(top->audio_l));
        audioSampleBuffer.push_back(int16_t(top->audio_r));
    }
}

std::vector<int16_t>& Simulator::getAudioSampleBuffer()
{
    return audioSampleBuffer;
}
