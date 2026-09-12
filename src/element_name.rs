//! The name E3D shows for an element that does not have one.
//!
//! Catalogue geometry is mostly nameless: a `GMSET` holds an `SCYLINDER` which
//! holds an `NSCYLINDER`, and none of the three below the set carries a `NAME`
//! record. E3D still prints something for `Name`, built from where the element
//! sits rather than from anything stored on it:
//!
//! ```text
//! LCYLINDER 1 of GMSET /MSIFBDD-G13-4
//! NSCYLINDER 1 of SCYLINDER 1 of GMSET /GRZGG00-G3-8
//! ```
//!
//! Read off the TTY sample, the rule is: the noun's catalogue long name, the
//! element's position among the owner's members **that share its noun**, then
//! the owner described the same way — recursing until an ancestor has a real
//! name. The position is among same-noun siblings and not among all of them:
//! `PTAXIS 1 of PTSET /MSIDBHP-P4-26` is the owner's first member overall but
//! also its first `PTAX`, while `PTCAR 2 of PTSET /ANRC900-P3-1` is the third
//! member overall and the second `PTCA`.

use e3d_attlib::db1_hash;

use crate::noun_catalog::NounCatalog;
use crate::record::element::ParsedElement;
use crate::record::explicit::ExplicitValue;
use crate::refno::RefNo;
use crate::ReadOnlyEngine;

/// How far up the owner chain to walk before giving up. Nesting this deep is
/// already beyond anything the sample shows; the limit is here so a cycle in a
/// damaged file cannot hang the caller.
const MAX_DEPTH: usize = 16;

/// The `NAME` an element stores, if it stores one.
pub fn stored_name(parsed: &ParsedElement) -> Option<String> {
    let name_hash = db1_hash("NAME");
    parsed
        .explicit_attributes
        .iter()
        .rev()
        .find(|attribute| attribute.hash == name_hash)
        .and_then(|attribute| match &attribute.value {
            ExplicitValue::Text(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
}

/// What E3D prints for `Name`: the stored name, or the positional description
/// it builds when there is none.
///
/// Returns `None` when the element cannot be read, or when the chain runs out
/// of owners before reaching a named one — an incomplete description would be
/// worse than admitting there is no answer.
pub fn display_name(
    engine: &mut ReadOnlyEngine,
    catalog: &NounCatalog,
    refno: RefNo,
) -> Option<String> {
    let parsed = read(engine, refno)?;
    if let Some(name) = stored_name(&parsed) {
        return Some(name);
    }
    positional(engine, catalog, refno, &parsed, 0)
}

/// How an owner appears inside a child's description: a named one contributes
/// `GMSET /MSIFBDD-G13-4`, a nameless one its own positional description.
fn as_owner(
    engine: &mut ReadOnlyEngine,
    catalog: &NounCatalog,
    refno: RefNo,
    depth: usize,
) -> Option<String> {
    let parsed = read(engine, refno)?;
    let noun = long_noun(catalog, parsed.noun_hash)?;
    match stored_name(&parsed) {
        Some(name) => Some(format!("{noun} {name}")),
        None => positional(engine, catalog, refno, &parsed, depth),
    }
}

fn positional(
    engine: &mut ReadOnlyEngine,
    catalog: &NounCatalog,
    refno: RefNo,
    parsed: &ParsedElement,
    depth: usize,
) -> Option<String> {
    if depth >= MAX_DEPTH || is_null(parsed.owner) {
        return None;
    }
    let noun = long_noun(catalog, parsed.noun_hash)?;
    let ordinal = ordinal_among_siblings(engine, refno, parsed.owner, parsed.noun_hash)?;
    let owner = as_owner(engine, catalog, parsed.owner, depth + 1)?;
    Some(format!("{noun} {ordinal} of {owner}"))
}

/// The element's 1-based position among the owner's members of the same noun.
fn ordinal_among_siblings(
    engine: &mut ReadOnlyEngine,
    refno: RefNo,
    owner: RefNo,
    noun_hash: u32,
) -> Option<usize> {
    let members = read(engine, owner)?.members;
    let mut seen = 0usize;
    for member in members {
        if read(engine, member).is_none_or(|child| child.noun_hash != noun_hash) {
            continue;
        }
        seen += 1;
        if member == refno {
            return Some(seen);
        }
    }
    None
}

fn long_noun(catalog: &NounCatalog, noun_hash: u32) -> Option<String> {
    catalog
        .noun(noun_hash)
        .map(|noun| noun.noun.clone())
        .filter(|noun| !noun.is_empty())
}

fn is_null(refno: RefNo) -> bool {
    refno.word0 == 0 && refno.word1 == 0
}

fn read(engine: &mut ReadOnlyEngine, refno: RefNo) -> Option<ParsedElement> {
    let view = engine.find_element(refno).ok()??;
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).ok()
}
