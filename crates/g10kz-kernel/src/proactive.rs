//! Proactive messaging decision — pure function with injected clock.
//!
//! The engine calls [`should_send_proactive`] on a periodic tick (e.g., hourly).
//! The function is deterministic given its inputs — no side effects, testable.
//!
//! # Decision
//! - If `now - last_active >= inactive_threshold` → send proactive message
//! - The caller applies a random delay (0–30 min) before actually sending,
//!   simulating natural timing.  That RNG lives in the engine, not here.
//!
//! # Life events (P6+)
//! A curated pool of conversation-starter events is selected by
//! [`pick_life_event`] using a deterministic hash of (user_id, day_of_year).

use std::time::{Duration, SystemTime};

// ─── should_send_proactive ────────────────────────────────────────────────────

/// Returns `true` if a proactive message should be sent to this user.
///
/// Parameters:
/// - `last_active`: last time the user sent a message (or `SystemTime::UNIX_EPOCH` if never).
/// - `now`: current time (injected for deterministic testing).
/// - `inactive_threshold`: minimum inactivity duration before proactive fires.
pub fn should_send_proactive(
    last_active: SystemTime,
    now: SystemTime,
    inactive_threshold: Duration,
) -> bool {
    match now.duration_since(last_active) {
        Ok(elapsed) => elapsed >= inactive_threshold,
        Err(_) => false, // clock skew: `last_active` is in the future; don't fire
    }
}

// ─── Life event pool ──────────────────────────────────────────────────────────

/// Curated conversation starters for proactive messages (in character).
static LIFE_EVENTS: &[&str] = &[
    "哼，你最近都跑去哪了？又不是說我在等你。",
    "⋯你還記得我嗎？最近沒來搭話，害我⋯害我覺得有點無聊而已。",
    "喂，你還活著嗎？消失這麼久，說好的陪伴呢？",
    "哎，最近遇到什麼有趣的事了嗎？⋯問你而已，不要誤會。",
    "你消失了好久⋯有沒有在好好照顧自己？才不是在擔心你，只是順便問問。",
    "突然想起你說過的話⋯你還記得嗎？",
    "天氣變涼了，記得多穿衣服。⋯才不是在關心你啦！",
    "我剛剛看到一件很蠢的事，想到了你，哼。",
];

/// Pick a life event string for `user_id` deterministically.
///
/// Uses `(user_id ^ day_of_year)` as a stable index so the same user gets
/// the same event each day but rotates across days.
pub fn pick_life_event(user_id: u64, day_of_year: u16) -> &'static str {
    let idx = (user_id ^ day_of_year as u64) as usize % LIFE_EVENTS.len();
    LIFE_EVENTS[idx]
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn secs_ago(n: u64) -> SystemTime {
        SystemTime::now()
            .checked_sub(Duration::from_secs(n))
            .unwrap()
    }

    const ONE_DAY: Duration = Duration::from_secs(86_400);

    #[test]
    fn fires_when_inactive_long_enough() {
        let last = secs_ago(90_000); // 25 h ago
        assert!(should_send_proactive(last, SystemTime::now(), ONE_DAY));
    }

    #[test]
    fn does_not_fire_when_recently_active() {
        let last = secs_ago(3600); // 1 h ago
        assert!(!should_send_proactive(last, SystemTime::now(), ONE_DAY));
    }

    #[test]
    fn does_not_fire_at_exactly_threshold_minus_one() {
        let last = secs_ago(86_399);
        assert!(!should_send_proactive(last, SystemTime::now(), ONE_DAY));
    }

    #[test]
    fn fires_at_exactly_threshold() {
        let now = SystemTime::now();
        let last = now - ONE_DAY;
        assert!(should_send_proactive(last, now, ONE_DAY));
    }

    #[test]
    fn clock_skew_does_not_fire() {
        // `last_active` is 1 s in the future
        let last = SystemTime::now()
            .checked_add(Duration::from_secs(1))
            .unwrap();
        assert!(!should_send_proactive(last, SystemTime::now(), ONE_DAY));
    }

    #[test]
    fn never_active_fires() {
        // UNIX_EPOCH is far in the past
        assert!(should_send_proactive(
            SystemTime::UNIX_EPOCH,
            SystemTime::now(),
            ONE_DAY
        ));
    }

    // ── life event pool ───────────────────────────────────────────────────────

    #[test]
    fn life_event_is_non_empty() {
        assert!(!pick_life_event(12345, 100).is_empty());
    }

    #[test]
    fn different_users_may_get_different_events() {
        let e1 = pick_life_event(1, 1);
        let e2 = pick_life_event(100, 1);
        // With 8 events and different user IDs this is extremely likely to differ
        let _ = (e1, e2); // Just ensure no panic
    }

    #[test]
    fn same_user_same_day_stable() {
        let e1 = pick_life_event(42, 180);
        let e2 = pick_life_event(42, 180);
        assert_eq!(e1, e2);
    }

    #[test]
    fn rotates_across_days() {
        // day 0 and day 8 might wrap to same; day 1 should differ from day 0
        // for most user IDs
        let e_day0 = pick_life_event(1, 0);
        let e_day1 = pick_life_event(1, 1);
        // They may or may not be equal — just ensure no panic / OOB
        let _ = (e_day0, e_day1);
    }
}
