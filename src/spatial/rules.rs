//! Confirmed rules of the SPAMAP codec (`.kiro/specs/write-path-phase-3`,
//! group 7): each function does what the E3D_Core routine its Evidence_Doc
//! names does, and reaches the codec only as a confirmed slot of
//! `RuleSet::confirmed` in `src/core_rules.rs`.
//!
//! Evidence: `docs/evidence/2026-10-03-spamap-encoding.md` — Core3D.dll's
//! `maplib/MAPINS`, `MAPKEY` and `MAPKY2`, core.dll's table code;
//! `docs/evidence/2026-10-03-spamap-coverage.md` — Core3D.dll's
//! `mapboxlib/MPBOX`, `maplist/MLSADD`, `dbboxlib/BOXELE` and the `IMAP` of
//! the noun dictionary; `docs/evidence/2026-10-03-spamap-branch-entries.md`
//! — Core3D.dll's `mapboxlib/MPBXBR` and the callers of `MBXGO`.

use std::collections::HashMap;
use std::sync::OnceLock;

use e3d_attlib::db1_hash;

use super::{Axis, EntryBoxes, SpatialState, UndecodableAttr};
use crate::core_rules::candidate;
use crate::record::descriptor::DescriptorValue;
use crate::refno::RefNo;

// --- spamap.rounding ---------------------------------------------------------------

/// Each of min and max to the nearest 0.1 mm, halves away from zero. Core3D
/// `maplib/MAPINS` takes `f_ldnint_val(10 × (coordinate − datum × 10^8))`,
/// Fortran DNINT, and the datum is 0 on every axis while [`SIGNED_WORDS`]
/// holds the coordinates.
pub fn round_half_away_from_zero(min_mm: f64, max_mm: f64) -> (i64, i64) {
    ((min_mm * 10.0).round() as i64, (max_mm * 10.0).round() as i64)
}

// --- spamap.word_sign --------------------------------------------------------------

/// The largest coordinate magnitude, in 0.1 mm, an entry holds with a datum of
/// 0. `MAPINS` gives an axis a datum once its bound of larger magnitude reaches
/// 10^8 mm; this side of 10^9 (0.1 mm) that cannot have happened.
pub const WITHIN_DATUM: i64 = 999_999_999;

/// Signed 32-bit words: `MAPINS` stores INTEGER*4 coordinates, `MAPKY2` keeps
/// Key0 within [5 × 10^7, 1.85 × 10^9], and core.dll compares key words
/// signed. A coordinate past [`WITHIN_DATUM`] would take a datum, which the
/// entry does not hold, so it is out of reach.
pub const SIGNED_WORDS: candidate::WordSign =
    candidate::WordSign { encode: encode_signed, decode: decode_signed, range: (-WITHIN_DATUM, WITHIN_DATUM) };

fn encode_signed(value: i64) -> Option<u32> {
    (-WITHIN_DATUM..=WITHIN_DATUM).contains(&value).then_some(value as i32 as u32)
}

fn decode_signed(word: u32) -> i64 {
    i64::from(word as i32)
}

// --- spamap.short_axis and spamap.bucket ---------------------------------------------

/// The short-axis extent, in 0.1 mm, below which buckets 1 to 5 end.
const BUCKET_CEILINGS: [i64; 5] = [600, 2_000, 6_000, 20_000, 200_000];

/// The least extent, in 0.1 mm, from which `MAPKEY` takes bucket 6 on x.
const BUCKET_6_FLOOR: i64 = 200_000;

/// `MAPKEY` on the rounded box: z when its extent is at or below both others,
/// else y when at or below both others, else x — the last of tied axes; and x
/// whenever that extent is 20 m or more.
pub fn least_axis_last_of_ties(extents: [i64; 3]) -> Axis {
    let [x, y, z] = extents;
    let axis = if z <= x && z <= y {
        Axis::Z
    } else if y <= x && y <= z {
        Axis::Y
    } else {
        Axis::X
    };
    if extents[axis as usize] >= BUCKET_6_FLOOR {
        Axis::X
    } else {
        axis
    }
}

/// An extent of 0 is nothing apart: `MAPKEY` buckets it like any under 600.
pub fn leave_zero_extent_to_the_bucket(_axis: Axis) -> Option<u8> {
    None
}

/// `MAPKEY`: below 600, 2 000, 6 000, 20 000 and 200 000 (0.1 mm) buckets 1 to
/// 5, a boundary in the upper bucket; from 200 000 bucket 6.
pub fn bucket_by_mapkey(extent: i64) -> Option<u8> {
    if extent < 0 {
        return None;
    }
    let below = BUCKET_CEILINGS.iter().position(|&ceiling| extent < ceiling);
    Some(below.map_or(6, |index| index as u8 + 1))
}

// --- spamap.key0 -------------------------------------------------------------------

