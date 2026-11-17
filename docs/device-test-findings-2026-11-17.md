# Device test findings, 17 November 2026

Found by the automated on-device suite (`firmware/tools/bubctl/devtest.py`) on a rev 4 Game Bub
(hardware 1.4.1.0) running firmware `b5453bb` (branch `cart-music`), with the SD card's
library: 5 favorites, Game Boy 24, Game Boy Advance 41, Game Boy Color 17, Game Gear 28,
Genesis 164, Master System 50, NES 16, Super Nintendo 11. No cartridge was inserted.

Full runs: `test-runs/20261117-164134` (whole suite), `20261117-165144` (core launches and memory
tests again), `20261117-165943` (music memory test alone); each has `report.md`, `serial.log`
and screenshots. `test-runs/` is ignored by git.

## What passed

- Boot: 190 KB free heap, largest block 127 KB.
- Run ROM: every system's list opens with the right count (9 lists, 2–3 s each).
- Every core starts from Run ROM and returns to the home screen: Game Boy, Game Boy Color, Game
  Boy Advance (all at 100% speed), Game Gear, Genesis, Master System, NES, Super Nintendo.
- Music from Run ROM (Y): plays for every Game Boy / Color game tried that has music data; song
  changes (Left/Right) and leaving work. Five play/leave rounds: free heap on the home screen
  103,588 → 103,444 bytes (about 36 bytes a round: noise, but worth watching).
- Tools (Cartridge Manager, Set Up SD Card with Check ROMs, battery info, Cartridge Music) and
  every Settings page open and close without errors.

## Findings

### 1. Run ROM crashes going back from a game list to the system list (high)

**Seen:** Run ROM → open a system (box art shows) → B. The firmware runs out of memory and
restarts: free heap falls to about 20 KB while it recounts the systems, and counting Genesis
(164 games) fails: `memory allocation of 5328 bytes failed`. Reproduced on every run
(`leak_run_rom`); also the cause of the out-of-memory crash seen earlier when opening Run ROM.

**Cause:** `UiState::library_open` (`src/ui/state/library.rs`) counts every system's games
while the game list it came from is still held: the box art strips (about 45 KB, freed by Slint
only on the next redraw), the game list model and `self.library.games`. And it counts by
calling `System::games()` (`src/library.rs`), which builds a `Vec<Game>` with a name and a full
path per game and sorts it (the sort's scratch buffer is the failing allocation), just to take
its length.

**Fix:**
- In `library_open`, release the game list and box art first (as `library_close` does,
  including `strip_pool`), and count after the next redraw, like box art loading already waits.
- Count without building lists: a `System::count()` that walks the folder and counts matching
  entries (no names, no paths, no sort).
- Add a test step that goes back from the biggest list (the suite's `leak_run_rom` covers it).

### 2. Built-in music data misses the Game Bub's own ROMs when they differ from the Pocket's (medium)

**Seen:** Donkey Kong (Game Boy) on the Game Bub's SD card shows no Music hint, though the
built-in data has Donkey Kong (Japan, USA) (SGB Enhanced) from the Pocket card. The Game Bub's
copy has a different header (another revision or dump), so its key doesn't match. The same will
happen for any game whose Game Bub ROM differs from the Pocket's copy, and for cartridges dumped
later.

**Fix:**
- Build the music data from every ROM set: the Game Bub card's `/roms/Game Boy` and `/roms/Game
  Boy Color` and `cart-dumps/`, as well as the Pocket card (`CART_MUSIC_ROMS` takes several
  folders). Needs the Game Bub card in SD mode once.
- Optionally, when a header doesn't match, try the entries for the same title (No-Intro name):
  the rebuild's CRC check already refuses a version it can't rebuild exactly.

### 3. The in-game menu ignores the d-pad the first time it's opened after a game starts (medium)

**Seen:** after launching a game from Run ROM and playing for 6 seconds, Home → Down → Down → A
selected **Return** instead of **Exit Core** every time (8 of 8 cores); the second time the menu
opened, the same presses worked. Leaving music after 2 seconds worked the first time. A person
pressing Down quickly after Home would hit the same thing.

**Likely cause:** focus isn't on the menu's list when it first opens (the `init` that focuses it
runs late), or input is dropped while the menu's opening animation runs (a past issue: "the UI
loop ignores input while animations run unless polled").

**Fix:** reproduce with `bubctl press home wait:1500 down down a` straight after starting a game
and check `Screen enter`; focus the list as the menu becomes visible, and poll input during the
animation.

### 4. With no cartridge inserted, the message says to clean the contacts (low)

**Seen:** Cartridge Manager and Cartridge Music with an empty slot: "Couldn't read the cartridge
header. Clean the cartridge contacts and try again." (`identify_dmg` in
`src/cart_tool/cart.rs`). The firmware can't tell an empty slot from a dirty cartridge (the logo
check fails either way), but the advice only fits the second.

**Fix:** "No cartridge found. Insert a cartridge, or clean its contacts if one is inserted." (and
if a read of all `0xFF`/`0x00` is typical of an empty slot, say "Insert a cartridge" for that).

### 5. "Dropping UI message" logged as an error at every boot (low)

**Seen:** `E (443) handheld::ui: Dropping UI message`, once per boot, before the UI thread is
up (`ui::send` with no sender yet, `src/ui/mod.rs`). Something is sent too early and lost.

**Fix:** log which message (e.g. `{message:?}` or its variant) to find the sender; queue it or
send it after the UI starts. Then it can stop being an error-level line that tests must ignore.

## Test suite limits

- Screenshots show the menus, not the game's picture; sound isn't checked (only that cores and
  music start, keep running and leave cleanly).
- Only the Game Boy core logs its run rate; logging it for every core would let the suite check
  speed everywhere.
- Not run: anything that erases, formats, writes to a cartridge or changes settings; tools that
  switch the USB mode (Mount SD Card over USB, FlashGBX); Download Box Art. Cartridge tests need
  a cartridge in the slot.

## Fixed earlier the same day (for reference)

- Run ROM crash drawing the Music hint: no Y button icon (`d4898405`).
- Restarting into update mode over USB ran the vibration motor and made the amplifier whine
  (`ffaa8025`, `b5453bbf`).
