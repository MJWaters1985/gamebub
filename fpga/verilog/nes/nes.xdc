# Timing exceptions from the MiSTer NES core (NES.sdc). The SDRAM controller
# runs at 4x the master clock; requests from the console, the loader and the
# host (all clk_sys) and the data it returns are handed across with two
# memory clock cycles to spare (multicycle 2 in NES.sdc).
set nes_sdram [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */nes/sdram/*}]
set_max_delay 23 -from [get_clocks clk_sys] -to $nes_sdram
set_max_delay 23 -from $nes_sdram -to [get_clocks clk_sys]
# Set once per game load
set_false_path -from [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */nes/mapper_flags_reg*}]
