--------------------------------------------------------------
-- Portable replacements for the MiSTer SNES core's altsyncram
-- wrappers (rtl/bram.vhd), written so that Vivado infers block
-- RAM and GHDL can convert them for Verilator simulation.
--
-- Behaviour matches the altsyncram configuration they replace:
--   * address registered, output unregistered (one cycle of read latency)
--   * read-during-write on the same port returns the new data
--   * for the dual-port RAMs, `enable` is a clock enable for that port
--     (the single-port RAMs ignore it, as the originals do)
--   * `cs` low forces the output to all ones and blocks writes
-- Memory initialisation files are only supported for the single-port RAMs, and
-- only for the chip ROMs in rom_init_pkg.vhd (see gen_rom_init.py).
--------------------------------------------------------------

LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;
USE work.rom_init_pkg.all;

ENTITY spram_sz IS
	generic (
		addr_width    : integer := 8;
		data_width    : integer := 8;
		numwords      : integer := 2**8;
		mem_init_file : string := " ";
		mem_name      : string := "MEM"
	);
	PORT
	(
		clock   : in  STD_LOGIC;
		address : in  STD_LOGIC_VECTOR (addr_width-1 DOWNTO 0);
		data    : in  STD_LOGIC_VECTOR (data_width-1 DOWNTO 0) := (others => '0');
		enable  : in  STD_LOGIC := '1';
		wren    : in  STD_LOGIC := '0';
		q       : out STD_LOGIC_VECTOR (data_width-1 DOWNTO 0);
		cs      : in  std_logic := '1'
	);
END ENTITY;

