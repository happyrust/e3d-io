//! Stored strings are seven-bit. Characters above 127 survive in an escaped
//! run: `&~` opens it, ` &` closes it, and every byte in between is written
//! with its high bit cleared. Reading the bytes as they lie turns
//! `反应堆厂房内部结构` into `&~74S&6Q3'7?DZ2?=a99 &`, which is what every
//! non-ASCII description in the sampled databases decoded to.
//!
//! Setting the bit back yields the project's multi-byte code page — GBK for
//! the AMS sample, where the eight distinct escaped strings in `ams1112_0001`
//! read as 墙, 板, 埋板, 二次浇注, 二次浇灌, 反应堆厂房, 反应堆厂房内部结构
//! and 动力院压力容器埋板. A run may sit inside otherwise plain text:
//! `-3.320m~&~!@ &0.00m` is the level range `-3.320m~±0.00m`.
//!
//! Payload bytes are consumed two at a time because the code page is
//! double-byte, and that is also what keeps the closing ` &` unambiguous: a
//! space can appear as the second byte of a pair (0xA0 is a legal trail byte)
//! but never as the first, since 0xA0 is not a legal lead byte.

const ESCAPE_OPEN: &[u8; 2] = b"&~";
const ESCAPE_CLOSE: &[u8; 2] = b" &";

/// Decode a stored string, restoring the high bit of any escaped run.
pub fn decode(bytes: &[u8]) -> String {
    let Some(restored) = restore_high_bits(bytes) else {
        return String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string();
    };
    let (decoded, _, _) = encoding_rs::GBK.decode(&restored);
    decoded.trim_end_matches('\0').to_string()
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
