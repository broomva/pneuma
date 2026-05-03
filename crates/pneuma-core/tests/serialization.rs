//! Serialization round-trips. Every public type round-trips through
//! `serde_json` so journals, replay, and cross-process IPC can rely on
//! the wire format.
//!
//! Properties (cross-references to `MIL-PROJECT.md` §10.3):
//!
//! - Round-trip through serde_json for all the load-bearing types.

use chrono::Utc;
use pneuma_core::CostClass;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::directive::{Committed, Composing, Ready};
use pneuma_core::{
    Act, ActId, ActPrimitive, AgentResponse, Arity, BindingKind, BlastRadius, Confidence,
    ConfidenceProducer, ConfidenceScore, ContextRef, ContextSnapshotId, Directive, DirectiveError,
    DirectiveId, DirectiveResult, ExecutorHint, ExecutorKind, FileRef, Modifier, PlannedStep,
    PolicyEnvelope, ProgressUpdate, ProposalKind, Provenance, RedactionRule, ReferentType,
    ReferentValue, ResolvedAct, ResolvedSlot, Reversibility, SelectionRef, SlotKind, SlotSignature,
    SpeechAct, StepStatus, SymbolRef, Tagged, TextSpan, TimeWindowSpec, TokenRef, WindowId,
};

#[test]
fn act_id_round_trips() {
    let id = ActId::new("file.rename").unwrap();
    let json = serde_json::to_string(&id).unwrap();
    let de: ActId = serde_json::from_str(&json).unwrap();
    assert_eq!(de.as_str(), "file.rename");
}

#[test]
fn directive_id_round_trips() {
    let id = DirectiveId::new();
    let json = serde_json::to_string(&id).unwrap();
    let de: DirectiveId = serde_json::from_str(&json).unwrap();
    assert_eq!(de, id);
}

#[test]
fn referent_value_round_trips() {
    let cases = vec![
        ReferentValue::File(FileRef::new("/tmp/x.txt").with_mime("text/plain")),
        ReferentValue::Window(WindowId::new("win-1").unwrap()),
        ReferentValue::Url("https://example.com".to_owned()),
        ReferentValue::Selection(SelectionRef::new(
            FileRef::new("/code.rs"),
            TextSpan::new(0, 100).unwrap(),
        )),
        ReferentValue::Symbol(SymbolRef::new(FileRef::new("/code.rs"), "module::func").unwrap()),
    ];
    for v in cases {
        let json = serde_json::to_string(&v).unwrap();
        let de: ReferentValue = serde_json::from_str(&json).unwrap();
        assert_eq!(de, v);
    }
}

#[test]
fn modifier_round_trips() {
    let cases = vec![
        Modifier::magnitude(0.7).unwrap(),
        Modifier::carefulness(0.5).unwrap(),
        Modifier::urgency(0.9).unwrap(),
        Modifier::commitment(0.4).unwrap(),
        Modifier::abstraction_level(0.6).unwrap(),
        Modifier::Distributive,
        Modifier::Negation,
        Modifier::TimeWindow(
            TimeWindowSpec::new(Utc::now(), Utc::now() + chrono::Duration::hours(1)).unwrap(),
        ),
        Modifier::Custom {
            kind: "vendor.special".to_owned(),
            payload: serde_json::json!({"extra": 42}),
        },
    ];
    for m in cases {
        let json = serde_json::to_string(&m).unwrap();
        let de: Modifier = serde_json::from_str(&json).unwrap();
        assert_eq!(de, m);
    }
}

#[test]
fn confidence_round_trips() {
    let conf = Confidence::from_slots(vec![
        (
            "a".to_owned(),
            ConfidenceScore::new(0.9, true, ConfidenceProducer::Deterministic).unwrap(),
        ),
        (
            "b".to_owned(),
            ConfidenceScore::new(0.7, false, ConfidenceProducer::LlmLogprob).unwrap(),
        ),
    ])
    .unwrap();
    let json = serde_json::to_string(&conf).unwrap();
    let de: Confidence = serde_json::from_str(&json).unwrap();
    assert_eq!(de, conf);
}

#[test]
fn policy_envelope_round_trips() {
    let mut p = PolicyEnvelope::intrinsic(Reversibility::Costly, BlastRadius::Project);
    p.permitted_executors = vec![ExecutorKind::Praxis, ExecutorKind::Arcan];
    p.redactions = vec![RedactionRule {
        path: "$.utterance".to_owned(),
        replacement: "<redacted>".to_owned(),
        reason: "potentially sensitive".to_owned(),
    }];
    p.tightened_by_user = true;
    let json = serde_json::to_string(&p).unwrap();
    let de: PolicyEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(de, p);
}

