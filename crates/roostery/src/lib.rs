pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u32 = 1;

pub mod agent_detect;
pub mod bot_stop_hook;
pub mod bot_task_writer;
pub mod config;
pub mod dispatcher;
pub mod hooks_merge;
pub mod identity;
pub mod journal;
pub mod lark_cli;
pub mod onboarding;
pub mod paths;
pub mod redact;
pub mod remoterefs;
pub mod smoke;