/// `MAPKEY` / `MAPKY2`: the short axis' min, remainder by 5 × 10^7 (truncated,
/// as Fortran MOD), held within ±5 × 10^7, plus 10^8 × (6 × axis + bucket).
/// Every min is represented; only a bucket no rule gives could overflow.
pub fn key0_by_mapky2(axis: Axis, bucket: u8, min: i64) -> Result<u32, (i64, i64)> {
    const STEP: i64 = 100_000_000;
    const WRAP: i64 = 50_000_000;
    let key0 = (min % WRAP).clamp(-WRAP, WRAP) + STEP * (6 * axis as i64 + i64::from(bucket));
    match i32::try_from(key0) {
        Ok(key0) if key0 >= 0 => Ok(key0 as u32),
        _ => Err((1, 0)),
    }
}

// --- spamap.entries_k --------------------------------------------------------------

/// Whether `mapboxlib/MPBOX` hands an element of the noun to
/// `mapboxlib/MPBXBR` — BRAN, and the nouns whose `HNGT` is set
/// (`HTCOMP`) — which records each of its members and the tube leaving it,
/// and the element itself by its head tube alone.
pub fn records_its_members(noun: u32) -> bool {
    noun == db1_hash("BRAN") || noun_in(&HNGT_SET, HNGT, noun)
}

/// The entries Core3D gives an element: one, k = 0, with its own box, from
/// `mapboxlib/MPBOX2` — `MBXGO`'s one other caller, `mapboxlib/MPBXBR`,
/// maps only the elements [`records_its_members`] names and their members.
/// There a member's k = 1 entry is the tube leaving it, and the element's
/// own entry its head tube, both boxed from catalogue P-points and the tube
/// spec's bore, which e3d-io does not read: missing input (requirement 6.15).
pub fn entries_by_mpbox(refno: RefNo, state: &dyn SpatialState, aabb: [i64; 6]) -> Result<EntryBoxes, UndecodableAttr> {
    if state.noun(refno).is_some_and(records_its_members) {
        return Ok(EntryBoxes::MissingInput("the head tube, boxed by MPBXBR from catalogue P-points and the tube spec"));
    }
    if state.owner(refno).and_then(|owner| state.noun(owner)).is_some_and(records_its_members) {
        return Ok(EntryBoxes::MissingInput("the tube leaving it, boxed by MPBXBR from catalogue P-points and the tube spec"));
    }
    Ok(EntryBoxes::Entries(vec![(0, aabb)]))
}

// --- spamap.flags ------------------------------------------------------------------

/// The flags of the k = 0 entry `mapboxlib/MPBOX2` gives an element:
/// `maplib/MAPINS` adds 1, 2, 4 for three logicals, which `mapboxlib/MBXGO`
/// clears unless `MPBOX` found flags enabled up the owner chain.
///
/// - With a `WBOX`, `MPBOX2` sets logical OBST for an `OBST` of 1 to 3.
/// - Without, `dbboxlib/BOXELE` sets it for an `OBST` of 1 or 2 before it
///   boxes the element by its box type; for an `IMAP` 5 element `MPBOX2`
///   then runs `BOXELE` on each `IMAP` 3 member with the same logicals.
/// - Unsaid where `BOXELE`'s box type passes the logicals on or sets one
///   (the nouns outside [`FLAGS_BY_OBST_ALONE`]), for a UDET, and in branch
///   and hanger trees, whose flags `MPBXBR` takes from catalogue geometry
///   and the tube spec.
pub fn flags_by_mpbox(refno: RefNo, state: &dyn SpatialState, _k: u32) -> Result<Option<u32>, UndecodableAttr> {
    let Some(noun) = state.noun(refno) else { return Ok(None) };
    let owner_records_members = state.owner(refno).and_then(|owner| state.noun(owner)).is_some_and(records_its_members);
    if records_its_members(noun) || owner_records_members {
        return Ok(None);
    }
    let Some(enabled) = flags_enabled(refno, state)? else { return Ok(None) };
    if !enabled {
        return Ok(Some(0));
    }
    if state.attr(refno, db1_hash("WBOX"))?.is_some() {
        return Ok(Some(obst_bit(refno, state, 3)?));
    }
    if !noun_in(&OBST_ALONE_SET, FLAGS_BY_OBST_ALONE, noun) {
        return Ok(None);
    }
    let mut flags = obst_bit(refno, state, 2)?;
    let Some(imap) = soft_imap(refno, noun, state)? else { return Ok(None) };
    if imap == 5 {
        let Some(members) = state.members(refno) else { return Ok(None) };
        for member in members {
            let Some(member_noun) = state.noun(member) else { continue };
            match soft_imap(member, member_noun, state)? {
                Some(3) if noun_in(&OBST_ALONE_SET, FLAGS_BY_OBST_ALONE, member_noun) => flags |= obst_bit(member, state, 2)?,
                Some(3) | None => return Ok(None),
                Some(_) => {}
            }
        }
    }
    Ok(Some(flags))
}

