import argparse
import os
import subprocess
from pathlib import Path
from edalize.edatool import get_edatool
import os.path
import shutil
from dataclasses import dataclass

ROOT_PATH = Path(__file__).resolve().parent.parent
MILL_PATH = ROOT_PATH / "mill"


@dataclass
class Sim:
    name: str
    # Chisel class generating the top module, or None for a Verilog-only sim
    main_class: str | None
    top_module: str
    files: list[str]
    # Verilog/SystemVerilog sources, relative to the FPGA root
    hdl_files: tuple[str, ...] = ()
    # Script converting VHDL sources to Verilog, run with an output directory
    vhdl_convert: str | None = None


COMMON_FILES: list[str] = [
    "sim/audio.hpp",
    "sim/common.hpp",
    "sim/framebuffer.hpp",
    "sim/input.hpp",
    "sim/window.hpp",
    "sim/recording_writer.hpp",
    "sim/audio.cpp",
    "sim/framebuffer.cpp",
    "sim/main.cpp",
    "sim/window.cpp",
    "sim/recording_writer.cpp",
]

SIMS: list[Sim] = [
    Sim(
        name="gameboy",
        main_class="net.gamebub.core.gameboy.SimGameboy",
        top_module="SimGameboy",
        files=[
            "sim/gb/cartridge.hpp",
            "sim/gb/simulator.hpp",
            "sim/gb/cartridge.cpp",
            "sim/gb/simulator.cpp",
        ],
    ),
    Sim(
        name="gba",
        main_class="net.gamebub.core.gba.SimGba",
        top_module="SimGba",
        files=[
            "sim/gba/cartridge.hpp",
            "sim/gba/simulator.hpp",
            "sim/gba/cartridge.cpp",
            "sim/gba/simulator.cpp",
        ],
    ),
    Sim(
        name="snes",
        main_class=None,
        top_module="snes_top",
        files=[
            "sim/snes/cartridge.hpp",
            "sim/snes/simulator.hpp",
            "sim/snes/cartridge.cpp",
            "sim/snes/simulator.cpp",
        ],
        hdl_files=(
            "verilog/snes/snes_top.sv",
            "verilog/snes/sdram_snes.sv",
            "verilog/snes/codes_stub.v",
            "verilog/snes/chip_stubs_sim.v",
            "verilog/snes/mister/main.v",
            "verilog/snes/mister/ioport.sv",
        ),
        vhdl_convert="verilog/snes/ghdl_synth.sh",
    ),
]


def build(
    sim: Sim,
    build_root: Path,
):
    files = []
    if build_root.exists():
        shutil.rmtree(build_root)
    build_root.mkdir(parents=True, exist_ok=True)

    generate_root = build_root / "generated"
    if sim.main_class:
        # Run Chisel
        args = [
            MILL_PATH,
            "-i",
            "GameBub.runMain",
            sim.main_class,
            f"--target-dir={generate_root}",
        ]
        subprocess.run(args)

        # Collect files from Chisel output
        filelist_path = generate_root / "filelist.f"
        with open(filelist_path) as f:
            for filename in f:
                path = filelist_path.parent / filename.rstrip()
                files.append(dict(name=path, file_type="systemVerilogSource"))

    for file in sim.hdl_files:
        file_type = "verilogSource" if file.endswith(".v") else "systemVerilogSource"
        files.append(dict(name=ROOT_PATH / file, file_type=file_type))

    if sim.vhdl_convert:
        vhdl_root = build_root / "vhdl"
        subprocess.run([ROOT_PATH / sim.vhdl_convert, vhdl_root], check=True)
        for path in sorted(vhdl_root.glob("*.v")):
            files.append(dict(name=path, file_type="verilogSource"))

    # Add additional simulator files
    sim_files = COMMON_FILES + sim.files
    for file in sim_files:
        file_extension = Path(file).suffix
        path = ROOT_PATH / file
        info = dict(name=path, file_type="cppSource")
        if file_extension == ".hpp":
            info["is_include_file"] = True
        files.append(info)

    # Make all file paths relative to build root.
    for f in files:
        f["name"] = os.path.relpath(str(f["name"]), str(build_root))

    sdl_cflags = (
        subprocess.check_output(["pkg-config", "sdl2", "--cflags"]).decode().strip()
    )
    sdl_libs = (
        subprocess.check_output(["pkg-config", "sdl2", "--libs"]).decode().strip()
    )
    options = [
        f'-CFLAGS "{sdl_cflags}"',
        f'-LDFLAGS "{sdl_libs}"',
        "-Wno-WIDTHEXPAND",
    ]
    if sim.vhdl_convert:
        # MiSTer/GHDL Verilog uses SystemVerilog keywords (`do`) as names, and
        # the placeholders for disabled modules have no matching pins.
        options += [
            "+1364-2005ext+v",
            "-Wno-fatal",
            "-Wno-PINNOTFOUND",
            "-Wno-LATCH",
            "-Wno-MULTIDRIVEN",
            "-Wno-UNOPTFLAT",
            "-CFLAGS -O2",
        ]

    edam = {
        "name": sim.name,
        "toplevel": sim.top_module,
        "files": files,
        "tool_options": {"verilator": {"verilator_options": options}},
    }
    backend = get_edatool("verilator")(edam=edam, work_root=build_root)
    backend.configure()
    backend.build()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "sim",
        choices=[s.name for s in SIMS],
    )
    parser.add_argument("--build-root", required=True, type=Path)

    args = parser.parse_args()
    sim = next(s for s in SIMS if s.name == args.sim)
    build(sim, args.build_root)


if __name__ == "__main__":
    main()
