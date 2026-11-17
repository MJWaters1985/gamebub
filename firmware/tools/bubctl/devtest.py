#!/usr/bin/env python3
"""Automated on-device tests for the Game Bub, driven over USB with bubctl.

    devtest.py [--out DIR] [--only NAME,...] [--music-all]

Each test restarts the Game Bub (a known state: the home screen), drives it with virtual
button presses and checks the USB serial log: screens entered ("Screen enter: X"), memory
notes ("...: heap free N, largest M"), cores starting ("Core ready to run"), and anything
that looks like a problem (errors, panics, out of memory, unexpected restarts). Screenshots
and the full log go to DIR (default ../../../test-runs/<date-time>), with report.md.

Safe by design: nothing is erased, formatted, written to a cartridge or changed in Settings.
Tools that switch the USB mode (Mount SD Card over USB, FlashGBX) would cut the connection
and aren't run; Download Box Art writes to the SD card and isn't run either.
"""
import argparse, datetime, os, re, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
BUBCTL = os.path.join(HERE, 'target', 'release', 'bubctl')

PROBLEM = re.compile(r'panicked|Out of memory|Guru Meditation|abort\(\)|assert|^E \(|stack overflow|'
                     r'Backtrace|CORRUPT|watchdog', re.I)
# Known noise, reported once as a low-priority finding but not failing tests.
KNOWN = [re.compile(r'handheld::ui: Dropping UI message')]
# Expected when no cartridge is inserted (the tools test notes it instead).
NO_CART = re.compile(r'No cartridge could be read')
HEAP = re.compile(r'(.*): heap free (\d+), largest (\d+)')


class Log:
    """The serial log, read continuously by `bubctl log` in the background."""

    def __init__(self, path):
        self.lines = []
        self.lock = threading.Lock()
        self.file = open(path, 'w')
        self.proc = subprocess.Popen([BUBCTL, 'log', '86400'], stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, errors='replace')
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            line = line.rstrip('\n')
            if 'Render + display' in line:
                continue
            with self.lock:
                self.lines.append((time.time(), line))
            self.file.write(line + '\n')
            self.file.flush()

    def mark(self):
        with self.lock:
            return len(self.lines)

    def since(self, mark):
        with self.lock:
            return [l for _, l in self.lines[mark:]]

    def wait(self, pattern, mark, timeout):
        rx = re.compile(pattern)
        end = time.time() + timeout
        while time.time() < end:
            for l in self.since(mark):
                m = rx.search(l)
                if m:
                    return m
            time.sleep(0.1)
        return None

    def close(self):
        self.proc.terminate()
        self.file.close()


class Failed(Exception):
    pass


class Device:
    def __init__(self, out):
        self.out = out
        self.log = Log(os.path.join(out, 'serial.log'))
        self.shots = 0
        self.restarts = []   # log marks of restarts asked for
        self.retries = []    # things that needed another try (flaky input)

    def bubctl(self, *args, timeout=60):
        r = subprocess.run([BUBCTL, *args], capture_output=True, text=True, timeout=timeout)
        if r.returncode != 0:
            raise Failed('bubctl %s: %s' % (' '.join(args), (r.stderr or r.stdout).strip()))
        return r.stdout

    def press(self, *buttons, gap=250):
        self.bubctl('press', *buttons, '--gap', str(gap))

    def shot(self, name):
        self.shots += 1
        path = os.path.join(self.out, '%02d-%s.png' % (self.shots, name))
        try:
            self.bubctl('screenshot', path)
            return os.path.basename(path)
        except Failed:
            return None

    def expect(self, pattern, mark, timeout=10, what=None):
        m = self.log.wait(pattern, mark, timeout)
        if not m:
            raise Failed('Expected %s within %d s' % (what or repr(pattern), timeout))
        return m

    def restart(self):
        """Restart and wait for the home screen."""
        mark = self.log.mark()
        self.restarts.append(mark)
        # After a crash the Game Bub takes a while to come back (it writes the crash log).
        end = time.time() + 60
        while True:
            try:
                self.bubctl('reboot')
                break
            except Failed:
                if time.time() > end:
                    raise
                time.sleep(2)
        self.expect(r'device ready', mark, 30, 'the restart')
        self.expect(r'logo y', mark, 15, 'the home screen')
        time.sleep(1.5)

    def heap(self, mark, label=None):
        """The last memory note since mark (optionally whose event contains label)."""
        found = None
        for l in self.log.since(mark):
            m = HEAP.search(l)
            if m and (label is None or label in m.group(1)):
                found = (int(m.group(2)), int(m.group(3)))
        return found

    def problems(self, mark):
        out = []
        boots = 0
        for l in self.log.since(mark):
            if PROBLEM.search(l) and not any(k.search(l) for k in KNOWN) and not NO_CART.search(l):
                out.append(l)
            if 'Hardware version' in l:
                boots += 1
        asked = sum(1 for m in self.restarts if m >= mark)
        if boots > asked:
            out.append('(the Game Bub restarted %d time(s) on its own)' % (boots - asked))
        return out


