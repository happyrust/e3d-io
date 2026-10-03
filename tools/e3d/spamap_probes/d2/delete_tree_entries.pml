-- spamap.delete_residue, levels 1 and 2 (requirement 5.4): the preparation gives the
-- primitives of setup/equi_tree.json their entries (LEVE), then E3D deletes the EQUI.
-- Recorded per level; the BOX outside must keep its entry.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP3}}=none {{SETUP5}}=same
-- comment: e3d-io SPAMAP probe d2 - delete of an EQUI over primitives with entries
={{SETUP1}}
LEVE 1 9
handle any
  $P PROBE-PREPARE-ERROR $!!error.text
endhandle
={{SETUP2}}
LEVE 1 9
handle any
  $P PROBE-PREPARE-ERROR $!!error.text
endhandle
={{SETUP4}}
LEVE 1 9
handle any
  $P PROBE-PREPARE-ERROR $!!error.text
endhandle
={{SETUP5}}
LEVE 1 9
handle any
  $P PROBE-PREPARE-ERROR $!!error.text
endhandle
-- prepare-end
={{SETUP0}}
DELETE EQUI
handle any
  $P PROBE-DELETE-ERROR $!!error.text
endhandle
Q REF
