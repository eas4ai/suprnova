//! Searchable example notes with words long enough to exercise typo tolerance.

use suprnova::{FrameworkError, Model, Seeder, async_trait, attrs};

use crate::models::note::Note;
use crate::models::user::User;

/// Gives the first registered account searchable notes without duplicating reruns.
///
/// The seeder creates no account of its own: a seeded account needs a password
/// written in this file, and `db:seed` runs on any machine, production included.
/// Register an account, then seed; the notes appear in that account's search.
pub struct NotesSeeder;

#[async_trait]
impl Seeder for NotesSeeder {
    fn name() -> &'static str {
        "NotesSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        Note::initialize_search().await?;
        let Some(user) = <User as Model>::query().order_by_asc("id").first().await? else {
            return Err(FrameworkError::bad_request(
                "NotesSeeder needs a registered account to own the example notes: register one, then run db:seed again",
            ));
        };
        for (title, body) in [
            (
                "Garden planning",
                "Planting flowers along the garden pathway.",
            ),
            (
                "Ocean journal",
                "Remember the ocean sunrise and the quiet forest.",
            ),
            (
                "Library reading",
                "Collect stories about science and discovery.",
            ),
        ] {
            if Note::owned_by(user.id)
                .filter("title", title)
                .first()
                .await?
                .is_none()
            {
                <Note as Model>::create(attrs! {
                    user_id: user.id,
                    title: title,
                    body: Some(body.to_owned()),
                })
                .await?;
            }
        }
        Ok(())
    }
}
