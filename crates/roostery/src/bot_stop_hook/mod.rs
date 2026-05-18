//! bot_stop_hook — agent 工作进展推飞书的核心模块。
//!
//! Module F 第 2 子 feature（feature `2026-05-18-bot-stop-hook`），0.1.0 触发判据。
//!
//! 拆分自单文件 `bot_stop_hook.rs`（refactor `2026-05-19-bot-stop-hook-split`）。
//! 业务块按 audit `2026-05-18-post-release-rust-idiom` finding-02 的边界切分：
//!
//! - [`types`] — 核心类型 `PushRequest` / `PushOptions` / `PushOutcome` / `PushStatus`
//!   + 全局常量 (`DEFAULT_SUMMARY` / `SUMMARY_MAX_BYTES`)
//! - [`stop_input`] — CC/Codex/Gemini SessionEnd stdin JSON schema + transcript jsonl tail
//! - [`util`] — UTF-8 安全截断 / cwd basename / blake3 idem key / receive_id 三层链
//! - [`push`] — 核心业务流 `push` + IM 兜底 + `run_stop_hook` 入口
//! - [`cli`] — clap CLI args（`BotArgs / BotSub / PushCliArgs / StopHookCliArgs`）+ `run` dispatch
//!
//! **公开 API 字面兼容**（`bot_stop_hook::push` / `run_stop_hook` / `cli::run` 等），
//! 拆分仅为可维护性提升，行为等价。

pub mod cli;
pub mod push;
pub mod stop_input;
pub mod types;
pub(crate) mod util;

// 公开 API re-export — 保留 0.1.0 字面兼容
pub use push::{push, run_stop_hook};
pub use stop_input::StopHookInput;
pub use types::{PushOptions, PushOutcome, PushRequest, PushStatus};

#[cfg(test)]
pub(crate) mod test_helpers {
    //! 跨子模块共享的测试 fixtures。所有调用方需先持 [`crate::paths::TEST_ENV_LOCK`]。

    use serde_json::{Value, json};
    use std::path::Path;

    /// 装一个 tempdir 作 ROOSTERY_HOME，让 config::load() 从里面读。返 TempDir 给
    /// caller 持有保活（drop 时 tempdir 被清）。
    pub(crate) fn install_tempdir_as_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var("ROOSTERY_HOME", dir.path()) };
        dir
    }

    pub(crate) fn write_config_with_user_id(home: &Path, user_id: &str) {
        let cfg_path = home.join("config.yaml");
        let yaml = format!(
            "schema_version: 1\nidentity:\n  user_id: \"{user_id}\"\n  default_chat_id: \"\"\n  default_task_app_token: \"\"\n"
        );
        std::fs::write(cfg_path, yaml).expect("write config");
    }

    pub(crate) fn task_create_response() -> Value {
        json!({"ok": true, "data": {"guid": "task_abc", "url": "https://feishu.cn/task/abc"}})
    }

    pub(crate) fn im_send_response() -> Value {
        json!({"ok": true, "data": {"message_id": "om_xxx"}})
    }
}
