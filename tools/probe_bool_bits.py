"""Which storage types actually use the packed-bit field of encoded_location.

core.dll (sub_5AB5600, sub_5B03B90) extracts a packed bit only when
descriptor[2] == 5; the crate's descriptor.rs also treats type 9 that way.
This counts, per storage type, how many descriptors have a non-zero
(encoded_location >> 20).
"""

import collections
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from probe_default_chunks import SCHEMA_BINARIES, TemplateFile, descriptors  # noqa: E402
from probe_noun_defaults import db1_dehash  # noqa: E402


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else SCHEMA_BINARIES
    stats = collections.defaultdict(collections.Counter)
    nine = []
    for path in sorted(root.glob("*vir.dat")):
        tf = TemplateFile(path)
        try:
            _tt, kids = tf.children()
        except Exception:  # noqa: BLE001
            continue
        for k in kids:
            tok, sz = k["chunks"][0]
            if not sz:
                continue
            try:
                payload = tf.read_chunked(tok, sz)
            except ValueError:
                continue
            for d in descriptors(payload):
                loc = d["wide"][1]
                stats[d["type"]]["n"] += 1
                if loc >> 20:
                    stats[d["type"]]["hi"] += 1
                if d["type"] == 9:
                    nine.append((path.name, db1_dehash(k["hash"]), db1_dehash(d["hash"]),
                                 d["logical_count"], hex(loc), d["wide"][0]))

    print(f"{'type':>4} {'count':>7} {'loc>>20 != 0':>14}")
    for t in sorted(stats):
        print(f"{t:>4} {stats[t]['n']:>7} {stats[t]['hi']:>14}")
    print("\nevery type-9 descriptor (file, noun, attr, logical_count, wide loc, wide words):")
    for row in nine:
        print("  ", row)


if __name__ == "__main__":
    main()
