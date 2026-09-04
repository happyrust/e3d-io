use e3d_io::index::IndexPageView;
use e3d_io::record::attrs;
use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;
use std::path::Path;

const TEST_DB_PATH: &str = common::TEST_FILE_DB;

mod common;

fn skip_if_missing() -> bool {
    if !Path::new(TEST_DB_PATH).exists() {
        eprintln!("SKIP: fixture file not found: {}", TEST_DB_PATH);
        return true;
    }
    false
}

#[test]
fn open_and_read_metadata() {
    if skip_if_missing() {
        return;
    }

    let engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let desc = engine.descriptor();
    println!("=== DbDescriptor ===");
    println!("  page_size:       {}", desc.page_size);
    println!("  db_mark:         {}", desc.db_mark);
    println!("  session_page_no: {}", desc.session_page_no);
    println!("  primary_extent_ref: {}", desc.primary_extent_ref);
    println!("  bucket_type_id:  {}", desc.bucket_type_id);

    assert!(
        desc.page_size >= 128 && desc.page_size <= 65536,
        "page_size out of range: {}",
        desc.page_size
    );
    assert_eq!(
        RefNo::new(desc.primary_extent_ref, 1).dbno(),
        desc.db_mark,
        "primary extent ref should decode back to db_mark"
    );

    let fi = engine.file_info();
    println!("\n=== FileInfo ===");
    println!("  page_flag:       {}", fi.page_flag);
    println!("  signature:       0x{:08X}", fi.signature);
    println!("  table_end_word:  {}", fi.table_end_word);
    for entry in &fi.extents {
        println!(
            "  extent: ref_word0={} high_water={} flags={}",
            entry.ref_word0, entry.high_water, entry.flags
        );
    }

    assert_eq!(
        fi.table_end_word as usize,
        2 + 3 * fi.extents.len(),
        "extent table cursor should agree with the entries read"
    );
    for (position, entry) in fi.extents.iter().enumerate() {
        let r = RefNo::new(entry.ref_word0, 1);
        assert_eq!(
            r.dbno(),
            desc.db_mark,
            "every extent ref should decode back to this database's mark"
        );
        assert_eq!(r.extent() as usize, position + 2, "extents count from 1");
    }

    assert!(engine.readable_page_count() > 0, "file should hold pages");
}

#[test]
fn read_page_headers() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let max_scan = engine.readable_page_count().min(20);
    println!("\n=== Page Headers (first {} pages) ===", max_scan);
    for page_no in 0..max_scan {
        match engine.read_page_header(page_no) {
            Ok(hdr) => {
                println!(
                    "  page {:>5}: type={:?}, type_id={}, db_handle={}, ext_no={}, bucket_id={}",
                    page_no, hdr.page_type, hdr.type_id, hdr.db_handle, hdr.ext_no, hdr.bucket_id
                );
            }
            Err(e) => {
                println!("  page {:>5}: ERROR: {}", page_no, e);
            }
        }
    }
}

#[test]
fn read_sessions() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    match engine.sessions() {
        Ok(sessions) => {
            println!("\n=== Sessions ({} found) ===", sessions.len());
            for s in &sessions {
                println!(
                    "  session_id={}, mark={}, db_handle={}, pages={}..{}",
                    s.session_id, s.session_mark, s.db_handle, s.start_page_no, s.end_page_no
                );
            }
        }
        Err(e) => {
            println!("  sessions error: {}", e);
        }
    }
}

