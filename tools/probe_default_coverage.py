"""How many descriptors across all 20 *vir.dat files carry an authoritative
default in chunk 1 / chunk 2, i.e. what the crate leaves unread.

Counts the same (db, noun, attr) triples the crate's `emit-attr-info` emits.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from probe_default_chunks import TemplateFile, descriptors  # noqa: E402


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else r"E:\reverse\e3d\shadow_e3d31_aps_all")
    files = sorted(p for p in root.glob("*vir.dat"))

    grand = {"triples": 0, "implicit_default": 0, "explicit_default": 0, "none": 0, "implicit_resident": 0, "implicit_no_chunk1": 0}
    print(f"{'file':<12} {'nouns':>6} {'descs':>7} {'impl_def':>9} {'expl_def':>9} {'no_def':>8}")
    for path in files:
        tf = TemplateFile(path)
        try:
            _tt, kids = tf.children()
        except Exception as exc:  # noqa: BLE001
            print(f"{path.name:<12} skipped: {exc}")
            continue
        n_desc = n_imp = n_exp = n_none = 0
        for k in kids:
            tok0, sz0 = k["chunks"][0]
            if not sz0:
                continue
            try:
                payload = tf.read_chunked(tok0, sz0)
                imp = tf.read_chunked(*k["chunks"][1]) if k["chunks"][1][1] else []
                exp = tf.read_chunked(*k["chunks"][2]) if k["chunks"][2][1] else []
            except ValueError:
                continue
            explicit = set()
            i = 0
            while i + 1 < len(exp):
                explicit.add(exp[i])
                i += (exp[i + 1] & 0x3FFFFFF) + 2
            for d in descriptors(payload):
                n_desc += 1
                off = d["wide"][1] & 0xFFFFF
                if off:
                    grand["implicit_resident"] += 1
                if off and 0 <= off - 11 < len(imp):
                    n_imp += 1
                elif not off and d["hash"] in explicit:
                    n_exp += 1
                else:
                    n_none += 1
                    if off:
                        grand["implicit_no_chunk1"] += 1
        print(f"{path.name:<12} {len(kids):>6} {n_desc:>7} {n_imp:>9} {n_exp:>9} {n_none:>8}")
        grand["triples"] += n_desc
        grand["implicit_default"] += n_imp
        grand["explicit_default"] += n_exp
        grand["none"] += n_none

    total = grand["triples"]
    covered = grand["implicit_default"] + grand["explicit_default"]
    print()
    print(f"triples                    {total}")
    print(f"authoritative default      {covered}  ({covered / total:.1%})")
    print(f"  from chunk 1 (implicit)  {grand['implicit_default']}")
    print(f"  from chunk 2 (explicit)  {grand['explicit_default']}")
    print(f"no stored default          {grand['none']}")
    print(f"implicit-resident triples  {grand['implicit_resident']}  "
          f"(wide word_offset != 0)")
    print(f"  of those, chunk 1 short  {grand['implicit_no_chunk1']}")


if __name__ == "__main__":
    main()
