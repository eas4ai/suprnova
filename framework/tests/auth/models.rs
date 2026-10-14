//! The scaffold's models, at the paths its templates name each other by
//! (`crate::models::user`, `crate::models::note`): `scaffold_tables` drives
//! the scaffold's `User`, and that model has many notes.

// `#[rustfmt::skip]`: the templates are scaffold output, not workspace
// source, so `cargo fmt` must not rewrite them.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/note.rs.tpl"]
pub mod note;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/user.rs.tpl"]
pub mod user;

// The scaffold's handlers read notes only through `Note::owned_by`, and none
// of them is in this binary. Naming it here checks its signature against the
// template.
const _: fn(u64) -> suprnova::Builder<note::Note> = note::Note::owned_by;