/// `mapboxlib/MPBOX` up the owner chain from the element itself: enabled at
/// a `DDEP` of 1 but on [`DDEP_DOES_NOT_ENABLE`], or at one of
/// [`ENABLE_FLAGS`]; not at the WORL or where an owner cannot be found.
fn flags_enabled(refno: RefNo, state: &dyn SpatialState) -> Result<Option<bool>, UndecodableAttr> {
    let mut at = refno;
    for _ in 0..OWNER_CHAIN_DEPTH {
        let Some(noun) = state.noun(at) else { return Ok(Some(false)) };
        let ddep = matches!(
            state.attr(at, db1_hash("DDEP"))?,
            Some(DescriptorValue::Int(1)) | Some(DescriptorValue::Word { raw: 1, .. })
        );
        if (ddep && !DDEP_DOES_NOT_ENABLE.iter().any(|name| db1_hash(name) == noun))
            || ENABLE_FLAGS.iter().any(|name| db1_hash(name) == noun)
        {
            return Ok(Some(true));
        }
        if noun == db1_hash("WORL") {
            return Ok(Some(false));
        }
        let Some(owner) = state.owner(at) else { return Ok(Some(false)) };
        at = owner;
    }
    Ok(None)
}

/// The logical `GATIN1` of `OBST` sets: bit `OBST − 1` for an `OBST` of 1 to
/// `highest`.
fn obst_bit(refno: RefNo, state: &dyn SpatialState, highest: i64) -> Result<u32, UndecodableAttr> {
    let obst = match state.attr(refno, db1_hash("OBST"))? {
        Some(DescriptorValue::Int(value)) => i64::from(value),
        Some(DescriptorValue::Word { raw, .. }) => i64::from(raw as i32),
        _ => 0,
    };
    Ok(if (1..=highest).contains(&obst) { 1 << (obst - 1) } else { 0 })
}

static OBST_ALONE_SET: OnceLock<Vec<u32>> = OnceLock::new();

/// Owner levels `flags_enabled` climbs before it gives up, against a cycle.
const OWNER_CHAIN_DEPTH: usize = 64;

/// The nouns at which `MPBOX` enables flags whatever their `DDEP`.
pub const ENABLE_FLAGS: &[&str] = &["SITE", "BLOCK", "SSOWLD", "HBLWLD"];

/// The nouns whose `DDEP` of 1 does not enable flags.
pub const DDEP_DOES_NOT_ENABLE: &[&str] = &["TPWL", "APPLDW", "TRNWLD", "FEMWLD"];

/// The nouns of the box types `dbboxlib/BOXELE` computes without touching
/// the logicals — `DB_Model::getBoxType` 2 to 17 and 20 to 22; types 1, 18,
/// 19 and 23 hand them on to catalogue geometry or set one.
pub const FLAGS_BY_OBST_ALONE: &[&str] = &[
    "CYLI", "NCYL", "BOX", "PVOL", "RPLA", "XCELL", "NBOX", "DISH", "NDIS", "CONE", "NCON", "SNOU", "NSNO", "PYRA",
    "NPYR", "CTOR", "RTOR", "NCTO", "NRTO", "EXTR", "REVO", "NREV", "NXTR", "LOOP", "PANE", "PLOO", "PALJ", "SLCY",
    "NSLC", "POHE", "GRDM", "POGO", "DRAW", "BOUN", "POLYHE", "NPOLYH", "POIN", "IPOI", "TPOI", "PAVE", "VERT",
    "LOAP", "DATU", "POINTR", "SELJ", "SEVE", "PNOD", "SNOD", "CABLE", "POINSP", "CURVE", "PLDATU",
];

// --- spamap.coverage ---------------------------------------------------------------

/// Whether Core3D gives the element entries: `Some(true)` when
/// `mapboxlib/MPBOX` records it through `MPBOX2`, which writes its entries
/// whenever E3D has a box for it; `Some(false)` when E3D never writes it any;
/// `None` where the evidence does not say.
///
/// - Its noun's `IMAP` (noun field 847458, read by `ATNINT` after `UDSOFT`):
///   1 and 5 record it, 7 records it under an owner whose `IMAP` is 2 and
///   merges it into the owner otherwise, 0, 2, 3 — and a noun the dictionary
///   lacks, which `ATNINT` reads as 0 — never record it (`MPBOX`, and
///   `maplist/MLSADD`, which queues a changed element for it).
/// - BRAN and the nouns whose `HNGT` is set go to `mapboxlib/MPBXBR`
///   instead, which gives them an entry only for a head tube of at least
///   1 mm between catalogue P-points: unsaid. Their members it records
///   whatever their `IMAP`.
/// - An element whose noun is a UDET, or whose `UDTYPE` names one, takes the
///   UDET's `IMAP` from the project's dictionary: unsaid.
/// - `maplib/MAPINS` writes only where `BTSELE(SPAMAP)` passes, and
///   [`NO_SPAMAP_IN_TEMPLATE`] lack SPAMAP in the design template: unsaid.
/// - The nouns `DB_Model::getBoxType` gives box type 1, but for SCTN, GENSEC
///   and WLJOIN, have no box without a catalogue: `dbboxlib/GTSTDT` finds no
///   geometry for an unset `SPRE`, `MBXSTL` fails, and `MBXGO` removes the
///   element's entries. Whether any other box E3D computes exists is the
///   World_AABB's to say.
pub fn coverage_by_imap(refno: RefNo, state: &dyn SpatialState) -> Result<Option<bool>, UndecodableAttr> {
    let Some(noun) = state.noun(refno) else { return Ok(None) };
    if records_its_members(noun) || noun_in(&NO_SPAMAP_SET, NO_SPAMAP_IN_TEMPLATE, noun) {
        return Ok(None);
    }
    if state.owner(refno).and_then(|owner| state.noun(owner)).is_some_and(records_its_members) {
        return Ok(Some(true));
    }
    let Some(imap) = soft_imap(refno, noun, state)? else { return Ok(None) };
    let recorded = match imap {
        1 | 5 => true,
        7 => {
            let Some(owner) = state.owner(refno) else { return Ok(None) };
            let Some(owner_noun) = state.noun(owner) else { return Ok(None) };
            match soft_imap(owner, owner_noun, state)? {
                Some(owner_imap) => owner_imap == 2,
                None => return Ok(None),
            }
        }
        _ => false,
    };
    if recorded && noun_in(&CATALOGUE_BOXED_SET, CATALOGUE_BOXED, noun) && !has_catalogue(refno, state)? {
        return Ok(Some(false));
    }
    Ok(Some(recorded))
}

