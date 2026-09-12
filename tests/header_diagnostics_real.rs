//! Release-paired evidence for the read-only database-header diagnostics.
//!
//! These tests deliberately pin three different meanings of "version":
//! `CRINFO` is the creator banner, `TMPLVN` is the database template version,
//! and the session trailer is the DAB engine that last wrote that session.
//! None of them assigns an E3D SchemaPack.

use e3d_io::{database_header_diagnostics, ReadOnlyEngine};

mod common;

const E3D21: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000";
const E3D31: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

fn paired(name: &str) -> (String, String) {
    (format!(r"{E3D21}\{name}"), format!(r"{E3D31}\{name}"))
}

#[test]
fn paired_design_headers_keep_the_same_creator_and_template_version() {
    let (e3d21, e3d31) = paired("ams8000_0001");
    if common::missing(&[&e3d21, &e3d31]) {
        return;
    }

    let old = database_header_diagnostics(&e3d21).expect("read E3D 2.1 header diagnostics");
    let new = database_header_diagnostics(&e3d31).expect("read E3D 3.1 header diagnostics");

    assert_eq!(old.template_type_name.as_deref(), Some("DESI"));
    assert_eq!(new.template_type_name, old.template_type_name);
    assert_eq!(old.template_version, -1);
    assert_eq!(new.template_version, old.template_version);
    assert_eq!(new.creation_info, old.creation_info);
    assert!(
        old.creation_info
            .as_deref()
            .is_some_and(|text| text.contains("AVEVA Administration Admin Mk1.4.0.3")),
        "{:?}",
        old.creation_info
    );
}

#[test]
fn latest_session_tag_reports_the_dab_engine_that_wrote_each_copy() {
    let (e3d21, e3d31) = paired("ams8000_0001");
    if common::missing(&[&e3d21, &e3d31]) {
        return;
    }

    let old = database_header_diagnostics(&e3d21).expect("read E3D 2.1 header diagnostics");
    let new = database_header_diagnostics(&e3d31).expect("read E3D 3.1 header diagnostics");

    assert!(
        old.dabacon_version_tag
            .as_deref()
            .is_some_and(|tag| tag.starts_with("2_28_")),
        "{:?}",
        old.dabacon_version_tag
    );
    assert!(
        new.dabacon_version_tag
            .as_deref()
            .is_some_and(|tag| tag.starts_with("2_35_")),
        "{:?}",
        new.dabacon_version_tag
    );
}

#[test]
fn a_database_in_the_31_tree_may_retain_the_same_old_writer_tag_as_21() {
    let (e3d21, e3d31) = paired("ams7000_0001");
    if common::missing(&[&e3d21, &e3d31]) {
        return;
    }

    let old = ReadOnlyEngine::open_at(&e3d21, 263)
        .expect("open retained E3D 2.1 session 263")
        .header_diagnostics();
    let new = ReadOnlyEngine::open_at(&e3d31, 263)
        .expect("open retained E3D 3.1 session 263")
        .header_diagnostics();

    assert_eq!(old.dabacon_version_tag.as_deref(), Some("2_28_19_"));
    assert_eq!(new.dabacon_version_tag, old.dabacon_version_tag);
    assert_eq!(new.creation_info, old.creation_info);
    assert_eq!(new.template_version, old.template_version);
}
