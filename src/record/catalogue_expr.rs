//! Catalogue parameter expressions: the five-word tuples that catalogue
//! geometry stores instead of numbers.
//!
//! A `PTCA` point does not hold a distance; it holds "0.7 times the second
//! design parameter". E3D prints that as `0.7 TIMES PARAM 2` and the database
//! stores `[4, 28, 1, 2, 0]`. Decoded straight, the reader reported the four
//! words and every such attribute counted as a mismatch against TTY — 132 of
//! them in the sampled catalogue elements.
//!
//! The encoding was recovered from those 132 pairs of tuple and E3D rendering:
//!
//! * word 0 is 4, the count of words that follow.
//! * word 1 is the coefficient in fortieths: 28 is 0.7, 40 is 1, 50 is 1.25,
//!   and negatives are two's complement, so -24 is -0.6.
//! * word 2 selects the operator.
//! * words 3 and 4 are operands, encoded as described on [`Operand`].
//!
//! An all-zero tuple is not an expression: those attributes hold a PML
//! expression that lives elsewhere in the catalogue, and E3D prints things
//! like `( 0.5 * ATTRIB PARA[20 ] )` for them. [`decode`] returns `None` there
//! rather than inventing a value.

use std::fmt;

use e3d_attlib::{db1_dehash, lookup_system_name};

use crate::record::catalogue_eval::ParamEnv;

/// Word 1 counts fortieths of a unit.
const COEFFICIENT_SCALE: f64 = 40.0;
/// The operator whose operands spell a name instead of naming a value.
const OP_NAME: u32 = 7;
/// The two words of a spelled name are its hash split by this.
const NAME_SPLIT: i32 = 1000;
/// Negative operands are constants in tenths (`-95` is 9.5), except the five
/// keyword slots just below zero — see [`DESIGN_DIMENSION_MAX`].
const CONSTANT_SCALE: i32 = 10;
/// Core3D `pplib/DECARG` (0x103C12B8) treats a negative operand whose
/// magnitude is at most this as a keyword, not a constant: 1 is 0.0, 2 is 1.0,
/// 3/4/5 read the design element's `RADI` / `ANGL` / `HEIG` (`DDRADIUS`,
/// `DDANGLE`, `DDHEIGHT`). Everything beyond is `value / -10`, whether or not
/// it is a whole ten — `-95` is 9.5, `-1875` is 187.5.
const DESIGN_DIMENSION_MAX: i32 = 5;
const IPARAM_BASE: i32 = 100;
const NEGATED_PARAM_BASE: i32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// `PARAM n` — the element's nth design parameter.
    Param(i32),
    /// `- PARAM n`, stored as 1000 + n.
    NegatedParam(i32),
    /// `IPARAM n` — an insulation parameter, stored as 100 + n.
    IParam(i32),
    /// A literal, stored negated and in tenths; the payload keeps the tenths,
    /// so `-1250` is `Constant(1250)` and reads as 125, `-95` is 9.5.
    Constant(i32),
    /// A design-dimension keyword, stored as its negated id (3 `DDRADIUS`,
    /// 4 `DDANGLE`, 5 `DDHEIGHT`, per DECARG). Other ids keep their number so
    /// a reader can tell an unrecognised keyword from a wrong one.
    DesignDimension(i32),
    Zero,
}