/// The `IMAP` `UDSOFT` and `ATNINT` read for the element, or `None` when its
/// soft type is a UDET.
fn soft_imap(refno: RefNo, noun: u32, state: &dyn SpatialState) -> Result<Option<u8>, UndecodableAttr> {
    if noun > LAST_SYSTEM_NOUN {
        return Ok(None);
    }
    let udet = match state.attr(refno, db1_hash("UDTYPE"))? {
        Some(DescriptorValue::Word { raw, .. }) => raw != 0,
        Some(DescriptorValue::Int(value)) => value != 0,
        _ => false,
    };
    Ok((!udet).then(|| imap_of(noun)))
}

/// The dictionary `IMAP` of a system noun: 0 for one that is in no list.
pub fn imap_of(noun: u32) -> u8 {
    static IMAP: OnceLock<HashMap<u32, u8>> = OnceLock::new();
    let table = IMAP.get_or_init(|| {
        [(1, IMAP_1), (2, IMAP_2), (3, IMAP_3), (5, IMAP_5), (7, IMAP_7)]
            .into_iter()
            .flat_map(|(imap, nouns)| nouns.iter().map(move |noun| (db1_hash(noun), imap)))
            .collect()
    });
    table.get(&noun).copied().unwrap_or(0)
}

fn has_catalogue(refno: RefNo, state: &dyn SpatialState) -> Result<bool, UndecodableAttr> {
    Ok(matches!(state.attr(refno, db1_hash("SPRE"))?, Some(DescriptorValue::RefNo(spre)) if spre.word0 != 0 || spre.word1 != 0))
}

fn noun_in(set: &'static OnceLock<Vec<u32>>, names: &[&str], noun: u32) -> bool {
    set.get_or_init(|| names.iter().map(|name| db1_hash(name)).collect()).contains(&noun)
}

static HNGT_SET: OnceLock<Vec<u32>> = OnceLock::new();
static NO_SPAMAP_SET: OnceLock<Vec<u32>> = OnceLock::new();
static CATALOGUE_BOXED_SET: OnceLock<Vec<u32>> = OnceLock::new();

/// The last hash of the base-27 system names; a UDET's is past it.
const LAST_SYSTEM_NOUN: u32 = 387_951_929;

/// The nouns whose `HNGT` is set: `HTCOMP` is true, and `MPBOX` hands them to
/// `MPBXBR` as it does BRAN.
pub const HNGT: &[&str] = &["HANG", "LUG", "SUPC", "TRUNNI"];

/// The recorded nouns whose design template (`desvir.dat`) has no SPAMAP.
pub const NO_SPAMAP_IN_TEMPLATE: &[&str] = &["DTPNCH", "DTSTEL", "DTSUBM", "FSPINS", "ICRPNT", "LADDER", "RSFINS", "TUBI"];

/// Box type 1 in `DB_Model::getBoxType` but for SCTN, GENSEC and WLJOIN.
pub const CATALOGUE_BOXED: &[&str] =
    &["CMFI", "CMPF", "COFI", "FITT", "FIXING", "PCOJ", "PFIT", "PJOI", "SBFI", "SCOJ", "SJOI", "SUBJ"];

