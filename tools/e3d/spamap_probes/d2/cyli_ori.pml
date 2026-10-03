-- spamap.recompute.CYLI, ORI (DCHC 3), on an entry-less CYLI (see cyli_diam.pml).
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - ORI of an entry-less CYLI
={{SETUP0}}
ORI Y is N 10 W and Z is U
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q ORI
