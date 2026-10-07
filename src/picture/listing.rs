//! Text listings of a picture file in the exact shape `gwdump.py` prints
//! them — the summary (`info`), the primitive list (`list`) and its CSV form.
//!
//! The reference implementation is frozen as the oracle (DRAFT plan Q8), so
//! reproducing its lines character for character lets a listing be diffed
//! against the Python output of the same file. Numbers are written the way
//! Python's `'%.4f' % v` then `.rstrip('0').rstrip('.')` writes them, and
//! strings the way `repr()` does.

use std::fmt::Write as _;

use super::directory::Picture;
use super::file::PictureFile;
use super::geom::Transform;
use super::prim::{class_name, kind_name, Payload, Primitive, Text2Body};
use super::segment::{Group, Segment};
use super::style::pen_name;
use super::text::TextFormat;
use super::word::Word;

/// `'%.4f' % v` with trailing zeros and a trailing point removed.
pub fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

/// `(x,y)` with [`num`] coordinates.
pub fn point(p: (f64, f64)) -> String {
    format!("({},{})", num(p.0), num(p.1))
}

/// Python `repr()` of a string: single quotes unless the text has a single
/// quote and no double quote; `\\`, the quote, `\n`, `\r`, `\t` escaped;
/// other control and non-printable characters as `\xNN` / `\uNNNN`.
pub fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_py_printable(c) => out.push(c),
            c if (c as u32) < 0x100 => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if (c as u32) < 0x1_0000 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => {
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
    }
    out.push(quote);
    out
}

/// Python's `str.isprintable` for the characters these files hold: controls,
/// DEL..U+009F, NBSP and the soft hyphen are not printable.
fn is_py_printable(c: char) -> bool {
    let u = c as u32;
    !(u < 0x20 || (0x7F..=0xA0).contains(&u) || u == 0xAD)
}

/// Python `repr()` of a list of `(u0, u1)` unit tuples.
fn units_repr(words: &[Word]) -> String {
    let items: Vec<String> = words
        .iter()
        .map(|w| format!("({}, {})", w.u0, w.u1))
        .collect();
    format!("[{}]", items.join(", "))
}

/// `gwdump.py group_label`: `grp?` for a bare primitive, `grpN=NOUN w0/w1`
/// when the segment's user block lists member `N`, else `grpN`.
pub fn group_label(segment: &Segment, group: &Group) -> String {
    let Some(id) = group.id else {
        return "grp?".to_string();
    };
    match segment.user.as_ref().and_then(|u| u.member(id)) {
        Some(m) => format!("grp{id}={} {}/{}", m.noun, m.w0, m.w1),
        None => format!("grp{id}"),
    }
}

