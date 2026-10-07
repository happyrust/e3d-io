//! Primitives: the ten Wigwam graphic kinds a group holds, decoded field by
//! field and encoded back to the same words.
//!
//! A primitive starts with a header word `u0 = pen | kind << 27`,
//! `u1 = length | colour << 16` (length in words, header included) and is
//! followed by `D = length − 1` data words
//! (`docs/evidence/2026-09-28-gw-picture-format.md` §3.2):
//!
//! | kind | name       | data                                               |
//! |------|------------|----------------------------------------------------|
//! | 1    | TEXT       | 4 box + 6 geometry reals, `[attr][n]`, `⌈n/4⌉` words of 4 chars |
//! | 10   | TEXT2      | 4 + 6 reals, `[attr][srclen]`, runs `[n][font][style][newline][z]` + `⌈n/2⌉` words |
//! | 2    | MARKER     | `D/2` points                                       |
//! | 3    | LINE       | `D/2` points, a polyline                           |
//! | 5    | FILL       | `D/2` points, closed and filled                    |
//! | 8    | BOUNDARY   | `D/2` points                                       |
//! | 4    | CONIC      | start point + `(D−2)/5` spans `[mx, my, w, ex, ey]` |
//! | 7    | FILLCONIC  | as 4, filled                                       |
//! | 9    | BOUNDCONIC | as 4                                               |
//! | 6    | INSTANCE   | `[w0][w1]` + 6 transform reals                     |
//!
//! Where the writer left bytes it did not mean (the tail of a TEXT's last
//! character word, the unit after an odd TEXT2 run), they are kept so the
//! encoding is the original bytes — the corpus gate is byte equality, not
//! structural equality.

use super::text::{codes_to_string, source_from_runs, Run, TextFormat};
use super::word::{Word, Words};
use super::PictureError;

/// Kind numbers, named.
pub mod kind {
    pub const TEXT: u8 = 1;
    pub const MARKER: u8 = 2;
    pub const LINE: u8 = 3;
    pub const CONIC: u8 = 4;
    pub const FILL: u8 = 5;
    pub const INSTANCE: u8 = 6;
    pub const FILLCONIC: u8 = 7;
    pub const BOUNDARY: u8 = 8;
    pub const BOUNDCONIC: u8 = 9;
    pub const TEXT2: u8 = 10;
    pub const SEGMENT: u8 = 14;
    pub const GROUP: u8 = 15;
}

/// The Wigwam name of a kind (`gwdump.py KIND_NAMES`), `?` for one it never
/// writes.
pub fn kind_name(kind: u8) -> &'static str {
    match kind {
        kind::TEXT => "TEXT",
        kind::MARKER => "MARKER",
        kind::LINE => "LINE",
        kind::CONIC => "CONIC",
        kind::FILL => "FILL",
        kind::INSTANCE => "INSTANCE",
        kind::FILLCONIC => "FILLCONIC",
        kind::BOUNDARY => "BOUNDARY",
        kind::BOUNDCONIC => "BOUNDCONIC",
        kind::TEXT2 => "TEXT2",
        kind::SEGMENT => "SEGHDR",
        kind::GROUP => "GROUP",
        _ => "?",
    }
}

/// The Core3D getter family a kind is read through (`WWPrimitive::
/// getPolyline / getPolyarc / getPolymarker / getPolytextAtt`), or `KINDn`.
pub fn class_name(kind: u8) -> String {
    match kind {
        kind::TEXT | kind::TEXT2 => "TEXT".to_string(),
        kind::MARKER => "POINT".to_string(),
        kind::LINE | kind::FILL | kind::BOUNDARY => "LINE".to_string(),
        kind::CONIC | kind::FILLCONIC | kind::BOUNDCONIC => "ARC".to_string(),
        kind::INSTANCE => "INSTANCE".to_string(),
        other => format!("KIND{other}"),
    }
}

/// One circular-arc span of a conic chain: from the current point through
/// `mid` to `end`, `w = bulge²` with bulge = tan(θ/4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub mid: (f64, f64),
    pub w: f64,
    pub end: (f64, f64),
}

