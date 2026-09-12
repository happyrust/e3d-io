//! How a database stores text is stated once, in its file header: the
//! `FORMAT` word at `+0x154` ([`crate::meta::DbDescriptor::header_format`]).
//! `core.dll` reads it before touching any string — `db_get_file_session_string`
//! (`sub_5ACECE0`) and the element text paths (`sub_5ABC400` and friends) all
//! begin with `db_get_file_header_integer(FORMAT)` and branch on `== 1`:
//!
//! * **`FORMAT == 1`** — `sub_5B8F2D0`: the words are unpacked to bytes,
//!   big-endian, stopping at the first NUL, and handed back as they lie. The
//!   API's output is UTF-8, so the bytes are UTF-8. Every AVEVA-shipped
//!   database (the `acp`/`aps` catalogues, the marine samples) and the
//!   Chinese `acw000` project store this way: `acw5001_0001` holds thousands
//!   of raw UTF-8 runs such as `梭式泄压阀` and no escapes.
//! * **anything else** — `sub_5471D70` unpacks, then `sub_54B2CE8` converts
//!   from the project's multi-byte character set (`dword_6453EDC`, the
//!   `MBCHARSET` project setting; `CI-CHI` for a Chinese site) to UTF-8.
//!   That family stores seven-bit text in which characters above 127 survive
//!   in an escaped run: `&~` opens it, ` &` closes it, and every byte in
//!   between is written with its high bit cleared. Reading the bytes as they
//!   lie turns `反应堆厂房内部结构` into `&~74S&6Q3'7?DZ2?=a99 &`, which is
//!   what every non-ASCII description in `ams1112_0001` decoded to, and what
//!   the `TES000` project's 84 databases hold (`TES1034_0001` has 21,164
//!   escapes and no UTF-8). In `core.dll` a high-bit byte *outside* a run
//!   falls through to the converter's single-byte fallback (`sub_54B8E01`,
//!   CP1252); here such a stretch is first read as the code page itself, see
//!   [`TextFormat::Escaped`].
//!
//! Setting the bit back yields the project's multi-byte code page — GBK for
//! the sampled sites, where the eight distinct escaped strings in
//! `ams1112_0001` read as 墙, 板, 埋板, 二次浇注, 二次浇灌, 反应堆厂房,
//! 反应堆厂房内部结构 and 动力院压力容器埋板. A run may sit inside otherwise
//! plain text: `-3.320m~&~!@ &0.00m` is the level range `-3.320m~±0.00m`.
//! Which code page the run is in is a project setting, not a header word, so
//! this reader assumes `CI-CHI` (GBK); nothing sampled says otherwise.
//!
//! Payload bytes are consumed two at a time because the code page is
//! double-byte, and that is also what keeps the closing ` &` unambiguous: a
//! space can appear as the second byte of a pair (0xA0 is a legal trail byte)
//! but never as the first, since 0xA0 is not a legal lead byte.
//!
//! [`decode_as`] takes the header's word; [`decode`] is for a caller that has
//! none and guesses from the bytes ([`TextFormat::Detect`]). The guess is
//! right on every sampled database, but a `FORMAT == 1` string that happens
//! to contain `&~` and ` &` would be mangled by it, which is why the record
//! and session parsers carry the header's answer instead.

const ESCAPE_OPEN: &[u8; 2] = b"&~";
const ESCAPE_CLOSE: &[u8; 2] = b" &";

/// The way a database's strings are stored, as its header's `FORMAT` word
/// states it. See the module documentation for what `core.dll` does with each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFormat {
    /// `FORMAT == 1`: bytes are UTF-8 as they lie; no escape processing.
    Utf8,
    /// Any other `FORMAT`: seven-bit text with `&~ … &` escaped runs in the
    /// project's multi-byte code page (taken to be GBK); high-bit bytes
    /// outside a run are read as that code page too, and as CP1252 when they
    /// are not valid in it.
    Escaped,
    /// The header is not at hand: an escaped run present means [`Escaped`],
    /// otherwise [`Utf8`]. This is the pre-2026-09-12 behaviour.
    ///
    /// [`Escaped`]: TextFormat::Escaped
    /// [`Utf8`]: TextFormat::Utf8
    #[default]
    Detect,
}

