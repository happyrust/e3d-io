//! One session of a Corpus database opened for Codec_Check: its SPAMAP table,
//! its element state, and who wrote each page — what the Corpus tests of
//! Properties 5 and 8 hold the codec and the built-in AABB against.

#![allow(dead_code)]

use super::{ATTLIB, DESVIR};
use e3d_attlib::AttlibData;
use e3d_io::core_rules::RuleSet;
use e3d_io::index::{read_spamap, SpamapEntry, SpamapRead};
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::spatial::aabb::{BuiltInAabb, ChainComposer};
use e3d_io::spatial::check::{check, Authors, Checked};
use e3d_io::spatial::codec::SpamapCodec;
use e3d_io::spatial::state::ReadState;
use e3d_io::RefNo;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

pub struct Session {
    pub sesno: u32,
    /// The entries in key order.
    pub table: Vec<SpamapEntry>,
    pub state: ReadState,
    pub authors: Authors,
}

impl Session {
    /// `None` when the session lists no SPAMAP table.
    pub fn open(path: &str, at: SessionSelector) -> Option<Session> {
        let mut view = DbView::open_at(path, at).unwrap_or_else(|error| panic!("{path}: open: {error}"));
        let sesno = view.selected_session();
        let authors = Authors::from_sessions(&view.walk_sessions().expect("walk the sessions"));
        let table = match read_spamap(&mut view).unwrap_or_else(|error| panic!("{path}: {error}")) {
            SpamapRead::NoTable { .. } => return None,
            SpamapRead::Table { table, .. } => table.entries().to_vec(),
        };
        let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
        let templates = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
        let state = ReadState::open(path, sesno, attlib, templates).expect("open the element state");
        Some(Session { sesno, table, state, authors })
    }

    /// Whether E3D wrote the element (E3D_Authored_Element).
    pub fn e3d_wrote(&self, refno: RefNo) -> bool {
        self.authors.e3d_wrote(&self.state, refno)
    }

    /// Codec_Check's items over `table` under `rules`, with nothing excluded.
    pub fn check(&self, rules: &RuleSet, table: &[SpamapEntry]) -> Checked {
        let composer = ChainComposer::new(rules);
        let built_in = BuiltInAabb::new(rules, &composer);
        check(table, &self.state, &SpamapCodec::new(rules), &built_in, &|refno| self.e3d_wrote(refno), &|_| false)
    }
}
