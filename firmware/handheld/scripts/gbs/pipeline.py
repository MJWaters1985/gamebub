"""GBS rip -> checked player: convert, compare with the rip on the ROM, repair, generate."""
import os, sys, json
sys.path.insert(0, os.path.dirname(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
from gbs2entry import convert
from gbsdiff import find_rom, S
from verify import verify, repair_immediates
from banks import bankmap
import gen_music_player as gm

X = S + '/gbs-dl/x'
OUT = S + '/gbs/players'
os.makedirs(OUT, exist_ok=True)


def run(n):
    rp = find_rom(n)
    if not rp:
        return dict(status='no ROM')
    rom = open(rp, 'rb').read()
    g, tracks, t, entry, problems, notes, cl = convert(os.path.join(X, n), rom, plays=30)
    if not entry:
        return dict(status='not converted', why=problems[:2])
    k = len(tracks)
    songs = sorted({tracks[i][0] for i in (0, k // 4, k // 2, 3 * k // 4, k - 1)})
    res = verify(g, entry, rom, songs, frames=90)
    fixes = []
    if any(ok != tot for s, ok, tot, d in res):
        bm = {b: rb for b, rb, sc, l in bankmap(g, rom)}
        entry, fixes = repair_immediates(g, entry, rom, bm, songs[:2])
        if fixes:
            res = verify(g, entry, rom, songs, frames=90)
    exact = sum(1 for s, ok, tot, d in res if ok == tot)
    out = dict(rom=os.path.basename(rp), tracks=k, exact=exact, checked=len(res), fixes=fixes,
               problems=problems, notes=notes,
               diffs=['%d: %d/%d %s' % (s, ok, tot, (d or '')[:60]) for s, ok, tot, d in res if ok != tot])
    if exact == len(res):
        try:
            data, size = gm.generate_game(entry, n)
            name = n.split(' [')[0].split(' (')[0] + '.bin'
            open(os.path.join(OUT, name), 'wb').write(data)
            out.update(status='verified', size=size, player=name)
        except AssertionError as ex:
            out.update(status='verified, too big', why=[str(ex).split(': ')[-1]])
    else:
        out.update(status='differs')
    return out


if __name__ == '__main__':
    results = {}
    for n in sorted(os.listdir(X)):
        if len(sys.argv) > 1 and sys.argv[1].lower() not in n.lower():
            continue
        try:
            r = run(n)
        except Exception as ex:
            r = dict(status='error', why=[repr(ex)[:120]])
        results[n] = r
        print('%-34s %-18s %s' % (n[:34], r['status'], ' '.join(str(x) for x in (r.get('why') or r.get('diffs') or r.get('fixes') or [])))[:110], flush=True)
    json.dump(results, open(S + '/gbs/results.json', 'w'), indent=1)
