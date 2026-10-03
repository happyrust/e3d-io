-- Requirement 5.9, the owner: E3D deletes the EQUI of setup/equi_tree.json, whose BOX,
-- CYLI and SUBE's BOX have no entries, and SAVEWORKs (see delete_box_entryless.pml).
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=none {{SETUP2}}=none {{SETUP3}}=none {{SETUP4}}=none {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - delete of an EQUI over entry-less primitives
={{SETUP0}}
DELETE EQUI
handle any
  $P PROBE-DELETE-ERROR $!!error.text
endhandle
Q REF
