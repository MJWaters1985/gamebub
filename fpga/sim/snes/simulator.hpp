#pragma once

#include <deque>
#include <filesystem>

#include "framebuffer.hpp"
#include "input.hpp"
#include "Vsnes_top.h"
#include "cartridge.hpp"

class Simulator {
public:
    Simulator(std::filesystem::path rom_path, std::filesystem::path bios_path);
    ~Simulator();

    void set_joypad_state(JoypadState state);
    void simulate_cycles(uint64_t cycles);
    void simulate_frame();
    void reset();
    Framebuffer& getFramebuffer() { return framebuffer; }
    std::vector<int16_t>& getAudioSampleBuffer();

    static int width() { return 256; }
    static int height() { return 224; }
    static int clockHz() { return 21477270; }
    static int audioSampleHz() { return 32000; }
    static float videoFramerate() { return 60.0988f; }

private:
    /// One clk_mem edge pair, with clk_sys rising on phase 0 and falling on phase 2.
    void stepMem(int phase);
    void stepSdram();
    void stepSram();
    void stepFramebuffer();
    void stepAudio();

    Vsnes_top* top = nullptr;

    uint64_t cycles = 0;
    uint64_t memEdges = 0;
    Framebuffer framebuffer;
    std::vector<int16_t> audioSampleBuffer;
    uint64_t audioAccumulator = 0;

    Cartridge cartridge;

    // SDRAM model: 4 banks x 8192 rows x 512 columns of 16-bit words.
    std::vector<uint16_t> sdram;
    uint16_t openRow[4] = {};
    struct PendingRead {
        uint64_t edge;
        uint16_t data;
    };
    std::deque<PendingRead> pendingReads;

    // External SRAM: 256K 16-bit words.
    std::vector<uint16_t> sram;
};
