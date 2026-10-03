-- spamap.recompute.BOX, NAME (DCHC 0; a name goes through PostSetName, which does not
-- queue), on an entry-less BOX (see box_pos.pml).
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=none
-- comment: e3d-io SPAMAP probe d2 - NAME of an entry-less BOX
={{SETUP0}}
NAME /E3DIO-PROBE-BOX-NAME
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q NAME
