// Dual-port block RAM for the MiSTer Genesis core's SystemVerilog modules
// (system.sv, miracle.sv), replacing its VHDL `dpram` (mister/bram.vhd) with
// the same behaviour: address registered, output unregistered, read-during-
// write on a port returns the new data, `enable` gates a port's clock and a
// port's output reads all ones while its `cs` is low.

module dpram #(
	parameter addr_width = 8,
	parameter data_width = 8,
	parameter mem_init_file = " "
) (
	input                   clock,

	input  [addr_width-1:0] address_a,
	input  [data_width-1:0] data_a,
	input                   enable_a,
	input                   wren_a,
	output [data_width-1:0] q_a,
	input                   cs_a,

	input  [addr_width-1:0] address_b,
	input  [data_width-1:0] data_b,
	input                   enable_b,
	input                   wren_b,
	output [data_width-1:0] q_b,
	input                   cs_b
);
	reg [data_width-1:0] mem[2**addr_width];
	reg [data_width-1:0] qa, qb;

	assign q_a = cs_a ? qa : '1;
	assign q_b = cs_b ? qb : '1;

	always @(posedge clock) begin
		if (enable_a) begin
			if (wren_a) begin
				mem[address_a] <= data_a;
				qa <= data_a;
			end else begin
				qa <= mem[address_a];
			end
		end
	end

	always @(posedge clock) begin
		if (enable_b) begin
			if (wren_b) begin
				mem[address_b] <= data_b;
				qb <= data_b;
			end else begin
				qb <= mem[address_b];
			end
		end
	end
endmodule
