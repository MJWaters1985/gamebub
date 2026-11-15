-- The MiSTer SMS / Game Gear core (mister/system.vhd) with its clock enables,
-- video timing, work RAM and save RAM, for the Game Bub wrapper (sms_top.sv).
-- In VHDL so the core's many optional ports (save states, arcade System E,
-- SC-3000 keyboard, ...) keep their default values. Based on the core's MiSTer
-- top level (SMS.sv).

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity sms_core is
	port (
		clk       : in  std_logic;  -- 53.69 MHz
		reset     : in  std_logic;
		enable    : in  std_logic;  -- 0 pauses the console
		gg        : in  std_logic;  -- Game Gear
		pal       : in  std_logic;
		region    : in  std_logic;  -- 1 = Japan

		-- Game loading (byte stream)
		dl_en     : in  std_logic;
		dl_wr     : in  std_logic;
		dl_addr   : in  std_logic_vector(24 downto 0);
		dl_data   : in  std_logic_vector(7 downto 0);

		-- Cartridge ROM (SDRAM), one read per request
		rom_rd    : out std_logic;
		rom_a     : out std_logic_vector(23 downto 0);
		rom_do    : in  std_logic_vector(7 downto 0);
		ce_cpu_o  : out std_logic;  -- CPU clock phase, also while paused (SDRAM cycle)

		-- Controller (active high)
		up, down, left, right, b1, b2, start : in std_logic;

		-- Save RAM, for the host (one cycle of read latency)
		nv_addr   : in  std_logic_vector(14 downto 0);
		nv_we     : in  std_logic;
		nv_din    : in  std_logic_vector(7 downto 0);
		nv_dout   : out std_logic_vector(7 downto 0);
		nv_used   : out std_logic;  -- the game has written its save RAM since loading

		-- Video: one pixel per ce_pix outside blanking
		color     : out std_logic_vector(11 downto 0);  -- {B, G, R}, 4 bits each
		ce_pix_o  : out std_logic;
		hblank    : out std_logic;
		vblank    : out std_logic;

		audio_l   : out std_logic_vector(15 downto 0);
		audio_r   : out std_logic_vector(15 downto 0)
	);
end sms_core;

architecture rtl of sms_core is
	signal clkd : unsigned(4 downto 0) := (others => '0');
	signal ce_cpu, ce_vdp, ce_pix, ce_sp, ce_cpu_free : std_logic := '0';

	signal x, y : std_logic_vector(8 downto 0);
	signal vcounter_cpu : std_logic_vector(7 downto 0);
	signal mask_column : std_logic;
	signal smode_M1, smode_M2, smode_M3, smode_M4 : std_logic;

	signal ram_a  : std_logic_vector(13 downto 0);
	signal ram_d, ram_q : std_logic_vector(7 downto 0);
	signal ram_we : std_logic;

	signal nvram_a : std_logic_vector(14 downto 0);
	signal nvram_d, nvram_q : std_logic_vector(7 downto 0);
	signal nvram_we : std_logic;

	-- RAM clearing while a game loads: work RAM to 0x00, save RAM to 0xFF
	signal clr_addr : unsigned(14 downto 0) := (others => '0');

	signal ram_addr_mux : std_logic_vector(13 downto 0);
	signal ram_data_mux : std_logic_vector(7 downto 0);
	signal nv_addr_mux  : std_logic_vector(14 downto 0);
	signal nv_data_mux  : std_logic_vector(7 downto 0);

	signal dbr : std_logic := '0';
	signal reset_n : std_logic;
