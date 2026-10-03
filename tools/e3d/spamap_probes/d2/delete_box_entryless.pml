-- Requirement 5.9, the element itself: E3D deletes a BOX e3d-io wrote, which has no entry,
-- and SAVEWORKs. What counts is whether the SAVEWORK goes through (the summary's SAVEWORK
-- session, or the error E3D gives).
-- setup: tools/e3d/write_gate_ops/10_box.json
-- watch: {{SETUP0}}
-- expect: {{SETUP0}}=none
-- comment: e3d-io SPAMAP probe d2 - delete of an entry-less BOX
={{SETUP0}}
DELETE BOX
handle any
  $P PROBE-DELETE-ERROR $!!error.text
endhandle
Q REF
