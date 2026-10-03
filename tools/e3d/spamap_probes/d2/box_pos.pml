-- spamap.recompute.BOX, POS (DCHC 3). The BOX e3d-io wrote has no entry, so it gains
-- one exactly when E3D recomputes it: "appeared" = the write triggers, "none" = it does not.
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - POS of an entry-less BOX
={{SETUP0}}
POS E 1100 N 2000 U 3000
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q POS
