//! Integration tests for [`ArcanExecutor`] implementations.
//!
//! Properties under test:
//!
//! 1. **`MockArcan`** — returns canned response with the right
//!    `ArcanOutcome` shape regardless of prompt contents.
//! 2. **`StdioCommandArcan`** with a real subprocess (`cat`) —
//!    round-trips: prompt-to-stdin → stdout → captured response.
//!    `cat` is universally available on macOS and Linux CI.
//! 3. **`StdioCommandArcan`** with a failing subprocess (`false`) —
//!    surfaces typed `SubprocessExitNonZero`.
//! 4. **`StdioCommandArcan`** with a missing binary —
//!    surfaces typed `SpawnFailed`.
//! 5. **`StdioCommandArcan::claude_code`** invocation against a real
//!    `claude` binary — `#[ignore]`'d, requires Claude Code on PATH
//!    + ANTHROPIC_API_KEY. Run manually.
//!
//! The test suite is **cross-platform** for properties 1–4 — the
//! `cat` and `false` binaries are present on every Unix-like CI
//! runner the workspace targets. Windows runners would need
//! adjustment (skipped here since the repo's CI matrix is macOS +
//! Ubuntu).

use pneuma_acts::registry;
use pneuma_arcan_bridge::{
    ArcanError, ArcanExecutor, ArcanOutcome, MockArcan, StdioCommandArcan, prompt_to_string,
};
use pneuma_core::ActId;
use pneuma_core::act::ResolvedSlotValue;
use pneuma_core::{FileRef, ReferentValue};
use pneuma_router::AgentPrompt;

// --- Helpers ---------------------------------------------------------------

fn refactor_prompt() -> AgentPrompt {
    let act = registry()
        .into_iter()
        .find(|a| a.id.as_str() == "agent.refactor")
        .expect("agent.refactor canonical");
    AgentPrompt {
        act_id: act.id,
        instruction: "Refactor the authentication module to use ed25519.".to_owned(),
        slots: vec![(
            "target".to_owned(),
            ResolvedSlotValue::Referent(ReferentValue::File(FileRef::new("/tmp/auth.rs"))),
        )],
    }
}

// --- Property 1: MockArcan -------------------------------------------------

#[test]
fn mock_arcan_dispatches_any_prompt() {
    let mock = MockArcan::new("Refactored 3 functions; tests pass.");
    let outcome = mock.execute(&refactor_prompt()).unwrap();
    assert_eq!(outcome.act_id.as_str(), "agent.refactor");
    assert_eq!(outcome.executor, "mock");
    assert_eq!(outcome.exit_code, 0);
    assert!(outcome.response.contains("Refactored"));
}

#[test]
fn mock_arcan_canned_response_is_unchanged_across_calls() {
    let mock = MockArcan::new("static");
    let p1 = refactor_prompt();
    let mut p2 = refactor_prompt();
    p2.instruction = "Different instruction".to_owned();

    assert_eq!(mock.execute(&p1).unwrap().response, "static");
    assert_eq!(mock.execute(&p2).unwrap().response, "static");
}

// --- Property 2: real subprocess via `cat` ---------------------------------

#[cfg(unix)]
#[test]
fn stdio_command_arcan_round_trips_prompt_through_cat() {
    // `cat` echoes stdin to stdout — perfect prompt round-trip test.
    let exec = StdioCommandArcan::new("/bin/cat", Vec::<String>::new(), "cat-echo");
    let prompt = refactor_prompt();
    let outcome: ArcanOutcome = exec.execute(&prompt).unwrap();
    assert_eq!(outcome.act_id.as_str(), "agent.refactor");
    assert_eq!(outcome.executor, "cat-echo");
    assert_eq!(outcome.exit_code, 0);

    // `cat` echoes the formatted prompt verbatim, so the response
    // should contain the original instruction text.
    let formatted = prompt_to_string(&prompt);
    // `cat`'s stdout is the formatted prompt minus trailing newline
    // (we trim_end() in execute). Check key substrings rather than
    // byte equality (trailing whitespace differs).
    assert!(
        outcome.response.contains("Refactor the authentication"),
        "cat should echo the instruction; got response: {}",
        outcome.response
    );
    assert!(
        outcome.response.contains("Bound slots"),
        "cat should echo the slot footer; formatted was: {formatted}"
    );
}

