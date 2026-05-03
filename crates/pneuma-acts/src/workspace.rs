//! Workspace-domain acts. Six acts: focus, split_pane, close_window,
//! switch_app, navigate_back, undo.

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, opt_string, req_referent};

/// Six workspace-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // workspace.focus — focus a window or app.
        act(
            "workspace.focus",
            vec![req_referent(
                "target",
                ReferentType::Any,
                "Window or app to focus",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("focus_previous"),
            "Move keyboard focus to a window or app.",
        ),
        // workspace.split_pane — open a new pane.
        act(
            "workspace.split_pane",
            vec![opt_string(
                "direction",
                "Split direction: horizontal | vertical (default vertical)",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("close_pane"),
            "Split the focused pane.",
        ),
        // workspace.close_window — close a window.
        act(
            "workspace.close_window",
            vec![req_referent(
                "target",
                ReferentType::Window,
                "Window to close",
            )],
            Reversibility::Costly,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("reopen_window"),
            "Close a window. Costly — reopen recovers via app history.",
        ),
        // workspace.switch_app — bring a different app to front.
        act(
            "workspace.switch_app",
            vec![req_referent("target", ReferentType::App, "App to focus")],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("switch_back"),
            "Switch to a different application.",
        ),
        // workspace.navigate_back — Cmd-[ / browser back.
        act(
            "workspace.navigate_back",
            vec![],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("navigate_forward"),
            "Navigate back in the focused app's history.",
        ),
        // workspace.undo — invoke the focused app's native undo.
        act(
            "workspace.undo",
            vec![],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("redo"),
            "Trigger the focused app's native undo.",
        ),
    ]
}
