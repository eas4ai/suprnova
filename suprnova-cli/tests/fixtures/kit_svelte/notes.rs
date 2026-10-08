use serde::Serialize;
use suprnova::{handler, InertiaProps, InertiaResponse, Request, Response};

#[derive(Serialize)]
pub struct NoteSummary {
    pub id: u64,
    pub title: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct NoteView {
    pub id: u64,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
}

#[derive(InertiaProps)]
pub struct NotesIndexProps {
    pub notes: Vec<NoteSummary>,
    pub search: String,
}

#[derive(InertiaProps)]
pub struct NotesShowProps {
    pub note: NoteView,
}

#[handler]
pub async fn index(req: Request) -> Response {
    InertiaResponse::new("Notes/Index").with_data(NotesIndexProps { notes, search })
}

#[handler]
pub async fn show(req: Request) -> Response {
    InertiaResponse::new("Notes/Show").with_data(NotesShowProps { note })
}
