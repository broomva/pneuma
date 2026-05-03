//! Selection-domain acts. Five acts: select, select_all, copy, paste, cut.

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, req_referent, req_string};

/// Five selection-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // selection.select — select a region.
        act(
            "selection.select",
            vec![req_referent(
                "target",
                ReferentType::Selection,
                "Span to select",
            )],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("clear_selection"),
            "Select a region in the focused buffer.",
        ),
        // selection.select_all — Cmd-A.
        act(
            "selection.select_all",
            vec![],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("clear_selection"),
            "Select the entire focused buffer.",
        ),
        // selection.copy — to clipboard. Free; reverse = restore prior
        // clipboard.
        act(
            "selection.copy",
            vec![],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("restore_clipboard"),
            "Copy the current selection to the clipboard.",
        ),
        // selection.paste — insert from clipboard.
        act(
            "selection.paste",
            vec![],
            Reversibility::Costly,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("undo_paste"),
            "Paste clipboard contents at cursor.",
        ),
        // selection.cut — copy + delete selection.
        act(
            "selection.cut",
            vec![req_string(
                "marker",
                "Identifier for the cut payload (for undo bookkeeping)",
            )],
            Reversibility::Costly,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("undo_cut"),
            "Cut the current selection (copy + delete).",
        ),
    ]
}
