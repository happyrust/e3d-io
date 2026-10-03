-- spamap.descendants.ori with the entries in place (requirement 5.8; preparation as in
-- tree_equi_pos_entries.pml), then E3D sets the EQUI's ORI.
-- setup: tools/e3d/spamap_probes/setup/equi_tree.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}} {{SETUP5}}
-- expect: {{SETUP0}}=none {{SETUP1}}=rewritten {{SETUP2}}=rewritten {{SETUP3}}=none {{SETUP4}}=rewritten {{SETUP5}}=same
-- comment: e3d-io SPAMAP probe d2 - ORI of an EQUI over primitives with entries
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
ORI Y is N 10 W and Z is U
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q ORI