# --- navigation ---------------------------------------------------------------------------

def open_home_item(d, item):
    """Home screen grid: Run Cartridge, Run ROM / Settings, Tools (Run Cartridge selected)."""
    moves = {'Run ROM': ['right'], 'Settings': ['down'], 'Tools': ['down', 'right']}[item]
    d.press(*moves, 'a')


SYSTEMS = None   # filled from the log: [(name, count)]


def run_rom_open_system(d, index, name):
    """Open Run ROM, go to the system at index (0 = Favorites) and open it."""
    mark = d.log.mark()
    open_home_item(d, 'Run ROM')
    d.expect(r'Screen enter: Library', mark, 10, 'the Run ROM screen')
    time.sleep(1.0)
    mark = d.log.mark()
    d.press(*(['up'] * 12), gap=60)
    d.press(*(['down'] * index + ['a']), gap=150)
    m = d.expect(r'Run ROM: (.+) \((\d+) games\)', mark, 15, 'the %s list' % name)
    if m.group(1) != name:
        raise Failed('Opened %s instead of %s' % (m.group(1), name))
    time.sleep(1.0)
    return int(m.group(2))


def exit_core(d):
    """In a core: Home, then Exit Core, the last item (the menu has Load ROM too for games
    started from Run ROM, and doesn't wrap: three Downs reach the end either way)."""
    for attempt in range(3):
        mark = d.log.mark()
        d.press('home', gap=1000)
        d.press('down', 'down', 'down', 'a', gap=300)
        if d.log.wait(r'Screen enter: MainMenu', mark, 8):
            if attempt:
                d.retries.append('in-game menu: Exit Core needed %d tries' % (attempt + 1))
            return
    raise Failed('Couldn\'t leave the core with Home > Exit Core')


# --- tests ----------------------------------------------------------------------------------

def t_boot(d, r):
    mark = d.log.mark()
    d.restart()
    h = d.heap(mark, 'device ready')
    r['notes'].append('Heap at boot: %s free, largest %s' % h if h else 'No heap note at boot')
    r['shots'].append(d.shot('home'))
    if h and h[0] < 150000:
        raise Failed('Only %d bytes free at boot' % h[0])


def t_run_rom_lists(d, r):
    global SYSTEMS
    d.restart()
    mark = d.log.mark()
    open_home_item(d, 'Run ROM')
    d.expect(r'Screen enter: Library', mark, 10)
    time.sleep(1.5)
    counted = re.findall(r'Run ROM: counting (.+?):', '\n'.join(d.log.since(mark)))
    SYSTEMS = [('Favorites', None)] + [(s, None) for s in sorted(counted, key=natural)]
    r['notes'].append('Systems: ' + ', '.join(s for s, _ in SYSTEMS))
    r['shots'].append(d.shot('run-rom'))
    d.press('b')
    for i, (name, _) in enumerate(SYSTEMS):
        d.restart()
        n = run_rom_open_system(d, i, name)
        SYSTEMS[i] = (name, n)
        r['shots'].append(d.shot('list-' + slug(name)))
        r['notes'].append('%s: %d games' % (name, n))


def t_launch_each_core(d, r):
    for i, (name, n) in enumerate(SYSTEMS or []):
        if name == 'Favorites' or not n:
            continue
        d.restart()
        run_rom_open_system(d, i, name)
        before = d.heap(0)
        mark = d.log.mark()
        d.press('a')
        try:
            d.expect(r'Core ready to run', mark, 60, '%s to start' % name)
        except Failed as e:
            r['shots'].append(d.shot('core-fail-' + slug(name)))
            r['errors'].append(str(e))
            continue
        time.sleep(6)
        mark2 = d.log.mark()
        exit_core(d)
        m = d.log.wait(r'Run rate: ([\d.]+)%', mark, 10)
        rate = float(m.group(1)) if m else None
        back = d.expect(r'Screen enter: (\w+)', mark2, 30, 'leaving %s' % name)
        time.sleep(1.5)
        after = d.heap(mark2)
        r['notes'].append('%s: started, run rate %s%%, back to %s, heap %s -> %s' %
                          (name, rate, back.group(1), before, after))
        if rate is not None and rate < 99.0:
            r['errors'].append('%s ran at %.1f%% speed' % (name, rate))


