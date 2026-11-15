# Timing exceptions from the MiSTer Genesis core (Genesis.sdc, FX68K/fx68k.sdc).

# SDRAM read data is taken by the console two master clock cycles after the
# controller (at 2x the master clock) returns it.
set gen_sdram_dout [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/sdram/dout_reg*}]
set gen_bus_data [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/data_reg*}]
set_multicycle_path -setup -end 2 -from $gen_sdram_dout -to $gen_bus_data
set_multicycle_path -hold -end 1 -from $gen_sdram_dout -to $gen_bus_data

# FX68K: the microcode and nanocode addresses, and the condition codes, settle
# over two clock cycles (the CPU's phases are enables on the master clock).
set fx_ir [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/Ir_reg*}]
set fx_uaddr [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/microAddr_reg*}]
set fx_naddr [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/nanoAddr_reg*}]
set fx_nlatch [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/nanoLatch_reg*}]
set fx_oper [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/excUnit/alu/oper_reg*}]
set fx_ccr [get_cells -hierarchical -filter {IS_SEQUENTIAL && NAME =~ */genesis/system/M68K/excUnit/alu/pswCcr_reg*}]
set_multicycle_path -start -setup 2 -from $fx_ir -to [concat $fx_uaddr $fx_naddr]
set_multicycle_path -start -hold 1 -from $fx_ir -to [concat $fx_uaddr $fx_naddr]
set_multicycle_path -start -setup 2 -from [concat $fx_nlatch $fx_oper] -to $fx_ccr
set_multicycle_path -start -hold 1 -from [concat $fx_nlatch $fx_oper] -to $fx_ccr
