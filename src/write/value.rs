//! An attribute value from its textual form — what a caller types, a JSON
//! instruction file carries, or E3D itself prints — to the [`DescriptorValue`]
//! [`set_attribute`](super::set_attribute) lays down. The descriptor decides
//! the shape; the text decides the value.
//!
//! | storage type | shape | accepted text |
//! |---|---|---|
//! | 2 real | `Real` | a number: `300`, `-3.5`, `1e3`; an angle attribute's degrees |
//! | 6 real array | `RealArray` | numbers separated by blanks or commas: `1000 2000 3000` |
//! | 3 word | `Int` / `Bool` / `Word` / angle `Real` | an integer; `true` / `false`; a word of letters, `TEXT`, which is stored as its base-27 hash |
//! | 7, 18 word array | `IntArray` / `WordArray` | integers, or words, separated by blanks or commas |
//! | 4, 16 reference | `RefNo` | `=24384/26484`, `24384/26484`, `/NAME` (looked up), `nulref` |
//! | 8, 17 reference array | `RefNoArray` | references separated by blanks or commas |
//! | 5 bit | `Bool` | `true` / `false` / `1` / `0` |
//! | 9 bit set | `BoolArray` | booleans separated by blanks or commas |
//! | 10, 15 text | `Text` | the text as it is (one layer of surrounding `'…'`, `"…"` or `\|…\|` quotes is stripped) |
//! | 19 string list | `TextList` | items separated by `\|`, or a JSON array of strings |
//! | 14 record | `RawWords` | integers separated by blanks or commas, the words as stored |
//! | any | `Unset` | `unset` (case-insensitive) |
//!
//! Nothing here consults the E3D dictionary; the limits it imposes are
//! [`super::dictionary`]'s and are checked by the caller
//! ([`super::Transaction::set`]) once the value has a shape.

use crate::record::descriptor::{descriptor_is_array, DescriptorValue, StorageShape};
use crate::record::template::TemplateAttr;
use crate::refno::RefNo;

/// Why a text could not be read as a value for an attribute. Nothing is
/// written when one of these comes back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueParseError {
    #[error("{token:?} is not {expected}")]
    Token { token: String, expected: &'static str },
    #[error("{word:?} is not a word E3D stores: a word is letters only (its base-27 hash names it back)")]
    NotAWord { word: String },
    #[error("no element is named {name:?}")]
    NameNotFound { name: String },
    #[error("looking up {name:?}: {detail}")]
    NameLookup { name: String, detail: String },
    #[error("storage type {storage_type} has no textual form")]
    NoShape { storage_type: u32 },
    #[error("an empty text is not a value; write `unset` to unset")]
    Empty,
}

/// Resolve a `/NAME` reference to its element: `Ok(None)` when there is none.
pub type NameResolver<'r> = dyn FnMut(&str) -> Result<Option<RefNo>, String> + 'r;

/// Read `input` as a value for `attr` (see the module table). `resolve` turns
/// a `/NAME` reference into its refno.
pub fn parse_value(attr: &TemplateAttr, input: &str, resolve: &mut NameResolver<'_>) -> Result<DescriptorValue, ValueParseError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(ValueParseError::Empty);
    }
    if text.eq_ignore_ascii_case("unset") {
        return Ok(DescriptorValue::Unset);
    }
    let storage_type = attr.storage_type_code;
    let shape = StorageShape::of(storage_type).ok_or(ValueParseError::NoShape { storage_type })?;
    let is_array = descriptor_is_array(attr);

    Ok(match shape {
        StorageShape::Real if !is_array => DescriptorValue::Real(real(text)?),
        StorageShape::Real => DescriptorValue::RealArray(items(text).map(real).collect::<Result<_, _>>()?),
        StorageShape::Word if !is_array => {
            if e3d_attlib::is_angle_attr(attr.hash) {
                return Ok(DescriptorValue::Real(real(text)?));
            }
            if let Ok(int) = text.parse::<i32>() {
                DescriptorValue::Int(int)
            } else if let Some(set) = boolean(text) {
                DescriptorValue::Bool(set)
            } else {
                let raw = word(text)?;
                DescriptorValue::Word { raw, text: Some(text.to_ascii_uppercase()) }
            }
        }
        StorageShape::Word => {
            let tokens: Vec<&str> = items(text).collect();
            let ints: Option<Vec<i32>> = tokens.iter().map(|token| token.parse::<i32>().ok()).collect();
            match ints {
                // A type-18 set reaches the read side as raw words; give it back so.
                Some(ints) if storage_type == 18 => DescriptorValue::RawWords(ints.into_iter().map(|int| int as u32).collect()),
                Some(ints) => DescriptorValue::IntArray(ints),
                None => DescriptorValue::WordArray(tokens.into_iter().map(word).collect::<Result<_, _>>()?),
            }
        }
        StorageShape::Reference if !is_array => DescriptorValue::RefNo(reference(text, resolve)?),
        StorageShape::Reference => {
            let mut refnos = Vec::new();
            for token in items(text) {
                refnos.push(reference(token, resolve)?);
            }
            DescriptorValue::RefNoArray(refnos)
        }
        StorageShape::PackedBit => DescriptorValue::Bool(boolean(text).ok_or_else(|| token(text, "a boolean (true / false / 1 / 0)"))?),
        StorageShape::BitSet => DescriptorValue::BoolArray(
            items(text)
                .map(|item| boolean(item).ok_or_else(|| token(item, "a boolean (true / false / 1 / 0)")))
                .collect::<Result<_, _>>()?,
        ),
        StorageShape::Characters => DescriptorValue::Text(unquote(text).to_string()),
        StorageShape::TextList => DescriptorValue::TextList(text_list(text)?),
        StorageShape::Record => DescriptorValue::RawWords(
            items(text)
                .map(|item| {
                    item.parse::<u32>()
                        .or_else(|_| item.parse::<i32>().map(|int| int as u32))
                        .map_err(|_| token(item, "an integer"))
                })
                .collect::<Result<_, _>>()?,
        ),
    })
}

