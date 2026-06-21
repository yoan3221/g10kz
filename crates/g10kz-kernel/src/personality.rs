//! JPAF — Jungian Personality Adaptation Framework (lightweight port).
//!
//! Tracks per-user activation of Jung's 8 cognitive functions and dynamically
//! modifies the system prompt to reflect the evolving user relationship.
//!
//! # Mechanism
//! - Fixed `BASE` weights encode the bot's core tsundere personality.
//! - Per-user `temp` weights drift based on inferred cognitive-function usage.
//! - After each Social turn, `classify_activation` heuristically picks the
//!   activated function, then `PersonalityState::update` decays temp and bumps it.
//! - `render_modifier` returns a short Traditional-Chinese annotation injected at
//!   the end of the system prompt once enough turns have accumulated.

use std::fmt;

// ─── JungFunction ─────────────────────────────────────────────────────────────

/// Jung's 8 cognitive functions, ordered by array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JungFunction {
    Ti = 0, // Introverted Thinking  — logic, analysis, defense
    Te = 1, // Extroverted Thinking  — structure, efficiency
    Fi = 2, // Introverted Feeling   — values, authenticity, trust
    Fe = 3, // Extroverted Feeling   — empathy, warmth, harmony
    Ni = 4, // Introverted Intuition — deep insight, symbolism
    Ne = 5, // Extroverted Intuition — curiosity, playfulness, ideas
    Si = 6, // Introverted Sensing   — memory, routine, nostalgia
    Se = 7, // Extroverted Sensing   — present-moment, action, fun
}

impl fmt::Display for JungFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

fn index_to_function(idx: usize) -> JungFunction {
    match idx {
        0 => JungFunction::Ti,
        1 => JungFunction::Te,
        2 => JungFunction::Fi,
        3 => JungFunction::Fe,
        4 => JungFunction::Ni,
        5 => JungFunction::Ne,
        6 => JungFunction::Si,
        7 => JungFunction::Se,
        _ => JungFunction::Ti,
    }
}

// ─── Base weights (tsundere) ──────────────────────────────────────────────────

/// Fixed g10kz base: Ti-dominant (defensive), Fe strong (hidden warmth),
/// Fi (internal values), Ne (playful curiosity).
///        Ti    Te    Fi    Fe    Ni    Ne    Si    Se
const BASE: [f32; 8] = [0.35, 0.01, 0.15, 0.25, 0.01, 0.15, 0.05, 0.03];

const DECAY: f32 = 0.90;
const BUMP: f32 = 0.08;
const MIN_TURNS: u32 = 3;
const DRIFT_THRESHOLD: f32 = 0.05;

// ─── PersonalityState ─────────────────────────────────────────────────────────

/// Per-user mutable personality state.
#[derive(Debug, Clone)]
pub struct PersonalityState {
    /// Temporary weight offsets accumulated from recent interactions.
    pub temp: [f32; 8],
    /// Total Social turns processed for this user.
    pub turn_count: u32,
}

impl Default for PersonalityState {
    fn default() -> Self {
        Self {
            temp: [0.0; 8],
            turn_count: 0,
        }
    }
}

impl PersonalityState {
    /// Effective weight per function = BASE + temp (clamped ≥ 0).
    pub fn effective(&self) -> [f32; 8] {
        let mut e = BASE;
        for (i, t) in self.temp.iter().enumerate() {
            e[i] = (e[i] + t).max(0.0);
        }
        e
    }

    /// Decay all temp weights then bump the activated function.
    pub fn update(&mut self, activated: JungFunction) {
        for t in self.temp.iter_mut() {
            *t *= DECAY;
        }
        self.temp[activated as usize] += BUMP;
        self.turn_count += 1;
    }

    /// Returns a Traditional-Chinese modifier to inject into the system prompt,
    /// or `None` when data is insufficient or drift is below threshold.
    pub fn render_modifier(&self) -> Option<String> {
        if self.turn_count < MIN_TURNS {
            return None;
        }

        let e = self.effective();
        let (idx, drift) = e
            .iter()
            .zip(BASE.iter())
            .enumerate()
            .map(|(i, (eff, b))| (i, eff - b))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())?;

        if drift < DRIFT_THRESHOLD {
            return None;
        }

        let text = modifier_text(index_to_function(idx));
        Some(format!("\n\n[人格動態]\n{text}"))
    }
}

// ─── Keyword classifier ───────────────────────────────────────────────────────

