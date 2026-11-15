// Game Bub wrapper for the MiSTer SMS / Game Gear core (sms_core.vhd, mister/):
// game loading into SDRAM, save RAM access for the host, and video/audio
// conditioning. Based on the core's MiSTer top level (SMS.sv).
//
// The ROM is in SDRAM from address 0 (512-byte copier headers are skipped by
// the host). The Master System build shows 256x192 (taller modes are cropped);
// the Game Gear build shows the Game Gear's 160x144 window.

module sms_top #(
	parameter GG = 0                // Game Gear build
) (
	input             clk_sys,      // 53.69 MHz
	input             clk_mem,      // 2x clk_sys, for the SDRAM controller
	input             reset,
	input             enable,       // 0 pauses the console
	input             sdram_init,   // hold high until the SDRAM has powered up
	input             pal,
	input             japan,

	// Game loading: the host writes the file 32 bits at a time, in order.
	input             dl_en,
	input             dl_wr,        // one pulse per 32-bit word
	input      [24:0] dl_addr,      // byte address of the word in the file
	input      [31:0] dl_data,      // file bytes, first byte in bits 7:0
	output            dl_busy,

	// Save RAM, for the host (one cycle of read latency)
	input             save_req,
	input             save_write,
	input      [14:0] save_addr,
	input       [7:0] save_wdata,
	output      [7:0] save_rdata,
	output reg        save_ack,
	output            save_used,    // the game has written its save RAM

	// Controller: {start, 2, 1, up, down, left, right}
	input       [6:0] joy1,

	output reg  [3:0] video_r,
	output reg  [3:0] video_g,
	output reg  [3:0] video_b,
	output reg        video_valid,
	output reg        video_hblank,
	output reg        video_vblank,

	output     [15:0] audio_l,      // signed
	output     [15:0] audio_r,

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

/////////////////////////////  Loading  /////////////////////////////////

// Each 32-bit host word is four bytes: each is written to SDRAM (toggling
// request/acknowledge) and shown to the core's mapper detection. SDRAM words
// hold two bytes: even addresses in bits 7:0, odd addresses in 15:8.
reg         rom_wr = 0;
wire        rom_wrack;
reg  [24:0] wr_addr;
reg  [31:0] wr_data;
reg   [2:0] wr_left = 0;
reg         core_wr;
assign dl_busy = wr_left != 0;

always @(posedge clk_sys) begin
	core_wr <= 0;
	if (dl_wr) begin
		wr_addr <= dl_addr;
		wr_data <= dl_data;
		wr_left <= 4;
		rom_wr  <= ~rom_wr;
		core_wr <= 1;
	end else if (wr_left != 0 && rom_wr == rom_wrack) begin
		wr_left <= wr_left - 1'd1;
		if (wr_left != 1) begin
			wr_addr <= wr_addr + 1'd1;
			wr_data <= wr_data >> 8;
			rom_wr  <= ~rom_wr;
			core_wr <= 1;
		end
	end
end

// ROM size mask, for mirroring smaller ROMs (SMS.sv's cart_mask)
reg [23:0] cart_mask;
always @(posedge clk_sys) begin
	if (dl_wr) begin
		if (dl_addr == 0) cart_mask <= 24'h3;
		else cart_mask <= cart_mask | dl_addr[23:0] | 24'h3;
	end
end

////////////////////////////////  Console  ////////////////////////////////

wire        rom_rd;
wire [23:0] rom_a;
wire  [7:0] rom_do;
wire        ce_cpu;
wire [11:0] color;
wire        ce_pix, hblank, vblank;

sms_core core (
	.clk      (clk_sys),
	.reset    (reset),
	.enable   (enable),
	.gg       (GG[0]),
	.pal      (pal),
	.region   (japan),

	.dl_en    (dl_en),
	.dl_wr    (core_wr),
	.dl_addr  (wr_addr),
	.dl_data  (wr_data[7:0]),

	.rom_rd   (rom_rd),
	.rom_a    (rom_a),
	.rom_do   (rom_do),
	.ce_cpu_o (ce_cpu),

	.up       (joy1[3]),
	.down     (joy1[2]),
	.left     (joy1[1]),
	.right    (joy1[0]),
	.b1       (joy1[4]),
	.b2       (joy1[5]),
	.start    (joy1[6]),

	.nv_addr  (save_addr),
	.nv_we    (save_req & save_write & ~save_ack),
	.nv_din   (save_wdata),
	.nv_dout  (save_rdata),
	.nv_used  (save_used),

	.color    (color),
	.ce_pix_o (ce_pix),
	.hblank   (hblank),
	.vblank   (vblank),

	.audio_l  (audio_l),
	.audio_r  (audio_r)
);

// Save RAM: a write takes one cycle, a read two (address, then data).
reg save_pending;
always @(posedge clk_sys) begin
	save_ack <= 0;
	if (!save_req || save_ack) save_pending <= 0;
	else if (save_write) save_ack <= 1;
	else if (!save_pending) save_pending <= 1;
	else save_ack <= 1;
end

/////////////////////////////////  SDRAM  /////////////////////////////////

// The Genesis core's SDRAM controller (genesis/mister/sdram.sv, at twice the
// master clock, request/acknowledge toggles), which works on this board,
// rather than the SMS core's single-clock one. A ROM read starts on the
// rising edge of rom_rd and its byte is held in rom_do until the next one.
reg         rom_req = 0;
wire        rom_ack;
reg  [24:1] rd_addr;
reg         rd_odd;
reg  [15:0] rd_word;
reg         rd_pending = 0;
wire [15:0] rom_dout;
reg         old_rom_rd;
assign rom_do = rd_odd ? rd_word[15:8] : rd_word[7:0];

always @(posedge clk_sys) begin
	old_rom_rd <= rom_rd;
	if (rom_rd & ~old_rom_rd) begin
		rd_addr <= {1'b0, rom_a[23:1] & cart_mask[23:1]};
		rd_odd  <= rom_a[0];
		rom_req <= ~rom_req;
		rd_pending <= 1;
	end else if (rd_pending && rom_req == rom_ack) begin
		rd_word <= rom_dout;
		rd_pending <= 0;
	end
end

sdram ram (
	.init        (sdram_init),
	.clk         (clk_mem),

	.addr0       (wr_addr[24:1]),
	.din0        ({wr_data[7:0], wr_data[7:0]}),
	.dout0       (),
	.wrl0        (~wr_addr[0]),
	.wrh0        (wr_addr[0]),
	.req0        (rom_wr),
	.ack0        (rom_wrack),

	.addr1       (rd_addr),
	.din1        (16'd0),
	.dout1       (rom_dout),
	.wrl1        (1'b0),
	.wrh1        (1'b0),
	.req1        (rom_req),
	.ack1        (rom_ack),

	.addr2       (24'd0),
	.din2        (16'd0),
	.dout2       (),
	.wrl2        (1'b0),
	.wrh2        (1'b0),
	.req2        (1'b0),
	.ack2        (),

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

/////////////////////////////////  Video  /////////////////////////////////

// One pixel per ce_pix outside blanking; the platform starts a new row on
// each rising edge of video_hblank, so it stays high through blanked lines.
// Lines past the framebuffer's height (224/240-line modes) aren't shown.
localparam HEIGHT = GG ? 144 : 192;

initial video_hblank = 1;

reg [8:0] line;
reg       old_hblank, old_vblank;
wire      shown = line < HEIGHT;

always @(posedge clk_sys) begin
	video_valid <= 0;
	if (ce_pix) begin
		old_hblank <= hblank;
		old_vblank <= vblank;
		if (vblank & ~old_vblank) line <= 0;
		else if (hblank & ~old_hblank & ~vblank) line <= line + 1'd1;

		video_valid <= ~hblank & ~vblank & shown;
		{video_b, video_g, video_r} <= color;
		video_hblank <= hblank | vblank | ~shown;
		video_vblank <= vblank;
	end
end

endmodule
