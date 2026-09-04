"""Census of the three per-noun chunks in a *vir.dat template file.

Chunk 0 is the descriptor payload the crate already reads. core.dll's
sub_5AF6AB0 mode 1 loads chunks 1 and 2 instead, and sub_5B03B90 (GCEAD)
reads attribute defaults out of them: chunk 1 is an implicit-shaped record
indexed at (wide_word_offset - 11), chunk 2 is a [hash][header][payload]
stream walked with (header & 0x3FFFFFF) + 2.
"""

import struct
import sys
from pathlib import Path

PAGE_BYTES = 2048
PAGE_WORDS = 512
CHUNK_DATA_WORDS = 511

# The 3.1 schema pack shipped in this repository, which the other probes import
# rather than each naming an installation of their own. `binaries/` is
# committed, so a clone has the files; `schema/README.md` covers the rest.
SCHEMA_BINARIES = Path(__file__).resolve().parents[1] / "schema" / "e3d31" / "binaries"


class TemplateFile:
    def __init__(self, path):
        self.data = Path(path).read_bytes()
        self.pages = len(self.data) // PAGE_BYTES

    def page(self, page_no):
        off = page_no * PAGE_BYTES
        return struct.unpack(">512I", self.data[off:off + PAGE_BYTES])

    def read_chunked(self, token, total):
        out = []
        remaining = total
        while remaining > 0:
            if token == 0 or token - 1 >= self.pages:
                raise ValueError(f"bad token {token}")
            page = self.page(token - 1)
            if remaining > CHUNK_DATA_WORDS:
                out.extend(page[:CHUNK_DATA_WORDS])
                token = page[CHUNK_DATA_WORDS]
                remaining -= CHUNK_DATA_WORDS
            else:
                out.extend(page[:remaining])
                remaining = 0
        return out

    def children(self):
        p0 = self.page(0)
        magic, template_type, child_count, tok1 = p0[0], p0[2], p0[5], p0[7]
        assert magic == 6, magic
        words = self.read_chunked(tok1, child_count * 7)
        out = []
        for i in range(child_count):
            c = words[i * 7:i * 7 + 7]
            out.append({"hash": c[0], "chunks": [(c[1], c[2]), (c[3], c[4]), (c[5], c[6])]})
        return template_type, out


def descriptors(payload):
    """Walk chunk 0 the way NounTemplate::parse_template_payload does."""
    if len(payload) < 14:
        return []
    count = payload[9]
    cur = 14
    out = []
    for _ in range(count):
        if cur + 10 > len(payload):
            break
        stride = payload[cur + 1]
        if stride < 10 or cur + stride > len(payload):
            break
        out.append({
            "hash": payload[cur],
            "type": payload[cur + 2],
            "logical_count": payload[cur + 3],
            "compact": payload[cur + 4:cur + 7],
            "wide": payload[cur + 7:cur + 10],
        })
        cur += stride
    return out


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else SCHEMA_BINARIES / "desvir.dat"
    tf = TemplateFile(path)
    template_type, kids = tf.children()
    print(f"file={Path(path).name} template_type={template_type} children={len(kids)}")

    have = [0, 0, 0]
    sizes = [0, 0, 0]
    for k in kids:
        for i, (_tok, sz) in enumerate(k["chunks"]):
            if sz:
                have[i] += 1
                sizes[i] += sz
    for i in range(3):
        print(f"  chunk{i}: {have[i]}/{len(kids)} children carry it, {sizes[i]} dwords total")

    # Type-code census over chunk 0, plus how often logical_count==0.
    types = {}
    zero_count = 0
    total = 0
    for k in kids:
        tok, sz = k["chunks"][0]
        if not sz:
            continue
        try:
            payload = tf.read_chunked(tok, sz)
        except ValueError:
            continue
        for d in descriptors(payload):
            total += 1
            types[d["type"]] = types.get(d["type"], 0) + 1
            if d["logical_count"] == 0:
                zero_count += 1
    print(f"  descriptors={total}  logical_count==0: {zero_count}")
    print("  storage_type_code histogram:")
    for t in sorted(types):
        print(f"    {t:>3}: {types[t]}")


if __name__ == "__main__":
    main()