/// Heuristic: pick the most likely activated cognitive function from a turn.
/// No LLM call — O(n) keyword scan.
pub fn classify_activation(user_text: &str, bot_reply: &str) -> JungFunction {
    let combined = format!("{user_text} {bot_reply}").to_lowercase();
    let t = combined.as_str();

    let mut scores = [0i32; 8];

    // Fe — warmth, care, harmony
    for kw in [
        "謝謝",
        "感謝",
        "喜歡",
        "♡",
        "♥",
        "開心",
        "好可愛",
        "溫柔",
        "陪",
        "在乎",
        "關心",
    ] {
        if t.contains(kw) {
            scores[3] += 2;
        }
    }
    // Fi — values, trust, authenticity
    for kw in [
        "真心",
        "秘密",
        "信任",
        "心裡",
        "真正",
        "說實話",
        "內心",
        "心動",
    ] {
        if t.contains(kw) {
            scores[2] += 2;
        }
    }
    // Ti — logic, analysis, defense
    for kw in [
        "為什麼",
        "原理",
        "機制",
        "分析",
        "邏輯",
        "原因",
        "怎麼",
        "how",
        "why",
        "因為",
        "解釋",
    ] {
        if t.contains(kw) {
            scores[0] += 1;
        }
    }
    // Ne — curiosity, ideas, playful
    for kw in [
        "如果",
        "假如",
        "可能",
        "想到",
        "玩",
        "有趣",
        "哈哈",
        "hhh",
        "lol",
        "好主意",
        "奇怪",
    ] {
        if t.contains(kw) {
            scores[5] += 1;
        }
    }
    // Se — immediate, reactive
    for kw in [
        "現在", "馬上", "快", "wow", "哇", "剛剛", "啊啊", "喔喔", "衝",
    ] {
        if t.contains(kw) {
            scores[7] += 1;
        }
    }
    // Si — memory, routine, past
    for kw in ["記得", "以前", "上次", "習慣", "常常", "每次", "之前"] {
        if t.contains(kw) {
            scores[6] += 1;
        }
    }
    // Ni — intuition, insight
    for kw in ["感覺", "直覺", "預感", "象徵", "深層", "意義"] {
        if t.contains(kw) {
            scores[4] += 1;
        }
    }
    // Te — structure, efficiency
    for kw in ["效率", "計畫", "步驟", "目標", "完成", "整理"] {
        if t.contains(kw) {
            scores[1] += 1;
        }
    }

    let idx = scores
        .iter()
        .enumerate()
        .max_by_key(|(_, &s)| s)
        .map(|(i, _)| i)
        .unwrap_or(0);
    index_to_function(idx)
}

// ─── Modifier text ────────────────────────────────────────────────────────────

fn modifier_text(func: JungFunction) -> &'static str {
    match func {
        JungFunction::Fe => "與此用戶互動時，情感比平時更容易自然流露，傲嬌防線偶爾會不小心降低。",
        JungFunction::Fi => "與此用戶的互動積累了信任感，偶爾會說出比平時更真心的話。",
        JungFunction::Ti => "與此用戶互動時，傾向用邏輯和分析保持距離感，不輕易放鬆防備。",
        JungFunction::Ne => "與此用戶在一起時思維特別活躍，有時會脫口說出奇怪但有趣的想法。",
        JungFunction::Se => "與此用戶互動時，反應比平時更直接、更即興，少一點算計。",
        JungFunction::Si => "與此用戶有共同的記憶積累，有時會不自覺提到之前的對話。",
        JungFunction::Ni => "與此用戶的互動激發了更深層的思考，偶爾說話會帶點意味深長。",
        JungFunction::Te => "與此用戶互動時，比平時更注重給出有條理、有用的回覆。",
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_temp_is_zero() {
        let s = PersonalityState::default();
        assert!(s.temp.iter().all(|&t| t == 0.0));
        assert_eq!(s.turn_count, 0);
    }

    #[test]
    fn effective_matches_base_with_no_drift() {
        let s = PersonalityState::default();
        for (i, b) in BASE.iter().enumerate() {
            assert!((s.effective()[i] - b).abs() < 0.001);
        }
    }

    #[test]
    fn update_bumps_activated_and_increments_count() {
        let mut s = PersonalityState::default();
        s.update(JungFunction::Fe);
        assert!(s.temp[3] > 0.0);
        assert_eq!(s.turn_count, 1);
    }

    #[test]
    fn no_modifier_below_min_turns() {
        let mut s = PersonalityState::default();
        s.update(JungFunction::Fe);
        s.update(JungFunction::Fe);
        assert!(s.render_modifier().is_none());
    }

    #[test]
    fn modifier_appears_after_sufficient_drift() {
        let mut s = PersonalityState::default();
        for _ in 0..6 {
            s.update(JungFunction::Fe);
        }
        assert!(s.render_modifier().is_some());
        assert!(s.render_modifier().unwrap().contains("人格動態"));
    }

    #[test]
    fn classify_warm_text_returns_fe() {
        assert_eq!(
            classify_activation("謝謝你陪我", "不客氣啦"),
            JungFunction::Fe
        );
    }

    #[test]
    fn classify_analytical_text_returns_ti() {
        assert_eq!(
            classify_activation("解釋一下這個機制的原理", "分析如下"),
            JungFunction::Ti
        );
    }

    #[test]
    fn decay_reduces_temp() {
        let mut s = PersonalityState::default();
        s.update(JungFunction::Fe);
        let peak = s.temp[3];
        for _ in 0..15 {
            s.update(JungFunction::Ti);
        }
        assert!(s.temp[3] < peak * 0.3);
    }
}
