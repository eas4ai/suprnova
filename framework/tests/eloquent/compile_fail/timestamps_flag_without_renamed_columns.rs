//! The bare `timestamps` flag asks for managed timestamps too, and the
//! error names the columns the model renamed them to.

#[suprnova::model(
    table = "renamed_stamp_rows",
    timestamps,
    created_at = "made_at",
    updated_at = "changed_at"
)]
pub struct RenamedStampRow {
    pub id: i64,
    pub name: String,
}

fn main() {}
