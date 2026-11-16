"""A tiny two-pass assembler for the few SM83 (Game Boy CPU) instructions the
cartridge music player uses: raw opcode bytes, plus labels for 16-bit operands
(ref16) and relative jumps (jr)."""


class Asm:
    def __init__(self, origin):
        self.origin = origin
        self.code = bytearray()
        self.labels = {}
        self.fixups = []  # (offset, label, kind)

    @property
    def pc(self):
        return self.origin + len(self.code)

    def label(self, name):
        assert name not in self.labels, name
        self.labels[name] = self.pc

    def b(self, *bs):
        self.code += bytes(bs)

    def ref16(self, op, target):
        self.b(op)
        if isinstance(target, str):
            self.fixups.append((len(self.code), target, "abs"))
            self.code += b"\0\0"
        else:
            self.code += bytes([target & 0xFF, (target >> 8) & 0xFF])

    def jr(self, op, target):
        self.b(op)
        self.fixups.append((len(self.code), target, "rel"))
        self.b(0)

    def resolve(self):
        for off, name, kind in self.fixups:
            addr = self.labels[name]
            if kind == "abs":
                self.code[off:off + 2] = bytes([addr & 0xFF, addr >> 8])
            else:
                rel = addr - (self.origin + off + 1)
                assert -128 <= rel <= 127, f"jr to {name} out of range ({rel})"
                self.code[off] = rel & 0xFF
        return bytes(self.code)
