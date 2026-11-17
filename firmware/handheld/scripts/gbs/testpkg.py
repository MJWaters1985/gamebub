"""Package each rip that has a ROM, match it as the device would, and check the result against
the rip (sound register writes, frame by frame)."""
import os, sys, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from package import make, match, parse, entry_for
from gbsdiff import find_rom, S
from verify import verify

X = S + '/gbs-dl/x'
out = {}
for n in sorted(os.listdir(X)):
    if len(sys.argv) > 1 and sys.argv[1].lower() not in n.lower():
        continue
    rp = find_rom(n)
    if not rp:
        continue
    rom = open(rp, 'rb').read()
    try:
        pkg, g, tracks, problems, entry = make(os.path.join(X, n))
    except Exception as ex:
        print('%-34s error %r' % (n[:34], ex)); continue
    if not pkg:
        print('%-34s not packaged: %s' % (n[:34], '; '.join(problems)[:80])); continue
    player, why = match(pkg, rom, mbc1=rom[0x147] in (1, 2, 3))
    if not player:
        print('%-34s no match: %s (%d B)' % (n[:34], why, len(pkg))); continue
    P = parse(pkg)
    e2 = entry_for(entry, player, P)
    k = len(tracks)
    songs = sorted({tracks[i][0] for i in (0, k // 2, k - 1)})
    res = verify(g, e2, rom, songs, frames=60)
    good = sum(1 for s, ok, tot, d in res if ok == tot)
    print('%-34s matched, %d/%d songs exact, %d B, %d checks' % (n[:34], good, len(res), len(pkg), len(P['checks'])))
    if good == len(res):
        os.makedirs(S + '/gbs/pkgplayers', exist_ok=True)
        open(S + '/gbs/pkgplayers/' + n.split(' [')[0].split(' (')[0] + '.bin', 'wb').write(player)