ARCHITECTURE SYN OF spram_sz IS
	type mem_t is array(0 to numwords-1) of std_logic_vector(data_width-1 downto 0);

	function ends_with(s, suffix : string) return boolean is
	begin
		return s'length >= suffix'length and
			s(s'right - suffix'length + 1 to s'right) = suffix;
	end function;

	function init_mem return mem_t is
		variable m : mem_t := (others => (others => '0'));
	begin
		if ends_with(mem_init_file, "dsp11b23410_p.mif") then
			for i in 0 to numwords-1 loop
				if i <= DSP_PROG_ROM'high then m(i) := DSP_PROG_ROM(i)(data_width-1 downto 0); end if;
			end loop;
		elsif ends_with(mem_init_file, "dsp11b23410_d.mif") then
			for i in 0 to numwords-1 loop
				if i <= DSP_DATA_ROM'high then m(i) := DSP_DATA_ROM(i)(data_width-1 downto 0); end if;
			end loop;
		elsif ends_with(mem_init_file, "drom.mif") then
			for i in 0 to numwords-1 loop
				if i <= CX4_DATA_ROM'high then m(i) := CX4_DATA_ROM(i)(data_width-1 downto 0); end if;
			end loop;
		end if;
		return m;
	end function;

	signal mem : mem_t := init_mem;
	signal q0  : std_logic_vector(data_width-1 downto 0);
BEGIN
	q <= q0 when cs = '1' else (others => '1');

	process(clock)
	begin
		if rising_edge(clock) then
			if wren = '1' and cs = '1' then
				mem(to_integer(unsigned(address))) <= data;
				q0 <= data;
			else
				q0 <= mem(to_integer(unsigned(address)));
			end if;
		end if;
	end process;
END SYN;

--------------------------------------------------------------

LIBRARY ieee;
USE ieee.std_logic_1164.all;

ENTITY spram IS
	generic (
		addr_width    : integer := 8;
		data_width    : integer := 8;
		mem_init_file : string := " ";
		mem_name      : string := "MEM"
	);
	PORT
	(
		clock   : in  STD_LOGIC;
		address : in  STD_LOGIC_VECTOR (addr_width-1 DOWNTO 0);
		data    : in  STD_LOGIC_VECTOR (data_width-1 DOWNTO 0) := (others => '0');
		enable  : in  STD_LOGIC := '1';
		wren    : in  STD_LOGIC := '0';
		q       : out STD_LOGIC_VECTOR (data_width-1 DOWNTO 0);
		cs      : in  std_logic := '1'
	);
END spram;

ARCHITECTURE SYN OF spram IS
BEGIN
	spram_sz : entity work.spram_sz
	generic map(addr_width, data_width, 2**addr_width, mem_init_file, mem_name)
	port map(clock,address,data,enable,wren,q,cs);
END SYN;

--------------------------------------------------------------
-- Dual port RAM, independent widths on each port (single clock).
-- The wider port's width must be a power-of-two multiple of the
-- narrower port's width.
--------------------------------------------------------------

LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

entity dpram_dif is
	generic (
		addr_width_a  : integer := 8;
		data_width_a  : integer := 8;
		addr_width_b  : integer := 8;
		data_width_b  : integer := 8;
		mem_init_file : string := " "
	);
	PORT
	(
		clock			: in  STD_LOGIC;

		address_a	: in  STD_LOGIC_VECTOR (addr_width_a-1 DOWNTO 0);
		data_a		: in  STD_LOGIC_VECTOR (data_width_a-1 DOWNTO 0) := (others => '0');
		enable_a		: in  STD_LOGIC := '1';
		wren_a		: in  STD_LOGIC := '0';
		q_a			: out STD_LOGIC_VECTOR (data_width_a-1 DOWNTO 0);
		cs_a        : in  std_logic := '1';

		address_b	: in  STD_LOGIC_VECTOR (addr_width_b-1 DOWNTO 0) := (others => '0');
		data_b		: in  STD_LOGIC_VECTOR (data_width_b-1 DOWNTO 0) := (others => '0');
		enable_b		: in  STD_LOGIC := '1';
		wren_b		: in  STD_LOGIC := '0';
		q_b			: out STD_LOGIC_VECTOR (data_width_b-1 DOWNTO 0);
		cs_b        : in  std_logic := '1'
	);
end entity;

ARCHITECTURE SYN OF dpram_dif IS
	function min(a, b : integer) return integer is
	begin
		if a < b then return a; else return b; end if;
	end function;

	constant MIN_WIDTH : integer := min(data_width_a, data_width_b);
	constant RATIO_A   : integer := data_width_a / MIN_WIDTH;
	constant RATIO_B   : integer := data_width_b / MIN_WIDTH;
	constant DEPTH     : integer := (2**addr_width_a) * RATIO_A;

	type mem_t is array(0 to DEPTH-1) of std_logic_vector(MIN_WIDTH-1 downto 0);
	shared variable mem : mem_t := (others => (others => '0'));

	signal q0 : std_logic_vector(data_width_a-1 downto 0);
	signal q1 : std_logic_vector(data_width_b-1 downto 0);
BEGIN
	q_a <= q0 when cs_a = '1' else (others => '1');
	q_b <= q1 when cs_b = '1' else (others => '1');

	process(clock)
		variable base : integer;
	begin
		if rising_edge(clock) then
			if enable_a = '1' then
				base := to_integer(unsigned(address_a)) * RATIO_A;
				for i in 0 to RATIO_A-1 loop
					if wren_a = '1' and cs_a = '1' then
						mem(base + i) := data_a((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH);
					end if;
					q0((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH) <= mem(base + i);
				end loop;
			end if;
		end if;
	end process;

	process(clock)
		variable base : integer;
	begin
		if rising_edge(clock) then
			if enable_b = '1' then
				base := to_integer(unsigned(address_b)) * RATIO_B;
				for i in 0 to RATIO_B-1 loop
					if wren_b = '1' and cs_b = '1' then
						mem(base + i) := data_b((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH);
					end if;
					q1((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH) <= mem(base + i);
				end loop;
			end if;
		end if;
	end process;
END SYN;

--------------------------------------------------------------
-- Dual port RAM, same parameters on both ports
--------------------------------------------------------------

LIBRARY ieee;
USE ieee.std_logic_1164.all;

entity dpram is
	generic (
		addr_width    : integer := 8;
		data_width    : integer := 8;
		mem_init_file : string := " "
	);
	PORT
	(
		clock			: in  STD_LOGIC;

		address_a	: in  STD_LOGIC_VECTOR (addr_width-1 DOWNTO 0);
		data_a		: in  STD_LOGIC_VECTOR (data_width-1 DOWNTO 0) := (others => '0');
		enable_a		: in  STD_LOGIC := '1';
		wren_a		: in  STD_LOGIC := '0';
		q_a			: out STD_LOGIC_VECTOR (data_width-1 DOWNTO 0);
		cs_a        : in  std_logic := '1';

		address_b	: in  STD_LOGIC_VECTOR (addr_width-1 DOWNTO 0) := (others => '0');
		data_b		: in  STD_LOGIC_VECTOR (data_width-1 DOWNTO 0) := (others => '0');
		enable_b		: in  STD_LOGIC := '1';
		wren_b		: in  STD_LOGIC := '0';
		q_b			: out STD_LOGIC_VECTOR (data_width-1 DOWNTO 0);
		cs_b        : in  std_logic := '1'
	);
end entity;

ARCHITECTURE SYN OF dpram IS
BEGIN
	ram : entity work.dpram_dif
	generic map(addr_width, data_width, addr_width, data_width, mem_init_file)
	port map(clock,address_a,data_a,enable_a,wren_a,q_a,cs_a,address_b,data_b,enable_b,wren_b,q_b,cs_b);
END SYN;

--------------------------------------------------------------
-- Dual port RAM, independent widths and clocks on each port
--------------------------------------------------------------

LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

entity dpram_difclk is
	generic (
		addr_width_a  : integer := 8;
		data_width_a  : integer := 8;
		addr_width_b  : integer := 8;
		data_width_b  : integer := 8;
		mem_init_file : string := " "
	);
	PORT
	(
		clock0		: in  STD_LOGIC;
		clock1		: in  STD_LOGIC;

		address_a	: in  STD_LOGIC_VECTOR (addr_width_a-1 DOWNTO 0);
		data_a		: in  STD_LOGIC_VECTOR (data_width_a-1 DOWNTO 0) := (others => '0');
		enable_a		: in  STD_LOGIC := '1';
		wren_a		: in  STD_LOGIC := '0';
		q_a			: out STD_LOGIC_VECTOR (data_width_a-1 DOWNTO 0);
		cs_a        : in  std_logic := '1';

		address_b	: in  STD_LOGIC_VECTOR (addr_width_b-1 DOWNTO 0) := (others => '0');
		data_b		: in  STD_LOGIC_VECTOR (data_width_b-1 DOWNTO 0) := (others => '0');
		enable_b		: in  STD_LOGIC := '1';
		wren_b		: in  STD_LOGIC := '0';
		q_b			: out STD_LOGIC_VECTOR (data_width_b-1 DOWNTO 0);
		cs_b        : in  std_logic := '1'
	);
end entity;

ARCHITECTURE SYN OF dpram_difclk IS
	function min(a, b : integer) return integer is
	begin
		if a < b then return a; else return b; end if;
	end function;

	constant MIN_WIDTH : integer := min(data_width_a, data_width_b);
	constant RATIO_A   : integer := data_width_a / MIN_WIDTH;
	constant RATIO_B   : integer := data_width_b / MIN_WIDTH;
	constant DEPTH     : integer := (2**addr_width_a) * RATIO_A;

	type mem_t is array(0 to DEPTH-1) of std_logic_vector(MIN_WIDTH-1 downto 0);
	shared variable mem : mem_t := (others => (others => '0'));

	signal q0 : std_logic_vector(data_width_a-1 downto 0);
	signal q1 : std_logic_vector(data_width_b-1 downto 0);
BEGIN
	q_a <= q0 when cs_a = '1' else (others => '1');
	q_b <= q1 when cs_b = '1' else (others => '1');

	process(clock0)
		variable base : integer;
	begin
		if rising_edge(clock0) then
			if enable_a = '1' then
				base := to_integer(unsigned(address_a)) * RATIO_A;
				for i in 0 to RATIO_A-1 loop
					if wren_a = '1' and cs_a = '1' then
						mem(base + i) := data_a((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH);
					end if;
					q0((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH) <= mem(base + i);
				end loop;
			end if;
		end if;
	end process;

	process(clock1)
		variable base : integer;
	begin
		if rising_edge(clock1) then
			if enable_b = '1' then
				base := to_integer(unsigned(address_b)) * RATIO_B;
				for i in 0 to RATIO_B-1 loop
					if wren_b = '1' and cs_b = '1' then
						mem(base + i) := data_b((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH);
					end if;
					q1((i+1)*MIN_WIDTH-1 downto i*MIN_WIDTH) <= mem(base + i);
				end loop;
			end if;
		end if;
	end process;
END SYN;
