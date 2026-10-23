//! The PML expressions catalogue geometry stores instead of a value.
//!
//! `SEXT`'s `Px` reads as the all-zero tuple [`crate::record::catalogue_expr`]
//! declines, yet E3D prints `( 0.5 * ATTRIB PARA[20 ] )`. The formula is in the
//! record: it sits in the explicit stream under the two's-complement negation
//! of the attribute's own hash, so `Px` (0x00081E89) carries its expression as
//! 0xFFF7E177. Sixteen sampled elements confirm the pairing, and the same hash
//! names the same attribute across nouns — 0xFFF7E141 is `Pz` on both `SEXT`
//! and `PTMI`. Being above the UDA floor, those entries were being read as
//! user-defined attributes no dictionary could explain.
//!
//! The body is a length word, a leading 1, and then reverse Polish:
//!
//! ```text
//! 33 1 101 6 20480 0 3 0 6            push 5
//!      106 2 564289 -1 -1 0 1601 1701 ATTRIB PARA[5 ]
//!      101 6 32768 0 2 0 6            push 4
//!      106 2 564289 -1 -1 0 1601 1701 ATTRIB PARA[4 ]
//!      802                            add
//! ```
//!
//! which E3D prints as `( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )`. Names are
//! ordinary `db1_hash` words: 564289 is `PARA`, 840258 is `RPRO`.
//!
//! A user-defined attribute is named by its Dictionary key instead. The CADC
//! HVAC catalogue's `/HRTUBE1-BOXI.PXLE` — E3D prints `( :HXYsize[2 ] )` —
//! is `106 2 401558369 -1 -1 0 1601 1701` behind `push 2`: 401558369 is
//! `0x17EF4B61`, above every base-27 name, and no `db1_dehash` reaches it. The
//! word is the `UKEY` the name had when the catalogue was written, and only a
//! Dictionary database can say which UDA that is. So the parser keeps the key
//! ([`Attribute::uda_key`]), [`evaluate_words`] asks the environment for the
//! value **by key** ([`ParamEnv::uda_number`]), and [`render_with_dictionary`]
//! prints the name when a Dictionary knows the key and `:0x17EF4B61` when
//! none does.
//!
//! By key, not by name, because the two are not the same question. On
//! `AvevaMarineSample /ALL` the name `HXYsize` is declared twice: `ams5100`
//! (the sample project's own Dictionary) allocates it `0x2652AB52`, and the
//! design BRANs store their `675 600` under that key; `acp7006` (the
//! `AvevaCatalogue` Dictionary the CADC catalogue was written against) has it
//! as `0x2C00D576` with `OLDKEY 0x17EF4B61` — the word in the program. A
//! by-key read of the BRAN finds the acp definition and no value under either
//! of its keys: unset, formula unresolved. That is exactly what E3D does with
//! the same program (`Catalogue error … unable to evaluate … PXLE` on every
//! head slot that reaches it, while `Q :HXYsize` on the same BRAN prints
//! `675 600`); a by-name read would invent the value E3D refuses.
//!
//! Anything this does not fully understand returns `None`. A half-read formula
//! would be worse than the all-zero tuple it replaces, because it would look
//! like an answer.
//!
//! [`render`] prints the program and [`evaluate_words`] works it out, and both
//! read the words through the one [`parse`]. Two walkers over the same wire
//! format drift: the renderer would keep accepting a shape the evaluator had
//! quietly stopped reading, and the only symptom would be a number going
//! missing on a formula that still prints.

use std::fmt::Write as _;

use e3d_attlib::system_names::SYSTEM_HASH_MAX;
use e3d_attlib::{db1_dehash, lookup_system_name};

use crate::record::catalogue_eval::{AttribRef, ParamEnv};
use crate::uda_catalog::UdaCatalog;

/// Word 0 of the program, before any opcode. Every sampled blob carries it.
const PROGRAM_MARKER: i32 = 1;
const OP_PUSH_NUMBER: i32 = 101;
const OP_ATTRIBUTE: i32 = 106;
const OP_PUSH_TEXT: i32 = 118;
/// The constant `PI`, an operand that takes no words of its own (0x6F; the
/// earlier `parse-pdms-db` reader spelt the same code `PI`).
///
/// The pipe-support saddles `/NPHS-SU-GMSE-A/B/C` (ams7999 `23984/99018` …)
/// write every arc point of their `SRTO` rings through it: `P6.PX` is
/// `( ( ATTRIB PARA[2 ] + ATTRIB PARA[4 ] ) / 2 * SIN ( 180 * ATTRIB PARA[6 ] /
/// ( PI * ( ATTRIB PARA[2 ] + ATTRIB PARA[4 ] ) ) ) )` — the arc length `I`
/// turned into half the sweep angle in degrees — and the `PTCD` of the same
/// point turns `X` toward `-Z` by that angle. Unread, the whole program was
/// declined: `PX` / `PZ` fell to 0 (`pplib/YPARUN`), the direction to `+Y`
/// (`pplib/DPARAM`), both axes of every saddle ring landed on the origin along
/// `+Y`, and `torus_frame` wrote the ring off as degenerate. ZONE 66457 lost
/// all 131 saddles (171 `RectangularTorus` records) that way.
const OP_PI: i32 = 111;
/// Numeric equality. The `case 401` arm of Core's expression interpreter
/// (`core.dll` `sub_51B4976`, reached from `exprlib/EXEVRE` at `0x51B485A`)
/// does not compare bits — it asks `exprlib/EXRCTL` (`0x51D3B32`) for a slack
/// first, see [`equal_within_slack`]. 402 is the text form, 403 the logical one
/// and 404 the two-component one; none is in the corpus, so none is read here.
const OP_EQUAL: i32 = 401;
/// The real comparisons of the same interpreter (`case 601` … `case 607`), each
/// against the [`comparison_slack`] `EQ` uses: `GT` is `a - b > slack`, `LT`
/// `b - a > slack`, `GE` `a - b >= -slack`, `LE` `b - a >= -slack`. 602 / 604 /
/// 606 are the position forms and are not read.
const OP_GREATER_THAN: i32 = 601;
const OP_LESS_THAN: i32 = 603;
const OP_GREATER_EQUAL: i32 = 605;
const OP_LESS_EQUAL: i32 = 607;
const OP_NEGATE: i32 = 801;
const OP_ADD: i32 = 802;
const OP_SUBTRACT: i32 = 803;
const OP_MULTIPLY: i32 = 804;
const OP_DIVIDE: i32 = 805;
const OP_SIN: i32 = 901;
const OP_COS: i32 = 902;
const OP_TAN: i32 = 903;
/// Inverse sine. PCLA point axes use it to derive both Pz and Ptcdirection.
const OP_ASIN: i32 = 904;
/// Unary arctangent, used by SQRD PTCA.PTCD (P7 through P10).
const OP_ATAN: i32 = 906;
/// Two-argument arctangent, `ATANT ( ordinate , abscissa )` in degrees — see
/// [`binary_number`]. `PTCA.Ptcdirection` turns through one.
const OP_ATANT: i32 = 907;
/// E3D SQRT, captured in LSNO /SQRD-BODY1 POFF (=15194/4150).
const OP_SQRT: i32 = 1001;
/// Native /CFLEX-BODY and /CFLEX-AEXT3 programs, verified by E3D TTY.
const OP_POW: i32 = 1002;
/// Absolute value. AMS cable-tray rung programs use it while distributing
/// rungs over a variable straight length.
const OP_ABS: i32 = 1005;
/// The other `ABS` Core writes. CADC HVAC `/RBREECH-NGAP` and `NLPYRAMID 2 of
/// /RBREECH-BODY1` store `PCTP = ( ATTRIB DESP[42 ] + ABS ( ATTRIB DESP[11 ] ) )`
/// as `… 106 [DESP 11] 1007 802`, and E3D's own `Q PCTP` prints that 1007 as
/// `ABS` (2026-09-19, ams7997 `THREEWAY 1 of BRANCH /-CUP-S-3-M-1405`, seg15 §26.12).
/// 1005 was only ever seen with integer literals underneath; 1007 sits on a real
/// attribute — the two typed arms of the same function. Unread, the whole program
/// is unreadable and the two V-notch cutters of the breeches piece were dropped
/// ("skipped by Core-style local failure"), leaving the notch unfilled or, with
/// the filler `GAP` inverted, cut to the filler's outline instead of the cutter's.
const OP_ABS_REAL: i32 = 1007;
/// Maximum. The same rung programs clamp repeated spacing calculations with
/// it; Core's function-name table places MAX immediately before MIN.
const OP_MAX: i32 = 1008;
const OP_MIN: i32 = 1009;
const OP_MATCH: i32 = 1308;
const OP_TRIM: i32 = 1314;
const OP_LOGICAL_STRING: i32 = 1410;
/// Real `UNSET ( x )`. `sub_51B4976` case 1802 (`0x70A`) pops one real
/// (`exprlib/EXPORE`) and pushes a logical (`EXPULE`): true when that pop's
/// error is 223, and the error is cleared; a real that arrived clean pushes
/// false. Any other error stays an error. The printer `sub_51C0DC1` at
/// `0x51CD573` spells the family `UNSET ( `; 1803 is the text arm (the printer
/// masks the low bit) and is not in the corpus.
///
/// acp7002 `15194/317` PTAX `ALLANG` guards `360 / DESP[78]` with it: a missing
/// parameter is 223, not a zero, so the division is not what runs.
const OP_UNSET_REAL: i32 = 1802;
/// Logical `OR`. Case 301 (`0x12D`) pops two logicals (`EXPOLE` twice) and
/// pushes true when either side is true with a clear error; otherwise
/// `exprlib/EXDEF` merges the errors. The printer at `0x51C19E4` spells ` OR `.
const OP_OR: i32 = 301;
/// `IFTRUE ( cond , a , b )` over reals. The `case 1822` arm of the same
/// interpreter pops `b`, then `a`, then a **logical**, and pushes `a` when
/// the logical is true and `b` when it is false. The sibling opcodes select the
/// same way over other types (1823 text, 1824 logical, 1825+ the rest); only
/// the real one occurs in the corpus.
///
/// Without it the whole program is unreadable, not just this step: the SolidSupport
/// U-bolt `/FHG_A_A1(S)_100` (SCOM `23055/25782`) writes every nut offset, the
/// U-bend diameter and the rubber-sleeve thickness as `IFTRUE`-guarded options,
/// so 5 of its 9 primitives went missing while the other 4 built normally.
const OP_IF_REAL: i32 = 1822;

/// `101` is followed by this word, then four words of number, then it again.
const NUMBER_SHAPE: i32 = 6;
/// Most captured numbers repeat [`NUMBER_SHAPE`] after the payload, while
/// dimensionless stack operands used as array subscripts end in zero.  Core's
/// PTCD programs use both forms (`... 0 6` for angles, `... 0 0` for DESP[n]).
const NUMBER_ENDS: [i32; 2] = [0, NUMBER_SHAPE];
const NUMBER_WORDS: usize = 7;
/// A number stored with a unit: `101 9 <four words of number> 11 1 1 0`. The
/// word after `101` counts the record from itself — 6 for the plain number, 9
/// for this one — and the three extra words are the unit a literal typed as a
/// distance carries. The value is the same four-word number, in millimetres.
///
/// aps250110's column base plate SFIT `/F1_SQR_PFI_6H_003` sizes its plate as
/// `OPAR[2]` plus one such 20 (`SBOX.PXLE`), and E3D's TTY RVM draws that plate
/// 329.2 wide on a 309.2 column; the eaves haunch JOIN `/J2S4_EAV_PFI_PFI_003`
/// extrudes its rafter cutter `PARA[1]` plus one such 1, the 16 mm the rafters
/// end short of the column in the same RVM. Only the trailer seen there is read.
const NUMBER_WITH_UNIT_SHAPE: i32 = 9;
const NUMBER_WITH_UNIT_WORDS: usize = 10;
const DISTANCE_UNIT: [i32; 3] = [11, 1, 1];
/// A typed number without the unit words: `101 7 <four words of number> 11 1`. The
/// same four-word payload, the same `11 1` the distance form opens its trailer with,
/// and nothing after it — the count word says 7, so the record ends there.
///
/// aps250110's door SFIT `/IS-SRO-NS/900x2045` (15193/25228) writes every whole-number
/// literal of its jamb and head profiles this way: `SVER 15193/25272 PY` is
/// `( ( ATTRIB RPRO WIDT - ATTRIB RPRO DWID - 2 ) * 0.5 + 15 )` with the 2 and the 15 in
/// this shape (the 0.5 stays a plain [`NUMBER_SHAPE`] number), its `PRAD` is one such 5,
/// and the neighbouring vertices put `( ( ATTRIB RPRO THK + 1 ) * -1 )` in `PX`. Unread,
/// every one of those vertices fell back to the all-zero tuple and the 95 × 61 jamb with
/// its 5 mm fillet built as the 35 × 10 door stop alone; E3D's TTY RVM draws
/// `FITTING 1 of STWALL 5 of CWALL /B01_LEV01_INT_WALLS` with the full 61 (900 − 806 − 2
/// halved, plus 15) and the arc from (56, −95) to (61, −90). Only this trailer is read.
const NUMBER_TYPED_SHAPE: i32 = 7;
const NUMBER_TYPED_WORDS: usize = 8;
const TYPED_TRAILER: [i32; 2] = [11, 1];
/// `106` is followed by this word, a name, three words of qualifier and two
/// closing words.
const ATTRIBUTE_SHAPE: i32 = 2;
const ATTRIBUTE_WORDS: usize = 8;
const ATTRIBUTE_END: [i32; 2] = [1601, 1701];
/// The shape both sampled owner-chain attributes carry, where the subscripted
/// `PARA` form carries [`ATTRIBUTE_SHAPE`].
const ATTRIBUTE_SHAPE_OWNED: i32 = 4;
/// Opens a chain of owners instead of a subscript. What follows is a word
/// count and then that many words, the closing [`ATTRIBUTE_TERMINATOR`]
/// included.
const OP_OWNER_CHAIN: i32 = 1602;
const ATTRIBUTE_TERMINATOR: i32 = 1701;
/// The owner chain opens with this and closes with this before its terminator.
const CHAIN_OPEN: i32 = 2;
const CHAIN_CLOSE: i32 = 6;
/// Below this a word cannot be a name, so it is one of the chain's own
/// connectives. See [`push_owner_chain`].
const NAME_FLOOR: i32 = 0x0008_1BF2;
/// The one-word chain that names no owner noun but the **previous element**:
/// `ATTRIB LEAWID OF PREV `. The HVAC implied-straight catalogue writes every
/// cross-section this way — `/RTUBE1-BOXI` … `/RTUBE3-BOXI` (`PXLE` / `PZLE`)
/// and `/CTUBE-TUBE` / `/CTUBE2-TUBE` (`PDIA`) in `ams7000_0001` (15194/226,
/// 252, 311, 321, 29188). The attribute part keeps the plain
/// [`ATTRIBUTE_SHAPE`] with the `[1, 1]` qualifier of an unsubscripted name,
/// and the chain is exactly three words — [`CHAIN_OPEN`], this word,
/// [`ATTRIBUTE_TERMINATOR`] — with no [`CHAIN_CLOSE`] before the terminator.
/// E3D prints it as `( ATTRIB LEAWID OF PREV  )` (`Q PXLE` on `/RTUBE2-BOXI`,
/// 2026-09-18, `_runs/ams-init-1112-7997-20260917-mc/e3d-hvac-all-121918/c6_plain_tubes.log`).
const PREVIOUS_ELEMENT: i32 = 10;
/// How E3D spells the [`PREVIOUS_ELEMENT`] owner, and the name an environment
/// sees in [`AttribRef::owner_chain`] for it.
pub const PREVIOUS_ELEMENT_NAME: &str = "PREV";
/// The qualifier of an attribute whose subscript is taken from the stack.
const SUBSCRIPT_FROM_STACK: [i32; 2] = [-1, -1];
/// The qualifier of an attribute named by a second word instead.
const SECOND_NAME_FOLLOWS: [i32; 2] = [1, 1];