/// A TEXT (kind 1) body: 4 box reals, 6 geometry reals, the attribute word
/// and the characters packed four to a word.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPrim {
    /// `(x0, y0, x1, y1)` in character heights, relative to the anchor.
    pub bbox: [f64; 4],
    /// `(x, y, upx·H, upy·H, CSP, LSP)`.
    pub geom: [f64; 6],
    /// The `GQUTXS` attribute word ([`TextFormat::decode`]).
    pub attr: i32,
    /// The `n` characters, one byte each (Latin-1).
    pub bytes: Vec<u8>,
    /// Bytes after the `n`-th character in the packed words: writer
    /// residue (NUL in GW5A1x files, the previous 4-character chunk in
    /// GW6A0x), kept for exact re-encoding.
    pub tail: Vec<u8>,
}

impl TextPrim {
    /// The characters as a string (Latin-1).
    pub fn text(&self) -> String {
        self.bytes.iter().map(|&b| b as char).collect()
    }

    /// The decoded attribute word.
    pub fn format(&self) -> TextFormat {
        TextFormat::decode(self.attr)
    }
}

/// A TEXT2 (kind 10) body: the same ten reals, `[attr][srclen]`, then the
/// runs — or, when the run list does not parse, the raw words.
#[derive(Clone, Debug, PartialEq)]
pub struct Text2Prim {
    pub bbox: [f64; 4],
    pub geom: [f64; 6],
    pub attr: i32,
    /// The word after `attr`: source-string length incl. escapes
    /// (+4 per newline, +6 per style reset).
    pub srclen: i32,
    pub body: Text2Body,
}

/// What follows `[attr][srclen]` in a TEXT2.
#[derive(Clone, Debug, PartialEq)]
pub enum Text2Body {
    /// The runs, parsed.
    Runs(Vec<Run>),
    /// Every word after the ten reals — `attr` and `srclen` included — kept
    /// verbatim because the run walk did not fit the words.
    Raw(Vec<Word>),
}

impl Text2Prim {
    /// The decoded attribute word.
    pub fn format(&self) -> TextFormat {
        TextFormat::decode(self.attr)
    }

    /// The runs when they parsed.
    pub fn runs(&self) -> Option<&[Run]> {
        match &self.body {
            Text2Body::Runs(runs) => Some(runs),
            Text2Body::Raw(_) => None,
        }
    }

    /// The runs' characters joined, `\n` before a run that starts a new
    /// line; empty when the runs did not parse (`gwdump.py Prim.text`).
    pub fn text(&self) -> String {
        match &self.body {
            Text2Body::Runs(runs) => {
                let mut out = String::new();
                for run in runs {
                    if run.newline != 0 {
                        out.push('\n');
                    }
                    out.push_str(&run.text());
                }
                out
            }
            Text2Body::Raw(_) => String::new(),
        }
    }

    /// The escaped source string rebuilt from the runs
    /// ([`source_from_runs`] with `&`), `None` when they did not parse.
    pub fn source(&self) -> Option<String> {
        self.runs()
            .map(|runs| source_from_runs(self.attr, runs, '&'))
    }
}

/// The body of a primitive, by kind family.
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    /// MARKER / LINE / FILL / BOUNDARY: `(x, y)` pairs.
    Points(Vec<(f64, f64)>),
    /// CONIC / FILLCONIC / BOUNDCONIC: a start point and arc spans.
    Conic { start: (f64, f64), spans: Vec<Span> },
    /// TEXT.
    Text(TextPrim),
    /// TEXT2.
    Text2(Text2Prim),
    /// INSTANCE: the `(w0, w1)` DB reference of a segment in another
    /// picture and the placing transform `(a, b, c, d, tx, ty)`.
    Instance {
        reference: (i32, i32),
        transform: [f64; 6],
    },
    /// A kind Wigwam is not known to write: the data words as they are.
    Raw(Vec<Word>),
}

