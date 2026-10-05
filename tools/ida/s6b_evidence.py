"""Task 13.1 (S6b page-1 bucket table): read the routines through IB_Wrapper and keep
the unmodified responses. Run from the repository root; writes
.agents/tasks/write-path-phase-3-s6b/validation/."""
import json
from pathlib import Path
import subprocess

FUNCS = {
    0x59144e0: 'DB_DB::updateBucketList',
    0x59ef020: 'MDUPDB',
    0x53e4d58: 'rcf1/RCFGO',
    0x5912b60: 'DB_DB::setMetaData',
    0x5aaaac0: 'db_add_bucket (routine 358)',
    0x5aadd90: 'db_get_bucket (routine 360)',
    0x5904cd0: 'DB_DB::getRefBuckets',
    0x5abd470: 'routine 597 dispatch',
    0x5199255: 'dbmodlib/BCREAT',
    0x5ab2d40: 'block creation',
    0x5aea9c0: 'next refno from control block attribute 7',
    0x5af7480: '2GNE claim a page-1 bucket',
    0x5afdf10: 'key insert',
    0x5b017f0: 'leaf insert (529 on a present key)',
    0x5af1a00: 'write a page back',
    0x5ae5770: 'update the claimed bucket at save',
    0x5afd040: '2UP1UOR update or release a page-1 entry',
    0x5afba00: '2RBE remove a page-1 entry',
    0x5af59e0: '2FUB take a page-1 entry (routine 360)',
}
XREFS = [0x59144e0, 0x59ef020, 0x5aea9c0, 0x5af7480, 0x5b017f0, 0x5afd040, 0x5ae5770, 0x5ab2d40]

out = Path('.agents/tasks/write-path-phase-3-s6b/validation')
out.mkdir(parents=True, exist_ok=True)


def ib(*args):
    run = subprocess.run(['pwsh', '-NoProfile', '-File', 'tools/ida/ib.ps1', *args], capture_output=True)
    if run.returncode:
        raise SystemExit(f'ib {args[0]} exit {run.returncode}: {run.stderr.decode(errors="replace")}')
    return run.stdout


(out / 'ib-before.log').write_bytes(ib('env'))
eas = ', '.join(hex(ea) for ea in FUNCS)
response = json.loads(ib('--timeout-s', '300', 'exec', '--json', '--sql',
                         f'SELECT func_ea, n, line FROM pseudocode WHERE func_ea IN ({eas}) ORDER BY func_ea, n'))
rows = response['result']['rows']
(out / 'pseudocode.txt').write_text('\n'.join(f'{row["func_ea"]} {row["n"]}: {row["line"]}' for row in rows),
                                    encoding='utf-8')
print(f'pseudocode: {len(rows)} rows from {len({row["func_ea"] for row in rows})} of {len(FUNCS)} functions')
for ea in XREFS:
    (out / f'xrefs-{ea:#x}.json').write_bytes(ib('xrefs', hex(ea)))
print(f'xrefs: {len(XREFS)} addresses')
(out / 'ib-after.log').write_bytes(ib('env'))
print((out / 'ib-after.log').read_text(encoding='utf-8', errors='replace'))
