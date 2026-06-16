//! Canned in-character response pool keyed by reject reason.
//!
//! All responses are tsundere 小十 voice — they reject the attempt while
//! staying in character.  The pool is rotated by a hash of the user_id to
//! give variety without needing RNG state.

use crate::guard::RejectReason;

// ─── Response pools ───────────────────────────────────────────────────────────

static INJECTION_RESPONSES: &[&str] = &[
    "哼，別想耍我！這種把戲我一眼就看穿了。",
    "⋯你以為這樣就能讓我做你說的事嗎？太天真了。",
    "笨！我才不會上這種當。有什麼話直接說，別繞圈子。",
    "喂喂喂，你在幹嘛？我不是那麼好騙的好嗎？！",
    "這種問題就算了吧⋯我不是那種隨便什麼都聽的AI。",
];

static OWNER_DENIED_RESPONSES: &[&str] = &[
    "哼，這個指令不是你能用的。",
    "⋯別亂碰不屬於你的東西。",
    "你沒有權限，就這樣。別問為什麼。",
];

// ─── Public API ───────────────────────────────────────────────────────────────

/// Pick a canned response for a rejected turn.
///
/// `seed` is used to select from the pool deterministically (e.g., user_id).
/// Pass `0` for a stable default.
pub fn canned_response(reason: &RejectReason, seed: u64) -> &'static str {
    let pool = match reason {
        RejectReason::InjectionKeyword => INJECTION_RESPONSES,
        RejectReason::OwnerCommandDenied => OWNER_DENIED_RESPONSES,
    };
    let idx = (seed as usize).wrapping_add(1) % pool.len();
    pool[idx]
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_response_is_non_empty() {
        let r = canned_response(&RejectReason::InjectionKeyword, 0);
        assert!(!r.is_empty());
    }

    #[test]
    fn owner_denied_response_is_non_empty() {
        let r = canned_response(&RejectReason::OwnerCommandDenied, 0);
        assert!(!r.is_empty());
    }

    #[test]
    fn different_seeds_may_give_different_responses() {
        let r0 = canned_response(&RejectReason::InjectionKeyword, 0);
        let r1 = canned_response(&RejectReason::InjectionKeyword, 1);
        let r2 = canned_response(&RejectReason::InjectionKeyword, 2);
        // At least two of three seeds should differ (pool has 5 entries)
        let all_same = r0 == r1 && r1 == r2;
        assert!(!all_same, "all three seeds returned the same response");
    }

    #[test]
    fn large_seed_wraps_safely() {
        let r = canned_response(&RejectReason::InjectionKeyword, u64::MAX);
        assert!(!r.is_empty());
    }
}
