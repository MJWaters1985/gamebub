# Device test findings, 17 November 2026

**Status (end of day):** every finding is fixed except 3, which was the test's mistake. The
reruns found 6 (fixed: `4f7109ff`, `58f5a803`), 7 (fixed: `dbe0010c`, `7c11f64a`) and 8, the SD
card failure caught over USB (fixed: `3fd4c9ac`, to confirm on later cold starts). Last full
run, on `7c11f64`: all eight tests pass (`test-runs/20261117-193657`).

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

### 1. Run ROM crashes going back from a game list to the system list (high) — fixed

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

**Fixed (`4f6e8282`):** both of the above. Ten rounds of `leak_run_rom` now keep about 103 KB
free on the home screen, with no crash.

### 2. Built-in music data misses the Game Bub's own ROMs when they differ from the Pocket's (medium) — fixed

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

**Fixed (`4f6e8282`, data rebuilt):** the data is now built from the Pocket card, `cart-dumps/` and
the Game Bub card's own 41 Game Boy / Color ROMs: 217 games, and all 41 Game Bub ROMs have
music (Donkey Kong included). Pokemon Silver has no rip of its own; `music_db.py` now has
aliases, and Silver rebuilds the Gold rip exactly (125 bytes of data).

### 3. The in-game menu ignores the d-pad the first time it's opened after a game starts — not a bug

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

**Resolved:** the test's mistake. A game started from Run ROM has a fourth item, **Load ROM**
(Return, Load ROM, Reset, Exit Core); music has three. Down, Down, A chose Reset. The test now
presses Down three times (the menu doesn't wrap, so that's Exit Core either way), and every core
exits on the first try.

### 4. With no cartridge inserted, the message says to clean the contacts (low) — fixed

**Seen:** Cartridge Manager and Cartridge Music with an empty slot: "Couldn't read the cartridge
header. Clean the cartridge contacts and try again." (`identify_dmg` in
`src/cart_tool/cart.rs`). The firmware can't tell an empty slot from a dirty cartridge (the logo
check fails either way), but the advice only fits the second.

**Fixed (`4f6e8282`):** a header that reads as all `0xFF` or all `0x00` (an empty slot) says
"There's no cartridge in the slot. Insert a cartridge."; otherwise "Couldn't read the cartridge
header. If a cartridge is inserted, clean its contacts and try again." (Game Boy and Advance).

**Fix:** "No cartridge found. Insert a cartridge, or clean its contacts if one is inserted." (and
if a read of all `0xFF`/`0x00` is typical of an empty slot, say "Insert a cartridge" for that).

### 5. "Dropping UI message" logged as an error at every boot (low) — fixed

**Seen:** `E (443) handheld::ui: Dropping UI message`, once per boot, before the UI thread is
up (`ui::send` with no sender yet, `src/ui/mod.rs`). Something is sent too early and lost.

**Fix:** log which message (e.g. `{message:?}` or its variant) to find the sender; queue it or
send it after the UI starts. Then it can stop being an error-level line that tests must ignore.

**Fixed (`4f6e8282`):** messages sent before the UI thread starts are queued (up to 32) and
delivered when it starts; the line no longer appears.

### 6. A screenshot of a game list with box art crashed the Game Bub (high) — fixed

**Seen (rerun on `4f6e828`):** right after a game list loaded its box art, a USB screenshot
(`bubctl screenshot`) ran out of memory inside the renderer's full redraw (Slint
`SceneTexture` vector growing: `memory allocation of 7168 bytes failed`; 19 KB free, largest
block 3.8 KB). The screenshot held an 8 KB write buffer and an open file during the redraw.

**Fixed (`4f7109ff`, `58f5a803`):** lines are written from the stack (no heap buffer), and a
screenshot is refused, with a warning and the stale file removed, when the largest free block is
under 24 KB; `bubctl screenshot` asks again up to four times.

### 7. Memory is very tight while a game list with box art is showing (high) — fixed

**Seen:** with box art on screen, the largest free block stays between 7 and 22 KB (free memory
about 55 KB), so 28 of 35 screenshot attempts were refused in `run_rom_lists`. Normal drawing
copes, but anything that allocates a few KB at that moment (a full redraw, a notification, a
Wi-Fi event) can run out, as finding 6 did. This is the Run ROM memory pressure noted before
(box art strip pool, game list model, renderer scene).

**Fix (to do):** measure what the list view holds (box art strips about 45 KB, the list model,
`library.games`); keep box art in fewer, smaller strips or load it into one pre-allocated buffer
at boot; and keep a reserve (e.g. 32 KB contiguous) that box art loading won't take
(`THUMBNAIL_HEADROOM` is 24 KB of total free, not contiguous). Then `run_rom_lists` should get
every screenshot.

**More seen while fixing:** the Genesis list (164 games) with box art ran the heap out in an
**ordinary frame**, not just a screenshot: Slint's render cache grew with 816 bytes free. An
emergency reserve block (`f00b85e9`) didn't help and was reverted (`21918834`): it cost 16 KB in
exactly this view, and a full redraw needs more than any reserve.

**Fixed:**
- Box art at 96 pixels (28 KB of strips) instead of 120 (43 KB) (`dbe0010c`).
- `Game` keeps its path as one boxed string and derives the name from it, about 10 KB less for
  Genesis; box art loads at full size only when 48 KB stays free (was 24 KB), so long lists get
  smaller box art instead of starving the renderer (`7c11f64a`).
- After the fix: about 72 KB free with any list and its box art on screen (Genesis: 87 KB with the
  list, 71 KB with its smaller box art); USB screenshots of every list go through, at most after
  a retry.

### 8. The SD card sometimes isn't found after a cold start (medium) — fixed, to confirm

**Seen (reported, then caught over USB serial):** after a cold start (the fuel gauge had lost
power) the card answered but FatFs found no FAT volume: `failed to mount card (13)`
(`FR_NO_FILESYSTEM`), `ESP_FAIL`. A restart fixed it every time, so the card and its file system
were fine; the first reads after power-up came back wrong. The mount retries (`2d0a169`) treated
`ESP_FAIL` as final and never ran.

**Fixed (`3fd4c9ac`):** `ESP_FAIL` is retried like the other errors (four attempts, the last two
at 20 MHz). To confirm: the next time the Game Bub starts from off, the log should show the card
mounted (and a warning if it needed a retry).

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
