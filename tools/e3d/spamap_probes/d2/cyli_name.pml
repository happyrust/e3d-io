-- spamap.recompute.CYLI, NAME (DCHC 0), on an entry-less CYLI (see cyli_diam.pml).
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=none
-- comment: e3d-io SPAMAP probe d2 - NAME of an entry-less CYLI
={{SETUP0}}
NAME /E3DIO-PROBE-CYLI-NAME
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q NAME
