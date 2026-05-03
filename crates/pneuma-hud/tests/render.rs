//! HUD rendering tests.
//!
//! Properties:
//!
//! - Each render method returns the right `HudFrameKind`.
//! - Frame body contains the act id (sanity).
//! - Composing frame flags unbound required slots distinctly.
//! - Ready frame surfaces the policy summary + confidence.
//! - Proposed frame includes the ratify-window line when policy has one.
//! - Outcome frame surfaces the undo affordance per ReverseAction kind.
//! - Width parameter is honored within bounds.

use chrono::Utc;
use pneuma_acts::registry;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{
    BindingKind, Confidence, ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId,
    ContractError, Directive, FileRef, Modifier, PolicyEnvelope, Provenance, ReferentValue,
    ResolvedAct, ResolvedSlot, SpeechAct,
};
use pneuma_hud::{HudFrameKind, HudRenderer};
use pneuma_praxis_bridge::{ExecutionOutcome, PraxisError, ReverseAction};

// --- Helpers ---------------------------------------------------------------

fn rename_act() -> pneuma_core::Act {
    registry()
        .into_iter()
        .find(|a| a.id.as_str() == "file.rename")
        .unwrap()
}

fn det_provenance() -> Provenance {
    Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now())
}

fn build_composing_with_target_only() -> Directive<pneuma_core::Composing> {
    Directive::new(SpeechAct::Directive, ResolvedAct::empty(rename_act()))
        .bind_slot(
            ResolvedSlot::new(
                "target",
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
                det_provenance(),
            )
            .unwrap(),
        )
        .with_modifier(Modifier::carefulness(0.7).unwrap())
}

