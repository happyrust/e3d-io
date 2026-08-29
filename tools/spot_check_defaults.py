"""Spot-check the `descriptor_default` column of an emit-attr-info document."""

import collections
import json
import sys

WANTED = {"HBOR", "PURP", "BUIL", "HPOS", "CACHID", "LCHKDA", "NAME", "DESC", "TPRESS"}


def main():
    doc = json.load(open(sys.argv[1], encoding="utf-8"))
    kinds = collections.Counter()
    shown = 0
    for nouns in doc["by_db"].values():
        for attrs in nouns.values():
            for a in attrs.values():
                v = a.get("descriptor_default")
                if v is None:
                    continue
                kinds[type(v).__name__] += 1
                if shown < 10 and a["name"] in WANTED:
                    print(f"{a['name']:<8} type={a['storage_type_code']:<3} "
                          f"descriptor={json.dumps(v, ensure_ascii=False)[:44]:<46} "
                          f"derived={json.dumps(a['default_val'], ensure_ascii=False)[:40]}")
                    shown += 1
    print()
    print("descriptor_default json kinds:", dict(kinds))


if __name__ == "__main__":
    main()
