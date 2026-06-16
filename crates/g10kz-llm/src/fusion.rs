//! Fusion multi-model synthesis.
//!
//! Fan-out → quorum/timeout → consensus short-circuit → judge.

use std::collections::HashSet;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use tracing::{debug, warn};

use crate::{
    provider::Provider,
    types::{CompletionParams, Message, Role, Usage},
    LlmError,
};

// ─── FusionConfig ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FusionConfig {
    pub drafter_models: Vec<String>,
    pub judge_model: String,
    pub quorum: usize,
    pub timeout_ms: u64,
    pub consensus_threshold: f32,
}

impl FusionConfig {
    pub fn reason_defaults(drafter_models: Vec<String>, judge_model: String) -> Self {
        Self { drafter_models, judge_model, quorum: 2, timeout_ms: 8_000, consensus_threshold: 0.82 }
    }
}

// ─── Consensus ────────────────────────────────────────────────────────────────

pub fn jaccard_similarity(a: &str, b: &str) -> f32 {
    let wa: HashSet<&str> = a.split_whitespace().collect();
    let wb: HashSet<&str> = b.split_whitespace().collect();
    if wa.is_empty() && wb.is_empty() { return 1.0; }
    let inter = wa.intersection(&wb).count();
    let union = wa.len() + wb.len() - inter;
    if union == 0 { return 1.0; }
    inter as f32 / union as f32
}

pub fn all_drafts_agree(drafts: &[String], threshold: f32) -> bool {
    if drafts.len() <= 1 { return true; }
    for i in 0..drafts.len() {
        for j in (i + 1)..drafts.len() {
            if jaccard_similarity(&drafts[i].to_lowercase(), &drafts[j].to_lowercase()) < threshold {
                return false;
            }
        }
    }
    true
}

// ─── Judge prompt ─────────────────────────────────────────────────────────────

fn build_judge_prompt(drafts: &[String], messages: &[Message]) -> Vec<Message> {
    let user_query = messages.iter().rev()
        .find(|m| matches!(m.role, Role::User))
        .map(|m| m.text_content()).unwrap_or_default();

    let drafts_text: String = drafts.iter().enumerate()
        .map(|(i, d)| format!("[草稿 {}]\n{}\n", (b'A' + i as u8) as char, d.trim()))
        .collect::<Vec<_>>().join("\n");

    vec![
        Message::text(Role::System,
            "你是評審，從多份匿名回覆中合成最佳答案。直接輸出，不解釋判斷過程。"),
        Message::text(Role::User,
            format!("用戶問：{user_query}\n\n{drafts_text}\n請合成最佳回覆：")),
    ]
}

// ─── fusion_complete ──────────────────────────────────────────────────────────

