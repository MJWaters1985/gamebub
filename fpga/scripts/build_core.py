import argparse
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from edalize.edatool import get_edatool
import os.path
import struct
import sys
import shutil

ROOT_PATH = Path(__file__).resolve().parent.parent
MILL_PATH = ROOT_PATH / "mill"


@dataclass
class Target:
    name: str
    main_class: str
    chisel_args: list[str]
    verilog_defines: list[str]
    files: list[str]
    top_module: str
    part: str


# Generate Target list
TARGETS = []
for r in [1, 2, 3, 4]:
    TARGETS.append(
        Target(
            name=f"gamebub_rev{r}",
            main_class="platform.handheld.HandheldTop",
            chisel_args=[str(r)],
            verilog_defines=[f"BOARD_REV_{r}"],
            files=[
                f"verilog/handheld/rev_{r}.xdc",
                # Common
                "verilog/handheld/common.xdc",
                "verilog/handheld/top.sv",
                "verilog/handheld/pll_reset_generator.sv",
                "verilog/handheld/clk_wiz_hdmi.v",
                "verilog/picorv32.v",
                # HDMI
                "verilog/hdmi/src/audio_clock_regeneration_packet.sv",
                "verilog/hdmi/src/audio_info_frame.sv",
                "verilog/hdmi/src/audio_sample_packet.sv",
                "verilog/hdmi/src/auxiliary_video_information_info_frame.sv",
                "verilog/hdmi/src/hdmi.sv",
                "verilog/hdmi/src/packet_assembler.sv",
                "verilog/hdmi/src/packet_picker.sv",
                "verilog/hdmi/src/serializer.sv",
                "verilog/hdmi/src/source_product_description_info_frame.sv",
                "verilog/hdmi/src/tmds_channel.sv",
            ],
            top_module="top_handheld",
            part="xc7a100tcsg324-1",
        )
    )


_SNES = "verilog/snes"
_SNES_MISTER = f"{_SNES}/mister"

# Extra HDL sources for cores that wrap external HDL (keyed by core class).
CORE_FILES: dict[str, list[str]] = {
    "net.gamebub.core.snes.HandheldSnes": [
        f"{_SNES}/snes.xdc",
        f"{_SNES}/snes_top.sv",
        f"{_SNES}/sdram_snes.sv",
        f"{_SNES}/codes_stub.v",
        f"{_SNES}/rom_init_pkg.vhd",
        f"{_SNES}/bram.vhd",
        f"{_SNES}/chip_ip.vhd",
        f"{_SNES_MISTER}/main.v",
        f"{_SNES_MISTER}/ioport.sv",
        *[
            f"{_SNES_MISTER}/{name}.vhd"
            for name in [
                "65C816/P65816_pkg", "65C816/ALU", "65C816/AddSubBCD", "65C816/AddrGen",
                "65C816/BCDAdder", "65C816/MCode", "65C816/P65C816",
                "SPC700/SPC700_pkg", "SPC700/ALU", "SPC700/AddSub", "SPC700/AddrGen",
                "SPC700/BCDAdj", "SPC700/MCode", "SPC700/MulDiv", "SPC700/SPC700",
                "PPU_PKG", "DSP_PKG", "CEGen", "CPU", "DSP", "PPU", "SMP", "SNES", "SWRAM",
                "chip/SRTC", "chip/DSP/OBC1", "chip/DSP/DSPn", "chip/DSP/DSP_LHRomMap",
                "chip/CX4/CX4", "chip/CX4/CX4Map",
                "chip/GSU/GSU_PKG", "chip/GSU/GSU", "chip/GSU/GSUMap",
                "chip/SA1/SA1", "chip/SA1/SA1Map",
                "chip/SDD1/Decoder", "chip/SDD1/InputMgr", "chip/SDD1/SDD1", "chip/SDD1/SDD1Map",
            ]
        ],
    ],
}
CORE_FILES["net.gamebub.core.snes.HandheldSnesSdd1"] = CORE_FILES["net.gamebub.core.snes.HandheldSnes"]

