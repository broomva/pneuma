//! File-domain acts. Eight acts: open, read, rename, move, copy,
//! delete, save, write.
//!
//! Each maps to a Praxis primitive with a deterministic reverse-action
//! recipe (when reversible). `delete` is irreversible and must
//! ratify; `read`, `open`, `save` are Free; `rename`, `move`, `copy`,
//! `write` are Costly (reversible but takes time/IO).

use pneuma_core::{Act, BlastRadius, ExecutorHint, ReferentType, Reversibility};

use crate::{act, opt_string, req_referent, req_string};

/// Eight file-domain acts.
#[must_use]
pub fn acts() -> Vec<Act> {
    vec![
        // file.open — bring a file into the focused editor.
        act(
            "file.open",
            vec![req_referent("target", ReferentType::File, "File to open")],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("close_file"),
            "Open a file in the focused editor.",
        ),
        // file.read — read content (no side effects).
        act(
            "file.read",
            vec![req_referent("target", ReferentType::File, "File to read")],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            None, // read has no side effect → nothing to reverse
            "Read a file's content.",
        ),
        // file.rename — atomic rename. Reverse: rename back.
        act(
            "file.rename",
            vec![
                req_referent("target", ReferentType::File, "File to rename"),
                req_string("new_name", "New filename (basename only)"),
            ],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Praxis,
            Some("rename_back"),
            "Rename a file in place.",
        ),
        // file.move — move to a new directory. Reverse: move back.
        act(
            "file.move",
            vec![
                req_referent("target", ReferentType::File, "File to move"),
                req_string("destination", "Destination directory absolute path"),
            ],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Praxis,
            Some("move_back"),
            "Move a file to a new directory.",
        ),
        // file.copy — duplicate. Reverse: delete the copy.
        act(
            "file.copy",
            vec![
                req_referent("target", ReferentType::File, "File to copy"),
                req_string("destination", "Destination path absolute"),
            ],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Praxis,
            Some("delete_copy"),
            "Copy a file to a new location.",
        ),
        // file.delete — irreversible. Forces ratify.
        act(
            "file.delete",
            vec![req_referent("target", ReferentType::File, "File to delete")],
            Reversibility::Irreversible,
            BlastRadius::Project,
            ExecutorHint::Praxis,
            None, // truly irreversible
            "Delete a file. Irreversible — requires ratification.",
        ),
        // file.save — save the focused buffer. Reverse: revert to last
        // saved version.
        act(
            "file.save",
            vec![opt_string("target_path", "Optional override save path")],
            Reversibility::Free,
            BlastRadius::Local,
            ExecutorHint::Praxis,
            Some("revert_save"),
            "Save the focused buffer.",
        ),
        // file.write — overwrite content. Reverse: restore prior content.
        act(
            "file.write",
            vec![
                req_referent("target", ReferentType::File, "File to overwrite"),
                req_string("content", "New file content"),
            ],
            Reversibility::Costly,
            BlastRadius::Project,
            ExecutorHint::Praxis,
            Some("restore_content"),
            "Overwrite a file's content.",
        ),
    ]
}
