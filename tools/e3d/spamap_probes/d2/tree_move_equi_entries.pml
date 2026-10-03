-- spamap.descendants.move with the entries in place (requirement 5.3; preparation as in
-- tree_equi_pos_entries.pml), then E3D moves the EQUI to the second ZONE. Recorded, not
-- expected: when both ZONEs place it alike the boxes do not change and MAPINS keeps them.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP3}}=none {{SETUP5}}=same
-- comment: e3d-io SPAMAP probe d2 - move of an EQUI over primitives with entries
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
/1RX03-MCT
INCLUDE ={{SETUP0}}
handle any
  $P PROBE-INCLUDE-ERROR $!!error.text
endhandle
={{SETUP0}}
Q OWNER
Q POS WRT /*
