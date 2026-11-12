"""
Clean up GHDL's Verilog output so Verilator accepts it.

GHDL lowercases VHDL names, and can name an instance's output net the same as
an existing signal (instance `HOAM`, port `Q` and signal `HOAM_Q` all become
`hoam_q`). It then declares the wire twice and emits `assign hoam_q = hoam_q;`.
Both names refer to the same net, so drop the repeated declaration and the
self-assignment.

GHDL also keeps the top module's port names in their VHDL spelling, while the
Verilog that instantiates it (main.v) uses lowercase, so the top module's ports
are lowercased. Its generics are already applied, so any parameters the
instantiating Verilog sets can be declared (and ignored) with `--param`.

When several GHDL outputs are compiled together, the same VHDL entity can
appear in more than one of them. With `--prefix`, every module except the top
one is renamed with that prefix so the files don't clash.

Usage: python3 ghdl_fixup.py <file.v> --top <module> [--prefix <prefix>]
                             [--param <name>=<value> ...]
(rewrites the file in place)
"""

import argparse
import re

DECL = re.compile(r"^\s*(wire|reg)\b[^;]*?\b(\w+);")
SELF_ASSIGN = re.compile(r"^\s*assign (\w+) = (\w+);")


def fixup(lines):
    out = []
    declared = set()
    for line in lines:
        if line.startswith("module "):
            declared = set()
        m = SELF_ASSIGN.match(line)
        if m and m.group(1) == m.group(2):
            continue
        m = DECL.match(line)
        if m:
            if m.group(2) in declared:
                continue
            declared.add(m.group(2))
        out.append(line)
    return out


def prefix_modules(lines, prefix, top):
    names = set()
    for line in lines:
        m = re.match(r"module (\w+)", line)
        if m and m.group(1) != top:
            names.add(m.group(1))
    out = []
    for line in lines:
        m = re.match(r"(module |\s*)(\w+)(\b.*)", line, re.S)
        if m and m.group(2) in names:
            line = m.group(1) + prefix + m.group(2) + m.group(3)
        out.append(line)
    return out


def fix_top(lines, top, params):
    start = next(i for i, line in enumerate(lines) if re.match(rf"module {top}\b", line))
    end = next(i for i in range(start, len(lines)) if lines[i].startswith("endmodule"))
    header_end = next(i for i in range(start, end) if ");" in lines[i])
    ports = set()
    for line in lines[start:header_end + 1]:
        m = re.search(r"(input|output|inout)\s+(\[[^\]]*\]\s*)?(\w+)", line)
        if m:
            ports.add(m.group(3))
    pattern = re.compile(r"\b(" + "|".join(sorted(ports)) + r")\b")
    for i in range(start + 1, end):
        lines[i] = pattern.sub(lambda m: m.group(1).lower(), lines[i])
    if params:
        decls = ", ".join(f"parameter {p}" for p in params)
        lines[start] = lines[start].replace(f"module {top}", f"module {top} #({decls})", 1)
    return lines


parser = argparse.ArgumentParser()
parser.add_argument("path")
parser.add_argument("--top", required=True)
parser.add_argument("--prefix")
parser.add_argument("--param", action="append", default=[])
args = parser.parse_args()

with open(args.path) as f:
    lines = fixup(f.readlines())
lines = fix_top(lines, args.top, args.param)
if args.prefix:
    lines = prefix_modules(lines, args.prefix, args.top)
with open(args.path, "w") as f:
    f.writelines(lines)