/// `IMAP` 1 in the E3D 3.1 dictionary (`tests/spamap_coverage_real.rs`
/// rereads every list from `schema/e3d31/binaries/attlib.dat`).
pub const IMAP_1: &[&str] = &[
    "AHU", "AIDARC", "AIDCIR", "AIDLIN", "ANCI", "ATTA", "BATT", "BBLT", "BEND", "BNDLIN", "BOUN", "BRCO", "BWLD",
    "CABLE", "CAP", "CBSEG", "CCURVE", "CLEV", "CLOS", "CMFI", "CMPF", "CNODE", "CNVCOM", "COUP", "COWL", "CPLANE",
    "CPLATE", "CPOINT", "CPROTR", "CROS", "CSEAM", "CSTIFF", "CSURF", "CTJOIN", "CTRAY", "CTSUPP", "DAMP", "DATU",
    "DOOR", "DRAW", "DTPNCH", "DTSTEL", "DTSUBM", "DUCT", "ELBO", "ELCONN", "ELFITT", "EQUCOM", "EYNT", "EYRD",
    "FBLI", "FEBEAM", "FEFRED", "FEMIMG", "FESHEL", "FETRUS", "FILT", "FIXTUR", "FLAN", "FLEX", "FLOOR", "FLRCOV",
    "FNCARE", "FNCBRN", "FPFITT", "FSPINS", "FTUB", "FURNIT", "GASK", "GENNC", "GENNG", "GENPRI", "GRDM", "GRIDCY",
    "GRIDPL", "GRIL", "GWALL", "HACC", "HANCI", "HBEAD", "HBRCKT", "HBRFLA", "HBRSTI", "HCLIP", "HCOMPT", "HCURVE",
    "HDOPLA", "HELE", "HFAN", "HFLANG", "HIBEA", "HIBRA", "HICPLA", "HICSTI", "HICUR", "HIDOU", "HIFLA", "HIPIL",
    "HIPLA", "HISTI", "HMARK", "HNUT", "HPILLR", "HPIN", "HPLATE", "HRGATE", "HROD", "HRPANE", "HRPOST", "HRSOF",
    "HRTERM", "HSAD", "HSTIFF", "HULCON", "HVACFI", "HVBRCO", "HVFLAN", "HVHACC", "HVIDAM", "HVSADD", "HVSKIR",
    "HVSPLR", "HVSTIF", "HVTPPO", "IBNDRY", "ICLIP", "ICRCUR", "ICRPNT", "ICTOUT", "IDAM", "IEDGE", "IENDFR",
    "IENDTO", "IFCPLA", "IHOLE", "ILIM", "INFITT", "INODE", "INOTCH", "INST", "IPANE", "IPILLR", "IPLATE", "ISEAM",
    "ISECT", "ISTIFF", "JNODE", "KICKPL", "LDRRUN", "LJSE", "LOAP", "LUANCI", "MESH", "MNOZ", "MPLATE", "MPROF",
    "NOZZ", "OFST", "OLET", "PANE", "PCLA", "PCLI", "PCOJ", "PCOM", "PEXSP", "PGBOX", "PIACT", "PIBLK", "PIGEN",
    "PLAT", "PLEN", "PLUG", "PNOD", "POHE", "POST", "PTAP", "PULLN", "PVOL", "PWCHA", "PWMAN", "RAIL", "RCPL",
    "REDU", "RNODE", "ROD", "RPLA", "RSEG", "RSFINS", "SBFI", "SCLA", "SCOJ", "SCREED", "SHOE", "SHU", "SILE",
    "SKIR", "SLUG", "SNUB", "SOST", "SPAC", "SPLR", "SPMSPC", "SRFCUR", "SRFSUR", "STIF", "STLS", "STRT", "SUBCOM",
    "SWBR", "TAPE", "TCOM", "TEE", "THRE", "TP", "TRANCI", "TRAP", "TREAD", "TRNB", "TRNCUR", "TRNN", "TRNS",
    "TRREDU", "TUBI", "UBOL", "UNIO", "VALV", "VENT", "VFWA", "VSPR", "VTWA", "WASH", "WELD", "WINDOW", "WKSUR",
    "WLJOIN", "WLPANE", "WLPROF", "WPAD", "XGEOM",
];

/// `IMAP` 2: containers, recorded only through their members.
pub const IMAP_2: &[&str] = &[
    "AIDGRO", "APPDAR", "APPLDA", "APPLDW", "AREADE", "AREASE", "AREAWL", "ASWL", "BBLOCK", "BLEVEL", "BLOCK",
    "BOTEXT", "BPFEAT", "BRAN", "BUNDLE", "CAGSEG", "CEILIN", "CFLOOR", "CGRDCP", "CGRDLP", "CLNCGR", "CLNLAT",
    "CLNPNG", "COMWLD", "CONGRP", "CONVEY", "CONWLD", "CPANEL", "CPLANS", "CPOINS", "CPROF", "CRMCUR", "CRMPNT",
    "CSCREE", "CSURPX", "CTBEND", "CTCOUP", "CTCROS", "CTFEAT", "CTMTRL", "CTREDU", "CTRISE", "CTSTRA", "CTTEE",
    "CURWLD", "CWALL", "CWAREA", "CWAY", "DEMOPA", "DEMOSE", "DEMOWL", "DETGRP", "DIMGRO", "DTGRID", "DTJOB",
    "DTMAIN", "DTPHSE", "DTSTRU", "DTWRLD", "ENVLOP", "EQUI", "EXTFRM", "FAWLD", "FEMODL", "FEMWLD", "FEMZON",
    "FMGRP", "FMWL", "FNCAAR", "FNCAW", "FNCCAR", "FNCCW", "FRMW", "GENPI", "GENPKG", "GPKGFL", "GPSET", "GPWL",
    "GRIDEL", "GRIDFA", "GRIDWL", "GROU", "HANG", "HBLWLD", "HCMWLD", "HIBLO", "HICPAN", "HIPAN", "HMKWLD",
    "HOOPSE", "HPANEL", "HRFEAT", "HRKPSE", "HRPNSE", "HRSO", "HSLIST", "HSPOOL", "HSUBPN", "HVAC", "IBRCKT",
    "IDLWLD", "INTFRM", "INTLNG", "ISTRU", "ISTRUR", "JNTBOX", "KSUEDS", "KSUFAS", "KSUNDS", "LDREXT", "LDRSTR",
    "LUG", "MANPKG", "MPKGFL", "OPENSP", "ORDWLD", "PIPE", "PLTGRD", "PNTWLD", "POSTSE", "PPIECE", "PPLIST",
    "PSLIST", "PSPOOL", "PTRA", "PULLP", "RAILSE", "REFDAT", "REFGRD", "REST", "RLCAGE", "RLGATE", "ROOM", "RPLG",
    "RSOWLD", "RSTARE", "RTFEAT", "RUNGSE", "RUTVOL", "SBFR", "SEGSEQ", "SITE", "SLRAIL", "SPMSA", "SPMSW",
    "SSOWLD", "STRSET", "STRSTR", "STRTWR", "STRU", "STRWEL", "SUBCNV", "SUBE", "SUBS", "SUPC", "SUPPO", "SVOLM",
    "TMAR", "TMPL", "TOPEXT", "TPWL", "TREADS", "TRNCC", "TRNCEL", "TRNCUC", "TRNCUS", "TRNDEF", "TRNMOD", "TRNPRI",
    "TRNSLD", "TRNWLD", "TRUNNI", "TWRSTL", "UPANEL", "VOLM", "WKPACK", "WKPKWL", "WKSWLD", "WLCOMP", "WLFEAT",
    "WLFITT", "WLJNTS", "WLOPEN", "WLPRSE", "WORL", "XCELS", "XCLTN", "ZONE",
];