/// `gwdump.py prim_line`: class, kind, counts, pen, colour, then the
/// geometry — in the segment's local frame, or placed by `transform`.
pub fn prim_line(prim: &Primitive, transform: Option<&Transform>) -> String {
    let pen = pen_name(prim.pen);
    let mut s = format!(
        "{:<5} kind={:<2} {:<10} n={:<4} npts={:<4} pen={}{} col={}",
        prim.class_name(),
        prim.kind,
        prim.kind_name(),
        prim.count(),
        prim.npts(),
        prim.pen,
        if pen.is_empty() {
            String::new()
        } else {
            format!("({pen})")
        },
        prim.colour
    );
    let place = |x: f64, y: f64| match transform {
        Some(t) => t.apply(x, y),
        None => (x, y),
    };
    match &prim.payload {
        Payload::Points(points) => {
            s.push_str(" pts=");
            let pts: Vec<String> = points.iter().map(|&(x, y)| point(place(x, y))).collect();
            s.push_str(&pts.join(" "));
        }
        Payload::Conic { start, spans } => {
            let spans: Vec<String> = spans
                .iter()
                .map(|span| {
                    format!(
                        "[mid={} w={} end={}]",
                        point(place(span.mid.0, span.mid.1)),
                        num(span.w),
                        point(place(span.end.0, span.end.1))
                    )
                })
                .collect();
            let _ = write!(
                s,
                " start={} spans={}",
                point(place(start.0, start.1)),
                spans.join(" ")
            );
        }
        Payload::Text(text) => {
            push_text_fields(
                &mut s,
                &text.geom,
                &text.bbox,
                text.attr,
                &text.text(),
                &place,
            );
        }
        Payload::Text2(text) => {
            push_text_fields(
                &mut s,
                &text.geom,
                &text.bbox,
                text.attr,
                &text.text(),
                &place,
            );
            match &text.body {
                Text2Body::Runs(runs) => {
                    let runs: Vec<String> = runs
                        .iter()
                        .map(|r| {
                            format!(
                                "(font={} style={} nl={} z={} {})",
                                r.font,
                                r.style_name(),
                                r.newline,
                                r.z,
                                py_repr(&r.text())
                            )
                        })
                        .collect();
                    let _ = write!(
                        s,
                        " srclen={} src={} runs=[{}]",
                        text.srclen,
                        py_repr(&text.source().unwrap_or_default()),
                        runs.join(" ")
                    );
                }
                Text2Body::Raw(words) => {
                    let _ = write!(
                        s,
                        " srclen={} runs=UNPARSED raw={}",
                        text.srclen,
                        units_repr(words)
                    );
                }
            }
        }
        Payload::Instance {
            reference,
            transform,
        } => {
            let t: Vec<String> = transform.iter().map(|&v| num(v)).collect();
            let _ = write!(
                s,
                " ref={}/{} xf=({})",
                reference.0,
                reference.1,
                t.join(",")
            );
        }
        Payload::Raw(words) => {
            let _ = write!(s, " raw={}", units_repr(words));
        }
    }
    s
}

fn push_text_fields(
    s: &mut String,
    geom: &[f64; 6],
    bbox: &[f64; 4],
    attr: i32,
    text: &str,
    place: &dyn Fn(f64, f64) -> (f64, f64),
) {
    let [x, y, ux, uy, csp, lsp] = *geom;
    let h = ux.hypot(uy);
    let (px, py) = place(x, y);
    let up = if h != 0.0 {
        point((ux / h, uy / h))
    } else {
        "(0,0)".to_string()
    };
    let bbox: Vec<String> = bbox.iter().map(|&v| num(v)).collect();
    let _ = write!(
        s,
        " pos={} h={} up={} csp={} lsp={} box=({}) attr={} fmt={} text={}",
        point((px, py)),
        num(h),
        up,
        num(csp),
        num(lsp),
        bbox.join(","),
        attr,
        py_repr(&TextFormat::decode(attr).to_string()),
        py_repr(text)
    );
}

/// The `(x, y)` pairs the CSV `points` column lists: every vertex for point
/// kinds, start + span ends for conics, the anchor for text, the
/// translation for an instance.
fn csv_points(prim: &Primitive) -> Vec<(f64, f64)> {
    match &prim.payload {
        Payload::Points(points) => points.clone(),
        Payload::Conic { start, spans } => {
            let mut pts = vec![*start];
            pts.extend(spans.iter().map(|s| s.end));
            pts
        }
        Payload::Text(t) => vec![(t.geom[0], t.geom[1])],
        Payload::Text2(t) => vec![(t.geom[0], t.geom[1])],
        Payload::Instance { transform, .. } => vec![(transform[4], transform[5])],
        Payload::Raw(_) => Vec::new(),
    }
}

/// The first line of `gwdump.py --list`.
pub fn list_header(file: &PictureFile, page: bool) -> String {
    format!(
        "# {}  version {}  name {}  coords={}",
        file.path().display(),
        py_repr(&file.header().version),
        py_repr(&file.name().text),
        if page {
            "sheet mm (composed transforms)"
        } else {
            "raw local"
        }
    )
}

