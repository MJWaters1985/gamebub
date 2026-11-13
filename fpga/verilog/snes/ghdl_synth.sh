#!/bin/sh
# Convert the SNES core's VHDL to Verilog with GHDL (for Verilator simulation).
# The enhancement chips are all converted (DSP_LHRomMap with the DSPn chip);
# which ones are used is chosen by snes_top's parameters.
# Usage: ghdl_synth.sh <output directory>
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$1
M=$HERE/mister
mkdir -p "$OUT"
cd "$OUT"

FILES="$HERE/rom_init_pkg.vhd $HERE/bram.vhd $HERE/chip_ip.vhd
  $M/65C816/P65816_pkg.vhd $M/65C816/*.vhd
  $M/SPC700/SPC700_pkg.vhd $M/SPC700/*.vhd
  $M/PPU_PKG.vhd $M/DSP_PKG.vhd $M/*.vhd
  $M/chip/DSP/*.vhd $M/chip/SRTC.vhd
  $M/chip/CX4/CX4.vhd $M/chip/CX4/CX4Map.vhd
  $M/chip/GSU/GSU_PKG.vhd $M/chip/GSU/GSU.vhd $M/chip/GSU/GSUMap.vhd
  $M/chip/SA1/SA1.vhd $M/chip/SA1/SA1Map.vhd
  $M/chip/SDD1/*.vhd"

# top module, module name prefix for its submodules
for spec in SNES: DSP_LHRomMap:lhrom_ CX4Map:cx4_ GSUMap:gsu_ SA1Map:sa1_ SDD1Map:sdd1_; do
  top=${spec%%:*}
  prefix=${spec#*:}
  ghdl --synth --std=08 -fsynopsys -frelaxed -Wno-binding --latches --out=verilog \
    $FILES -e $top > $top.v 2> $top.log || { grep -B1 -A3 error $top.log | head -40; exit 1; }
  if [ -n "$prefix" ]; then
    # main.v leaves the chip maps' ENABLE input (VHDL default '1') unconnected.
    python3 "$HERE/ghdl_fixup.py" $top.v --top $top --prefix $prefix --tie "enable=1'b1"
  else
    python3 "$HERE/ghdl_fixup.py" $top.v --top $top
  fi
done
# main.v sets DSP_LHRomMap's USE_DSPn parameter (the conversion always includes DSPn).
python3 - <<'EOF'
p = "DSP_LHRomMap.v"
s = open(p).read().replace("module DSP_LHRomMap\n", "module DSP_LHRomMap #(parameter USE_DSPn = 1'b1)\n", 1)
open(p, "w").write(s)
EOF
wc -l *.v
