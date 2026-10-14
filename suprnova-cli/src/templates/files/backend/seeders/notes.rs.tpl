//! Searchable example notes with words long enough to exercise typo tolerance.

use suprnova::{FrameworkError, Model, Seeder, async_trait, attrs};

use crate::models::note::Note;
use crate::models::user::User;

/// Gives a fresh kit a known account and searchable notes without duplicating reruns.
pub struct NotesSeeder;

#[async_trait]
impl Seeder for NotesSeeder {
    fn name() -> &'static str {
        "NotesSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        Note::initialize_search().await?;
        let user = match User::find_by_email("notes@example.com").await? {
            Some(user) => user,
            None => {
                User::create(
                    "Notes Explorer",
                    "notes@example.com",
                    "change-this-password",
                )
                .await?
            }
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
