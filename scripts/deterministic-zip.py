#!/usr/bin/env python3
"""Pack files into a byte-reproducible zip (release artifacts, gap G5).

Plain `zip` records each entry's mtime, the traversal order and the file mode,
so packing the same files twice gives two hashes. Here every entry gets the
timestamp from SOURCE_DATE_EPOCH (UTC), the entries are sorted, and the mode is
fixed at 0644, so the same inputs give the same bytes on any machine with the
same zlib.

    SOURCE_DATE_EPOCH=<epoch> scripts/deterministic-zip.py OUT.zip DIR ENTRY...

ENTRY is a file or directory relative to DIR; directories are walked in sorted
order. Files whose basename contains ".test." are left out (tests are not part
of a shipped extension). ARC names are relative to DIR.
"""
import os
import sys
import time
import zipfile


def entries(root):
    if os.path.isfile(root):
        yield root
        return
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for name in sorted(filenames):
            yield os.path.join(dirpath, name)


def main(argv):
    if len(argv) < 4:
        sys.exit(__doc__.strip().splitlines()[0])
    out, src, roots = os.path.abspath(argv[1]), argv[2], argv[3:]
    try:
        stamp = time.gmtime(int(os.environ["SOURCE_DATE_EPOCH"]))[:6]
    except (KeyError, ValueError):
        sys.exit("set SOURCE_DATE_EPOCH (see scripts/repro-env.sh)")

    os.chdir(src)
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for root in roots:
            for path in entries(root):
                if ".test." in os.path.basename(path):
                    continue
                zi = zipfile.ZipInfo(path, stamp)
                zi.external_attr = 0o644 << 16
                with open(path, "rb") as fp:
                    zf.writestr(zi, fp.read())


if __name__ == "__main__":
    main(sys.argv)
