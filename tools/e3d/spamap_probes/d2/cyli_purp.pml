-- spamap.recompute.CYLI, PURP (DCHC 0), on an entry-less CYLI (see cyli_diam.pml).
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=none
-- comment: e3d-io SPAMAP probe d2 - PURP of an entry-less CYLI
={{SETUP0}}
PURP PIPE
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q PURP
