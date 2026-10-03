-- spamap.descendants.pos with the entries in place (requirement 5.8): the preparation gives
-- the primitives of setup/equi_tree.json their entries (LEVE recomputes each one itself),
-- then E3D sets the EQUI's POS. Levels 1 and 2 must be rewritten, the BOX outside kept.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=rewritten {{SETUP2}}=rewritten {{SETUP3}}=none {{SETUP4}}=rewritten {{SETUP5}}=same
-- comment: e3d-io SPAMAP probe d2 - POS of an EQUI over primitives with entries
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
POS E 5100 N 5000 U 0
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q POS
