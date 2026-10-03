-- spamap.recompute.<NOUN> sampled outside BOX / CYLI: NAME (DCHC 0) of an entry-less
-- CONE, DISH and PYRA; none may gain an entry.
-- setup: tools/e3d/spamap_probes/setup/others.json
-- watch: {{SETUP0}} {{SETUP1}} {{SETUP2}}
-- expect: {{SETUP0}}=none {{SETUP1}}=none {{SETUP2}}=none
-- comment: e3d-io SPAMAP probe d2 - NAME of an entry-less CONE, DISH and PYRA
={{SETUP0}}
NAME /E3DIO-PROBE-CONE-NAME
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
={{SETUP1}}
NAME /E3DIO-PROBE-DISH-NAME
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
={{SETUP2}}
NAME /E3DIO-PROBE-PYRA-NAME
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
