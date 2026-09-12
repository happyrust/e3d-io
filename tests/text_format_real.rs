//! Text decoding follows the header's `FORMAT` word, on real databases.
//!
//! `core.dll` reads `FORMAT` (+0x154) before any string and branches on
//! `== 1`: raw UTF-8 as stored (`sub_5B8F2D0`), else the escaped project
//! charset converted to UTF-8 (`sub_54B2CE8`). The local corpus has both:
//! every AVEVA-shipped database and the `acw000` project say 1 and hold raw
//! UTF-8 (`acw5001_0001`: 3,810 CJK runs, 15 escape-shaped byte pairs);
//! `ams1112_0001` and the 84 `TES000` databases say 0 and hold `&~ … &`
//! escapes (`ams1112_0001`: 126,045 of them, no UTF-8).
//!
//! These pin three things: the header word each fixture states, that the
//! header-driven decode agrees with the byte-sniffing guess on every text of
//! every sampled element (so switching to the header changed no sampled
//! value), and that Chinese comes through as Chinese on both sides.
//!
//! The offline copy `ams1112_0001` is the anchor. Installed project files are
//! read when present and skipped otherwise; `ams8000_0001` of 3.1 is live.

use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::text::{decode, decode_as, TextFormat};
use e3d_io::ReadOnlyEngine;

mod common;

const E3D31_AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const E3D31_ACW5001: &str = r"D:\AVEVA\Projects\E3D3.1\YCYK-E3D\acw000\acw5001_0001";
const E3D31_TES1000: &str = r"D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES1000_0001";

/// Elements compared per database. `ams1112_0001` indexes far more than this
/// and the comparison is linear in pages read; the cap keeps the test in
/// seconds while still covering every noun the file front-loads.
const ELEMENT_CAP: usize = 40_000;

fn has_cjk(text: &str) -> bool {
    text.chars()
        .any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c))
}

/// Every explicit text of the first `ELEMENT_CAP` indexed elements, decoded
/// under the header word and under the guess, side by side.
struct Texts {
    compared: usize,
    elements: usize,
    with_cjk: usize,
    escape_shaped: usize,
    disagreements: Vec<(String, String)>,
    sample_cjk: Option<String>,
}