impl TextFormat {
    /// The format a header word states. `None` — the descriptor was read from
    /// a buffer too short to hold `+0x154` — is [`TextFormat::Detect`].
    pub fn from_header(format: Option<u32>) -> Self {
        match format {
            Some(1) => Self::Utf8,
            Some(_) => Self::Escaped,
            None => Self::Detect,
        }
    }
}

/// Decode a stored string the way the header's `FORMAT` word says to.
pub fn decode_as(bytes: &[u8], format: TextFormat) -> String {
    match format {
        TextFormat::Utf8 => utf8_as_it_lies(bytes),
        TextFormat::Escaped => escaped(bytes),
        TextFormat::Detect => decode(bytes),
    }
}

/// Decode a stored string without the header's word: restore the high bit of
/// any escaped run, else take the bytes as UTF-8. Prefer [`decode_as`].
pub fn decode(bytes: &[u8]) -> String {
    let Some(restored) = restore_high_bits(bytes) else {
        return String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string();
    };
    let (decoded, _, _) = encoding_rs::GBK.decode(&restored);
    decoded.trim_end_matches('\0').to_string()
}

/// `FORMAT == 1`, `sub_5B8F2D0`: the bytes stop at the first NUL and are
/// otherwise the string.
fn utf8_as_it_lies(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// `FORMAT != 1`, `sub_54B2CE8` from `CI-CHI`: escaped runs are GBK with the
/// high bit restored. The stretches between runs are seven-bit by contract,
/// but `ams1112_0001` holds 78 texts that are raw GBK with no escape at all
/// (`B7B4 D3A6 B6D1 B3A7 B7BF` is `反应堆厂房`, written by something that
/// skipped the escaping), so a stretch is read as the project code page too,
/// and only when it is not valid GBK as CP1252 — the converter's own fallback
/// for a high-bit byte that is not a lead (`sub_54B8E01`). `core.dll` under
/// `CI-CHI` shows those 78 as mojibake; reading them as what they are is the
/// one place this decoder knowingly does better than the product.
fn escaped(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    let mut out = String::with_capacity(bytes.len());
    let mut cursor = 0;
    let mut plain_from = 0;
    let flush = |out: &mut String, plain: &[u8]| {
        if plain.is_empty() {
            return;
        }
        if plain.is_ascii() {
            out.push_str(std::str::from_utf8(plain).expect("ASCII is UTF-8"));
            return;
        }
        let (decoded, _, had_errors) = encoding_rs::GBK.decode(plain);
        if had_errors {
            let (fallback, _, _) = encoding_rs::WINDOWS_1252.decode(plain);
            out.push_str(&fallback);
        } else {
            out.push_str(&decoded);
        }
    };
    while cursor < bytes.len() {
        if let Some(payload) = escaped_run_at(bytes, cursor) {
            flush(&mut out, &bytes[plain_from..cursor]);
            let restored: Vec<u8> = payload.iter().map(|byte| byte | 0x80).collect();
            let (decoded, _, _) = encoding_rs::GBK.decode(&restored);
            out.push_str(&decoded);
            cursor += ESCAPE_OPEN.len() + payload.len() + ESCAPE_CLOSE.len();
            plain_from = cursor;
            continue;
        }
        cursor += 1;
    }
    flush(&mut out, &bytes[plain_from..]);
    out
}

/// `None` when the string carries no escaped run and can be taken as it lies.
fn restore_high_bits(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out: Option<Vec<u8>> = None;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(payload) = escaped_run_at(bytes, cursor) else {
            if let Some(out) = out.as_mut() {
                out.push(bytes[cursor]);
            }
            cursor += 1;
            continue;
        };
        let out = out.get_or_insert_with(|| bytes[..cursor].to_vec());
        out.extend(payload.iter().map(|byte| byte | 0x80));
        cursor += ESCAPE_OPEN.len() + payload.len() + ESCAPE_CLOSE.len();
    }
    out
}

