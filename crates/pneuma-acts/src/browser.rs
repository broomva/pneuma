//! Browser-domain acts. v0.2 ships a single act: `browser.navigate`.
//!
//! This is the seed of the `browser.*` namespace. Step #13 of the MIL
//! build order (`MIL-PROJECT.md` §11.2) — first real OS-control act,
//! the act that unblocks Tier 3 (empirical user testing on real
//! workflows).
//!
//! ## What's here
//!
//! | Act               | Slot   | Reversibility | Reverse hint            |
//! |-------------------|--------|---------------|-------------------------|
//! | `browser.navigate`| `url`  | Costly        | `navigate_to_prior_url` |
//!
//! ## Why only one act
//!
//! Per the prior-art evaluation in
//! `research/entities/project/browser-use-ecosystem.md`, the right
//! v0.2 stance is: ship `navigate` via deterministic AppleScript, not
//! via an LLM-driven harness. Future browser acts (`browser.click_at`,
//! `browser.fill_form`, `browser.scroll_to`) belong here when the
//! `browser.*` namespace grows past `navigate`. At that point
//! `browser-use/browser-harness-js` (typed CDP wrapper, 652 wrappers
//! across 56 domains) becomes the right reference for the slot
//! signatures.
//!
//! ## Slot kind
//!
//! `url` is a `Referent::Url(String)`, not a plain `String`. URLs are
//! first-class referents in MIL — see `MIL-PROJECT.md` §6 — because the
//! workspace observers can resolve "this", "that", "the focused tab"
//! into a concrete URL the same way they resolve "the focused file"
//! into a `FileRef`. v0.2 callers always supply a literal URL; v0.3+
//! deictics will be resolved by `pneuma-resolver` before binding.

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, req_referent};

/// Browser-domain acts. v0.2 ships only `browser.navigate`.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // browser.navigate — point the frontmost browser tab at a URL.
        // Costly because we cannot guarantee the prior URL is recoverable
        // (the tab may close, the back-stack may be cleared); the bridge
        // captures the prior URL at execution time and routes reverse
        // through that capture, but the bridge can refuse if conditions
        // changed. Local blast — the user's own browser, no network
        // side effects beyond loading the page.
        act(
            "browser.navigate",
            vec![req_referent(
                "url",
                ReferentType::Url,
                "URL the frontmost browser tab will be navigated to",
            )],
            Reversibility::Costly,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("navigate_to_prior_url"),
            "Navigate the frontmost browser tab to a URL.",
        ),
    ]
}
