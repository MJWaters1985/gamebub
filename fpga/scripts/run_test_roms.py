"""Run test ROMs in the Verilator simulator and compare screenshots against
reference images.

Expects the test ROM collection from https://github.com/c-sp/game-boy-test-roms
(for Game Boy) and https://github.com/jsmolka/gba-tests (for GBA).

Example:
    python scripts/run_test_roms.py gameboy --sim build/sim-gb/VSimGameboy \\
        --roms ~/testroms --bios-dmg dmg.bin --bios-cgb cgb.bin
"""

import argparse
import concurrent.futures
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image

GB_FPS = 4194304 / 70224
GBA_FPS = 16777216 / 280896


@dataclass
class Test:
    rom: str
    expected: str
    seconds: float
    # "dmg" or "cgb" for Game Boy; ignored for GBA.
    model: str = "cgb"


GB_TESTS = [
    # Blargg
    Test("blargg/cpu_instrs/cpu_instrs.gb", "blargg/cpu_instrs/cpu_instrs-dmg-cgb.png", 31),
    Test("blargg/cpu_instrs/cpu_instrs.gb", "blargg/cpu_instrs/cpu_instrs-dmg-cgb.png", 55, "dmg"),
    Test("blargg/instr_timing/instr_timing.gb", "blargg/instr_timing/instr_timing-dmg-cgb.png", 1),
    Test("blargg/instr_timing/instr_timing.gb", "blargg/instr_timing/instr_timing-dmg-cgb.png", 1, "dmg"),
    Test("blargg/mem_timing/mem_timing.gb", "blargg/mem_timing/mem_timing-dmg-cgb.png", 3),
    Test("blargg/mem_timing/mem_timing.gb", "blargg/mem_timing/mem_timing-dmg-cgb.png", 3, "dmg"),
    Test("blargg/mem_timing-2/mem_timing.gb", "blargg/mem_timing-2/mem_timing-dmg-cgb.png", 4),
    Test("blargg/mem_timing-2/mem_timing.gb", "blargg/mem_timing-2/mem_timing-dmg-cgb.png", 4, "dmg"),
    Test("blargg/interrupt_time/interrupt_time.gb", "blargg/interrupt_time/interrupt_time-cgb.png", 2),
    Test("blargg/halt_bug.gb", "blargg/halt_bug-dmg-cgb.png", 2),
    Test("blargg/halt_bug.gb", "blargg/halt_bug-dmg-cgb.png", 2, "dmg"),
    Test("blargg/oam_bug/oam_bug.gb", "blargg/oam_bug/oam_bug-dmg.png", 21, "dmg"),
    Test("blargg/cgb_sound/cgb_sound.gb", "blargg/cgb_sound/cgb_sound-cgb.png", 37),
    Test("blargg/dmg_sound/dmg_sound.gb", "blargg/dmg_sound/dmg_sound-dmg.png", 36, "dmg"),
    # Acid tests (PPU)
    Test("dmg-acid2/dmg-acid2.gb", "dmg-acid2/dmg-acid2-dmg.png", 1, "dmg"),
    Test("dmg-acid2/dmg-acid2.gb", "dmg-acid2/dmg-acid2-cgb.png", 1),
    Test("cgb-acid2/cgb-acid2.gbc", "cgb-acid2/cgb-acid2.png", 1),
]

GBA_TESTS = [
    Test("arm/arm.gba", "", 1),
    Test("thumb/thumb.gba", "", 1),
    Test("memory/memory.gba", "", 1),
    Test("bios/bios.gba", "", 1),
    Test("nes/nes.gba", "", 1),
    Test("save/sram.gba", "", 1),
    Test("save/flash64.gba", "", 6),
    Test("save/flash128.gba", "", 6),
]


def load_ppm(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.int32)


def shade_ranks(image: np.ndarray) -> np.ndarray:
    """Map each pixel to the brightness rank of its color.

    DMG screenshots differ only in palette, so compare shade indices instead
    of colors.
    """
    luma = image[..., 0] * 299 + image[..., 1] * 587 + image[..., 2] * 114
    levels = np.unique(luma)
    return np.searchsorted(levels, luma)


def compare(actual: np.ndarray, expected: np.ndarray, model: str) -> bool:
    if actual.shape != expected.shape:
        return False
    grayscale = np.array_equal(expected[..., 0], expected[..., 1]) and np.array_equal(
        expected[..., 1], expected[..., 2]
    )
    if model == "dmg" or grayscale:
        return np.array_equal(shade_ranks(actual), shade_ranks(expected))
    return int(np.abs(actual - expected).max()) <= 8


GOLDEN_DIR = Path(__file__).resolve().parent.parent / "sim" / "golden"


def gba_passed(image: np.ndarray) -> bool:
    # jsmolka's tests print "All tests passed" on success.
    golden = Image.open(GOLDEN_DIR / "gba-tests-all-passed.png").convert("RGB")
    return np.array_equal(image, np.asarray(golden, dtype=np.int32))


def run(test: Test, args, out_dir: Path) -> tuple[Test, bool, str]:
    rom = args.roms / test.rom
    name = test.rom.replace("/", "_") + f"-{test.model}"
    shot = out_dir / f"{name}.ppm"
    if args.system == "gameboy":
        bios = args.bios_dmg if test.model == "dmg" else args.bios_cgb
        frames = int(test.seconds * GB_FPS) + 1
    else:
        bios = args.bios_gba
        frames = int(test.seconds * GBA_FPS) + 1
    # Run from a copy of the ROM, so save files don't pollute the ROM directory.
    with tempfile.TemporaryDirectory() as tmp:
        rom_copy = Path(tmp) / rom.name
        rom_copy.write_bytes(rom.read_bytes())
        cmd = [str(args.sim), "--headless", "--frames", str(frames),
               "--screenshot", str(shot), "--bios-path", str(bios), str(rom_copy)]
        result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0 or not shot.exists():
        return test, False, f"simulator failed: {result.stderr.strip()[-200:]}"

    actual = load_ppm(shot)
    if args.system == "gameboy":
        expected = np.asarray(Image.open(args.roms / test.expected).convert("RGB"), dtype=np.int32)
        ok = compare(actual, expected, test.model)
    else:
        ok = gba_passed(actual)
    return test, ok, str(shot)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("system", choices=["gameboy", "gba"])
    parser.add_argument("--sim", required=True, type=Path)
    parser.add_argument("--roms", required=True, type=Path)
    parser.add_argument("--bios-dmg", type=Path)
    parser.add_argument("--bios-cgb", type=Path)
    parser.add_argument("--bios-gba", type=Path)
    parser.add_argument("--out", type=Path, default=Path("test-rom-results"))
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--filter", default="")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    tests = GB_TESTS if args.system == "gameboy" else GBA_TESTS
    tests = [t for t in tests if args.filter in t.rom]

    failures = 0
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for test, ok, detail in pool.map(lambda t: run(t, args, args.out), tests):
            status = "PASS" if ok else "FAIL"
            model = f" ({test.model})" if args.system == "gameboy" else ""
            print(f"{status}  {test.rom}{model}  {detail if not ok else ''}", flush=True)
            failures += not ok
    print(f"{len(tests) - failures}/{len(tests)} passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