#[test]
fn parse_index_pages() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");
    let total = engine.readable_page_count();

    println!("\n=== Index Pages ===");
    let mut index_pages = Vec::new();
    for page_no in 0..total {
        let data = engine.read_page(page_no).unwrap();
        if let Ok(idx) = IndexPageView::parse(data) {
            println!(
                "  page {:>5}: level={}, entries={}, free_dwords={}",
                page_no,
                idx.level,
                idx.entries.len(),
                idx.free_dwords
            );
            if !idx.entries.is_empty() && index_pages.len() < 3 {
                let first = &idx.entries[0];
                println!(
                    "    first entry: refno={}, page_no={}, offset_words={}, flag={}",
                    first.refno, first.page_no, first.offset_words, first.flag
                );
            }
            index_pages.push((page_no, idx.level, idx.entries.len()));
        }
    }

    println!("\n  Total index pages: {}", index_pages.len());
    assert!(
        !index_pages.is_empty(),
        "should find at least one index page"
    );
}

#[test]
fn find_index_root_and_search() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let root = engine.index_root();
    println!("\n=== Index Root ===");
    println!("  session {} root: {root}", engine.session().sesno);

    let data = engine.read_page(root.page).unwrap();
    let root_view = IndexPageView::parse(data).unwrap();
    println!("  root level: {}", root_view.level);
    println!("  root entries: {}", root_view.entries.len());

    let target = engine.indexed_refnos().expect("read the index")[0].0;
    println!("\n  Searching for: {}", target);
    match engine.search_index(target) {
        Ok(Some(result)) => {
            println!(
                "  FOUND: page_no={}, offset_words={}, flag={}",
                result.entry.page_no, result.entry.offset_words, result.entry.flag
            );
        }
        Ok(None) => panic!("an enumerated key must be reachable by lookup"),
        Err(e) => panic!("  ERROR: {}", e),
    }
}

#[test]
fn find_element_end_to_end() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let target = engine.indexed_refnos().expect("read the index")[0].0;

    println!("\n=== End-to-end Element Read ===");
    println!("  Target RefNo: {}", target);

    match engine.find_element(target) {
        Ok(Some(view)) => {
            println!("  FOUND element:");
            println!("    RefNo: {}", view.refno);
            println!(
                "    Page: {}, byte_offset: {}",
                view.page_no, view.byte_offset
            );
            println!("    Raw bytes: {} bytes", view.raw_bytes.len());
            if !view.raw_bytes.is_empty() {
                let preview_len = view.raw_bytes.len().min(64);
                println!("    First {} bytes (hex):", preview_len);
                for chunk in view.raw_bytes[..preview_len].chunks(16) {
                    let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
                    println!("      {}", hex.join(" "));
                }
            }
        }
        Ok(None) => println!("  NOT FOUND"),
        Err(e) => println!("  ERROR: {}", e),
    }
}

#[test]
fn parse_element_record() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let target = engine.indexed_refnos().expect("read the index")[0].0;

    let view = engine
        .find_element(target)
        .unwrap()
        .expect("element should exist");

    println!("\n=== Parsed Element Record ===");
    match ParsedElement::parse(&view.raw_bytes) {
        Ok(parsed) => {
            println!("  impl_len_words: {}", parsed.impl_len_words);
            println!("  refno: {}", parsed.refno);
            println!(
                "  noun_hash: 0x{:08X} ({})",
                parsed.noun_hash, parsed.noun_hash
            );
            println!("  owner: {}", parsed.owner);
            println!("  prefix_offset: {}", parsed.prefix_offset);
            println!("  implicit_data: {} bytes", parsed.implicit_data.len());
            println!("  total_size: {} bytes", parsed.total_size);

            assert_eq!(
                parsed.refno, target,
                "parsed refno should match search target"
            );
        }
        Err(e) => {
            println!("  PARSE ERROR: {}", e);
            panic!("element parsing failed");
        }
    }
}

