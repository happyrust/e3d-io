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

/// Mixed optimization golden: setup refs already exist, all new refs are pre-minted.
/// Kept separate from the milestone-F baseline above, whose bytes must not change.
#[allow(dead_code)]
pub fn build_mixed(engine: &mut ReadOnlyEngine, templates: &mut TemplateProvider, owner_ref: RefNo, fixtures: &[RefNo], n: usize) -> Result<(Vec<ElementWrite>, Vec<RefNo>), Box<dyn std::error::Error>> {
    if fixtures.len() != 1100 { return Err("mixed batch needs exactly 1100 prepared BOX records".into()); }
    let max = engine.indexed_refnos()?.into_iter().filter(|(r, _, _)| r.word0 == owner_ref.word0).map(|(r, _, _)| r.word1).max().ok_or("owner extent absent")?;
    let template = templates.template_for(db1_hash("BOX"))?.ok_or("no BOX template")?.clone();
    let mut owner = RecordDraft::from_record_bytes(&engine.find_element(owner_ref)?.ok_or("owner disappeared")?.raw_bytes)?;
    let mut writes = Vec::with_capacity(n+1101);
    let mut refs = Vec::with_capacity(n);
    for i in 0..n {
        let r = RefNo::new(owner_ref.word0, max.checked_add(u32::try_from(i)?+1).ok_or("word1 exhausted")?);
        let mut draft = RecordDraft::from_template(r, owner_ref, &template)?;
        for (name,value) in [
            ("POS", DescriptorValue::RealArray(vec![1000.+(i%1000) as f64, 2000.+(i/1000) as f64, 3000.])),
            ("XLEN", DescriptorValue::Real(300.)), ("YLEN", DescriptorValue::Real(400.)), ("ZLEN", DescriptorValue::Real(500.)),
        ] { draft.set_attribute(&template, db1_hash(name), &value, engine.text_format())?; }
        owner.push_member(r);
        refs.push(r);
        writes.push(ElementWrite::Insert(draft));
    }
    for &r in &fixtures[..1000] {
        let mut draft = RecordDraft::from_record_bytes(&engine.find_element(r)?.ok_or("update fixture missing")?.raw_bytes)?;
        draft.set_attribute(&template, db1_hash("XLEN"), &DescriptorValue::Real(301.), engine.text_format())?;
        writes.push(ElementWrite::Update(draft));
    }
    for &r in &fixtures[1000..] { owner.remove_member(r); writes.push(ElementWrite::Delete(r)); }
    writes.push(ElementWrite::Update(owner));
    Ok((writes, refs))
}
