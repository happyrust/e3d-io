-- spamap.delete_residue, the element itself (requirement 5.4): the preparation gives the
-- BOX e3d-io wrote its entry (LEVE), then E3D deletes the BOX. Recorded: "vanished" = the
-- delete took the entry out, "same" = it stays behind.
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- comment: e3d-io SPAMAP probe d2 - delete of a BOX with an entry
={{SETUP0}}
LEVE 1 9
handle any
  $P PROBE-PREPARE-ERROR $!!error.text
endhandle
-- prepare-end
={{SETUP0}}
DELETE BOX
handle any
  $P PROBE-DELETE-ERROR $!!error.text
endhandle
Q REF
