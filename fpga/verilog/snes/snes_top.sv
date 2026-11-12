// Game Bub top level for the MiSTer SNES core: the console (main.v) plus its
// memories, controller ports and video output conditioning.
//
// - ROM and WRAM live in SDRAM (MiSTer's controller, clocked at 4x clk_sys).
// - VRAM and ARAM are block RAM.
// - Cartridge battery RAM (BSRAM) is the external asynchronous SRAM.
module snes_top (
	input             clk_sys,      // SNES master clock (~21.47 MHz)
	input             clk_mem,      // 4x clk_sys, phase-aligned

	input             reset,        // hold the console in reset
	input             enable,       // 0 pauses the console
	input             sdram_init,   // high until the SDRAM may be initialized

	// Cartridge configuration (from the ROM header, set by the firmware)
	input       [7:0] rom_type,
	input      [23:0] rom_mask,
	input      [23:0] ram_mask,
	input       [3:0] ram_size,
	input             pal,

	// ROM download into SDRAM (core must be in reset). One 16-bit word per
	// dl_wr pulse; leave at least 3 clk_sys cycles between pulses.
	input             dl_en,
	input             dl_wr,
	input      [23:0] dl_addr,
	input      [15:0] dl_data,

	// Controller 1: {start, select, r, l, y, x, b, a, up, down, left, right}
	input      [11:0] joy1,

	// Video: 256x224, 5 bits per channel
	output reg  [4:0] video_r,
	output reg  [4:0] video_g,
	output reg  [4:0] video_b,
	output reg        video_valid,
	output reg        video_hblank,
	output reg        video_vblank,

	output     [15:0] audio_l,
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
	output            sdram_cke,

	// External SRAM (512 KiB, 16-bit) for BSRAM
	output     [17:0] sram_addr,
	input      [15:0] sram_dq_in,
	output     [15:0] sram_dq_out,
	output            sram_dq_dir,
	output            sram_oe_n,
	output reg        sram_we_n,
	output      [1:0] sram_mask_n
);

////////////////////////////  RESET  ///////////////////////////////////

// As in MiSTer: release reset on a fixed phase, and generate refresh requests
// for SDRAM port 0 while the console isn't running.
reg RESET_N = 0;
reg IDLE_REFRESH = 0;
always @(posedge clk_sys) begin
	reg [1:0] div;

	div <= div + 1'd1;
	IDLE_REFRESH <= !div;

	if (div == 2) RESET_N <= ~(reset | dl_en);
end
wire running = RESET_N & enable;

////////////////////////////  CONSOLE  /////////////////////////////////

wire [23:0] ROM_ADDR;
wire [15:0] ROM_D, ROM_Q;
wire        ROM_OE_N, ROM_WE_N, ROM_WORD;

wire [19:0] BSRAM_ADDR;
wire  [7:0] BSRAM_D;
reg   [7:0] BSRAM_Q;
wire        BSRAM_CE_N, BSRAM_WE_N;

wire [16:0] WRAM_ADDR;
wire  [7:0] WRAM_D, WRAM_Q;
wire        WRAM_CE_N, WRAM_OE_N, WRAM_WE_N;

wire [15:0] VRAM1_ADDR, VRAM2_ADDR;
wire  [7:0] VRAM1_D, VRAM2_D;
reg   [7:0] VRAM1_Q, VRAM2_Q;
wire        VRAM1_WE_N, VRAM2_WE_N;

wire [15:0] ARAM_ADDR;
wire  [7:0] ARAM_D;
reg   [7:0] ARAM_Q;
wire        ARAM_CE_N, ARAM_WE_N;

wire        SYSCLKR_CE, SYSCLKF_CE, REFRESH;

wire  [7:0] R, G, B;
wire        DOTCLK, HBLANKn, VBLANKn, V224_MODE;

wire  [1:0] JOY1_DI, JOY2_DI;
wire        JOY_STRB, JOY1_CLK, JOY2_CLK, JOY1_P6, JOY2_P6;

