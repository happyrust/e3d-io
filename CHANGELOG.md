# Changelog

## Unreleased

### Added

- Added `point_list`, the decoder for the point runs catalogue wireframe geometry stores for `LINE.Pts`. `1000` closes the list rather than naming a point — every one of the 82 `LINE` elements in `ams7000_0001` ends with it and no list holds it anywhere else — and the hundreds digit of each item picks the letter, so `[1, 100, 2, 1000]` reads as `P1 T0 P2`. A third hundreds block has never been seen and returns nothing rather than a guessed letter.
- Added noun-level default values, read from chunks 1 and 2 of each `*vir.dat` child — the same data `core.dll` loads through `sub_5AF6AB0` mode 1 and returns from `sub_5B03B90`. 10892 of the 35734 descriptors in the 20 shipped schema files state a default; the reader had been taking chunk 0 only. Attributes resolved this way carry the new `TemplateDefault` status, and `emit-attr-info` records the value in a new `descriptor_default` column beside the derived `default_val`.
- Added `StorageShape`, the per-storage-type element geometry recovered from `core.dll`'s runtime width table and confirmed against every descriptor in the schema files. Types 10, 15 and 19 pack four characters to a word and now decode to text; type 9 packs 32 booleans to a word and now decodes to a `BoolArray`; type 14 takes its element width from the descriptor itself; types 18 and 19 had no decode path at all.

- Added ordered multi-Dictionary UDA loading (`--dictionary-db-list`) and direct materialisation of static and dynamic UDA names/defaults.
- Added typed TTY value comparison and `--reference-db-list` name resolution so complete extraction verifies scalar, unit, position, direction, WORD, and RefNo values.
- Added a v1-compatible metadata emission mode that retains legacy-only entries unchanged while descriptor-backed entries expand coverage, enabling zero-regression dual-read migration.
- Added direct AMS Dictionary UDA catalog extraction from `dicvir.dat`, including UKEY/STUDNA/UTYP/ELEL and type-checked default resolution.
- Added `e3d-descriptor list-uda` and configurable `--template-type` support for non-Design descriptor databases.
- Added a DBMDAT authority regression covering its internal explicit `DBMVAL` and `DBMEXT` payloads.
- Added JSON-free noun descriptor extraction from `desvir.dat`/`attlib.dat`, explicit-tail decoding, report-name recovery, and `extract-complete` TTY `ATTLIST` reconciliation with zero-row-omission accounting.
- Added `TemplateFile` and `TemplateProvider` for reading and caching noun templates from `.dat` files such as `desvir.dat`.
- Added template-provider-based element summarization and element summary diagnostics.
- Added tests for template file loading, template payload parsing, provider caching, metadata normalization, and truncated record handling.

### Changed

- `tests/session_l1_contract.rs` no longer states which session `ams8000_0001` is at. It said 264 and latest page 8897; the sample database is live — E3D had written it to 281 and 9277 — and two assertions were red for it. The session number and both descriptor pointers are now compared with the bytes at their offsets, and the chain checks (newest first, strictly descending, ending at 1) hold on a merged file too.
- Tests that read the installed E3D fixtures now check for them at runtime instead of carrying `#[ignore]`. `#[ignore]` hid four failing assertions behind a green `cargo test`, two of which this changelog lists as regressions. Only the three whole-file scans that take about a minute each stay ignored.
- `summarize_element` and `resolve_attribute` are deprecated: they address attributes at word `6 + atgtdf_position - 1`, where `core.dll` uses the descriptor's own `encoded_location & 0xFFFFF`. The README quick start now uses `extract_element_with_descriptors`.
- `ReadOnlyEngine::open` now initializes `FileInfo` from page 1 metadata instead of synthesizing it from the descriptor.
- Cached discovered B+tree index roots to avoid repeated full-file root scans.
- Clarified the crate description as read-only to match current capabilities.

### Fixed

- An element whose member list arrives in more than one block no longer loses everything behind it. `parse_tail` took one member block and then looked for explicit blocks, so a second member block stopped the walk: BRAN `24383/85432` in the AMS sample reported **no explicit attributes at all**, `NAME` included, along with the only UDA value it stores. A scan of `ams7999_0001` finds 493 of 29920 elements carrying two or more member blocks, 297 of them with an explicit block behind the cut. The failure was silent in the worst way — "stores nothing" and "the reader stopped early" produce identical output — and it took E3D's own `q att` plus the raw bytes to tell them apart.
- A member list split across blocks no longer drops the RefNo that straddles the boundary or invents one. The list splits mid-RefNo just as an attribute stream does: the first block of that BRAN ends with the word `24383` and the second begins with `85434`, which together are member `24383/85434`. Decoding the blocks separately produced a phantom `85434/0`. The two words after each block header are also skipped unconditionally now rather than only when zero — the same block carries `18010/8193` there, and it was being reported as a member.

