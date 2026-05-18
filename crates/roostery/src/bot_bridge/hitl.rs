//! `bot_bridge::hitl` — HitlDecision 三态分类 + abort/adjust 关键词常量。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（HitlDecision / ABORT_KEYWORDS / ADJUST_PREFIXES / classify）+ §3 N* 验收契约。
//!
//! 本模块只做"文本 → 判定"的纯计算，不做任何副作用（无 os::kill / 无 channel send）。
//! 副作用在 `daemon.rs` 拿到 `HitlDecision::Abort/Adjust` 后转 `HitlSignal` 走 oneshot。
//!
//! 来源：legacy/python/src/roostery/hitl_router.py（参考，不维护）。

/// HITL 判定三态（design §2.1 + §0 术语表）。
///
/// - `Abort`  ：用户要求立刻中止当前 runner（matched `/stop` / `/abort` / `停` / `中止`）
/// - `Adjust` ：用户要求带新指令重启 runner（`/adjust <body>` / `调整 <body>`，body 必须非空）
/// - `Pass`   ：未命中任何 HITL 关键词；事件继续走 `event_matches_bot` 常规分流
///
/// `non_exhaustive` 锁定外部 match 必须带 `_` 兜底，便于未来加新态而不破坏 caller。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HitlDecision {
    Abort { reason: String },
    Adjust { body: String },
    Pass,
}

/// abort 关键词全集（design §2.1 写死，不开放给用户改）。
///
/// 整文本 trim 后必须**完全等于**列表中某一个才算命中——避免 "请暂停下" 被误识别。
pub const ABORT_KEYWORDS: &[&str] = &["/stop", "/abort", "停", "中止"];

/// adjust 前缀全集（design §2.1 写死）。
///
/// 命中前缀后剩余部分作为 `body`；body trim 后为空 → 退化为 `Pass`（design §2.1 注释）。
pub const ADJUST_PREFIXES: &[&str] = &["/adjust ", "/adjust\n", "调整 ", "调整\n"];

/// 把 IM 消息正文文本判定为 HITL 三态之一。
///
/// 流程：
/// 1. 整文本 trim 命中 `ABORT_KEYWORDS` 任一 → `Abort{ reason = 原始 trimmed 文本 }`
/// 2. 文本以 `ADJUST_PREFIXES` 任一开头 →
///    - 剥前缀后 trim 非空 → `Adjust{ body = trim 后正文 }`
///    - 剥前缀后 trim 为空 → `Pass`（空 adjust 不动作）
/// 3. 其他 → `Pass`
pub fn classify(content: &str) -> HitlDecision {
    let trimmed = content.trim();

    // abort：整段精确匹配
    for kw in ABORT_KEYWORDS {
        if trimmed == *kw {
            return HitlDecision::Abort {
                reason: trimmed.to_string(),
            };
        }
    }

    // adjust：前缀匹配（用原始 content 取 body，避免 trim 误吞 body 内有意义空白）
    for prefix in ADJUST_PREFIXES {
        if let Some(rest) = content.strip_prefix(prefix) {
            let body = rest.trim();
            if body.is_empty() {
                return HitlDecision::Pass;
            }
            return HitlDecision::Adjust {
                body: body.to_string(),
            };
        }
    }

    HitlDecision::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_slash_stop_classifies_as_abort() {
        match classify("/stop") {
            HitlDecision::Abort { reason } => assert_eq!(reason, "/stop"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn abort_slash_abort_classifies_as_abort() {
        match classify("/abort") {
            HitlDecision::Abort { reason } => assert_eq!(reason, "/abort"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn abort_chinese_ting_classifies_as_abort() {
        match classify("停") {
            HitlDecision::Abort { reason } => assert_eq!(reason, "停"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn abort_chinese_zhongzhi_classifies_as_abort() {
        match classify("中止") {
            HitlDecision::Abort { reason } => assert_eq!(reason, "中止"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn adjust_slash_space_prefix_classifies_as_adjust() {
        match classify("/adjust use rust 2024 edition") {
            HitlDecision::Adjust { body } => assert_eq!(body, "use rust 2024 edition"),
            other => panic!("expected Adjust, got {other:?}"),
        }
    }

    #[test]
    fn adjust_slash_newline_prefix_classifies_as_adjust() {
        match classify("/adjust\nmulti\nline body") {
            HitlDecision::Adjust { body } => assert_eq!(body, "multi\nline body"),
            other => panic!("expected Adjust, got {other:?}"),
        }
    }

    #[test]
    fn adjust_chinese_space_prefix_classifies_as_adjust() {
        match classify("调整 改用 sqlite") {
            HitlDecision::Adjust { body } => assert_eq!(body, "改用 sqlite"),
            other => panic!("expected Adjust, got {other:?}"),
        }
    }

    #[test]
    fn adjust_chinese_newline_prefix_classifies_as_adjust() {
        match classify("调整\n切换数据库") {
            HitlDecision::Adjust { body } => assert_eq!(body, "切换数据库"),
            other => panic!("expected Adjust, got {other:?}"),
        }
    }

    #[test]
    fn adjust_empty_body_falls_back_to_pass() {
        // /adjust + 空白 body → Pass（不动作）
        assert_eq!(classify("/adjust "), HitlDecision::Pass);
        assert_eq!(classify("/adjust \n  \t  "), HitlDecision::Pass);
        assert_eq!(classify("调整 "), HitlDecision::Pass);
        assert_eq!(classify("调整\n   "), HitlDecision::Pass);
    }

    #[test]
    fn non_matching_text_is_pass() {
        assert_eq!(classify("hello world"), HitlDecision::Pass);
        assert_eq!(classify("@tl please look at this"), HitlDecision::Pass);
        // "请停一下" 不应命中 abort（非整段精确匹配）
        assert_eq!(classify("请停一下"), HitlDecision::Pass);
        // "/stopping" 不应命中 abort
        assert_eq!(classify("/stopping"), HitlDecision::Pass);
    }

    #[test]
    fn abort_keywords_list_has_exactly_four_entries() {
        assert_eq!(ABORT_KEYWORDS.len(), 4);
        assert_eq!(
            ABORT_KEYWORDS,
            &["/stop", "/abort", "停", "中止"],
            "design §2.1 写死的 4 个 abort 关键词必须与此列表一致"
        );
    }

    #[test]
    fn adjust_prefixes_list_has_exactly_four_entries() {
        assert_eq!(ADJUST_PREFIXES.len(), 4);
        assert_eq!(
            ADJUST_PREFIXES,
            &["/adjust ", "/adjust\n", "调整 ", "调整\n"],
            "design §2.1 写死的 4 个 adjust 前缀必须与此列表一致"
        );
    }
}
