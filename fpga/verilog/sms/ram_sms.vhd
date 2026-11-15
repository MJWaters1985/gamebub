-- Portable replacements for the MiSTer SMS core's altsyncram wrappers
-- (mister/spram.vhd, dpram.vhd, sprom.vhd), for Vivado (inferred block RAM)
-- and GHDL, with the same entity names, generics and ports.
--
-- Behaviour matches the altsyncram configuration they replace: address
-- registered, output unregistered (one cycle of read latency), and
-- read-during-write on a port returns the new data. The registered-output
-- options (outdata_reg_*) are honoured. Memory initialisation files aren't
-- supported: the only RAM that has one, the save RAM (nvram_ff.mif, all
-- 0xFF), is filled by the wrapper, and the only ROM (sprom, the boot ROM) is
-- unused here (BIOS off) and reads all ones.

LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

ENTITY spram IS
	GENERIC
	(
		init_file     : string := "none";
		widthad_a     : natural;
		width_a       : natural := 8;
		outdata_reg_a : string := "UNREGISTERED"
	);
	PORT
	(
		address : IN STD_LOGIC_VECTOR (widthad_a-1 DOWNTO 0);
		clock   : IN STD_LOGIC := '1';
		data    : IN STD_LOGIC_VECTOR (width_a-1 DOWNTO 0) := (others => '0');
		wren    : IN STD_LOGIC := '0';
		q       : OUT STD_LOGIC_VECTOR (width_a-1 DOWNTO 0)
	);
END spram;

ARCHITECTURE SYN OF spram IS
	type ram_t is array (0 to 2**widthad_a-1) of std_logic_vector(width_a-1 downto 0);
	signal ram : ram_t;
	signal q0, q1 : std_logic_vector(width_a-1 downto 0);
BEGIN
	process (clock)
	begin
		if rising_edge(clock) then
			if wren = '1' then
				ram(to_integer(unsigned(address))) <= data;
				q0 <= data;
			else
				q0 <= ram(to_integer(unsigned(address)));
			end if;
			q1 <= q0;
		end if;
	end process;
	q <= q1 when outdata_reg_a = "CLOCK0" else q0;
END SYN;

LIBRARY ieee;
USE ieee.std_logic_1164.all;
USE ieee.numeric_std.all;

ENTITY dpram IS
	GENERIC
	(
		init_file       : string := " ";
		widthad_a       : natural;
		width_a         : natural := 8;
		mixed_port_rdwr : string := "DONT_CARE";
		outdata_reg_a   : string := "UNREGISTERED";
		outdata_reg_b   : string := "UNREGISTERED"
	);
	PORT
	(
		address_a : IN STD_LOGIC_VECTOR (widthad_a-1 DOWNTO 0);
		address_b : IN STD_LOGIC_VECTOR (widthad_a-1 DOWNTO 0);
		clock_a   : IN STD_LOGIC;
		clock_b   : IN STD_LOGIC;
		data_a    : IN STD_LOGIC_VECTOR (width_a-1 DOWNTO 0);
		data_b    : IN STD_LOGIC_VECTOR (width_a-1 DOWNTO 0);
		wren_a    : IN STD_LOGIC := '1';
		wren_b    : IN STD_LOGIC := '1';
		q_a       : OUT STD_LOGIC_VECTOR (width_a-1 DOWNTO 0);
		q_b       : OUT STD_LOGIC_VECTOR (width_a-1 DOWNTO 0)
	);
END dpram;

ARCHITECTURE SYN OF dpram IS
	type ram_t is array (0 to 2**widthad_a-1) of std_logic_vector(width_a-1 downto 0);
	shared variable ram : ram_t;
	signal qa0, qa1, qb0, qb1 : std_logic_vector(width_a-1 downto 0);
BEGIN
	process (clock_a)
	begin
		if rising_edge(clock_a) then
			if wren_a = '1' then
				ram(to_integer(unsigned(address_a))) := data_a;
				qa0 <= data_a;
			else
				qa0 <= ram(to_integer(unsigned(address_a)));
			end if;
			qa1 <= qa0;
		end if;
	end process;

	process (clock_b)
	begin
		if rising_edge(clock_b) then
			if wren_b = '1' then
				ram(to_integer(unsigned(address_b))) := data_b;
				qb0 <= data_b;
			else
				qb0 <= ram(to_integer(unsigned(address_b)));
			end if;
			qb1 <= qb0;
		end if;
	end process;

	q_a <= qa1 when outdata_reg_a = "CLOCK0" or outdata_reg_a = "CLOCK1" else qa0;
	q_b <= qb1 when outdata_reg_b = "CLOCK0" or outdata_reg_b = "CLOCK1" else qb0;
END SYN;

LIBRARY ieee;
USE ieee.std_logic_1164.all;

ENTITY sprom IS
	GENERIC
	(
		init_file     : string := "";
		widthad_a     : natural;
		width_a       : natural := 8;
		outdata_reg_a : string := "UNREGISTERED"
	);
	PORT
	(
		address : IN STD_LOGIC_VECTOR (widthad_a-1 DOWNTO 0);
		clock   : IN STD_LOGIC;
		q       : OUT STD_LOGIC_VECTOR (width_a-1 DOWNTO 0)
	);
END sprom;

ARCHITECTURE SYN OF sprom IS
BEGIN
	q <= (others => '1');
END SYN;
