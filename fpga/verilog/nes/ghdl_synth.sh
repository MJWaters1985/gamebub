#!/bin/sh
# Convert the NES core's VHDL (the T65 CPU and the VRC7 sound chip) to Verilog
# with GHDL, for Verilator simulation. Vivado uses the VHDL directly.
# Usage: ghdl_synth.sh <output directory>
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
FIXUP=$HERE/../snes/ghdl_fixup.py
OUT=$1
M=$HERE/mister
mkdir -p "$OUT"
cd "$OUT"

T65="$M/bus_savestates.vhd $M/t65/T65_Pack.vhd $M/t65/T65_MCode.vhd $M/t65/T65_ALU.vhd $M/t65/T65.vhd"
OPLL="$M/SOUND/OPLL/VM2413/vm2413.vhd $(ls $M/SOUND/OPLL/VM2413/*.vhd | grep -v vm2413.vhd) $M/SOUND/OPLL/eseopll.vhd"

synth() {
  std=$1; top=$2; shift 2
  ghdl --synth --std=$std -fsynopsys -fexplicit -frelaxed -Wno-binding --latches --out=verilog \
    "$@" -e $top > $top.v 2> $top.log || { grep -B1 -A3 error $top.log | head -40; exit 1; }
}

# nes.v connects T65's ports in a mix of VHDL and lowercase spellings.
synth 08 T65 $T65
python3 "$FIXUP" T65.v --top T65 --prefix t65_ --keep-case \
  --rename Mode=mode --rename Res_n=res_n --rename Pwr_n=pwr_n \
  --rename Clk=clk --rename Enable=enable --rename Rdy=rdy

# VM2413 uses names that are VHDL-2008 keywords (release).
synth 93c eseopll $OPLL
python3 "$FIXUP" eseopll.v --top eseopll --prefix opll_
wc -l *.v
