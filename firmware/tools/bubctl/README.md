# bubctl

Control a Game Bub over USB from a Mac (or Linux), with no extra libraries (Rust + `nusb`).

    cargo +stable build --release
    target/release/bubctl info
    target/release/bubctl press right a wait:1500 down a     # buttons in turn; a+b together
    target/release/bubctl screenshot out.png                 # the menus, not the game's picture
    target/release/bubctl log 30 --until "Core ready"        # the USB serial log, read only
    target/release/bubctl install firmware.uf2               # update mode must already be on
    target/release/bubctl reboot

Needs firmware with USB screenshots and the quiet gamepad slot (commit bdb5e5f3 or later).

Update mode has to be entered with the buttons: restarting into it over USB starts the update
firmware without its drive and with the vibration motor and audio amplifier pins floating.

## On-device tests

    python3 devtest.py                  # the whole suite (about 15 minutes)
    python3 devtest.py --only boot,tools
    python3 devtest.py --music-all      # music for every Game Boy / Color game, not just 3

Each test restarts the Game Bub, drives it with button presses and checks the USB serial log
(screens entered, memory notes, cores starting, errors, panics, unexpected restarts). Results
go to `test-runs/<date-time>/` at the top of the repository: `report.md`, `serial.log` and
screenshots. Nothing is erased, formatted, written to a cartridge or changed in Settings.
Findings from the first run: `docs/device-test-findings-2026-11-17.md`.
