//! Evaluating catalogue expressions to numbers.
//!
//! [`crate::record::catalogue_expr`] and [`crate::record::catalogue_pml`] decode
//! the two forms a catalogue attribute stores an expression in, and render each
//! the way E3D's `Q` does. Building geometry needs the number, not the text: a
//! `PTCA` point that reads `0.7 TIMES PARAM 2` is 0.7 times the instanced
//! component's second design parameter, and until that resolves to a distance
//! nothing downstream can place it.
//!
//! The binding mirrors the FORTRAN readers this stands in for — core.dll
//! `exprlib/EXEV*` (0x51b37da..0x51b485a) and `exppdms/GATPAR` (0x51b228f),
//! reached through `catdblib/GATINS` (0x51b2e1e). `GATINS(instance)` fixes which
//! element `PARAM n` resolves against; `GATPAR(n)` then returns that element's
//! n-th design parameter as a double — 1-based, and out of range is an error
//! (223), not a guessed zero. A [`ParamEnv`] is that fixed instance: a consumer
//! wires it over a `DbElement`, and the evaluators ask it for the numbers the
//! decoded forms name.
//!
//! Nothing here invents a value. An operand or attribute the environment cannot
//! answer, or a function whose numeric meaning is not yet pinned against E3D,
//! makes the whole evaluation return `None` — an admitted gap, never a zero
//! wearing the shape of an answer.

use std::collections::HashMap;

pub use crate::record::catalogue_expr::ParamArray;

/// An attribute an expression reads, in the shapes the PML form names one.
///
/// `ATTRIB PARA[5]` is `{ name: "PARA", subscript: Some(5.0), .. }`;
/// `ATTRIB RPRO RADI` is `{ name: "RADI", qualifier: Some("RPRO"), .. }`; and
/// `ATTRIB NAMN OF PSPE OF PIPE` is `{ name: "NAMN", owner_chain: ["PSPE",
/// "PIPE"], .. }`. The tuple form ([`crate::record::catalogue_expr`]) names no
/// attributes, so only the PML evaluator builds these.
#[derive(Debug, Clone, PartialEq)]
pub struct AttribRef<'a> {
    pub name: &'a str,
    /// The `[i]` of `ATTRIB PARA[i]`. A number because the PML form takes it off
    /// the evaluation stack, where every value is one.
    pub subscript: Option<f64>,
    /// The second word of `ATTRIB X Y`.
    pub qualifier: Option<&'a str>,
    /// The `OF a OF b` owners, nearest first, as E3D prints them.
    pub owner_chain: &'a [&'a str],
}

impl<'a> AttribRef<'a> {
    /// A bare `ATTRIB name`.
    pub fn named(name: &'a str) -> Self {
        Self {
            name,
            subscript: None,
            qualifier: None,
            owner_chain: &[],
        }
    }

    /// `ATTRIB name[subscript]`.
    pub fn subscripted(name: &'a str, subscript: f64) -> Self {
        Self {
            name,
            subscript: Some(subscript),
            qualifier: None,
            owner_chain: &[],
        }
    }
}

/// The instanced component an expression resolves against.
///
/// Object-safe on purpose: the evaluators take `&dyn ParamEnv` so a consumer can
/// wire one implementation over a `DbElement` and another (see [`MapParamEnv`])
/// over fixtures without either evaluator being generic.
pub trait ParamEnv {
    /// `PARAM n` — the instance's n-th design parameter, 1-based (`GATPAR`).
    fn param(&self, n: i32) -> Option<f64>;
    /// `IPARAM n` — the n-th insulation parameter, 1-based.
    fn iparam(&self, n: i32) -> Option<f64>;
    /// An entry of any of the parameter arrays the tuple form can name
    /// (`PARAM` / `DESP` / `IPARAM` / `APARAM` / `ADESP` / `OPARAM` / `ODESP`,
    /// see [`ParamArray`]), 1-based.
    ///
    /// `PARAM` and `IPARAM` route to the two dedicated methods so an existing
    /// environment keeps answering them; every other array is declined until
    /// the environment says otherwise — a live element knows where its `DESP`
    /// is, a fixture does not.
    fn parameter(&self, array: ParamArray, n: i32) -> Option<f64> {
        match array {
            ParamArray::Para => self.param(n),
            ParamArray::IPar => self.iparam(n),
            _ => None,
        }
    }
    /// A design-dimension keyword, by its id (`DDRADIUS` = 3, `DDANGLE` = 4,
    /// `DDHEIGHT` = 5; others keep their number).
    fn design_dimension(&self, id: i32) -> Option<f64>;
    /// `ATTRIB …` read to a number, in any of the shapes [`AttribRef`] names.
    fn attribute_number(&self, attribute: &AttribRef<'_>) -> Option<f64>;
    /// A user-defined attribute the program names by its Dictionary key
    /// (`:HXYsize[2]` is stored as the `UKEY` the name had when the catalogue
    /// was written, not as the name), read to a number: the whole value when
    /// `subscript` is `None`, entry `i` of an array for `Some(i)`.
    ///
    /// A key nobody can name is a UDA nobody can read. The default declines,
    /// so an environment that has no Dictionary — a fixture, or a `DbSet` that
    /// was never handed one — reports the formula unresolved rather than
    /// guessing at a value; the live environment answers through the MDB's
    /// Dictionary databases (`e3d_io::db_element::DbElement::uda_by_key`).
    fn uda_number(&self, key: u32, subscript: Option<f64>) -> Option<f64> {
        let _ = (key, subscript);
        None
    }

    /// Whether `EXFAST` may keep 0 after this parameter-array miss.
    ///
    /// The default is yes (GATPAR 223: the array exists, the index does not).
    /// A live element makes one exception: a `FITT` with no owning `SCTN`
    /// answers `ODES`/`OPAR` as "no array" (202), and the fast path must
    /// decline so the primitive is skipped instead of drawn at 0.
    fn fast_path_missing_parameter_is_zero(&self, name: &str, subscript: Option<f64>) -> bool {
        let _ = (name, subscript);
        true
    }

    /// A direction-turn angle, the value `dopelib/DPEVAN` asks `EXEVRU` for
    /// in degrees (unit 35). The live element fails this for a `FITT` whose
    /// whole program is `ATTRIB RPRO <key>` and that key's `PTYP` is `DIST`.
    fn direction_angle(&self, program: &[i32]) -> Option<f64>;
}

/// Which of the two stores an expression came out of.
///
/// The forms are not interchangeable — a five-word tuple and a PML program are
/// different encodings with different readers — and a report that says only
/// "unresolved" cannot tell whether the gap is in the tuple decoder or in the
/// program one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionForm {
    /// The five-word tuple in the attribute's own slot
    /// ([`crate::record::catalogue_expr`]).
    Tuple,
    /// The reverse-Polish program in the explicit stream
    /// ([`crate::record::catalogue_pml`]).
    Pml,
}

