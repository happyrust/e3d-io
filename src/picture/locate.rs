//! From the PADD database to the picture file and the thumbnail.
//!
//! A sheet (`SHEE`, or an overlay `OVER`) with a picture version `PVNO > 0`
//! has its view graphics at `<proj>pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`: `w0/w1`
//! its refno, `EXFI` its extract database number, `NN = w1 mod 32` (read-3
//! round 1 §2.2: Plant 328 / 328 and Marine 718 / 720 owners found this
//! way, the two missing being sheets of the `/MARKS` library). `BACK` and
//! the symbol / label / template libraries carry `PVNO ≤ 0` and have no
//! file. The same owner and version may exist under several `EXFI` (10000
//! is the master; 10 / 22 / 34 / 46 / 7322 are extracts), and files whose
//! `(owner, EXFI, PVNO)` no longer matches a live element are left behind by
//! `CMRMPF`.
//!
//! DRAW also keeps a 128 × 128 thumbnail of each sheet in the database:
//! `SHEE → THUMBN → ARCHIV (UUIDAR) → ATTDAT`, whose `ELDATA` is
//! `[4][total bytes][UUID × 4][payload bytes][payload]` with the payload's
//! 4-byte words stored little-endian. The payload is a .NET
//! `BinaryFormatter` stream of a `System.Drawing.Bitmap` whose `Data` field
//! is a PNG — 161 bytes of formatter records in front of `‰PNG`, a
//! one-byte `MessageEnd` behind `IEND` (round 1 §2.4). Both `EXFI` and
//! `PVNO` are word-typed in `padvir.dat`, so they are read as the raw word.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db_element::{DbElement, DbElementError, DbFilePin, DbSet};
use crate::engine::{ReadOnlyEngine, ScanTier};
use crate::error::E3dReaderError;
use crate::meta::DbDescriptor;
use crate::record::descriptor::DescriptorValue;
use crate::refno::RefNo;
use crate::schema::{SchemaError, SchemaPack};

/// What can go wrong between a database element and its picture / thumbnail.
#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error(transparent)]
    Db(#[from] DbElementError),

    #[error(transparent)]
    Schema(#[from] SchemaError),

    #[error(transparent)]
    Read(#[from] E3dReaderError),

    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A directory holds no PADD database.
    #[error("{}: no PADD database found", path.display())]
    NoPadd { path: PathBuf },

    /// An attribute that should be a word (or integer) holds something else.
    #[error("{refno}: {name} is {found}, not a word")]
    NotAWord {
        refno: RefNo,
        name: String,
        found: String,
    },

    /// `ELDATA` does not have the thumbnail layout.
    #[error("ELDATA of {refno}: {reason}")]
    BadEldata { refno: RefNo, reason: String },
}

/// The picture-file name of `owner`'s version `pvno` in extract `exfi`.
pub fn picture_file_name(owner: RefNo, exfi: i32, pvno: i32) -> String {
    format!("M{}-{}-{exfi}-{pvno}", owner.word0, owner.word1 as i32)
}

/// The `NN` subdirectory the picture of `owner` lives in: `w1 mod 32`,
/// two digits.
pub fn picture_subdir(owner: RefNo) -> String {
    format!("{:02}", owner.word1 % 32)
}

/// Where a sheet's picture file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureLocation {
    pub owner: RefNo,
    pub exfi: i32,
    pub pvno: i32,
    /// `NN`.
    pub subdir: String,
    /// `M<w0>-<w1>-<EXFI>-<PVNO>`.
    pub file_name: String,
    /// `<pic_dir>\NN\M…`.
    pub path: PathBuf,
}

impl PictureLocation {
    /// The location under `pic_dir` for `owner` at `exfi` / `pvno`.
    pub fn new(pic_dir: &Path, owner: RefNo, exfi: i32, pvno: i32) -> Self {
        let subdir = picture_subdir(owner);
        let file_name = picture_file_name(owner, exfi, pvno);
        let path = pic_dir.join(&subdir).join(&file_name);
        Self {
            owner,
            exfi,
            pvno,
            subdir,
            file_name,
            path,
        }
    }

    /// True when the file is on disk.
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    /// The same owner and version under other extract numbers
    /// (`M<w0>-<w1>-*-<PVNO>` in the subdirectory), sorted, this file
    /// excluded.
    pub fn copies(&self) -> Vec<PathBuf> {
        let prefix = format!("M{}-{}-", self.owner.word0, self.owner.word1 as i32);
        let suffix = format!("-{}", self.pvno);
        let Some(dir) = self.path.parent() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut copies: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p != &self.path && p.is_file())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(&prefix)
                        && n.ends_with(&suffix)
                        && n[prefix.len()..n.len() - suffix.len()]
                            .parse::<i32>()
                            .is_ok()
                })
            })
            .collect();
        copies.sort();
        copies
    }
}

