-- spamap.recompute.<NOUN> sampled outside BOX / CYLI: one size attribute (DCHC 4) each
-- of an entry-less CONE, DISH and PYRA e3d-io wrote; each must gain its entry.
-- setup: tools/e3d/spamap_probes/setup/others.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}}
-- expect: {{SETUP0}}=appeared {{SETUP1}}=appeared {{SETUP2}}=appeared
-- comment: e3d-io SPAMAP probe d2 - a size of an entry-less CONE, DISH and PYRA
={{SETUP0}}
DTOP 110
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
={{SETUP1}}
DIAM 410
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
={{SETUP2}}
XBOT 310
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