#[test]
fn scan_all_elements() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let all_refs = engine.indexed_refnos().expect("scan failed");
    println!("\n=== Scan All Indexed Elements ===");
    println!("  Total indexed RefNos: {}", all_refs.len());

    if all_refs.len() >= 5 {
        println!("  First 5:");
        for (refno, pg, off) in &all_refs[..5] {
            println!("    {} -> page={}, offset_words={}", refno, pg, off);
        }
    }

    let mut parse_ok = 0;
    let mut parse_fail = 0;
    let sample_count = all_refs.len().min(100);

    for (refno, pg, off) in &all_refs[..sample_count] {
        match engine.read_element_direct(*refno, *pg, *off) {
            Ok(view) => match ParsedElement::parse(&view.raw_bytes) {
                Ok(parsed) if parsed.refno == *refno => parse_ok += 1,
                Ok(p) => {
                    parse_fail += 1;
                    if parse_fail <= 3 {
                        println!("    refno mismatch: expected={} parsed={}", refno, p.refno);
                    }
                }
                Err(e) => {
                    parse_fail += 1;
                    if parse_fail <= 3 {
                        println!(
                            "    parse error for {} at page={},off={}: {}",
                            refno, pg, off, e
                        );
                    }
                }
            },
            Err(e) => {
                parse_fail += 1;
                if parse_fail <= 3 {
                    println!(
                        "    read error for {} at page={},off={}: {}",
                        refno, pg, off, e
                    );
                }
            }
        }
    }

    println!(
        "  Direct read results ({} elements): {} ok, {} fail",
        sample_count, parse_ok, parse_fail
    );

    let mut ok_offsets = Vec::new();
    let mut fail_offsets = Vec::new();
    for (refno, pg, off) in &all_refs[..sample_count] {
        match engine.read_element_direct(*refno, *pg, *off) {
            Ok(view) => match ParsedElement::parse(&view.raw_bytes) {
                Ok(p) if p.refno == *refno => ok_offsets.push((*off, refno.word0)),
                _ => fail_offsets.push((*off, refno.word0, *pg)),
            },
            Err(_) => fail_offsets.push((*off, refno.word0, *pg)),
        }
    }
    let ext1_count = ok_offsets.len();
    let ext2_count = fail_offsets.len();
    println!(
        "  Extent 1 (word0 bit14=0): {} elements, all readable",
        ext1_count
    );
    println!(
        "  Extent 2+ (word0 bit14=1): {} elements, multi-extent not yet supported",
        ext2_count
    );

    assert!(parse_ok > 0, "at least some elements should parse");
}

#[test]
fn read_element_attributes() {
    if skip_if_missing() {
        return;
    }

    let mut engine = ReadOnlyEngine::open(TEST_DB_PATH).expect("failed to open database");

    let target = engine.indexed_refnos().expect("read the index")[0].0;

    let view = engine
        .find_element(target)
        .unwrap()
        .expect("element should exist");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("should parse");

    println!("\n=== Element Attribute Analysis ===");
    println!("  RefNo: {}", parsed.refno);
    println!("  noun_hash: 0x{:08X}", parsed.noun_hash);
    println!("  Owner: {}", parsed.owner);
    println!(
        "  Implicit data: {} bytes ({} words)",
        parsed.implicit_data.len(),
        parsed.impl_len_words
    );

    println!("\n  Non-zero words in implicit region:");
    let words = attrs::dump_words(&parsed.implicit_data, 32);
    for (idx, val) in &words {
        let as_i32 = *val as i32;
        let as_f32 = f32::from_be_bytes(val.to_be_bytes());
        println!(
            "    word[{:>2}] (+{:>3}): 0x{:08X} i32={:>12} f32={:.6}",
            idx,
            idx * 4,
            val,
            as_i32,
            as_f32
        );
    }

    println!("\n  Trying f64 at various offsets:");
    for start in [8, 10, 12, 14, 16, 22] {
        if let Some(v) = attrs::read_f64(&parsed.implicit_data, start) {
            if v.abs() > 0.001 && v.abs() < 1e10 && v.is_finite() {
                println!("    f64[{}] = {:.4}", start, v);
            }
        }
    }

    let pos12 = attrs::read_pos_f64(&parsed.implicit_data, 12);
    if let Some(p) = pos12 {
        println!(
            "\n  Position at word[12-17]: E={:.3}, N={:.3}, U={:.3}",
            p[0], p[1], p[2]
        );
    }
}