fn build_ready() -> Directive<pneuma_core::Ready> {
    let act = rename_act();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let confidence = Confidence::from_slots(vec![
        (
            "target".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
        (
            "new_name".to_owned(),
            ConfidenceScore::new(0.95, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
    ])
    .unwrap();
    Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(
            ResolvedSlot::new(
                "target",
                ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x.txt"))),
                det_provenance(),
            )
            .unwrap(),
        )
        .bind_slot(
            ResolvedSlot::new(
                "new_name",
                ResolvedSlotValue::String("y.txt".to_owned()),
                det_provenance(),
            )
            .unwrap(),
        )
        .try_finalize(
            ContextRef::new(ContextSnapshotId::new(), Utc::now()),
            policy,
            confidence,
        )
        .unwrap()
}

// --- Composing -------------------------------------------------------------

#[test]
fn composing_frame_kind_is_composing() {
    let r = HudRenderer::new();
    let d = build_composing_with_target_only();
    let frame = r.render_composing(&d);
    assert_eq!(frame.kind, HudFrameKind::Composing);
}

#[test]
fn composing_frame_includes_act_id() {
    let r = HudRenderer::new();
    let d = build_composing_with_target_only();
    let frame = r.render_composing(&d);
    assert!(frame.body.contains("file.rename"));
}

#[test]
fn composing_frame_marks_bound_and_unbound_slots_distinctly() {
    let r = HudRenderer::new();
    let d = build_composing_with_target_only();
    let frame = r.render_composing(&d);
    // target was bound → checkmark
    assert!(frame.body.contains("✓ target"));
    // new_name was not bound → cross
    assert!(frame.body.contains("✗ new_name"));
    // unbound count surfaces
    assert!(frame.body.contains("unbound required: 1"));
}

#[test]
fn composing_frame_renders_modifiers() {
    let r = HudRenderer::new();
    let d = build_composing_with_target_only();
    let frame = r.render_composing(&d);
    assert!(frame.body.contains("carefulness=0.70"));
}

// --- Ready -----------------------------------------------------------------

#[test]
fn ready_frame_kind_and_act_id() {
    let r = HudRenderer::new();
    let d = build_ready();
    let frame = r.render_ready(&d);
    assert_eq!(frame.kind, HudFrameKind::Ready);
    assert!(frame.body.contains("file.rename"));
}

#[test]
fn ready_frame_surfaces_policy_and_confidence() {
    let r = HudRenderer::new();
    let d = build_ready();
    let frame = r.render_ready(&d);
    assert!(frame.body.contains("threshold ≥"));
    assert!(frame.body.contains("confidence:"));
    assert!(frame.body.contains("calibrated: true"));
}

#[test]
fn ready_frame_indicates_direct_commit_when_no_ratify() {
    let r = HudRenderer::new();
    let d = build_ready();
    // file.rename is Costly + Project → no ratify required
    let frame = r.render_ready(&d);
    assert!(frame.body.contains("→ ready to .commit() directly"));
}

// --- Proposed --------------------------------------------------------------

#[test]
fn proposed_frame_includes_ratify_controls_and_window() {
    let r = HudRenderer::new();
    // spaces.broadcast (Irreversible+External) requires ratify
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == "spaces.broadcast")
        .unwrap();
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let confidence = Confidence::from_slots(vec![(
        "body".to_owned(),
        ConfidenceScore::new(0.96, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .unwrap();
    let proposed = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(
            ResolvedSlot::new(
                "body",
                ResolvedSlotValue::String("hello".to_owned()),
                det_provenance(),
            )
            .unwrap(),
        )
        .try_finalize(
            ContextRef::new(ContextSnapshotId::new(), Utc::now()),
            policy,
            confidence,
        )
        .unwrap()
        .propose();
    let frame = r.render_proposed(&proposed);
    assert_eq!(frame.kind, HudFrameKind::Proposed);
    assert!(frame.body.contains("[Enter] approve"));
    assert!(frame.body.contains("[Esc] reject"));
    assert!(frame.body.contains("ratify-window:"));
}

// --- Committed -------------------------------------------------------------

#[test]
fn committed_frame_includes_directive_id_and_dispatched_marker() {
    let r = HudRenderer::new();
    let committed = build_ready().commit().unwrap();
    let frame = r.render_committed(&committed);
    assert_eq!(frame.kind, HudFrameKind::Committed);
    assert!(frame.body.contains("directive-id:"));
    assert!(frame.body.contains("dispatched"));
}

// --- Outcome ---------------------------------------------------------------

#[test]
fn outcome_frame_surfaces_undo_label_per_reverse_action() {
    let r = HudRenderer::new();

    let cases = [
        (ReverseAction::None, "no reverse needed"),
        (
            ReverseAction::RenameBack {
                from: "/tmp/a".into(),
                to: "/tmp/b".into(),
            },
            "rename back",
        ),
        (
            ReverseAction::DeleteCopy {
                path: "/tmp/c".into(),
            },
            "delete the copy",
        ),
        (
            ReverseAction::RestoreContent {
                path: "/tmp/d".into(),
                prior: vec![1, 2, 3],
            },
            "restore prior content",
        ),
    ];

    for (reverse, expected_substring) in cases {
        let outcome = ExecutionOutcome {
            act_id: pneuma_core::ActId::new("file.rename").unwrap(),
            result: serde_json::json!({"ok": true}),
            reverse_action: reverse,
        };
        let frame = r.render_outcome(&outcome);
        assert_eq!(frame.kind, HudFrameKind::Outcome);
        assert!(
            frame.body.contains(expected_substring),
            "frame body for {expected_substring}: {}",
            frame.body
        );
    }
}

// --- Errors ----------------------------------------------------------------

#[test]
fn contract_error_frame_kind_is_contract_error() {
    let r = HudRenderer::new();
    let err = ContractError::RatifyRequired;
    let frame = r.render_contract_error(&err);
    assert_eq!(frame.kind, HudFrameKind::ContractError);
    assert!(frame.body.contains("requires ratification"));
}

#[test]
fn praxis_error_frame_kind_is_praxis_error() {
    let r = HudRenderer::new();
    let err = PraxisError::UnsupportedAct("debug.show".to_owned());
    let frame = r.render_praxis_error(&err);
    assert_eq!(frame.kind, HudFrameKind::PraxisError);
    assert!(frame.body.contains("debug.show"));
}

// --- Width / chrome --------------------------------------------------------

#[test]
fn width_is_honored_within_bounds() {
    assert_eq!(HudRenderer::new().width(), 80);
    assert_eq!(HudRenderer::new().with_width(120).width(), 120);
    // Lower clamp at 40, upper clamp at 200
    assert_eq!(HudRenderer::new().with_width(10).width(), 40);
    assert_eq!(HudRenderer::new().with_width(500).width(), 200);
}

#[test]
fn frames_have_top_and_bottom_chrome() {
    let r = HudRenderer::new();
    let frame = r.render_info("HELLO", "world");
    // Box-drawing chrome.
    assert!(frame.body.starts_with('┌'));
    let last_line = frame.body.lines().last().unwrap();
    assert!(last_line.starts_with('└'));
    // Title label embedded in top.
    assert!(frame.body.contains("HELLO"));
    // Body content.
    assert!(frame.body.contains("│ world"));
}

#[test]
fn frame_round_trips_through_serde_json() {
    let r = HudRenderer::new();
    let frame = r.render_info("TEST", "round-trip");
    let json = serde_json::to_string(&frame).unwrap();
    let de: pneuma_hud::HudFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(de, frame);
}
