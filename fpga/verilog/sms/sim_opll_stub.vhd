-- Simulation-only stand-in for the VM2413 FM sound chip (mister/VM2413), which
-- GHDL can't convert together with the rest of the core (it uses VHDL-2008
-- keywords as names). FM sound is silent in the simulator.
library ieee;
use ieee.std_logic_1164.all;

entity opll is
    port(
        xin         : in  std_logic;
        xout        : out std_logic;
        xena        : in  std_logic;
        d           : in  std_logic_vector( 7 downto 0 );
        a           : in  std_logic;
        cs_n        : in  std_logic;
        we_n        : in  std_logic;
        ic_n        : in  std_logic;
        mixout      : out std_logic_vector(13 downto 0 )
    );
end opll;

architecture stub of opll is
begin
    xout <= xin;
    mixout <= (others => '0');
end stub;