/// Word 2 of a number sets this bit when the number is an IEEE double.
const DOUBLE_FLAG: i32 = 0x4000_0000;
const EXPONENT_MASK: u32 = 0x7FF;
const MANTISSA_MASK: u32 = 0x000F_FFFF;
/// Word 0 of a double carries the sign here. The redundant leading one that
/// sometimes appears at bit 20 is masked off with the rest of the word.
const SIGN_BIT: u32 = 0x4000_0000;
/// Without the flag the number is a scaled fixed-point value: 24576 with word
/// 2 of 5 is 24, and 8192 with word 2 of 1 is 0.5. Word 1 carries the fraction
/// below the scale, which is what separates the 45 degrees `PTCA` stores as
/// `23040 0 6` from the 44.999401 it stores as `23039 22720 6`.
const SCALED_SHIFT: i32 = 15;
const SCALED_FRACTION: f64 = (1u32 << SCALED_SHIFT) as f64;
/// Word 2 of a scaled number is a small binary exponent, of either sign. A
/// larger one is not a shape this recognises. A negative one sets every high
/// bit, [`DOUBLE_FLAG`] included: the 0.25 aps7200's pipe clamps (catalogue
/// `15195`) scale their U-bolt sleeves by is `16384 0 -1`, and read as a double
/// it came out NaN.
const MAX_SCALED_EXPONENT: i32 = 64;

/// Loosest first, so a child of the same precedence needs no brackets on the
/// left. Unary minus takes everything to its right: E3D prints
/// `( - ATTRIB PARA[2 ] / 2 )` for a negated quotient.
const PRECEDENCE_COMPARISON: u8 = 0;
const PRECEDENCE_NEGATE: u8 = 1;
const PRECEDENCE_ADDITIVE: u8 = 2;
const PRECEDENCE_MULTIPLICATIVE: u8 = 3;
const PRECEDENCE_ATOM: u8 = 4;

/// One value on the evaluation stack, with what it took to build it.
#[derive(Debug, Clone)]
struct Term {
    text: String,
    precedence: u8,
    /// The number itself, when the term is nothing but one. An attribute's
    /// subscript has to be a number, not a formula.
    number: Option<f64>,
}

impl Term {
    fn atom(text: String) -> Self {
        Term {
            text,
            precedence: PRECEDENCE_ATOM,
            number: None,
        }
    }

    /// Bracket this term where a parent of `required` precedence would
    /// otherwise read it wrong.
    fn nested(&self, required: u8) -> String {
        match self.precedence < required {
            true => format!("( {} )", self.text),
            false => self.text.clone(),
        }
    }
}

/// One step of the reverse-Polish program, as the words state it.
///
/// Parsing stops at what the wire says and leaves every reading to the
/// consumer, so [`render`] and [`evaluate_words`] cannot disagree about which
/// blobs are programs at all.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Number(f64),
    Text(String),
    Attribute(Attribute),
    Operator(i32),
}

/// An `ATTRIB …` operand, in the three shapes the form names one.
#[derive(Debug, Clone, PartialEq)]
struct Attribute {
    /// The system name, or — for a UDA — the `:0x…` spelling of its key,
    /// which [`render_with_dictionary`] replaces when a Dictionary knows it.
    name: String,
    /// `Some` when the name word is a Dictionary key rather than a system
    /// name: the program reads a user-defined attribute. See the module head.
    uda_key: Option<u32>,
    /// `ATTRIB PARA[…]` takes its subscript off the evaluation stack.
    subscript_from_stack: bool,
    /// `ATTRIB PARA[4 ]` with the subscript written into the operand itself:
    /// the qualifier words are `[n, n]` with `n >= 2` and no second name. The
    /// BS profile catalogue in ams7324 stores `SPVE.PX` as
    /// `0.5 * ATTRIB PARA[1 ] - ATTRIB PARA[4 ]` this way — `[1, 1]` for the
    /// first (the shape [`SECOND_NAME_FOLLOWS`] already reads as a plain
    /// attribute) and `[4, 4]` for the second, which nothing read before.
    inline_subscript: Option<i32>,
    /// The second word of `ATTRIB X Y`.
    qualifier: Option<String>,
    /// The `OF a OF b` owners, nearest first. Non-empty only for the chain form.
    owners: Vec<String>,
}

/// Read the whole program, or nothing.
fn parse(words: &[i32]) -> Option<Vec<Step>> {
    let program = program_of(words)?;
    let mut steps = Vec::new();
    let mut cursor = 0;

    while cursor < program.len() {
        let (step, consumed) = match program[cursor] {
            OP_PUSH_NUMBER => parse_number(&program[cursor..])?,
            OP_ATTRIBUTE => parse_attribute(&program[cursor..])?,
            OP_PUSH_TEXT => parse_text(&program[cursor..])?,
            OP_PI => (Step::Operator(OP_PI), 1),
            OP_NEGATE | OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE | OP_SIN | OP_COS
            | OP_TAN | OP_ASIN | OP_ATAN | OP_ATANT | OP_SQRT | OP_POW | OP_ABS | OP_ABS_REAL
            | OP_MAX | OP_MIN | OP_EQUAL | OP_GREATER_THAN | OP_LESS_THAN | OP_GREATER_EQUAL
            | OP_LESS_EQUAL | OP_MATCH | OP_TRIM | OP_LOGICAL_STRING | OP_UNSET_REAL | OP_OR
            | OP_IF_REAL => (Step::Operator(program[cursor]), 1),
            // An opcode this has never seen used is a program it has not read.
            _ => return None,
        };
        steps.push(step);
        cursor += consumed;
    }
    Some(steps)
}

/// Render the expression stored for an attribute, if this can read all of it.
///
/// A user-defined attribute prints as its key (`:0x17EE2B61`); give
/// [`render_with_dictionary`] the MDB's Dictionary to print `:HXYsize` instead.
pub fn render(words: &[i32]) -> Option<String> {
    render_with_dictionary(words, None)
}

/// [`render`], spelling a user-defined attribute by the name `dictionary`
/// gives its key — the way E3D prints `( :HXYsize[2 ] )`. A key the
/// Dictionary does not declare (as current `UKEY` or as `OLDKEY`) keeps its
/// `:0x…` spelling, so the text still says which UDA the formula wanted.
pub fn render_with_dictionary(words: &[i32], dictionary: Option<&UdaCatalog>) -> Option<String> {
    let mut stack: Vec<Term> = Vec::new();

    for step in parse(words)? {
        match step {
            Step::Number(value) => stack.push(Term {
                text: format_number(value),
                precedence: PRECEDENCE_ATOM,
                number: Some(value),
            }),
            Step::Text(value) => stack.push(Term::atom(format!("'{value}'"))),
            Step::Attribute(mut attribute) => {
                if let Some(name) = attribute
                    .uda_key
                    .and_then(|key| dictionary?.by_key(key))
                    .map(|definition| format!(":{}", definition.name))
                {
                    attribute.name = name;
                }
                let text = render_attribute(&attribute, &mut stack)?;
                stack.push(Term::atom(text));
            }
            Step::Operator(OP_NEGATE) => {
                let operand = stack.pop()?;
                stack.push(Term {
                    text: format!("- {}", operand.nested(PRECEDENCE_NEGATE)),
                    precedence: PRECEDENCE_NEGATE,
                    number: None,
                });
            }
            Step::Operator(OP_PI) => stack.push(Term::atom("PI".to_string())),
            Step::Operator(OP_SIN) => apply_function("SIN", &mut stack)?,
            Step::Operator(OP_COS) => apply_function("COS", &mut stack)?,
            Step::Operator(OP_TAN) => apply_function("TAN", &mut stack)?,
            Step::Operator(OP_ASIN) => apply_function("ASIN", &mut stack)?,
            Step::Operator(OP_ATAN) => apply_function("ATAN", &mut stack)?,
            Step::Operator(OP_SQRT) => apply_function("SQRT", &mut stack)?,
            Step::Operator(OP_POW) => apply_binary_function("POW", &mut stack)?,
            Step::Operator(OP_ABS | OP_ABS_REAL) => apply_function("ABS", &mut stack)?,
            Step::Operator(OP_MAX) => apply_binary_function("MAX", &mut stack)?,
            Step::Operator(OP_MIN) => apply_binary_function("MIN", &mut stack)?,
            Step::Operator(OP_MATCH) => apply_binary_function("MAT", &mut stack)?,
            Step::Operator(OP_TRIM) => apply_function("TRIM", &mut stack)?,
            Step::Operator(OP_LOGICAL_STRING) => apply_function("STR", &mut stack)?,
            Step::Operator(OP_UNSET_REAL) => apply_function("UNSET", &mut stack)?,
            Step::Operator(OP_ATANT) => apply_binary_function("ATANT", &mut stack)?,
            Step::Operator(OP_IF_REAL) => {
                let otherwise = stack.pop()?;
                let then = stack.pop()?;
                let condition = stack.pop()?;
                stack.push(Term::atom(format!(
                    "IFTRUE ( {} , {} , {} )",
                    condition.text, then.text, otherwise.text
                )));
            }
            Step::Operator(operator) => apply_binary(operator, &mut stack)?,
        }
    }

    match stack.as_slice() {
        [only] => Some(format!("( {} )", only.text)),
        _ => None,
    }
}

/// Work the program out to a number, against `env`.
///
/// Angles are degrees. Three things say so and none of them is a convention
/// borrowed from elsewhere: `PTAX.Pdistance` holds `ATTRIB PARA[3 ] * TAN ( 27
/// )`, and 27 radians makes that tangent **negative** — a P point behind the
/// component it belongs to; the census's other tangents are all the bend
/// formula `R * TAN ( ANGL / 2 )`, which is degrees wherever it is written; and
/// [`crate::record::axis_spec`] already reads the rotation angle beside these
/// as hundredths of a degree.
///
/// Returns `None`, never a guess, when the environment cannot answer an
/// attribute, when the program is one [`parse`] declines, when the stack does
/// not end on exactly one value, and when the arithmetic leaves the reals (a
/// division by an unset parameter is a degenerate to report, not an infinity to
/// pass on).
///
/// One reading is E3D's rather than a guess of ours: a program the fast path
/// (`fast_path_eligible`) would take reads a parameter-array entry the instance
/// does not have (`ATTRIB DESP[1 ]` on a valve with no `DESP`) as 0 and goes on,
/// exactly as `exppdms/EXFAST` does. Everywhere else the same miss is `None`.
///
/// This is the `pplib/YPARUN` reading — a catalogue **attribute** whose value
/// is a program (`PX`, `PDIS`, `PHEI`, …). Programs embedded in a direction /
/// position specification (`PTCD`'s angles) never see the fast path; those go
/// through [`evaluate_words_full`].
pub fn evaluate_words(words: &[i32], env: &dyn ParamEnv) -> Option<f64> {
    evaluate(words, env, FastPath::AsYparun)
}

/// [`evaluate_words`] without `EXFAST`'s concession: a parameter the instance
/// does not have is fatal whatever the program's shape.
///
/// `dopelib/DPEVAN` (core.dll `0x519D16D`) hands the angle inside `E ( ATTRIB
/// DESP[1 ] ) N` straight to `exprlib/EXEVRU`; only `pplib/YPARUN` tries
/// `EXFAST` first. So on a valve with no `DESP` the attribute `PX = ( 0.1 *
/// ATTRIB PARA[13 ] * SIN ( ATTRIB DESP[1 ] ) )` and the direction `E ( ATTRIB
/// DESP[1 ] ) N` both fail — but a direction `E ( ATTRIB DESP[1 ] + 10 ) N`
/// fails too, where the same program as an attribute would read 10. E3D's
/// export log for `/1WCC318VN` shows it: `PX` / `PY` each get a `parameter
/// number ( 1 ) out of range` line (the fast path tried and cleared it) and an
/// `unable to evaluate` line; `PTCD` gets only the `unable to evaluate` line.
/// `plant-model-5/outputs/rvm-check-20261010/ida/README.md` §③.3, §④.
pub fn evaluate_words_full(words: &[i32], env: &dyn ParamEnv) -> Option<f64> {
    evaluate(words, env, FastPath::Never)
}

/// Whether `exppdms/EXFAST` gets a look at the program before the full
/// evaluator does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FastPath {
    /// `pplib/YPARUN`: try the fast path when the program's shape allows.
    AsYparun,
    /// `dopelib` / anything that calls `EXEVRU` directly.
    Never,
}

fn evaluate(words: &[i32], env: &dyn ParamEnv, fast_path: FastPath) -> Option<f64> {
    let mut stack: Vec<Value> = Vec::new();
    let steps = parse(words)?;
    let fast_path = fast_path == FastPath::AsYparun && fast_path_eligible(&steps);

    for step in steps {
        match step {
            Step::Number(value) => stack.push(Value::Number(value)),
            Step::Text(value) => stack.push(Value::Text(value)),
            Step::Attribute(attribute) => {
                let subscript = match attribute.subscript_from_stack {
                    true => Some(stack.pop()?.number()?),
                    false => attribute.inline_subscript.map(f64::from),
                };
                if let Some(key) = attribute.uda_key {
                    // A UDA with a qualifier or an owner chain is a shape the
                    // corpus has not shown; only the plain and subscripted
                    // forms are read. The plain `[1, 1]` shape falls back to
                    // entry 1 the way a system attribute does below.
                    if attribute.qualifier.is_some() || !attribute.owners.is_empty() {
                        return None;
                    }
                    let mut value = env.uda_number(key, subscript);
                    if value.is_none() && subscript.is_none() {
                        value = env.uda_number(key, Some(1.0));
                    }
                    let value = value?;
                    if !value.is_finite() {
                        return None;
                    }
                    stack.push(Value::Number(value));
                    continue;
                }
                let owners: Vec<&str> = attribute.owners.iter().map(String::as_str).collect();
                let reference = AttribRef {
                    name: &attribute.name,
                    subscript,
                    qualifier: attribute.qualifier.as_deref(),
                    owner_chain: &owners,
                };
                let mut value = env.attribute_number(&reference);
                // The plain shape is `[1, 1]` — inline subscript 1. On a scalar
                // that is the value itself; on an array (`ATTRIB PARA` written
                // with `[1, 1]`) it is the first entry, which the unsubscripted
                // lookup cannot give.
                if value.is_none()
                    && subscript.is_none()
                    && attribute.qualifier.is_none()
                    && owners.is_empty()
                {
                    value = env.attribute_number(&AttribRef {
                        subscript: Some(1.0),
                        ..reference
                    });
                }
                let value = match value {
                    Some(value) => value,
                    // `EXFAST`'s one concession: a parameter the instance does
                    // not have reads as 0 and the program goes on. See
                    // [`fast_path_eligible`].
                    None if fast_path
                        && is_parameter_array(&attribute.name)
                        && env.fast_path_missing_parameter_is_zero(&attribute.name, subscript) =>
                    {
                        0.0
                    }
                    // Full path. 223 (the array is there, the index is not) stays
                    // on the stack as `Unset`: `UNSET` turns it true and `OR`
                    // counts it false. 202 (no array — a `FITT`'s `ODES`/`OPAR`
                    // with no `SCTN`) is a fault, and `UNSET` does not clear it.
                    // `fast_path_missing_parameter_is_zero` is that split: true
                    // means 223. The fast path already returned 0 or `None`
                    // above; a program `EXFAST` would run never contains `UNSET`.
                    None if !fast_path && is_parameter_array(&attribute.name) => {
                        let out_of_range = env.fast_path_missing_parameter_is_zero(
                            &attribute.name,
                            subscript,
                        );
                        stack.push(if out_of_range {
                            Value::Unset
                        } else {
                            Value::Fault
                        });
                        continue;
                    }
                    None => return None,
                };
                if !value.is_finite() {
                    return None;
                }
                stack.push(Value::Number(value));
            }
            Step::Operator(operator) => apply_value(operator, &mut stack)?,
        }
    }

    match stack.as_slice() {
        [Value::Number(only)] if only.is_finite() => Some(*only),
        _ => None,
    }
}

