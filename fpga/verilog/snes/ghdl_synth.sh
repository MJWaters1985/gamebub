#!/bin/sh
# Convert the SNES core's VHDL to Verilog with GHDL (for Verilator simulation).
# Usage: ghdl_synth.sh <output directory>
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$1
M=$HERE/mister
mkdir -p "$OUT"
cd "$OUT"

FILES="$HERE/bram.vhd
  $M/65C816/P65816_pkg.vhd $M/65C816/*.vhd
  $M/SPC700/SPC700_pkg.vhd $M/SPC700/*.vhd
  $M/PPU_PKG.vhd $M/DSP_PKG.vhd $M/*.vhd
  $M/chip/DSP/*.vhd $M/chip/SRTC.vhd"

for top in SNES DSP_LHRomMap; do
  # Enhancement chips are left out of the base build.
  GENERICS=""
  [ $top = DSP_LHRomMap ] && GENERICS="-gUSE_DSPn='0'"
  ghdl --synth $GENERICS --std=08 -fsynopsys -frelaxed -Wno-binding --latches --out=verilog \
    $FILES -e $top > $top.v 2> $top.log || { grep -B1 -A3 error $top.log | head -40; exit 1; }
done
python3 "$HERE/ghdl_fixup.py" SNES.v --top SNES
python3 "$HERE/ghdl_fixup.py" DSP_LHRomMap.v --top DSP_LHRomMap --prefix lhrom_ --param "USE_DSPn = 1'b0"
wc -l *.v
