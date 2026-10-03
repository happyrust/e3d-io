-- spamap.recompute.CYLI, POS (DCHC 3), on an entry-less CYLI (see cyli_diam.pml).
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - POS of an entry-less CYLI
={{SETUP0}}
POS E 1100 N 2000 U 3000
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q POS