/// The attributes `exppdms/GATPAR` serves: the parameter arrays whose
/// Dictionary unit is `UNIPAR` (the unit travels with the entry). `EXFAST`
/// routes an `ATTRIB` of one of these through `GATPAR`; every other attribute
/// goes through `GATREA`, whose failures end the fast path instead.
const PARAMETER_ARRAYS: &[&str] = &[
    "PARA", "DESP", "IPAR", "CPAR", "DDES", "APAR", "ADES", "OPAR", "ODES",
];

fn is_parameter_array(name: &str) -> bool {
    PARAMETER_ARRAYS.contains(&name)
}

/// Deepest the fast path's value stack may get: `exppdms/EXFAST` keeps five
/// slots and gives up when a push would need the fifth.
const FAST_PATH_STACK: usize = 4;

/// Whether `exppdms/EXFAST` (core.dll `0x51A6EBE`) would run this program
/// rather than hand it to the full evaluator — and with it, whether a parameter
/// the instance does not have reads as 0.
///
/// E3D evaluates a catalogue expression twice over: `pplib/YPARUN` tries the
/// fast path first and only falls back to `exprlib/EXEVRD` when the fast path
/// declines. The fast path knows nothing but number constants, `ATTRIB` of a
/// real attribute with a single subscript (the `[-1, -1]`, `[n, n]` and
/// `[1, 1]` shapes, each closed by `1601 1701`), and `- + - * /` on a stack
/// of at most four values. Inside it, an `ATTRIB PARA[n]` / `ATTRIB DESP[n]`
/// that `GATPAR` cannot answer (223, parameter number out of range) prints
/// `Catalogue error … DESP parameter number ( n ) out of range`, **clears the
/// error and keeps the 0.0 `GATPAR` left on the stack** — the program finishes
/// and its value is used. On the full path the same miss is [`Value::Unset`]
/// (223 left on the stack). `UNSET` turns that into true and clears it; `OR`
/// and `IFTRUE` drop an error on a side they do not take. A 202 (no array)
/// is [`Value::Fault`] instead, so `UNSET` stays false and the error remains.
/// A program that
/// cannot absorb a 223 still fails, and `YPARUN` then prints `unable to evaluate`
/// catalogue expression for attribute …` and takes 0 for the whole attribute.
/// A miss that is not a parameter array is an immediate failure on either path.
///
/// So `( ATTRIB DESP[1 ] + ATTRIB PARA[2 ] * 0.5 )` on a valve with no `DESP`
/// is `0 + PARA[2] * 0.5` (ams7999 `/1RCV071VP`'s connection points sit exactly
/// there), while `( 0.1 * ATTRIB PARA[13 ] * SIN ( ATTRIB DESP[1 ] ) )` on the
/// same kind of valve is unresolved — the `SIN` sends it down the full path,
/// where the missing parameter is an error (ams7999 `/1WCC318VN`, `PX` 0 and
/// `PY` 0 where a 0-degree reading would have given `-52.5`). Both are in
/// `plant-model-5/outputs/rvm-check-20261010/ida/README.md` §③ with the
/// decompiled `EXFAST` / `GATPAR` beside them.
///
/// The unit-class check `EXFAST` also makes (`KCONDM` ∈ {0, 11, 21}) is not
/// modelled: catalogue numbers are plain or distance / angle, and all three
/// pass it.
fn fast_path_eligible(steps: &[Step]) -> bool {
    let mut depth = 0usize;
    for step in steps {
        match step {
            Step::Number(_) => depth += 1,
            Step::Attribute(attribute) => {
                // The owner-chain shape ends in `1602`, not `1601 1701`.
                if !attribute.owners.is_empty() {
                    return false;
                }
                if attribute.subscript_from_stack {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                depth += 1;
            }
            Step::Operator(OP_NEGATE) => {
                if depth == 0 {
                    return false;
                }
            }
            Step::Operator(OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE) => {
                if depth < 2 {
                    return false;
                }
                depth -= 1;
            }
            Step::Text(_) | Step::Operator(_) => return false,
        }
        if depth > FAST_PATH_STACK {
            return false;
        }
    }
    depth == 1
}

/// Catalogue conditions travel through logical and text values before MAT
/// turns the one-based match position back into a numeric dimension mask.
enum Value {
    Number(f64),
    Logical(bool),
    Text(String),
    /// GATPAR error 223, parameter number out of range. `UNSET` clears it.
    Unset,
    /// Any other evaluation fault (a non-finite result, or 223 mixed with a
    /// harder error). `UNSET` does not clear it; `IFTRUE` still drops it when
    /// it sits on the branch the condition does not take.
    Fault,
}

impl Value {
    fn number(self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }
    fn text(self) -> Option<String> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }
}

fn apply_value(operator: i32, stack: &mut Vec<Value>) -> Option<()> {
    let value = match operator {
        OP_GREATER_THAN | OP_LESS_THAN | OP_GREATER_EQUAL | OP_LESS_EQUAL => {
            let right = stack.pop()?;
            let left = stack.pop()?;
            match (left, right) {
                (Value::Number(left), Value::Number(right)) => {
                    let slack = comparison_slack(left, right);
                    Value::Logical(match operator {
                        OP_GREATER_THAN => left - right > slack,
                        OP_LESS_THAN => right - left > slack,
                        OP_GREATER_EQUAL => left - right >= -slack,
                        _ => right - left >= -slack,
                    })
                }
                (Value::Fault, _) | (_, Value::Fault) => Value::Fault,
                (Value::Unset, _) | (_, Value::Unset) => Value::Unset,
                _ => return None,
            }
        }
        OP_EQUAL => {
            let right = stack.pop()?;
            let left = stack.pop()?;
            match (left, right) {
                (Value::Number(left), Value::Number(right)) => {
                    Value::Logical(equal_within_slack(left, right))
                }
                (Value::Fault, _) | (_, Value::Fault) => Value::Fault,
                (Value::Unset, _) | (_, Value::Unset) => Value::Unset,
                _ => return None,
            }
        }
        // Case 1802: 223 becomes true and is cleared; a clean real is false;
        // any other fault stays a fault (EXPULE keeps the error).
        OP_UNSET_REAL => match stack.pop()? {
            Value::Unset => Value::Logical(true),
            Value::Number(_) => Value::Logical(false),
            Value::Fault => Value::Fault,
            _ => return None,
        },
        // Case 301: 223 is cleared and that side counts as false. A clean true
        // on either side wins without reading the other error. Two falses,
        // including unset, are false. A harder fault remains.
        OP_OR => {
            let right = stack.pop()?;
            let left = stack.pop()?;
            match (left, right) {
                (Value::Logical(true), _) | (_, Value::Logical(true)) => Value::Logical(true),
                (Value::Fault, _) | (_, Value::Fault) => Value::Fault,
                (
                    Value::Logical(false) | Value::Unset,
                    Value::Logical(false) | Value::Unset,
                ) => Value::Logical(false),
                _ => return None,
            }
        }
        // The branch not taken is dropped whole — Core pops both before it
        // looks at the logical, so an unreadable `b` cannot fail a program
        // whose condition selects `a`.
        OP_IF_REAL => {
            let otherwise = stack.pop()?;
            let then = stack.pop()?;
            let selected = match stack.pop()? {
                Value::Logical(true) => then,
                Value::Logical(false) => otherwise,
                // Core pops a logical here; an error or another type is a
                // program this has misread, not a condition to coerce.
                _ => return None,
            };
            match selected {
                Value::Number(value) if value.is_finite() => Value::Number(value),
                _ => return None,
            }
        }
        OP_LOGICAL_STRING => match stack.pop()? {
            Value::Logical(value) => Value::Text(value.to_string()),
            // 1410 is logical STR; other STR types have different opcodes.
            _ => return None,
        },
        OP_TRIM => Value::Text(stack.pop()?.text()?.trim_matches(' ').to_string()),
        OP_MATCH => {
            let pattern = stack.pop()?.text()?;
            let text = stack.pop()?.text()?;
            let position = text
                .find(&pattern)
                .map(|at| text[..at].chars().count() + 1)
                .unwrap_or(0);
            Value::Number(position as f64)
        }
        _ => return apply_numeric(operator, stack),
    };
    stack.push(value);
    Some(())
}

/// Apply one operator to the numeric stack.
fn apply_numeric(operator: i32, stack: &mut Vec<Value>) -> Option<()> {
    let value = match operator {
        OP_PI => Value::Number(std::f64::consts::PI),
        OP_NEGATE => unary_number(stack, |n| -n)?,
        OP_SIN => unary_number(stack, |n| n.to_radians().sin())?,
        OP_COS => unary_number(stack, |n| n.to_radians().cos())?,
        OP_TAN => unary_number(stack, |n| n.to_radians().tan())?,
        OP_ASIN => unary_number(stack, |n| n.asin().to_degrees())?,
        OP_ATAN => unary_number(stack, |n| n.atan().to_degrees())?,
        OP_SQRT => unary_number(stack, |n| n.sqrt())?,
        OP_ABS | OP_ABS_REAL => unary_number(stack, |n| n.abs())?,
        _ => binary_number(operator, stack)?,
    };
    stack.push(value);
    Some(())
}

/// A non-finite result is a fault, not an abort: `IFTRUE` has already popped
/// the branch it will not take, and that branch may be `360 / 0`.
fn finite_number(value: f64) -> Value {
    match value.is_finite() {
        true => Value::Number(value),
        false => Value::Fault,
    }
}

fn unary_number(stack: &mut Vec<Value>, op: impl FnOnce(f64) -> f64) -> Option<Value> {
    Some(match stack.pop()? {
        Value::Number(n) => finite_number(op(n)),
        Value::Unset => Value::Unset,
        Value::Fault => Value::Fault,
        _ => return None,
    })
}

/// `ATANT ( a , b )` is `atan2(a, b)` in degrees: the first argument is the
/// ordinate, the second the abscissa, and the quadrant is kept. Pinned against
/// E3D's TTY export of aps7200 `TRNS /HVAC2-B1/ITEM8` (acp7002 `/FORD`, a
/// round-to-rectangular duct transition): its two offset snouts take `PBAX`
/// from `P7` / `P8`, whose `PTCD` is `AXIS -Y ( ATANT ( ±DESP[10] - (DESP[4] -
/// DESP[5]) / 2 , 0 - DESP[11] ) ) ∓X`. With `DESP[10] = 200`, `DESP[4] = 800`,
/// `DESP[5] = 300`, `DESP[11] = 0` the angles are `atan2(-450, 0)` and
/// `atan2(-50, 0)` = −90°, which turns `-Y` onto `+X` / `-X`: the snouts lean
/// along the component's X axis (world −Y / +Y in the export), 450 and 50 mm.
/// The other order gives 180° and leaves both on `-Y`; the template default
/// `+Y` the failed evaluation used to fall back on put them 375 mm off. Read as
/// `atan2(a, b)` both chains point at the stadium's half-circle centres
/// `(DESP[10] ± (DESP[4] - DESP[5]) / 2, DESP[11])` for any offset, which is
/// what the catalogue author wrote them for.
fn binary_number(operator: i32, stack: &mut Vec<Value>) -> Option<Value> {
    let right = stack.pop()?;
    let left = stack.pop()?;
    let (left, right) = match (left, right) {
        (Value::Fault, _) | (_, Value::Fault) => return Some(Value::Fault),
        (Value::Unset, _) | (_, Value::Unset) => return Some(Value::Unset),
        (Value::Number(left), Value::Number(right)) => (left, right),
        _ => return None,
    };
    let value = match operator {
        OP_ADD => left + right,
        OP_SUBTRACT => left - right,
        OP_MULTIPLY => left * right,
        OP_DIVIDE => left / right,
        OP_POW if left != 0.0 || right > 0.0 => left.powf(right),
        OP_MAX => left.max(right),
        OP_MIN => left.min(right),
        OP_ATANT => left.atan2(right).to_degrees(),
        _ => return None,
    };
    Some(finite_number(value))
}

/// `EQ` the way `exprlib/EXRCTL` (`core.dll` `0x51D3B32`) compares two reals.
///
/// The slack is relative — `1e-6 * max(|a|, |b|)` — and is *capped* at 0.9 when
/// both operands are whole numbers inside the `int32` range. The cap only bites
/// past a magnitude of 9e5, which no catalogue parameter reaches; it is here
/// because transcribing half of a comparison is how a rule silently stops being
/// the one the reference uses.
///
/// Bit equality would be the wrong reading: these operands arrive from
/// [`decode_number`] and from `PARA`/`DESP` arrays, so a discriminator written
/// as `DESP[3] EQ 1` is comparing a stored float against a literal.
fn equal_within_slack(left: f64, right: f64) -> bool {
    (left - right).abs() <= comparison_slack(left, right)
}

/// The slack `exprlib/EXRCTL` hands every real comparison (see [`equal_within_slack`]).
fn comparison_slack(left: f64, right: f64) -> f64 {
    const RELATIVE: f64 = 1e-6;
    const WHOLE_NUMBER_CAP: f64 = 0.9;
    let whole = |value: f64| {
        value >= f64::from(i32::MIN)
            && value <= f64::from(i32::MAX)
            && (value - value.round()).abs() <= RELATIVE
    };
    let slack = RELATIVE * left.abs().max(right.abs());
    if whole(left) && whole(right) {
        slack.min(WHOLE_NUMBER_CAP)
    } else {
        slack
    }
}

