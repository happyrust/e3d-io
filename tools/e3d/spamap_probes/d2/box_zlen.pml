-- spamap.recompute.BOX, ZLEN (DCHC 4), on an entry-less BOX (see box_pos.pml).
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - ZLEN of an entry-less BOX
={{SETUP0}}
ZLEN 501
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q ZLEN
