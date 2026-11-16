"""Package every rip in <work>/gbs-dl/x for /system/cart-music/rips on the SD card.

package_all.py <out dir> [jobs]
Writes <out>/<name>.cmr, <out>/index.txt ("<normalized title>\t<file>", for the English title
and the other titles in brackets in the rip set's name) and <work>/gbs/package_all.json.
"""
import os, re, sys, json, multiprocessing

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gbsdiff import S, norm

X = S + '/gbs-dl/x'


def titles(set_name):
    english = re.split(r' \[| \(', set_name)[0]
    alts = [a for a in re.findall(r'\[([^\]]+)\]', set_name) if a not in ('GB', 'GBC', 'SGB')]
    out = []
    for t in [english] + alts:
        n = norm(t)
        if n and n not in out:
            out.append(n)
    return out


def slug(set_name):
    s = re.sub(r'[^A-Za-z0-9]+', '_', re.split(r' \[| \(', set_name)[0]).strip('_')
    sysname = re.findall(r'\[(GB|GBC|SGB)\]$', set_name)
    date = re.findall(r'\((\d{4}[-\d]*)\)', set_name)
    return '%s_%s%s' % (s[:40], (date[0] if date else '').replace('-', ''), ('_' + sysname[0]) if sysname else '')


def work(set_name):
    import signal
    signal.alarm(300)   # a rip whose tracing never ends
    try:
        from package import make
        pkg, g, tracks, problems, entry = make(os.path.join(X, set_name))
        if not pkg:
            return set_name, None, problems[:2]
        return set_name, pkg, len(tracks)
    except BaseException as ex:
        return set_name, None, ['error: %r' % ex][:1]


if __name__ == '__main__':
    out = sys.argv[1]
    jobs = int(sys.argv[2]) if len(sys.argv) > 2 else os.cpu_count()
    os.makedirs(out, exist_ok=True)
    sets = sorted(n for n in os.listdir(X)
                  if any(f.lower().endswith('.gbs') for f in os.listdir(os.path.join(X, n))))
    results, index = {}, []
    with multiprocessing.Pool(jobs) as pool:
        for i, (name, pkg, info) in enumerate(pool.imap_unordered(work, sets)):
            if pkg:
                f = slug(name) + '.cmr'
                open(os.path.join(out, f), 'wb').write(pkg)
                for t in titles(name):
                    index.append('%s\t%s' % (t, f))
                results[name] = dict(file=f, tracks=info)
            else:
                results[name] = dict(problems=info)
            if i % 50 == 0:
                print(i, len(sets), sum(1 for r in results.values() if 'file' in r), flush=True)
    open(os.path.join(out, 'index.txt'), 'w').write('\n'.join(sorted(index)) + '\n')
    json.dump(results, open(S + '/gbs/package_all.json', 'w'), indent=1)
    ok = sum(1 for r in results.values() if 'file' in r)
    print('packaged %d of %d' % (ok, len(sets)))