/// Spell an operand the way E3D's `Q` does, taking its subscript off the stack.
///
/// A user-defined attribute is written by its `:name` alone — E3D prints
/// `( :HXYsize[2 ] )`, not `ATTRIB :HXYsize[2 ]`.
fn render_attribute(attribute: &Attribute, stack: &mut Vec<Term>) -> Option<String> {
    let mut text = match attribute.uda_key {
        Some(_) => attribute.name.clone(),
        None => format!("ATTRIB {}", attribute.name),
    };
    if attribute.subscript_from_stack {
        // The subscript has to be a number, not a formula.
        let subscript = stack.pop()?.number?;
        let _ = write!(text, "[{} ]", format_number(subscript));
    } else if let Some(subscript) = attribute.inline_subscript {
        let _ = write!(text, "[{subscript} ]");
    }
    if let Some(qualifier) = &attribute.qualifier {
        text.push(' ');
        text.push_str(qualifier);
    }
    for owner in &attribute.owners {
        let _ = write!(text, " OF {owner}");
    }
    // Both sampled chains print a trailing blank before the closing bracket
    // the caller adds.
    if !attribute.owners.is_empty() {
        text.push(' ');
    }
    Some(text)
}

/// Whether these words have the shape of a PML program at all: a length word
/// that counts the whole blob, then [`PROGRAM_MARKER`].
///
/// The explicit stream keys formulas by negated attribute hash, but not every
/// body stored there is PML. The BS profile catalogue in ams7324 (`/BS-...`,
/// reached when `SPRE` points straight at the `SPRF`) stores `SPVE.PX` as the
/// four-word tuple `[20, 1, 1, 0]` — `0.5 TIMES PARAM 1` — in that same slot.
/// [`evaluate_words`] declines it (20 is not the length), and the caller needs
/// to know whether to hand the words to the tuple reader or to report an
/// unreadable program. Two words decide that: `[20, 1, …]` fails on the first.
pub fn is_program(words: &[i32]) -> bool {
    program_of(words).is_some()
}

/// Strip the length word and the marker, checking the length as we go.
fn program_of(words: &[i32]) -> Option<&[i32]> {
    let (declared, rest) = words.split_first()?;
    if *declared as usize != words.len() {
        return None;
    }
    match rest.split_first() {
        Some((&PROGRAM_MARKER, program)) if !program.is_empty() => Some(program),
        _ => None,
    }
}

/// `118 <count> <character>...` pushes a text literal. Catalogue DATA rows use
/// this form for PPRO values such as `( '484' )`.
fn parse_text(words: &[i32]) -> Option<(Step, usize)> {
    let [OP_PUSH_TEXT, count, rest @ ..] = words else {
        return None;
    };
    let count = usize::try_from(*count).ok()?;
    let chars = rest.get(..count)?;
    let value: String = chars
        .iter()
        .map(|word| u8::try_from(*word).ok().map(char::from))
        .collect::<Option<_>>()?;
    Some((Step::Text(value), count + 2))
}

fn parse_number(words: &[i32]) -> Option<(Step, usize)> {
    // The fourth word is zero throughout the sample. Reading a formula whose
    // meaning turns on a word this has never seen used would be a guess.
    match words {
        [OP_PUSH_NUMBER, NUMBER_SHAPE, first, second, third, 0, end, ..]
            if NUMBER_ENDS.contains(end) =>
        {
            let value = decode_number(*first, *second, *third)?;
            Some((Step::Number(value), NUMBER_WORDS))
        }
        [OP_PUSH_NUMBER, NUMBER_WITH_UNIT_SHAPE, first, second, third, 0, unit @ ..]
            if unit.len() >= 4 && unit[..3] == DISTANCE_UNIT && unit[3] == 0 =>
        {
            let value = decode_number(*first, *second, *third)?;
            Some((Step::Number(value), NUMBER_WITH_UNIT_WORDS))
        }
        [OP_PUSH_NUMBER, NUMBER_TYPED_SHAPE, first, second, third, 0, trailer @ ..]
            if trailer.len() >= 2 && trailer[..2] == TYPED_TRAILER =>
        {
            let value = decode_number(*first, *second, *third)?;
            Some((Step::Number(value), NUMBER_TYPED_WORDS))
        }
        _ => None,
    }
}

fn decode_number(first: i32, second: i32, third: i32) -> Option<f64> {
    if third.unsigned_abs() < MAX_SCALED_EXPONENT.unsigned_abs() {
        let mantissa = f64::from(first) + f64::from(second as u32) / SCALED_FRACTION;
        return Some(mantissa * f64::from(third - SCALED_SHIFT).exp2());
    }
    if third & DOUBLE_FLAG == 0 {
        return None;
    }
    let sign = u64::from((first as u32 & SIGN_BIT) != 0);
    let exponent = u64::from(third as u32 & EXPONENT_MASK);
    let mantissa = u64::from(first as u32 & MANTISSA_MASK);
    Some(f64::from_bits(
        (sign << 63) | (exponent << 52) | (mantissa << 32) | u64::from(second as u32),
    ))
}

/// Print a number the way E3D's `Q` does.
///
/// The stored double for `0.3` is the one a decimal literal rounds to, and
/// printing every digit of it would say `0.30000000000000004`. Six places is
/// past anything the sample shows and still lands on `0.3`.
fn format_number(value: f64) -> String {
    let text = format!("{value:.6}");
    match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    }
}

fn parse_attribute(words: &[i32]) -> Option<(Step, usize)> {
    if let Some(found) = parse_previous_element_chain(words) {
        return Some(found);
    }
    if let Some(found) = parse_owner_chain(words) {
        return Some(found);
    }
    let [OP_ATTRIBUTE, ATTRIBUTE_SHAPE, name, first, second, third, end_first, end_second, ..] =
        words
    else {
        return None;
    };
    if [*end_first, *end_second] != ATTRIBUTE_END {
        return None;
    }
    let (name, uda_key) = attribute_name(*name)?;

    let attribute = match ([*first, *second], third) {
        (SUBSCRIPT_FROM_STACK, 0) => Attribute {
            name,
            uda_key,
            subscript_from_stack: true,
            inline_subscript: None,
            qualifier: None,
            owners: Vec::new(),
        },
        // The same qualifier words with no second name behind them: the
        // attribute stands on its own. `PTAX.Pdistance` reaches both readings
        // in one formula — `ATTRIB RADI * TAN ( ATTRIB ANGL / 2 )`.
        (SECOND_NAME_FOLLOWS, 0) => Attribute {
            name,
            uda_key,
            subscript_from_stack: false,
            inline_subscript: None,
            qualifier: None,
            owners: Vec::new(),
        },
        (SECOND_NAME_FOLLOWS, qualifier) => Attribute {
            name,
            uda_key,
            subscript_from_stack: false,
            inline_subscript: None,
            qualifier: Some(name_of(*qualifier)?),
            owners: Vec::new(),
        },
        // `[n, n]` past 1 with no second name: the subscript itself, written
        // inline (`ATTRIB PARA[4 ]` in the BS profile catalogue). See
        // [`Attribute::inline_subscript`].
        ([index, repeat], 0) if index == repeat && index >= 2 => Attribute {
            name,
            uda_key,
            subscript_from_stack: false,
            inline_subscript: Some(index),
            qualifier: None,
            owners: Vec::new(),
        },
        _ => return None,
    };
    Some((Step::Attribute(attribute), ATTRIBUTE_WORDS))
}

/// An attribute reached through its owners: `ATTRIB NAMN OF PSPE OF PIPE `.
///
/// The chain is a counted run between [`CHAIN_OPEN`] and [`CHAIN_CLOSE`] whose
/// names are ordinary hash words, in the order E3D prints them. What joins
/// them is not read — `2 0 34 PSPE 1 5 18 PIPE 6` gives up nothing about what
/// `34` or `1 5 18` decide, and the two sampled chains are too few to say. So
/// the connectives are only required to *be* connectives: every word that is
/// not a name must sit below [`NAME_FLOOR`], where no name can reach. A chain
/// carrying a large word this cannot name is one it has not seen, and it
/// declines rather than dropping the word.
///
/// Both sampled chains print a trailing blank before the closing bracket the
/// caller adds, which [`render_attribute`] adds back for a non-empty chain.
fn parse_owner_chain(words: &[i32]) -> Option<(Step, usize)> {
    let [OP_ATTRIBUTE, ATTRIBUTE_SHAPE_OWNED, name, 1, _, 0, OP_OWNER_CHAIN, length, chain @ ..] =
        words
    else {
        return None;
    };
    let length = usize::try_from(*length).ok()?;
    let chain = chain.get(..length)?;
    let [CHAIN_OPEN, owners @ .., CHAIN_CLOSE, ATTRIBUTE_TERMINATOR] = chain else {
        return None;
    };
    let (name, uda_key) = attribute_name(*name)?;

    let mut named = Vec::new();
    for word in owners {
        if *word < NAME_FLOOR {
            continue;
        }
        named.push(name_of(*word)?);
    }
    if named.is_empty() {
        return None;
    }
    let attribute = Attribute {
        name,
        uda_key,
        subscript_from_stack: false,
        inline_subscript: None,
        qualifier: None,
        owners: named,
    };
    Some((Step::Attribute(attribute), ATTRIBUTE_WORDS + length))
}

/// `ATTRIB <name> OF PREV `: the attribute read off the previous element.
///
/// Read whole or not at all, like the noun chain: the attribute words must be
/// the plain unsubscripted shape and the chain must be exactly
/// `[CHAIN_OPEN, PREVIOUS_ELEMENT, ATTRIBUTE_TERMINATOR]`. A chain that
/// mixes [`PREVIOUS_ELEMENT`] with noun names, or a subscripted attribute in
/// front of it, is a shape this has not seen and declines. The catalogue
/// blobs this was read from are the five HVAC straight primitives named at
/// [`PREVIOUS_ELEMENT`]; each is `[13, 1, 106, 2, <name>, 1, 1, 0, 1602, 3, 2,
/// 10, 1701]` with `<name>` one of `LEAWID` / `LEAHEI` / `LBOR`.
fn parse_previous_element_chain(words: &[i32]) -> Option<(Step, usize)> {
    let [OP_ATTRIBUTE, ATTRIBUTE_SHAPE, name, 1, 1, 0, OP_OWNER_CHAIN, 3, CHAIN_OPEN, PREVIOUS_ELEMENT, ATTRIBUTE_TERMINATOR, ..] =
        words
    else {
        return None;
    };
    let (name, uda_key) = attribute_name(*name)?;
    let attribute = Attribute {
        name,
        uda_key,
        subscript_from_stack: false,
        inline_subscript: None,
        qualifier: None,
        owners: vec![PREVIOUS_ELEMENT_NAME.to_string()],
    };
    Some((Step::Attribute(attribute), ATTRIBUTE_WORDS + 3))
}