#[test]
fn act_round_trips() {
    let a = Act {
        id: ActId::new("file.rename").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
            SlotSignature::new("new_name", SlotKind::String, Arity::Required).unwrap(),
        ],
        reversibility: Reversibility::Costly,
        blast_radius: BlastRadius::Project,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: Some("rename_back".to_owned()),
        description: Some("Rename a file".to_owned()),
    };
    let json = serde_json::to_string(&a).unwrap();
    let de: Act = serde_json::from_str(&json).unwrap();
    assert_eq!(de, a);
}

#[test]
fn directive_composing_round_trips() {
    let act = Act {
        id: ActId::new("file.read").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    };
    let resolved = ResolvedAct::empty(act);
    let composing: Directive<Composing> = Directive::new(SpeechAct::Directive, resolved)
        .with_modifier(Modifier::carefulness(0.5).unwrap())
        .with_token(TokenRef::new("tok-1", "voice", Utc::now()))
        .with_utterance("read it");

    let json = serde_json::to_string(&composing).unwrap();
    let de: Directive<Composing> = serde_json::from_str(&json).unwrap();
    assert_eq!(de.id, composing.id);
    assert_eq!(de.utterance, composing.utterance);
    assert_eq!(de.modifiers.len(), 1);
}

#[test]
fn directive_committed_round_trips() {
    let act = Act {
        id: ActId::new("file.read").unwrap(),
        primitive: ActPrimitive::Custom,
        slots: vec![
            SlotSignature::new(
                "target",
                SlotKind::Referent(ReferentType::File),
                Arity::Required,
            )
            .unwrap(),
        ],
        reversibility: Reversibility::Free,
        blast_radius: BlastRadius::Local,
        executor_hint: ExecutorHint::Praxis,
        reverse_recipe: None,
        description: None,
    };
    let policy = PolicyEnvelope::intrinsic(act.reversibility, act.blast_radius);
    let resolved = ResolvedAct::empty(act);
    let bound = ResolvedSlot::new(
        "target",
        ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/x"))),
        Provenance::new(Vec::new(), BindingKind::Deterministic, Utc::now()),
    )
    .unwrap();
    let confidence = Confidence::from_slots(vec![(
        "target".to_owned(),
        ConfidenceScore::new(0.9, true, ConfidenceProducer::Deterministic).unwrap(),
    )])
    .unwrap();
    let committed: Directive<Committed> = Directive::new(SpeechAct::Directive, resolved)
        .bind_slot(bound)
        .try_finalize(
            ContextRef::new(ContextSnapshotId::new(), Utc::now()),
            policy,
            confidence,
        )
        .unwrap()
        .commit()
        .unwrap();

    let json = serde_json::to_string(&committed).unwrap();
    let de: Directive<Ready> = serde_json::from_str(&json).unwrap();
    // Note: typestate is not serialized; the runtime state field is.
    // De-serializing into a different typestate succeeds because the
    // PhantomData is `serde(skip)`. The runtime state mirror is the
    // honest one.
    assert_eq!(de.id, committed.id);
    assert_eq!(de.state, pneuma_core::DirectiveState::Committed);
}

#[test]
fn agent_response_round_trips_all_variants() {
    let did = DirectiveId::new();
    let cases = vec![
        AgentResponse::Plan {
            directive_id: did,
            steps: vec![PlannedStep {
                step_id: "s1".to_owned(),
                description: "step one".to_owned(),
                cost: CostClass::Small,
                status: StepStatus::Pending,
                depends_on: Vec::new(),
            }],
            emitted_at: Utc::now(),
        },
        AgentResponse::Progress {
            directive_id: did,
            update: ProgressUpdate {
                step_id: "s1".to_owned(),
                fraction: 0.5,
                message: Some("halfway".to_owned()),
            },
            emitted_at: Utc::now(),
        },
        AgentResponse::Proposed {
            directive_id: did,
            kind: ProposalKind::IrreversibleAction,
            summary: "delete it?".to_owned(),
            emitted_at: Utc::now(),
        },
        AgentResponse::Done {
            directive_id: did,
            result: DirectiveResult {
                payload: serde_json::json!({"ok": true}),
                reverse_action: None,
            },
            emitted_at: Utc::now(),
        },
        AgentResponse::Error {
            directive_id: did,
            error: DirectiveError {
                code: "file.not_found".to_owned(),
                message: "missing".to_owned(),
                details: None,
            },
            emitted_at: Utc::now(),
        },
    ];

    for r in cases {
        let json = serde_json::to_string(&r).unwrap();
        let de: AgentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, r);
    }
}

#[test]
fn tagged_round_trips() {
    let v = Tagged::new(
        42_u32,
        Provenance::new(
            vec![TokenRef::new("tok", "voice", Utc::now())],
            BindingKind::ModelInterpretation,
            Utc::now(),
        ),
    );
    let json = serde_json::to_string(&v).unwrap();
    let de: Tagged<u32> = serde_json::from_str(&json).unwrap();
    assert_eq!(de.value, v.value);
    assert_eq!(de.provenance, v.provenance);
}
