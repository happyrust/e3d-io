-- Every reference source to :to_ea except ordinary flow: the SQL side of
-- IB_Xref_Helper (tools/ida/xrefs_to.py). tools/ida/check_xrefs.ps1 puts the
-- address in for :to_ea. type_name is NULL on codes ida-bridge does not name.
SELECT from_ea FROM xrefs WHERE to_ea = :to_ea AND (type_name IS NULL OR type_name <> 'flow') ORDER BY from_ea
