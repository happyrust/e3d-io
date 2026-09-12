use e3d_attlib::db1_hash;
use e3d_io::{ReadOnlyEngine, ScanTier};

fn main() {
    let path = std::env::args().nth(1).expect("database path");
    let mut engine = ReadOnlyEngine::open(path).expect("open database");
    let target = db1_hash("WORL");
    let mut scan = engine.scan_elements(ScanTier::Full).expect("scan");
    for item in &mut scan {
        let item = item.expect("element");
        if item.noun_hash == target {
            println!("{item:#?}");
        }
    }
}
