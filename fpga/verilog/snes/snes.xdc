# Timing exceptions from the MiSTer SNES core (SNES.sdc). The SDRAM controller
# runs at 4x the master clock, but the console only samples its outputs (and
# changes its inputs) on master clock edges a few memory clocks apart.
set snes_sdram [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */snes/sdram/*}]
set snes_main [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */snes/main/*}]
set_max_delay 20 -from $snes_main -to $snes_sdram
set_max_delay 23 -from $snes_sdram -to $snes_main
