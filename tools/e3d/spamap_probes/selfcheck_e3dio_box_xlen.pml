-- The other known case (docs/evidence/2026-09-22-e3d-spamap-consequences.md section 0
-- item 5, session 648): a BOX e3d-io wrote has no entry until E3D changes it. E3D sets the
-- XLEN of write_gate_ops/10_box.json's BOX (E 1000 N 2000 U 3000) 300 -> 301 and writes
-- key0 300008495, value [8495, 18000, 27500, 11505, 22000, 32500, 2].
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe selfcheck - XLEN of a BOX e3d-io wrote
={{SETUP0}}
Q REF
Q XLEN
XLEN 301
handle any
  $P PROBE-XLEN-ERROR $!!error.text
endhandle
Q XLEN
