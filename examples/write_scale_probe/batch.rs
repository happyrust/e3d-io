//! Deterministic primitive batch: fixed refnos derived before staging.
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::{TemplateProvider, WIDE_LAYOUT_FLAG};
use e3d_io::write::{ElementWrite, RecordDraft};
use e3d_io::{ReadOnlyEngine, RefNo};

pub fn build(engine: &mut ReadOnlyEngine, templates: &mut TemplateProvider, n: usize) -> Result<(Vec<ElementWrite>, Vec<RefNo>), Box<dyn std::error::Error>> {
    if n > 4000 { return Err("this baseline batch is limited to 4000 members per owner".into()); }
    let zone = engine.scan_elements(ScanTier::Header)?.map(|e| e.map(|e| (e.refno, e.noun_hash)))
        .collect::<Result<Vec<_>, _>>()?.into_iter().find(|(_, noun)| *noun == db1_hash("ZONE"))
        .ok_or("source has no ZONE")?.0;
    let max = engine.indexed_refnos()?.into_iter().filter(|(r, _, _)| r.word0 == zone.word0).map(|(r, _, _)| r.word1).max().unwrap_or(0);
    let template = templates.template_for(db1_hash("BOX"))?.ok_or("no BOX template")?.clone();
    let mut owner = RecordDraft::from_record_bytes(&engine.find_element(zone)?.ok_or("ZONE disappeared")?.raw_bytes)?;
    let mut writes = Vec::new();
    let mut refs = Vec::new();
    for i in 0..n {
        let word1 = max.checked_add(u32::try_from(i)? + 1).ok_or("word1 exhausted")?;
        let refno = RefNo::new(zone.word0, word1);
        let mut implicit = vec![0; 11];
        implicit.extend_from_slice(&template.defaults.implicit);
        implicit[10] |= WIDE_LAYOUT_FLAG;
        let mut draft = RecordDraft::new(refno, db1_hash("BOX"), zone, implicit, vec![], vec![])?;
        for (name, value) in [
            ("POS", DescriptorValue::RealArray(vec![1000.0 + i as f64, 2000.0, 3000.0])),
            ("XLEN", DescriptorValue::Real(300.0)),
            ("YLEN", DescriptorValue::Real(400.0)),
            ("ZLEN", DescriptorValue::Real(500.0)),
        ] { draft.set_attribute(&template, db1_hash(name), &value, engine.text_format())?; }
        owner.push_member(refno);
        refs.push(refno);
        writes.push(ElementWrite::Insert(draft));
    }
    writes.push(ElementWrite::Update(owner));
    Ok((writes, refs))
}
