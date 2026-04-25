pub mod config;
pub mod dates;
pub mod db;
pub mod export;
pub mod frontmatter;
pub mod model;
pub mod notify;
pub mod output;
pub mod prefs;
pub mod search;
pub mod store;
pub mod verbose;

pub(crate) mod cli;
pub(crate) mod mcp;
pub(crate) mod runner;

pub use model::{Attachment, Note, PinRecord, Tag};
pub use store::SqliteStore;

pub fn run() -> anyhow::Result<()> {
    runner::run()
}