/// The CSV header line of `gwdump.py --csv`.
pub const CSV_HEADER: &str = "idx,picture,seg_index,seg_rec,path,group,member_noun,member_w0,member_w1,class,kind,kind_name,count,npts,pen,pen_name,colour,text,points";

/// Every primitive of `file` as `gwdump.py --list` (or `--csv`) lines, in
/// tree order; `page` places coordinates on the sheet through the composed
/// transforms. The header line is not included.
pub fn list_lines(file: &PictureFile, page: bool, csv: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut n = 0usize;
    for picture in file.pictures() {
        for (index, _depth) in picture.walk() {
            let Some(segment) = picture.segment(index) else {
                continue;
            };
            let transform = page.then(|| picture.composed_transform(index));
            let path = picture.path_labels(index).join("/");
            let entry = &picture.entries[index];
            for (group, prim) in segment.prims() {
                if csv {
                    let member = group
                        .id
                        .and_then(|id| segment.user.as_ref().and_then(|u| u.member(id)));
                    let pts: Vec<String> = csv_points(prim)
                        .into_iter()
                        .map(|(x, y)| {
                            let (x, y) = match &transform {
                                Some(t) => t.apply(x, y),
                                None => (x, y),
                            };
                            format!("{};{}", num(x), num(y))
                        })
                        .collect();
                    lines.push(format!(
                        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                        n,
                        picture.number,
                        index,
                        entry.rec(),
                        path,
                        group.id.map_or(String::new(), |id| id.to_string()),
                        member.map_or("", |m| m.noun.as_str()),
                        member.map_or(String::new(), |m| m.w0.to_string()),
                        member.map_or(String::new(), |m| m.w1.to_string()),
                        prim.class_name(),
                        prim.kind,
                        prim.kind_name(),
                        prim.count(),
                        prim.npts(),
                        prim.pen,
                        pen_name(prim.pen),
                        prim.colour,
                        prim.text()
                            .map_or(String::new(), |t| format!("\"{}\"", t.replace('"', "\"\""))),
                        pts.join(" ")
                    ));
                } else {
                    lines.push(format!(
                        "#{:<7} SEG#{:<5} {:<60} {:<28} {}",
                        n,
                        index,
                        if path.is_empty() { "-" } else { path.as_str() },
                        group_label(segment, group),
                        prim_line(prim, transform.as_ref())
                    ));
                }
                n += 1;
            }
        }
    }
    lines
}

/// Kind histogram of a picture: `(kind, count)` ascending by kind.
pub fn kind_histogram(picture: &Picture) -> Vec<(u8, usize)> {
    let mut counts = std::collections::BTreeMap::new();
    for segment in picture.segments.iter().flatten() {
        for (_, prim) in segment.prims() {
            *counts.entry(prim.kind).or_insert(0usize) += 1;
        }
    }
    counts.into_iter().collect()
}

/// `MARKER(2)=368, LINE(3)=259, CONIC(4)=28` — `gwdump.py kinds_str`.
pub fn kinds_str(histogram: &[(u8, usize)]) -> String {
    let parts: Vec<String> = histogram
        .iter()
        .map(|&(k, n)| format!("{}({k})={n}", kind_name(k)))
        .collect();
    parts.join(", ")
}

