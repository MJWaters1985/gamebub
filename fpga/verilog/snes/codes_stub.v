// Stand-in for the MiSTer core's Game Genie module (cheatcodes.sv), which the
// Game Bub build leaves out: no codes are ever available or applied.
module CODES #(
	parameter ADDR_WIDTH = 16,
	parameter DATA_WIDTH = 8,
	parameter MAX_CODES  = 32
) (
	input                     clk,
	input                     reset,
	input                     enable,
	output                    available,
	input  [ADDR_WIDTH - 1:0] addr_in,
	input  [DATA_WIDTH - 1:0] data_in,
	input  [128:0]            code,
	output                    genie_ovr,
	output [DATA_WIDTH - 1:0] genie_data
);

assign available = 1'b0;
assign genie_ovr = 1'b0;
assign genie_data = {DATA_WIDTH{1'b0}};

endmodule
