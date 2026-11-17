"""What cartridge music would do for each ROM in a collection, as the device does it: look the
game up in the package index by its No-Intro name (the file name), match the packages against
the ROM, and for a match compare the player's sound register writes with the rip's.

rom_report.py <package dir> <out.tsv> [jobs]
ROMs come from CART_MUSIC_ROMS; <work>/gbs/package_all.json (from package_all.py) gives why a
rip has no package.
"""
import os, sys, json, multiprocessing

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gbsdiff import S, ROMS, norm
from package_all import X, titles

PKG = None


def rom_name(path):
    return os.path.splitext(os.path.basename(path))[0]


def work(args):
    path, cands, sets = args
    import signal

    def timeout(*_):
        raise TimeoutError('took over 600 s')
    signal.signal(signal.SIGALRM, timeout)
    signal.alarm(600)
    try:
        from package import match, make, parse
        from verify import verify
        rom = open(path, 'rb').read()
        mbc1 = rom[0x147] in (1, 2, 3)
        if not cands:
            return path, 'no package', ''
        reasons = []
        for f in cands:
            pkg = open(os.path.join(PKG, f), 'rb').read()
            player, why = match(pkg, rom, mbc1)
            if not player:
                reasons.append('%s: %s' % (f, why))
                continue
            # sound check: rebuild the entry from the rip, run the matched player's code on the ROM
            name = sets.get(f)
            if not name:
                return path, 'matches', f
            pkg2, g, tracks, problems, entry = make(os.path.join(X, name))
            P = parse(pkg)
            raw_at = P['table_at'] - (entry['raw_len'] - P['tsize'])
            addr = entry['raw_blobs'][0][0]
            e2 = dict(entry, blobs=[(addr, player[raw_at:raw_at + entry['raw_len']])])
            songs = [i for i, _, _ in entry['tracks']]
            pick = sorted({songs[0], songs[len(songs) // 2], songs[-1]})
            res = verify(g, e2, rom, pick, 90)
            exact = sum(1 for r in res if r[1] == r[2])
            return path, 'plays' if exact == len(res) else 'plays, some songs differ', \
                '%s: %d/%d songs exact' % (f, exact, len(res))
        return path, 'refused', '; '.join(reasons)
    except BaseException as ex:
        return path, 'error', repr(ex)[:200]
    finally:
        signal.alarm(0)


def init(pkg_dir):
    global PKG
    PKG = pkg_dir


if __name__ == '__main__':
    pkg_dir, out = sys.argv[1], sys.argv[2]
    jobs = int(sys.argv[3]) if len(sys.argv) > 3 else os.cpu_count()
    index = {}
    for line in open(os.path.join(pkg_dir, 'index.txt')):
        if '\t' in line:
            n, f = line.rstrip('\n').split('\t')
            index.setdefault(n, []).append(f)
    status = json.load(open(S + '/gbs/package_all.json'))
    sets = {r['file']: n for n, r in status.items() if 'file' in r}
    by_title = {}
    for n in status:
        for t in titles(n):
            by_title.setdefault(t, []).append(n)
    roms = [r for r in ROMS if r.lower().endswith(('.gb', '.gbc'))]
    work_list = []
    for r in roms:
        n = norm(rom_name(r))
        cands = list(dict.fromkeys(index.get(n, [])))
        work_list.append((r, cands, sets))
    rows = []
    with multiprocessing.Pool(jobs, initializer=init, initargs=(pkg_dir,)) as pool:
        for path, result, detail in pool.imap_unordered(work, work_list):
            if result == 'no package':
                rips = by_title.get(norm(rom_name(path)), [])
                if rips:
                    why = [p for n in rips for p in status[n].get('problems', [])]
                    result, detail = 'rip not packaged', '; '.join(why)[:200]
                else:
                    result, detail = 'no rip', ''
            rows.append((rom_name(path), result, detail))
            print('%-50s %-26s %s' % (rom_name(path)[:50], result, detail[:90]), flush=True)
    rows.sort()
    with open(out, 'w') as f:
        f.write('rom\tresult\tdetail\n')
        for r in rows:
            f.write('\t'.join(r) + '\n')
    from collections import Counter
    print(Counter(r[1] for r in rows))
