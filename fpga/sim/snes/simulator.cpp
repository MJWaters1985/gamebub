#include <iostream>

#include "simulator.hpp"

namespace {

constexpr int SDRAM_COLUMN_BITS = 9;
constexpr int SDRAM_ROW_BITS = 13;

size_t sdram_index(int bank, int row, int column)
{
    return (size_t(bank) << (SDRAM_ROW_BITS + SDRAM_COLUMN_BITS)) |
        (size_t(row) << SDRAM_COLUMN_BITS) | size_t(column);
}

/// SDRAM word holding SNES ROM byte `address` (and `address ^ 1`), as the
/// controller maps port 0: bank {0, A23}, row A13:1, column A22:14.
size_t rom_index(uint32_t address)
{
    int bank = (address >> 23) & 1;
    int row = (address >> 1) & 0x1FFF;
    int column = (address >> 14) & 0x1FF;
    return sdram_index(bank, row, column);
}

} // namespace

Simulator::Simulator(std::filesystem::path rom_path, std::filesystem::path bios_path)
    : framebuffer(width(), height()), cartridge(rom_path)
{
    (void)bios_path;
    std::cerr << "Cartridge: " << cartridge.description() << "\n";

    top = new Vsnes_top;
    sdram.resize(size_t(4) << (SDRAM_ROW_BITS + SDRAM_COLUMN_BITS));
    sram.resize(256 * 1024, 0xFFFF);

    // Load the ROM straight into the SDRAM model (the download port is only
    // needed on hardware).
    for (size_t i = 0; i + 1 < cartridge.rom.size(); i += 2) {
        sdram[rom_index(i)] = cartridge.rom[i] | (cartridge.rom[i + 1] << 8);
    }
}

Simulator::~Simulator()
{
    top->final();
    delete top;
}

void Simulator::reset()
{
    top->rom_type = cartridge.rom_type;
    top->rom_mask = cartridge.rom_mask();
    top->ram_mask = cartridge.ram_mask();
    top->ram_size = cartridge.ram_size;
    top->pal = cartridge.pal;
    top->sdram_init = 0;
    top->dl_en = 0;
    top->dl_wr = 0;
    top->enable = 1;

    // Hold reset while the SDRAM controller runs its initialization.
    top->reset = 1;
    simulate_cycles(2000);
    top->reset = 0;
}

void Simulator::set_joypad_state(JoypadState state)
{
    // {start, select, r, l, y, x, b, a, up, down, left, right}
    top->joy1 =
        (state.start << 11) | (state.select << 10) | (state.r << 9) | (state.l << 8) |
        (state.y << 7) | (state.x << 6) | (state.b << 5) | (state.a << 4) |
        (state.up << 3) | (state.down << 2) | (state.left << 1) | (state.right << 0);
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

void Simulator::stepSram()
{
    uint16_t& word = sram[top->sram_addr];
    if (!top->sram_we_n) {
        uint16_t data = top->sram_dq_out;
        if (!(top->sram_mask_n & 1)) word = (word & 0xFF00) | (data & 0x00FF);
        if (!(top->sram_mask_n & 2)) word = (word & 0x00FF) | (data & 0xFF00);
    }
    top->sram_dq_in = word;
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
    stepSram();
    top->eval();

    top->clk_mem = 0;
    top->eval();
    stepSram();
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
    simulate_cycles(1364 * 262);
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