_NES = "verilog/nes"
_NES_MISTER = f"{_NES}/mister"
CORE_FILES["net.gamebub.core.nes.HandheldNes"] = [
    f"{_NES}/nes.xdc",
    f"{_NES_MISTER}/regs_savestates.sv",
    f"{_NES}/nes_top.sv",
    f"{_NES}/sdram_nes.sv",
    f"{_NES}/bram_nes.sv",
    *[
        f"{_NES_MISTER}/{name}"
        for name in [
            # (compat.v, Xilinx primitive models for Quartus, is simulation-only here)
            "gameloader.sv", "nes.v", "cart.sv", "ppu.sv", "apu.sv", "video.sv",
            "EEPROM_24C0x.sv", "cheatcodes.sv", "composite_board.sv", "miracle.sv",
            "zapper.sv", "keyboard.sv", "sys/iir_filter.v",
            "mappers/FDS.sv", "mappers/JYCompany.sv", "mappers/MMC1.sv", "mappers/MMC2.sv",
            "mappers/MMC3.sv", "mappers/MMC5.sv", "mappers/Mapper99.sv", "mappers/Namco.sv",
            "mappers/Sachen.sv", "mappers/Sunsoft.sv", "mappers/VRC.sv",
            "mappers/generic.sv", "mappers/misc.sv",
            "bus_savestates.vhd", "savestates.vhd", "statemanager.vhd",
            "t65/T65_Pack.vhd", "t65/T65_MCode.vhd", "t65/T65_ALU.vhd", "t65/T65.vhd",
            "SOUND/OPLL/VM2413/vm2413.vhd",
            *[
                f"SOUND/OPLL/VM2413/{n}.vhd"
                for n in [
                    "attacktable", "controller", "envelopegenerator", "envelopememory",
                    "feedbackmemory", "lineartable", "operator", "opll", "outputgenerator",
                    "outputmemory", "phasegenerator", "phasememory", "registermemory",
                    "sinetable", "slotcounter", "temporalmixer", "voicememory", "voicerom",
                ]
            ],
            "SOUND/OPLL/eseopll.vhd",
        ]
    ],
]


_GEN = "verilog/genesis"
_GEN_MISTER = f"{_GEN}/mister"
CORE_FILES["net.gamebub.core.genesis.HandheldGenesis"] = [
    f"{_GEN}/genesis.xdc",
    f"{_GEN}/genesis_top.sv",
    f"{_GEN}/bram_genesis.sv",
    *[
        f"{_GEN_MISTER}/{name}"
        for name in [
            "system.sv", "sdram.sv", "bram.vhd", "vdp_common.vhd", "vdp.vhd",
            "gen_io.sv", "multitap.sv", "teamplayer.sv", "fourway.v",
            # (lightgun.sv is used by MiSTer's top level only)
            "miracle.sv", "cheatcodes.sv", "EEPROM_STM95.sv", "audio_iir_filter.v",
            "genesis_lpf.v",
            "FX68K/fx68k.sv", "FX68K/fx68kAlu.sv", "FX68K/uaddrPla.sv",
            "FX68K/microrom.mem", "FX68K/nanorom.mem",
            "T80/T80.vhd",
            "T80/T80_ALU.vhd", "T80/T80_MCode.vhd", "T80/T80_Reg.vhd", "T80/T80s.vhd",
            "jt89/jt89.v", "jt89/jt89_noise.v", "jt89/jt89_vol.v", "jt89/jt89_mixer.v",
            "jt89/jt89_tone.v",
            *[
                f"jt12/{n}.v"
                for n in [
                    "jt12", "jt12_top", "jt12_acc", "jt12_single_acc", "jt12_eg", "jt12_eg_cnt",
                    "jt12_eg_comb", "jt12_eg_step", "jt12_eg_pure", "jt12_eg_final",
                    "jt12_eg_ctrl", "jt12_exprom", "jt12_kon", "jt12_lfo", "jt12_div",
                    "jt12_mod", "jt12_op", "jt12_csr", "jt12_pg", "jt12_pg_inc", "jt12_pg_dt",
                    "jt12_pg_sum", "jt12_pg_comb", "jt12_pm", "jt12_logsin", "jt12_reg",
                    "jt12_sh", "jt12_sh_rst", "jt12_sh24", "jt12_sumch", "jt12_timers",
                    "jt12_pcm_interpol", "jt12_mmr", "jt12_dout", "jt12_rst",
                    "adpcm/jt10_adpcm_div", "mixer/jt12_genmix", "mixer/jt12_decim",
                    "mixer/jt12_interpol", "mixer/jt12_comb", "mixer/jt12_fm_uprate",
                ]
            ],
        ]
    ],
]