/// Blank- or comma-separated items.
fn items(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || c == ',').filter(|item| !item.is_empty())
}

fn token(token: &str, expected: &'static str) -> ValueParseError {
    ValueParseError::Token { token: token.to_string(), expected }
}

/// A number; a trailing `mm` / `deg` unit E3D prints is tolerated.
fn real(text: &str) -> Result<f64, ValueParseError> {
    let bare = text
        .strip_suffix("mm")
        .or_else(|| text.strip_suffix("deg"))
        .unwrap_or(text)
        .trim();
    bare.parse::<f64>().filter_finite().map_err(|_| token(text, "a number"))
}

trait FilterFinite {
    fn filter_finite(self) -> Result<f64, ()>;
}

impl FilterFinite for Result<f64, std::num::ParseFloatError> {
    fn filter_finite(self) -> Result<f64, ()> {
        match self {
            Ok(value) if value.is_finite() => Ok(value),
            _ => Err(()),
        }
    }
}

fn boolean(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "1" | "on" => Some(true),
        "false" | "f" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// A word E3D stores as its base-27 hash: letters only, and the hash must
/// name the word back (`word_name`), so that what is written reads back as
/// what was typed.
fn word(text: &str) -> Result<u32, ValueParseError> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ValueParseError::NotAWord { word: text.to_string() });
    }
    let upper = text.to_ascii_uppercase();
    let raw = e3d_attlib::db1_hash(&upper);
    match crate::record::descriptor::word_name(raw) {
        Some(name) if name == upper => Ok(raw),
        _ => Err(ValueParseError::NotAWord { word: text.to_string() }),
    }
}

/// `=w0/w1`, `w0/w1`, `nulref` / `=0/0`, or `/NAME` through the resolver.
fn reference(text: &str, resolve: &mut NameResolver<'_>) -> Result<RefNo, ValueParseError> {
    if text.eq_ignore_ascii_case("nulref") {
        return Ok(RefNo::new(0, 0));
    }
    if let Some(name) = text.strip_prefix('/') {
        return match resolve(&format!("/{name}")) {
            Ok(Some(refno)) => Ok(refno),
            Ok(None) => Err(ValueParseError::NameNotFound { name: text.to_string() }),
            Err(detail) => Err(ValueParseError::NameLookup { name: text.to_string(), detail }),
        };
    }
    let body = text.strip_prefix('=').unwrap_or(text);
    let Some((word0, word1)) = body.split_once('/') else {
        return Err(token(text, "a reference (=word0/word1 or /NAME)"));
    };
    match (word0.trim().parse::<u32>(), word1.trim().parse::<u32>()) {
        (Ok(word0), Ok(word1)) => Ok(RefNo::new(word0, word1)),
        _ => Err(token(text, "a reference (=word0/word1 or /NAME)")),
    }
}

/// Text with one layer of surrounding `'…'`, `"…"` or `|…|` removed.
fn unquote(text: &str) -> &str {
    for (open, close) in [('\'', '\''), ('"', '"'), ('|', '|')] {
        if text.len() >= 2 && text.starts_with(open) && text.ends_with(close) {
            return &text[1..text.len() - 1];
        }
    }
    text
}

