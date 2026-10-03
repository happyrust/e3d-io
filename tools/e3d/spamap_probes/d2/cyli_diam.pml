-- spamap.recompute.CYLI, DIAM (DCHC 4), on an entry-less CYLI e3d-io wrote: "appeared" =
-- the write triggers the recompute, "none" = it does not.
-- setup: tools/e3d/spamap_probes/setup/cyli.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=appeared
-- comment: e3d-io SPAMAP probe d2 - DIAM of an entry-less CYLI
={{SETUP0}}
DIAM 201
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DIAM
