// Game Bub wrapper for the MiSTer Genesis / Mega Drive core (mister/): game
// loading into SDRAM, save RAM access for the host, controller, pausing and
// video/audio conditioning. Based on the core's MiSTer top level (Genesis.sv).
//
// The ROM is in SDRAM from address 0 (big-endian 16-bit words, as the 68000
// sees them). The save RAM is the core's 64 KB block RAM (one byte per 68000
// word in the save area), read and written by the host through the save_*
// port.

module genesis_top (
	input             clk_sys,      // 53.69 MHz master clock (MCLK)
	input             clk_mem,      // 2x clk_sys, for the SDRAM controller
	input             reset,
	input             enable,       // 0 pauses the console
	input             sdram_init,   // hold high until the SDRAM has powered up
	input       [1:0] region,       // 0 = Japan, 1 = USA, 2 = Europe (PAL)
	input             six_button,   // 6-button controller (else 3-button)

	// Game loading: the host writes the file in 32-bit words, in order.
	input             dl_en,        // high while loading (clears the console's RAM)
	input             dl_wr,        // one pulse per 32-bit word
	input      [24:0] dl_addr,      // byte address of the word in the file
	input      [31:0] dl_data,      // file bytes, first byte in bits 7:0
	output            dl_busy,      // the word is still being written
	input      [24:0] rom_size,     // file size in bytes, set before dl_en falls

	// Save RAM, for the host (one cycle of read latency)
	input             save_req,
	input             save_write,
	input      [15:0] save_addr,
	input       [7:0] save_wdata,
	output      [7:0] save_rdata,
	output reg        save_ack,

	// Controller: {Z, Y, X, mode, start, C, B, A, up, down, left, right}
	input      [11:0] joy1,

	// Video: 320x224 (256-pixel modes are centered)
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

/////////////////////////////  Pausing  /////////////////////////////////

// The console's clock stops while paused (the SDRAM controller keeps
// running, and refreshes by itself when idle). It runs for host save RAM
// accesses, since the save RAM is clocked with the console.
wire clk;
`ifdef VERILATOR
assign clk = clk_sys;
`else
reg run = 1;
always @(negedge clk_sys) run <= enable | reset | dl_en | save_req;
BUFGCE clk_gate (.I(clk_sys), .CE(run), .O(clk));
`endif

/////////////////////////////  Loading  /////////////////////////////////

// Each 32-bit host word is two SDRAM writes (req/ack toggles, as in
// Genesis.sv), big-endian.
reg         rom_wr = 0;
wire        rom_wrack;
reg  [24:1] wr_addr;
reg  [15:0] wr_data;
reg  [15:0] wr_next;
reg   [1:0] wr_left = 0;
assign dl_busy = wr_left != 0;

always @(posedge clk_sys) begin
	if (dl_wr) begin
		wr_addr <= dl_addr[24:1];
		wr_data <= {dl_data[7:0], dl_data[15:8]};
		wr_next <= {dl_data[23:16], dl_data[31:24]};
		rom_wr  <= ~rom_wr;
		wr_left <= 2;
	end else if (wr_left != 0 && rom_wr == rom_wrack) begin
		if (wr_left == 2) begin
			wr_addr <= wr_addr + 1'd1;
			wr_data <= wr_next;
			rom_wr  <= ~rom_wr;
		end
		wr_left <= wr_left - 1'd1;
	end
end

// Per-game settings from the cartridge header (Genesis.sv): the serial
// number at 0x182-0x18B.
reg sram_quirk = 0, sram00_quirk = 0, eeprom_quirk = 0, fifo_quirk = 0;
reg noram_quirk = 0, pier_quirk = 0, fmbusy_quirk = 0, schan_quirk = 0;
reg [63:0] cart_id;
always @(posedge clk_sys) begin
	reg old_dl;
	old_dl <= dl_en;
	if (dl_en & ~old_dl)
		{fifo_quirk, eeprom_quirk, sram_quirk, sram00_quirk, noram_quirk, pier_quirk, fmbusy_quirk, schan_quirk} <= 0;

	if (dl_wr) begin
		// The 8 bytes at 0x183-0x18A, first byte in the top bits (as Genesis.sv)
		if (dl_addr == 'h180) cart_id[63:56] <= dl_data[31:24];
		if (dl_addr == 'h184) cart_id[55:24] <= {dl_data[7:0], dl_data[15:8], dl_data[23:16], dl_data[31:24]};
		if (dl_addr == 'h188) cart_id[23:0]  <= {dl_data[7:0], dl_data[15:8], dl_data[23:16]};
	end
	if (dl_wr && dl_addr == 'h18C) begin
		     if (cart_id == "T-081276") sram_quirk   <= 1; // NFL Quarterback Club
		else if (cart_id == "T-81406 ") sram_quirk   <= 1; // NBA Jam TE
		else if (cart_id == "T-081586") sram_quirk   <= 1; // NFL Quarterback Club '96
		else if (cart_id == "T-81576 ") sram_quirk   <= 1; // College Slam
		else if (cart_id == "T-81476 ") sram_quirk   <= 1; // Frank Thomas Big Hurt Baseball
		else if (cart_id == "MK-1215 ") eeprom_quirk <= 1; // Evander Real Deal Holyfield's Boxing
		else if (cart_id == "G-4060  ") eeprom_quirk <= 1; // Wonder Boy
		else if (cart_id == "00001211") eeprom_quirk <= 1; // Sports Talk Baseball
		else if (cart_id == "MK-1228 ") eeprom_quirk <= 1; // Greatest Heavyweights
		else if (cart_id == "G-5538  ") eeprom_quirk <= 1; // Greatest Heavyweights JP
		else if (cart_id == "00004076") eeprom_quirk <= 1; // Honoo no Toukyuuji Dodge Danpei
		else if (cart_id == "T-12046 ") eeprom_quirk <= 1; // Mega Man - The Wily Wars
		else if (cart_id == "T-12053 ") eeprom_quirk <= 1; // Rockman Mega World
		else if (cart_id == "G-4524  ") eeprom_quirk <= 1; // Ninja Burai Densetsu
		else if (cart_id == "T-113016") noram_quirk  <= 1; // Puggsy fake ram check
		else if (cart_id == "T-89016 ") fifo_quirk   <= 1; // Clue
		else if (cart_id == "T-574023") pier_quirk   <= 1; // Pier Solar Reprint
		else if (cart_id == "T-574013") pier_quirk   <= 1; // Pier Solar 1st Edition
		else if (cart_id == "T-35036 ") fmbusy_quirk <= 1; // Hellfire US
		else if (cart_id == "T-25073 ") fmbusy_quirk <= 1; // Hellfire JP
		else if (cart_id == "MK-1137-") fmbusy_quirk <= 1; // Hellfire EU
		else if (cart_id == "T-68???-") schan_quirk  <= 1; // Game no Kanzume Otokuyou
		else if (cart_id == " GM 0000") sram00_quirk <= 1; // Sonic 1 Remastered
	end
end

////////////////////////////////  Console  ////////////////////////////////

wire        rom_req, rom_rdack, rom_we;
wire [24:1] rom_addr;
wire [15:0] rom_data, rom_wdata;
wire  [1:0] rom_be;

wire  [3:0] r, g, b;
wire        hblank, vblank, ce_pix;
wire  [1:0] resolution;
wire [15:0] dac_l, dac_r;

wire        bram_we = save_req & save_write & ~save_ack;

system system (
	.RESET_N       (~(reset | dl_en)),
	.MCLK          (clk),

	.LPF_MODE      (2'd0),
	.ENABLE_FM     (1'b1),
	.ENABLE_PSG    (1'b1),
	.DAC_LDATA     (dac_l),
	.DAC_RDATA     (dac_r),

	.LOADING       (dl_en),
	.PAL           (region == 2'd2),
	.EXPORT        (region != 2'd0),
	.FAST_FIFO     (fifo_quirk),
	.SRAM_QUIRK    (sram_quirk),
	.SRAM00_QUIRK  (sram00_quirk),
	.EEPROM_QUIRK  (eeprom_quirk),
	.NORAM_QUIRK   (noram_quirk),
	.PIER_QUIRK    (pier_quirk),
	.SVP_QUIRK     (1'b0),
	.FMBUSY_QUIRK  (fmbusy_quirk),
	.SCHAN_QUIRK   (schan_quirk),

	.TURBO         (2'd0),

	.GG_RESET      (1'b0),
	.GG_EN         (1'b0),
	.GG_CODE       (129'd0),
	.GG_AVAILABLE  (),

	.BRAM_A        (save_addr),
	.BRAM_DI       (save_wdata),
	.BRAM_DO       (save_rdata),
	.BRAM_WE       (bram_we),
	.BRAM_CHANGE   (),

	.RED           (r),
	.GREEN         (g),
	.BLUE          (b),
	.VS            (),
	.HS            (),
	.HBL           (hblank),
	.VBL           (vblank),
	.CE_PIX        (ce_pix),
	.BORDER        (1'b0),
	.CRAM_DOTS     (1'b0),

	.INTERLACE     (),
	.FIELD         (),
	.RESOLUTION    (resolution),

	.J3BUT         (~six_button),
	.JOY_1         (joy1),
	.JOY_2         (12'd0),
	.JOY_3         (12'd0),
	.JOY_4         (12'd0),
	.JOY_5         (12'd0),
	.MULTITAP      (3'd0),

	.MOUSE         (25'd0),
	.MOUSE_OPT     (3'd0),

	.GUN_OPT       (1'b0),
	.GUN_TYPE      (1'b0),
	.GUN_SENSOR    (1'b0),
	.GUN_A         (1'b0),
	.GUN_B         (1'b0),
	.GUN_C         (1'b0),
	.GUN_START     (1'b0),

	.SERJOYSTICK_IN(8'd0),
	.SERJOYSTICK_OUT(),
	.SER_OPT       (2'd0),

	.ROMSZ         (rom_size[24:1]),
	.ROM_ADDR      (rom_addr),
	.ROM_DATA      (rom_data),
	.ROM_WDATA     (rom_wdata),
	.ROM_WE        (rom_we),
	.ROM_BE        (rom_be),
	.ROM_REQ       (rom_req),
	.ROM_ACK       (rom_rdack),

	.ROM_ADDR2     (),
	.ROM_DATA2     (16'd0),
	.ROM_REQ2      (),
	.ROM_ACK2      (1'b0),

	.EN_HIFI_PCM   (1'b0),
	.LADDER        (1'b1),
	.OBJ_LIMIT_HIGH(1'b0),

	.TRANSP_DETECT (),

	.PAUSE_EN      (1'b0),
	.BGA_EN        (1'b1),
	.BGB_EN        (1'b1),
	.SPR_EN        (1'b1),
	.DBG_M68K_A    (),
	.DBG_VBUS_A    ()
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

sdram sdram (
	.init        (sdram_init),
	.clk         (clk_mem),

	.addr0       (wr_addr),
	.din0        (wr_data),
	.dout0       (),
	.wrl0        (1'b1),
	.wrh0        (1'b1),
	.req0        (rom_wr),
	.ack0        (rom_wrack),

	.addr1       (rom_addr),
	.din1        (rom_wdata),
	.dout1       (rom_data),
	.wrl1        (rom_we & rom_be[0]),
	.wrh1        (rom_we & rom_be[1]),
	.req1        (rom_req),
	.ack1        (rom_rdack),

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

// The platform writes one framebuffer pixel per video_valid and starts a new
// row on each rising edge of video_hblank. Lines are 320 pixels: 256-pixel
// (H32) lines get 32 black pixels on each side, sent one per clock from a
// small FIFO that holds the line's first pixels meanwhile. 240-line (V30, PAL)
// frames lose 8 lines at the top and bottom.
localparam PAD = 32;

initial video_hblank = 1;

reg  [11:0] fifo[8];
reg   [2:0] fifo_wr = 0, fifo_rd = 0;
wire        fifo_empty = fifo_wr == fifo_rd;

reg   [1:0] state = 0;        // 0: between lines, 1: left pad, 2: line, 3: right pad
reg   [5:0] pad_count;
reg         h32;
reg         line_done;
reg   [8:0] line;
reg         old_hblank, old_vblank;

// Lines shown: 0-223 of a 224-line frame, 8-231 of a 240-line one.
wire        v30 = resolution[1];
wire        line_shown = v30 ? (line >= 8 && line < 232) : (line < 224);

always @(posedge clk_sys) begin
	video_valid <= 0;
	video_r <= 0;
	video_g <= 0;
	video_b <= 0;

	if (ce_pix) begin
		old_hblank <= hblank;
		old_vblank <= vblank;
		if (vblank & ~old_vblank) line <= 0;
		if (hblank & ~old_hblank & ~vblank) begin
			line <= line + 1'd1;
			line_done <= 1;
		end
		// Pixels of the line go to the FIFO.
		if (~hblank & ~vblank & line_shown) begin
			fifo[fifo_wr] <= {r, g, b};
			fifo_wr <= fifo_wr + 1'd1;
		end
		// A line starts.
		if (~hblank & old_hblank & ~vblank & line_shown) begin
			h32 <= ~resolution[0];
			state <= resolution[0] ? 2'd2 : 2'd1;
			pad_count <= 0;
			line_done <= 0;
			video_hblank <= 0;
		end
	end

	case (state)
	1: begin
		video_valid <= 1;
		pad_count <= pad_count + 1'd1;
		if (pad_count == PAD - 1) state <= 2;
	end
	2: if (!fifo_empty) begin
		{video_r, video_g, video_b} <= fifo[fifo_rd];
		fifo_rd <= fifo_rd + 1'd1;
		video_valid <= 1;
	end else if (line_done) begin
		pad_count <= 0;
		state <= h32 ? 2'd3 : 2'd0;
		if (!h32) video_hblank <= 1;
	end
	3: begin
		video_valid <= 1;
		pad_count <= pad_count + 1'd1;
		if (pad_count == PAD - 1) begin
			state <= 0;
			video_hblank <= 1;
		end
	end
	default: ;
	endcase

	video_vblank <= vblank;
	if (vblank) begin
		state <= 0;
		video_hblank <= 1;
		fifo_rd <= fifo_wr;
	end
end

/////////////////////////////////  Audio  /////////////////////////////////

assign audio_l = dac_l;
assign audio_r = dac_r;

endmodule
