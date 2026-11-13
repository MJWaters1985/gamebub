// Simulation-only stand-ins for the NES core's savestate VHDL (Vivado uses
// mister/bus_savestates.vhd, savestates.vhd and statemanager.vhd directly).

// Savestate bus register: holds `def` until the savestate bus writes it.
// Same behaviour as mister/bus_savestates.vhd.
module eReg_SavestateV #(
	parameter [9:0]  Adr = 0,
	parameter [63:0] def = 0
) (
	input             clk,
	input      [63:0] BUS_Din,
	input       [9:0] BUS_Adr,
	input             BUS_wren,
	input             BUS_rst,
	output     [63:0] BUS_Dout,
	input      [63:0] Din,
	output reg [63:0] Dout = def
);
	always @(posedge clk) begin
		if (BUS_rst) Dout <= def;
		else if (BUS_Adr == Adr && BUS_wren) Dout <= BUS_Din;
	end
	assign BUS_Dout = (BUS_Adr == Adr) ? Din : 64'd0;
endmodule

// Savestates are not used: the controller stays idle.
module savestates (
	input         clk,
	input         reset_in,
	output        reset_ss,
	output        reset_delay,
	output        load_done,
	input         increaseSSHeaderCount,
	input         save,
	input         load,
	input  [31:0] savestate_address,
	output        savestate_busy,
	input         paused,
	output [63:0] BUS_Din,
	output  [9:0] BUS_Adr,
	output        BUS_wren,
	output        BUS_rst,
	input  [63:0] BUS_Dout,
	output        loading_savestate,
	output        saving_savestate,
	output        sleep_savestate,
	output [24:0] Save_RAMAddr,
	output        Save_RAMRdEn,
	output        Save_RAMWrEn,
	output  [7:0] Save_RAMWriteData,
	input   [7:0] Save_RAMReadData,
	output  [2:0] Save_RAMType,
	output [63:0] bus_out_Din,
	input  [63:0] bus_out_Dout,
	output [25:0] bus_out_Adr,
	output        bus_out_rnw,
	output        bus_out_ena,
	output  [7:0] bus_out_be,
	input         bus_out_done
);
	assign {reset_ss, reset_delay, load_done, savestate_busy} = 0;
	assign {BUS_Din, BUS_Adr, BUS_wren, BUS_rst} = 0;
	assign {loading_savestate, saving_savestate, sleep_savestate} = 0;
	assign {Save_RAMAddr, Save_RAMRdEn, Save_RAMWrEn, Save_RAMWriteData, Save_RAMType} = 0;
	assign {bus_out_Din, bus_out_Adr, bus_out_rnw, bus_out_ena, bus_out_be} = 0;
endmodule

module statemanager #(
	parameter Softmap_SaveState_ADDR = 0,
	parameter Softmap_Rewind_ADDR = 0
) (
	input         clk,
	input         reset,
	input         rewind_on,
	input         rewind_active,
	input  [31:0] savestate_number,
	input         save,
	input         load,
	output        sleep_rewind,
	input         vsync,
	output        request_savestate,
	output        request_loadstate,
	output [31:0] request_address,
	input         request_busy
);
	assign {sleep_rewind, request_savestate, request_loadstate, request_address} = 0;
endmodule
