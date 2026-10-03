-- spamap.recompute.BOX, member order: E3D puts the second of two entry-less BOXes in an EQUI
-- before the first (REORDER inserts through DINSER, which refuses a BOX under a ZONE).
-- PreReorderElement / PostReorderElement queue the reordered element (descases VDESR1 /
-- VDESRO), so it gains its entry; the other members are recorded.
-- setup: tools/e3d/spamap_probes/setup/equi_members.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}} {{SETUP3}} {{SETUP4}}
-- expect: {{SETUP0}}=none {{SETUP2}}=appeared
-- comment: e3d-io SPAMAP probe d2 - reorder of an entry-less BOX
={{SETUP0}}
REORDER ={{SETUP2}} BEFORE ={{SETUP1}}
handle any
  $P PROBE-REORDER-ERROR $!!error.text
endhandle
Q MEM
