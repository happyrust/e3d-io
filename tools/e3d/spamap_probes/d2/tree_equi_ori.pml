-- spamap.descendants.ori (requirement 5.8): E3D sets the ORI of the EQUI of
-- setup/equi_tree.json (levels as in tree_equi_pos.pml).
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=appeared {{SETUP2}}=appeared {{SETUP3}}=none {{SETUP4}}=appeared {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - ORI of an EQUI over entry-less primitives
={{SETUP0}}
ORI Y is N 10 W and Z is U
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q ORI
