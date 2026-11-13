// Portable replacements for the MiSTer NES core's altsyncram wrappers
// (mister/dpram.vhd), for Vivado and Verilator.
//
// Behaviour matches the altsyncram configuration they replace:
//   * address registered, output unregistered (one cycle of read latency)
//   * read-during-write on the same port returns the new data
// Ports have no default values (Verilator doesn't support them), so the ones
// the core leaves unconnected are ignored: `enable` and `cs` (always enabled
// where spram is used) and `byteena_a/b` (always all ones where dpram is used).
// Memory initialisation files aren't supported. The only one in the core
// (video.sv's custom palette RAM, tao.mif) is used only after a palette is
// loaded, which this port doesn't do.

module spram #(
	parameter addr_width = 8,
	parameter data_width = 8,
	parameter mem_init_file = " ",
	parameter mem_name = "MEM"
) (
	input                   clock,
	input  [addr_width-1:0] address,
	input  [data_width-1:0] data,
	input                   enable,
	input                   wren,
	output [data_width-1:0] q,
	input                   cs
);
	spram_sz #(addr_width, data_width, 2**addr_width, mem_init_file, mem_name) ram (
		.clock(clock), .address(address), .data(data), .enable(enable),
		.wren(wren), .q(q), .cs(cs)
	);
endmodule

module spram_sz #(
	parameter addr_width = 8,
	parameter data_width = 8,
	parameter numwords = 2**8,
	parameter mem_init_file = " ",
	parameter mem_name = "MEM"
) (
	input                   clock,
	input  [addr_width-1:0] address,
	input  [data_width-1:0] data,
	input                   enable,
	input                   wren,
	output [data_width-1:0] q,
	input                   cs
);
	reg [data_width-1:0] mem[2**addr_width];
	reg [data_width-1:0] q0;

	assign q = q0;

	always @(posedge clock) begin
		if (wren) begin
			mem[address] <= data;
			q0 <= data;
		end else begin
			q0 <= mem[address];
		end
	end
endmodule

module dpram #(
	parameter init_file = " ",
	parameter widthad_a = 8,
	parameter width_a = 8,
	parameter outdata_reg_a = "UNREGISTERED",
	parameter outdata_reg_b = "UNREGISTERED"
) (
	input  [widthad_a-1:0] address_a,
	input  [widthad_a-1:0] address_b,
	input                  clock_a,
	input                  clock_b,
	input    [width_a-1:0] data_a,
	input    [width_a-1:0] data_b,
	input                  wren_a,
	input                  wren_b,
	input  [width_a/8-1:0] byteena_a,
	input  [width_a/8-1:0] byteena_b,
	output   [width_a-1:0] q_a,
	output   [width_a-1:0] q_b
);
	reg [width_a-1:0] mem[2**widthad_a];
	reg [width_a-1:0] qa0, qb0, qa1, qb1;

	assign q_a = (outdata_reg_a == "UNREGISTERED") ? qa0 : qa1;
	assign q_b = (outdata_reg_b == "UNREGISTERED") ? qb0 : qb1;

	always @(posedge clock_a) begin
		reg [width_a-1:0] value;
		value = mem[address_a];
		if (wren_a) begin
			value = data_a;
			mem[address_a] <= value;
		end
		qa0 <= value;
		qa1 <= qa0;
	end

	always @(posedge clock_b) begin
		reg [width_a-1:0] value;
		value = mem[address_b];
		if (wren_b) begin
			value = data_b;
			mem[address_b] <= value;
		end
		qb0 <= value;
		qb1 <= qb0;
	end
endmodule
