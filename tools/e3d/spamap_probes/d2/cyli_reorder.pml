-- spamap.recompute.CYLI, member order (see box_reorder.pml): E3D puts the second entry-less
-- CYLI in the EQUI before the first; it gains its entry, the other members are recorded.
-- setup: tools/e3d/spamap_probes/setup/equi_members.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}}
-- expect: {{SETUP0}}=none {{SETUP4}}=appeared
-- comment: e3d-io SPAMAP probe d2 - reorder of an entry-less CYLI
={{SETUP0}}
REORDER ={{SETUP4}} BEFORE ={{SETUP3}}
handle any
  $P PROBE-REORDER-ERROR $!!error.text
endhandle
Q MEM
