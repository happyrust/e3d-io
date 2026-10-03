-- spamap.descendants.move, the moved element itself (requirement 5.3): E3D moves the BOX
-- outside the EQUI of setup/equi_tree.json (SETUP5) into the SUBE (INCLUDE inserts through
-- DINSER, which refuses a BOX under a ZONE).
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP3}}=none {{SETUP5}}=appeared
-- comment: e3d-io SPAMAP probe d2 - move of an entry-less BOX
={{SETUP3}}
INCLUDE ={{SETUP5}}
handle any
  $P PROBE-INCLUDE-ERROR $!!error.text
endhandle
={{SETUP5}}
Q OWNER
