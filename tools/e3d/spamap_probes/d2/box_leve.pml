-- spamap.recompute.BOX, LEVE (DCHC 4), on an entry-less BOX (see box_pos.pml). LEVE
-- does not move the box, so on a BOX with an entry the recompute would not show.
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - LEVE of an entry-less BOX
={{SETUP0}}
Q LEVE
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q LEVE
