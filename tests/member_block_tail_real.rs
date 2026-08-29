//! An element whose member list arrives in more than one block.
//!
//! `parse_tail` used to take one member block and then look for explicit
//! blocks. BRAN `24383/85432` has two (8 words then 7) before its explicit
//! stream, so the search stopped on the second member block and the element
//! reported **no explicit attributes at all** — `NAME` included, and with it
//! the only UDA value it stores.
//!
//! The failure was silent in exactly the wrong way: "this element stores no
//! explicit attributes" and "the reader stopped early" produce identical
//! output. It took E3D's own `q att` and the raw bytes to tell them apart.
//!
//! PIPE `24383/73958` in the same database has a single member block and was
//! always read correctly; it is here so a future change to the block walk has
//! to keep both shapes working.

use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

mod common;

const DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const FIXTURES: &[&str] = &[DB];

/// `:ROOM_NO` in the AMS Dictionary. The record carries it as
/// `2652ab51 28000003 00000005 "NB122"` at byte 556.
const ROOM_NO_UKEY: u32 = 642_952_017;

fn parse(refno: RefNo) -> ParsedElement {
    let mut engine = ReadOnlyEngine::open(DB).expect("open the design database");
    let view = engine
        .find_element(refno)
        .expect("index lookup")
        .unwrap_or_else(|| panic!("{refno} is not in the database"));
    ParsedElement::parse(&view.raw_bytes).expect("parse the record")
}

fn text_of(parsed: &ParsedElement, hash: u32) -> Option<&str> {
    parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == hash)
        .and_then(|attribute| match &attribute.value {
            ExplicitValue::Text(value) => Some(value.as_str()),
            _ => None,
        })
}

#[test]
fn two_member_blocks_do_not_hide_the_explicit_stream() {
    if common::missing(FIXTURES) {
        return;
    }
    let parsed = parse(RefNo::new(24383, 85432));

    assert_eq!(
        parsed.explicit_attributes.len(),
        9,
        "the record stores nine explicit attributes; reading zero means the \
         walk stopped at the second member block"
    );
    assert_eq!(
        text_of(&parsed, e3d_attlib::db1_hash("NAME")),
        Some("/C-CO-5RX122-C"),
        "q att prints this name"
    );

    let room_no = parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == ROOM_NO_UKEY)
        .expect("the stored :ROOM_NO value");
    assert!(room_no.is_uda);
    assert_eq!(room_no.value, ExplicitValue::Text("NB122".into()));

    // The member list splits mid-RefNo: the first block ends with the word
    // 24383 and the second begins with 85434. Reading the blocks separately
    // yields a phantom 85434/0 and loses 24383/85434; reading only the first
    // loses it as well. `old-parse-pdms-db` reads two members here.
    assert_eq!(
        parsed.members,
        vec![RefNo::new(24383, 85433), RefNo::new(24383, 85434)]
    );
}

#[test]
fn a_single_member_block_still_reads_the_same_way() {
    if common::missing(FIXTURES) {
        return;
    }
    let parsed = parse(RefNo::new(24383, 73958));

    assert_eq!(parsed.explicit_attributes.len(), 5);
    assert_eq!(parsed.members, vec![RefNo::new(24383, 73959)]);
    assert_eq!(
        text_of(&parsed, e3d_attlib::db1_hash("NAME")),
        Some("/1RCV0102A")
    );
    assert!(
        !parsed
            .explicit_attributes
            .iter()
            .any(|attribute| attribute.is_uda),
        "this element stores no UDA; its q att lines are Dictionary defaults"
    );
}
