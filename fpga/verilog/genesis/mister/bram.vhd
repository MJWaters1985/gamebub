-- Portable replacements for the MiSTer Genesis core's altsyncram wrappers,
-- for Vivado (inferred block RAM) and GHDL.
--
-- Behaviour matches the altsyncram configuration they replace:
--   * address registered, output unregistered (one cycle of read latency)
--   * read-during-write on the same port returns the new data
--   * `enable` gates the port's clock (address and data registers)
--   * a port's output reads all ones while its `cs` is low
-- Ports A and B of dpram_dif must have the same widths (the core's only
-- mixed-width RAM, the save RAM, is a dpram in this port). The single-port
-- RAMs (spram, spram_sz) and memory initialisation files are unused and left
-- out.

--------------------------------------------------------------
-- Dual port Block RAM, different parameters on ports (same widths here)
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
	type ram_t is array (0 to 2**addr_width_a-1) of std_logic_vector(data_width_a-1 downto 0);
	shared variable ram : ram_t;
	signal q0 : std_logic_vector((data_width_a - 1) downto 0);
	signal q1 : std_logic_vector((data_width_b - 1) downto 0);
BEGIN
	assert addr_width_a = addr_width_b and data_width_a = data_width_b
		report "dpram_dif: mixed-width ports aren't supported" severity failure;

	q_a <= q0 when cs_a = '1' else (others => '1');
	q_b <= q1 when cs_b = '1' else (others => '1');

	process (clock)
	begin
		if rising_edge(clock) then
			if enable_a = '1' then
				if wren_a = '1' then
					ram(to_integer(unsigned(address_a))) := data_a;
					q0 <= data_a;
				else
					q0 <= ram(to_integer(unsigned(address_a)));
				end if;
			end if;
		end if;
	end process;

	process (clock)
	begin
		if rising_edge(clock) then
			if enable_b = '1' then
				if wren_b = '1' then
					ram(to_integer(unsigned(address_b))) := data_b;
					q1 <= data_b;
				else
					q1 <= ram(to_integer(unsigned(address_b)));
				end if;
			end if;
		end if;
	end process;
END SYN;

-- (dpram, used only by the SystemVerilog modules, is in ../bram_genesis.sv.)

--------------------------------------------------------------
-- Dual port Block RAM same parameters on both ports (for VDP)
--------------------------------------------------------------
LIBRARY ieee;
USE ieee.std_logic_1164.all;

entity DualPortRAM is
	generic (
		addrbits    : integer := 8;
		databits    : integer := 8
	);
	PORT
	(
		clock			: in  STD_LOGIC;

		address_a	: in  STD_LOGIC_VECTOR (addrbits-1 DOWNTO 0);
		data_a		: in  STD_LOGIC_VECTOR (databits-1 DOWNTO 0) := (others => '0');
		wren_a		: in  STD_LOGIC := '0';
		q_a			: out STD_LOGIC_VECTOR (databits-1 DOWNTO 0);

		address_b	: in  STD_LOGIC_VECTOR (addrbits-1 DOWNTO 0) := (others => '0');
		data_b		: in  STD_LOGIC_VECTOR (databits-1 DOWNTO 0) := (others => '0');
		wren_b		: in  STD_LOGIC := '0';
		q_b			: out STD_LOGIC_VECTOR (databits-1 DOWNTO 0)
	);
end entity;

ARCHITECTURE SYN OF DualPortRAM IS
BEGIN
	ram : entity work.dpram_dif generic map(addrbits,databits,addrbits,databits)
	port map(clock,address_a,data_a,'1',wren_a,q_a,'1',address_b,data_b,'1',wren_b,q_b,'1');
END SYN;

--------------------------------------------------------------
-- Dual port Block RAM with byte enable (for VDP)
--------------------------------------------------------------
LIBRARY ieee;
USE ieee.std_logic_1164.all;

ENTITY obj_cache IS
	PORT
	(
		byteena_a	: IN STD_LOGIC_VECTOR (3 DOWNTO 0) :=  (OTHERS => '1');
		clock		: IN STD_LOGIC  := '1';
		data		: IN STD_LOGIC_VECTOR (31 DOWNTO 0);
		rdaddress	: IN STD_LOGIC_VECTOR (6 DOWNTO 0);
		wraddress	: IN STD_LOGIC_VECTOR (6 DOWNTO 0);
		wren		: IN STD_LOGIC  := '0';
		q			: OUT STD_LOGIC_VECTOR (31 DOWNTO 0)
	);
END obj_cache;

ARCHITECTURE SYN OF obj_cache IS
	signal we : std_logic_vector(3 downto 0);
BEGIN
	we <= byteena_a when wren = '1' else "0000";

	gen : for i in 0 to 3 generate
		ram : entity work.dpram_dif generic map(7,8,7,8)
		port map
		(
			clock, rdaddress, (others => '0'), '1', '0', q(8*i+7 downto 8*i), '1',
			wraddress, data(8*i+7 downto 8*i), '1', we(i), open, '1'
		);
	end generate;
END SYN;