/// `IMAP` 3: merged into the owner's box (`MPBOX2` of an `IMAP` 5 owner).
pub const IMAP_3: &[&str] = &[
    "COFI", "COMFIX", "CURVE", "DTGEOM", "ENDATU", "FCUTPL", "FLRLAY", "HCTOUT", "HNODE", "HTFEAT", "IPOI",
    "JLDATU", "KSUSVE", "LOOP", "NBOX", "NCON", "NCTO", "NCYL", "NDIS", "NPOLYH", "NPYR", "NREV", "NRTO", "NSLC",
    "NSNO", "NXTR", "PAVE", "PLDATU", "PLOO", "POGO", "POIN", "POINSP", "POINTR", "POLPTL", "RPATH", "SNOD",
    "SPINE", "TANP", "VERT",
];

/// `IMAP` 5: recorded, with the boxes of its `IMAP` 3 members merged in.
pub const IMAP_5: &[&str] = &[
    "ABOX", "ACONE", "ACTOR", "ACYLI", "ADISH", "AEXTR", "APOLYH", "APYRA", "AREVO", "ARTOR", "ASLCY", "ASNOU",
    "BOX", "BPANEL", "BPFITT", "CLNTIL", "CONE", "CTOR", "CTWALL", "CWBRAN", "CYLI", "DISH", "DTBOLT", "DTHOLE",
    "DTPLAT", "DTPROF", "DTWELD", "EXTR", "FITT", "FIXING", "GENCUR", "GENPC", "GENPG", "GENSEC", "HANDRA",
    "INSURQ", "LADDER", "LDRCAG", "MFIX", "PFIT", "PJOI", "PLTFRM", "POLYHE", "PYRA", "RBRAN", "REVO", "RLADDR",
    "RTOR", "SCTN", "SDAFIX", "SJOI", "SLADDR", "SLCY", "SNOU", "STRFLT", "STRLNG", "STWALL", "SUBJ", "WALL",
    "XCELL",
];