_SMS = "verilog/sms"
_SMS_MISTER = f"{_SMS}/mister"
CORE_FILES["net.gamebub.core.sms.HandheldSms"] = [
    f"{_SMS}/sms_top.sv",
    # The Genesis core's SDRAM controller (instead of mister/sdram.sv)
    "verilog/genesis/mister/sdram.sv",
    f"{_SMS}/sms_core.vhd",
    # Portable spram/dpram/sprom (instead of mister/spram.vhd, dpram.vhd, sprom.vhd)
    f"{_SMS}/ram_sms.vhd",
    *[
        f"{_SMS_MISTER}/{name}"
        for name in [
            "cheatcodes.sv", "MC8123.v", "SEGASYS1_PRGDEC.v", "parts.v",
            "system.vhd", "video.vhd", "io.vhd", "AudioMix.vhd", "cart_eeprom.vhd",
            "eeprom_93c46.vhd", "evolution_mapper.vhd", "evolution_rom_address.vhd",
            "mapper_ctrl.vhd", "mapper_detect.vhd", "memory_bus.vhd", "memory_decode.vhd",
            "vdp.vhd", "vdp_background.vhd", "vdp_cram.vhd", "vdp_main.vhd",
            "vdp_sprite_shifter.vhd", "vdp_sprites.vhd",
            "T80/T80.vhd", "T80/T80_ALU.vhd", "T80/T80_MCode.vhd", "T80/T80_Reg.vhd",
            "T80/T80s.vhd",
            "jt89/jt89.vhd", "jt89/jt89.v", "jt89/jt89_mixer.v", "jt89/jt89_noise.v",
            "jt89/jt89_tone.v", "jt89/jt89_vol.v",
            *[
                f"VM2413/{n}.vhd"
                for n in [
                    "vm2413", "opll", "attacktable", "controller", "envelopegenerator",
                    "envelopememory", "feedbackmemory", "lineartable", "operator",
                    "outputgenerator", "outputmemory", "phasegenerator", "phasememory",
                    "registermemory", "sinetable", "slotcounter", "temporalmixer",
                    "voicememory", "voicerom",
                ]
            ],
        ]
    ],
]
CORE_FILES["net.gamebub.core.sms.HandheldGameGear"] = CORE_FILES["net.gamebub.core.sms.HandheldSms"]

def vhdl_file_type(file: str) -> str:
    # bram.vhd uses the shared-variable RAM template, and VM2413 uses names
    # that are VHDL-2008 keywords: both are VHDL-93.
    if file.endswith(("/bram.vhd", "/ram_sms.vhd")) or "/OPLL/" in file or "/VM2413/" in file:
        return "vhdlSource"
    return "vhdlSource-2008"


def get_git_revision() -> str:
    try:
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True)
        revision = revision.strip()
    except:
        print("Failed to find Git revision")
        return ""

    return revision[:12]


def insert_bitstream_metadata(bitstream: bytearray, extra: str):
    # Very crude way of not parsing the Xilinx header
    # Assumes the first tag is "a" (design name)
    HEADER = b"\x00\x09\x0f\xf0\x0f\xf0\x0f\xf0\x0f\xf0\x00\x00\x01a"
    if bitstream[: len(HEADER)] != HEADER:
        raise SystemExit("Unexpected bitstream header")

    # Find the length of the tag
    index = len(HEADER)
    (length,) = struct.unpack(">H", bitstream[index : (index + 2)])
    if length < 1:
        return
    index += 2
    # Extract the tag
    tag = bitstream[index : (index + length)]
    # Modify the tag
    tag = tag[:-1] + extra.encode("utf-8") + bytearray([0])
    bitstream[index : (index + length)] = tag
    # Modify the length
    length = struct.pack(">H", len(tag))
    index = len(HEADER)
    bitstream[index : (index + 2)] = length


