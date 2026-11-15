// Game Bub wrapper for the MiSTer NES core (mister/): game loading, SDRAM,
// save RAM access for the host, controller and video/audio conditioning.
// Based on the core's MiSTer top level (NES.sv) and agg23's Analogue Pocket
// port (openfpga-NES, rtl/nes_top.sv).
//
// Memory (all in SDRAM, byte addresses as the core uses them): PRG ROM from
// 0x000000, CHR ROM/RAM from 0x200000, RAM (cleared on load) at 0x300000-
// 0x3FFFFF with the cartridge's save RAM at 0x3C0000. The host reads and
// writes save RAM through the save_* port; mappers with EEPROM saves
// (bram_override) use a block RAM instead.

module nes_top #(
	parameter CLEAR_RAM = 1,  // clear RAM after loading (slow; the simulator skips it)
	parameter TRACE_FROM = 0  // simulation: trace CPU writes from this frame (0: off)
) (
	input             clk_sys,      // 21.477 MHz master clock
	input             clk_mem,      // 4x clk_sys, for the SDRAM controller
	input             reset,
	input             enable,       // 0 pauses the console
	input             sdram_init,   // hold high until the SDRAM has powered up
	input       [1:0] sys_type,     // 0 = NTSC, 1 = PAL, 2 = Dendy
	input       [1:0] max_diskside, // FDS: number of disk sides - 1

	// Game loading: a byte stream, as the file appears on disk.
	input             dl_en,        // high for the whole file
	input       [1:0] dl_type,      // 0 = .nes, 1 = .fds, 2 = FDS BIOS
	input             dl_wr,        // one pulse per byte, at least 4 cycles apart
	input       [7:0] dl_data,
	output            loader_busy,  // still loading or clearing RAM after dl_en falls
	output            loader_fail,  // unsupported file

	// Save RAM, for the host
	input             save_req,
	input             save_write,
	input      [17:0] save_addr,
	input       [7:0] save_wdata,
	output reg  [7:0] save_rdata,
	output reg        save_ack,

	// Controller: {right, left, down, up, start, select, b, a}
	input       [7:0] joy1,

	// Video: 256x224
	output reg  [4:0] video_r,
	output reg  [4:0] video_g,
	output reg  [4:0] video_b,
	output reg        video_valid,
	output reg        video_hblank,
	output reg        video_vblank,

	output     [15:0] audio,        // signed

	// SDRAM
	input      [15:0] sdram_dq_in,
	output     [15:0] sdram_dq_out,
	output            sdram_dq_dir,
	output     [12:0] sdram_a,
	output      [1:0] sdram_dqm,
	output      [1:0] sdram_ba,
	output            sdram_cs_n,
	output            sdram_we_n,
	output            sdram_ras_n,
	output            sdram_cas_n,
	output            sdram_cke
);

wire clk = clk_sys;

///////////////////////////////  Loading  ////////////////////////////////

wire downloading = dl_en;
wire type_nes  = dl_type == 2'd0;
wire type_fds  = dl_type == 2'd1;
wire type_bios = dl_type == 2'd2;

wire [24:0] loader_addr;
wire  [7:0] loader_write_data;
wire        loader_write;
wire [63:0] loader_flags;
reg  [63:0] mapper_flags;
wire  [9:0] prg_mask, chr_mask;
wire        loader_done;
wire        rom_loaded;
reg         loader_reset;

// After a BIOS is loaded, the loader copies it in front of FDS games.
wire  [7:0] bios_data;
wire  [7:0] loader_input = (loader_busy && !downloading) ? bios_data : dl_data;