impl ExpressionForm {
    pub fn as_str(self) -> &'static str {
        match self {
            ExpressionForm::Tuple => "tuple",
            ExpressionForm::Pml => "pml",
        }
    }
}

/// Reading one attribute as a number, and — when that fails — why.
///
/// Catalogue geometry needs distances, and an attribute holding one can be a
/// stored double, a tuple, or a program. Collapsing all three misses into a
/// bare `None` would put "this component has no such point" and "this formula
/// asks for something we cannot answer yet" in the same bucket, and those two
/// send a reader to entirely different places.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeNumber {
    /// A number in the attribute's own slot.
    Stored(f64),
    /// A number the expression worked out to.
    Evaluated { value: f64, form: ExpressionForm },
    /// Logically unset, or this record decoded no value for it.
    Unset,
    /// A value that is neither a number nor an expression.
    NotNumeric,
    /// An expression that did not resolve: the environment could not answer an
    /// operand, or the form is one whose numeric meaning is not yet pinned.
    /// `text` is what E3D would print, so a report can name the formula that
    /// stalled instead of only counting it.
    Unresolved {
        form: ExpressionForm,
        text: Option<String>,
    },
}

impl AttributeNumber {
    /// The number, if there is one. Use the variants themselves to account for
    /// the misses; this is for the call sites that only need the value.
    pub fn value(&self) -> Option<f64> {
        match self {
            AttributeNumber::Stored(value) => Some(*value),
            AttributeNumber::Evaluated { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// A short tag for accounting tables.
    pub fn kind(&self) -> &'static str {
        match self {
            AttributeNumber::Stored(_) => "stored",
            AttributeNumber::Evaluated { form, .. } => form.as_str(),
            AttributeNumber::Unset => "unset",
            AttributeNumber::NotNumeric => "not_numeric",
            AttributeNumber::Unresolved { .. } => "unresolved",
        }
    }
}

/// A [`ParamEnv`] backed by fixed tables.
///
/// Its reason to exist is fixtures — the evaluators' unit tests bind expressions
/// to numbers they choose — but a tool wanting to evaluate against a captured
/// parameter set can use it too. The real generation path implements [`ParamEnv`]
/// over a live element instead.
#[derive(Debug, Default, Clone)]
pub struct MapParamEnv {
    params: Vec<f64>,
    desps: Vec<f64>,
    iparams: Vec<f64>,
    design_dimensions: HashMap<i32, f64>,
    attributes: HashMap<String, f64>,
    /// UDA values by Dictionary key. One entry is a scalar, more is an array.
    udas: HashMap<u32, Vec<f64>>,
}

impl MapParamEnv {
    pub fn new() -> Self {
        Self::default()
    }

    /// The design parameters, in order: index 0 answers `PARAM 1`.
    pub fn with_params(mut self, params: impl IntoIterator<Item = f64>) -> Self {
        self.params = params.into_iter().collect();
        self
    }

    /// The design element's `DESP`, in order: index 0 answers `DESP 1`.
    pub fn with_desps(mut self, desps: impl IntoIterator<Item = f64>) -> Self {
        self.desps = desps.into_iter().collect();
        self
    }

    /// The insulation parameters, in order: index 0 answers `IPARAM 1`.
    pub fn with_iparams(mut self, iparams: impl IntoIterator<Item = f64>) -> Self {
        self.iparams = iparams.into_iter().collect();
        self
    }

    pub fn set_design_dimension(mut self, id: i32, value: f64) -> Self {
        self.design_dimensions.insert(id, value);
        self
    }

    /// Register an attribute under the key [`MapParamEnv::attribute_key`] builds,
    /// so `set_attribute("PARA[5]", …)` answers `ATTRIB PARA[5]`.
    pub fn set_attribute(mut self, key: impl Into<String>, value: f64) -> Self {
        self.attributes.insert(key.into(), value);
        self
    }

    /// Register a UDA by its Dictionary key: one value answers the bare
    /// reference, several answer `[i]`.
    pub fn with_uda(mut self, key: u32, values: impl IntoIterator<Item = f64>) -> Self {
        self.udas.insert(key, values.into_iter().collect());
        self
    }

    /// The canonical string an [`AttribRef`] registers under, matching E3D's
    /// spelling: `PARA[5]`, `RPRO RADI`, `NAMN OF PSPE OF PIPE`.
    pub fn attribute_key(attribute: &AttribRef<'_>) -> String {
        let mut key = attribute.name.to_string();
        if let Some(subscript) = attribute.subscript {
            if subscript.fract() == 0.0 {
                key.push_str(&format!("[{}]", subscript as i64));
            } else {
                key.push_str(&format!("[{subscript}]"));
            }
        }
        if let Some(qualifier) = attribute.qualifier {
            key.push(' ');
            key.push_str(qualifier);
        }
        for owner in attribute.owner_chain {
            key.push_str(" OF ");
            key.push_str(owner);
        }
        key
    }
}

/// A 1-based lookup into a parameter vector, out of range → `None` (`GATPAR`'s
/// 223 rather than a zero).
fn nth(values: &[f64], n: i32) -> Option<f64> {
    let index = usize::try_from(n).ok()?;
    index.checked_sub(1).and_then(|i| values.get(i)).copied()
}

impl ParamEnv for MapParamEnv {
    fn param(&self, n: i32) -> Option<f64> {
        nth(&self.params, n)
    }

    fn iparam(&self, n: i32) -> Option<f64> {
        nth(&self.iparams, n)
    }

    fn parameter(&self, array: ParamArray, n: i32) -> Option<f64> {
        match array {
            ParamArray::Para => self.param(n),
            ParamArray::Desp => nth(&self.desps, n),
            ParamArray::IPar => self.iparam(n),
            _ => None,
        }
    }

    fn design_dimension(&self, id: i32) -> Option<f64> {
        self.design_dimensions.get(&id).copied()
    }

    fn attribute_number(&self, attribute: &AttribRef<'_>) -> Option<f64> {
        self.attributes
            .get(&Self::attribute_key(attribute))
            .copied()
    }

    fn uda_number(&self, key: u32, subscript: Option<f64>) -> Option<f64> {
        let values = self.udas.get(&key)?;
        match subscript {
            None => match values.as_slice() {
                [only] => Some(*only),
                _ => None,
            },
            Some(index) if index.fract() == 0.0 => nth(values, index as i32),
            Some(_) => None,
        }
    }

    fn direction_angle(&self, program: &[i32]) -> Option<f64> {
        crate::record::catalogue_pml::evaluate_words_full(program, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_one_based_and_bounded() {
        let env = MapParamEnv::new().with_params([10.0, 20.0, 30.0]);
        assert_eq!(env.param(1), Some(10.0));
        assert_eq!(env.param(3), Some(30.0));
        assert_eq!(env.param(0), None, "PARAM 0 is not a parameter");
        assert_eq!(
            env.param(4),
            None,
            "past the end is GATPAR's 223, not a zero"
        );
        assert_eq!(env.param(-1), None);
    }

    #[test]
    fn the_attribute_key_matches_e3d_spelling() {
        assert_eq!(
            MapParamEnv::attribute_key(&AttribRef::subscripted("PARA", 5.0)),
            "PARA[5]"
        );
        assert_eq!(
            MapParamEnv::attribute_key(&AttribRef::named("RADI")),
            "RADI"
        );
        // The wire order is first-token-first: the PML form stores the local
        // attribute (`RPRO`) as the name and the attribute on what it points to
        // (`RADI`) as the qualifier, and E3D prints them in that order.
        assert_eq!(
            MapParamEnv::attribute_key(&AttribRef {
                name: "RPRO",
                subscript: None,
                qualifier: Some("RADI"),
                owner_chain: &[],
            }),
            "RPRO RADI"
        );
        assert_eq!(
            MapParamEnv::attribute_key(&AttribRef {
                name: "NAMN",
                subscript: None,
                qualifier: None,
                owner_chain: &["PSPE", "PIPE"],
            }),
            "NAMN OF PSPE OF PIPE"
        );
    }

    #[test]
    fn a_registered_attribute_answers_its_ref() {
        let env = MapParamEnv::new().set_attribute("PARA[5]", 42.0);
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted("PARA", 5.0)),
            Some(42.0)
        );
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted("PARA", 4.0)),
            None
        );
    }
}