def t_music_from_rom(d, r, all_games=False):
    for i, (name, n) in enumerate(SYSTEMS or []):
        if name not in ('Game Boy', 'Game Boy Color') or not n:
            continue
        count = n if all_games else min(n, 3)
        for g in range(count):
            d.restart()
            run_rom_open_system(d, i, name)
            d.press(*(['down'] * g), gap=120)
            time.sleep(1.2)
            mark = d.log.mark()
            d.press('y')
            m = d.log.wait(r'Core ready to run|ROM music: (.+)', mark, 30)
            if not m:
                r['notes'].append('%s #%d: no music (no data, or nothing happened)' % (name, g + 1))
                continue
            if m.group(1):
                r['errors'].append('%s #%d: %s' % (name, g + 1, m.group(1)))
                continue
            time.sleep(4)
            mark2 = d.log.mark()
            d.press('right', gap=1500)   # next song
            d.press('left', gap=1500)
            exit_core(d)
            d.expect(r'Screen enter: (\w+)', mark2, 20, 'leaving the music')
            game = d.log.wait(r'File selected: (.+)', mark, 1)
            r['notes'].append('%s #%d: music played (%s)' % (name, g + 1, game.group(1) if game else '?'))


def t_tools(d, r):
    # (index in the Tools list, name, buttons once open, log to expect)
    tools = [
        (0, 'Cartridge Manager', [], r'Cartridge|cart'),
        (3, 'Set Up SD Card', [], None),
        (5, 'Debug battery info', [], None),
        (6, 'Cartridge Music', [], r'Cartridge Music|cartridge'),
    ]
    for index, name, extra, pattern in tools:
        d.restart()
        mark = d.log.mark()
        open_home_item(d, 'Tools')
        d.expect(r'Screen enter: Tools', mark, 10)
        time.sleep(0.8)
        d.press(*(['down'] * index + ['a']), gap=150)
        time.sleep(4)
        r['shots'].append(d.shot('tool-' + slug(name)))
        if name == 'Set Up SD Card':
            # Check ROMs (reads only): second item
            d.press('down', 'a', gap=300)
            m = d.log.wait(r'Checking ROMs|check', mark, 5)
            time.sleep(20)
            r['shots'].append(d.shot('tool-check-roms'))
        if name == 'Cartridge Music':
            if d.log.wait(r'Core ready to run', mark, 20):
                r['notes'].append('Cartridge Music: playing (a cartridge is inserted)')
                time.sleep(3)
                exit_core(d)
            else:
                r['notes'].append('Cartridge Music: didn\'t play (no cartridge, or refused; see screenshot)')
        else:
            d.press('b', gap=600)
            d.press('b', gap=600)
        r['notes'].append('%s: opened' % name)


def t_settings(d, r):
    pages = ['About', 'General', 'Display', 'Wi-Fi', 'Dock', 'Core: GB / GBC', 'Core: GBA']
    for i, page in enumerate(pages):
        d.restart()
        mark = d.log.mark()
        open_home_item(d, 'Settings')
        d.expect(r'Screen enter: Settings', mark, 10)
        time.sleep(0.8)
        d.press(*(['down'] * i + ['a']), gap=150)
        time.sleep(1.5)
        r['shots'].append(d.shot('settings-' + slug(page)))
        d.press('b', gap=500)
        r['notes'].append('%s: opened' % page)


def t_leak_run_rom(d, r):
    """Open and close Run ROM and a game list ten times: memory should come back."""
    d.restart()
    first = None
    samples = []
    for k in range(10):
        mark = d.log.mark()
        open_home_item(d, 'Run ROM')
        d.expect(r'Screen enter: Library', mark, 10)
        time.sleep(0.8)
        d.press('a', gap=1500)
        d.press('b', gap=600)
        mark2 = d.log.mark()
        d.press('b', gap=1200)
        d.expect(r'Screen enter: MainMenu', mark2, 10, 'the home screen')
        time.sleep(0.5)
        h = d.heap(mark2, 'MainMenu')
        samples.append(h[0] if h else None)
        if first is None:
            first = h
    r['notes'].append('Free heap on the home screen after each round: %s' % samples)
    good = [s for s in samples if s]
    if len(good) >= 2 and good[0] - good[-1] > 4096:
        r['errors'].append('Free memory fell by %d bytes over 10 rounds' % (good[0] - good[-1]))


