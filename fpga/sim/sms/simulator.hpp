#pragma once

#include <deque>
#include <filesystem>
#include <vector>

#include "framebuffer.hpp"
#include "input.hpp"
#include "Vsms_top.h"

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
    static int height() { return 192; }
    static int clockHz() { return 53693175; }
    static int audioSampleHz() { return 48000; }
    static float videoFramerate() { return 59.922f; }

private:
    /// One clk_mem edge pair (clk_mem is 2x clk_sys), with clk_sys rising on phase 0 and falling on phase 1.
    void stepMem(int phase);
    void stepSdram();
    void stepFramebuffer();
    void stepAudio();

    Vsms_top* top = nullptr;

    std::vector<uint8_t> rom;
    uint64_t cycles = 0;
    uint64_t memEdges = 0;
    Framebuffer framebuffer;
    std::vector<int16_t> audioSampleBuffer;
    uint64_t audioAccumulator = 0;

    // SDRAM model: 4 banks x 8192 rows x 512 columns of 16-bit words.
    std::vector<uint16_t> sdram;
    uint16_t openRow[4] = {};
    struct PendingRead {
        uint64_t edge;
        uint16_t data;
    };
    std::deque<PendingRead> pendingReads;
};
