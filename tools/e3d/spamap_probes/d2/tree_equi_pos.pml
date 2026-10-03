-- spamap.descendants.pos (requirement 5.8): E3D sets the POS of an EQUI e3d-io wrote.
-- SETUP0 EQUI, SETUP1 BOX and SETUP2 CYLI in it (level 1), SETUP3 SUBE in it, SETUP4 BOX in
-- the SUBE (level 2), SETUP5 a BOX outside, all without entries: a level whose elements
-- gain entries is recomputed.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=appeared {{SETUP2}}=appeared {{SETUP3}}=none {{SETUP4}}=appeared {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - POS of an EQUI over entry-less primitives
={{SETUP0}}
POS E 5100 N 5000 U 0
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q POS
