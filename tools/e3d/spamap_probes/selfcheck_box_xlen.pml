-- The known case (requirement 5.2; docs/evidence/2026-09-22-e3d-spamap-consequences.md,
-- session 648): E3D sets the XLEN of the native BOX 24384/24777 and rewrites its entry.
-- watch: 24384/24777
-- expect: 24384/24777=rewritten
-- comment: e3d-io SPAMAP probe selfcheck - XLEN of the native BOX 24384/24777
=24384/24777
Q REF
Q XLEN
XLEN 333
handle any
  $P PROBE-XLEN-ERROR $!!error.text
endhandle
Q XLEN
