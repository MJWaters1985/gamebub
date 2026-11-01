<p align="center">
  <a href="https://gamebub.net/">
    <img src="./assets/logo.svg" width="480" alt="Game Bub logo">
  </a>
</p>

> [!NOTE]
> **This is an unofficial fork** of [elipsitz/gamebub](https://github.com/elipsitz/gamebub)
> for the Rev 4 handheld, based on the official 1.0.3 release. It isn't affiliated with or
> supported by the Game Bub team: please report problems with this fork here, not upstream.

## About this fork

This fork adds cartridge compatibility fixes, quality-of-life features and bug fixes on top
of the official firmware and FPGA cores. See **[CHANGES.md](CHANGES.md)** for the full list.
Highlights:

- **Run ROM**: browse your games by system, with box art, and start them directly.
- **Wi-Fi**: download box art for your games and set the clock from the internet.
- **Cartridge Manager**: identify cartridges, dump ROMs to the SD card, and back up or
  restore saves, all on the device.
- **More physical GBA cartridges work**: fixed cartridge read timing for slow and large ROMs
  (e.g. Super Mario Advance 2, Pokémon Emerald, Lunar Legend, Donkey Kong Country).
- **Game Boy core fixes**: Kirby's Dream Land 2 no longer hangs at a white screen, the HALT
  bug is implemented, and DMG games can't accidentally switch to double speed.
- **Better file browser**: natural sorting, large folders, favorites (Select) and core
  settings before loading a ROM (Start). **Load ROM** from the in-game menu.
- **Safer saves** with a backup copy, and an optional `/saves` folder.
- **New settings**: auto power off, X/Y as Select/Start, game color temperature, integer
  overscale for GB/GBC and tilt sensitivity.
- **Crash and bug fixes**: WarioWare: Twisted, Game Boy Player rumble, headphones after a
  power cycle, USB drive mode after updates.

Firmware built from this fork shows a version like `1.0.3.mod-20261101 (3711c01)` in
**Settings → About**: the official version it's based on, the commit date and the commit.

### Installing

Download the `.uf2` update file from the latest
**[release](https://github.com/MJWaters1985/gamebub/releases/latest)** (Rev 4 only), or
build your own by following [docs/building-local.md](docs/building-local.md). Install it
like an official update: hold **Volume Down** while pressing **Power** to start update mode,
and copy the file to the **GAME BUB** drive that appears on your computer. To go back,
install an official update the same way.

---

*The rest of this README is from the official project.*

[![Game Bub trailer](./assets/video-poster.jpg)](https://www.youtube.com/watch?v=f16E5J6qljw)

**Game Bub** is an open-source FPGA retro emulation handheld, with support for Game Boy, Game Boy Color, and Game Boy Advance games.

You can buy your own, prebuilt Game Bub from **[Crowd Supply](https://www.crowdsupply.com/second-bedroom/game-bub)**!

You can find the [user guide and documentation here](https://docs.gamebub.net/).

Want to chat about Game Bub? Feel free to [join the Discord](https://discord.gg/T5xrYpMfN7).

Check out the [announcement blog post](https://eli.lipsitz.net/posts/introducing-gamebub/) for an in-depth look at the development process!


## Features
* Play physical Game Boy / Color / Advance cartridges
* Load and play ROM files from a microSD card (with built-in support for rumble, clock, accelerometer, gyroscope)
* Multiplayer link cable functionality
* Custom, from-scratch Game Boy and Game Boy Advance FPGA cores with great game compatibility
* 14+ hour battery life 
* Video output to TV or monitor via Game Bub Dock
* Extensible hardware, designed for future improvements

## Building

Building a Game Bub handheld requires manufacturing PCBs, 3D printing the shell and buttons, and assembling components from a variety of sources. For information on manufacturing and assembling your own, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/building.md). Note that this information is for the older, vertical revision 2. Updated building information will be available in the Game Bub Docs soon.

For other inquiries, contact us at support@gamebub.net.

## Architecture

For an in-depth description of the project architecture, as of revision 2, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/architecture.md).

The Game Bub handheld consists of a Xilinx XC7A100T FPGA to do the main emulation and I/O, and an ESP32-S3 microcontroller to do auxiliary tasks (configuring the FPGA, rendering the UI, loading ROM files from a microSD card and sending it to the FPGA).

### Directory Structure

* `fpga`: FPGA source code (HDL), written in [Chisel](https://github.com/chipsalliance/chisel)
* `firmware`: Microcontroller firmware

## License

Unless otherwise specified:

* Firmware (in `firmware/`) and scripts (in `scripts/`) are licensed under GPLv3 (`GPL-3.0-only`).
* FPGA source code (in `fpga/`) is licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)
* PCB (schematic and layout), mechanical, and hardware design files are licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)

At a high level, this means that you can copy, share, and modify the source code, as long as you provide proper attribution and share your source code / design files with the same license. However, the "Game Bub" name and logo are trademarked, and you may not use them for your product without permission.
