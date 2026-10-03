-- An owner's attribute that does not place anything but has DCHC 4: NUMB of the EQUI of
-- setup/equi_tree.json. VDESPT queues the EQUI (IMAP 2) and MPBOX walks its subtree, so
-- by D0 every level should be recomputed; this checks it.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=appeared {{SETUP2}}=appeared {{SETUP3}}=none {{SETUP4}}=appeared {{SETUP5}}=none
-- comment: e3d-io SPAMAP probe d2 - NUMB of an EQUI over entry-less primitives
={{SETUP0}}
Q NUMB
NUMB 7
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q NUMB