/// The payload of a complete escaped run starting at `cursor`, if there is one.
fn escaped_run_at(bytes: &[u8], cursor: usize) -> Option<&[u8]> {
    if bytes.get(cursor..cursor + ESCAPE_OPEN.len())? != ESCAPE_OPEN.as_slice() {
        return None;
    }
    let start = cursor + ESCAPE_OPEN.len();
    let mut end = start;
    loop {
        match bytes.get(end..end + ESCAPE_CLOSE.len()) {
            Some(pair) if pair == ESCAPE_CLOSE.as_slice() => return Some(&bytes[start..end]),
            // A truncated or unterminated run is left alone rather than
            // guessed at, so no byte is ever invented.
            Some(_) => end += 2,
            None => return None,
        }
    }
}

/// Read a word array as a byte count followed by the characters themselves.
///
/// This is the shape a UDA's `STUDNA` already uses, and geometry attributes
/// reach the comparison as bare words in it too: `Fixity` arrives as
/// `[4, 0x46524545, 0, 0]`, where the four bytes spell `FREE`.
///
/// Every byte the count covers has to be printable for this to be text at all,
/// which is what keeps ordinary numeric arrays out: `Level`'s `[2, 0, 10]`
/// claims two characters and offers two NULs.
pub fn packed(words: &[u32]) -> Option<String> {
    let (length, rest) = words.split_first()?;
    let length = *length as usize;
    if length == 0 || length > rest.len() * 4 {
        return None;
    }
    let bytes: Vec<u8> = rest
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .take(length)
        .collect();
    bytes
        .iter()
        .all(|byte| matches!(byte, 0x20..=0x7E))
        .then(|| String::from_utf8(bytes).expect("printable ASCII is UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every distinct escaped string found in `ams1112_0001`, against the text
    /// it has to be for the description to mean anything.
    #[test]
    fn restores_the_sampled_escaped_strings() {
        let cases: &[(&[u8], &str)] = &[
            (b"&~G= &", "墙"),
            (b"&~0e &", "板"),
            (b"&~Bq0e &", "埋板"),
            (b"&~6~4N==W\" &", "二次浇注"),
            (b"&~6~4N==9` &", "二次浇灌"),
            (b"&~74S&6Q3'7? &", "反应堆厂房"),
            (b"&~74S&6Q3'7?DZ2?=a99 &", "反应堆厂房内部结构"),
            (b"&~6/A&T:Q9A&H]FwBq0e &", "动力院压力容器埋板"),
        ];
        for (stored, expected) in cases {
            assert_eq!(decode(stored), *expected, "{stored:?}");
        }
    }

    /// A run can open and close inside plain text, so the bytes around it must
    /// come through untouched.
    #[test]
    fn restores_a_run_embedded_in_plain_text() {
        assert_eq!(decode(b"-3.320m~&~!@ &0.00m"), "-3.320m~±0.00m");
    }

    /// A space can be the trail byte of a character — 範 is 0xB9 0xA0, which
    /// stores as `9` followed by a space — so scanning byte by byte for the
    /// closing ` &` would cut the run in half.
    #[test]
    fn a_space_inside_a_character_does_not_close_the_run() {
        assert_eq!(decode(b"&~9 0e &"), "範板");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(decode(b"/1RS-CIVI"), "/1RS-CIVI");
        assert_eq!(decode(b"CIVI\0\0"), "CIVI");
        assert_eq!(decode(b""), "");
        // An ampersand that opens nothing stays an ampersand.
        assert_eq!(decode(b"AT&T"), "AT&T");
    }

    /// An unterminated run is data this cannot read, not a licence to guess.
    #[test]
    fn an_unterminated_run_is_left_alone() {
        assert_eq!(decode(b"&~74S&6Q3"), "&~74S&6Q3");
    }

    /// The header word picks the decoder: 1 is UTF-8 as it lies, anything
    /// else is the escaped project charset, none is the guess.
    #[test]
    fn the_header_word_picks_the_decoder() {
        assert_eq!(TextFormat::from_header(Some(1)), TextFormat::Utf8);
        assert_eq!(TextFormat::from_header(Some(0)), TextFormat::Escaped);
        assert_eq!(TextFormat::from_header(Some(2)), TextFormat::Escaped);
        assert_eq!(TextFormat::from_header(None), TextFormat::Detect);
        assert_eq!(TextFormat::default(), TextFormat::Detect);
    }

    /// `FORMAT == 1`: what `acw5001_0001` stores, raw UTF-8 with no escapes,
    /// comes back as itself — and an escape-shaped sequence inside such text
    /// is text, not an escape.
    #[test]
    fn utf8_format_takes_the_bytes_as_they_lie() {
        assert_eq!(
            decode_as("梭式泄压阀".as_bytes(), TextFormat::Utf8),
            "梭式泄压阀"
        );
        assert_eq!(decode_as(b"CIVI\0\0", TextFormat::Utf8), "CIVI");
        assert_eq!(decode_as(b"A&~G= &B", TextFormat::Utf8), "A&~G= &B");
        // The guess would have restored the run.
        assert_eq!(decode_as(b"A&~G= &B", TextFormat::Detect), "A墙B");
        // core stops at the first NUL (`sub_5B8F2D0`).
        assert_eq!(decode_as(b"AB\0CD", TextFormat::Utf8), "AB");
    }

    /// `FORMAT != 1`: escaped runs restore to GBK, plain bytes stay, raw GBK
    /// outside a run (the 78 unescaped texts of `ams1112_0001`) reads as GBK,
    /// and a byte that is not valid GBK is the converter's CP1252 fallback
    /// rather than a replacement character.
    #[test]
    fn escaped_format_restores_runs_and_reads_the_code_page_outside_them() {
        // Raw GBK with no escape, as ams1112_0001 stores it 78 times.
        assert_eq!(
            decode_as(
                b"\xB7\xB4\xD3\xA6\xB6\xD1\xB3\xA7\xB7\xBF",
                TextFormat::Escaped
            ),
            "反应堆厂房"
        );
        assert_eq!(
            decode_as(b"/A-\xB7\xB4\xD3\xA6-&~0e &", TextFormat::Escaped),
            "/A-反应-板"
        );
        assert_eq!(
            decode_as(b"&~74S&6Q3'7? &", TextFormat::Escaped),
            "反应堆厂房"
        );
        assert_eq!(
            decode_as(b"-3.320m~&~!@ &0.00m", TextFormat::Escaped),
            "-3.320m~±0.00m"
        );
        assert_eq!(decode_as(b"&~9 0e &", TextFormat::Escaped), "範板");
        assert_eq!(
            decode_as(b"/1RS-CIVI\0\0", TextFormat::Escaped),
            "/1RS-CIVI"
        );
        assert_eq!(decode_as(b"AT&T", TextFormat::Escaped), "AT&T");
        assert_eq!(decode_as(b"&~74S&6Q3", TextFormat::Escaped), "&~74S&6Q3");
        assert_eq!(decode_as(b"caf\xE9", TextFormat::Escaped), "café");
        assert_eq!(decode_as(b"caf\xE9", TextFormat::Detect), "caf\u{FFFD}");
    }

    /// The guess is what the sampled databases needed before the header word
    /// was read, and stays as it was.
    #[test]
    fn detect_is_the_old_guess() {
        assert_eq!(decode_as(b"&~G= &", TextFormat::Detect), decode(b"&~G= &"));
        assert_eq!(
            decode_as("阀门".as_bytes(), TextFormat::Detect),
            decode("阀门".as_bytes())
        );
    }

    #[test]
    fn reads_count_prefixed_characters() {
        assert_eq!(packed(&[4, 0x4652_4545, 0, 0]).as_deref(), Some("FREE"));
        assert_eq!(
            packed(&[13, 0x482D_4149, 0x5251_5541, 0x4E54_4954, 0x5900_0000]).as_deref(),
            Some("H-AIRQUANTITY")
        );
    }

    /// Numbers that merely start with a plausible count are not text.
    #[test]
    fn declines_numeric_arrays() {
        assert_eq!(packed(&[2, 0, 10]), None);
        assert_eq!(packed(&[4, 40, 1, 1005, 0]), None);
        assert_eq!(packed(&[0]), None);
        assert_eq!(packed(&[9, 0x4652_4545]), None);
        assert_eq!(packed(&[]), None);
    }
}