begin
	-- Clock enables (SMS.sv): CPU 3.58 MHz, VDP 10.7 MHz, pixels 5.37 MHz.
	-- They stop while paused.
	process (clk)
	begin
		if falling_edge(clk) then
			-- The divider keeps running while paused (the SDRAM controller's cycle is
			-- aligned to its CPU enable); only the console's enables stop.
			ce_sp  <= clkd(0) and enable;
			ce_vdp <= '0';
			ce_pix <= '0';
			ce_cpu <= '0';
			ce_cpu_free <= '0';
			clkd <= clkd + 1;
			case to_integer(clkd) is
				when 29 => clkd <= (others => '0'); ce_vdp <= enable; ce_pix <= enable;
				when 24 => ce_cpu <= enable; ce_cpu_free <= '1'; ce_vdp <= enable;
				when 19 => ce_vdp <= enable; ce_pix <= enable;
				when 14 => ce_vdp <= enable;
				when 9  => ce_cpu <= enable; ce_cpu_free <= '1'; ce_vdp <= enable; ce_pix <= enable;
				when 4  => ce_vdp <= enable;
				when others => null;
			end case;
		end if;
	end process;
	ce_cpu_o <= ce_cpu_free;
	ce_pix_o <= ce_pix;

	process (clk)
	begin
		if rising_edge(clk) then
			if dl_en = '1' then
				clr_addr <= clr_addr + 1;
				dbr <= '1';
				nv_used <= '0';
			else
				clr_addr <= (others => '0');
				if nvram_we = '1' then
					nv_used <= '1';
				end if;
			end if;
		end if;
	end process;

	reset_n <= not (reset or dl_en);

	ram_addr_mux <= std_logic_vector(clr_addr(13 downto 0)) when dl_en = '1' else ('0' & ram_a(12 downto 0));
	ram_data_mux <= x"00" when dl_en = '1' else ram_d;
	nv_addr_mux  <= std_logic_vector(clr_addr) when dl_en = '1' else nv_addr;
	nv_data_mux  <= x"FF" when dl_en = '1' else nv_din;

	system_inst : entity work.system
	generic map (MAX_SPPL => 63)
	port map (
		clk_sys    => clk,
		ce_cpu     => ce_cpu,
		ce_vdp     => ce_vdp,
		ce_pix     => ce_pix,
		ce_sp      => ce_sp,
		turbo      => '0',
		gg         => gg,
		ggres      => gg,
		systeme    => '0',
		bios_en    => '0',
		ext_bios_sel => '0',
		ext_bios_loaded => '0',
		gg_bios_en => '0',
		ext_gg_bios_loaded => '0',
		GG_BIOSWEN => '0',

		GG_EN      => '0',
		GG_CODE    => (others => '0'),
		GG_RESET   => '0',
		GG_AVAIL   => open,
		gg_link_en => '0',
		gg_link_in => (others => '0'),
		gg_link_out => open,

		RESET_n    => reset_n,

		rom_rd     => rom_rd,
		rom_a      => rom_a,
		rom_do     => rom_do,

		j1_up      => not up,
		j1_down    => not down,
		j1_left    => not left,
		j1_right   => not right,
		j1_tl      => not b1,
		j1_tr      => not b2,
		j1_th      => '1',
		j1_start   => not start,
		j1_coin    => '1',
		j1_a3      => '1',
		j2_up      => '1',
		j2_down    => '1',
		j2_left    => '1',
		j2_right   => '1',
		j2_tl      => '1',
		j2_tr      => '1',
		j2_th      => '1',
		j2_start   => '1',
		j2_coin    => '1',
		j2_a3      => '1',
		-- The Master System's Pause button, which is also the Game Gear's Start
		-- (port $00 bit 7), as in SMS.sv
		pause      => not start,
		soft_reset => '0',

		E0Type     => "00",
		E1Use      => '0',
		E2Use      => '0',
		E0         => (others => '1'),
		F2         => (others => '1'),
		F3         => (others => '1'),

		has_paddle => '0',
		has_pedal  => '0',
		paddle     => (others => '0'),
		paddle2    => (others => '0'),
		pedal      => (others => '0'),
		sc3000_en  => '0',
		sc_multicart_en => '0',
		sc_megacart_en  => '0',
		sc_cart_ram => "00",
		sk1100_en  => '0',
		sk1100_row_sel => open,
		sk1100_row_data => (others => '1'),

		j1_tr_out  => open,
		j1_th_out  => open,
		j2_tr_out  => open,
		j2_th_out  => open,

		x          => x,
		y          => y,
		vcounter_cpu => vcounter_cpu,
		color      => color,
		palettemode => '0',
		mask_column => mask_column,
		black_column => '0',
		smode_M1   => smode_M1,
		smode_M2   => smode_M2,
		smode_M3   => smode_M3,
		smode_M4   => smode_M4,
		ysj_quirk  => '0',
		pal        => pal,
		region     => region,
		mapper_lock => '0',
		mapper_codies_force => '0',
		mapper_dahjee_a_force => '0',
		mapper_linear_force => '0',
		mapper_zemina_force => '0',
		mapper_evolution_force => '0',
		evolution_gg_active => open,
		evolution_active => open,
		mapper_eeprom_out => open,
		vdp_enables => "00",
		psg_enables => "00",

		audioL     => audio_l,
		audioR     => audio_r,
		fm_ena     => '1',

		dbr        => dbr,
		sp64       => '0',

		ram_a      => ram_a,
		ram_d      => ram_d,
		ram_we     => ram_we,
		ram_q      => ram_q,

		nvram_a    => nvram_a,
		nvram_d    => nvram_d,
		nvram_we   => nvram_we,
		nvram_q    => nvram_q,

		encrypt    => "00",
		key_a      => open,
		key_d      => (others => '0'),

		ROMCL      => clk,
		ROMAD      => dl_addr,
		ROMDT      => dl_data,
		ROMEN      => dl_wr,
		BIOSWEN    => '0',

		z80_reg_out => open,
		vdp_regs_out => open,
		vdp_cram_out => open,
		ss_vram_D  => open,
		psg_out    => open,
		psg_ext_out => open,
		psg2_ext_out => open,
		psg_div_out => open,
		audio_control_out => open,
		audio_quiescent => open,
		audio_phase_out => open,
		mapper_out => open,
		evolution_ss_out => open,
		eeprom_ss_out => open,
		z80_m1_n   => open,
		z80_mreq_n => open,
		z80_iset   => open,
		vdp2_regs_out => open,
		vdp2_cram_out => open,
		ss_vram2_D => open,
		psg2_out   => open,
		io_state_out => open
	);

	video_inst : entity work.video
	port map (
		clk          => clk,
		ce_pix       => ce_pix,
		pal          => pal,
		ggres        => gg,
		border       => '0',
		mask_column  => mask_column,
		cut_mask     => '0',
		smode_M1     => smode_M1,
		smode_M2     => smode_M2,
		smode_M3     => smode_M3,
		smode_M4     => smode_M4,
		video_state_out => open,
		x            => x,
		y            => y,
		vcounter_cpu => vcounter_cpu,
		hsync        => open,
		vsync        => open,
		hblank       => hblank,
		vblank       => vblank
	);

	-- Work RAM (16 KB for the SC-3000; 8 KB used), cleared while loading
	ram_inst : entity work.spram
	generic map (widthad_a => 14)
	port map (
		clock   => clk,
		address => ram_addr_mux,
		wren    => dl_en or ram_we,
		data    => ram_data_mux,
		q       => ram_q
	);

	-- Save RAM (32 KB): the console on port A, the host on port B (filled with
	-- 0xFF while loading, before the host loads the save)
	nvram_inst : entity work.dpram
	generic map (widthad_a => 15)
	port map (
		clock_a   => clk,
		address_a => nvram_a,
		data_a    => nvram_d,
		wren_a    => nvram_we,
		q_a       => nvram_q,
		clock_b   => clk,
		address_b => nv_addr_mux,
		data_b    => nv_data_mux,
		wren_b    => dl_en or nv_we,
		q_b       => nv_dout
	);
end rtl;
