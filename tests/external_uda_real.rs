use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const ACP7323: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7323_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";

mod common;

const FIXTURES: &[&str] = &[ACP7323, AMS5100];

#[test]
fn mdscomment_record_exposes_its_explicit_name_and_applicable_nouns() {
    if common::missing(FIXTURES) {
        return;
    }
    let refno = RefNo::new(15_515, 126);
    let mut engine = ReadOnlyEngine::open(ACP7323).expect("open acp7323");
    let view = engine
        .find_element(refno)
        .expect("read MDSComment")
        .expect("MDSComment exists");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse MDSComment");
    eprintln!(
        "raw_len={} implicit_end={} record_end={} explicit={} members={}",
        view.raw_bytes.len(),
        parsed.prefix_offset + parsed.impl_len_words as usize * 4,
        parsed.record_end,
        parsed.explicit_attributes.len(),
        parsed.members.len()
    );
    assert!(
        parsed
            .explicit_attributes
            .iter()
            .any(|attribute| attribute.hash == 639_374),
        "NAME must be decoded from the explicit block"
    );
    assert!(
        parsed
            .explicit_attributes
            .iter()
            .any(|attribute| attribute.hash == 771_611),
        "ELEL must be decoded from the explicit block"
    );
}

#[test]
fn isodrawingno_record_exposes_bran_in_its_explicit_noun_list() {
    if common::missing(FIXTURES) {
        return;
    }
    let refno = RefNo::new(13_292, 174);
    let mut engine = ReadOnlyEngine::open(AMS5100).expect("open ams5100");
    let view = engine
        .find_element(refno)
        .expect("read ISODrawingNo")
        .expect("ISODrawingNo exists");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse ISODrawingNo");
    eprintln!(
        "raw_len={} implicit_end={} record_end={} explicit={:?}",
        view.raw_bytes.len(),
        parsed.prefix_offset + parsed.impl_len_words as usize * 4,
        parsed.record_end,
        parsed
            .explicit_attributes
            .iter()
            .map(|attribute| attribute.hash)
            .collect::<Vec<_>>()
    );
    assert!(
        parsed.explicit_attributes.iter().any(|attribute| {
            attribute.hash == 771_611
                && matches!(
                    &attribute.value,
                    e3d_io::record::explicit::ExplicitValue::IntArray(values)
                        if values.contains(&(808_220i32))
                )
        }),
        "ELEL must include BRAN"
    );
}
