# Master System / Game Gear core: credits and licences

The Master System and Game Gear cores in this fork are built from the
[MiSTer SMS core](https://github.com/MiSTer-devel/SMS_MiSTer) (commit 66f16d5),
vendored in this directory with small portability changes (see the git history).

## Original author

The core is based on **Ben's (ben0109) Sega Master System implementation for
the Papilio FPGA board**:

- Source: <https://github.com/ben0109/Papilio-Master-System>
- Blog: <http://fpga-hacks.blogspot.de/>

Ben's work is the foundation of the Master System cores on MiST, MiSTer, the
Analogue Pocket and other boards, including this one. Thank you, Ben.

## Ports this builds on

- The **MiST** port, and the **MiSTer** core maintained by the MiSTer-devel
  contributors (including Till Harbaum, Alexey Melnikov (Sorgelig) and
  Alastair M. Robinson), which added the Game Gear, mappers, FM sound and much
  more.

## Included components and their licences

| Component | Files | Author | Licence |
|---|---|---|---|
| T80 Z80 CPU | `T80/` | Daniel Wallner; later versions Sorgelig | BSD-style (in the file headers) |
| jt89 SN76489 PSG | `jt89/` | Jose Tejada | GPL-3.0 or later |
| VM2413 YM2413 FM | `VM2413/` | Mitsutaka Okazaki | BSD-style (in the file headers) |
| Audio mixer | `AudioMix.vhd` | Alastair M. Robinson | copyright notice, no licence terms |
| Arcade decryption | `MC8123.v`, `SEGASYS1_PRGDEC.v`, `parts.v` | blackwine, MiSTer-X | copyright notices, no licence terms |
| SDRAM controller (not used here) | `sdram.sv` | Sorgelig | GPL-3.0 or later |

## Licence status of the core's own files

The core's own files (`system.vhd`, `vdp*.vhd`, `io.vhd`, `video.vhd`, the
memory, mapper and EEPROM files) carry no licence headers, and neither the
MiSTer repository nor Ben's original has a LICENSE file. They are included here
with credit, as in the other public ports of this core. Ben has said he is glad
to see his work reused and improved in MiSTer
([ben0109/Papilio-Master-System#3](https://github.com/ben0109/Papilio-Master-System/issues/3)),
but no licence has been stated. If you are an author of this code and want it
credited differently or removed, please open an issue on this fork.

The rest of this fork's Master System / Game Gear work (`../sms_core.vhd`,
`../sms_top.sv`, `../ram_sms.vhd`, the Chisel wrapper and the firmware) is part
of this fork.
