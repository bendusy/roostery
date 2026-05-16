pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u32 = 1;

pub mod journal;
pub mod lark_cli;
pub mod paths;
pub mod redact;
pub mod remoterefs;
