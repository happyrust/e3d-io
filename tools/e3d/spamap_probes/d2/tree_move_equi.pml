-- spamap.descendants.move (requirement 5.3): E3D moves the EQUI of setup/equi_tree.json to
-- the second ZONE (INCLUDE); levels as in tree_equi_pos.pml.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=appeared {{SETUP2}}=appeared {{SETUP3}}=none {{SETUP4}}=appeared {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - move of an EQUI over entry-less primitives
/1RX03-MCT
INCLUDE ={{SETUP0}}
handle any
  $P PROBE-INCLUDE-ERROR $!!error.text
endhandle
={{SETUP0}}
Q OWNER
