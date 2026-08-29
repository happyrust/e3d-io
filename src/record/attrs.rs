use crate::refno::RefNo;

/// Read a 32-bit integer from implicit data at the given word offset.
pub fn read_int(data: &[u8], word_offset: u32) -> Option<i32> {
    let off = word_offset as usize * 4;
    if off + 4 > data.len() {
        return None;
    }
    Some(i32::from_be_bytes(data[off..off + 4].try_into().unwrap()))
}

/// Read a 64-bit float from implicit data at the given word offset.
///
/// Fortran double storage convention: two 32-bit big-endian words in
/// SWAPPED order (low word first, high word second). Each half is
/// individually big-endian, but the two halves are in reverse order
/// relative to standard IEEE 754 big-endian.
pub fn read_f64(data: &[u8], word_offset: u32) -> Option<f64> {
    let off = word_offset as usize * 4;
    if off + 8 > data.len() {
        return None;
    }
    let lo = &data[off..off + 4];
    let hi = &data[off + 4..off + 8];
    let bytes = [hi[0], hi[1], hi[2], hi[3], lo[0], lo[1], lo[2], lo[3]];
    Some(f64::from_be_bytes(bytes))
}

/// Read a 32-bit float from implicit data at the given word offset.
pub fn read_f32(data: &[u8], word_offset: u32) -> Option<f32> {
    let off = word_offset as usize * 4;
    if off + 4 > data.len() {
        return None;
    }
    Some(f32::from_be_bytes(data[off..off + 4].try_into().unwrap()))
}

/// Read a RefNo (2 × u32) from implicit data.
pub fn read_refno(data: &[u8], word_offset: u32) -> Option<RefNo> {
    let off = word_offset as usize * 4;
    if off + 8 > data.len() {
        return None;
    }
    let hi = u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
    let lo = u32::from_be_bytes(data[off + 4..off + 8].try_into().unwrap());
    Some(RefNo::new(hi, lo))
}

/// Read a 3D position/direction as [x, y, z] from f64 components.
pub fn read_pos_f64(data: &[u8], word_offset: u32) -> Option<[f64; 3]> {
    let x = read_f64(data, word_offset)?;
    let y = read_f64(data, word_offset + 2)?;
    let z = read_f64(data, word_offset + 4)?;
    Some([x, y, z])
}

/// Dump all 4-byte words in the implicit region for debugging.
pub fn dump_words(data: &[u8], max_words: usize) -> Vec<(usize, u32)> {
    let mut result = Vec::new();
    for i in 0..max_words.min(data.len() / 4) {
        let off = i * 4;
        let val = u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
        if val != 0 {
            result.push((i, val));
        }
    }
    result
}
