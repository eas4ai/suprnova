//! `timestamps = true` asks for managed timestamps, so a model without the
//! columns that hold them fails the build, naming both columns.

#[suprnova::model(table = "stamped_rows", timestamps = true)]
pub struct StampedRow {
    pub id: i64,
    pub name: String,
}

fn main() {}