impl Operand {
    fn decode(raw: i32) -> Self {
        match raw {
            0 => Operand::Zero,
            n if n >= NEGATED_PARAM_BASE => Operand::NegatedParam(n - NEGATED_PARAM_BASE),
            n if n >= IPARAM_BASE => Operand::IParam(n - IPARAM_BASE),
            n if n > 0 => Operand::Param(n),
            // DECARG: `abs(n) > 5` is a constant in tenths; 1 and 2 are the
            // literal 0 and 1; 3, 4, 5 are the DD keywords.
            n if -n > DESIGN_DIMENSION_MAX => Operand::Constant(-n),
            -1 => Operand::Zero,
            -2 => Operand::Constant(CONSTANT_SCALE),
            n => Operand::DesignDimension(-n),
        }
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Param(n) => write!(f, "PARAM {n}"),
            Operand::NegatedParam(n) => write!(f, "- PARAM {n}"),
            Operand::IParam(n) => write!(f, "IPARAM {n}"),
            Operand::Constant(tenths) => {
                write!(f, "{}", format_number(f64::from(*tenths) / CONSTANT_SCALE as f64))
            }
            Operand::DesignDimension(3) => write!(f, "DDRADIUS"),
            Operand::DesignDimension(4) => write!(f, "DDANGLE"),
            Operand::DesignDimension(5) => write!(f, "DDHEIGHT"),
            Operand::DesignDimension(id) => write!(f, "DD#{id}"),
            Operand::Zero => write!(f, "0"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CatalogueExpression {
    /// `<coefficient> TIMES <operand>`, with the coefficient omitted when it
    /// is exactly one.
    Scaled { coefficient: f64, operand: Operand },
    /// A named function of two operands, carrying the same optional
    /// coefficient: `-0.55 TIMES DIFFERENCE PARAM 6 PARAM 10`.
    Function {
        coefficient: f64,
        name: &'static str,
        left: Operand,
        right: Operand,
    },
    /// The tuple spells a name rather than a value.
    Named(String),
}

/// `<coefficient> TIMES `, or nothing at all when the coefficient is one.
fn scale_prefix(coefficient: f64) -> String {
    match coefficient == 1.0 {
        true => String::new(),
        false => format!("{} TIMES ", format_number(coefficient)),
    }
}

impl fmt::Display for CatalogueExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogueExpression::Scaled {
                coefficient,
                operand,
            } => {
                if matches!(operand, Operand::Zero) {
                    write!(f, "0")
                } else {
                    // Even against a literal E3D spells the product out, as
                    // `-1 TIMES 87` rather than `-87`.
                    write!(f, "{}{operand}", scale_prefix(*coefficient))
                }
            }
            CatalogueExpression::Function {
                coefficient,
                name,
                left,
                right,
            } => write!(f, "{}{name} {left} {right}", scale_prefix(*coefficient)),
            CatalogueExpression::Named(name) => write!(f, "{name}"),
        }
    }
}

fn format_number(value: f64) -> String {
    let text = format!("{value}");
    match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    }
}

/// Decode a catalogue tuple, with or without its stored count.
///
/// The implicit reader knows an array is count-prefixed and hands over just the
/// four payload words; explicit storage and the raw-word fallback still carry
/// the count. Both forms reach here, so both are tried — payload first, since
/// stripping a count off a payload whose coefficient happens to equal three
/// would silently change what it means.
///
/// Returns `None` for anything that is not a tuple, including the all-zero one
/// of attributes whose expression is held elsewhere.
pub fn decode(words: &[u32]) -> Option<CatalogueExpression> {
    decode_payload(words).or_else(|| match words {
        [count, rest @ ..] if *count as usize == rest.len() => decode_payload(rest),
        _ => None,
    })
}

fn decode_payload(words: &[u32]) -> Option<CatalogueExpression> {
    let [coefficient, operator, first, second] = words else {
        return None;
    };
    if *coefficient == 0 && *operator == 0 && *first == 0 && *second == 0 {
        return None;
    }

    let coefficient = f64::from(*coefficient as i32) / COEFFICIENT_SCALE;
    let left = Operand::decode(*first as i32);
    let right = Operand::decode(*second as i32);

    Some(match *operator {
        0 | 1 => CatalogueExpression::Scaled {
            coefficient,
            operand: left,
        },
        OP_NAME => CatalogueExpression::Named(spelled_name(*first as i32, *second as i32)?),
        operator => CatalogueExpression::Function {
            coefficient,
            name: function_name(operator)?,
            left,
            right,
        },
    })
}

fn function_name(operator: u32) -> Option<&'static str> {
    match operator {
        2 => Some("TANF"),
        3 => Some("DIFFERENCE"),
        4 => Some("SUM"),
        5 => Some("SINF"),
        6 => Some("COSF"),
        _ => None,
    }
}

