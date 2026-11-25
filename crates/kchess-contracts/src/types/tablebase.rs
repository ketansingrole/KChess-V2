//! The offline endgame tablebases (`tablebaseStatus`, `tablebaseInstall`, `tablebase:progress`):
//! the Syzygy tables of every position with up to five pieces, which answer tablebase lookups on
//! this computer.

use super::*;

union!(#[doc = "How far the tablebases' download is."] TablebasePhase {
    Downloading = "downloading",
    Done = "done",
    Cancelled = "cancelled",
    Failed = "failed",
});

impl TablebasePhase {
    /// The last report of a download: none follows it.
    pub fn is_terminal(self) -> bool {
        !matches!(self, TablebasePhase::Downloading)
    }
}

/// The tablebases' download in progress (or finished), streamed to every window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TablebaseProgress {
    pub phase: TablebasePhase,
    /// The bytes on disk so far, the files kept from an earlier download included.
    pub received: u64,
    pub total: u64,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// What the profile holds of the tablebases.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TablebaseStatus {
    /// Every table is on disk.
    pub installed: bool,
    /// The tables on disk, and all there are.
    pub files: u32,
    pub total_files: u32,
    /// Their size on disk, and the size of them all.
    pub bytes: u64,
    pub total_bytes: u64,
    /// The most pieces the tables cover.
    pub pieces: u32,
    /// True while a download runs; its progress arrives as events.
    pub busy: bool,
}