/// One primitive with its header fields and where it sits in its segment.
#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    /// Wigwam kind 1..10.
    pub kind: u8,
    /// Pen (representation) number: 8001–8045 line styles, 8000 fill,
    /// 8100–8115 system pens.
    pub pen: u32,
    /// DRAFT colour-table index; `> 365` means layer `n − 365`.
    pub colour: u16,
    /// 1-based word of the header within the segment.
    pub pos: usize,
    /// Words including the header.
    pub length: u16,
    pub payload: Payload,
}

impl Primitive {
    /// Decode the primitive whose header is word `pos` of `words`.
    pub fn decode(words: &Words<'_>, pos: usize) -> Result<Self, PictureError> {
        let header = words.get(pos)?;
        let kind = header.kind();
        let pen = header.payload();
        let length = header.length();
        let colour = header.extra();
        if length == 0 {
            return Err(PictureError::BadPrimitive {
                rec: words.rec(),
                word: pos,
                reason: "zero-length primitive".to_string(),
            });
        }
        let d = length as usize - 1;
        let real = |i: usize| words.real(pos + i);
        let payload = match kind {
            kind::MARKER | kind::LINE | kind::FILL | kind::BOUNDARY => {
                let mut points = Vec::with_capacity(d / 2);
                for i in 0..d / 2 {
                    points.push((real(1 + 2 * i)?, real(2 + 2 * i)?));
                }
                Payload::Points(points)
            }
            kind::CONIC | kind::FILLCONIC | kind::BOUNDCONIC => {
                let start = (real(1)?, real(2)?);
                let count = d.saturating_sub(2) / 5;
                let mut spans = Vec::with_capacity(count);
                for i in 0..count {
                    let base = 3 + 5 * i;
                    spans.push(Span {
                        mid: (real(base)?, real(base + 1)?),
                        w: real(base + 2)?,
                        end: (real(base + 3)?, real(base + 4)?),
                    });
                }
                Payload::Conic { start, spans }
            }
            kind::TEXT | kind::TEXT2 => {
                let bbox = [real(1)?, real(2)?, real(3)?, real(4)?];
                let geom = [real(5)?, real(6)?, real(7)?, real(8)?, real(9)?, real(10)?];
                let ints = words.slice(pos + 11, d.saturating_sub(10))?;
                let attr = ints.first().map_or(0, |w| w.as_int());
                let n = ints.get(1).map_or(0, |w| w.as_int());
                if kind == kind::TEXT {
                    // `[attr][n][⌈n/4⌉ words]`, u1 of each word = 4 chars,
                    // first char in the high byte.
                    let mut packed = Vec::with_capacity(ints.len().saturating_sub(2) * 4);
                    for word in ints.iter().skip(2) {
                        packed.extend_from_slice(&word.u1.to_be_bytes());
                    }
                    let n = usize::try_from(n).unwrap_or(0).min(packed.len());
                    let tail = packed.split_off(n);
                    Payload::Text(TextPrim {
                        bbox,
                        geom,
                        attr,
                        bytes: packed,
                        tail,
                    })
                } else {
                    let body = match parse_runs(&ints) {
                        Some(runs) => Text2Body::Runs(runs),
                        None => Text2Body::Raw(ints),
                    };
                    Payload::Text2(Text2Prim {
                        bbox,
                        geom,
                        attr,
                        srclen: n,
                        body,
                    })
                }
            }
            kind::INSTANCE => {
                let reference = (words.int(pos + 1)?, words.int(pos + 2)?);
                let transform = [real(3)?, real(4)?, real(5)?, real(6)?, real(7)?, real(8)?];
                Payload::Instance {
                    reference,
                    transform,
                }
            }
            _ => Payload::Raw(words.slice(pos + 1, d)?),
        };
        Ok(Self {
            kind,
            pen,
            colour,
            pos,
            length,
            payload,
        })
    }

