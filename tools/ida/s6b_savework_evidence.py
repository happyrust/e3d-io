"""Task 13.2 follow-up (S6b page-1 bucket table): the SAVEWORK write order and the
page-1 claim's life cycle. Reads the routines through IB_Wrapper and keeps the
unmodified responses, then tabulates page 0 / page 1 words from the validated
probe snapshots. Run from the repository root; writes
.agents/tasks/write-path-phase-3-s6b/savework-order/."""
import json
from pathlib import Path
import subprocess

FUNCS = {
    0x5b15e10: 'session page and page 0 at SAVEWORK',
    0x5af0fb0: 'switch the file mode (FHSWIT QXMO / MODIFY / READ)',
    0x5af2260: 'write one page (FHDBWN)',
    0x5af2a70: 'select the db block',
    0x5af2520: 'write out the dirty pages',
    0x5af1590: 'end a write batch',
    0x5af54b0: "release the current user's claim (2UP1UOR a7=0)",
    0x5b0b6e0: 'EXPUNGE (EXPNG + 1, 2UP1UOR on every extent, a7=0)',
    0x5abe980: 'EXPUNGE dispatch: users not in the list',
    0x5abeb60: 'EXPUNGE dispatch: one user, or all',
    0x5ae68d0: 'per-database save (2UP1UOR a7=1)',
    0x5b12760: 'after the bucket update at save',
    0x5aeb2f0: 'end of SAVEWORK',
    0x5ae4dd0: 'anything to save',
}
XREFS = [0x5b0b6e0, 0x5af54b0]
CASES = ['live', 'deleted', 'gap']
SNAPSHOTS = Path('docs/evidence/assets/2026-10-05-s6b/validated')
# Page 0 words (src/meta/descriptor.rs): LSPGNO, NXTBKT, EXPNG, HCCNT.
PAGE0_WORDS = {'LSPGNO': 10, 'NXTBKT': 12, 'EXPNG': 95, 'HCCNT': 96}

out = Path('.agents/tasks/write-path-phase-3-s6b/savework-order')
out.mkdir(parents=True, exist_ok=True)


def ib(*args):
    run = subprocess.run(['pwsh', '-NoProfile', '-File', 'tools/ida/ib.ps1', *args], capture_output=True)
    if run.returncode:
        raise SystemExit(f'ib {args[0]} exit {run.returncode}: {run.stderr.decode(errors="replace")}')
    return run.stdout


def word(page_hex, index):
    return int(page_hex[index * 8:index * 8 + 8], 16)


(out / 'ib-first.log').write_bytes(ib('env'))
eas = ', '.join(hex(ea) for ea in FUNCS)
response = json.loads(ib('--timeout-s', '600', 'exec', '--json', '--sql',
                         f'SELECT func_ea, n, line FROM pseudocode WHERE func_ea IN ({eas}) ORDER BY func_ea, n'))
rows = response['result']['rows']
(out / 'pseudocode.txt').write_text('\n'.join(f'{row["func_ea"]} {row["n"]}: {row["line"]}' for row in rows),
                                    encoding='utf-8')
print(f'pseudocode: {len(rows)} rows from {len({row["func_ea"] for row in rows})} of {len(FUNCS)} functions')
for ea in XREFS:
    (out / f'xrefs-{ea:#x}.json').write_bytes(ib('xrefs', hex(ea)))
print(f'xrefs: {len(XREFS)} addresses')
(out / 'ib-last.log').write_bytes(ib('env'))
print((out / 'ib-last.log').read_text(encoding='utf-8', errors='replace'))

table = ['case\tsnapshot\t' + '\t'.join(PAGE0_WORDS) + '\tpage1 [extent,counter,third]']
for case in CASES:
    pages = json.loads((SNAPSHOTS / case / 'pages.json').read_text(encoding='utf-8'))
    for name, snapshot in pages.items():
        page0, page1 = snapshot['page0_hex'], snapshot['page1_hex']
        entries = (word(page1, 2) - 2) // 3
        cells = ' '.join(f'[{word(page1, 3 + 3 * k)},{word(page1, 4 + 3 * k)},{word(page1, 5 + 3 * k)}]'
                         for k in range(entries))
        table.append(f'{case}\t{name}\t' + '\t'.join(str(word(page0, i)) for i in PAGE0_WORDS.values()) + f'\t{cells}')
(out / 'snapshots.tsv').write_text('\n'.join(table) + '\n', encoding='utf-8')
print('\n'.join(table))
