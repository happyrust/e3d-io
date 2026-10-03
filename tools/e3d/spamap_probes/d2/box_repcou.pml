-- spamap.recompute.BOX, REPCOU (DCHC 0), on an entry-less BOX (see box_pos.pml).
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=none
-- comment: e3d-io SPAMAP probe d2 - REPCOU of an entry-less BOX
={{SETUP0}}
Q REPCOU
REPCOU 2
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q REPCOU