    /// The words `sub_54D9F04` appended for this primitive: header first.
    pub fn encode(&self) -> Vec<Word> {
        let mut out = Vec::with_capacity(self.length as usize);
        out.push(Word::header(self.kind, self.pen, self.length, self.colour));
        match &self.payload {
            Payload::Points(points) => {
                for &(x, y) in points {
                    out.push(Word::real(x));
                    out.push(Word::real(y));
                }
            }
            Payload::Conic { start, spans } => {
                out.push(Word::real(start.0));
                out.push(Word::real(start.1));
                for span in spans {
                    out.push(Word::real(span.mid.0));
                    out.push(Word::real(span.mid.1));
                    out.push(Word::real(span.w));
                    out.push(Word::real(span.end.0));
                    out.push(Word::real(span.end.1));
                }
            }
            Payload::Text(text) => {
                out.extend(text.bbox.iter().map(|&v| Word::real(v)));
                out.extend(text.geom.iter().map(|&v| Word::real(v)));
                out.push(Word::int(text.attr));
                out.push(Word::int(text.bytes.len() as i32));
                let mut packed = text.bytes.clone();
                packed.extend_from_slice(&text.tail);
                while packed.len() % 4 != 0 {
                    packed.push(0);
                }
                for chunk in packed.as_chunks::<4>().0 {
                    out.push(Word::units(0, u32::from_be_bytes(*chunk)));
                }
            }
            Payload::Text2(text) => {
                out.extend(text.bbox.iter().map(|&v| Word::real(v)));
                out.extend(text.geom.iter().map(|&v| Word::real(v)));
                match &text.body {
                    Text2Body::Runs(runs) => {
                        out.push(Word::int(text.attr));
                        out.push(Word::int(text.srclen));
                        for run in runs {
                            out.extend(
                                [run.n, run.font, run.style, run.newline, run.z].map(Word::int),
                            );
                            let mut codes: Vec<u32> = run.codes.clone();
                            codes.extend_from_slice(&run.pad);
                            if codes.len() % 2 == 1 {
                                codes.push(0);
                            }
                            for pair in codes.as_chunks::<2>().0 {
                                out.push(Word::units(pair[0], pair[1]));
                            }
                        }
                    }
                    Text2Body::Raw(words) => out.extend_from_slice(words),
                }
            }
            Payload::Instance {
                reference,
                transform,
            } => {
                out.push(Word::int(reference.0));
                out.push(Word::int(reference.1));
                out.extend(transform.iter().map(|&v| Word::real(v)));
            }
            Payload::Raw(words) => out.extend_from_slice(words),
        }
        out
    }