// --- Property 3: subprocess exit non-zero ---------------------------------

#[cfg(unix)]
#[test]
fn stdio_command_arcan_surfaces_non_zero_exit() {
    let exec = StdioCommandArcan::new("/usr/bin/false", Vec::<String>::new(), "false");
    let err = exec.execute(&refactor_prompt()).unwrap_err();
    assert!(
        matches!(err, ArcanError::SubprocessExitNonZero { .. }),
        "expected SubprocessExitNonZero, got {err:?}"
    );
    if let ArcanError::SubprocessExitNonZero { exit_code, .. } = err {
        assert_eq!(exit_code, Some(1), "false exits with code 1");
    }
}

// --- Property 4: missing binary -------------------------------------------

#[test]
fn stdio_command_arcan_surfaces_spawn_failure_for_missing_binary() {
    let exec = StdioCommandArcan::new(
        "/this/path/definitely/does/not/exist/xyz123",
        Vec::<String>::new(),
        "missing",
    );
    let err = exec.execute(&refactor_prompt()).unwrap_err();
    assert!(
        matches!(err, ArcanError::SpawnFailed { .. }),
        "expected SpawnFailed, got {err:?}"
    );
}

// --- Cross-cutting: prompt formatter --------------------------------------

#[test]
fn prompt_to_string_includes_act_id_in_footer() {
    let formatted = prompt_to_string(&refactor_prompt());
    assert!(formatted.contains("- act: agent.refactor"));
}

#[test]
fn prompt_to_string_renders_each_slot_on_own_line() {
    let mut p = refactor_prompt();
    p.slots.push((
        "branch".to_owned(),
        ResolvedSlotValue::String("main".to_owned()),
    ));
    let formatted = prompt_to_string(&p);
    let slot_lines: Vec<&str> = formatted
        .lines()
        .filter(|l| l.starts_with("- ") && l.contains(':'))
        .collect();
    // 2 slots + 1 act-id line.
    assert!(
        slot_lines.len() >= 3,
        "expected ≥3 footer lines, got {slot_lines:?}"
    );
}

// --- Cross-cutting: ActId-clone semantics ---------------------------------

#[test]
fn act_id_in_outcome_matches_act_id_in_prompt() {
    let mock = MockArcan::new("ok");
    let p = refactor_prompt();
    let outcome = mock.execute(&p).unwrap();
    assert_eq!(outcome.act_id, p.act_id);
    assert_eq!(outcome.act_id.as_str(), "agent.refactor");
    // Verify ActId is constructible from the outcome's clone.
    let _: ActId = outcome.act_id.clone();
}

// --- Property 5: real Claude Code (gated, ignored by default) -------------

/// Real round-trip against the local `claude` CLI. Disabled by
/// default — requires `claude` on PATH and `ANTHROPIC_API_KEY` set,
/// and incurs API charges. Run manually with:
///
/// ```bash
/// cargo test -p pneuma-arcan-bridge --test executor_dispatch -- --ignored
/// ```
#[test]
#[ignore = "calls real Claude API — run manually"]
fn claude_code_round_trip() {
    let exec = StdioCommandArcan::claude_code();
    let outcome = exec
        .execute(&refactor_prompt())
        .expect("claude CLI should respond");
    assert_eq!(outcome.executor, "claude-code");
    assert!(
        !outcome.response.is_empty(),
        "claude should produce a non-empty response"
    );
    eprintln!("claude response:\n{}", outcome.response);
}
