# Building Game Bub firmware and FPGA cores

How this fork's firmware and FPGA cores are built, tested, and packaged into a
`.uf2` update file for a Rev 4 handheld.

## Toolchains

| Part | Host | Tools |
|---|---|---|
| ESP32-S3 firmware | macOS or Linux | `rustup`, `espup` (Xtensa Rust toolchain), `ldproxy`, `espflash` |
| FPGA (Chisel → Verilog) | Linux (x86-64) | OpenJDK 21, `./mill` (bundled launcher) |
| FPGA simulation | Linux | Verilator 5, SDL2, Python 3 with `edalize numpy pillow` |
| FPGA bitstreams | Linux (x86-64) | AMD Vivado 2023.2 (free ML Standard edition, Artix-7 device support only) |
| UF2 packaging | Python ≥ 3.12 | `construct`, ESP-IDF's `components/fatfs` scripts |

Ubuntu packages used: `openjdk-21-jdk-headless verilator libsdl2-dev flex bison
python3-venv rgbds`. The Python packages live in a venv at `fpga/.venv`:

```sh
cd fpga
python3 -m venv .venv
.venv/bin/pip install edalize heatshrink2 numpy pillow construct
```

Vivado can be installed without the GUI: download the 2023.2 Linux web installer
from AMD, then:

```sh
./FPGAs_AdaptiveSoCs_Unified_2023.2_*_Lin64.bin --keep --noexec --target /tmp/vivado-installer
/tmp/vivado-installer/xsetup -b AuthTokenGen          # asks for your AMD account
/tmp/vivado-installer/xsetup -b ConfigGen             # choose Vivado, ML Standard
# edit ~/.Xilinx/install_config.txt: Modules=...Artix-7:1 (all others :0)
/tmp/vivado-installer/xsetup -b Install -a XilinxEULA,3rdPartyEULA -c ~/.Xilinx/install_config.txt
```

On newer distributions (e.g. Ubuntu 24.04+) Vivado 2023.2 fails to start with
`libtinfo.so.5: cannot open shared object file`. Point it at the current
library from inside the Vivado install:

```sh
L=/tools/Xilinx/Vivado/2023.2/lib/lnx64.o
ln -s /usr/lib/x86_64-linux-gnu/libtinfo.so.6 $L/libtinfo.so.5
ln -s /usr/lib/x86_64-linux-gnu/libncurses.so.6 $L/libncurses.so.5
```

## Firmware

```sh
cd firmware/handheld
. ~/export-esp.sh
cargo build --release --features rev4
```

The first build downloads ESP-IDF v5.4.3 into `~/.espressif` (several minutes).

The version shown in **Settings → About** comes from git:
`<Cargo.toml version>.mod-<commit date> (<short commit>)`, e.g.
`1.0.3.mod-20261101 (3711c01)`, with `-dirty` added to the commit when the
working tree has uncommitted changes. Commit before building an update you
intend to keep, so the version identifies the exact code.

## FPGA

Run the unit tests (some upstream tests are known to fail; see below):

```sh
cd fpga
./mill -i GameBub.test
```

Build the three Rev 4 bitstreams (each takes roughly 10–20 minutes):

```sh
source /tools/Xilinx/Vivado/2023.2/settings64.sh
for core in boot:net.gamebub.core.boot.HandheldBoot \
            gameboy:net.gamebub.core.gameboy.HandheldGameboy \
            gba:net.gamebub.core.gba.HandheldGba; do
  .venv/bin/python scripts/build_core.py --name ${core%%:*} --target gamebub_rev4 \
      --build-root build/${core%%:*} --core-class ${core#*:}
done
```

Each build produces `build/<name>/<name>.bit.hs` (the heatshrink-compressed
bitstream the firmware loads).

### Simulation and test ROMs

```sh
.venv/bin/python scripts/build_sim.py gameboy --build-root build/sim-gb
.venv/bin/python scripts/build_sim.py gba --build-root build/sim-gba

# Interactive (needs a display):
build/sim-gba/VSimGba --bios-path gba.bios.bin game.gba
# Headless, with a screenshot after N frames and scripted input:
build/sim-gba/VSimGba --headless --frames 600 --screenshot out.ppm \
    --input 300:start:5,400:a:5 --bios-path gba.bios.bin game.gba
```

To simulate a physical cartridge instead of the emulated one, set these
environment variables when building the GBA simulator:

| Variable | Effect |
|---|---|
| `SIM_PHYSICAL_CART=1` | Runs the cartridge controller in physical mode and serves ROM data from the bus signals like a Game Pak (address latched when nCS falls, incremented when nRD rises, A16–23 live). EEPROM, backup memory and GPIO still come from the emulated cartridge. Prints `ROMCHECK` for reads that don't match the ROM image and `CARTBUS` if A16–23 change during an access. |
| `SIM_SLOW_CART=1` | With `SIM_PHYSICAL_CART=1`: ROM data is only valid once nRD has been low long enough, like a slow cartridge ROM. |

```sh
SIM_PHYSICAL_CART=1 SIM_SLOW_CART=1 .venv/bin/python scripts/build_sim.py gba --build-root build/sim-gba-slow
```

The simulator can't reproduce every real cartridge's timing quirks, so test
cartridge timing changes on hardware too.

`scripts/run_test_roms.py` runs a set of test ROMs and compares screenshots
against reference images. Test ROMs come from
[c-sp/game-boy-test-roms](https://github.com/c-sp/game-boy-test-roms) and
[jsmolka/gba-tests](https://github.com/jsmolka/gba-tests):

```sh
.venv/bin/python scripts/run_test_roms.py gameboy --sim build/sim-gb/VSimGameboy \
    --roms testroms/gb --bios-dmg dmg.bin --bios-cgb cgb.bin
.venv/bin/python scripts/run_test_roms.py gba --sim build/sim-gba/VSimGba \
    --roms testroms/gba-tests --bios-gba gba.bios.bin
```

### Debugging physical GBA cartridges

The GBA core logs physical cartridge ROM reads in the header region (and the
EEPROM range): address, data and a cycle timestamp, for up to 2048 reads after
the core starts. To capture it on the device:

1. Create an empty file `/sdcard/cartlog.enable` on the SD card.
2. Run the cartridge. At the point of interest (e.g. a frozen BIOS logo), open
   the in-game menu: the firmware writes `/sdcard/cartlog.bin`.
3. Copy the log off the SD card (**Tools → USB drive**), then compare it with a
   dump of the cartridge:

```sh
python3 scripts/parse_cart_log.py cartlog.bin game.gba
```

Each read is printed with its cycle gap from the previous read, and reads that
don't match the dump are marked `MISMATCH`. Delete `cartlog.enable` when done.

## Packaging a UF2 update

The system data partition holds the bitstreams and boot ROMs. Start from the
files in an official release (extract its `system_data` partition with ESP-IDF's
`fatfsparse.py`), replace the `.bit.hs` files with newly built ones, then:

```sh
cd firmware/handheld
export IDF_PATH=/path/to/esp-idf
python3 generate_firmware_uf2.py \
    --firmware target/xtensa-esp32s3-espidf/release/handheld \
    --system-data path/to/system_data_dir \
    --hw-revision 4 \
    --output gamebub-rev4.uf2
```

Install it the same way as an official update (see the Game Bub user guide).

## Known test status

- `gba.cpu.ARM7TDMISpec`: 13 of 14 tests fail on upstream code too (stale
  expectations for the CPU's bus signals).
- `gameboy.cpu.CpuSpec` and `platform.handheld.spi.SpiReceiverSpec` abort:
  they use `chiseltest`, which doesn't work with Chisel 7.
