"""Print the descriptor prefix for named attributes of a named noun."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from probe_default_chunks import TemplateFile, descriptors  # noqa: E402
from probe_noun_defaults import db1_dehash, hash_name  # noqa: E402


def main():
    path = sys.argv[1]
    noun = sys.argv[2]
    wanted = set(sys.argv[3:])
    tf = TemplateFile(path)
    _tt, kids = tf.children()
    child = next((k for k in kids if k["hash"] == hash_name(noun)), None)
    if child is None:
        print(f"{noun} absent from {Path(path).name}")
        return
    payload = tf.read_chunked(*child["chunks"][0])
    for d in descriptors(payload):
        name = db1_dehash(d["hash"])
        if wanted and name not in wanted:
            continue
        print(f"{noun}.{name}: type={d['type']} logical_count={d['logical_count']} "
              f"compact={d['compact']} wide={d['wide']}")


if __name__ == "__main__":
    main()
