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
//! Anything this does not fully understand returns `None`. A half-read formula
//! would be worse than the all-zero tuple it replaces, because it would look
//! like an answer.

use std::fmt::Write as _;

use e3d_attlib::{db1_dehash, lookup_system_name};

/// Word 0 of the program, before any opcode. Every sampled blob carries it.
const PROGRAM_MARKER: i32 = 1;
const OP_PUSH_NUMBER: i32 = 101;
const OP_ATTRIBUTE: i32 = 106;
const OP_NEGATE: i32 = 801;
const OP_ADD: i32 = 802;
const OP_SUBTRACT: i32 = 803;
const OP_MULTIPLY: i32 = 804;
const OP_DIVIDE: i32 = 805;
const OP_SIN: i32 = 901;
const OP_COS: i32 = 902;
const OP_TAN: i32 = 903;
/// Two-argument arctangent. `PTCA.Ptcdirection` turns through one.
const OP_ATANT: i32 = 907;

/// `101` is followed by this word, then four words of number, then it again.
const NUMBER_SHAPE: i32 = 6;
const NUMBER_WORDS: usize = 7;
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
/// Word 2 of a scaled number is a small binary exponent. A larger one is not a
/// shape this recognises.
const MAX_SCALED_EXPONENT: i32 = 64;

/// Loosest first, so a child of the same precedence needs no brackets on the
/// left. Unary minus takes everything to its right: E3D prints
/// `( - ATTRIB PARA[2 ] / 2 )` for a negated quotient.
const PRECEDENCE_NEGATE: u8 = 0;
const PRECEDENCE_ADDITIVE: u8 = 1;
const PRECEDENCE_MULTIPLICATIVE: u8 = 2;
const PRECEDENCE_ATOM: u8 = 3;

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

