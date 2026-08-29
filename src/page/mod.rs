pub mod cache;
pub mod io;

use nom::number::complete::be_u32;
use nom::IResult;

/// Page type enumeration.
///
/// Evidence: E3D_数据库文件架构完整分析.md + struct_layouts.json page_header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    Descriptor,
    RefArray,
    Session,
    Data,
    Special,
    Index,
    Unknown(u32),
}

impl PageType {
    pub fn from_raw(value: u32) -> Self {
        match value {
            0 => Self::Descriptor,
            1 => Self::RefArray,
            3 => Self::Session,
            5 => Self::Data,
            7 => Self::Special,
            8 => Self::Index,
            other => Self::Unknown(other),
        }
    }

    pub fn as_raw(&self) -> u32 {
        match self {
            Self::Descriptor => 0,
            Self::RefArray => 1,
            Self::Session => 3,
            Self::Data => 5,
            Self::Special => 7,
            Self::Index => 8,
            Self::Unknown(v) => *v,
        }
    }
}

/// Common page header present at the start of every database page.
///
/// Layout from struct_layouts.json "page_header" (all fields big-endian):
/// - +0: page_type (u32)
/// - +4: type_id (u32)
/// - +8: db_handle (u32)
/// - +12: ext_no (u32)
/// - +16: page_no (u32)
/// - +20: bucket_id (u32, low 13 bits significant)
#[derive(Debug, Clone)]
pub struct PageHeader {
    pub page_type: PageType,
    pub type_id: u32,
    pub db_handle: u32,
    pub ext_no: u32,
    pub page_no: u32,
    pub bucket_id: u16,
}

impl PageHeader {
    pub const SIZE: usize = 24;

    pub fn parse(data: &[u8]) -> Option<Self> {
        parse_page_header(data).ok().map(|(_, hdr)| hdr)
    }
}

fn parse_page_header(input: &[u8]) -> IResult<&[u8], PageHeader> {
    use nom::Parser;
    let (input, page_type_raw) = be_u32.parse(input)?;
    let (input, type_id) = be_u32.parse(input)?;
    let (input, db_handle) = be_u32.parse(input)?;
    let (input, ext_no) = be_u32.parse(input)?;
    let (input, page_no) = be_u32.parse(input)?;
    let (input, bucket_raw) = be_u32.parse(input)?;

    Ok((
        input,
        PageHeader {
            page_type: PageType::from_raw(page_type_raw),
            type_id,
            db_handle,
            ext_no,
            page_no,
            bucket_id: (bucket_raw & 0x1FFF) as u16,
        },
    ))
}
