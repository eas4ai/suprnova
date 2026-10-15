//! `timestamps = true` asks for managed timestamps, so a model without the
//! fields that hold them fails the build, naming the fields to add.

#[suprnova::document(collection = "users", fillable = ["name"], timestamps = true)]
pub struct User {
    pub name: String,
}

fn main() {}
