"""IB_Xref_Helper (requirement 1.6 of .kiro/specs/write-path-phase-3): every
code and data reference to one address, ordinary flow left out, sorted by
source address.

Runs inside an IDA instance. `tools/ida/ib.ps1 xrefs 0x<address>` executes
this file and then `_result_ = xrefs_to(0x<address>)`: ida-bridge runs -f
files before -c, so the file defines and -c calls.

Stops at BADADDR or at LIMIT references; in the second case the result says
`truncated`. The SQL side of the same question is
`tools/ida/queries/xrefs_to.sql`; `tools/ida/check_xrefs.ps1` compares them.
"""

import ida_funcs
import ida_idaapi
import ida_xref

LIMIT = 100_000

# Leaves out ordinary flow. Newer SDKs call it XREF_NOFLOW; older ones only
# have XREF_FAR, with the same value.
_NO_FLOW = getattr(ida_xref, "XREF_NOFLOW", None)
if _NO_FLOW is None:
    _NO_FLOW = ida_xref.XREF_FAR


def xrefs_to(ea):
    rows = []
    truncated = False
    block = ida_xref.xrefblk_t()
    more = block.first_to(ea, _NO_FLOW)
    while more:
        if block.frm == ida_idaapi.BADADDR:
            break
        if len(rows) == LIMIT:
            truncated = True
            break
        func = ida_funcs.get_func(block.frm)
        rows.append(
            {
                "from": hex(block.frm),
                "kind": "code" if block.iscode else "data",
                "type": int(block.type),
                "func": ida_funcs.get_func_name(func.start_ea) or "" if func else "",
            }
        )
        more = block.next_to()
    rows.sort(key=lambda row: int(row["from"], 16))
    result = {"to": hex(ea), "count": len(rows), "xrefs": rows}
    if truncated:
        result["truncated"] = True
    return result