GameLoader loader (
	.clk          (clk),
	.clearval     (CLEAR_RAM[0]),   // clear save RAM (then the host loads the save, if any)
	.cleardata    (8'h00),
	.reset        (loader_reset),
	.downloading  (downloading),
	.filetype     ({4'b0000, 1'b0, type_fds, type_nes, type_bios}),
	.is_bios      (1'b0),
	.indata       (loader_input),
	.indata_clk   (dl_wr),
	.mem_addr     (loader_addr),
	.mem_data     (loader_write_data),
	.mem_write    (loader_write),
	.mapper_flags (loader_flags),
	.prg_mask     (prg_mask),
	.chr_mask     (chr_mask),
	.busy         (loader_busy),
	.done         (loader_done),
	.error        (loader_fail),
	.rom_loaded   (rom_loaded)
);

always @(posedge clk) if (loader_done) mapper_flags <= loader_flags;

wire bios_write = loader_write && downloading && type_bios;

spram #(.addr_width(13)) fds_bios (
	.clock   (clk),
	.address (loader_addr[12:0]),
	.wren    (bios_write),
	.data    (loader_write_data),
	.q       (bios_data)
);

// Hold the console in reset while loading, and for a while after.
reg [7:0] download_reset_cnt;
wire download_reset = download_reset_cnt != 0;
always @(posedge clk) begin
	if (downloading) download_reset_cnt <= 8'hFF;
	else if (!loader_busy && download_reset_cnt) download_reset_cnt <= download_reset_cnt - 1'd1;
end

// Hold the console in reset until a game has been loaded.
reg init_reset_n = 0;
always @(posedge clk) if (downloading & (type_nes | type_fds)) init_reset_n <= 1;

always @(posedge clk) begin
	reg old_downld;
	old_downld   <= downloading;
	loader_reset <= !download_reset || (~old_downld && downloading);
end

wire hold_reset;
wire reset_nes = reset || ~init_reset_n || download_reset || loader_fail || hold_reset;

// Loader writes: hold the SDRAM request until the controller takes it (its
// busy flag rises), so that a byte can come every 4 cycles. (The PPU doesn't
// use the SDRAM while loading.)
wire [1:0] nes_ce;
reg        loader_write_mem = 0;
reg  [7:0] loader_write_data_mem;
reg [24:0] loader_addr_mem;
wire       ch0_busy;
reg        ch0_busy_d;
always @(posedge clk) begin
	ch0_busy_d <= ch0_busy;
	if (ch0_busy & ~ch0_busy_d) loader_write_mem <= 1'b0;
	if (loader_write) begin
		loader_write_mem <= 1'b1;
		loader_addr_mem <= loader_addr;
		loader_write_data_mem <= loader_write_data;
	end
end

////////////////////////////////  Console  ////////////////////////////////

reg  pausecore;
wire corepaused;
reg  [1:0] videopause_ce;
always @(posedge clk) begin
	pausecore <= ~enable & ~reset_nes;
	if (!corepaused) videopause_ce <= nes_ce + 1'd1;
	else videopause_ce <= videopause_ce + 1'd1;
end

wire  [5:0] color;
wire  [2:0] emphasis;
wire  [8:0] cycle, scanline;
wire        nes_hblank, nes_hsync, nes_vsync, nes_vblank;
wire [15:0] sample;

wire [24:0] cpu_addr;
wire [21:0] ppu_addr;
wire        cpu_read, cpu_write, ppu_read, ppu_write;
wire  [7:0] cpu_dout, cpu_din, ppu_dout, ppu_din;
wire        refresh;

wire [17:0] bram_addr;
wire  [7:0] bram_din, bram_dout;
wire        bram_write, bram_en;

wire  [2:0] joypad_out;
wire  [1:0] joypad_clock;
reg  [23:0] joypad_bits;
reg   [1:0] last_joypad_clock;
wire  [4:0] joypad1_data = {4'b0000, joypad_bits[0]};
wire  [4:0] joypad2_data = 5'b00000;

wire [1:0] diskside;

NES nes (
	.clk             (clk),
	.reset_nes       (reset_nes),
	.ppu_rst_behavior(1'b0),
	.cold_reset      (downloading & (type_fds | type_nes)),
	.pausecore       (pausecore),
	.corepaused      (corepaused),
	.debug_dots      (1'b0),
	.sys_type        (sys_type),
	.vs_dip_switches (8'h00),
	.nes_div         (nes_ce),
	.mapper_flags    (downloading ? 64'd0 : mapper_flags),
	.gg              (1'b0),
	.gg_code         (129'd0),
	.gg_reset        (1'b0),
	.gg_avail        (),
	.sample          (sample),
	.audio_channels  (5'b11111),
	.int_audio       (1'b1),
	.ext_audio       (1'b1),
	.apu_ce          (),
	.ex_sprites      (1'b0),
	.color           (color),
	.hsync           (nes_hsync),
	.hblank          (nes_hblank),
	.vsync           (nes_vsync),
	.vblank          (nes_vblank),
	.composite_a     (),
	.composite_b     (),
	.emphasis        (emphasis),
	.cycle           (cycle),
	.scanline        (scanline),
	.mask            (2'b00),
	.dejitter_timing (1'b0),
	.joypad_out      (joypad_out),
	.joypad_clock    (joypad_clock),
	.joypad1_data    (joypad1_data),
	.joypad2_data    (joypad2_data),
	.diskside        (diskside),
	.fds_busy        (1'b0),
	.fds_eject       (1'b0),
	.fds_auto_eject  (1'b1),
	.max_diskside    (max_diskside),
	.fds_fast        (1'b1),
	.cpumem_addr     (cpu_addr),
	.cpumem_read     (cpu_read),
	.cpumem_write    (cpu_write),
	.cpumem_dout     (cpu_dout),
	.cpumem_din      (cpu_din),
	.ppumem_addr     (ppu_addr),
	.ppumem_read     (ppu_read),
	.ppumem_write    (ppu_write),
	.ppumem_dout     (ppu_dout),
	.ppumem_din      (ppu_din),
	.refresh         (refresh),
	.prg_mask        (prg_mask),
	.chr_mask        (chr_mask),
	.bram_addr       (bram_addr),
	.bram_din        (bram_din),
	.bram_dout       (bram_dout),
	.bram_write      (bram_write),
	.bram_override   (bram_en),
	.save_written    (),
	.mapper_has_flashsaves(),
	.mapper_has_savestate(),
	.increaseSSHeaderCount(1'b1),
	.save_state      (1'b0),
	.load_state      (1'b0),
	.savestate_number(2'd0),
	.sleep_savestate (),
	.state_loaded    (),
	.Savestate_SDRAMAddr(),
	.Savestate_SDRAMRdEn(),
	.Savestate_SDRAMWrEn(),
	.Savestate_SDRAMWriteData(),
	.Savestate_SDRAMReadData(8'h00),
	.SaveStateExt_Din(),
	.SaveStateExt_Adr(),
	.SaveStateExt_wren(),
	.SaveStateExt_rst(),
	.SaveStateExt_Dout(64'd0),
	.SaveStateExt_load(),
	.SAVE_out_Din    (),
	.SAVE_out_Dout   (64'd0),
	.SAVE_out_Adr    (),
	.SAVE_out_rnw    (),
	.SAVE_out_ena    (),
	.SAVE_out_be     (),
	.SAVE_out_done   (1'b0)
);

`ifdef VERILATOR
// Simulation-only trace of CPU writes to the PPU, mapper and PRG RAM, from frame TRACE_FROM
// (0: off).
reg [15:0] dbg_frame = 0;
reg        dbg_vblank;
always @(posedge clk) begin
	dbg_vblank <= nes_vblank;
	if (nes_vblank & ~dbg_vblank) dbg_frame <= dbg_frame + 1'd1;
	if (TRACE_FROM != 0 && dbg_frame >= TRACE_FROM && nes.cpu_ce && nes.mw_int &&
			(nes.addr[15] || nes.addr[15:13] == 3'b001 || nes.addr[15:13] == 3'b011))
		$display("f%0d sl%0d c%0d %04x <= %02x", dbg_frame, scanline, cycle, nes.addr, nes.dbus);
	if (TRACE_FROM != 0 && dbg_frame == TRACE_FROM + 40 && nes.cpu_ce && nes.mr_int)
		$display("f%0d sl%0d c%0d %04x -> %02x", dbg_frame, scanline, cycle, nes.addr, nes.internal_data_bus);
end
`endif

// Controller 1 shift register (the core strobes and clocks it)
always @(posedge clk) begin
	if (reset_nes) begin
		joypad_bits <= 0;
		last_joypad_clock <= 0;
	end else begin
		if (joypad_out[0]) joypad_bits <= {16'hFFFF, joy1};
		if (!joypad_clock[0] && last_joypad_clock[0]) joypad_bits <= {1'b0, joypad_bits[23:1]};
		last_joypad_clock <= joypad_clock;
	end
end

/////////////////////////////////  SDRAM  /////////////////////////////////

// The core only asks for refreshes while paused; its other accesses refresh
// as a side effect. Add some while it's held in reset or loading.
reg [5:0] refresh_div;
always @(posedge clk) refresh_div <= refresh_div + 1'd1;
wire idle_refresh = (reset_nes | loader_busy) & (refresh_div == 0);

// Save RAM requests from the host
wire        save_busy;
wire  [7:0] save_dout;
reg         ch2_rd = 0, ch2_wr = 0;
reg   [1:0] save_state = 0;

sdram_nes sdram (
	.clk         (clk_mem),
	.init        (sdram_init),

	.ch0_addr    ((downloading | loader_busy) ? loader_addr_mem : {3'b0, ppu_addr}),
	.ch0_wr      (loader_write_mem | ppu_write),
	.ch0_din     ((downloading | loader_busy) ? loader_write_data_mem : ppu_dout),
	.ch0_rd      (~(downloading | loader_busy) & ppu_read),
	.ch0_dout    (ppu_din),
	.ch0_busy    (ch0_busy),

	.ch1_addr    (cpu_addr),
	.ch1_wr      (cpu_write),
	.ch1_din     (cpu_dout),
	.ch1_rd      (cpu_read),
	.ch1_dout    (cpu_din),
	.ch1_busy    (),

	.ch2_addr    ({7'b0001111, save_addr}),
	.ch2_wr      (ch2_wr),
	.ch2_din     (save_wdata),
	.ch2_rd      (ch2_rd),
	.ch2_dout    (save_dout),
	.ch2_busy    (save_busy),

	.refresh     (refresh | idle_refresh),
	.ss_in       (16'd0),
	.ss_load     (1'b0),
	.ss_out      (),

	.SDRAM_DQ_IN (sdram_dq_in),
	.SDRAM_DQ_OUT(sdram_dq_out),
	.SDRAM_DQ_DIR(sdram_dq_dir),
	.SDRAM_A     (sdram_a),
	.SDRAM_DQML  (sdram_dqm[0]),
	.SDRAM_DQMH  (sdram_dqm[1]),
	.SDRAM_BA    (sdram_ba),
	.SDRAM_nCS   (sdram_cs_n),
	.SDRAM_nWE   (sdram_we_n),
	.SDRAM_nRAS  (sdram_ras_n),
	.SDRAM_nCAS  (sdram_cas_n),
	.SDRAM_CKE   (sdram_cke)
);

// EEPROM saves (mappers 16 and 159)
wire [7:0] eeprom_dout;
dpram #(" ", 11) eeprom (
	.clock_a   (clk_mem),
	.address_a (bram_addr[10:0]),
	.data_a    (bram_dout),
	.wren_a    (bram_write),
	.q_a       (bram_din),

	.clock_b   (clk),
	.address_b (save_addr[10:0]),
	.data_b    (save_wdata),
	.wren_b    (save_req & save_write & ~save_ack & bram_en & save_state == 0),
	.q_b       (eeprom_dout)
);

// Host save access: hold the request until the SDRAM controller takes it
// (save_busy rises); a write is done then, a read when it finishes. Requests
// wait while a game is loading, since the loader clears the save RAM.
reg save_busy_d;
always @(posedge clk) begin
	save_ack <= 0;
	save_busy_d <= save_busy;
	case (save_state)
	0: if (save_req & ~save_ack & ~downloading & ~loader_busy) begin
		if (bram_en) begin
			save_state <= 3;
		end else begin
			ch2_rd <= ~save_write;
			ch2_wr <= save_write;
			save_state <= 1;
		end
	end
	1: if (save_busy & ~save_busy_d) begin
		ch2_rd <= 0;
		ch2_wr <= 0;
		if (ch2_wr) begin
			save_ack <= 1;
			save_state <= 0;
		end else begin
			save_state <= 2;
		end
	end
	2: if (~save_busy) begin
		save_rdata <= save_dout;
		save_ack <= 1;
		save_state <= 0;
	end
	3: begin
		save_rdata <= eeprom_dout;
		save_ack <= 1;
		save_state <= 0;
	end
	endcase
end

/////////////////////////////////  Video  /////////////////////////////////

wire [1:0] nes_ce_video = corepaused ? videopause_ce : nes_ce;
wire       ce_pix;
wire       HBlank, VBlank;
wire [7:0] R, G, B;

video video (
	.clk             (clk),
	.reset           (reset_nes),
	.cnt             (nes_ce_video),
	.color           (color),
	.count_h         (cycle),
	.count_v         (scanline),
	.hide_overscan   (2'b00),       // 256x224
	.palette         (4'd0),
	.emphasis        (emphasis),
	.reticle         (2'b00),
	.sys_type        (sys_type),
	.vs_ppu_type     (mapper_flags[48:45]),
	.pal_video       (|sys_type),
	.nes_hblank      (nes_hblank),
	.nes_hsync       (nes_hsync),
	.nes_vsync       (nes_vsync),
	.nes_vblank      (nes_vblank),
	.load_color      (1'b0),
	.load_color_data (24'd0),
	.load_color_index(6'd0),
	.hold_reset      (hold_reset),
	.ce_pix          (ce_pix),
	.HSync           (),
	.VSync           (),
	.HBlank          (HBlank),
	.VBlank          (VBlank),
	.R               (R),
	.G               (G),
	.B               (B)
);

// One pixel per ce_pix outside blanking. The platform starts a new
// framebuffer row on each rising edge of hblank, so hblank stays high through
// the blanked lines.
always @(posedge clk) begin
	video_valid <= ce_pix & ~HBlank & ~VBlank;
	video_r <= R[7:3];
	video_g <= G[7:3];
	video_b <= B[7:3];
	video_hblank <= HBlank | VBlank;
	video_vblank <= VBlank;
end

/////////////////////////////////  Audio  /////////////////////////////////

assign audio = sample ^ 16'h8000;

endmodule
