-- spamap.descendants.pos one level down: E3D sets the POS of the SUBE of
-- setup/equi_tree.json; only its BOX (SETUP4) is below it.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=none {{SETUP2}}=none {{SETUP3}}=none {{SETUP4}}=appeared {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - POS of a SUBE over an entry-less BOX
={{SETUP3}}
POS E 0 N 1100 U 0
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q POS