def t_leak_music(d, r):
    """Play and leave music from a ROM five times: memory should come back."""
    gbc = next((i for i, (n, c) in enumerate(SYSTEMS or []) if n == 'Game Boy Color' and c), None)
    if gbc is None:
        r['notes'].append('No Game Boy Color list')
        return
    samples = []
    for k in range(5):
        d.restart() if k == 0 else None
        run_rom_open_system(d, gbc, 'Game Boy Color')
        mark = d.log.mark()
        d.press('y')
        if not d.log.wait(r'Core ready to run', mark, 30):
            r['errors'].append('Round %d: the music didn\'t start' % (k + 1))
            return
        time.sleep(2)
        mark2 = d.log.mark()
        exit_core(d)
        time.sleep(1)
        h = d.heap(mark2, 'MainMenu')
        samples.append(h[0] if h else None)
    r['notes'].append('Free heap on the home screen after each music round: %s' % samples)
    good = [s for s in samples if s]
    if len(good) >= 2 and good[0] - good[-1] > 4096:
        r['errors'].append('Free memory fell by %d bytes over 5 rounds' % (good[0] - good[-1]))


TESTS = [
    ('boot', t_boot),
    ('run_rom_lists', t_run_rom_lists),
    ('launch_each_core', t_launch_each_core),
    ('music_from_rom', t_music_from_rom),
    ('tools', t_tools),
    ('settings', t_settings),
    ('leak_run_rom', t_leak_run_rom),
    ('leak_music', t_leak_music),
]


def natural(s):
    return [int(t) if t.isdigit() else t.lower() for t in re.split(r'(\d+)', s)]


def slug(s):
    return re.sub(r'[^a-z0-9]+', '-', s.lower()).strip('-')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out')
    ap.add_argument('--only')
    ap.add_argument('--music-all', action='store_true')
    a = ap.parse_args()
    stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
    out = a.out or os.path.normpath(os.path.join(HERE, '..', '..', '..', 'test-runs', stamp))
    os.makedirs(out, exist_ok=True)
    d = Device(out)
    time.sleep(1)
    info = d.bubctl('info').strip()
    results = []
    only = set(a.only.split(',')) if a.only else None
    for name, fn in TESTS:
        if only and name not in only and not (name == 'run_rom_lists' and SYSTEMS is None and
                                               only & {'launch_each_core', 'music_from_rom', 'leak_music'}):
            continue
        r = {'name': name, 'notes': [], 'errors': [], 'shots': [], 'problems': []}
        mark = d.log.mark()
        start = time.time()
        print('== %s' % name, flush=True)
        try:
            if name == 'music_from_rom':
                fn(d, r, a.music_all)
            else:
                fn(d, r)
        except Failed as e:
            r['errors'].append(str(e))
            r['shots'].append(d.shot('fail-' + name))
        except Exception as e:
            r['errors'].append('test error: %r' % e)
        r['problems'] = d.problems(mark)
        r['notes'] += d.retries
        d.retries = []
        r['secs'] = time.time() - start
        r['ok'] = not r['errors'] and not r['problems']
        results.append(r)
        print('   %s (%.0f s)%s' % ('ok' if r['ok'] else 'FAILED', r['secs'],
                                   ''.join('\n   ! ' + e for e in r['errors'] + r['problems'][:10])), flush=True)
    d.log.close()
    known = [l for _, l in d.log.lines if any(k.search(l) for k in KNOWN)]
    with open(os.path.join(out, 'report.md'), 'w') as f:
        f.write('# Game Bub device tests, %s\n\n%s\n\n' % (stamp, info))
        f.write('| Test | Result | Time |\n|---|---|---|\n')
        for r in results:
            f.write('| %s | %s | %.0f s |\n' % (r['name'], 'ok' if r['ok'] else '**failed**', r['secs']))
        for r in results:
            f.write('\n## %s\n\n' % r['name'])
            for e in r['errors']:
                f.write('- **%s**\n' % e)
            for p in r['problems']:
                f.write('- log: `%s`\n' % p.replace('`', "'"))
            for n in r['notes']:
                f.write('- %s\n' % n)
            shots = [s for s in r['shots'] if s]
            if shots:
                f.write('\nScreenshots: %s\n' % ', '.join(shots))
        if known:
            f.write('\n## Known log noise\n\n- `%s` (%d times)\n' % (known[0], len(known)))
    print('Report: %s' % os.path.join(out, 'report.md'))
    sys.exit(0 if all(r['ok'] for r in results) else 1)


if __name__ == '__main__':
    main()
