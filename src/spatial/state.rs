//! [`ReadState`]: element state as one session of a database stores it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use e3d_attlib::{db1_dehash, AttlibData};

use super::{SpatialState, UndecodableAttr};
use crate::engine::ReadOnlyEngine;
use crate::error::E3dReaderError;
use crate::record::descriptor::{AttributeExtractionStatus, DescriptorValue, ElementExtraction};
use crate::record::template::TemplateProvider;
use crate::refno::RefNo;
use crate::session::SessionSelector;

/// Element state as one session of a database stores it — what Codec_Check,
/// gate question Q13g and requirement 6.9 hold the codec and the built-in AABB
/// against. An element is read, through the descriptor path, the first time it
/// is asked about, and kept.
///
/// A record that cannot be read answers like a missing element to `noun` and
/// `owner` and as undecodable to `attr`; [`ReadState::failures`] says which
/// records those were and why, so a check can tell them from absent ones.
pub struct ReadState {
    engine: RefCell<ReadOnlyEngine>,
    attlib: AttlibData,
    templates: RefCell<TemplateProvider>,
    elements: RefCell<HashMap<RefNo, Element>>,
}

#[derive(Clone)]
enum Element {
    Absent,
    Read(Rc<ElementExtraction>),
    Unreadable(Rc<str>),
}

/// The owner a record states when it has none.
const NO_OWNER: RefNo = RefNo { word0: 0, word1: 0 };

impl ReadState {
    /// Open `path` at the session `at` selects; a session not on the chain is
    /// an error, not another session.
    pub fn open(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
        attlib: AttlibData,
        templates: TemplateProvider,
    ) -> Result<Self, E3dReaderError> {
        Ok(Self {
            engine: RefCell::new(ReadOnlyEngine::open_at(path, at)?),
            attlib,
            templates: RefCell::new(templates),
            elements: RefCell::default(),
        })
    }

    pub fn sesno(&self) -> u32 {
        self.engine.borrow().session().sesno
    }

    /// The records asked about so far that could not be read, in refno order.
    pub fn failures(&self) -> Vec<(RefNo, String)> {
        let mut failures: Vec<(RefNo, String)> = self
            .elements
            .borrow()
            .iter()
            .filter_map(|(refno, element)| match element {
                Element::Unreadable(why) => Some((*refno, why.to_string())),
                _ => None,
            })
            .collect();
        failures.sort_by_key(|(refno, _)| (refno.word0, refno.word1));
        failures
    }

    fn element(&self, refno: RefNo) -> Element {
        if let Some(known) = self.elements.borrow().get(&refno) {
            return known.clone();
        }
        let element = self.read(refno);
        self.elements.borrow_mut().insert(refno, element.clone());
        element
    }

    fn read(&self, refno: RefNo) -> Element {
        let mut engine = self.engine.borrow_mut();
        match engine.find_element(refno) {
            Ok(Some(_)) => {}
            Ok(None) => return Element::Absent,
            Err(error) => return Element::Unreadable(error.to_string().into()),
        }
        match engine.extract_element_with_descriptors(refno, &self.attlib, &mut self.templates.borrow_mut()) {
            Ok(extraction) => Element::Read(Rc::new(extraction)),
            Err(error) => Element::Unreadable(error.to_string().into()),
        }
    }
}

impl SpatialState for ReadState {
    fn noun(&self, refno: RefNo) -> Option<u32> {
        match self.element(refno) {
            Element::Read(element) => Some(element.noun_hash),
            Element::Absent | Element::Unreadable(_) => None,
        }
    }

    fn owner(&self, refno: RefNo) -> Option<RefNo> {
        match self.element(refno) {
            Element::Read(element) if element.owner != NO_OWNER => Some(element.owner),
            _ => None,
        }
    }

    fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        let undecodable = |name: String| UndecodableAttr { refno, name, hash };
        let element = match self.element(refno) {
            Element::Read(element) => element,
            Element::Absent => return Ok(None),
            Element::Unreadable(_) => return Err(undecodable(db1_dehash(hash).unwrap_or_default())),
        };
        let Some(row) = element.attributes.iter().find(|row| row.hash == hash) else {
            return Ok(None);
        };
        match (&row.value, &row.status) {
            (Some(value), _) => Ok(Some(value.clone())),
            // An explicit-only attribute that is not set.
            (None, AttributeExtractionStatus::NonImplicit) => Ok(None),
            (None, _) => Err(undecodable(row.name.clone())),
        }
    }
}
