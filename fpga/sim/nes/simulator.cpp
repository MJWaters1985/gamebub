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
    if (rom.size() >= 16) {
        int mapper = (rom[6] >> 4) | (rom[7] & 0xF0);
        std::cerr << "ROM: " << rom.size() << " bytes, mapper " << mapper << ", PRG "
                  << rom[4] * 16 << " KiB, CHR " << rom[5] * 8 << " KiB\n";
    }

    top = new Vnes_top;
    sdram.resize(size_t(4) << (SDRAM_ROW_BITS + SDRAM_COLUMN_BITS), 0xFFFF);
}

Simulator::~Simulator()
{
    top->final();
    delete top;
}

void Simulator::reset()
{
    top->sys_type = 0;
    top->enable = 1;
    top->dl_en = 0;
    top->dl_wr = 0;
    top->dl_type = 0;
    top->save_req = 0;

    // Let the SDRAM controller initialize, then load the game through the
    // download port, one byte every 4 cycles like the hardware wrapper.
    top->reset = 1;
    top->sdram_init = 1;
    simulate_cycles(100);
    top->sdram_init = 0;
    simulate_cycles(2000);
    top->reset = 0;

    top->dl_en = 1;
    simulate_cycles(16);
    for (uint8_t byte : rom) {
        top->dl_data = byte;
        top->dl_wr = 1;
        simulate_cycles(1);
        top->dl_wr = 0;
        simulate_cycles(3);
    }
    top->dl_en = 0;
    simulate_cycles(16);
    while (top->loader_busy) {
        simulate_cycles(64);
    }
    if (top->loader_fail) {
        std::cerr << "Loader: unsupported file\n";
    }
}

void Simulator::set_joypad_state(JoypadState state)
{
    // {right, left, down, up, start, select, b, a}
    top->joy1 = (state.right << 7) | (state.left << 6) | (state.down << 5) | (state.up << 4) |
        (state.start << 3) | (state.select << 2) | (state.b << 1) | (state.a << 0);
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
    if (phase == 2) top->clk_sys = 0;
    top->eval();
    memEdges++;
    stepSdram();

    top->clk_mem = 0;
    top->eval();
}

void Simulator::simulate_cycles(uint64_t num_cycles)
{
    for (uint64_t i = 0; i < num_cycles; i++) {
        for (int phase = 0; phase < 4; phase++) {
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
        framebuffer.pushBGR(top->video_r | (top->video_g << 5) | (top->video_b << 10));
    }
}

void Simulator::simulate_frame()
{
    // 341 dots x 262 lines, 4 master clock cycles per dot
    simulate_cycles(341 * 262 * 4);
}

void Simulator::stepAudio()
{
    audioAccumulator += audioSampleHz();
    if (audioAccumulator >= uint64_t(clockHz())) {
        audioAccumulator -= clockHz();
        audioSampleBuffer.push_back(int16_t(top->audio));
        audioSampleBuffer.push_back(int16_t(top->audio));
    }
}

std::vector<int16_t>& Simulator::getAudioSampleBuffer()
{
    return audioSampleBuffer;
}
