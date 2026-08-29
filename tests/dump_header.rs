use std::fs::File;
use std::io::Read;
use std::path::Path;

const TEST_DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";

#[test]
fn dump_raw_header() {
    if !Path::new(TEST_DB_PATH).exists() {
        eprintln!("SKIP");
        return;
    }

    let mut file = File::open(TEST_DB_PATH).unwrap();
    let mut buf = [0u8; 256];
    file.read_exact(&mut buf).unwrap();

    println!("\n=== Raw header bytes (first 256 bytes, big-endian) ===");
    for i in (0..256).step_by(4) {
        let val_be = u32::from_be_bytes(buf[i..i + 4].try_into().unwrap());
        let val_le = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
        if val_be != 0 {
            println!(
                "  offset 0x{:02X} ({:>3}): BE=0x{:08X} ({:>12})  LE=0x{:08X} ({:>12})",
                i, i, val_be, val_be, val_le, val_le
            );
        }
    }

    let file_size = file.metadata().unwrap().len();
    println!("\n  File size: {} bytes", file_size);

    for ps in [128, 256, 512, 1024, 2048, 4096, 8192, 11168] {
        let pages = file_size / ps;
        if file_size % ps == 0 {
            println!("  page_size={}: {} pages (exact)", ps, pages);
        }
    }
}
