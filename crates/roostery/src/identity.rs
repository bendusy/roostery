//! Identity reflection from `lark-cli` profile.
//!
//! **Roostery does not invent identity.** This module reflects the active
//! `lark-cli` profile + auth state via the [`LarkRunner`] trait into an
//! immutable [`Identity`] snapshot used for human-readable summaries
//! (`roostery init` report, future task summaries). Per project hard
//! constraint, every Feishu API call goes through `lark-cli`; no direct
//! HTTP / SDK.
//!
//! See `.codestable/features/2026-05-18-roostery-init/roostery-init-design.md`
//! §2.1.1.

use crate::lark_cli::{LarkError, LarkRunner};
use gethostname::gethostname;
use thiserror::Error;

/// Immutable snapshot of the active lark-cli identity at one point in time.
///
/// Fields are private to enforce typestate-lite (idiom #4): callers go
/// through accessors that return `Option<&str>` rather than touching
/// `Option<String>` directly. `host` is always present (hostname fallback to
/// `"unknown"`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Identity {
    profile_name: Option<String>,
    user_open_id: Option<String>,
    user_name: Option<String>,
    bot_app_id: Option<String>,
    brand: Option<String>,
    token_status: Option<String>,
    host: String,
}

impl Identity {
    pub fn profile_name(&self) -> Option<&str> {
        self.profile_name.as_deref()
    }
    pub fn user_open_id(&self) -> Option<&str> {
        self.user_open_id.as_deref()
    }
    pub fn user_name(&self) -> Option<&str> {
        self.user_name.as_deref()
    }
    pub fn bot_app_id(&self) -> Option<&str> {
        self.bot_app_id.as_deref()
    }
    pub fn brand(&self) -> Option<&str> {
        self.brand.as_deref()
    }
    pub fn token_status(&self) -> Option<&str> {
        self.token_status.as_deref()
    }
    pub fn host(&self) -> &str {
        &self.host
    }

    /// `user_name` first, else open_id last 6 chars, else `"anon"`.
    pub fn short_user(&self) -> &str {
        if let Some(n) = self.user_name.as_deref() {
            return n;
        }
        if let Some(oid) = self.user_open_id.as_deref()
            && oid.len() >= 6
        {
            return &oid[oid.len() - 6..];
        }
        "anon"
    }

    /// `cli_xxxxxxxx` 8-char suffix after `cli_` prefix, else first 8 chars,
    /// else `"no-bot"`.
    pub fn short_bot(&self) -> &str {
        if let Some(app) = self.bot_app_id.as_deref() {
            if let Some(rest) = app.strip_prefix("cli_") {
                return &rest[..rest.len().min(8)];
            }
            return &app[..app.len().min(8)];
        }
        "no-bot"
    }

    /// Token valid and both user/bot identifiers present.
    pub fn is_ready(&self) -> bool {
        self.user_open_id.is_some()
            && self.bot_app_id.is_some()
            && self.token_status.as_deref() == Some("valid")
    }

    /// Single-line human-readable summary.
    pub fn describe(&self) -> String {
        let mark = if self.is_ready() { "✓" } else { "✗" };
        format!(
            "{mark} profile={profile} user={short_user} ({user_open_id}) \
             bot={short_bot} ({bot_app_id}) host={host} token={token}",
            profile = self.profile_name.as_deref().unwrap_or("?"),
            short_user = self.short_user(),
            user_open_id = self.user_open_id.as_deref().unwrap_or("-"),
            short_bot = self.short_bot(),
            bot_app_id = self.bot_app_id.as_deref().unwrap_or("-"),
            host = self.host,
            token = self.token_status.as_deref().unwrap_or("-"),
        )
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IdentityError {
    #[error("lark-cli auth status failed: {source}")]
    AuthStatusFailed {
        #[source]
        source: LarkError,
    },
    #[error("lark-cli profile list failed: {source}")]
    ProfileListFailed {
        #[source]
        source: LarkError,
    },
}

/// Resolve the active identity via [`LarkRunner`]. See module-level docs.
pub async fn current(runner: &dyn LarkRunner) -> Result<Identity, IdentityError> {
    let auth = runner
        .run(&["auth", "status"])
        .await
        .map_err(|source| IdentityError::AuthStatusFailed { source })?;
    let profiles = runner
        .run(&["profile", "list"])
        .await
        .map_err(|source| IdentityError::ProfileListFailed { source })?;

    let take_str = |k: &str| -> Option<String> {
        auth.get(k)
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
    };

    let profile_name = profiles.as_array().and_then(|arr| {
        arr.iter()
            .find(|p| p.get("active").and_then(|v| v.as_bool()).unwrap_or(false))
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
    });

    Ok(Identity {
        profile_name,
        user_open_id: take_str("userOpenId"),
        user_name: take_str("userName"),
        bot_app_id: take_str("appId"),
        brand: take_str("brand"),
        token_status: take_str("tokenStatus"),
        host: detect_host(),
    })
}

fn detect_host() -> String {
    let raw = gethostname();
    let s = raw.to_string_lossy();
    s.split('.').next().unwrap_or("unknown").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        profile: Option<&str>,
        open_id: Option<&str>,
        name: Option<&str>,
        bot: Option<&str>,
        token: Option<&str>,
    ) -> Identity {
        Identity {
            profile_name: profile.map(String::from),
            user_open_id: open_id.map(String::from),
            user_name: name.map(String::from),
            bot_app_id: bot.map(String::from),
            brand: None,
            token_status: token.map(String::from),
            host: "mac-test".to_string(),
        }
    }

    #[test]
    fn short_user_prefers_name_then_oid_tail_then_anon() {
        assert_eq!(
            build(None, Some("ou_abcdef123456"), Some("Ben"), None, None).short_user(),
            "Ben"
        );
        assert_eq!(
            build(None, Some("ou_abcdef123456"), None, None, None).short_user(),
            "123456"
        );
        assert_eq!(build(None, None, None, None, None).short_user(), "anon");
    }

    #[test]
    fn short_bot_strips_cli_prefix_then_truncates() {
        assert_eq!(
            build(None, None, None, Some("cli_abcdef1234"), None).short_bot(),
            "abcdef12"
        );
        assert_eq!(
            build(None, None, None, Some("noncli_xxx"), None).short_bot(),
            "noncli_x"
        );
        assert_eq!(build(None, None, None, None, None).short_bot(), "no-bot");
    }

    #[test]
    fn is_ready_requires_user_bot_and_valid_token() {
        assert!(build(None, Some("ou"), None, Some("cli"), Some("valid")).is_ready());
        assert!(!build(None, Some("ou"), None, Some("cli"), Some("expired")).is_ready());
        assert!(!build(None, None, None, Some("cli"), Some("valid")).is_ready());
        assert!(!build(None, Some("ou"), None, None, Some("valid")).is_ready());
    }

    #[test]
    fn describe_includes_all_fields() {
        let i = build(
            Some("default"),
            Some("ou_xxxxxxxxxxxx"),
            Some("Ben"),
            Some("cli_app00112233"),
            Some("valid"),
        );
        let s = i.describe();
        assert!(s.contains("✓"));
        assert!(s.contains("profile=default"));
        assert!(s.contains("user=Ben"));
        assert!(s.contains("bot=app00112"));
        assert!(s.contains("host=mac-test"));
        assert!(s.contains("token=valid"));
    }
}
