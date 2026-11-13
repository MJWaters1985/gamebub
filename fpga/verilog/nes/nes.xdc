# Timing exceptions from the MiSTer NES core (NES.sdc). The SDRAM controller
# runs at 4x the master clock; the core and the controller hand data across
# with two memory clock cycles to spare (multicycle 2 in NES.sdc).
set nes_sdram [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */nes/sdram/*}]
set nes_core [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */nes/nes/*}]
set_max_delay 23 -from $nes_core -to $nes_sdram
set_max_delay 23 -from $nes_sdram -to $nes_core
# Set once per game load
set_false_path -from [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */nes/mapper_flags_reg*}]
