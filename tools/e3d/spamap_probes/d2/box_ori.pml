-- spamap.recompute.BOX, ORI (DCHC 3), on an entry-less BOX (see box_pos.pml).
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - ORI of an entry-less BOX
={{SETUP0}}
ORI Y is N 10 W and Z is U
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q ORI