- A reference array is now named entry by entry instead of being left waiting on resolution forever. `MBOL.Sbrarray` holds ten slots of which two are filled and E3D prints ` 1 /MSAAD 2 /MSAAG`; `SCOM.Blrfarray` holds one and E3D prints `/MSIDBHP-4-24` bare. Empty slots do not appear, and a position is written only where there is more than one entry to tell apart. Nothing is offered unless every filled slot resolved, so a database the lookup was not given still reads as a reference waiting on a name rather than as a disagreement about the value.
- A TTY heading now reaches its attribute through the catalog's exported query text as well as through the name recovered from `attlib.dat`. The recovery walks the metadata record and takes the first token without whitespace, which for some attributes is an editor label rather than a heading: `BANG` yields `BangleDoubleConverter`, so the `Bangle` rows of `q att` reached no attribute at all, and `Ptdiameter` reached `PTDI` — a different attribute — instead of `PTDM`. `QTXT` is the query text itself and needs no recovery, and no two of the 4271 exported attributes share one.
- `Detref` now resolves to `DETA`. No name source holds that heading: attlib and the catalog both call the attribute `Detail`. Two things say it is `DETA`'s anyway. The rows of `q att` follow the catalog's attribute order exactly for `GPART` and `PRTELE`, and `Detref` lands on `DETA`'s slot in both; and `SPCO.DETA` is a two-word reference in the template, which is what `/AAPA100-SR` has to be read from.
- An attribute the element does not store, whose descriptor gives it a two-word reference slot, now defaults to `Nulref` rather than to the zero of whatever the catalog calls the type. `DETA` is declared `BOOL` against a reference slot, and `q att` prints `Nulref` for a `GPART` that stores nothing.
- An empty word slot now agrees with a zero printed in any unit. Which reading the word holds stays undetermined where attlib has no definition for it, but every reading of an empty slot is zero, and E3D writes that zero with whatever unit the attribute carries: `SCTN.Bangle` prints `0degree`.
- Three stored numbers are no longer taken for a point or a direction on the strength of the count alone. `SCOM.Param` holds `[50, 61.2, TUB]`, which E3D prints as the list it is; the vector reading now applies only where the text is a position or a direction, and anything else falls through to the item-by-item list comparison that the seven-item sibling of that row already used.
- A boolean the element does not store now keeps the default its noun declares. The rule that reads an unstored attribute as its type's zero was recovered from 29 rows of the TTY sample, all of them `WORD` or `STRING`; the one `BOOL` among them, `TABGRO.Fixedb`, goes the other way, so the rule is held to the types it was learned from. Of the 27 booleans the sampled nouns declare `true`, only that one and `CCTA.Activ` — which E3D answers with a sentence about active users — ever reach this path.
- An attribute whose stored word is `0x80000000` or `0x80000001` is now reported as having no value. Those are the two markers `core.dll` checks; the reader had been looking for `0xFFFFFFFF`, which is an ordinary value — `Rlock` stores it and E3D prints -1.
- `logical_count == 0` now reads as an array with no declared upper bound, which is how `sub_5AB5BC0` reads it, instead of as a scalar.
- Storage types 14, 17 and 19 are now always arrays, matching `sub_5AB5BC0`.
- Explicit attribute streams split across Dabacon continuation blocks are now reassembled before decoding.
- Element end-marker detection now skips declared member/explicit blocks, preventing aligned integer `7` values from truncating their payloads.
- Dynamic Dictionary UDA names now resolve from `DYUDNA` when `STUDNA` is empty, including PF fabrication and SCH attributes.
- Dictionary catalog scans now retain per-record read/decode issues and continue instead of aborting the entire catalog.
- Explicit integers with WORD storage semantics now decode to named WORD values (for example `FALS`) even when ATGTDF metadata is absent.
- RefNo fallback scanning now covers concatenated physical extents and rejects small page fields that mimic element headers.
- Element end-marker detection now ignores literal integer `7` values inside the declared implicit region, preventing truncated UDA definitions.
- Element record reads now return `TruncatedRecord` when the end marker is missing instead of silently returning partial data.
- Template payload parsing now rejects truncated walks instead of returning partial templates.
