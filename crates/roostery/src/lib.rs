pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u32 = 1;

pub mod journal;
pub mod paths;
pub mod redact;
