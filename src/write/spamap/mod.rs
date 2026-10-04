//! Write-side SPAMAP planning primitives.

mod end_state;
mod plan;

pub(crate) use end_state::EndState;
pub(crate) use end_state::Snapshot;
pub(crate) use plan::SpamapPlan;

use std::collections::HashMap;

use crate::refno::RefNo;
use crate::spatial::{SpatialPolicy, SpatialReport, WorldAabb};

/// Input-only spatial state carried by a Transaction between edits and stage.
#[derive(Debug, Clone)]
pub(crate) struct SpatialStage {
    pub(crate) policy: SpatialPolicy,
    pub(crate) given: HashMap<RefNo, WorldAabb>,
    pub(crate) reminted: Vec<(RefNo, RefNo)>,
    pub(crate) base_sesno: u32,
}

impl SpatialStage {
    pub(crate) fn prepare(
        &self,
        path: &std::path::Path,
        latest: u32,
        elements: &[super::ElementWrite],
        templates: &mut crate::record::template::TemplateProvider,
    ) -> Result<(SpamapPlan, Vec<super::ElementWrite>), super::WriteError> {
        let mut elements = elements.to_vec();
        if self.policy == SpatialPolicy::Off {
            return Ok((
                SpamapPlan {
                    report: self.report(),
                    ..SpamapPlan::default()
                },
                elements,
            ));
        }
        let shared = std::rc::Rc::new(std::cell::RefCell::new(templates));
        let previous = Snapshot::open(path, latest, shared.clone())?;
        let base = if self.base_sesno != latest {
            Some(Snapshot::open(path, self.base_sesno, shared.clone())?)
        } else {
            None
        };
        let end = EndState::new(&previous, base.as_ref(), &elements);
        let old = if plan::recompute_set(&end).is_empty() {
            None
        } else {
            let mut view = crate::session::DbView::open_at(path, latest)?;
            match crate::index::read_spamap(&mut view)
                .map_err(crate::spatial::SpatialError::MalformedTable)?
            {
                crate::index::SpamapRead::NoTable { .. } => None,
                crate::index::SpamapRead::Table { table, .. } => Some(table.entries().to_vec()),
            }
        };
        let plan = self.plan(&end, &self.reminted, old.as_deref())?;
        let text = crate::ReadOnlyEngine::open_at(path, latest)?.text_format();
        let patches = if matches!(
            crate::core_rules::RuleSet::confirmed().record_item,
            crate::core_rules::Slot::Confirmed(crate::core_rules::RecordItem::Key0ByK)
        ) {
            plan.record_items.as_slice()
        } else {
            &[]
        };
        for patch in patches {
            let position = elements.iter().position(|e| match e {
                super::ElementWrite::Insert(d) | super::ElementWrite::Update(d) => {
                    d.refno == patch.refno
                }
                super::ElementWrite::Delete(_) => false,
            });
            let position = match position {
                Some(position) => position,
                None => {
                    let draft = previous
                        .record(patch.refno)
                        .map_err(|detail| super::WriteError::Io {
                            path: path.to_path_buf(),
                            source: std::io::Error::other(detail),
                        })?
                        .ok_or_else(|| super::WriteError::Io {
                            path: path.to_path_buf(),
                            source: std::io::Error::other("missing record for SPAMAP patch"),
                        })?;
                    elements.push(super::ElementWrite::Update(draft));
                    elements.len() - 1
                }
            };
            let (super::ElementWrite::Insert(draft) | super::ElementWrite::Update(draft)) =
                &mut elements[position]
            else {
                unreachable!()
            };
            let mut provider = shared.borrow_mut();
            let template =
                provider
                    .template_for(draft.noun_hash)?
                    .ok_or_else(|| super::WriteError::Io {
                        path: path.to_path_buf(),
                        source: std::io::Error::other("missing SPAMAP template"),
                    })?;
            let hash = crate::index::SPAMAP_NAME_HASH;
            let attr = template
                .attrs
                .iter()
                .find(|a| a.hash == hash)
                .ok_or_else(|| super::WriteError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("missing SPAMAP descriptor"),
                })?;
            let mut slots = if attr.wide.location.word_offset() != 0 {
                let at = attr.wide.location.word_offset() as usize;
                let count = draft.implicit().get(at).copied().unwrap_or(0);
                if count <= attr.wide.storage_words.saturating_sub(1) {
                    draft
                        .implicit()
                        .get(at + 1..at + 1 + count as usize)
                        .unwrap_or_default()
                        .to_vec()
                } else {
                    Vec::new()
                }
            } else {
                draft
                    .explicit_words(hash)
                    .map(|(_, words)| words)
                    .unwrap_or_default()
            };
            slots.resize(slots.len().max(patch.k as usize + 1), 0x8000_0001);
            slots[patch.k as usize] = patch.key0;
            super::attr::set_attribute(
                draft,
                template,
                hash,
                &crate::record::descriptor::DescriptorValue::RawWords(slots),
                text,
            )
            .map_err(|error| crate::spatial::SpatialError::UndecodableAttr {
                refno: patch.refno,
                name: error.to_string(),
                hash,
            })?;
        }
        Ok((plan, elements))
    }

    pub(crate) fn report(&self) -> SpatialReport {
        SpatialReport {
            policy: self.policy,
            ..SpatialReport::default()
        }
    }

    pub(crate) fn plan(
        &self,
        end: &EndState<'_, '_>,
        remint: &[(RefNo, RefNo)],
        previous_entries: Option<&[crate::index::SpamapEntry]>,
    ) -> Result<SpamapPlan, crate::spatial::SpatialError> {
        plan::plan(self, end, remint, previous_entries)
    }
}
