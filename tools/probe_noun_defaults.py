"""Decode a noun's default values out of desvir.dat chunks 1 and 2.

Mirrors core.dll sub_5B03B90 (GCEAD):

  sub_5AF6AB0(db, noun, mode=1, &implicit_defaults, ..., &explicit_defaults, &n)
    -> mode 1 loads child chunk index 1 into the implicit slot
       and child chunk index 2 into the explicit slot

  implicit default word = implicit_defaults[(wide.encoded_location & 0xFFFFF) - 11]
  packed BOOL default   = (that word >> (wide.encoded_location >> 20)) & 1
  array default         = count at that word, items from the next word on
  explicit default      = walk [hash][header][payload...],
                          advance by (header & 0x3FFFFFF) + 2
"""

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from probe_default_chunks import SCHEMA_BINARIES, TemplateFile, descriptors  # noqa: E402

HASH_BASE = 0x81BF1
UDA_BASE = 0x171FAD39


def db1_dehash(value):
    if value > UDA_BASE:
        k = (value - UDA_BASE) % 0x1000000
        out = [":"]
        for _ in range(6):
            if k <= 0:
                break
            out.append(chr(k % 64 + 32))
            k //= 64
        return "".join(out)
    if value <= HASH_BASE:
        return ""
    k = value - HASH_BASE
    out = []
    while k > 0:
        out.append(chr(k % 27 + 64))
        k //= 27
    return "".join(out)


def hash_name(name):
    v = 0
    for ch in reversed(name):
        v = v * 27 + (ord(ch) - 64)
    return v + HASH_BASE


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else SCHEMA_BINARIES / "desvir.dat"
    noun = sys.argv[2] if len(sys.argv) > 2 else "BRAN"
    tf = TemplateFile(path)
    _tt, kids = tf.children()

    target = hash_name(noun)
    child = next((k for k in kids if k["hash"] == target), None)
    if child is None:
        print(f"noun {noun} (hash {target}) not in {Path(path).name}")
        print("first few child hashes:", [hex(k['hash']) for k in kids[:5]])
        return
    print(f"noun={noun} hash={target} chunks={child['chunks']}")

    payload = tf.read_chunked(*child["chunks"][0])
    descs = descriptors(payload)
    print(f"descriptors={len(descs)}")

    imp_tok, imp_sz = child["chunks"][1]
    exp_tok, exp_sz = child["chunks"][2]
    imp = tf.read_chunked(imp_tok, imp_sz) if imp_sz else []
    exp = tf.read_chunked(exp_tok, exp_sz) if exp_sz else []
    print(f"chunk1 (implicit defaults) = {len(imp)} dwords, "
          f"chunk2 (explicit defaults) = {len(exp)} dwords")

    # Index the explicit default stream the way GCEAD walks it.
    explicit = {}
    i = 0
    while i + 1 < len(exp):
        h = exp[i]
        hdr = exp[i + 1]
        length = hdr & 0x3FFFFFF
        explicit[h] = (hdr >> 26, exp[i + 2:i + 2 + length])
        i += length + 2

    print(f"explicit default entries = {len(explicit)}")

    implicit_hits = 0
    explicit_hits = 0
    shown = 0
    for d in descs:
        loc = d["wide"][1]
        off = loc & 0xFFFFF
        name = db1_dehash(d["hash"])
        if off:
            idx = off - 11
            if 0 <= idx < len(imp):
                implicit_hits += 1
                word = imp[idx]
                if d["type"] == 5:
                    val = f"bool {(word >> (loc >> 20)) & 1}"
                elif d["type"] in (2, 6):
                    lo, hi = word, imp[idx + 1] if idx + 1 < len(imp) else 0
                    val = f"real {struct.unpack('<d', struct.pack('<II', lo, hi))[0]!r}"
                else:
                    val = f"word 0x{word:08X}"
                if shown < 14:
                    print(f"  implicit {name:<8} type={d['type']:<3} off={off:<4} -> {val}")
                    shown += 1
        elif d["hash"] in explicit:
            explicit_hits += 1
            t, words = explicit[d["hash"]]
            if shown < 14:
                print(f"  explicit {name:<8} type={d['type']:<3} exptype={t:<3} "
                      f"-> {[hex(w) for w in words[:4]]}")
                shown += 1

    print(f"descriptors with an implicit default = {implicit_hits}")
    print(f"descriptors with an explicit default = {explicit_hits}")


if __name__ == "__main__":
    main()