/// Render the expression stored for an attribute, if this can read all of it.
pub fn render(words: &[i32]) -> Option<String> {
    let program = program_of(words)?;
    let mut stack: Vec<Term> = Vec::new();
    let mut cursor = 0;

    while cursor < program.len() {
        let consumed = match program[cursor] {
            OP_PUSH_NUMBER => push_number(&program[cursor..], &mut stack)?,
            OP_ATTRIBUTE => push_attribute(&program[cursor..], &mut stack)?,
            OP_NEGATE => {
                let operand = stack.pop()?;
                stack.push(Term {
                    text: format!("- {}", operand.nested(PRECEDENCE_NEGATE)),
                    precedence: PRECEDENCE_NEGATE,
                    number: None,
                });
                1
            }
            OP_SIN => apply_function("SIN", &mut stack)?,
            OP_COS => apply_function("COS", &mut stack)?,
            OP_TAN => apply_function("TAN", &mut stack)?,
            OP_ATANT => apply_binary_function("ATANT", &mut stack)?,
            operator => apply_binary(operator, &mut stack)?,
        };
        cursor += consumed;
    }

    match stack.as_slice() {
        [only] => Some(format!("( {} )", only.text)),
        _ => None,
    }
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

fn push_number(words: &[i32], stack: &mut Vec<Term>) -> Option<usize> {
    let [OP_PUSH_NUMBER, NUMBER_SHAPE, first, second, third, fourth, NUMBER_SHAPE, ..] = words
    else {
        return None;
    };
    // The fourth word is zero throughout the sample. Reading a formula whose
    // meaning turns on a word this has never seen used would be a guess.
    if *fourth != 0 {
        return None;
    }
    let value = decode_number(*first, *second, *third)?;
    stack.push(Term {
        text: format_number(value),
        precedence: PRECEDENCE_ATOM,
        number: Some(value),
    });
    Some(NUMBER_WORDS)
}

fn decode_number(first: i32, second: i32, third: i32) -> Option<f64> {
    if third & DOUBLE_FLAG == 0 {
        let mantissa = f64::from(first) + f64::from(second as u32) / SCALED_FRACTION;
        return (third.abs() < MAX_SCALED_EXPONENT)
            .then(|| mantissa * f64::from(third - SCALED_SHIFT).exp2());
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

fn push_attribute(words: &[i32], stack: &mut Vec<Term>) -> Option<usize> {
    if let Some(consumed) = push_owner_chain(words, stack) {
        return Some(consumed);
    }
    let [OP_ATTRIBUTE, ATTRIBUTE_SHAPE, name, first, second, third, end_first, end_second, ..] =
        words
    else {
        return None;
    };
    if [*end_first, *end_second] != ATTRIBUTE_END {
        return None;
    }
    let name = name_of(*name)?;

    let text = match ([*first, *second], third) {
        (SUBSCRIPT_FROM_STACK, 0) => {
            let subscript = stack.pop()?.number?;
            let mut text = format!("ATTRIB {name}[");
            let _ = write!(text, "{} ]", format_number(subscript));
            text
        }
        // The same qualifier words with no second name behind them: the
        // attribute stands on its own. `PTAX.Pdistance` reaches both readings
        // in one formula — `ATTRIB RADI * TAN ( ATTRIB ANGL / 2 )`.
        (SECOND_NAME_FOLLOWS, 0) => format!("ATTRIB {name}"),
        (SECOND_NAME_FOLLOWS, qualifier) => format!("ATTRIB {name} {}", name_of(*qualifier)?),
        _ => return None,
    };
    stack.push(Term::atom(text));
    Some(ATTRIBUTE_WORDS)
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
/// caller adds, which is why the text ends in one.
fn push_owner_chain(words: &[i32], stack: &mut Vec<Term>) -> Option<usize> {
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
    let name = name_of(*name)?;

    let mut text = format!("ATTRIB {name}");
    let mut named = 0;
    for word in owners {
        if *word < NAME_FLOOR {
            continue;
        }
        let _ = write!(text, " OF {}", name_of(*word)?);
        named += 1;
    }
    if named == 0 {
        return None;
    }
    text.push(' ');
    stack.push(Term::atom(text));
    Some(ATTRIBUTE_WORDS + length)
}

fn name_of(hash: i32) -> Option<String> {
    let hash = hash as u32;
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

fn apply_function(name: &str, stack: &mut Vec<Term>) -> Option<usize> {
    let argument = stack.pop()?;
    stack.push(Term::atom(format!("{name} ( {} )", argument.text)));
    Some(1)
}

/// A function of two arguments, the second pushed last.
///
/// The brackets and the comma delimit the arguments already, so neither takes
/// any of its own: E3D prints `ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] -
/// 1.8 * ATTRIB PARA[7 ] )` with the difference bare.
fn apply_binary_function(name: &str, stack: &mut Vec<Term>) -> Option<usize> {
    let second = stack.pop()?;
    let first = stack.pop()?;
    stack.push(Term::atom(format!(
        "{name} ( {} , {} )",
        first.text, second.text
    )));
    Some(1)
}

fn apply_binary(operator: i32, stack: &mut Vec<Term>) -> Option<usize> {
    let (symbol, precedence) = match operator {
        OP_ADD => ("+", PRECEDENCE_ADDITIVE),
        OP_SUBTRACT => ("-", PRECEDENCE_ADDITIVE),
        OP_MULTIPLY => ("*", PRECEDENCE_MULTIPLICATIVE),
        OP_DIVIDE => ("/", PRECEDENCE_MULTIPLICATIVE),
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
    Some(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every distinct expression blob in the 2026-08-26 TTY sample, against
    /// what E3D printed for the attribute that owns it.
    #[test]
    fn renders_sampled_expressions_the_way_e3d_does() {
        let cases: &[(&[i32], &str)] = &[
            // A bare number, in both of the shapes a number is stored in.
            (
                &[9, 1, 101, 6, 24576, 0, 5, 0, 6],
                "( 24 )",
            ),
            // The scaled shape carries its fraction in word 1. These three
            // are `PTCA.Ptcdirection` angles read back on 2026-08-27.
            (&[9, 1, 101, 6, 30719, 22720, 5, 0, 6], "( 29.999701 )"),
            (&[9, 1, 101, 6, 23039, 22720, 6, 0, 6], "( 44.999401 )"),
            (&[9, 1, 101, 6, 17920, 1664, 6, 0, 6], "( 35.000099 )"),
            (
                &[9, 1, 101, 6, 1048576, 0, 1073742851, 0, 6],
                "( 16 )",
            ),
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
                    34, 1, 101, 6, 1677721, -1717986918, 1073742845, 0, 6, 101, 6, 655360, 0,
                    1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 101, 6, 1966080,
                    0, 1073742851, 0, 6, 902, 804,
                ],
                "( 0.4 * ATTRIB PARA[13 ] * COS ( 30 ) )",
            ),
            (
                &[
                    34, 1, 101, 6, 1677721, -1717986918, 1073742845, 0, 6, 101, 6, 655360, 0,
                    1073742850, 0, 6, 106, 2, 564289, -1, -1, 0, 1601, 1701, 804, 101, 6, 1966080,
                    0, 1073742851, 0, 6, 901, 804,
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

    /// The chain's connectives are unread, so one carrying a word large enough
    /// to be a name that this cannot name is a chain it has not seen.
    #[test]
    fn declines_an_owner_chain_it_cannot_read_whole() {
        // A word above the name floor that dehashes to nothing.
        assert_eq!(
            render(&[14, 1, 106, 4, 74699205, 1, 100, 0, 1602, 4, 2, 0x7FFF_FFFF, 6, 1701]),
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
}
