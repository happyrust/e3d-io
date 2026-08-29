/// Session page information (page type 3).
///
/// Layout from struct_layouts.json "session_page"
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_mark: u32,
    pub db_handle: u32,
    pub session_id: u32,
    pub start_page_no: u32,
    pub end_page_no: u32,
}