/// The name operator [`OP_NAME`] spells, out of the two operand words.
///
/// They are the attribute hash split by thousands: `776 966` is 776966, which
/// dehashes to `NULL`, and `534 980` is 534980, which is `BWD`. Reading it as
/// `NULL` whatever the words said is how the second one came to print the
/// first one's name.
fn spelled_name(first: i32, second: i32) -> Option<String> {
    if first <= 0 || !(0..NAME_SPLIT).contains(&second) {
        return None;
    }
    let hash = u32::try_from(first.checked_mul(NAME_SPLIT)?.checked_add(second)?).ok()?;
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// Render a tuple the way E3D's `Q` command does, if it is an expression.
pub fn render(words: &[u32]) -> Option<String> {
    decode(words).map(|expression| expression.to_string())
}

/// Evaluate a decoded tuple to a number, against `env`.
///
/// The coefficient multiplies the operand (or the function of the two operands):
/// `0.7 TIMES PARAM 2` is `0.7 * param(2)`, and `-0.55 TIMES DIFFERENCE PARAM 6
/// PARAM 10` is `-0.55 * (param(6) - param(10))`.
///
/// Returns `None`, never a guess, when the environment cannot answer an operand,
/// when the tuple spells a [`CatalogueExpression::Named`] name rather than a
/// value, and when the function is one whose numeric meaning is not yet pinned
/// against E3D (see [`apply_function`]).
pub fn evaluate(expression: &CatalogueExpression, env: &dyn ParamEnv) -> Option<f64> {
    match expression {
        CatalogueExpression::Scaled {
            coefficient,
            operand,
        } => Some(coefficient * operand_value(operand, env)?),
        CatalogueExpression::Function {
            coefficient,
            name,
            left,
            right,
        } => {
            let left = operand_value(left, env)?;
            let right = operand_value(right, env)?;
            Some(coefficient * apply_function(name, left, right)?)
        }
        CatalogueExpression::Named(_) => None,
    }
}

/// Decode a tuple straight from its words and evaluate it, if it is one.
pub fn evaluate_words(words: &[u32], env: &dyn ParamEnv) -> Option<f64> {
    evaluate(&decode(words)?, env)
}

fn operand_value(operand: &Operand, env: &dyn ParamEnv) -> Option<f64> {
    match operand {
        Operand::Param(n) => env.param(*n),
        Operand::NegatedParam(n) => env.param(*n).map(|value| -value),
        Operand::IParam(n) => env.iparam(*n),
        Operand::Constant(tenths) => Some(f64::from(*tenths) / CONSTANT_SCALE as f64),
        Operand::DesignDimension(id) => env.design_dimension(*id),
        Operand::Zero => Some(0.0),
    }
}

/// The two-operand functions whose numeric meaning is settled.
///
/// `SUM` and `DIFFERENCE` are addition and subtraction. The trigonometric
/// `SINF` / `COSF` / `TANF` carry a second operand whose role E3D has not yet
/// been made to show — `SINF PARAM 10 PARAM 11` could be any of several things —
/// so they return `None` until a TTY `Q` reading pins them, rather than a value
/// that only looks like an answer.
fn apply_function(name: &str, left: f64, right: f64) -> Option<f64> {
    match name {
        "SUM" => Some(left + right),
        "DIFFERENCE" => Some(left - right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::catalogue_eval::MapParamEnv;

    fn sample_env() -> MapParamEnv {
        MapParamEnv::new()
            .with_params([
                10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0,
            ])
            .with_iparams([100.0, 200.0])
            .set_design_dimension(3, 200.0) // DDRADIUS
            .set_design_dimension(4, 45.0) // DDANGLE
            .set_design_dimension(5, 500.0) // DDHEIGHT
    }

    #[track_caller]
    fn assert_close(got: Option<f64>, expected: f64) {
        let got = got.expect("expression evaluates to a number");
        assert!(
            (got - expected).abs() < 1e-9,
            "got {got}, expected {expected}"
        );
    }

    /// The same tuples the render test covers, now as numbers against a fixed
    /// parameter set. Expected values are the arithmetic the rendering spells.
    #[test]
    fn evaluates_sampled_tuples_against_a_parameter_environment() {
        let env = sample_env();
        assert_close(evaluate_words(&[4, 28, 1, 2, 0], &env), 0.7 * 20.0);
        assert_close(evaluate_words(&[4, 40, 1, 6, 0], &env), 60.0);
        assert_close(evaluate_words(&[4, 50, 1, 2, 0], &env), 1.25 * 20.0);
        assert_close(evaluate_words(&[4, 40, 1, 1005, 0], &env), -50.0);
        assert_close(evaluate_words(&[4, 40, 1, (-20i32) as u32, 0], &env), 2.0);
        assert_close(
            evaluate_words(&[4, (-40i32) as u32, 1, (-870i32) as u32, 0], &env),
            -87.0,
        );
        assert_close(
            evaluate_words(&[4, 11, 1, (-3i32) as u32, 0], &env),
            0.275 * 200.0,
        );
        assert_close(evaluate_words(&[4, 0, 1, 0, 0], &env), 0.0);
        assert_close(evaluate_words(&[4, 40, 4, 4, 101], &env), 40.0 + 100.0);
        assert_close(
            evaluate_words(&[(-22i32) as u32, 3, 6, 10], &env),
            -0.55 * (60.0 - 100.0),
        );
        assert_close(evaluate_words(&[40, 3, 6, 11], &env), 60.0 - 110.0);
    }

    /// What it must refuse to number: a spelled name, an out-of-range parameter
    /// (GATPAR's 223), an operand the environment does not carry, and the trig
    /// functions whose second operand is not yet pinned against E3D.
    #[test]
    fn declines_what_it_cannot_number() {
        let env = sample_env();
        assert_eq!(evaluate_words(&[4, 40, 7, 776, 966], &env), None); // NULL
        assert_eq!(evaluate_words(&[4, 40, 1, 12, 0], &env), None); // PARAM 12, unset
        assert_eq!(evaluate_words(&[40, 5, 10, 11], &env), None); // SINF
        assert_eq!(evaluate_words(&[40, 6, 10, 11], &env), None); // COSF
        assert_eq!(evaluate_words(&[40, 2, 2, (-4i32) as u32], &env), None); // TANF
    }

    /// The negative side of Core3D `pplib/DECARG` (0x103C12B8): magnitudes
    /// above five are constants in tenths whatever their last digit, and the
    /// five slots below are keywords. `/YKS-ABDCEB2-GS` stores its 9.5 mm
    /// handwheel stem as `PBDM = [40, 1, -95, 0]` and its 8.5 mm hub as
    /// `PDIA = [40, 1, -85, 0]`; reading those as unresolved `DD#95` / `DD#85`
    /// dropped the primitives E3D draws.
    #[test]
    fn decodes_decarg_negative_operands_as_tenths_or_keywords() {
        let env = sample_env();
        assert_close(evaluate_words(&[4, 40, 1, (-95i32) as u32, 0], &env), 9.5);
        assert_close(evaluate_words(&[4, 40, 1, (-85i32) as u32, 0], &env), 8.5);
        assert_close(evaluate_words(&[4, 40, 1, (-1875i32) as u32, 0], &env), 187.5);
        assert_close(evaluate_words(&[4, 40, 1, (-7i32) as u32, 0], &env), 0.7);
        assert_close(evaluate_words(&[4, 40, 1, (-6i32) as u32, 0], &env), 0.6);
        assert_close(evaluate_words(&[4, 40, 1, (-20i32) as u32, 0], &env), 2.0);
        assert_close(evaluate_words(&[4, 40, 1, (-1i32) as u32, 0], &env), 0.0);
        assert_close(evaluate_words(&[4, 40, 1, (-2i32) as u32, 0], &env), 1.0);
        assert_close(evaluate_words(&[4, 40, 1, (-3i32) as u32, 0], &env), 200.0); // DDRADIUS
        assert_close(evaluate_words(&[4, 40, 1, (-4i32) as u32, 0], &env), 45.0); // DDANGLE
        assert_close(evaluate_words(&[4, 40, 1, (-5i32) as u32, 0], &env), 500.0); // DDHEIGHT
        assert_eq!(Operand::decode(-95), Operand::Constant(95));
        assert_eq!(Operand::decode(-1), Operand::Zero);
        assert_eq!(Operand::decode(-2), Operand::Constant(10));
        assert_eq!(Operand::decode(-5), Operand::DesignDimension(5));
        assert_eq!(render(&[40, 1, (-95i32) as u32, 0]).as_deref(), Some("9.5"));
        assert_eq!(render(&[40, 1, (-870i32) as u32, 0]).as_deref(), Some("87"));
        assert_eq!(render(&[40, 1, (-4i32) as u32, 0]).as_deref(), Some("DDANGLE"));
    }

    /// Every distinct tuple the TTY sample produced, against E3D's own text.
    #[test]
    fn renders_sampled_tuples_the_way_e3d_does() {
        let cases: &[(&[u32], &str)] = &[
            // coefficient in fortieths
            (&[4, 8, 1, 7, 0], "0.2 TIMES PARAM 7"),
            (&[4, 12, 1, 5, 0], "0.3 TIMES PARAM 5"),
            (&[4, 16, 1, 2, 0], "0.4 TIMES PARAM 2"),
            (&[4, 20, 1, 4, 0], "0.5 TIMES PARAM 4"),
            (&[4, 28, 1, 2, 0], "0.7 TIMES PARAM 2"),
            (&[4, 32, 1, 4, 0], "0.8 TIMES PARAM 4"),
            (&[4, 40, 1, 6, 0], "PARAM 6"),
            (&[4, 50, 1, 2, 0], "1.25 TIMES PARAM 2"),
            (&[4, 80, 1, 2, 0], "2 TIMES PARAM 2"),
            // negative coefficients are two's complement
            (&[4, (-8i32) as u32, 1, 7, 0], "-0.2 TIMES PARAM 7"),
            (&[4, (-20i32) as u32, 1, 5, 0], "-0.5 TIMES PARAM 5"),
            (&[4, (-24i32) as u32, 1, 8, 0], "-0.6 TIMES PARAM 8"),
            // literals are stored negated, in tenths
            (&[4, 40, 1, (-20i32) as u32, 0], "2"),
            (&[4, 40, 1, (-50i32) as u32, 0], "5"),
            (&[4, 40, 1, (-250i32) as u32, 0], "25"),
            (&[4, 40, 1, (-920i32) as u32, 0], "92"),
            (&[4, 40, 1, (-1250i32) as u32, 0], "125"),
            (&[4, 40, 1, (-3600i32) as u32, 0], "360"),
            // a literal keeps its coefficient spelled out
            (&[4, (-40i32) as u32, 1, (-870i32) as u32, 0], "-1 TIMES 87"),
            // negative operands that are not whole tens name a design dimension
            (&[4, 11, 1, (-3i32) as u32, 0], "0.275 TIMES DDRADIUS"),
            (&[4, 29, 1, (-3i32) as u32, 0], "0.725 TIMES DDRADIUS"),
            (&[4, 40, 1, (-5i32) as u32, 0], "DDHEIGHT"),
            // 1000 + n negates a parameter
            (&[4, 40, 1, 1005, 0], "- PARAM 5"),
            (&[4, 40, 1, 1002, 0], "- PARAM 2"),
            // two-operand forms
            (&[4, 40, 3, 4, (-80i32) as u32], "DIFFERENCE PARAM 4 8"),
            (&[4, 40, 3, 5, 2], "DIFFERENCE PARAM 5 PARAM 2"),
            (&[4, 40, 4, 4, 101], "SUM PARAM 4 IPARAM 1"),
            (&[4, 40, 4, 101, 6], "SUM IPARAM 1 PARAM 6"),
            (&[4, 40, 7, 776, 966], "NULL"),
            (&[4, 0, 1, 0, 0], "0"),
            // The 2026-08-27 census: three more functions, a coefficient in
            // front of a two-operand one, and a second spelled name.
            (&[40, 5, 10, 11], "SINF PARAM 10 PARAM 11"),
            (
                &[(-40i32) as u32, 5, 10, 11],
                "-1 TIMES SINF PARAM 10 PARAM 11",
            ),
            (
                &[(-40i32) as u32, 6, 10, 11],
                "-1 TIMES COSF PARAM 10 PARAM 11",
            ),
            (&[40, 2, 2, (-4i32) as u32], "TANF PARAM 2 DDANGLE"),
            (
                &[(-22i32) as u32, 3, 6, 10],
                "-0.55 TIMES DIFFERENCE PARAM 6 PARAM 10",
            ),
            (&[40, 3, 6, 11], "DIFFERENCE PARAM 6 PARAM 11"),
            (&[40, 7, 534, 980], "BWD"),
        ];
        for (words, expected) in cases {
            assert_eq!(
                render(words).as_deref(),
                Some(*expected),
                "tuple {words:?} should render as {expected}"
            );
        }
    }

    /// The attributes whose expression is a PML formula held elsewhere store
    /// nothing here, and E3D prints `( 0.5 * ATTRIB PARA[20 ] )` for them.
    /// Guessing a value would be worse than admitting there is none.
    #[test]
    fn declines_the_all_zero_tuple() {
        assert_eq!(decode(&[4, 0, 0, 0, 0]), None);
    }

    /// The implicit reader strips the count before handing an array over, so
    /// the same expression arrives as four words.
    #[test]
    fn reads_the_payload_without_its_count() {
        assert_eq!(render(&[28, 1, 2, 0]).as_deref(), Some("0.7 TIMES PARAM 2"));
        assert_eq!(render(&[40, 1, 6, 0]).as_deref(), Some("PARAM 6"));
        assert_eq!(
            render(&[(-24i32) as u32, 1, 8, 0]).as_deref(),
            Some("-0.6 TIMES PARAM 8")
        );
        assert_eq!(decode(&[0, 0, 0, 0]), None);
    }

    #[test]
    fn declines_word_runs_that_are_not_tuples() {
        assert_eq!(decode(&[4, 40, 1, 2]), None);
        assert_eq!(decode(&[3, 40, 1, 2, 0]), None);
        // An operator with no name behind it.
        assert_eq!(decode(&[4, 40, 8, 5, 2]), None);
        // Words that spell nothing are not a name, whatever the operator says.
        assert_eq!(decode(&[4, 40, 7, 1, 2]), None);
        assert_eq!(decode(&[4, 40, 7, 776, 1966]), None);
        assert_eq!(decode(&[4, 40, 7, (-776i32) as u32, 966]), None);
    }
}
