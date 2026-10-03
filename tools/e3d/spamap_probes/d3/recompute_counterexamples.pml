-- D3 (task 7.5): E3D recomputes the 18 Built_In_AABB counterexamples of ams7999_0001
-- (docs/evidence/2026-10-03-spamap-aabb.md). Run with -DbRelative ams000\ams7999_0001.
-- LEVE has DCHC 4 and moves no box: setting it queues the element (descases/VDESPT), the
-- SAVEWORK recomputes it from its final state (MPBOX), and MAPINS rewrites an entry that
-- is more than 1 off. Set twice so that at least one set changes the value.
-- watch: 24383/78369 24383/78372 24383/78299 24383/78302 24383/76689 24383/76692
-- watch: 24383/76700 24383/76703 24383/76711 24383/76714 24383/76685 24383/76696
-- watch: 24383/76707 24383/77889 24383/71552 24383/71555 24383/77136 24383/77023
-- expect: 24383/78369=rewritten 24383/78372=rewritten 24383/78299=rewritten 24383/78302=rewritten
-- expect: 24383/76689=rewritten 24383/76692=rewritten 24383/76700=rewritten 24383/76703=rewritten
-- expect: 24383/76711=rewritten 24383/76714=rewritten 24383/76685=rewritten 24383/76696=rewritten
-- expect: 24383/76707=rewritten 24383/77889=rewritten 24383/71552=rewritten 24383/71555=rewritten
-- expect: 24383/77136=rewritten 24383/77023=rewritten
-- comment: e3d-io SPAMAP probe d3 - LEVE of the Built_In_AABB counterexamples
=24383/78369
handle any
  $P PROBE-NAV-ERROR 24383/78369 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/78372
handle any
  $P PROBE-NAV-ERROR 24383/78372 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/78299
handle any
  $P PROBE-NAV-ERROR 24383/78299 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/78302
handle any
  $P PROBE-NAV-ERROR 24383/78302 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76689
handle any
  $P PROBE-NAV-ERROR 24383/76689 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76692
handle any
  $P PROBE-NAV-ERROR 24383/76692 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76700
handle any
  $P PROBE-NAV-ERROR 24383/76700 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76703
handle any
  $P PROBE-NAV-ERROR 24383/76703 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76711
handle any
  $P PROBE-NAV-ERROR 24383/76711 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76714
handle any
  $P PROBE-NAV-ERROR 24383/76714 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76685
handle any
  $P PROBE-NAV-ERROR 24383/76685 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76696
handle any
  $P PROBE-NAV-ERROR 24383/76696 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/76707
handle any
  $P PROBE-NAV-ERROR 24383/76707 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/77889
handle any
  $P PROBE-NAV-ERROR 24383/77889 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/71552
handle any
  $P PROBE-NAV-ERROR 24383/71552 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/71555
handle any
  $P PROBE-NAV-ERROR 24383/71555 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/77136
handle any
  $P PROBE-NAV-ERROR 24383/77136 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
=24383/77023
handle any
  $P PROBE-NAV-ERROR 24383/77023 $!!error.text
endhandle
Q LEVE
LEVE 2 8
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
LEVE 1 9
handle any
  $P PROBE-SET-ERROR $!!error.text
endhandle