/// `IMAP` 7: recorded or merged as the owner's `IMAP` says.
pub const IMAP_7: &[&str] = &["PALJ"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_go_to_the_last_axis_and_20_m_goes_to_x() {
        assert_eq!(least_axis_last_of_ties([5, 3, 9]), Axis::Y);
        assert_eq!(least_axis_last_of_ties([3, 3, 9]), Axis::Y);
        assert_eq!(least_axis_last_of_ties([3, 9, 3]), Axis::Z);
        assert_eq!(least_axis_last_of_ties([3, 3, 3]), Axis::Z);
        assert_eq!(least_axis_last_of_ties([199_999, 300_000, 400_000]), Axis::X);
        assert_eq!(least_axis_last_of_ties([300_000, 250_000, 200_000]), Axis::X);
    }

    #[test]
    fn buckets_end_where_mapkey_ends_them() {
        let buckets: Vec<Option<u8>> = [-1, 0, 599, 600, 1_999, 2_000, 5_999, 6_000, 19_999, 20_000, 199_999, 200_000]
            .into_iter()
            .map(bucket_by_mapkey)
            .collect();
        assert_eq!(
            buckets,
            [None, Some(1), Some(1), Some(2), Some(2), Some(3), Some(3), Some(4), Some(4), Some(5), Some(5), Some(6)]
        );
    }

    /// Requirement 4.20's entry, the native BOX's negative min, the 2026-09-22
    /// evidence's `24384/2869` (N 16), and a min past 5 km.
    #[test]
    fn key0_is_mapky2s() {
        assert_eq!(key0_by_mapky2(Axis::X, 3, 8_495), Ok(300_008_495));
        assert_eq!(key0_by_mapky2(Axis::X, 3, -30_014), Ok(299_969_986));
        assert_eq!(key0_by_mapky2(Axis::Z, 4, 20_129), Ok(1_600_020_129));
        assert_eq!(key0_by_mapky2(Axis::Z, 5, 50_000_123), Ok(1_700_000_123));
        assert_eq!(key0_by_mapky2(Axis::Z, 5, -50_000_123), Ok(1_699_999_877));
        assert_eq!(key0_by_mapky2(Axis::X, 6, -49_999_999), Ok(550_000_001));
    }

    #[test]
    fn halves_round_away_from_zero() {
        assert_eq!(round_half_away_from_zero(849.5, 1150.5), (8_495, 11_505));
        assert_eq!(round_half_away_from_zero(-0.05, 0.05), (-1, 1));
        assert_eq!(round_half_away_from_zero(-0.04, 0.04), (0, 0));
    }

    #[test]
    fn words_hold_what_a_datum_of_0_holds() {
        assert_eq!((SIGNED_WORDS.encode)(-30_014), Some(4_294_937_282));
        assert_eq!((SIGNED_WORDS.decode)(4_294_937_282), -30_014);
        assert_eq!((SIGNED_WORDS.encode)(WITHIN_DATUM), Some(999_999_999));
        assert_eq!((SIGNED_WORDS.encode)(WITHIN_DATUM + 1), None);
        assert_eq!((SIGNED_WORDS.encode)(-WITHIN_DATUM - 1), None);
    }

    struct Element {
        noun: u32,
        owner: Option<RefNo>,
        attrs: HashMap<u32, DescriptorValue>,
    }

    /// Elements in memory.
    #[derive(Default)]
    struct Tree {
        elements: HashMap<RefNo, Element>,
    }

    impl Tree {
        fn with(mut self, refno: RefNo, noun: u32, owner: Option<RefNo>, attrs: &[(&str, DescriptorValue)]) -> Self {
            let attrs = attrs.iter().map(|(name, value)| (db1_hash(name), value.clone())).collect();
            self.elements.insert(refno, Element { noun, owner, attrs });
            self
        }
    }

    impl SpatialState for Tree {
        fn noun(&self, refno: RefNo) -> Option<u32> {
            self.elements.get(&refno).map(|element| element.noun)
        }
        fn owner(&self, refno: RefNo) -> Option<RefNo> {
            self.elements.get(&refno).and_then(|element| element.owner)
        }
        fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
            Ok(self.elements.get(&refno).and_then(|element| element.attrs.get(&hash).cloned()))
        }
        fn members(&self, refno: RefNo) -> Option<Vec<RefNo>> {
            let mut members: Vec<RefNo> =
                self.elements.iter().filter(|(_, element)| element.owner == Some(refno)).map(|(member, _)| *member).collect();
            members.sort_by_key(|member| (member.word0, member.word1));
            Some(members)
        }
    }

    const OWNER: RefNo = RefNo { word0: 24_384, word1: 1 };
    const ELEMENT: RefNo = RefNo { word0: 24_384, word1: 2 };
    const A_UDET: u32 = LAST_SYSTEM_NOUN + 1;

    /// What the rule makes of an element of `noun` under an owner of `owner`.
    fn covered_under(owner: u32, noun: u32, attrs: &[(&str, DescriptorValue)]) -> Option<bool> {
        let tree = Tree::default().with(OWNER, owner, None, &[]).with(ELEMENT, noun, Some(OWNER), attrs);
        coverage_by_imap(ELEMENT, &tree).expect("every attribute decodes")
    }

    fn covered(noun: &str, attrs: &[(&str, DescriptorValue)]) -> Option<bool> {
        covered_under(db1_hash("EQUI"), db1_hash(noun), attrs)
    }

    #[test]
    fn the_nouns_imap_decides_and_imap_7_asks_the_owner() {
        assert_eq!(covered("BOX", &[]), Some(true));
        assert_eq!(covered("FTUB", &[]), Some(true));
        assert_eq!(covered("EQUI", &[]), Some(false));
        assert_eq!(covered("NBOX", &[]), Some(false));
        assert_eq!(covered("TEXT", &[]), Some(false));
        assert_eq!(covered("DBMETA", &[]), Some(false), "a noun the dictionary lacks reads as IMAP 0");
        assert_eq!(covered("PALJ", &[]), Some(true), "under an IMAP 2 owner");
        assert_eq!(covered_under(db1_hash("BOX"), db1_hash("PALJ"), &[]), Some(false));
        assert_eq!(coverage_by_imap(OWNER, &Tree::default()).unwrap(), None, "no element");
    }

    #[test]
    fn mpbxbr_udets_and_nouns_without_spamap_are_unsaid() {
        assert_eq!(covered("BRAN", &[]), None);
        assert_eq!(covered("HANG", &[]), None);
        assert_eq!(covered("TUBI", &[]), None);
        let udtype = DescriptorValue::Word { raw: A_UDET, text: None };
        assert_eq!(covered("BOX", &[("UDTYPE", udtype)]), None);
        assert_eq!(covered("BOX", &[("UDTYPE", DescriptorValue::Int(0))]), Some(true));
        assert_eq!(covered_under(db1_hash("EQUI"), A_UDET, &[]), None);
        assert_eq!(covered_under(A_UDET, db1_hash("PALJ"), &[]), None);
    }

    #[test]
    fn box_type_1_nouns_but_the_profiles_need_a_catalogue() {
        let spre = |word0, word1| ("SPRE", DescriptorValue::RefNo(RefNo { word0, word1 }));
        assert_eq!(covered("PJOI", &[]), Some(false));
        assert_eq!(covered("PJOI", &[spre(0, 0)]), Some(false));
        assert_eq!(covered("PJOI", &[spre(24_384, 100)]), Some(true));
        assert_eq!(covered("SCOJ", &[spre(24_384, 100)]), Some(true));
        assert_eq!(covered("SCTN", &[]), Some(true), "MBXSTL boxes it by its ends");
        assert_eq!(covered("COFI", &[spre(24_384, 100)]), Some(false), "IMAP 3");
    }

    #[test]
    fn mpbxbr_records_the_members_of_bran_and_hngt_nouns_whatever_their_imap() {
        assert_eq!(covered_under(db1_hash("BRAN"), db1_hash("ELBO"), &[]), Some(true));
        assert_eq!(covered_under(db1_hash("HANG"), db1_hash("TEXT"), &[]), Some(true));
        assert_eq!(covered_under(db1_hash("BRAN"), db1_hash("TUBI"), &[]), None, "no SPAMAP in its template");
    }

    #[test]
    fn one_k_0_entry_but_in_branch_and_hanger_trees() {
        let aabb = [1, 2, 3, 4, 5, 6];
        let entries = |owner: &str, noun: &str| {
            let tree = Tree::default()
                .with(OWNER, db1_hash(owner), None, &[])
                .with(ELEMENT, db1_hash(noun), Some(OWNER), &[]);
            entries_by_mpbox(ELEMENT, &tree, aabb).expect("reads no attribute")
        };
        assert_eq!(entries("EQUI", "BOX"), EntryBoxes::Entries(vec![(0, aabb)]));
        assert_eq!(entries("SBFR", "SCTN"), EntryBoxes::Entries(vec![(0, aabb)]));
        assert!(matches!(entries("BRAN", "ELBO"), EntryBoxes::MissingInput(_)));
        assert!(matches!(entries("HANG", "PCLA"), EntryBoxes::MissingInput(_)));
        assert!(matches!(entries("PIPE", "BRAN"), EntryBoxes::MissingInput(_)));
    }

    #[test]
    fn the_flag_nouns_hash_to_what_mpbox_compares() {
        let sorted = |names: &[&str]| {
            let mut hashes: Vec<u32> = names.iter().map(|name| db1_hash(name)).collect();
            hashes.sort_unstable();
            hashes
        };
        assert_eq!(sorted(ENABLE_FLAGS), [644_698, 6_447_602, 64_765_880, 64_768_537]);
        assert_eq!(sorted(DDEP_DOES_NOT_ENABLE), [784_856, 64_766_688, 64_767_782, 332_930_359]);
    }

    #[test]
    fn flags_are_the_obst_bits_once_an_owner_enables_them() {
        const SITE: RefNo = RefNo { word0: 24_384, word1: 10 };
        const MEMBER: RefNo = RefNo { word0: 24_384, word1: 3 };
        let obst = |value: i32| ("OBST", DescriptorValue::Int(value));
        let under = |site: bool, noun: &str, attrs: &[(&str, DescriptorValue)]| {
            let zone_owner = site.then_some(SITE);
            Tree::default()
                .with(SITE, db1_hash("SITE"), None, &[])
                .with(OWNER, db1_hash("ZONE"), zone_owner, &[])
                .with(ELEMENT, db1_hash(noun), Some(OWNER), attrs)
        };
        let flags = |tree: &Tree, k| flags_by_mpbox(ELEMENT, tree, k).expect("every attribute decodes");
        assert_eq!(flags(&under(true, "BOX", &[obst(2)]), 0), Some(2));
        assert_eq!(flags(&under(false, "BOX", &[obst(2)]), 0), Some(0), "no SITE above it");
        assert_eq!(flags(&under(true, "BOX", &[obst(3)]), 0), Some(0), "BOXELE sets OBST 1 and 2 alone");
        let wbox = ("WBOX", DescriptorValue::RealArray(vec![0.0; 6]));
        assert_eq!(flags(&under(true, "BOX", &[obst(3), wbox]), 0), Some(4));
        let with_member = under(true, "BOX", &[obst(2)]).with(MEMBER, db1_hash("NBOX"), Some(ELEMENT), &[obst(1)]);
        assert_eq!(flags(&with_member, 0), Some(3), "an IMAP 3 member's OBST joins in");
        assert_eq!(flags(&under(true, "PJOI", &[obst(2)]), 0), None, "box type 1 hands the logicals on");
        assert_eq!(flags(&under(true, "BOX", &[obst(2)]), 1), Some(2), "MPBOX2 writes k = 0 alone; k does not enter");
        let in_a_branch = Tree::default()
            .with(OWNER, db1_hash("BRAN"), None, &[])
            .with(ELEMENT, db1_hash("ELBO"), Some(OWNER), &[]);
        assert_eq!(flags(&in_a_branch, 0), None);
    }
}