fn name_of(hash: i32) -> Option<String> {
    let hash = hash as u32;
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// The name word of an `ATTRIB` operand: a system attribute by its name, or a
/// user-defined attribute by its Dictionary key.
///
/// Everything a system name can hash to lies at or below `SYSTEM_HASH_MAX`;
/// a positive word above it is the band UDA keys are allocated from (every
/// `UKEY` / `OLDKEY` in the AMS, ACP and SCB dictionaries: `0x171F…`–`0x2C00…`).
/// A negative word is not a name at all — the explicit stream keys formulas by
/// negated hash, and one of those inside a program is a shape not yet read.
fn attribute_name(word: i32) -> Option<(String, Option<u32>)> {
    if let Some(name) = name_of(word) {
        return Some((name, None));
    }
    let key = u32::try_from(word).ok()?;
    (key > SYSTEM_HASH_MAX).then(|| (format!(":0x{key:08X}"), Some(key)))
}

fn apply_function(name: &str, stack: &mut Vec<Term>) -> Option<()> {
    let argument = stack.pop()?;
    stack.push(Term::atom(format!("{name} ( {} )", argument.text)));
    Some(())
}

/// A function of two arguments, the second pushed last.
///
/// The brackets and the comma delimit the arguments already, so neither takes
/// any of its own: E3D prints `ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] -
/// 1.8 * ATTRIB PARA[7 ] )` with the difference bare.
fn apply_binary_function(name: &str, stack: &mut Vec<Term>) -> Option<()> {
    let second = stack.pop()?;
    let first = stack.pop()?;
    stack.push(Term::atom(format!(
        "{name} ( {} , {} )",
        first.text, second.text
    )));
    Some(())
}

fn apply_binary(operator: i32, stack: &mut Vec<Term>) -> Option<()> {
    let (symbol, precedence) = match operator {
        OP_ADD => ("+", PRECEDENCE_ADDITIVE),
        OP_SUBTRACT => ("-", PRECEDENCE_ADDITIVE),
        OP_MULTIPLY => ("*", PRECEDENCE_MULTIPLICATIVE),
        OP_DIVIDE => ("/", PRECEDENCE_MULTIPLICATIVE),
        OP_EQUAL => ("EQ", PRECEDENCE_COMPARISON),
        OP_GREATER_THAN => ("GT", PRECEDENCE_COMPARISON),
        OP_LESS_THAN => ("LT", PRECEDENCE_COMPARISON),
        OP_OR => ("OR", PRECEDENCE_COMPARISON),
        OP_GREATER_EQUAL => ("GE", PRECEDENCE_COMPARISON),
        OP_LESS_EQUAL => ("LE", PRECEDENCE_COMPARISON),
        _ => return None,
    };
    let right = stack.pop()?;
    let left = stack.pop()?;
    // `a - ( b - c )` and `a / ( b / c )` are not what dropping the brackets
    // would say, so the right operand of those two keeps them at equal
    // precedence.
    let right_required = match matches!(operator, OP_SUBTRACT | OP_DIVIDE) {
        true => precedence + 1,
        false => precedence,
    };
    stack.push(Term {
        text: format!(
            "{} {symbol} {}",
            left.nested(precedence),
            right.nested(right_required)
        ),
        precedence,
        number: None,
    });
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SLYK `/FHG_A_A1(S)_100` (SCOM 23055/25782) SEXT `23055/25724`.Pz — the
    /// U-bolt nut offset. Two `EQ` discriminators pick between three nut
    /// positions through two nested `IFTRUE`s; before 401/1822 were read the
    /// whole program was undecodable and five of the U-bolt's nine primitives
    /// silently went missing.
    ///
    /// With `DESP[1]=2`, `DESP[2]=0`, `RPRO DESC=1`, `PARA[8]=10.8`,
    /// `PARA[9]=114.3` this is `0.5 * 114.3 - 10.8 = 46.35`, and the nut E3D
    /// exports sits exactly there (local z −57.15..−46.35 along the SEXT's
    /// negated axis, thickness `PARA[8]`).
    #[test]
    fn the_ubolt_nut_offset_reads_its_two_iftrue_discriminators() {
        let words = [
            113, 1, 101, 6, 0, 0, 1073742846, 0, 0, 101, 6, 131072, 0, 1073742850, 0, 0, 106, 2,
            564289, -1, -1, 0, 1601, 1701, 804, 101, 6, 1048576, 0, 1073742847, 0, 0, 106, 2,
            860359, -1, -1, 0, 1601, 1701, 101, 6, 1048576, 0, 1073742847, 0, 0, 401, 101, 6, 0, 0,
            1073742848, 0, 0, 106, 2, 860359, -1, -1, 0, 1601, 1701, 106, 2, 840258, 1, 1, 604480,
            1601, 1701, 101, 6, 1048576, 0, 1073742847, 0, 0, 401, 101, 6, 0, 0, 1073741824, 0, 0,
            101, 6, 1572864, 0, 1073742849, 0, 0, 1822, 802, 101, 6, 0, 0, 1073742850, 0, 0, 106,
            2, 564289, -1, -1, 0, 1601, 1701, 801, 1822, 802,
        ];
        assert_eq!(
            render(&words).as_deref(),
            Some(
                "( 0.5 * ATTRIB PARA[9 ] + IFTRUE ( ATTRIB DESP[1 ] EQ 1 , \
                 ATTRIB DESP[2 ] + IFTRUE ( ATTRIB RPRO DESC EQ 1 , 0 , 6 ) , \
                 - ATTRIB PARA[8 ] ) )"
            )
        );

        let env = MapParamEnv::new()
            .set_attribute("PARA[9]", 114.3)
            .set_attribute("PARA[8]", 10.8)
            .set_attribute("DESP[1]", 2.0)
            .set_attribute("DESP[2]", 0.0)
            .set_attribute("RPRO DESC", 1.0);
        let value = evaluate_words(&words, &env).expect("both branches readable");
        assert!((value - 46.35).abs() < 1e-9, "{value}");

        // `A/G = 1` takes the other branch of the outer IFTRUE: the nut moves
        // to `DESP[2] + 0` above the datum instead of one thickness below it.
        let env = env.set_attribute("DESP[1]", 1.0);
        let value = evaluate_words(&words, &env).expect("other branch");
        assert!((value - 57.15).abs() < 1e-9, "{value}");
    }

    /// `EQ` is not bit equality: `exprlib/EXRCTL` allows `1e-6 * max(|a|,|b|)`.
    #[test]
    fn equality_carries_exrctls_relative_slack() {
        assert!(equal_within_slack(1.0, 1.0));
        assert!(equal_within_slack(0.0, 0.0), "零对零不能靠相对量");
        assert!(equal_within_slack(114.3, 114.3 + 1e-7));
        assert!(!equal_within_slack(114.3, 114.3 + 1e-3));
        assert!(!equal_within_slack(1.0, 2.0));
        // Past 9e5 two whole numbers stop tracking the relative rule.
        assert!(!equal_within_slack(2_000_000.0, 2_000_001.0));
        assert!(equal_within_slack(2_000_000.5, 2_000_001.5));
    }

    #[test]
    fn flex_body_poff_evaluates_the_native_pow_program() {
        // LSNO 15194/5489 /CFLEX-BODY, queried through E3D TTY.
        let words = [
            50, 1, 101, 6, 20480, 0, 4, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101, 6, 16384,
            0, 2, 0, 6, 1002, 101, 6, 22528, 0, 4, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701,
            101, 6, 16384, 0, 2, 0, 6, 1002, 802, 1001,
        ];
        assert_eq!(
            render(&words).as_deref(),
            Some("( SQRT ( POW ( ATTRIB DESP[10 ] , 2 ) + POW ( ATTRIB DESP[11 ] , 2 ) ) )")
        );
        let env = MapParamEnv::new()
            .set_attribute("DESP[10]", -3.0)
            .set_attribute("DESP[11]", 4.0);
        assert_eq!(evaluate_words(&words, &env), Some(5.0));
    }

    #[test]
    fn flex_extension_diameter_obeys_native_strict_threshold() {
        // SCYL 15194/22899 /CFLEX-AEXT3. Native MAT returns 1 or 0 here.
        let words = [
            66, 1, 101, 6, 16384, 0, 2, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101, 6, 20480,
            0, 5, 0, 6, 802, 101, 6, 30720, 0, 4, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101,
            6, 25600, 0, 6, 0, 6, 101, 6, 24576, 0, 2, 0, 6, 804, 601, 1410, 1314, 118, 4, 116,
            114, 117, 101, 1308, 804,
        ];
        assert_eq!(render(&words).as_deref(), Some("( ( ATTRIB DESP[2 ] + 20 ) * MAT ( TRIM ( STR ( ATTRIB DESP[15 ] GT 50 * 3 ) ) , 'true' ) )"));
        for (distance, diameter) in [(50.0, 0.0), (150.0, 0.0), (151.0, 370.0)] {
            let env = MapParamEnv::new()
                .set_attribute("DESP[2]", 350.0)
                .set_attribute("DESP[15]", distance);
            assert_eq!(evaluate_words(&words, &env), Some(diameter));
        }
    }

    /// A scaled number with a negative exponent: aps7200's U-bolt clamp
    /// (catalogue SCYL `15195/17376`) sizes its sleeve `0.25 * (…)`, the 0.25
    /// stored as `16384 0 -1`. Its `-1` carries [`DOUBLE_FLAG`] like every
    /// negative word, and read as a double the sleeve came out NaN and was not
    /// drawn; E3D draws it.
    #[test]
    fn a_negative_scaled_exponent_is_not_a_double() {
        let sleeve = [
            81, 1, 101, 6, 16384, 0, -1, 0, 6, 101, 6, 16384, 0, 2, 0, 6, 101, 6, 18432, 0, 4, 0,
            6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 101, 6, 16384, 0, 2, 0, 6, 101, 6,
            24576, 0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 803, 101, 6, 24576, 0,
            2, 0, 6, 101, 6, 16384, 0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 802,
            804,
        ];
        assert_eq!(
            render(&sleeve).as_deref(),
            Some("( 0.25 * ( 2 * ATTRIB PARA[9 ] - 2 * ATTRIB PARA[3 ] + 3 * ATTRIB PARA[2 ] ) )")
        );
        let env = MapParamEnv::new()
            .set_attribute("PARA[2]", 10.0)
            .set_attribute("PARA[3]", 20.0)
            .set_attribute("PARA[9]", 50.0);
        assert_eq!(evaluate_words(&sleeve, &env), Some(22.5));
        // The doubles keep their flag: 0.75 is `524288 0 1073742846`.
        assert_eq!(decode_number(524288, 0, 1_073_742_846), Some(0.75));
        assert_eq!(decode_number(16384, 0, -2), Some(0.125));
    }

    #[test]
    fn a_number_with_a_distance_unit_reads_as_its_value() {
        // `/F1_SQR_PFI_6H_003` GMSE SBOX (=2013287246/21086) PXLE: OPAR[2] plus a 20 stored with its
        // unit. With OPAR[2] = 309.2 (`/BS-UC305x305x137`) E3D draws the plate 329.2 wide.
        let plate = [
            28, 1, 101, 6, 0, 0, 1073742848, 0, 0, 106, 2, 886911, -1, -1, 0, 1601, 1701, 101, 9,
            1310720, 0, 1073742851, 0, 11, 1, 1, 0, 802,
        ];
        assert_eq!(render(&plate).as_deref(), Some("( ATTRIB OPAR[2 ] + 20 )"));
        let env = MapParamEnv::new().set_attribute("OPAR[2]", 309.2);
        assert_eq!(evaluate_words(&plate, &env), Some(309.2 + 20.0));
        // `/J2S4_EAV_PFI_PFI_003` NGMS NSEX (=2013287246/25848) PZ: `- PARA[1] - 1`, the 1 with its
        // unit — the rafter cutter starts 16 mm back (PARA[1] = 15).
        let cutter = [
            29, 1, 101, 6, 1048576, 0, 1073742847, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701,
            801, 101, 9, 1048576, 0, 1073742847, 0, 11, 1, 1, 0, 803,
        ];
        let env = MapParamEnv::new().set_attribute("PARA[1]", 15.0);
        assert_eq!(evaluate_words(&cutter, &env), Some(-16.0));
        // Any other trailer is a unit this has not seen: unreadable, not a guess.
        let mut other = plate;
        other[24] = 2;
        assert_eq!(evaluate_words(&other, &env), None);
    }

    #[test]
    fn a_typed_number_without_unit_words_reads_as_its_value() {
        // Door SFIT `/IS-SRO-NS/900x2045` (=15193/25228), jamb SEXT 15193/25264, SVER
        // 15193/25272 PY: the 2 and the 15 are `101 7 … 11 1`, the 0.5 a plain number.
        // WIDT 900 / DWID 806 → 61, the jamb width E3D's TTY RVM draws on
        // `FITTING 1 of STWALL 5 of CWALL /B01_LEV01_INT_WALLS` (aps250110).
        let jamb_width = [
            45, 1, 106, 2, 840258, 1, 1, 928283, 1601, 1701, 106, 2, 840258, 1, 1, 617359, 1601,
            1701, 803, 101, 7, 16384, 0, 2, 0, 11, 1, 803, 101, 6, 16384, 0, 0, 0, 6, 804, 101, 7,
            30720, 0, 4, 0, 11, 1, 802,
        ];
        assert_eq!(
            render(&jamb_width).as_deref(),
            Some("( ( ATTRIB RPRO WIDT - ATTRIB RPRO DWID - 2 ) * 0.5 + 15 )")
        );
        let env = MapParamEnv::new()
            .set_attribute("RPRO WIDT", 900.0)
            .set_attribute("RPRO DWID", 806.0)
            .set_attribute("RPRO THK", 40.0);
        assert_eq!(evaluate_words(&jamb_width, &env), Some(61.0));
        // The same vertex's PRAD: a lone 5, the fillet the RVM arc (56, -95) → (61, -90) turns.
        let fillet = [10, 1, 101, 7, 20480, 0, 3, 0, 11, 1];
        assert_eq!(render(&fillet).as_deref(), Some("( 5 )"));
        assert_eq!(evaluate_words(&fillet, &env), Some(5.0));
        // SVER 15193/25273 PX: `( ( ATTRIB RPRO THK + 1 ) * -1 )`, the 1 typed, the -1 plain.
        let rebate = [
            27, 1, 106, 2, 840258, 1, 1, 539696, 1601, 1701, 101, 7, 16384, 0, 1, 0, 11, 1, 802,
            101, 6, -16384, 0, 1, 0, 6, 804,
        ];
        assert_eq!(
            render(&rebate).as_deref(),
            Some("( ( ATTRIB RPRO THK + 1 ) * -1 )")
        );
        assert_eq!(evaluate_words(&rebate, &env), Some(-41.0));
        // Any other trailer is a shape this has not seen: unreadable, not a guess.
        let mut other = fillet;
        other[9] = 2;
        assert_eq!(evaluate_words(&other, &env), None);
    }

    #[test]
    fn real_comparisons_select_the_eaves_haunch_depths() {
        // `/J2S4_EAV_PFI_PFI_003` DTSE rows BONE / BTWO (=2013287246/25771, 25772):
        // `IFTRUE ( ANYZ LT 0 , BLEN / 2 , BLEN / 2 + OPYZ )` and
        // `IFTRUE ( ANYZ LT 0 , BLEN / 2 - OPYZ , BLEN / 2 )`, LT stored as 603.
        let bone = [
            60, 1, 106, 2, 840258, 1, 1, 1061803, 1601, 1701, 101, 6, 0, 0, 1073741824, 0, 0, 603,
            106, 2, 840258, 1, 1, 810974, 1601, 1701, 101, 6, 0, 0, 1073742848, 0, 0, 805, 106, 2,
            840258, 1, 1, 810974, 1601, 1701, 101, 6, 0, 0, 1073742848, 0, 0, 805, 106, 2, 840258,
            1, 1, 1061871, 1601, 1701, 802, 1822,
        ];
        let btwo = [
            60, 1, 106, 2, 840258, 1, 1, 1061803, 1601, 1701, 101, 6, 0, 0, 1073741824, 0, 0, 603,
            106, 2, 840258, 1, 1, 810974, 1601, 1701, 101, 6, 0, 0, 1073742848, 0, 0, 805, 106, 2,
            840258, 1, 1, 1061871, 1601, 1701, 803, 106, 2, 840258, 1, 1, 810974, 1601, 1701, 101,
            6, 0, 0, 1073742848, 0, 0, 805, 1822,
        ];
        assert_eq!(
            render(&bone).as_deref(),
            Some(
                "( IFTRUE ( ATTRIB RPRO ANYZ LT 0 , ATTRIB RPRO BLEN / 2 , ATTRIB RPRO BLEN / 2 + ATTRIB RPRO OPYZ ) )"
            )
        );
        let env = |anyz: f64| {
            MapParamEnv::new()
                .set_attribute("RPRO ANYZ", anyz)
                .set_attribute("RPRO BLEN", 200.0)
                .set_attribute("RPRO OPYZ", 0.4 * anyz)
        };
        assert_eq!(evaluate_words(&bone, &env(10.0)), Some(104.0));
        assert_eq!(evaluate_words(&btwo, &env(10.0)), Some(100.0));
        assert_eq!(evaluate_words(&bone, &env(-10.0)), Some(100.0));
        assert_eq!(evaluate_words(&btwo, &env(-10.0)), Some(104.0));
        // LT against the slack: 0 is not below 0.
        assert_eq!(evaluate_words(&bone, &env(0.0)), Some(100.0));
    }

    #[test]
    fn the_real_comparisons_follow_core_with_its_slack() {
        let mut stack = vec![Value::Number(1.0), Value::Number(1.0)];
        apply_value(OP_GREATER_EQUAL, &mut stack).unwrap();
        assert!(matches!(stack.as_slice(), [Value::Logical(true)]));
        for (operator, left, right, expected) in [
            (OP_GREATER_THAN, 2.0, 1.0, true),
            (OP_GREATER_THAN, 1.0, 1.0, false),
            (OP_LESS_THAN, 1.0, 2.0, true),
            (OP_LESS_THAN, 1.0, 1.0, false),
            (OP_GREATER_EQUAL, 1.0, 1.0 + 1e-9, true),
            (OP_GREATER_EQUAL, 1.0, 2.0, false),
            (OP_LESS_EQUAL, 1.0 + 1e-9, 1.0, true),
            (OP_LESS_EQUAL, 2.0, 1.0, false),
        ] {
            let mut stack = vec![Value::Number(left), Value::Number(right)];
            apply_value(operator, &mut stack).unwrap();
            assert!(
                matches!(stack.as_slice(), [Value::Logical(got)] if *got == expected),
                "{operator} {left} {right}"
            );
        }
    }

    #[test]
    fn flex_extension_distance_uses_native_minimum() {
        let words = [
            33, 1, 101, 6, 30720, 0, 4, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101, 6, 25600,
            0, 6, 0, 6, 101, 6, 24576, 0, 2, 0, 6, 804, 1009,
        ];
        for (distance, expected) in [(50.0, 50.0), (200.0, 150.0)] {
            let env = MapParamEnv::new().set_attribute("DESP[15]", distance);
            assert_eq!(evaluate_words(&words, &env), Some(expected));
        }
    }

    #[test]
    fn cable_tray_rung_programs_use_native_abs_and_max() {
        // Core's built-in function table maps 1005=ABS and 1008=MAX. Both
        // occur in AMS /SW5/SL3/STRAIGHT-VAR-GMSE rung PY programs.
        let abs = [
            18, 1, 101, 6, 0, 0, 1073742848, 0, 6, 101, 6, 1310720, 0, 1073742849, 0, 6, 803, 1005,
        ];
        assert_eq!(render(&abs).as_deref(), Some("( ABS ( 2 - 5 ) )"));
        assert_eq!(evaluate_words(&abs, &MapParamEnv::new()), Some(3.0));

        let max = [
            17, 1, 101, 6, 0, 0, 1073742848, 0, 6, 101, 6, 1310720, 0, 1073742849, 0, 6, 1008,
        ];
        assert_eq!(render(&max).as_deref(), Some("( MAX ( 2 , 5 ) )"));
        assert_eq!(evaluate_words(&max, &MapParamEnv::new()), Some(5.0));
    }

    #[test]
    fn breeches_cutter_programs_use_the_real_abs_opcode() {
        // `/RBREECH-NGAP` (=15194/8524) PCTP, read back from ams7000 on 2026-09-19:
        // `( ATTRIB DESP[42 ] + ABS ( ATTRIB DESP[11 ] ) )`, ABS encoded as 1007.
        // With DESP[42] = 710 and DESP[11] = 100 (THREEWAY 1 of BRANCH /-CUP-S-3-M-1405)
        // E3D's `holes off` RVM writes the cutter's top y-length as 810.
        let words = [
            34, 1, 101, 6, 21504, 0, 6, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101, 6, 22528,
            0, 4, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 1007, 802,
        ];
        assert_eq!(
            render(&words).as_deref(),
            Some("( ATTRIB DESP[42 ] + ABS ( ATTRIB DESP[11 ] ) )")
        );
        let env = MapParamEnv::new()
            .set_attribute("DESP[42]", 710.0)
            .set_attribute("DESP[11]", 100.0);
        assert_eq!(evaluate_words(&words, &env), Some(810.0));
        // A negative operand is what ABS is there for.
        let env = MapParamEnv::new()
            .set_attribute("DESP[42]", 710.0)
            .set_attribute("DESP[11]", -352.5);
        assert_eq!(evaluate_words(&words, &env), Some(1062.5));
    }

    /// Every distinct expression blob in the 2026-08-26 TTY sample, against
    /// what E3D printed for the attribute that owns it.
    #[test]
    fn renders_sampled_expressions_the_way_e3d_does() {
        let cases: &[(&[i32], &str)] = &[
            // A bare number, in both of the shapes a number is stored in.
            (&[9, 1, 101, 6, 24576, 0, 5, 0, 6], "( 24 )"),
            // The scaled shape carries its fraction in word 1. These three
            // are `PTCA.Ptcdirection` angles read back on 2026-08-27.
            (&[9, 1, 101, 6, 30719, 22720, 5, 0, 6], "( 29.999701 )"),
            (&[9, 1, 101, 6, 23039, 22720, 6, 0, 6], "( 44.999401 )"),
            (&[9, 1, 101, 6, 17920, 1664, 6, 0, 6], "( 35.000099 )"),
            (&[9, 1, 101, 6, 1048576, 0, 1073742851, 0, 6], "( 16 )"),
            // An attribute subscripted by the number under it on the stack.
            (
                &[
                    17, 1, 101, 6, 16384, 0, 1, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
                ],
                "( ATTRIB PARA[1 ] )",
            ),
            (
                &[
                    17, 1, 101, 6, 1048576, 0, 1073742847, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701,
                ],
                "( ATTRIB PARA[1 ] )",
            ),
            // An attribute of an attribute, named rather than subscripted.
            (
                &[10, 1, 106, 2, 840258, 1, 1, 703815, 1601, 1701],
                "( ATTRIB RPRO FLTH )",
            ),
            // Multiplication by a fraction, in both number shapes.
            (
                &[
                    25, 1, 101, 6, 8192, 0, 1, 0, 6, 101, 6, 20480, 0, 5, 0, 6, 106, 2, 564289, -1,
                    -1, 0, 1601, 1701, 804,
                ],
                "( 0.5 * ATTRIB PARA[20 ] )",
            ),
            (
                &[
                    25, 1, 101, 6, 524288, 0, 1073742846, 0, 6, 101, 6, 1310720, 0, 1073742849, 0,
                    6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804,
                ],
                "( 0.75 * ATTRIB PARA[5 ] )",
            ),
            // A negative coefficient: the sign rides bit 30 of the mantissa.
            (
                &[
                    25, 1, 101, 6, 1073741824, 0, 1073742846, 0, 6, 101, 6, 0, 0, 1073742848, 0, 6,
                    106, 2, 564289, -1, -1, 0, 1601, 1701, 804,
                ],
                "( -0.5 * ATTRIB PARA[2 ] )",
            ),
            (
                &[
                    25, 1, 101, 6, 734003, 858993459, 1073742846, 0, 6, 101, 6, 393216, 0,
                    1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804,
                ],
                "( 0.85 * ATTRIB PARA[11 ] )",
            ),
            // Sum and difference of two parameters.
            (
                &[
                    33, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
                    6, 32768, 0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802,
                ],
                "( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )",
            ),
            (
                &[
                    33, 1, 101, 6, 24576, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
                    6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 803,
                ],
                "( ATTRIB PARA[6 ] - ATTRIB PARA[5 ] )",
            ),
            (
                &[
                    33, 1, 101, 6, 1310720, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 786432, 0, 1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 803,
                ],
                "( ATTRIB PARA[5 ] - ATTRIB PARA[14 ] )",
            ),
            // A difference divided by two keeps its brackets; a quotient that
            // is then negated does not get any.
            (
                &[
                    41, 1, 101, 6, 24576, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
                    6, 16384, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 803, 101, 6,
                    16384, 0, 2, 0, 6, 805,
                ],
                "( ( ATTRIB PARA[6 ] - ATTRIB PARA[4 ] ) / 2 )",
            ),
            (
                &[
                    26, 1, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
                    101, 6, 0, 0, 1073742848, 0, 6, 805, 801,
                ],
                "( - ATTRIB PARA[2 ] / 2 )",
            ),
            (
                &[
                    25, 1, 101, 6, 393216, 0, 1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 0, 0, 1073742848, 0, 6, 805,
                ],
                "( ATTRIB PARA[11 ] / 2 )",
            ),
            // Multiplication binds tighter than the subtraction around it.
            (
                &[
                    57, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 262144, 0, 1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 802, 101, 6, 1258291, 858993460, 1073742845, 0, 6, 101, 6, 1572864, 0,
                    1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 803,
                ],
                "( ATTRIB PARA[4 ] + ATTRIB PARA[10 ] - 0.3 * ATTRIB PARA[6 ] )",
            ),
            // Trigonometry.
            (
                &[
                    34,
                    1,
                    101,
                    6,
                    1677721,
                    -1717986918,
                    1073742845,
                    0,
                    6,
                    101,
                    6,
                    655360,
                    0,
                    1073742850,
                    0,
                    6,
                    106,
                    2,
                    564289,
                    -1,
                    -1,
                    0,
                    1601,
                    1701,
                    804,
                    101,
                    6,
                    1966080,
                    0,
                    1073742851,
                    0,
                    6,
                    902,
                    804,
                ],
                "( 0.4 * ATTRIB PARA[13 ] * COS ( 30 ) )",
            ),
            (
                &[
                    34,
                    1,
                    101,
                    6,
                    1677721,
                    -1717986918,
                    1073742845,
                    0,
                    6,
                    101,
                    6,
                    655360,
                    0,
                    1073742850,
                    0,
                    6,
                    106,
                    2,
                    564289,
                    -1,
                    -1,
                    0,
                    1601,
                    1701,
                    804,
                    101,
                    6,
                    1966080,
                    0,
                    1073742851,
                    0,
                    6,
                    901,
                    804,
                ],
                "( 0.4 * ATTRIB PARA[13 ] * SIN ( 30 ) )",
            ),
            (
                &[
                    17, 1, 101, 6, 1572864, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701,
                ],
                "( ATTRIB PARA[6 ] )",
            ),
            // Tangent, and an attribute standing on its own. All three
            // `PTAX.Pdistance` formulas the 2026-08-27 census turned up.
            (
                &[
                    26, 1, 101, 6, 524288, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 1769472, 0, 1073742851, 0, 6, 903, 804,
                ],
                "( ATTRIB PARA[3 ] * TAN ( 27 ) )",
            ),
            (
                &[
                    28, 1, 106, 2, 711549, 1, 1, 0, 1601, 1701, 106, 2, 773119, 1, 1, 0, 1601,
                    1701, 101, 6, 0, 0, 1073742848, 0, 6, 805, 903, 804,
                ],
                "( ATTRIB RADI * TAN ( ATTRIB ANGL / 2 ) )",
            ),
            (
                &[
                    43, 1, 106, 2, 840258, 1, 1, 711549, 1601, 1701, 101, 6, 425984, 0, 1073742852,
                    0, 6, 101, 6, 0, 0, 1073742848, 0, 6, 805, 903, 804, 101, 6, 524288, 0,
                    1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802,
                ],
                "( ATTRIB RPRO RADI * TAN ( 45 / 2 ) + ATTRIB PARA[3 ] )",
            ),
            // Two-argument arctangent, both `PTCA.Ptcdirection` turns that
            // hold one. Neither argument is bracketed inside the call.
            (
                &[
                    49, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
                    101, 6, 1835008, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
                    802, 907,
                ],
                "( ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] + ATTRIB PARA[7 ] ) )",
            ),
            (
                &[
                    57, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601,
                    1701, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
                    101, 6, 1887436, -858993459, 1073742847, 0, 6, 101, 6, 1835008, 0, 1073742849,
                    0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 803, 907,
                ],
                "( ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] - 1.8 * ATTRIB PARA[7 ] ) )",
            ),
        ];
        for (words, expected) in cases {
            assert_eq!(
                render(words).as_deref(),
                Some(*expected),
                "blob {words:?} should render as {expected}"
            );
        }
    }

    /// Both `TABQUE.Qusexpression` blobs in `ams7000_0001`, against what E3D
    /// printed for them on 2026-08-27.
    #[test]
    fn renders_an_attribute_reached_through_its_owners() {
        assert_eq!(
            render(&[
                20, 1, 106, 4, 816521, 1, 500, 0, 1602, 10, 2, 0, 34, 642049, 1, 5, 18, 641779, 6,
                1701
            ])
            .as_deref(),
            Some("( ATTRIB NAMN OF PSPE OF PIPE  )")
        );
        assert_eq!(
            render(&[14, 1, 106, 4, 74699205, 1, 100, 0, 1602, 4, 2, 640493, 6, 1701]).as_deref(),
            Some("( ATTRIB FACODE OF ZONE  )")
        );
    }

    /// The HVAC implied-straight primitives read their cross-section off the
    /// element **before** the tube: `/RTUBE2-BOXI` (`ams7000_0001` 15194/252)
    /// stores `PXLE` / `PZLE` as these words and E3D prints them as
    /// `( ATTRIB LEAWID OF PREV  )` / `( ATTRIB LEAHEI OF PREV  )`; `/RTUBE1-BOXI`
    /// (15194/226) swaps the two names, `/CTUBE-TUBE` (15194/321) reads `LBOR`.
    /// The name words are the ordinary `db1_hash` of the pseudo-attribute.
    #[test]
    fn renders_an_attribute_of_the_previous_element() {
        let leawid = e3d_attlib::db1_hash("LEAWID") as i32;
        let leahei = e3d_attlib::db1_hash("LEAHEI") as i32;
        let lbor = e3d_attlib::db1_hash("LBOR") as i32;
        assert_eq!(leawid, 63163623, "the word the RTUBE2-BOXI blob carries");
        assert_eq!(leahei, 132487149, "the word the RTUBE1-BOXI blob carries");
        assert_eq!(
            render(&[13, 1, 106, 2, leawid, 1, 1, 0, 1602, 3, 2, 10, 1701]).as_deref(),
            Some("( ATTRIB LEAWID OF PREV  )")
        );
        assert_eq!(
            render(&[13, 1, 106, 2, leahei, 1, 1, 0, 1602, 3, 2, 10, 1701]).as_deref(),
            Some("( ATTRIB LEAHEI OF PREV  )")
        );
        assert_eq!(
            render(&[13, 1, 106, 2, lbor, 1, 1, 0, 1602, 3, 2, 10, 1701]).as_deref(),
            Some("( ATTRIB LBOR OF PREV  )")
        );
    }

    /// The evaluator hands the previous-element owner to the environment under
    /// the same spelling the renderer prints, so a fixture keyed
    /// `LEAWID OF PREV` answers it; a chain the environment does not know stays
    /// unanswered rather than falling back to a bare `LEAWID`.
    #[test]
    fn evaluates_an_attribute_of_the_previous_element_through_the_environment() {
        use crate::record::catalogue_eval::MapParamEnv;
        let leawid = e3d_attlib::db1_hash("LEAWID") as i32;
        let words = [13, 1, 106, 2, leawid, 1, 1, 0, 1602, 3, 2, 10, 1701];
        let env = MapParamEnv::new().set_attribute("LEAWID OF PREV", 1460.0);
        assert_eq!(evaluate_words(&words, &env), Some(1460.0));
        let bare = MapParamEnv::new().set_attribute("LEAWID", 1460.0);
        assert_eq!(evaluate_words(&words, &bare), None);
    }

    /// The previous-element chain is read whole or not at all: a chain that
    /// closes like the noun form, one that mixes in a noun, or a subscripted
    /// attribute in front of it are shapes this has not seen.
    #[test]
    fn declines_a_previous_element_chain_of_another_shape() {
        let leawid = e3d_attlib::db1_hash("LEAWID") as i32;
        assert_eq!(
            render(&[14, 1, 106, 2, leawid, 1, 1, 0, 1602, 4, 2, 10, 6, 1701]),
            None
        );
        assert_eq!(
            render(&[14, 1, 106, 2, leawid, 1, 1, 0, 1602, 4, 2, 10, 640493, 1701]),
            None
        );
        assert_eq!(
            render(&[13, 1, 106, 2, leawid, -1, -1, 0, 1602, 3, 2, 10, 1701]),
            None
        );
    }

    /// The chain's connectives are unread, so one carrying a word large enough
    /// to be a name that this cannot name is a chain it has not seen.
    #[test]
    fn declines_an_owner_chain_it_cannot_read_whole() {
        // A word above the name floor that dehashes to nothing.
        assert_eq!(
            render(&[
                14,
                1,
                106,
                4,
                74699205,
                1,
                100,
                0,
                1602,
                4,
                2,
                0x7FFF_FFFF,
                6,
                1701
            ]),
            None
        );
        // No owner named at all.
        assert_eq!(
            render(&[13, 1, 106, 4, 74699205, 1, 100, 0, 1602, 3, 2, 6, 1701]),
            None
        );
        // A count that runs past the words.
        assert_eq!(
            render(&[14, 1, 106, 4, 74699205, 1, 100, 0, 1602, 9, 2, 640493, 6, 1701]),
            None
        );
        // The subscripted form's shape does not open an owner chain.
        assert_eq!(
            render(&[14, 1, 106, 2, 74699205, 1, 100, 0, 1602, 4, 2, 640493, 6, 1701]),
            None
        );
    }

    #[test]
    fn declines_what_is_not_a_program() {
        // A length that does not match the words.
        assert_eq!(render(&[8, 1, 101, 6, 24576, 0, 5, 0, 6]), None);
        // No leading marker.
        assert_eq!(render(&[8, 101, 6, 24576, 0, 5, 0, 6]), None);
        // An operator with nothing under it.
        assert_eq!(render(&[3, 1, 804]), None);
        // Two values and no operator to join them.
        assert_eq!(
            render(&[16, 1, 101, 6, 24576, 0, 5, 0, 6, 101, 6, 24576, 0, 5, 0, 6]),
            None
        );
        assert_eq!(render(&[]), None);
        assert_eq!(render(&[2, 1]), None);
    }

    /// The catalogue tuple decoder and this one must not both claim a blob.
    #[test]
    fn declines_a_catalogue_tuple() {
        assert_eq!(render(&[4, 40, 1, 2, 0]), None);
        assert_eq!(render(&[28, 1, 2, 0]), None);
    }

    // ---- evaluation ---------------------------------------------------

    use crate::record::catalogue_eval::MapParamEnv;

    #[test]
    fn sqrd_snout_offset_sqrt_opcode() {
        // E3D TTY Q POFF on LSNO =15194/4150 prints SQRT; its stream ends
        // in opcode 1001. A 250 x 250 inlet needs sqrt(125² + 125²).
        let words = [10, 1, 101, 6, 18432, 0, 4, 0, 6, 1001]; // SQRT(9)
        assert_eq!(evaluate_words(&words, &MapParamEnv::new()), Some(3.0));
        assert!(render(&words).unwrap().contains("SQRT"));
        let negative = [11, 1, 101, 6, 18432, 0, 4, 0, 6, 801, 1001];
        assert_eq!(evaluate_words(&negative, &MapParamEnv::new()), None);
        let angle = [10, 1, 101, 6, 16384, 0, 1, 0, 6, 906]; // ATAN(1)
        assert_eq!(evaluate_words(&angle, &MapParamEnv::new()), Some(45.0));
    }

    /// The three `PTAX.Pdistance` blobs of the census, and the `PARA` values a
    /// real 50 mm component carries (`SCOM /ACP1000-TFVL-50`).
    fn bend_env() -> MapParamEnv {
        MapParamEnv::new()
            .set_attribute("PARA[1]", 50.0)
            .set_attribute("PARA[3]", 3000.0)
            .set_attribute("PARA[9]", 1.5)
            .set_attribute("RADI", 100.0)
            .set_attribute("ANGL", 90.0)
    }

    /// `ATTRIB PARA[3 ] * TAN ( 27 )`, the shape the whole degree reading rests
    /// on: 27 radians makes this tangent negative, which would put the P point
    /// behind its own component.
    #[test]
    fn trigonometry_is_in_degrees() {
        let words = &[
            26, 1, 101, 6, 524288, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
            6, 1769472, 0, 1073742851, 0, 6, 903, 804,
        ];
        assert_eq!(
            render(words).as_deref(),
            Some("( ATTRIB PARA[3 ] * TAN ( 27 ) )")
        );
        let got = evaluate_words(words, &bend_env()).expect("the census formula evaluates");
        assert!(
            (got - 3000.0 * 27f64.to_radians().tan()).abs() < 1e-9,
            "expected 3000 * tan(27 degrees), got {got}"
        );
        assert!(got > 0.0, "27 radians would make this negative, got {got}");
    }

    /// PCLA /1XP19 uses `ASIN(PARA[10] / PARA[2])` in the implicit Pz and
    /// Ptcdirection programs of points P11..P14. Opcode 904 therefore returns
    /// degrees, matching the forward trigonometric operators.
    #[test]
    fn inverse_sine_is_rendered_and_evaluated_in_degrees() {
        let words = &[
            34, 1, 101, 6, 262144, 0, 1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
            6, 0, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 805, 904,
        ];
        let env = MapParamEnv::new()
            .set_attribute("PARA[10]", 3.0)
            .set_attribute("PARA[2]", 6.0);
        assert_eq!(
            render(words).as_deref(),
            Some("( ASIN ( ATTRIB PARA[10 ] / ATTRIB PARA[2 ] ) )")
        );
        assert!((evaluate_words(words, &env).unwrap() - 30.0).abs() < 1.0e-12);
    }

    /// `ATTRIB RADI * TAN ( ATTRIB ANGL / 2 )` — the bend tangent length, and
    /// the one formula in the census that reads two bare attributes at once.
    #[test]
    fn evaluates_the_bend_tangent_length() {
        let words = &[
            28, 1, 106, 2, 711549, 1, 1, 0, 1601, 1701, 106, 2, 773119, 1, 1, 0, 1601, 1701, 101,
            6, 0, 0, 1073742848, 0, 6, 805, 903, 804,
        ];
        // A right-angle bend's tangent length is its radius.
        let got = evaluate_words(words, &bend_env()).expect("evaluates");
        assert!((got - 100.0).abs() < 1e-9, "expected 100, got {got}");
    }

    /// Sum, difference and a subscript taken off the stack, in one program.
    #[test]
    fn evaluates_the_shapes_the_census_turns_up() {
        // `( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )`
        let sum = &[
            33, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 32768,
            0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802,
        ];
        let env = MapParamEnv::new()
            .set_attribute("PARA[5]", 7.0)
            .set_attribute("PARA[4]", 5.0);
        assert_eq!(evaluate_words(sum, &env), Some(12.0));

        // `( 0.5 * ATTRIB PARA[20 ] )`
        let scaled = &[
            25, 1, 101, 6, 8192, 0, 1, 0, 6, 101, 6, 20480, 0, 5, 0, 6, 106, 2, 564289, -1, -1, 0,
            1601, 1701, 804,
        ];
        let env = MapParamEnv::new().set_attribute("PARA[20]", 24.0);
        assert_eq!(evaluate_words(scaled, &env), Some(12.0));
    }

    /// ams7999 `/NPHS-SU-PTSE` P6 (`23984/99015`), the arc point of the saddle
    /// ring `SRTO 23984/99019`: `PX` / `PZ` place it at half the sweep angle
    /// round the pipe, the angle itself being `180 * I / ( PI * ( OD + t ) )`
    /// degrees. With `PARA = [150, 168.3, 10, 3.4, 150, 45, 175.1, 0.2]`
    /// (`/NPSU168/10S/A`) that is 15.0166°, and E3D's own RVM record for
    /// `/1WCC-R220.003/ATTA-02` sweeps exactly twice it (0.52417 rad) at
    /// centre radius 85.85. Opcode 111 was unread and all three programs
    /// were declined.
    #[test]
    fn evaluates_pi_in_the_saddle_arc_programs() {
        let px = &[
            100, 1, 101, 6, 0, 0, 1073742848, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6,
            1048576, 0, 1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802, 101, 6, 0, 0,
            1073742848, 0, 0, 805, 101, 6, 425984, 0, 1073742854, 0, 0, 101, 6, 1572864, 0,
            1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 111, 101, 6, 0, 0,
            1073742848, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 1048576, 0,
            1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802, 804, 805, 901, 804,
        ];
        let pz = &[
            101, 1, 101, 6, 0, 0, 1073742848, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6,
            1048576, 0, 1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802, 101, 6, 0, 0,
            1073742848, 0, 0, 805, 101, 6, 425984, 0, 1073742854, 0, 0, 101, 6, 1572864, 0,
            1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 111, 101, 6, 0, 0,
            1073742848, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 1048576, 0,
            1073742849, 0, 0, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802, 804, 805, 902, 804, 801,
        ];
        // The angle inside the same point's `PTCD` (`X ( … ) -Z`), count word and all.
        let angle = &[
            59, 1, 101, 6, 425984, 0, 1073742854, 0, 0, 101, 6, 1572864, 0, 1073742849, 0, 0, 106,
            2, 564289, -1, -1, 0, 1601, 1701, 804, 111, 101, 6, 0, 0, 1073742848, 0, 0, 106, 2,
            564289, -1, -1, 0, 1601, 1701, 101, 6, 1048576, 0, 1073742849, 0, 0, 106, 2, 564289,
            -1, -1, 0, 1601, 1701, 802, 804, 805,
        ];
        assert_eq!(
            render(px).as_deref(),
            Some(
                "( ( ATTRIB PARA[2 ] + ATTRIB PARA[4 ] ) / 2 * SIN ( 180 * ATTRIB PARA[6 ] / \
                 ( PI * ( ATTRIB PARA[2 ] + ATTRIB PARA[4 ] ) ) ) )"
            )
        );
        assert_eq!(
            render(angle).as_deref(),
            Some("( 180 * ATTRIB PARA[6 ] / ( PI * ( ATTRIB PARA[2 ] + ATTRIB PARA[4 ] ) ) )")
        );
        let env = MapParamEnv::new()
            .set_attribute("PARA[2]", 168.3)
            .set_attribute("PARA[4]", 3.4)
            .set_attribute("PARA[6]", 45.0);
        let half_sweep_deg = 180.0 * 45.0 / (std::f64::consts::PI * 171.7);
        let got = evaluate_words_full(angle, &env).expect("PI reads");
        assert!((got - half_sweep_deg).abs() < 1.0e-9, "{got}");
        let centre_radius = 171.7 / 2.0;
        let got = evaluate_words(px, &env).expect("PX reads");
        assert!(
            (got - centre_radius * half_sweep_deg.to_radians().sin()).abs() < 1.0e-9,
            "{got}"
        );
        let got = evaluate_words(pz, &env).expect("PZ reads");
        assert!(
            (got + centre_radius * half_sweep_deg.to_radians().cos()).abs() < 1.0e-9,
            "{got}"
        );
        // A bare `PI` is a program of one operand.
        assert_eq!(render(&[3, 1, 111]).as_deref(), Some("( PI )"));
        assert_eq!(
            evaluate_words(&[3, 1, 111], &MapParamEnv::new()),
            Some(std::f64::consts::PI)
        );
    }

    /// `( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )` is a program `exppdms/EXFAST`
    /// runs itself, and there a parameter `GATPAR` cannot answer is 0 and the
    /// sum goes on — E3D prints `parameter number ( 4 ) out of range` and
    /// uses the value anyway. ams7999 `/1RCV071VP` (no `DESP`, PTRE
    /// `DESP[1] + PARA[2] * 0.5`) has its connection points at exactly
    /// `0 + PARA[2] * 0.5` in the native RVM.
    #[test]
    fn a_missing_parameter_reads_as_zero_on_the_fast_path() {
        let words = &[
            33, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 32768,
            0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802,
        ];
        // Only one of the two `PARA` subscripts is registered.
        let env = MapParamEnv::new().set_attribute("PARA[5]", 7.0);
        assert_eq!(evaluate_words(words, &env), Some(7.0));
        assert!(render(words).is_some(), "it prints as before");
    }

    /// The same two parameters with a `SIN` between them: the fast path does
    /// not know `SIN`, declines the program, and the full evaluator makes the
    /// missing parameter fatal. ams7999 `/1WCC318VN` (no `DESP`) gets `PX` and
    /// `PY` from `( … * SIN ( ATTRIB DESP[1 ] ) )` / `( … * COS ( … ) )`, and
    /// E3D logs `unable to evaluate catalogue expression for attribute PY`
    /// and draws it at 0 — not at the `-52.5` a 0-degree reading would give.
    #[test]
    fn a_missing_parameter_is_fatal_off_the_fast_path() {
        let words = &[
            34, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 32768,
            0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 901, 802,
        ];
        assert_eq!(
            render(words).as_deref(),
            Some("( ATTRIB PARA[5 ] + SIN ( ATTRIB PARA[4 ] ) )")
        );
        let missing = MapParamEnv::new().set_attribute("PARA[5]", 7.0);
        assert_eq!(evaluate_words(words, &missing), None);
        let present = missing.set_attribute("PARA[4]", 30.0);
        assert_eq!(evaluate_words(words, &present), Some(7.5));
    }

    /// Only the parameter arrays get the fast path's concession: `GATPAR`
    /// serves those, and it is `GATPAR`'s error the fast path clears. A plain
    /// attribute goes through `GATREA`, whose failure ends the fast path and
    /// leaves the full evaluator to fail the program.
    #[test]
    fn a_missing_plain_attribute_is_fatal_even_on_the_fast_path() {
        // `( ATTRIB PARA[5 ] + ATTRIB ANGL )`; 773119 is `ANGL`.
        let words = &[
            26, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 106, 2,
            773119, 1, 1, 0, 1601, 1701, 802,
        ];
        assert_eq!(
            render(words).as_deref(),
            Some("( ATTRIB PARA[5 ] + ATTRIB ANGL )")
        );
        let env = MapParamEnv::new().set_attribute("PARA[5]", 7.0);
        assert_eq!(evaluate_words(words, &env), None);
        assert_eq!(
            evaluate_words(words, &env.set_attribute("ANGL", 90.0)),
            Some(97.0)
        );
    }

    /// What the fast path takes, step for step: constants, single-subscript
    /// attributes, the four operators and negation, at most four values deep.
    #[test]
    fn the_fast_path_takes_exactly_what_exfast_does() {
        let steps = |words: &[i32]| parse(words).expect("a program");
        // `( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )`.
        assert!(fast_path_eligible(&steps(&[
            33, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 32768,
            0, 2, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802,
        ])));
        // `( - ATTRIB PARA[5 ] )`.
        assert!(fast_path_eligible(&steps(&[
            18, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 801,
        ])));
        // A function.
        assert!(!fast_path_eligible(&steps(&[
            18, 1, 101, 6, 20480, 0, 3, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 901,
        ])));
        // An owner chain (`1602` where `1601 1701` would be).
        assert!(!fast_path_eligible(&steps(&[
            20, 1, 106, 4, 816521, 1, 500, 0, 1602, 10, 2, 0, 34, 642049, 1, 5, 18, 641779, 6,
            1701,
        ])));
        // Five constants on the stack before the first operator.
        assert!(!fast_path_eligible(&steps(&[
            41, 1, 101, 6, 16384, 0, 1, 0, 6, 101, 6, 16384, 0, 1, 0, 6, 101, 6, 16384, 0, 1, 0, 6,
            101, 6, 16384, 0, 1, 0, 6, 101, 6, 16384, 0, 1, 0, 6, 802, 802, 802, 802,
        ])));
        // Four is still inside the five slots.
        assert!(fast_path_eligible(&steps(&[
            33, 1, 101, 6, 16384, 0, 1, 0, 6, 101, 6, 16384, 0, 1, 0, 6, 101, 6, 16384, 0, 1, 0, 6,
            101, 6, 16384, 0, 1, 0, 6, 802, 802, 802,
        ])));
    }

    /// `ATANT ( a , b )` is `atan2(a, b)` in degrees — ordinate first, quadrant
    /// kept. acp7002 `/FORD` P7 (`=15194/4232`) stores `( ATANT ( 0 - ATTRIB
    /// DESP[10 ] - ( ATTRIB DESP[4 ] - ATTRIB DESP[5 ] ) / 2 , 0 - ATTRIB
    /// DESP[11 ] ) )` word for word as below; on aps7200 `TRNS /HVAC2-B1/ITEM8`
    /// (`DESP[10] = 200`, `[4] = 800`, `[5] = 300`, `[11] = 0`) E3D turns `-Y`
    /// through it onto `+X`, so the angle is −90°, not the 180° of the other
    /// order. Both arguments non-zero keeps the quadrant: an offset of (0, 250)
    /// is `atan2(-250, -250)` = −135°.
    #[test]
    fn the_two_argument_arctangent_is_atan2_ordinate_first() {
        let words = &[
            89, 1, 101, 6, 0, 0, 0, 0, 6, 101, 6, 20480, 0, 4, 0, 6, 106, 2, 860359, -1, -1, 0,
            1601, 1701, 803, 101, 6, 16384, 0, 3, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101,
            6, 20480, 0, 3, 0, 6, 106, 2, 860359, -1, -1, 0, 1601, 1701, 803, 101, 6, 16384, 0, 2,
            0, 6, 805, 803, 101, 6, 0, 0, 0, 0, 6, 101, 6, 22528, 0, 4, 0, 6, 106, 2, 860359, -1,
            -1, 0, 1601, 1701, 803, 907,
        ];
        assert_eq!(
            render(words).as_deref(),
            Some(
                "( ATANT ( 0 - ATTRIB DESP[10 ] - ( ATTRIB DESP[4 ] - ATTRIB DESP[5 ] ) / 2 , 0 - ATTRIB DESP[11 ] ) )"
            )
        );
        let item8 = MapParamEnv::new()
            .set_attribute("DESP[10]", 200.0)
            .set_attribute("DESP[4]", 800.0)
            .set_attribute("DESP[5]", 300.0)
            .set_attribute("DESP[11]", 0.0);
        assert_eq!(evaluate_words_full(words, &item8), Some(-90.0));
        let leaning = MapParamEnv::new()
            .set_attribute("DESP[10]", 0.0)
            .set_attribute("DESP[4]", 800.0)
            .set_attribute("DESP[5]", 300.0)
            .set_attribute("DESP[11]", 250.0);
        assert_eq!(evaluate_words_full(words, &leaning), Some(-135.0));
        // `ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] + ATTRIB PARA[7 ] )`: 3 over 9.
        let words = &[
            49, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701,
            101, 6, 0, 0, 1073742848, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101, 6, 1835008,
            0, 1073742849, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 802, 907,
        ];
        let env = MapParamEnv::new()
            .set_attribute("PARA[4]", 3.0)
            .set_attribute("PARA[2]", 4.0)
            .set_attribute("PARA[7]", 5.0);
        let got = evaluate_words(words, &env).expect("numbers");
        assert!((got - 3.0_f64.atan2(9.0).to_degrees()).abs() < 1e-12, "{got}");
    }

    /// Dividing by an unset parameter is a degenerate to report, not an
    /// infinity to hand downstream where it becomes a P point at the horizon.
    #[test]
    fn arithmetic_that_leaves_the_reals_declines() {
        // `( ATTRIB PARA[11 ] / 2 )` with the divisor swapped in as zero.
        let words = &[
            25, 1, 101, 6, 393216, 0, 1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 101,
            6, 0, 0, 0, 0, 6, 805,
        ];
        let env = MapParamEnv::new().set_attribute("PARA[11]", 1.0);
        assert_eq!(render(words).as_deref(), Some("( ATTRIB PARA[11 ] / 0 )"));
        assert_eq!(evaluate_words(words, &env), None);
    }

    /// ams7324 `/BS-…` profile catalogue, `SPVE 31896/9872 .PX`: the subscripts
    /// are written into the operands (`[1, 1]` and `[4, 4]`) instead of being
    /// pushed on the stack. The first shape already read as a plain `ATTRIB
    /// PARA`; the second was declined, so the vertex — and the 72 SCTN plus 72
    /// GENSEC on that profile — fell back to a centreline.
    #[test]
    fn reads_an_inline_subscript() {
        let words = &[
            27, 1, 101, 6, 8192, 0, 1, 0, 6, 106, 2, 564289, 1, 1, 0, 1601, 1701, 804, 106, 2,
            564289, 4, 4, 0, 1601, 1701, 803,
        ];
        assert_eq!(
            render(words).as_deref(),
            Some("( 0.5 * ATTRIB PARA - ATTRIB PARA[4 ] )")
        );
        let env = MapParamEnv::new()
            .set_attribute("PARA[1]", 80.0)
            .set_attribute("PARA[4]", 23.4);
        assert_eq!(evaluate_words(words, &env), Some(0.5 * 80.0 - 23.4));
    }

    /// The inline form is only `[n, n]`; a mismatched pair is still a shape
    /// this has not seen.
    #[test]
    fn a_mismatched_inline_subscript_is_declined() {
        let words = &[10, 1, 106, 2, 564289, 4, 5, 0, 1601, 1701];
        assert_eq!(render(words), None);
    }

    /// A program neither reader accepts stays rejected by both.
    #[test]
    fn the_two_readers_agree_on_what_is_a_program() {
        let env = MapParamEnv::new().set_attribute("PARA[1]", 1.0);
        for words in [
            [8, 1, 101, 6, 24576, 0, 5, 0, 6].as_slice(),
            [8, 101, 6, 24576, 0, 5, 0, 6].as_slice(),
            [3, 1, 804].as_slice(),
            [4, 40, 1, 2, 0].as_slice(),
            [2, 1].as_slice(),
            [].as_slice(),
        ] {
            assert_eq!(render(words), None, "renders {words:?}");
            assert_eq!(evaluate_words(words, &env), None, "numbers {words:?}");
        }
    }

    #[test]
    fn renders_text_literal_programs() {
        assert_eq!(
            render(&[7, 1, 118, 3, 52, 56, 52]).as_deref(),
            Some("( '484' )")
        );
        assert_eq!(render(&[5, 1, 118, 1, 32]).as_deref(), Some("( ' ' )"));
        assert!(render(&[6, 1, 118, 3, 52, 56]).is_none());
    }

    /// `/HRTUBE1-BOXI.PXLE` and `.PZLE` as `acp7002_0001` stores them
    /// (2026-09-18, `AvevaCatalogue`): `push 2` / `push 1`, then a subscripted
    /// attribute whose name word `401558369 = 0x17EF4B61` is a Dictionary key
    /// — the `OLDKEY` of `acp7006_0001`'s `/UDA/HVAC/HXYsize`.
    const HRTUBE1_PXLE: [i32; 17] = [
        17, 1, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2, 401558369, -1, -1, 0, 1601, 1701,
    ];
    const HRTUBE1_PZLE: [i32; 17] = [
        17, 1, 101, 6, 1048576, 0, 1073742847, 0, 6, 106, 2, 401558369, -1, -1, 0, 1601, 1701,
    ];
    const HXYSIZE_CATALOGUE_KEY: u32 = 0x17EF_4B61;

    fn hxysize_dictionary(key: u32, old_key: Option<u32>) -> UdaCatalog {
        use crate::uda_catalog::UdaDefinition;
        UdaCatalog::from_definitions(vec![UdaDefinition {
            refno: crate::RefNo::new(15198, 55),
            owner: crate::RefNo::new(15198, 3),
            key,
            old_key,
            element_name: "/UDA/HVAC/HXYsize".into(),
            name: "HXYsize".into(),
            dynamic_name: false,
            is_pseudo: false,
            value_type_hash: 768_519,
            value_type: Some("REAL".into()),
            length: 2,
            applicable_nouns: vec![808_220],
            default_is_defined: true,
            default_type: Some("REAL".into()),
            default_mode: 0,
            default_text: None,
            default_real: Some(0.0),
            default_location: None,
            default_attribute: None,
            default_expression: None,
        }])
    }

    /// A key word parses where a name word would have declined the whole
    /// program, and prints as the key until a Dictionary names it — E3D prints
    /// `( :HXYsize[2 ] )`, without `ATTRIB`.
    #[test]
    fn a_uda_key_renders_as_its_key_or_its_dictionary_name() {
        assert_eq!(
            render(&HRTUBE1_PXLE).as_deref(),
            Some("( :0x17EF4B61[2 ] )")
        );
        assert_eq!(
            render(&HRTUBE1_PZLE).as_deref(),
            Some("( :0x17EF4B61[1 ] )")
        );
        let by_current_key = hxysize_dictionary(HXYSIZE_CATALOGUE_KEY, None);
        assert_eq!(
            render_with_dictionary(&HRTUBE1_PXLE, Some(&by_current_key)).as_deref(),
            Some("( :HXYsize[2 ] )")
        );
        // The key the catalogue was compiled with has since moved (acp7006:
        // UKEY 0x2C00D576): OLDKEY still names it.
        let by_old_key = hxysize_dictionary(0x2C00_D576, Some(HXYSIZE_CATALOGUE_KEY));
        assert_eq!(
            render_with_dictionary(&HRTUBE1_PZLE, Some(&by_old_key)).as_deref(),
            Some("( :HXYsize[1 ] )")
        );
        // A Dictionary that declares the same name under other keys (ams5100)
        // does not name this word: the text keeps saying which key the formula
        // wanted.
        let same_name_other_keys = hxysize_dictionary(0x2652_AB52, Some(0x18EF_4B61));
        assert_eq!(
            render_with_dictionary(&HRTUBE1_PXLE, Some(&same_name_other_keys)).as_deref(),
            Some("( :0x17EF4B61[2 ] )")
        );
    }

    /// The value is asked of the environment **by key**. An environment that
    /// holds the key answers the subscript; one that does not — no Dictionary,
    /// or a Dictionary that never declared this key — leaves the formula
    /// unresolved rather than reaching for a same-named attribute.
    #[test]
    fn a_uda_evaluates_by_key_through_the_environment() {
        use crate::record::catalogue_eval::MapParamEnv;
        let env = MapParamEnv::new().with_uda(HXYSIZE_CATALOGUE_KEY, [675.0, 600.0]);
        assert_eq!(evaluate_words(&HRTUBE1_PXLE, &env), Some(600.0));
        assert_eq!(evaluate_words(&HRTUBE1_PZLE, &env), Some(675.0));

        let other_key = MapParamEnv::new().with_uda(0x2652_AB52, [675.0, 600.0]);
        assert_eq!(evaluate_words(&HRTUBE1_PXLE, &other_key), None);
        assert_eq!(evaluate_words(&HRTUBE1_PXLE, &MapParamEnv::new()), None);

        // The plain `[1, 1]` shape reads a scalar UDA whole, and an array's
        // first entry the way `ATTRIB PARA` written `[1, 1]` does.
        let plain = [
            10,
            1,
            106,
            2,
            HXYSIZE_CATALOGUE_KEY as i32,
            1,
            1,
            0,
            1601,
            1701,
        ];
        assert_eq!(render(&plain).as_deref(), Some("( :0x17EF4B61 )"));
        let scalar = MapParamEnv::new().with_uda(HXYSIZE_CATALOGUE_KEY, [350.0]);
        assert_eq!(evaluate_words(&plain, &scalar), Some(350.0));
        assert_eq!(evaluate_words(&plain, &env), Some(675.0));
    }

    /// Only positive words above the system band are keys. A negative word in
    /// the name slot is the explicit stream's negated-hash convention, which
    /// no program has been seen to carry, and a small word is a connective.
    #[test]
    fn only_a_word_above_the_system_band_is_a_uda_key() {
        assert_eq!(
            attribute_name(0x2652_AB52),
            Some((":0x2652AB52".into(), Some(0x2652_AB52)))
        );
        assert_eq!(
            attribute_name(SYSTEM_HASH_MAX as i32 + 1).map(|(_, key)| key),
            Some(Some(SYSTEM_HASH_MAX + 1))
        );
        assert_eq!(attribute_name(564289), Some(("PARA".into(), None)));
        assert_eq!(attribute_name(0xFFF6_3EDCu32 as i32), None);
        assert_eq!(attribute_name(3), None);
        assert_eq!(
            render(&[17, 1, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2, -1, -1, -1, 0, 1601, 1701]),
            None
        );
    }

    /// acp7002 `15194/317` PTAX `ALLANG`. core.dll `sub_51B4976` case 1802 is
    /// real `UNSET` (true only on GATPAR 223) and case 301 is logical `OR`.
    /// The two stack readings that both balance collapse to the first:
    /// `IFTRUE ( UNSET ( DESP[78] ) OR DESP[78] LT 1 , 1 , 360 / DESP[78] )`.
    #[test]
    fn evaluates_unset_or_in_the_allang_program() {
        use crate::record::catalogue_eval::MapParamEnv;
        let words = [
            73, 1, 101, 6, 1277952, 0, 1073742853, 0, 0, 106, 2, 860359, -1, -1, 0, 1601, 1701,
            1802, 101, 6, 1277952, 0, 1073742853, 0, 0, 106, 2, 860359, -1, -1, 0, 1601, 1701, 101,
            6, 1048576, 0, 1073742847, 0, 0, 603, 301, 101, 6, 1048576, 0, 1073742847, 0, 0, 101,
            6, 1474560, 0, 1073742855, 0, 0, 101, 6, 1277952, 0, 1073742853, 0, 0, 106, 2, 860359,
            -1, -1, 0, 1601, 1701, 805, 1822,
        ];
        assert_eq!(
            render(&words).as_deref(),
            Some(
                "( IFTRUE ( UNSET ( ATTRIB DESP[78 ] ) OR ATTRIB DESP[78 ] LT 1 , 1 , 360 / ATTRIB DESP[78 ] ) )"
            )
        );
        // Missing DESP[78] is 223, so UNSET is true and the division is dropped.
        assert_eq!(evaluate_words(&words, &MapParamEnv::new()), Some(1.0));
        let at = |n: f64| MapParamEnv::new().set_attribute("DESP[78]", n);
        // 0 is defined and < 1, so the result is still 1; 360/0 is not taken.
        assert_eq!(evaluate_words(&words, &at(0.0)), Some(1.0));
        assert_eq!(evaluate_words(&words, &at(0.5)), Some(1.0));
        assert_eq!(evaluate_words(&words, &at(1.0)), Some(360.0));
        assert_eq!(evaluate_words(&words, &at(4.0)), Some(90.0));
        assert_eq!(evaluate_words(&words, &at(8.0)), Some(45.0));

        // GATPAR 202 is not 223. `UNSET` is true only when the array exists
        // and the index does not; a missing array stays a fault, so ALLANG
        // does not resolve to 1.
        struct MissingArray;
        impl crate::record::catalogue_eval::ParamEnv for MissingArray {
            fn param(&self, _: i32) -> Option<f64> {
                None
            }
            fn iparam(&self, _: i32) -> Option<f64> {
                None
            }
            fn design_dimension(&self, _: i32) -> Option<f64> {
                None
            }
            fn attribute_number(
                &self,
                _: &crate::record::catalogue_eval::AttribRef<'_>,
            ) -> Option<f64> {
                None
            }
            fn fast_path_missing_parameter_is_zero(&self, _: &str, _: Option<f64>) -> bool {
                false
            }
            fn direction_angle(&self, _: &[i32]) -> Option<f64> {
                None
            }
        }
        assert_eq!(evaluate_words(&words, &MissingArray), None);
    }
}