    /// The count `GQUPRA` (op 23) reports: spans for LINE / CONIC, points
    /// for MARKER / FILL / BOUNDARY, characters for TEXT, `srclen` for TEXT2.
    pub fn count(&self) -> usize {
        let d = self.length as usize - 1;
        match self.kind {
            kind::MARKER | kind::FILL | kind::BOUNDARY => d / 2,
            kind::LINE => d.saturating_sub(2) / 2,
            kind::CONIC => d.saturating_sub(2) / 5,
            kind::FILLCONIC | kind::BOUNDCONIC => d.saturating_sub(2) / 5 + 1,
            kind::TEXT => match &self.payload {
                Payload::Text(text) => text.bytes.len(),
                _ => 0,
            },
            kind::TEXT2 => match &self.payload {
                Payload::Text2(text) => text.srclen.max(0) as usize,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// `(x, y)` pairs stored: every vertex for point kinds, start + one end
    /// per span for conics, the anchor for text, the translation for an
    /// instance.
    pub fn npts(&self) -> usize {
        match &self.payload {
            Payload::Points(points) => points.len(),
            Payload::Conic { spans, .. } => 1 + spans.len(),
            Payload::Text(_) | Payload::Text2(_) | Payload::Instance { .. } => 1,
            Payload::Raw(_) => 0,
        }
    }

    /// The Wigwam kind name.
    pub fn kind_name(&self) -> &'static str {
        kind_name(self.kind)
    }

    /// The Core3D getter family name.
    pub fn class_name(&self) -> String {
        class_name(self.kind)
    }

    /// The text of a TEXT / TEXT2, `None` for other kinds.
    pub fn text(&self) -> Option<String> {
        match &self.payload {
            Payload::Text(text) => Some(text.text()),
            Payload::Text2(text) => Some(text.text()),
            _ => None,
        }
    }
}

/// Walk `[attr][srclen]` + runs. `None` when a run header runs past the
/// words, a count is negative, or a character code is not below 0x110000 —
/// the same three checks `gwdump.py` applies before trusting the run view.
fn parse_runs(ints: &[Word]) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut q = 2usize;
    while q < ints.len() {
        if q + 5 > ints.len() {
            return None;
        }
        let n = ints[q].as_int();
        let font = ints[q + 1].as_int();
        let style = ints[q + 2].as_int();
        let newline = ints[q + 3].as_int();
        let z = ints[q + 4].as_int();
        let n_usize = usize::try_from(n).ok()?;
        let nw = n_usize.div_ceil(2);
        if q + 5 + nw > ints.len() {
            return None;
        }
        let mut codes = Vec::with_capacity(nw * 2);
        for word in &ints[q + 5..q + 5 + nw] {
            codes.push(word.u0);
            codes.push(word.u1);
        }
        if codes.iter().take(n_usize).any(|&c| c >= 0x11_0000) {
            return None;
        }
        let pad = codes.split_off(n_usize);
        runs.push(Run {
            n,
            font,
            style,
            newline,
            z,
            codes,
            pad,
        });
        q += 5 + nw;
    }
    Some(runs)
}

/// Characters of TEXT2 runs as one string (used by listings).
pub fn runs_text(runs: &[Run]) -> String {
    let mut out = String::new();
    for run in runs {
        out.push_str(&codes_to_string(&run.codes));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::word::to_bytes;

    fn words_of(words: &[Word]) -> Vec<u8> {
        to_bytes(words)
    }

    #[test]
    fn a_line_decodes_to_its_points_and_encodes_back() {
        // The small sample's sheet frame: LINE pen 8107 colour 1, 5 points.
        let mut words = vec![Word::header(kind::LINE, 8107, 11, 1)];
        for &(x, y) in &[
            (0.0, 0.0),
            (210.0, 0.0),
            (210.0, 297.0),
            (0.0, 297.0),
            (0.0, 0.0),
        ] {
            words.push(Word::real(x));
            words.push(Word::real(y));
        }
        let bytes = words_of(&words);
        let prim = Primitive::decode(&Words::new(&bytes, 15), 1).unwrap();
        assert_eq!(
            (prim.kind, prim.pen, prim.colour, prim.length),
            (3, 8107, 1, 11)
        );
        assert_eq!(prim.count(), 4);
        assert_eq!(prim.npts(), 5);
        assert_eq!(prim.kind_name(), "LINE");
        assert_eq!(prim.class_name(), "LINE");
        match &prim.payload {
            Payload::Points(points) => assert_eq!(points[2], (210.0, 297.0)),
            other => panic!("{other:?}"),
        }
        assert_eq!(prim.encode(), words);
    }

    #[test]
    fn a_conic_splits_into_start_and_spans() {
        let mut words = vec![Word::header(kind::CONIC, 8001, 13, 101)];
        for v in [
            -237.6231, -2381.8999, -237.6231, -2339.2401, 1.0, -280.2829, -2339.2401, -280.2829,
            -2381.8999, 1.0, -237.6231, -2381.8999,
        ] {
            words.push(Word::real(v));
        }
        let bytes = words_of(&words);
        let prim = Primitive::decode(&Words::new(&bytes, 255), 1).unwrap();
        assert_eq!(prim.count(), 2);
        assert_eq!(prim.npts(), 3);
        match &prim.payload {
            Payload::Conic { start, spans } => {
                assert_eq!(*start, (-237.6231, -2381.8999));
                assert_eq!(spans.len(), 2);
                assert_eq!(spans[0].w, 1.0);
                assert_eq!(spans[1].end, (-237.6231, -2381.8999));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(prim.encode(), words);
    }

    #[test]
    fn a_text_keeps_the_writer_residue_after_its_characters() {
        // 'sjr' (n = 3) packed in one word whose last byte is residue 0x58.
        let mut words = vec![Word::header(kind::TEXT, 8001, 14, 1)];
        words.extend([-0.7246, -0.6667, 0.7246, 0.6667].map(Word::real));
        words.extend([697.89, 60.83, -0.0, 3.0, 0.0, 0.0].map(Word::real));
        words.push(Word::int(-1_509_947_401));
        words.push(Word::int(3));
        words.push(Word::units(0, u32::from_be_bytes(*b"sjrX")));
        let bytes = words_of(&words);
        let prim = Primitive::decode(&Words::new(&bytes, 7), 1).unwrap();
        assert_eq!(prim.count(), 3);
        assert_eq!(prim.text().as_deref(), Some("sjr"));
        match &prim.payload {
            Payload::Text(text) => {
                assert_eq!(text.tail, b"X");
                assert_eq!(text.format().to_string(), "FONT    11,RIGHT,CENTRE,HBODY");
                assert_eq!(text.geom[3], 3.0);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(prim.encode(), words);
    }

    #[test]
    fn a_text2_parses_its_runs_or_keeps_the_words_raw() {
        // '?pieceno?' as one 9-character run in font 5 (from
        // amspic\00\M2013287597-1792-10000-1, record 177 word 57).
        let mut words = vec![Word::header(kind::TEXT2, 8001, 23, 10)];
        words.extend([0.0, -0.0, 5.23, 1.3333].map(Word::real));
        words.extend([27.8651, 12.6491, 0.0, 2.4, 0.0, 0.0].map(Word::real));
        words.extend([-1_509_948_495, 9, 9, 5, 0, 0, 0].map(Word::int));
        for pair in [('?', 'p'), ('i', 'e'), ('c', 'e'), ('n', 'o')] {
            words.push(Word::units(pair.0 as u32, pair.1 as u32));
        }
        words.push(Word::units('?' as u32, 0));
        assert_eq!(words.len(), 23);
        let bytes = words_of(&words);
        let prim = Primitive::decode(&Words::new(&bytes, 177), 1).unwrap();
        assert_eq!(prim.count(), 9, "GQUPRA reports srclen");
        assert_eq!(prim.text().as_deref(), Some("?pieceno?"));
        match &prim.payload {
            Payload::Text2(text) => {
                let runs = text.runs().expect("the run list parses");
                assert_eq!(runs.len(), 1);
                assert_eq!(runs[0].pad, vec![0]);
                assert_eq!(text.source().as_deref(), Some("?pieceno?"));
                assert_eq!(text.format().to_string(), "FONT     5,RIGHT,LBODY,BBODY");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(prim.encode(), words);

        // A run header claiming more characters than there are words falls
        // back to the raw view, which still encodes to the same bytes.
        let mut broken = words.clone();
        broken[13] = Word::int(99);
        let bytes = words_of(&broken);
        let prim = Primitive::decode(&Words::new(&bytes, 177), 1).unwrap();
        match &prim.payload {
            Payload::Text2(text) => {
                assert!(text.runs().is_none());
                assert_eq!(text.text(), "");
                assert_eq!(text.source(), None);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(prim.encode(), broken);
    }

    #[test]
    fn an_instance_is_a_reference_and_a_transform() {
        let mut words = vec![Word::header(kind::INSTANCE, 0, 9, 0)];
        words.push(Word::int(15199));
        words.push(Word::int(27821));
        words.extend([1.0, 0.0, 0.0, 1.0, 0.0, 0.0].map(Word::real));
        let bytes = words_of(&words);
        let prim = Primitive::decode(&Words::new(&bytes, 1), 1).unwrap();
        assert_eq!(prim.count(), 0);
        assert_eq!(prim.npts(), 1);
        assert_eq!(
            prim.payload,
            Payload::Instance {
                reference: (15199, 27821),
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }
        );
        assert_eq!(prim.encode(), words);
    }

    #[test]
    fn a_zero_length_header_is_an_error_not_a_loop() {
        let words = [Word::header(kind::LINE, 8001, 0, 1)];
        let bytes = words_of(&words);
        let err = Primitive::decode(&Words::new(&bytes, 20), 1).unwrap_err();
        assert!(
            matches!(
                err,
                PictureError::BadPrimitive {
                    rec: 20,
                    word: 1,
                    ..
                }
            ),
            "{err}"
        );
    }
}
