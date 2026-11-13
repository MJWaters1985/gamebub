--------------------------------------------------------------
-- Portable replacements for the Intel IP the MiSTer SNES core's
-- enhancement chips use (altdpram, LPM_MULT, LPM_DIVIDE), with the
-- same ports and timing.
--------------------------------------------------------------

-- CX4 cache: 512x8, registered write, asynchronous read (altdpram).
LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

ENTITY cx4cache IS
	PORT
	(
		clock		: IN STD_LOGIC ;
		data		: IN STD_LOGIC_VECTOR (7 DOWNTO 0);
		rdaddress		: IN STD_LOGIC_VECTOR (8 DOWNTO 0);
		wraddress		: IN STD_LOGIC_VECTOR (8 DOWNTO 0);
		wren		: IN STD_LOGIC  := '0';
		q		: OUT STD_LOGIC_VECTOR (7 DOWNTO 0)
	);
END cx4cache;

ARCHITECTURE SYN OF cx4cache IS
	type mem_t is array(0 to 511) of std_logic_vector(7 downto 0);
	signal mem : mem_t := (others => (others => '0'));
BEGIN
	process(clock)
	begin
		if rising_edge(clock) then
			if wren = '1' then
				mem(to_integer(unsigned(wraddress))) <= data;
			end if;
		end if;
	end process;
	q <= mem(to_integer(unsigned(rdaddress)));
END SYN;

--------------------------------------------------------------
-- SA-1 multiplier: signed 16x16 -> 32, combinational (LPM_MULT).
LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

ENTITY SA1MULT IS
	PORT
	(
		dataa		: IN STD_LOGIC_VECTOR (15 DOWNTO 0);
		datab		: IN STD_LOGIC_VECTOR (15 DOWNTO 0);
		result		: OUT STD_LOGIC_VECTOR (31 DOWNTO 0)
	);
END SA1MULT;

ARCHITECTURE SYN OF SA1MULT IS
BEGIN
	result <= std_logic_vector(signed(dataa) * signed(datab));
END SYN;

--------------------------------------------------------------
-- SA-1 divider (LPM_DIVIDE): signed numerator, unsigned denominator,
-- remainder always positive, 6 clocks of latency. A restoring divider
-- spread over the 6 pipeline stages; division by zero gives the same
-- all-ones quotient and numerator remainder as the unsigned algorithm.
LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

ENTITY SA1DIV IS
	PORT
	(
		clock		: IN STD_LOGIC ;
		denom		: IN STD_LOGIC_VECTOR (15 DOWNTO 0);
		numer		: IN STD_LOGIC_VECTOR (15 DOWNTO 0);
		quotient		: OUT STD_LOGIC_VECTOR (15 DOWNTO 0);
		remain		: OUT STD_LOGIC_VECTOR (15 DOWNTO 0)
	);
END SA1DIV;

ARCHITECTURE SYN OF SA1DIV IS
	constant STAGES : integer := 6;
	constant BITS   : integer := 16;

	type stage_t is record
		negative : std_logic;
		denom    : unsigned(BITS-1 downto 0);
		-- Remaining dividend bits (shifted out from the top) and partial remainder
		dividend : unsigned(BITS-1 downto 0);
		remain   : unsigned(BITS downto 0);
		quotient : unsigned(BITS-1 downto 0);
	end record;
	type pipe_t is array(0 to STAGES) of stage_t;
	signal pipe : pipe_t;

	-- Quotient bits computed in each stage (16 over 6 stages: 3,3,3,3,2,2).
	function bits_in_stage(s : integer) return integer is
	begin
		if s < 4 then return 3; else return 2; end if;
	end function;

	function step(x : stage_t) return stage_t is
		variable y : stage_t := x;
		variable trial : unsigned(BITS downto 0);
	begin
		trial := y.remain(BITS-1 downto 0) & y.dividend(BITS-1);
		y.dividend := y.dividend(BITS-2 downto 0) & '0';
		if trial >= ('0' & y.denom) then
			y.remain := trial - ('0' & y.denom);
			y.quotient := y.quotient(BITS-2 downto 0) & '1';
		else
			y.remain := trial;
			y.quotient := y.quotient(BITS-2 downto 0) & '0';
		end if;
		return y;
	end function;
BEGIN
	-- Stage 0 (combinational input)
	process(numer, denom)
		variable n : signed(BITS-1 downto 0);
	begin
		n := signed(numer);
		pipe(0).negative <= numer(BITS-1);
		pipe(0).denom <= unsigned(denom);
		if n < 0 then
			pipe(0).dividend <= unsigned(-n);
		else
			pipe(0).dividend <= unsigned(n);
		end if;
		pipe(0).remain <= (others => '0');
		pipe(0).quotient <= (others => '0');
	end process;

	gen : for s in 0 to STAGES-1 generate
		process(clock)
			variable x : stage_t;
		begin
			if rising_edge(clock) then
				x := pipe(s);
				for i in 1 to bits_in_stage(s) loop
					x := step(x);
				end loop;
				pipe(s+1) <= x;
			end if;
		end process;
	end generate;

	-- Euclidean result for negative numerators.
	process(pipe(STAGES))
		variable x : stage_t;
		variable q : unsigned(BITS-1 downto 0);
		variable r : unsigned(BITS-1 downto 0);
	begin
		x := pipe(STAGES);
		q := x.quotient;
		r := x.remain(BITS-1 downto 0);
		if x.negative = '1' then
			if r = 0 then
				q := unsigned(-signed(q));
			else
				q := unsigned(-signed(q + 1));
				r := x.denom - r;
			end if;
		end if;
		quotient <= std_logic_vector(q);
		remain <= std_logic_vector(r);
	end process;
END SYN;
