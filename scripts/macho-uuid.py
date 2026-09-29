#!/usr/bin/env python3
"""Read, blank or rebuild the LC_UUID load command of a Mach-O (thin or universal).

Tier 2 of the reproducible-build check (scripts/repro-app-check.sh) compares the
Mach-O files inside two builds of Aether.app. Everything is comparable except
the UUID: ld64's -reproducible still derives LC_UUID from its link inputs, and
the ad-hoc signature that covers the UUID is keyed by the output file's
basename, so a build in another checkout directory comes out with a different
UUID (and a different signature page). Zeroing that one field lets the check
compare the remaining bytes exactly; the UUID itself is reported, not compared,
because Xcode's own link keeps it (dSYM lookup needs it).

`rebuild` is the other half: it is how a release Rust binary (scripts/repro-env.sh
aether_repro_fix_uuid) gets a UUID that follows its code instead of the build
directory, so the binary itself is byte-identical too.

    macho-uuid.py print FILE...   # one UUID per architecture, or "none"
    macho-uuid.py zero FILE...    # overwrite the UUID field(s) with zeroes
    macho-uuid.py rebuild FILE... # set them to a digest of the file (see below)
"""
import hashlib
import struct
import sys

MH_MAGIC = 0xFEEDFACE  # 32-bit, in the file's byte order
MH_MAGIC_64 = 0xFEEDFACF
FAT_MAGIC = 0xCAFEBABE  # these two are always big-endian
FAT_MAGIC_64 = 0xCAFEBABF
LC_UUID = 0x1B
UUID_LEN = 16


def arch_slices(data):
    """(offset, length) of every thin image; one entry for a non-fat file."""
    magic_be = struct.unpack_from(">I", data, 0)[0]
    if magic_be in (MH_MAGIC, MH_MAGIC_64) or \
       struct.unpack_from("<I", data, 0)[0] in (MH_MAGIC, MH_MAGIC_64):
        return [(0, len(data))]
    if magic_be not in (FAT_MAGIC, FAT_MAGIC_64):
        raise ValueError("not a Mach-O file")
    wide = magic_be == FAT_MAGIC_64
    step = 32 if wide else 20
    slices = []
    for i in range(struct.unpack_from(">I", data, 4)[0]):
        head = 8 + i * step + 8  # cputype/cpusubtype, then offset/size
        if wide:
            offset, length = struct.unpack_from(">QQ", data, head)
        else:
            offset, length = struct.unpack_from(">II", data, head)
        slices.append((offset, length))
    return slices


def image_header(data, base):
    """(endian, header length) of the thin image at base."""
    for endian in ("<", ">"):
        magic = struct.unpack_from(endian + "I", data, base)[0]
        if magic == MH_MAGIC_64:
            return endian, 32
        if magic == MH_MAGIC:
            return endian, 28
    raise ValueError("not a Mach-O image")


def uuid_format(raw):
    h = raw.hex().upper()
    return "%s-%s-%s-%s-%s" % (h[:8], h[8:12], h[12:16], h[16:20], h[20:])


def uuid_offsets(data):
    """Offsets of every UUID payload in the file, fat slices included."""
    found = []
    for base, _ in arch_slices(data):
        endian, off = image_header(data, base)
        for _ in range(struct.unpack_from(endian + "I", data, base + 16)[0]):
            cmd, size = struct.unpack_from(endian + "II", data, base + off)
            if size < 8:
                raise ValueError("bad load command size")
            if cmd == LC_UUID:
                found.append(base + off + 8)
            off += size
    return found


def show(path):
    data = open(path, "rb").read()
    values = [uuid_format(data[o:o + UUID_LEN]) for o in uuid_offsets(data)]
    return " ".join(values) if values else "none"


def blank(path):
    data = bytearray(open(path, "rb").read())
    for o in uuid_offsets(data):
        data[o:o + UUID_LEN] = bytes(UUID_LEN)
    with open(path, "wb") as fp:
        fp.write(data)


def rebuild(path):
    """Replace every UUID with a digest of the file's own bytes.

    The file must have been signed after this, since the hash covers the UUID:
    the signature that was there when it was read is part of the digest and is
    gone by the time this returns. A digest of the code makes the UUID a build
    id -- equal for equal code, different for different code -- which is what
    reproducible releases need and what dyld wants to find (macOS 26 refuses to
    run a Mach-O without LC_UUID, so dropping the command is not an option).
    """
    data = bytearray(open(path, "rb").read())
    offsets = uuid_offsets(data)
    if not offsets:
        raise ValueError("no LC_UUID load command")
    for off in offsets:
        data[off:off + UUID_LEN] = bytes(UUID_LEN)
    digest = hashlib.sha256(data).digest()
    for off in offsets:
        data[off:off + UUID_LEN] = digest[:UUID_LEN]
    with open(path, "wb") as fp:
        fp.write(data)


def main(argv):
    if len(argv) < 3 or argv[1] not in ("print", "zero", "rebuild"):
        sys.exit("usage: macho-uuid.py print|zero|rebuild FILE...")
    for path in argv[2:]:
        try:
            if argv[1] == "print":
                print(f"{path}: {show(path)}")
            elif argv[1] == "zero":
                blank(path)
            else:
                rebuild(path)
        except ValueError as err:
            sys.exit(f"{path}: {err}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
