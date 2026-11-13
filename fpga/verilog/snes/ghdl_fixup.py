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
instantiating Verilog sets can be declared (and ignored) with `--param`. VHDL
port defaults are lost, so an input the instantiating Verilog leaves unconnected
can be removed and tied to its default value with `--tie`.

When several GHDL outputs are compiled together, the same VHDL entity can
appear in more than one of them. With `--prefix`, every module except the top
one is renamed with that prefix so the files don't clash.

Usage: python3 ghdl_fixup.py <file.v> --top <module> [--prefix <prefix>]
                             [--param <name>=<value> ...] [--tie <port>=<value> ...]
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


def tie_port(lines, start, port, value):
    """Remove input `port` from the top module's header and drive it with `value`."""
    header_end = next(i for i in range(start, len(lines)) if ");" in lines[i])
    for i in range(start, header_end + 1):
        m = re.match(r"(\s*)(\(?)input\s+(\[[^\]]*\]\s*)?" + port + r"\s*(,|\);)(.*)", lines[i], re.S)
        if not m:
            continue
        indent, paren, _, end, rest = m.groups()
        del lines[i]
        if paren:
            # First port: move the parenthesis to the next one.
            lines[i] = re.sub(r"^(\s*)", r"\1(", lines[i], count=1)
        if end == ");":
            # Last port: end the list at the previous one.
            lines[i - 1] = lines[i - 1].rstrip().rstrip(",") + ");\n"
            i -= 1
        else:
            i -= 1
        header_end = next(k for k in range(start, len(lines)) if ");" in lines[k])
        lines.insert(header_end + 1, f"  wire {port} = {value};\n")
        return lines
    raise SystemExit(f"--tie: input {port} not found")


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
parser.add_argument("--tie", action="append", default=[])
args = parser.parse_args()

with open(args.path) as f:
    lines = fixup(f.readlines())
lines = fix_top(lines, args.top, args.param)
for tie in args.tie:
    port, value = tie.split("=", 1)
    start = next(i for i, line in enumerate(lines) if re.match(rf"module {args.top}\b", line))
    lines = tie_port(lines, start, port.strip(), value.strip())
if args.prefix:
    lines = prefix_modules(lines, args.prefix, args.top)
with open(args.path, "w") as f:
    f.writelines(lines)