/// A word-typed attribute (`EXFI`, `PVNO`) as the number its raw word is;
/// `Ok(None)` when the noun has no such attribute or it is unset.
pub fn word_number(element: &DbElement, name: &str) -> Result<Option<i32>, LocateError> {
    let value = match element.attribute(name) {
        Ok(value) => value,
        Err(DbElementError::UnknownAttribute { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    match value {
        None | Some(DescriptorValue::Unset) => Ok(None),
        Some(DescriptorValue::Word { raw, .. }) => Ok(Some(raw as i32)),
        Some(DescriptorValue::Int(int)) => Ok(Some(int)),
        Some(other) => Err(LocateError::NotAWord {
            refno: element.refno(),
            name: name.to_string(),
            found: format!("{other:?}"),
        }),
    }
}

/// The element's `(EXFI, PVNO)`, `None` when it carries no `PVNO`.
pub fn picture_version(element: &DbElement) -> Result<Option<(i32, i32)>, LocateError> {
    let Some(pvno) = word_number(element, "PVNO")? else {
        return Ok(None);
    };
    let exfi = word_number(element, "EXFI")?.unwrap_or(-1);
    Ok(Some((exfi, pvno)))
}

/// The picture file of `element` under `pic_dir`: `None` when it has no
/// `PVNO` or `PVNO ≤ 0` (no picture was ever saved). The location is
/// computed, not checked; ask [`PictureLocation::exists`].
pub fn locate(pic_dir: &Path, element: &DbElement) -> Result<Option<PictureLocation>, LocateError> {
    let Some((exfi, pvno)) = picture_version(element)? else {
        return Ok(None);
    };
    if pvno <= 0 {
        return Ok(None);
    }
    Ok(Some(PictureLocation::new(
        pic_dir,
        element.refno(),
        exfi,
        pvno,
    )))
}

/// A sheet's stored thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    /// The `ATTDAT` the bytes came from.
    pub attdat: RefNo,
    /// `ELDATA` words 3–6 (= `ARCHIV.UUIDAR`).
    pub uuid: [u32; 4],
    /// The `BinaryFormatter` stream, byte order restored.
    pub payload: Vec<u8>,
    /// The PNG inside it (`‰PNG` … `IEND` + CRC).
    pub png: Vec<u8>,
}

impl Thumbnail {
    /// Decode an `ELDATA` word block: `[4][total bytes][UUID × 4][payload
    /// bytes][payload words, little-endian]`.
    pub fn from_eldata(attdat: RefNo, words: &[u32]) -> Result<Self, LocateError> {
        let bad = |reason: String| LocateError::BadEldata {
            refno: attdat,
            reason,
        };
        if words.len() < 7 {
            return Err(bad(format!(
                "{} words, fewer than the 7-word header",
                words.len()
            )));
        }
        if words[0] != 4 {
            return Err(bad(format!("first word is {}, not 4", words[0])));
        }
        // The byte total is padded up to whole words in the block.
        let total = words[1] as usize;
        let held = words.len() * 4;
        if total > held || held - total >= 4 {
            return Err(bad(format!(
                "declares {total} bytes but holds {} words = {held} bytes",
                words.len()
            )));
        }
        let uuid = [words[2], words[3], words[4], words[5]];
        let payload_len = words[6] as usize;
        let mut payload: Vec<u8> = Vec::with_capacity((words.len() - 7) * 4);
        for word in &words[7..] {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        if payload_len > payload.len() {
            return Err(bad(format!(
                "declares a {payload_len}-byte payload but holds {} bytes",
                payload.len()
            )));
        }
        payload.truncate(payload_len);
        let png = png_in(&payload)
            .ok_or_else(|| bad("no PNG (‰PNG … IEND) in the payload".to_string()))?
            .to_vec();
        Ok(Self {
            attdat,
            uuid,
            payload,
            png,
        })
    }
}

/// The first PNG in `bytes`, signature to the end of its `IEND` chunk.
fn png_in(bytes: &[u8]) -> Option<&[u8]> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let start = bytes
        .windows(SIGNATURE.len())
        .position(|w| w == SIGNATURE)?;
    let rest = &bytes[start..];
    let iend = rest.windows(4).position(|w| w == b"IEND")?;
    let end = iend + 8; // "IEND" + 4-byte CRC
    rest.get(..end)
}

/// The thumbnail of `sheet`: its first `THUMBN`'s first `ARCHIV`'s first
/// `ATTDAT`'s `ELDATA`. `None` when the chain is not there (a sheet that
/// was never displayed has no thumbnail).
pub fn thumbnail(sheet: &DbElement) -> Result<Option<Thumbnail>, LocateError> {
    let Some(thumbn) = sheet.members_of_type("THUMBN")?.into_iter().next() else {
        return Ok(None);
    };
    let Some(archiv) = thumbn.members_of_type("ARCHIV")?.into_iter().next() else {
        return Ok(None);
    };
    let Some(attdat) = archiv.members_of_type("ATTDAT")?.into_iter().next() else {
        return Ok(None);
    };
    let words = match attdat.attribute("ELDATA")? {
        Some(DescriptorValue::RawWords(words)) => words,
        Some(DescriptorValue::WordArray(words)) => words,
        None | Some(DescriptorValue::Unset) => return Ok(None),
        Some(other) => {
            return Err(LocateError::BadEldata {
                refno: attdat.refno(),
                reason: format!("is {other:?}, not a word block"),
            })
        }
    };
    Thumbnail::from_eldata(attdat.refno(), &words).map(Some)
}

/// The PADD databases of a project, opened as one [`DbSet`] so that
/// references between them resolve.
pub struct PaddProject {
    set: Arc<DbSet>,
    databases: Vec<(PathBuf, u32)>,
}

impl PaddProject {
    /// Open every PADD database directly under `db_dir` (a file is a PADD
    /// database when its descriptor's type dehashes to `PADD`), or `db_dir`
    /// itself when it is one file, with the pack's `padvir.dat` and attlib.
    pub fn open(pack: &SchemaPack, db_dir: impl AsRef<Path>) -> Result<Self, LocateError> {
        let db_dir = db_dir.as_ref();
        let io = |path: &Path, source: std::io::Error| LocateError::Io {
            path: path.to_path_buf(),
            source,
        };
        let candidates: Vec<PathBuf> = if db_dir.is_dir() {
            let mut files: Vec<PathBuf> = fs::read_dir(db_dir)
                .map_err(|e| io(db_dir, e))?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            files
        } else {
            vec![db_dir.to_path_buf()]
        };
        let template = pack.template("PADD")?;
        let set = DbSet::with_attlib_file(pack.attlib()?)?;
        let mut databases = Vec::new();
        for path in candidates {
            if !is_padd(&path) {
                continue;
            }
            let dbno = set.add_db(DbFilePin {
                file: path.clone(),
                template: template.clone(),
                db_type: Some("PADD".to_string()),
                sesno: None,
            })?;
            databases.push((path, dbno));
        }
        if databases.is_empty() {
            return Err(LocateError::NoPadd {
                path: db_dir.to_path_buf(),
            });
        }
        Ok(Self {
            set: Arc::new(set),
            databases,
        })
    }

    /// The set the databases are open in.
    pub fn set(&self) -> &Arc<DbSet> {
        &self.set
    }

    /// The databases opened, `(file, dbno)`, in file-name order.
    pub fn databases(&self) -> &[(PathBuf, u32)] {
        &self.databases
    }

    /// The element `refno`.
    pub fn element(&self, refno: RefNo) -> DbElement {
        self.set.element(refno)
    }

    /// The elements named `name` across the databases.
    pub fn find_named(&self, name: &str) -> Result<Vec<DbElement>, LocateError> {
        let mut out = Vec::new();
        for (_, dbno) in &self.databases {
            for refno in self.set.find_named(*dbno, name)? {
                out.push(self.set.element(refno));
            }
        }
        Ok(out)
    }

    /// Every element whose noun carries `PVNO` — sheets, overlays,
    /// backgrounds and libraries — database by database, in RefNo order.
    /// Whether a noun carries `PVNO` is asked of its template once.
    pub fn picture_owners(&self) -> Result<Vec<DbElement>, LocateError> {
        let mut owners = Vec::new();
        let mut noun_has_pvno: HashMap<u32, bool> = HashMap::new();
        for (path, _) in &self.databases {
            let mut engine = ReadOnlyEngine::open(path)?;
            for scanned in engine.scan_elements(ScanTier::Header)? {
                let scanned = scanned?;
                let element = self.set.element(scanned.refno);
                let has = match noun_has_pvno.get(&scanned.noun_hash) {
                    Some(has) => *has,
                    None => {
                        let has = element.has_attribute("PVNO")?;
                        noun_has_pvno.insert(scanned.noun_hash, has);
                        has
                    }
                };
                if has {
                    owners.push(element);
                }
            }
        }
        Ok(owners)
    }
}

/// True when the file's descriptor page says it is a PADD database.
pub fn is_padd(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut head = vec![0u8; 8192];
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    head.truncate(n);
    DbDescriptor::parse(&head)
        .ok()
        .and_then(|d| d.db_type_name())
        .is_some_and(|ty| ty == "PADD")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_name_and_subdirectory_follow_the_round_1_rule() {
        let owner = RefNo::new(2013286677, 4350);
        assert_eq!(
            picture_file_name(owner, 10000, 11),
            "M2013286677-4350-10000-11"
        );
        assert_eq!(picture_subdir(owner), "30", "4350 mod 32 = 30");
        assert_eq!(picture_subdir(RefNo::new(2013286677, 265)), "09");
        let location = PictureLocation::new(Path::new(r"D:\proj\apspic"), owner, 10000, 11);
        assert_eq!(
            location.path,
            PathBuf::from(r"D:\proj\apspic\30\M2013286677-4350-10000-11")
        );
        assert_eq!(location.subdir, "30");
    }

    #[test]
    fn copies_are_the_other_extracts_of_the_same_version() {
        let dir = std::env::temp_dir().join(format!("e3d-io-locate-{}", std::process::id()));
        let sub = dir.join("00");
        fs::create_dir_all(&sub).unwrap();
        let owner = RefNo::new(2013287597, 16864);
        for name in [
            "M2013287597-16864-10000-1",
            "M2013287597-16864-20-1",
            "M2013287597-16864-32-1",
            "M2013287597-16864-10000-2", // another version
            "M2013287597-16865-10000-1", // another owner
            "M2013287597-16864-x-1",     // not an extract number
        ] {
            fs::write(sub.join(name), b"").unwrap();
        }
        let location = PictureLocation::new(&dir, owner, 10000, 1);
        assert!(location.exists());
        let copies: Vec<String> = location
            .copies()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            copies,
            vec!["M2013287597-16864-20-1", "M2013287597-16864-32-1"]
        );
        assert!(!PictureLocation::new(&dir, owner, 46, 1).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn eldata_yields_the_png_between_the_formatter_records() {
        // A payload: 3 bytes of prefix, a minimal PNG shell, one trailing byte.
        let mut payload = vec![0xAA, 0xBB, 0xCC];
        let png_start = payload.len();
        payload.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        payload.extend_from_slice(&[0, 0, 0, 0]); // IEND length
        payload.extend_from_slice(b"IEND");
        payload.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]); // CRC
        let png_end = payload.len();
        payload.push(0x0B); // MessageEnd
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        let payload_len = png_end + 1;
        let mut words = vec![4u32, 0, 0x1111, 0x2222, 0x3333, 0x4444, payload_len as u32];
        for chunk in payload.as_chunks::<4>().0 {
            words.push(u32::from_le_bytes(*chunk));
        }
        words[1] = (words.len() * 4) as u32;
        let thumb = Thumbnail::from_eldata(RefNo::new(1, 579), &words).unwrap();
        assert_eq!(thumb.uuid, [0x1111, 0x2222, 0x3333, 0x4444]);
        assert_eq!(thumb.payload.len(), payload_len);
        assert_eq!(thumb.png, payload[png_start..png_end]);

        // The header is checked.
        let err = Thumbnail::from_eldata(RefNo::new(1, 579), &words[..5]).unwrap_err();
        assert!(matches!(err, LocateError::BadEldata { .. }), "{err}");
        let mut wrong = words.clone();
        wrong[1] += 4;
        assert!(Thumbnail::from_eldata(RefNo::new(1, 579), &wrong).is_err());
        // A total short of the block by the word padding is fine (aps250133's
        // SHEE 2013286677/1470 declares 3 294 bytes in 824 words).
        let mut padded = words.clone();
        padded[1] -= 2;
        assert!(Thumbnail::from_eldata(RefNo::new(1, 579), &padded).is_ok());
        let mut too_short = words.clone();
        too_short[1] -= 4;
        assert!(Thumbnail::from_eldata(RefNo::new(1, 579), &too_short).is_err());
        let mut no_png = words.clone();
        no_png[7] = 0;
        assert!(Thumbnail::from_eldata(RefNo::new(1, 579), &no_png).is_err());
    }
}
