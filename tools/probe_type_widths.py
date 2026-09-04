"""Recover core.dll's per-type element width from descriptor geometry.

core.dll sizes a value with `dbl_6453DF8[type]` = elements per dword:

    scalar : words = (ratio <= 1) ? 1/ratio : 1
    array  : words = ceil(count / ratio), plus one count word

`dbl_6453DF8` is initialised at runtime so it cannot be read out of the IDB.
The same ratio is recoverable from the descriptors themselves: for each
storage type, check which `elements_per_word` makes
`storage_words == 1 + ceil(logical_count / ratio)` hold for arrays, or
`storage_words == 1 / ratio` for scalars.
"""

import collections
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from probe_default_chunks import SCHEMA_BINARIES, TemplateFile, descriptors  # noqa: E402

CANDIDATES = [0.5, 1.0, 4.0, 32.0]


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else SCHEMA_BINARIES
    # per type: Counter of (shape, ratio) that explains the row
    fits = collections.defaultdict(collections.Counter)
    totals = collections.Counter()
    examples = {}

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
                t = d["type"]
                n = d["logical_count"]
                w = d["wide"][0]
                totals[t] += 1
                examples.setdefault(t, (path.name, n, w, d["compact"][0]))
                matched = False
                for ratio in CANDIDATES:
                    if n > 1 and w == 1 + math.ceil(n / ratio):
                        fits[t][f"array/{ratio}"] += 1
                        matched = True
                    if n <= 1 and ratio <= 1.0 and w == int(1 / ratio):
                        fits[t][f"scalar/{ratio}"] += 1
                        matched = True
                    if n <= 1 and ratio > 1.0 and w == 1:
                        fits[t][f"scalar/{ratio}"] += 1
                        matched = True
                if not matched:
                    fits[t]["unexplained"] += 1

    print(f"{'type':>4} {'total':>7}  best fits")
    for t in sorted(totals):
        best = ", ".join(f"{k}={v}" for k, v in fits[t].most_common(4))
        print(f"{t:>4} {totals[t]:>7}  {best}")

    # Type 14 sizes each element from the descriptor itself:
    #   words_per_element = (compact.storage_words - 1) / logical_count
    print("\ntype 14: (compact_words - 1) / logical_count")
    stride = collections.Counter()
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
                if d["type"] != 14:
                    continue
                n = d["logical_count"]
                cw = d["compact"][0]
                if n == 0:
                    stride["logical_count == 0"] += 1
                    continue
                exact = (cw - 1) % n == 0
                stride[f"{(cw - 1) // n}{'' if exact else ' (not exact)'}"] += 1
    for k, v in sorted(stride.items()):
        print(f"    {k}: {v}")


if __name__ == "__main__":
    main()