pub async fn fusion_complete(
    provider: &dyn Provider,
    messages: &[Message],
    _base_params: &CompletionParams,
    fusion: &FusionConfig,
) -> anyhow::Result<(String, Usage)> {
    if fusion.drafter_models.is_empty() {
        return Err(LlmError::Request("no drafter models".into()).into());
    }

    // 1. Fan-out — pre-compute params so futures don't hold refs to temporaries
    let drafter_params: Vec<CompletionParams> = fusion.drafter_models.iter()
        .map(|m| CompletionParams::reason(m))
        .collect();
    let mut futs: FuturesUnordered<_> = drafter_params.iter()
        .map(|p| provider.complete(messages, p))
        .collect();

    // 2. Quorum / timeout
    let mut drafts: Vec<String> = Vec::new();
    let mut total = Usage::default();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(fusion.timeout_ms);

    loop {
        if drafts.len() >= fusion.quorum { break; }
        if futs.is_empty() { break; }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::select! {
            Some(result) = futs.next() => {
                match result {
                    Ok((text, u)) if !text.trim().is_empty() => {
                        drafts.push(text);
                        total.prompt_tokens += u.prompt_tokens;
                        total.completion_tokens += u.completion_tokens;
                        total.cost_usd += u.cost_usd;
                    }
                    Ok(_) => {}
                    Err(e) => warn!("drafter failed: {e}"),
                }
            }
            _ = tokio::time::sleep(remaining) => {
                debug!("fusion timeout, {} drafts", drafts.len());
                break;
            }
        }
    }

    if drafts.is_empty() {
        return Err(LlmError::Exhausted.into());
    }

    if drafts.len() == 1 {
        return Ok((drafts.remove(0), total));
    }

    // 3. Consensus short-circuit
    if all_drafts_agree(&drafts, fusion.consensus_threshold) {
        debug!("fusion: consensus, skipping judge");
        let best = drafts.into_iter().max_by_key(|d| d.len()).unwrap();
        return Ok((best, total));
    }

    // 4. Judge
    debug!(judge = %fusion.judge_model, "fusion: judge synthesis");
    let judge_msgs = build_judge_prompt(&drafts, messages);
    let judge_params = CompletionParams::judge(&fusion.judge_model);

    match provider.complete(&judge_msgs, &judge_params).await {
        Ok((text, u)) => {
            total.prompt_tokens += u.prompt_tokens;
            total.completion_tokens += u.completion_tokens;
            total.cost_usd += u.cost_usd;
            Ok((text, total))
        }
        Err(e) => {
            warn!("judge failed ({e}), falling back to longest draft");
            let best = drafts.into_iter().max_by_key(|d| d.len()).unwrap();
            Ok((best, total))
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mock::MockProvider, types::Role};

    fn msgs() -> Vec<Message> {
        vec![Message::text(Role::System, "你是小十"), Message::text(Role::User, "分析量子纏繞")]
    }

    #[test]
    fn jaccard_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn jaccard_disjoint() {
        assert!(jaccard_similarity("hello world", "foo bar") < 0.01);
    }

    #[test]
    fn jaccard_partial() {
        let s = jaccard_similarity("hello world", "hello universe");
        assert!((s - 1.0 / 3.0).abs() < 0.01, "got {s}");
    }

    #[test]
    fn jaccard_empty() {
        assert!((jaccard_similarity("", "") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn agree_identical() {
        assert!(all_drafts_agree(&["abc def".into(), "abc def".into()], 0.9));
    }

    #[test]
    fn disagree_different() {
        assert!(!all_drafts_agree(&[
            "quantum entanglement alpha beta gamma".into(),
            "蘋果橘子香蕉草莓藍莓桃子梨子".into(),
        ], 0.5));
    }

    #[test]
    fn single_draft_agrees() {
        assert!(all_drafts_agree(&["any reply".into()], 1.0));
    }

    #[test]
    fn empty_drafts_agree() {
        assert!(all_drafts_agree(&[], 1.0));
    }

    #[tokio::test]
    async fn consensus_returns_without_judge() {
        // Same reply for both drafters → consensus → no judge (3rd call never happens)
        let provider = MockProvider::new(vec![
            "quantum entanglement interesting physics".into(),
            "quantum entanglement interesting physics".into(),
            "JUDGE OUTPUT".into(), // would only be called if judge runs
        ]);
        let fusion = FusionConfig {
            drafter_models: vec!["m-a".into(), "m-b".into()],
            judge_model: "m-j".into(),
            quorum: 2,
            timeout_ms: 5_000,
            consensus_threshold: 0.85,
        };
        let (reply, _) = fusion_complete(&provider, &msgs(), &CompletionParams::reason("m"), &fusion)
            .await.unwrap();
        // consensus path: reply is one of the identical drafts, not judge output
        assert_ne!(reply, "JUDGE OUTPUT", "judge should not have been called");
    }

    #[tokio::test]
    async fn divergent_calls_judge() {
        let provider = MockProvider::new(vec![
            "quantum alpha beta gamma delta epsilon zeta".into(),
            "完全不同的中文回覆沒有任何英文單詞交集".into(),
            "judge synthesised output".into(),
        ]);
        let fusion = FusionConfig {
            drafter_models: vec!["m-a".into(), "m-b".into()],
            judge_model: "m-j".into(),
            quorum: 2,
            timeout_ms: 5_000,
            consensus_threshold: 0.85,
        };
        let (reply, _) = fusion_complete(&provider, &msgs(), &CompletionParams::reason("m"), &fusion)
            .await.unwrap();
        assert_eq!(reply, "judge synthesised output");
    }

    #[tokio::test]
    async fn single_drafter_skips_judge() {
        let provider = MockProvider::with_reply("only drafter");
        let fusion = FusionConfig {
            drafter_models: vec!["m-only".into()],
            judge_model: "m-j".into(),
            quorum: 1,
            timeout_ms: 5_000,
            consensus_threshold: 0.85,
        };
        let (reply, _) = fusion_complete(&provider, &msgs(), &CompletionParams::reason("m"), &fusion)
            .await.unwrap();
        assert_eq!(reply, "only drafter");
    }

    #[tokio::test]
    async fn exhausted_when_all_fail() {
        struct FailProvider;
        impl Provider for FailProvider {
            fn complete<'a>(&'a self, _: &'a [Message], _: &'a CompletionParams)
                -> crate::provider::BoxFuture<'a, anyhow::Result<(String, Usage)>> {
                Box::pin(async { Err(anyhow::anyhow!("fail")) })
            }
        }
        let fusion = FusionConfig {
            drafter_models: vec!["m-a".into()],
            judge_model: "m-j".into(),
            quorum: 1,
            timeout_ms: 1_000,
            consensus_threshold: 0.85,
        };
        let r = fusion_complete(&FailProvider, &msgs(), &CompletionParams::reason("m"), &fusion).await;
        assert!(r.is_err());
    }

    #[test]
    fn judge_prompt_has_draft_labels() {
        let drafts = vec!["draft one".into(), "draft two".into()];
        let messages = vec![Message::text(Role::User, "user question")];
        let j = build_judge_prompt(&drafts, &messages);
        let content = j.last().unwrap().text_content();
        assert!(content.contains("[草稿 A]"), "got: {content}");
        assert!(content.contains("[草稿 B]"), "got: {content}");
    }

    #[test]
    fn judge_prompt_includes_user_query() {
        let drafts = vec!["d".into()];
        let messages = vec![Message::text(Role::User, "specific question here")];
        let j = build_judge_prompt(&drafts, &messages);
        assert!(j.last().unwrap().text_content().contains("specific question here"));
    }
}