/// The summary `gwdump.py <file>` prints (without primitives): file, size,
/// header, name, one block per picture with counts and the segment tree.
pub fn summary(file: &PictureFile) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("file      : {}", file.path().display()));
    out.push(format!(
        "size      : {} bytes = {} records of {}",
        file.bytes().len(),
        file.records(),
        super::file::RECORD_BYTES
    ));
    let h = file.header();
    out.push(format!(
        "header    : version {}  name rec {} len {}  attr rec {} len {}  pictab rec {} len {}",
        py_repr(&h.version),
        h.name_rec,
        h.name_len,
        h.attr_rec,
        h.attr_len,
        h.pic_rec,
        h.pic_len
    ));
    out.push(format!("name      : {}", py_repr(&file.name().text)));
    for picture in file.pictures() {
        let window: Vec<String> = picture.window.iter().map(|&v| num(v)).collect();
        out.push(format!(
            "picture {} : dir rec {} (len {} words, {} entries)  int4={}  window=({})",
            picture.number,
            picture.dir_rec,
            picture.dir_len,
            picture.entries.len(),
            picture.int4,
            window.join(",")
        ));
        let histogram = kind_histogram(picture);
        let nseg = picture.segments.iter().flatten().count();
        out.push(format!(
            "  segments {}  primitives {}  by kind: {}",
            nseg,
            picture.prim_count(),
            kinds_str(&histogram)
        ));
        let mut by_class = std::collections::BTreeMap::new();
        for &(k, n) in &histogram {
            *by_class.entry(class_name(k)).or_insert(0usize) += n;
        }
        let by_class: Vec<String> = by_class.iter().map(|(c, n)| format!("{c}={n}")).collect();
        out.push(format!("  by class : {}", by_class.join(", ")));
        out.push("  -- tree (segments) --".to_string());
        for (index, depth) in picture.walk() {
            let Some(segment) = picture.segment(index) else {
                continue;
            };
            let entry = &picture.entries[index];
            let xf = match &segment.transform {
                Some(t) => {
                    let t: Vec<String> = t.iter().map(|&v| num(v)).collect();
                    format!("({})", t.join(","))
                }
                None => "-".to_string(),
            };
            let members = segment
                .user
                .as_ref()
                .filter(|u| !u.members.is_empty())
                .map_or(String::new(), |u| format!("  members={}", u.members.len()));
            let note =
                segment
                    .user
                    .as_ref()
                    .and_then(|u| u.texp.as_ref())
                    .map_or(String::new(), |n| {
                        let corners: Vec<String> = n.corners.iter().map(|&c| point(c)).collect();
                        format!("  note={} box={}", py_repr(&n.text), corners.join(" "))
                    });
            out.push(format!(
                "  {}SEG#{} rec={} words={} flags={:#x} {} xf={} groups={} prims={}{}{}",
                "  ".repeat(depth),
                index,
                entry.rec(),
                entry.nwords(),
                segment.flags,
                segment.label(),
                xf,
                segment.groups.len(),
                segment.prim_count(),
                members,
                note
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_python_percent_point_four_f_stripped() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.0), "-0");
        assert_eq!(num(1.0), "1");
        assert_eq!(num(60.98), "60.98");
        assert_eq!(num(-2360.57), "-2360.57");
        assert_eq!(num(0.0106), "0.0106");
        assert_eq!(num(-138.5407), "-138.5407");
        assert_eq!(num(0.17157287525381), "0.1716");
        assert_eq!(num(1.00005), "1.0001");
        assert_eq!(point((0.0, 297.0)), "(0,297)");
    }

    #[test]
    fn strings_print_like_python_repr() {
        assert_eq!(py_repr("sjr"), "'sjr'");
        assert_eq!(py_repr("GW6A04    PC 06"), "'GW6A04    PC 06'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("say \"hi\" it's"), "'say \"hi\" it\\'s'");
        assert_eq!(py_repr("a\nb\\c"), "'a\\nb\\\\c'");
        assert_eq!(py_repr("\u{0}x\u{9f}\u{a0}\u{e9}"), "'\\x00x\\x9f\\xa0é'");
        assert_eq!(py_repr(""), "''");
    }

    #[test]
    fn the_kind_histogram_prints_like_kinds_str() {
        assert_eq!(
            kinds_str(&[(2, 368), (3, 259), (4, 28)]),
            "MARKER(2)=368, LINE(3)=259, CONIC(4)=28"
        );
    }
}
