//! The read side's types move between threads. A GUI or a converter keeps one
//! engine and one template cache per worker thread and shares the parsed
//! attribute library; that only compiles while these bounds hold, so they are
//! asserted here rather than discovered by the first caller that needs them.

use e3d_attlib::AttlibData;
use e3d_io::record::descriptor::ElementExtraction;
use e3d_io::record::template::TemplateProvider;
use e3d_io::{DbSet, E3dReaderError, ReadOnlyEngine, SchemaError, SchemaPack};

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn an_engine_and_its_schema_can_move_to_a_worker_thread() {
    assert_send::<ReadOnlyEngine>();
    assert_send::<TemplateProvider>();
    assert_send::<SchemaPack>();
    assert_send::<ElementExtraction>();
    assert_send::<E3dReaderError>();
    assert_send::<SchemaError>();
}

#[test]
fn the_attribute_library_and_the_element_pool_are_shared_across_threads() {
    assert_send::<AttlibData>();
    assert_sync::<AttlibData>();
    assert_send::<DbSet>();
    assert_sync::<DbSet>();
}
