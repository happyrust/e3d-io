-- D3 (task 7.5) control for recompute_counterexamples.pml, whose LEVE left all 18
-- entries as they were: each element grows by 100 mm (BOX XLEN, CYLI DIAM), so its new
-- box leaves the stored one and MAPINS must rewrite the entry. The rewritten entries are
-- E3D's own boxes for the final state, compared with Built_In_AABB afterwards
-- (spamap-check --refno on the probed copy). Run with -DbRelative ams000\ams7999_0001.
-- watch: 24383/78369 24383/78372 24383/78299 24383/78302 24383/76689 24383/76692
-- watch: 24383/76700 24383/76703 24383/76711 24383/76714 24383/76685 24383/76696
-- watch: 24383/76707 24383/77889 24383/71552 24383/71555 24383/77136 24383/77023
-- expect: 24383/78369=rewritten 24383/78372=rewritten 24383/78299=rewritten 24383/78302=rewritten
-- expect: 24383/76689=rewritten 24383/76692=rewritten 24383/76700=rewritten 24383/76703=rewritten
-- expect: 24383/76711=rewritten 24383/76714=rewritten 24383/76685=rewritten 24383/76696=rewritten
-- expect: 24383/76707=rewritten 24383/77889=rewritten 24383/71552=rewritten 24383/71555=rewritten
-- expect: 24383/77136=rewritten 24383/77023=rewritten
-- comment: e3d-io SPAMAP probe d3 - grow the Built_In_AABB counterexamples by 100 mm
=24383/78369
handle any
  $P PROBE-NAV-ERROR 24383/78369 $!!error.text
endhandle
Q XLEN
XLEN 205
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/78372
handle any
  $P PROBE-NAV-ERROR 24383/78372 $!!error.text
endhandle
Q XLEN
XLEN 205
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/78299
handle any
  $P PROBE-NAV-ERROR 24383/78299 $!!error.text
endhandle
Q XLEN
XLEN 205
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/78302
handle any
  $P PROBE-NAV-ERROR 24383/78302 $!!error.text
endhandle
Q XLEN
XLEN 205
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76689
handle any
  $P PROBE-NAV-ERROR 24383/76689 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76692
handle any
  $P PROBE-NAV-ERROR 24383/76692 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76700
handle any
  $P PROBE-NAV-ERROR 24383/76700 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76703
handle any
  $P PROBE-NAV-ERROR 24383/76703 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76711
handle any
  $P PROBE-NAV-ERROR 24383/76711 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76714
handle any
  $P PROBE-NAV-ERROR 24383/76714 $!!error.text
endhandle
Q XLEN
XLEN 760
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76685
handle any
  $P PROBE-NAV-ERROR 24383/76685 $!!error.text
endhandle
Q XLEN
XLEN 1025
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76696
handle any
  $P PROBE-NAV-ERROR 24383/76696 $!!error.text
endhandle
Q XLEN
XLEN 1025
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/76707
handle any
  $P PROBE-NAV-ERROR 24383/76707 $!!error.text
endhandle
Q XLEN
XLEN 1025
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/77889
handle any
  $P PROBE-NAV-ERROR 24383/77889 $!!error.text
endhandle
Q XLEN
XLEN 525
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q XLEN
=24383/71552
handle any
  $P PROBE-NAV-ERROR 24383/71552 $!!error.text
endhandle
Q DIAM
DIAM 276.74
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DIAM
=24383/71555
handle any
  $P PROBE-NAV-ERROR 24383/71555 $!!error.text
endhandle
Q DIAM
DIAM 276.74
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DIAM
=24383/77136
handle any
  $P PROBE-NAV-ERROR 24383/77136 $!!error.text
endhandle
Q DIAM
DIAM 220
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DIAM
=24383/77023
handle any
  $P PROBE-NAV-ERROR 24383/77023 $!!error.text
endhandle
Q DIAM
DIAM 250
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
Q DIAM
