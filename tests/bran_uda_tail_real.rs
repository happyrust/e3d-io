use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DEFAULT_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const DICTIONARY_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";

mod common;

const FIXTURES: &[&str] = &[DEFAULT_DB, DICTIONARY_DB];

#[test]
fn print_bran_raw_tail_and_uda_key_hits() {
    if common::missing(FIXTURES) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(DEFAULT_DB).expect("open AMS 7999 database");
    let view = engine
        .find_element(RefNo::new(24383, 73930))
        .expect("find BRAN")
        .expect("BRAN exists");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse BRAN");

    println!(
        "RAW_LEN={}|PREFIX={}|IMPL_WORDS={}|IMPL_END={}|RECORD_END={}|EXPLICIT={}|MEMBERS={}",
        view.raw_bytes.len(),
        parsed.prefix_offset,
        parsed.impl_len_words,
        parsed.prefix_offset + parsed.impl_len_words as usize * 4,
        parsed.record_end,
        parsed.explicit_attributes.len(),
        parsed.members.len()
    );
    for attribute in &parsed.explicit_attributes {
        println!(
            "EXPLICIT|0x{:08X}|TYPE=0x{:04X}|WORDS={}|UDA={}|{:?}",
            attribute.hash,
            attribute.type_code,
            attribute.data_words,
            attribute.is_uda,
            attribute.value
        );
    }

    // The four `H-` hashes are the values these UDAs carried before their keys
    // were re-allocated; the Dictionary definition keeps them in `OLDKEY`. Both
    // generations are probed because an element last written before the move
    // still stores its value under the old one.
    let known_keys = [
        ("H-AIRQUANTITY (OLDKEY)", 398_835_873u32),
        ("H-AIRQUANTITY (UKEY)", 642_952_083u32),
        ("H-FLOWDIRECTION (OLDKEY)", 399_642_785u32),
        ("H-PRESSURESPEC (OLDKEY)", 401_256_609u32),
        ("H-TEMPERATURE (OLDKEY)", 397_865_121u32),
        ("BranWidth", 642_952_068u32),
    ];
    for (name, key) in known_keys {
        let bytes = key.to_be_bytes();
        let hits = view
            .raw_bytes
            .windows(4)
            .enumerate()
            .filter_map(|(offset, candidate)| (candidate == bytes).then_some(offset))
            .collect::<Vec<_>>();
        println!("KEY|{name}|0x{key:08X}|HITS={hits:?}");
    }

    let impl_end = parsed.prefix_offset + parsed.impl_len_words as usize * 4;
    for (chunk_index, chunk) in view.raw_bytes[impl_end..].chunks(16).enumerate() {
        let hex = chunk
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("TAIL|{:08X}|{hex}", impl_end + chunk_index * 16);
    }
}

#[test]
fn print_tpress_definition_tail() {
    if common::missing(FIXTURES) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(DICTIONARY_DB).expect("open AMS Dictionary");
    let view = engine
        .find_element(RefNo::new(13292, 120))
        .expect("find TPress definition")
        .expect("TPress definition exists");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse TPress definition");
    let impl_end = parsed.prefix_offset + parsed.impl_len_words as usize * 4;
    println!(
        "TPRESS_RAW_LEN={}|IMPL_END={}|RECORD_END={}|EXPLICIT={}|MEMBERS={}",
        view.raw_bytes.len(),
        parsed.record_end,
        impl_end,
        parsed.explicit_attributes.len(),
        parsed.members.len()
    );
    for attribute in &parsed.explicit_attributes {
        println!(
            "TPRESS_EXPLICIT|0x{:08X}|0x{:04X}|{}|{:?}",
            attribute.hash, attribute.type_code, attribute.data_words, attribute.value
        );
    }
    for (chunk_index, chunk) in view.raw_bytes[impl_end..parsed.record_end]
        .chunks(16)
        .enumerate()
    {
        println!(
            "TPRESS_TAIL|{:08X}|{}",
            impl_end + chunk_index * 16,
            chunk
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}