/// A JSON array of strings, or `|`-separated items.
fn text_list(text: &str) -> Result<Vec<String>, ValueParseError> {
    if text.starts_with('[') {
        return serde_json::from_str::<Vec<String>>(text).map_err(|_| token(text, "a JSON array of strings"));
    }
    Ok(text.split('|').map(|item| item.trim().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::{EncodedLocation, StorageDescriptor};
    use e3d_attlib::db1_hash;

    fn attr(name: &str, storage_type_code: u32, logical_count: u32) -> TemplateAttr {
        let storage = StorageDescriptor { storage_words: 4, location: EncodedLocation { raw: 12 }, flags: 0 };
        TemplateAttr {
            hash: db1_hash(name),
            stride: 10,
            aux: storage_type_code,
            storage_type_code,
            logical_count,
            compact: storage,
            wide: storage,
            tail: Vec::new(),
            authoritative: true,
        }
    }

    fn parse(attr: &TemplateAttr, text: &str) -> Result<DescriptorValue, ValueParseError> {
        parse_value(attr, text, &mut |name: &str| {
            Ok(match name {
                "/PUMP-1" => Some(RefNo::new(24_384, 100)),
                "/BOOM" => return Err("table unreadable".to_string()),
                _ => None,
            })
        })
    }

    #[test]
    fn every_shape_reads_its_text() {
        assert_eq!(parse(&attr("XLEN", 2, 1), " 300 "), Ok(DescriptorValue::Real(300.0)));
        assert_eq!(parse(&attr("XLEN", 2, 1), "300mm"), Ok(DescriptorValue::Real(300.0)));
        assert_eq!(parse(&attr("XLEN", 2, 1), "-1.5e2"), Ok(DescriptorValue::Real(-150.0)));
        assert_eq!(
            parse(&attr("POS", 2, 3), "1000, 2000 3000"),
            Ok(DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0]))
        );
        assert_eq!(parse(&attr("DESP", 6, 100), "1.25 2.5"), Ok(DescriptorValue::RealArray(vec![1.25, 2.5])));

        assert_eq!(parse(&attr("NUMB", 3, 1), "-7"), Ok(DescriptorValue::Int(-7)));
        assert_eq!(parse(&attr("LOCK", 3, 1), "TRUE"), Ok(DescriptorValue::Bool(true)));
        assert_eq!(parse(&attr("LOCK", 3, 1), "0"), Ok(DescriptorValue::Int(0)), "a digit is an integer, which a bool slot stores as 0 anyway");
        assert_eq!(
            parse(&attr("DTYP", 3, 1), "text"),
            Ok(DescriptorValue::Word { raw: db1_hash("TEXT"), text: Some("TEXT".into()) }),
            "a word is stored as its base-27 hash, upper-cased"
        );
        assert!(e3d_attlib::is_angle_attr(db1_hash("BANG")));
        assert_eq!(parse(&attr("BANG", 3, 1), "-90"), Ok(DescriptorValue::Real(-90.0)), "an angle word takes degrees");
        assert_eq!(parse(&attr("BANG", 3, 1), "11.5deg"), Ok(DescriptorValue::Real(11.5)));

        assert_eq!(parse(&attr("UUIDIA", 7, 4), "11 12 13 14"), Ok(DescriptorValue::IntArray(vec![11, 12, 13, 14])));
        assert_eq!(
            parse(&attr("PARA", 7, 4), "TEXT REAL"),
            Ok(DescriptorValue::WordArray(vec![db1_hash("TEXT"), db1_hash("REAL")]))
        );
        assert_eq!(parse(&attr("TYPEX", 14, 1), "1 4294967295"), Ok(DescriptorValue::RawWords(vec![1, u32::MAX])));
        assert_eq!(parse(&attr("SETX", 18, 3), "1 2"), Ok(DescriptorValue::RawWords(vec![1, 2])));

        let target = RefNo::new(24_384, 26_484);
        assert_eq!(parse(&attr("CREF", 4, 1), "=24384/26484"), Ok(DescriptorValue::RefNo(target)));
        assert_eq!(parse(&attr("CREF", 4, 1), "24384/26484"), Ok(DescriptorValue::RefNo(target)));
        assert_eq!(parse(&attr("CREF", 4, 1), "Nulref"), Ok(DescriptorValue::RefNo(RefNo::new(0, 0))));
        assert_eq!(parse(&attr("CREF", 4, 1), "/PUMP-1"), Ok(DescriptorValue::RefNo(RefNo::new(24_384, 100))));
        assert_eq!(
            parse(&attr("HREF", 8, 2), "/PUMP-1, =1/2"),
            Ok(DescriptorValue::RefNoArray(vec![RefNo::new(24_384, 100), RefNo::new(1, 2)]))
        );
        assert_eq!(parse(&attr("SPRE", 16, 1), "=13244/108564"), Ok(DescriptorValue::RefNo(RefNo::new(13_244, 108_564))));

        assert_eq!(parse(&attr("INVI", 5, 1), "yes"), Ok(DescriptorValue::Bool(true)));
        assert_eq!(parse(&attr("DEPTYP", 9, 40), "1 0 true false"), Ok(DescriptorValue::BoolArray(vec![true, false, true, false])));

        assert_eq!(parse(&attr("DESC", 10, 480), "反应堆厂房 -3.320m~±0.00m"), Ok(DescriptorValue::Text("反应堆厂房 -3.320m~±0.00m".into())));
        assert_eq!(parse(&attr("DESC", 10, 480), "'quoted text'"), Ok(DescriptorValue::Text("quoted text".into())));
        assert_eq!(parse(&attr("DESC", 10, 480), "|pml text|"), Ok(DescriptorValue::Text("pml text".into())));
        assert_eq!(parse(&attr("NAME", 15, 200), "/E3DIO-1"), Ok(DescriptorValue::Text("/E3DIO-1".into())));

        assert_eq!(parse(&attr("ROKEYS", 19, 500_000), "K1|K2| K3 "), Ok(DescriptorValue::TextList(vec!["K1".into(), "K2".into(), "K3".into()])));
        assert_eq!(parse(&attr("ROKEYS", 19, 500_000), r#"["a|b", "c"]"#), Ok(DescriptorValue::TextList(vec!["a|b".into(), "c".into()])));

        assert_eq!(parse(&attr("XLEN", 2, 1), "UNSET"), Ok(DescriptorValue::Unset));
        assert_eq!(parse(&attr("DESC", 10, 480), "unset"), Ok(DescriptorValue::Unset));
    }

    /// A value's `Display` is text this module reads back into the same value
    /// for the attribute it belongs to.
    #[test]
    fn a_value_printed_reads_back_as_itself() {
        let on = db1_hash("ON");
        let cases: Vec<(TemplateAttr, DescriptorValue)> = vec![
            (attr("XLEN", 2, 1), DescriptorValue::Real(300.0)),
            (attr("XLEN", 2, 1), DescriptorValue::Real(-3.5)),
            (attr("XLEN", 2, 1), DescriptorValue::Real(1e-7)),
            (attr("XLEN", 2, 1), DescriptorValue::Real(0.1 + 0.2)),
            (attr("XLEN", 2, 1), DescriptorValue::Real(1e300)),
            (attr("POS", 2, 3), DescriptorValue::RealArray(vec![1000.0, 2000.5, -3000.25])),
            (attr("NUMB", 3, 1), DescriptorValue::Int(-7)),
            (attr("LOCK", 3, 1), DescriptorValue::Bool(true)),
            (attr("DTYP", 3, 1), DescriptorValue::Word { raw: db1_hash("TEXT"), text: Some("TEXT".into()) }),
            (attr("BANG", 3, 1), DescriptorValue::Real(-90.0)),
            (attr("BANG", 3, 1), DescriptorValue::Real(0.29)),
            (attr("UUIDIA", 7, 4), DescriptorValue::IntArray(vec![11, -12, 13])),
            (attr("PARA", 7, 4), DescriptorValue::WordArray(vec![db1_hash("TEXT"), db1_hash("REAL"), on])),
            (attr("TYPEX", 14, 1), DescriptorValue::RawWords(vec![1, u32::MAX])),
            (attr("SETX", 18, 3), DescriptorValue::RawWords(vec![1, 2])),
            (attr("CREF", 4, 1), DescriptorValue::RefNo(RefNo::new(24_384, 26_484))),
            (attr("CREF", 4, 1), DescriptorValue::RefNo(RefNo::new(0, 0))),
            (attr("HREF", 8, 2), DescriptorValue::RefNoArray(vec![RefNo::new(24_384, 100), RefNo::new(0, 0)])),
            (attr("SPRE", 16, 1), DescriptorValue::RefNo(RefNo::new(13_244, 108_564))),
            (attr("INVI", 5, 1), DescriptorValue::Bool(false)),
            (attr("DEPTYP", 9, 40), DescriptorValue::BoolArray(vec![true, false, true])),
            (attr("DESC", 10, 480), DescriptorValue::Text("反应堆厂房 -3.320m~±0.00m".into())),
            (attr("DESC", 10, 480), DescriptorValue::Text(String::new())),
            (attr("DESC", 10, 480), DescriptorValue::Text("  padded ".into())),
            (attr("DESC", 10, 480), DescriptorValue::Text("UNSET".into())),
            (attr("DESC", 10, 480), DescriptorValue::Text("'quoted'".into())),
            (attr("DESC", 10, 480), DescriptorValue::Text("|pml|".into())),
            (attr("DESC", 10, 480), DescriptorValue::Text("'".into())),
            (attr("NAME", 15, 200), DescriptorValue::Text("/E3DIO-1".into())),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec!["K1".into(), "K2".into()])),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec!["".into(), "b".into(), "".into()])),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec!["a|b".into(), " c".into()])),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(Vec::new())),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec![String::new()])),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec!["[x".into()])),
            (attr("ROKEYS", 19, 500_000), DescriptorValue::TextList(vec!["unset".into()])),
            (attr("XLEN", 2, 1), DescriptorValue::Unset),
        ];
        for (attr, value) in cases {
            let text = value.to_string();
            assert_eq!(parse(&attr, &text), Ok(value.clone()), "{value:?} printed as {text:?}");
        }

        // What the texts look like.
        assert_eq!(DescriptorValue::Real(300.0).to_string(), "300");
        assert_eq!(DescriptorValue::Real(1e-7).to_string(), "1e-7");
        assert_eq!(DescriptorValue::RealArray(vec![1.0, 2.5]).to_string(), "1 2.5");
        assert_eq!(DescriptorValue::RefNo(RefNo::new(0, 0)).to_string(), "nulref");
        assert_eq!(DescriptorValue::RefNo(RefNo::new(24_384, 26_484)).to_string(), "=24384/26484");
        assert_eq!(DescriptorValue::Text(String::new()).to_string(), "''");
        assert_eq!(DescriptorValue::TextList(vec!["a|b".into()]).to_string(), r#"["a|b"]"#);
        assert_eq!(DescriptorValue::RealArray(Vec::new()).to_string(), "");

        // A lone word the parser would take for a boolean, and a word with no
        // name, print as the word's integer: the same word, read back as `Int`.
        let word_on = DescriptorValue::Word { raw: on, text: Some("ON".into()) };
        assert_eq!(parse(&attr("DTYP", 3, 1), &word_on.to_string()), Ok(DescriptorValue::Int(on as i32)));
        let nameless = DescriptorValue::Word { raw: 5, text: None };
        assert_eq!(nameless.to_string(), "5");
    }

    #[test]
    fn what_has_no_reading_is_refused() {
        assert_eq!(parse(&attr("XLEN", 2, 1), ""), Err(ValueParseError::Empty));
        assert_eq!(parse(&attr("XLEN", 2, 1), "x"), Err(ValueParseError::Token { token: "x".into(), expected: "a number" }));
        assert_eq!(parse(&attr("XLEN", 2, 1), "inf"), Err(ValueParseError::Token { token: "inf".into(), expected: "a number" }));
        assert_eq!(parse(&attr("POS", 2, 3), "1 two 3"), Err(ValueParseError::Token { token: "two".into(), expected: "a number" }));
        assert_eq!(parse(&attr("DTYP", 3, 1), "TE-XT"), Err(ValueParseError::NotAWord { word: "TE-XT".into() }));
        assert_eq!(parse(&attr("DTYP", 3, 1), "P1"), Err(ValueParseError::NotAWord { word: "P1".into() }));
        assert_eq!(
            parse(&attr("CREF", 4, 1), "24384"),
            Err(ValueParseError::Token { token: "24384".into(), expected: "a reference (=word0/word1 or /NAME)" })
        );
        assert_eq!(parse(&attr("CREF", 4, 1), "/NOBODY"), Err(ValueParseError::NameNotFound { name: "/NOBODY".into() }));
        assert_eq!(
            parse(&attr("CREF", 4, 1), "/BOOM"),
            Err(ValueParseError::NameLookup { name: "/BOOM".into(), detail: "table unreadable".into() })
        );
        assert_eq!(
            parse(&attr("INVI", 5, 1), "maybe"),
            Err(ValueParseError::Token { token: "maybe".into(), expected: "a boolean (true / false / 1 / 0)" })
        );
        assert_eq!(
            parse(&attr("ROKEYS", 19, 500_000), "[1, 2]"),
            Err(ValueParseError::Token { token: "[1, 2]".into(), expected: "a JSON array of strings" })
        );
        assert_eq!(parse(&attr("ODD", 99, 1), "1"), Err(ValueParseError::NoShape { storage_type: 99 }));
    }
}
