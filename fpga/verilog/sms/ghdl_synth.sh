#!/bin/sh
# Convert the SMS / Game Gear core's VHDL (sms_core.vhd and mister/) to Verilog
# with GHDL, for Verilator simulation. Vivado uses the VHDL directly. The
# core's Verilog modules (jt89, CODES, MC8123, SEGASYS1) stay black boxes for
# Verilator to fill in; the FM chip is a silent stub (sim_opll_stub.vhd).
# Usage: ghdl_synth.sh <output directory>
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$1
M=$HERE/mister
mkdir -p "$OUT"
cd "$OUT"

FILES="$HERE/ram_sms.vhd $HERE/sim_opll_stub.vhd $M/jt89/jt89.vhd
  $M/T80/T80_Pack.vhd $M/T80/T80_Reg.vhd $M/T80/T80_MCode.vhd $M/T80/T80_ALU.vhd $M/T80/T80.vhd $M/T80/T80s.vhd
  $M/vdp_cram.vhd $M/vdp_sprite_shifter.vhd $M/vdp_sprites.vhd $M/vdp_background.vhd $M/vdp_main.vhd $M/vdp.vhd
  $M/io.vhd $M/AudioMix.vhd $M/eeprom_93c46.vhd $M/cart_eeprom.vhd
  $M/evolution_rom_address.vhd $M/evolution_mapper.vhd $M/mapper_detect.vhd $M/mapper_ctrl.vhd
  $M/memory_decode.vhd $M/memory_bus.vhd $M/video.vhd $M/system.vhd $HERE/sms_core.vhd"
FILES=$(for f in $FILES; do [ -f "$f" ] && echo "$f"; done)

ghdl --synth --std=08 -fsynopsys -frelaxed -Wno-binding --latches --out=verilog \
  $FILES -e sms_core > sms_core.v 2> sms_core.log || { grep -B1 -A3 error sms_core.log | head -40; exit 1; }
python3 "$HERE/../snes/ghdl_fixup.py" sms_core.v --top sms_core --prefix smsv_
wc -l *.v