def build(
    name: str,
    target: Target,
    build_root: Path,
    core_class: str,
):
    files = []
    if build_root.exists():
        shutil.rmtree(build_root)
    build_root.mkdir(parents=True, exist_ok=True)

    # Run Chisel
    # TODO: handle multiple source roots
    generate_root = build_root / "generated"
    args = [
        MILL_PATH,
        "-i",
        "GameBub.runMain",
        target.main_class,
        core_class,
        *target.chisel_args,
        f"--target-dir={generate_root}",
    ]
    subprocess.run(args)

    # Collect files from Chisel output
    filelist_path = generate_root / "filelist.f"
    with open(filelist_path) as f:
        for filename in f:
            path = filelist_path.parent / filename.rstrip()
            files.append(dict(name=path, file_type="systemVerilogSource"))

    # Add additional target files
    for file in target.files:
        file_extension = Path(file).suffix
        file_type = {
            ".xdc": "xdc",
            ".v": "verilogSource",
            ".sv": "systemVerilogSource",
        }[file_extension]
        path = ROOT_PATH / file
        files.append(dict(name=path, file_type=file_type))

    # Add additional core files
    for file in CORE_FILES.get(core_class, []):
        file_type = {
            ".xdc": "xdc",
            # MiSTer's sys/ Verilog uses SystemVerilog syntax
            # (as do the Genesis and SMS cores')
            ".v": "systemVerilogSource" if "/sys/" in file or "/genesis/" in file or "/sms/" in file
            else "verilogSource",
            ".sv": "systemVerilogSource",
            ".vhd": vhdl_file_type(file),
            ".mem": "mem",
        }[Path(file).suffix]
        files.append(dict(name=ROOT_PATH / file, file_type=file_type))

    # Make all file paths relative to build root.
    for f in files:
        f["name"] = os.path.relpath(str(f["name"]), str(build_root))

    # Set Verilog defines
    parameters = {
        name: {"datatype": "bool", "paramtype": "vlogdefine", "default": True}
        for name in target.verilog_defines
    }

    # Run Vivado (via Edalize)
    edam = {
        "name": name,
        "toplevel": target.top_module,
        "files": files,
        "parameters": parameters,
        "tool_options": {"vivado": {"part": target.part}},
    }
    backend = get_edatool("vivado")(edam=edam, work_root=build_root)
    backend.configure()
    backend.build()

    # After a successful build, the bitstream will be at <name>.bit.
    bitstream_path = build_root / f"{name}.bit"
    if not bitstream_path.exists():
        sys.exit("Bitstream path doesn't exist")
    print("Found bitstream at", bitstream_path)
    bitstream = bytearray(open(bitstream_path, "rb").read())

    # Determine Git Revision
    git_revision = get_git_revision()
    if git_revision:
        print("Git revision:", git_revision)

    # Insert Git information into bitstream
    insert_bitstream_metadata(bitstream, ";GitRev=" + git_revision)
    open(bitstream_path, "wb").write(bitstream)

    # Compress bitstream (heatshrink)
    try:
        import heatshrink2
        compressed_path = bitstream_path.with_name(bitstream_path.name + ".hs")
        compressed = heatshrink2.encode(bitstream, window_sz2=9, lookahead_sz2=6)
        with open(compressed_path, "wb") as f:
            f.write(compressed)
    except ImportError:
        # Skip heatshrink
        pass

    print("*" * 80)
    print("Bitstream:")
    print(bitstream_path.resolve())
    print("*" * 80)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--name", required=True)
    parser.add_argument(
        "--target",
        choices=[t.name for t in TARGETS],
        required=True,
        help="Target hardware",
    )
    parser.add_argument("--build-root", required=True, type=Path)
    parser.add_argument("--core-class", required=True)

    args = parser.parse_args()
    target = next(t for t in TARGETS if t.name == args.target)
    build(
        args.name,
        target,
        args.build_root.resolve(),
        args.core_class,
    )


if __name__ == "__main__":
    main()