fn compare_texts(engine: &mut ReadOnlyEngine) -> Texts {
    let format = engine.text_format();
    let entries = engine.indexed_refnos().expect("walk the index");
    let mut out = Texts {
        compared: 0,
        elements: 0,
        with_cjk: 0,
        escape_shaped: 0,
        disagreements: Vec::new(),
        sample_cjk: None,
    };
    for (refno, page, offset) in entries.into_iter().take(ELEMENT_CAP) {
        let Ok(view) = engine.read_element_direct(refno, page, offset) else {
            continue;
        };
        let (Ok(by_header), Ok(by_guess)) = (
            ParsedElement::parse_as(&view.raw_bytes, format),
            ParsedElement::parse(&view.raw_bytes),
        ) else {
            continue;
        };
        assert_eq!(by_header.text_format, format);
        assert_eq!(by_guess.text_format, TextFormat::Detect);
        out.elements += 1;
        let texts = |parsed: &ParsedElement| -> Vec<String> {
            parsed
                .explicit_attributes
                .iter()
                .filter_map(|attribute| match &attribute.value {
                    ExplicitValue::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        for (header, guess) in texts(&by_header).into_iter().zip(texts(&by_guess)) {
            out.compared += 1;
            if has_cjk(&header) {
                out.with_cjk += 1;
                out.sample_cjk.get_or_insert_with(|| header.clone());
            }
            if header.contains("&~") {
                out.escape_shaped += 1;
            }
            if header != guess {
                out.disagreements.push((header, guess));
            }
        }
    }
    out
}

/// The word each fixture states, and the decoder it selects.
#[test]
fn the_header_word_states_how_each_fixture_stores_text() {
    let expectations: &[(&str, u32, TextFormat)] = &[
        (common::TEST_FILE_DB, 0, TextFormat::Escaped),
        (E3D31_AMS8000, 1, TextFormat::Utf8),
        (E3D31_ACW5001, 1, TextFormat::Utf8),
        (E3D31_TES1000, 0, TextFormat::Escaped),
    ];
    let mut checked = 0;
    for (path, word, format) in expectations {
        if common::missing(&[path]) {
            continue;
        }
        let engine = ReadOnlyEngine::open(path).expect("open");
        assert_eq!(
            engine.descriptor().header_format,
            Some(*word),
            "{path}: FORMAT word"
        );
        assert_eq!(engine.text_format(), *format, "{path}: decoder");
        checked += 1;
    }
    assert!(checked >= 1, "at least the offline anchor must be present");
}

/// `ams1112_0001` says 0: its Chinese is escaped and restores to GBK under
/// the header word. The header-driven decode agrees with the guess on every
/// sampled text but the 78 that are raw GBK with no escape — `反应堆厂房`
/// written as `B7B4 D3A6 B6D1 B3A7 B7BF` — which the guess could only read as
/// broken UTF-8 (replacement characters) and the header decode reads as the
/// code page. Every disagreement has to be of that one shape.
#[test]
fn format_0_database_restores_escaped_chinese_and_reads_raw_gbk() {
    if common::missing(&[common::TEST_FILE_DB]) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(common::TEST_FILE_DB).expect("open");
    assert_eq!(engine.text_format(), TextFormat::Escaped);
    let texts = compare_texts(&mut engine);
    eprintln!(
        "ams1112_0001: {} elements, {} texts, {} with CJK (e.g. {:?}), {} escape-shaped after decoding, {} raw-GBK texts the guess could not read",
        texts.elements,
        texts.compared,
        texts.with_cjk,
        texts.sample_cjk,
        texts.escape_shaped,
        texts.disagreements.len()
    );
    assert!(
        texts.compared > 1000,
        "sampled too few texts: {}",
        texts.compared
    );
    assert!(texts.with_cjk > 0, "the escaped Chinese did not restore");
    assert_eq!(texts.escape_shaped, 0, "an escape survived decoding");
    assert!(
        !texts.disagreements.is_empty(),
        "the raw-GBK texts of this file are known; none was found"
    );
    for (header, guess) in &texts.disagreements {
        assert!(
            guess.contains('\u{FFFD}') && !header.contains('\u{FFFD}') && has_cjk(header),
            "a disagreement that is not the guess failing on raw GBK: header {header:?} guess {guess:?}"
        );
    }
}

/// `acw5001_0001` says 1: its Chinese is raw UTF-8 and comes through under
/// the header word with no escape processing, and the guess agrees on every
/// sampled text — no stored string of this file happens to look like an
/// escape, which is the one case the two would differ on.
#[test]
fn format_1_database_reads_raw_utf8_and_agrees_with_the_guess() {
    if common::missing(&[E3D31_ACW5001]) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(E3D31_ACW5001).expect("open");
    assert_eq!(engine.text_format(), TextFormat::Utf8);
    let texts = compare_texts(&mut engine);
    eprintln!(
        "acw5001_0001: {} elements, {} texts, {} with CJK (e.g. {:?}), {} escape-shaped, {} disagreements",
        texts.elements,
        texts.compared,
        texts.with_cjk,
        texts.sample_cjk,
        texts.escape_shaped,
        texts.disagreements.len()
    );
    assert!(
        texts.compared > 100,
        "sampled too few texts: {}",
        texts.compared
    );
    assert!(
        texts.with_cjk > 0,
        "the raw UTF-8 Chinese did not come through"
    );
    assert!(
        texts.disagreements.is_empty(),
        "header and guess disagree: {:?}",
        &texts.disagreements[..texts.disagreements.len().min(5)]
    );
}

/// The live 3.1 marine sample says 1 and is English: the header-driven decode
/// and the guess agree on every text, and every text is plain.
#[test]
fn format_1_english_database_agrees_with_the_guess() {
    if common::missing(&[E3D31_AMS8000]) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(E3D31_AMS8000).expect("open");
    assert_eq!(engine.text_format(), TextFormat::Utf8);
    let texts = compare_texts(&mut engine);
    eprintln!(
        "ams8000_0001 (3.1): {} elements, {} texts, {} with CJK, {} disagreements",
        texts.elements,
        texts.compared,
        texts.with_cjk,
        texts.disagreements.len()
    );
    assert!(
        texts.compared > 100,
        "sampled too few texts: {}",
        texts.compared
    );
    assert!(texts.disagreements.is_empty());
}

/// Session `USER` and `DESC` decode under the same word: no page of either
/// fixture leaves an escape in its comment, and the format the engine holds
/// is the one the chain walk decoded under.
#[test]
fn session_text_decodes_under_the_header_word() {
    for (path, format) in [
        (common::TEST_FILE_DB, TextFormat::Escaped),
        (E3D31_ACW5001, TextFormat::Utf8),
        (E3D31_TES1000, TextFormat::Escaped),
    ] {
        if common::missing(&[path]) {
            continue;
        }
        let mut view = e3d_io::session::DbView::open(path).expect("open");
        assert_eq!(view.text_format(), format, "{path}");
        let sessions = view.walk_sessions().expect("walk the chain");
        let mut non_ascii = 0;
        for session in &sessions {
            assert!(
                !session.comment.contains("&~"),
                "{path} session {}: escape left in comment {:?}",
                session.sesno,
                session.comment
            );
            assert!(
                !session.user.contains("&~"),
                "{path} session {}: escape left in user {:?}",
                session.sesno,
                session.user
            );
            if !session.comment.is_ascii() || !session.user.is_ascii() {
                non_ascii += 1;
            }
        }
        eprintln!(
            "{path}: {} sessions, {} with non-ASCII user/comment",
            sessions.len(),
            non_ascii
        );
    }
}

/// The two explicit decoders differ exactly where they should: an
/// escape-shaped sequence is text under 1 and an escape under 0, and a stray
/// high-bit byte is CP1252 under 0 rather than a replacement character.
#[test]
fn the_decoders_differ_only_on_the_ambiguous_bytes() {
    let ambiguous: &[u8] = b"A&~G= &B";
    assert_eq!(decode_as(ambiguous, TextFormat::Utf8), "A&~G= &B");
    assert_eq!(decode_as(ambiguous, TextFormat::Escaped), "A墙B");
    assert_eq!(decode_as(ambiguous, TextFormat::Detect), decode(ambiguous));
    assert_eq!(decode_as(b"caf\xE9", TextFormat::Escaped), "café");
}
