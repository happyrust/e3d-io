-- spamap.recompute.CYLI, LEVE (DCHC 4), on an entry-less CYLI (see cyli_diam.pml).
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - LEVE of an entry-less CYLI
={{SETUP0}}
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q LEVE
