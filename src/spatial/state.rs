//! [`ReadState`]: element state as one session of a database stores it.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use e3d_attlib::{db1_dehash, AttlibData};

use super::{SpatialState, UndecodableAttr};
use crate::engine::ReadOnlyEngine;
use crate::error::E3dReaderError;
use crate::record::descriptor::{AttributeExtractionStatus, DescriptorValue, ElementExtraction};
use crate::record::template::TemplateProvider;
use crate::record::RecordHeader;
use crate::refno::RefNo;
use crate::session::SessionSelector;

/// Element state as one session of a database stores it — what Codec_Check,
/// gate question Q13g and requirement 6.9 hold the codec and the built-in AABB
/// against.
///
/// An element's record header (noun, owner, the page it is on) is read the
/// first time the element is asked about; its attributes go through the
/// descriptor path only when one is asked for. Both are kept.
///
/// A record that cannot be read answers like a missing element to `noun` and
/// `owner` and as undecodable to `attr`; [`ReadState::failures`] says which
/// records those were and why, so a check can tell them from absent ones.
pub struct ReadState {
    engine: RefCell<ReadOnlyEngine>,
    attlib: AttlibData,
    templates: RefCell<TemplateProvider>,
    elements: RefCell<HashMap<RefNo, Rc<Element>>>,
}

enum Element {
    Absent,
    Read { header: RecordHeader, page: u32, attributes: OnceCell<Result<ElementExtraction, String>> },
    Unreadable(String),
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

    /// The page the element's record starts on, or `None` when there is no
    /// such element or its record cannot be read.
    pub fn record_page(&self, refno: RefNo) -> Option<u32> {
        match &*self.element(refno) {
            Element::Read { page, .. } => Some(*page),
            Element::Absent | Element::Unreadable(_) => None,
        }
    }

    /// The records asked about so far that could not be read, with why, in
    /// refno order: unreadable headers, and attributes that failed to extract.
    pub fn failures(&self) -> Vec<(RefNo, String)> {
        let mut failures: Vec<(RefNo, String)> = self
            .elements
            .borrow()
            .iter()
            .filter_map(|(refno, element)| match &**element {
                Element::Unreadable(why) => Some((*refno, why.clone())),
                Element::Read { attributes, .. } => match attributes.get() {
                    Some(Err(why)) => Some((*refno, why.clone())),
                    _ => None,
                },
                Element::Absent => None,
            })
            .collect();
        failures.sort_by_key(|(refno, _)| (refno.word0, refno.word1));
        failures
    }

    fn element(&self, refno: RefNo) -> Rc<Element> {
        if let Some(known) = self.elements.borrow().get(&refno) {
            return Rc::clone(known);
        }
        let element = Rc::new(match self.engine.borrow_mut().find_element(refno) {
            Ok(Some(view)) => Element::Read { header: view.header(), page: view.page_no, attributes: OnceCell::new() },
            Ok(None) => Element::Absent,
            Err(error) => Element::Unreadable(error.to_string()),
        });
        self.elements.borrow_mut().insert(refno, Rc::clone(&element));
        element
    }
}

impl SpatialState for ReadState {
    fn noun(&self, refno: RefNo) -> Option<u32> {
        match &*self.element(refno) {
            Element::Read { header, .. } => Some(header.noun_hash),
            Element::Absent | Element::Unreadable(_) => None,
        }
    }

    fn owner(&self, refno: RefNo) -> Option<RefNo> {
        match &*self.element(refno) {
            Element::Read { header, .. } if header.owner != NO_OWNER => Some(header.owner),
            _ => None,
        }
    }

    fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        let undecodable = |name: String| UndecodableAttr { refno, name, hash };
        let element = self.element(refno);
        let attributes = match &*element {
            Element::Read { attributes, .. } => attributes.get_or_init(|| {
                self.engine
                    .borrow_mut()
                    .extract_element_with_descriptors(refno, &self.attlib, &mut self.templates.borrow_mut())
                    .map_err(|error| error.to_string())
            }),
            Element::Absent => return Ok(None),
            Element::Unreadable(_) => return Err(undecodable(db1_dehash(hash).unwrap_or_default())),
        };
        let Ok(extraction) = attributes else {
            return Err(undecodable(db1_dehash(hash).unwrap_or_default()));
        };
        let Some(row) = extraction.attributes.iter().find(|row| row.hash == hash) else {
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