main #(
	.USE_DLH(1'b1),
	.USE_CX4(1'b0),
	.USE_SDD1(1'b0),
	.USE_GSU(1'b0),
	.USE_SA1(1'b0),
	.USE_DSPn(1'b0),
	.USE_SPC7110(1'b0),
	.USE_BSX(1'b0),
	.USE_SUFAMI(1'b0),
	.USE_MSU(1'b0),
	.USE_SS(1'b0)
) main (
	.RESET_N(RESET_N),
	.ENABLE(enable),

	.MCLK(clk_sys),
	.ACLK(clk_sys),

	.ROM_TYPE(rom_type),
	.ROM_MASK(rom_mask),
	.RAM_MASK(ram_mask),
	.RAM_SIZE(ram_size),

	.SYSCLKR_CE(SYSCLKR_CE),
	.SYSCLKF_CE(SYSCLKF_CE),
	.REFRESH(REFRESH),

	.ROM_ADDR(ROM_ADDR),
	.ROM_D(ROM_D),
	.ROM_Q(ROM_Q),
	.ROM_CE_N(),
	.ROM_OE_N(ROM_OE_N),
	.ROM_WE_N(ROM_WE_N),
	.ROM_WORD(ROM_WORD),

	.BSRAM_ADDR(BSRAM_ADDR),
	.BSRAM_D(BSRAM_D),
	.BSRAM_Q(BSRAM_Q),
	.BSRAM_CE_N(BSRAM_CE_N),
	.BSRAM_OE_N(),
	.BSRAM_WE_N(BSRAM_WE_N),

	.WRAM_ADDR(WRAM_ADDR),
	.WRAM_D(WRAM_D),
	.WRAM_Q(WRAM_Q),
	.WRAM_CE_N(WRAM_CE_N),
	.WRAM_OE_N(WRAM_OE_N),
	.WRAM_WE_N(WRAM_WE_N),

	.VRAM1_ADDR(VRAM1_ADDR),
	.VRAM1_DI(VRAM1_Q),
	.VRAM1_DO(VRAM1_D),
	.VRAM1_WE_N(VRAM1_WE_N),
	.VRAM2_ADDR(VRAM2_ADDR),
	.VRAM2_DI(VRAM2_Q),
	.VRAM2_DO(VRAM2_D),
	.VRAM2_WE_N(VRAM2_WE_N),
	.VRAM_OE_N(),

	.ARAM_ADDR(ARAM_ADDR),
	.ARAM_D(ARAM_D),
	.ARAM_Q(ARAM_Q),
	.ARAM_CE_N(ARAM_CE_N),
	.ARAM_OE_N(),
	.ARAM_WE_N(ARAM_WE_N),

	.GSU_ACTIVE(),
	.GSU_TURBO(1'b0),
	.GSU_FASTROM(1'b0),
	.SUFAMI_SWAP(1'b0),

	.BLEND(1'b0),
	.PAL(pal),
	.HIGH_RES(),
	.V224_MODE(V224_MODE),
	.FIELD(),
	.INTERLACE(),
	.DOTCLK(DOTCLK),
	.R(R),
	.G(G),
	.B(B),
	.HBLANKn(HBLANKn),
	.VBLANKn(VBLANKn),
	.HSYNC(),
	.VSYNC(),

	.JOY1_DI(JOY1_DI),
	.JOY2_DI(JOY2_DI),
	.JOY_STRB(JOY_STRB),
	.JOY1_CLK(JOY1_CLK),
	.JOY2_CLK(JOY2_CLK),
	.JOY1_P6(JOY1_P6),
	.JOY2_P6(JOY2_P6),
	.JOY2_P6_in(1'b1),

	.EXT_RTC(65'd0),

	.GG_EN(1'b0),
	.GG_CODE(129'd0),
	.GG_RESET(1'b0),
	.GG_AVAILABLE(),

	.SPC_MODE(1'b0),

	.IO_ADDR(17'd0),
	.IO_DAT(16'd0),
	.IO_WR(1'b0),

	.DBG_BG_EN(5'b11111),
	.DBG_CPU_EN(1'b1),

	.TURBO(1'b0),
	.TURBO_ALLOW(),

	.MSU_TRACK_NUM(),
	.MSU_TRACK_REQUEST(),
	.MSU_TRACK_MOUNTING(1'b0),
	.MSU_TRACK_MISSING(1'b0),
	.MSU_VOLUME(),
	.MSU_AUDIO_STOP(1'b0),
	.MSU_AUDIO_REPEAT(),
	.MSU_AUDIO_PLAYING(),
	.MSU_DATA_ADDR(),
	.MSU_DATA(8'd0),
	.MSU_DATA_ACK(1'b0),
	.MSU_DATA_SEEK(),
	.MSU_DATA_REQ(),
	.MSU_ENABLE(1'b0),

	.SS_SAVE(1'b0),
	.SS_TOSD(1'b0),
	.SS_LOAD(1'b0),
	.SS_SLOT(2'd0),
	.SS_AVAIL(),
	.SS_DDR_DI(64'd0),
	.SS_DDR_ACK(1'b0),
	.SS_DDR_DO(),
	.SS_DDR_ADDR(),
	.SS_DDR_WE(),
	.SS_DDR_BE(),
	.SS_DDR_REQ(),

	.AUDIO_L(audio_l),
	.AUDIO_R(audio_r)
);

////////////////////////////  SDRAM  ///////////////////////////////////

reg        sdram_dl_en;
reg        sdram_dl_wr;
reg [23:0] sdram_dl_addr;
reg [15:0] sdram_dl_data;
reg        READ_PULSE;
always @(posedge clk_sys) begin
	sdram_dl_en <= dl_en;
	sdram_dl_wr <= dl_wr;
	sdram_dl_addr <= dl_addr;
	sdram_dl_data <= dl_data;
	READ_PULSE <= SYSCLKR_CE;
end

wire [15:0] sdr_dout1;
assign WRAM_Q = sdr_dout1[7:0];

sdram_snes sdram (
	.SDRAM_DQ_IN(sdram_dq_in),
	.SDRAM_DQ_OUT(sdram_dq_out),
	.SDRAM_DQ_DIR(sdram_dq_dir),
	.SDRAM_A(sdram_a),
	.SDRAM_DQM(sdram_dqm),
	.SDRAM_BA(sdram_ba),
	.SDRAM_nCS(sdram_cs_n),
	.SDRAM_nWE(sdram_we_n),
	.SDRAM_nRAS(sdram_ras_n),
	.SDRAM_nCAS(sdram_cas_n),
	.SDRAM_CKE(sdram_cke),

	.init(sdram_init),
	.clk(clk_mem),

	.addr0(sdram_dl_en ? sdram_dl_addr : ROM_ADDR),
	.din0(sdram_dl_en ? sdram_dl_data : ROM_D),
	.dout0(ROM_Q),
	.rd0(sdram_dl_en ? 1'b0 : !running ? IDLE_REFRESH : ~ROM_OE_N),
	.wr0(sdram_dl_en ? sdram_dl_wr : ~ROM_WE_N),
	.word0(sdram_dl_en | ROM_WORD),

	.addr1({7'b0000000, WRAM_ADDR}),
	.din1({8'h00, WRAM_D}),
	.dout1(sdr_dout1),
	.rd1(~WRAM_CE_N & ~WRAM_OE_N & READ_PULSE),
	.wr1(~WRAM_CE_N & ~WRAM_WE_N & SYSCLKF_CE),
	.rfs1(!running ? IDLE_REFRESH : REFRESH),
	.word1(1'b0)
);

////////////////////////////  BLOCK RAM  ///////////////////////////////

reg [7:0] vram1[32768];
reg [7:0] vram2[32768];
reg [7:0] aram[65536];

always @(posedge clk_sys) begin
	if (~VRAM1_WE_N) vram1[VRAM1_ADDR[14:0]] <= VRAM1_D;
	VRAM1_Q <= vram1[VRAM1_ADDR[14:0]];
end

always @(posedge clk_sys) begin
	if (~VRAM2_WE_N) vram2[VRAM2_ADDR[14:0]] <= VRAM2_D;
	VRAM2_Q <= vram2[VRAM2_ADDR[14:0]];
end

always @(posedge clk_sys) begin
	if (~ARAM_CE_N & ~ARAM_WE_N) aram[ARAM_ADDR] <= ARAM_D;
	ARAM_Q <= aram[ARAM_ADDR];
end

////////////////////////////  BSRAM (external SRAM)  ///////////////////

// The SRAM is asynchronous: present the address for the whole clk_sys cycle
// and register the data at the end of it, which matches the one-cycle read
// latency of the block RAM MiSTer uses. Writes pulse WE# in the middle two
// quarters of the cycle (timed with clk_mem) so the address is stable around it.
wire bsram_write = ~BSRAM_CE_N & ~BSRAM_WE_N;

assign sram_addr = BSRAM_ADDR[18:1];
assign sram_dq_out = {BSRAM_D, BSRAM_D};
assign sram_dq_dir = bsram_write;
assign sram_oe_n = bsram_write;
assign sram_mask_n = BSRAM_ADDR[0] ? 2'b01 : 2'b10;

always @(posedge clk_sys) begin
	BSRAM_Q <= BSRAM_ADDR[0] ? sram_dq_in[15:8] : sram_dq_in[7:0];
end

reg sys_toggle = 0;
always @(posedge clk_sys) sys_toggle <= ~sys_toggle;

always @(posedge clk_mem) begin
	reg       last_toggle;
	reg [1:0] phase;

	last_toggle <= sys_toggle;
	phase <= (last_toggle != sys_toggle) ? 2'd1 : phase + 2'd1;
	sram_we_n <= ~(bsram_write & (phase == 2'd1 || phase == 2'd2));
end

////////////////////////////  CONTROLLERS  /////////////////////////////

// ioport.sv takes MiSTer joystick bits:
// {start, select, r, l, y, x, b, a, up, down, left, right}
ioport port1 (
	.CLK(clk_sys),
	.MULTITAP(1'b0),
	.PORT_LATCH(JOY_STRB),
	.PORT_CLK(JOY1_CLK),
	.PORT_P6(JOY1_P6),
	.PORT_DO(JOY1_DI),
	.JOYSTICK1_RUMBLE(),
	.JOYSTICK1(joy1),
	.JOYSTICK2(12'd0),
	.JOYSTICK3(12'd0),
	.JOYSTICK4(12'd0),
	.MOUSE(25'd0),
	.MOUSE_EN(1'b0)
);

ioport port2 (
	.CLK(clk_sys),
	.MULTITAP(1'b0),
	.PORT_LATCH(JOY_STRB),
	.PORT_CLK(JOY2_CLK),
	.PORT_P6(JOY2_P6),
	.PORT_DO(JOY2_DI),
	.JOYSTICK1_RUMBLE(),
	.JOYSTICK1(12'd0),
	.JOYSTICK2(12'd0),
	.JOYSTICK3(12'd0),
	.JOYSTICK4(12'd0),
	.MOUSE(25'd0),
	.MOUSE_EN(1'b0)
);

////////////////////////////  VIDEO  ///////////////////////////////////

// One pixel per dot clock rising edge (256 per line; in hi-res modes this
// takes one of each pair of half-pixels). The visible lines are cropped to
// 224: in 239-line mode the top 8 and bottom 7 are dropped. The platform
// starts a new framebuffer row on each rising edge of hblank, so hblank stays
// high through dropped lines.
always @(posedge clk_sys) begin
	reg       last_dotclk;
	reg       last_hblank_n;
	reg [8:0] line;
	reg       line_ok;

	last_dotclk <= DOTCLK;
	video_valid <= 0;

	if (DOTCLK & ~last_dotclk) begin
		last_hblank_n <= HBLANKn;

		if (~VBLANKn) begin
			line <= 0;
			line_ok <= 0;
		end else if (HBLANKn & ~last_hblank_n) begin
			// Start of a visible line
			line_ok <= V224_MODE ? (line < 224) : (line >= 8 && line < 232);
			line <= line + 1'd1;
		end

		video_r <= R[7:3];
		video_g <= G[7:3];
		video_b <= B[7:3];
		video_vblank <= ~VBLANKn;
	end

	if (DOTCLK & ~last_dotclk & HBLANKn & VBLANKn) begin
		// line_ok was updated this cycle for the line's first pixel, so use
		// the same condition here.
		if (~last_hblank_n) video_valid <= V224_MODE ? (line < 224) : (line >= 8 && line < 232);
		else video_valid <= line_ok;
	end

	video_hblank <= ~(HBLANKn & line_ok) | ~VBLANKn;
end

endmodule
