-- An owner's attribute with DCHC 0: DESC of the EQUI of setup/equi_tree.json; nothing may
-- gain an entry.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=none {{SETUP2}}=none {{SETUP3}}=none {{SETUP4}}=none {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - DESC of an EQUI over entry-less primitives
={{SETUP0}}
DESC 'e3d-io SPAMAP probe'
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DESC
