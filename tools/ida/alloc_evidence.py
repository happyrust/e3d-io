"""Read allocation routines through IB_Wrapper; retain the unmodified response."""
import json
from pathlib import Path
import subprocess

out = Path('.agents/tasks/write-path-phase-3-alloc/validation')
out.mkdir(parents=True, exist_ok=True)
for name, args in [
    ('ib-before', ['env']),
    ('allocation', ['exec', '--json', '--sql',
     'SELECT func_ea, n, line FROM pseudocode WHERE func_ea IN '
     '(0x5aadd90, 0x5aaaac0, 0x5904cd0, 0x59144e0, 0x5ac3810, '
     '0x5af60f0, 0x5af59e0, 0x5abd470, 0x5ab2d40, 0x5aea9c0, 0x5af7480, '
     '0x5afdf10, 0x5b01bf0, 0x5b00f10, 0x5b010f0) ORDER BY func_ea, n']),
    ('index-instructions', ['exec', '--json', '--sql',
     'SELECT address, disasm FROM instructions WHERE func_ea = 0x5b00f10 ORDER BY address']),
    ('ib-after', ['env']),
]:
    run = subprocess.run(['pwsh', '-NoProfile', '-File', 'tools/ida/ib.ps1', *args],
                         capture_output=True)
    (out / (name + '.log')).write_bytes(run.stdout + run.stderr)
    if run.returncode:
        raise SystemExit(f'{name} exit {run.returncode}: {run.stderr.decode(errors="replace")}')
    if name == 'allocation':
        response = json.loads(run.stdout)
        rows = response['result']['rows']
        (out / 'allocation.txt').write_text('\n'.join(
            f'{row["func_ea"]} {row["n"]}: {row["line"]}' for row in rows), encoding='utf-8')
        print(f'{name}: {len(rows)} pseudocode rows')
    elif name == 'index-instructions':
        rows = json.loads(run.stdout)['result']['rows']
        (out / 'index-instructions.txt').write_text('\n'.join(
            f'{row["address"]}: {row["disasm"]}' for row in rows), encoding='utf-8')
        print(f'{name}: {len(rows)} instructions')
    else:
        print(run.stdout.decode(errors='replace'))
