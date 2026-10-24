//! Posts controller.
//!
//! Dogfoods the real SeaORM-backed `Post` model added by codex review
//! finding #17. Four endpoints:
//!
//! - `POST /api/posts` - authenticated create. Body is the `CreatePost`
//!   payload (title/body/is_public). `author_id` comes from the signed-in
//!   user via `Auth::user_as::<User>()`, never from the request body.
//! - `GET  /api/posts` - list every public post (gated at the query
//!   layer via `Post::public_page`); does not require auth.
//! - `GET  /api/posts/{id}` - fetch a single post; the `PostPolicy`
//!   `view` rule (post.is_public) is enforced via `Gate::authorize`.
//!
//! - `POST /api/posts/tools` - the validated `create_post` tool, for a
//!   signed-in user by browser session or by a token the user approved for
//!   an MCP client with the `posts:write` scope. The listing is also the
//!   public `list_posts` tool (`src/agents.rs`). Each tool call runs its
//!   route, including its session gate. The MCP endpoint signs the token's
//!   user in first, so `Auth::user_as` reads the same user either way.
//!
//! `DELETE /api/posts/{id}` continues to live in
//! `controllers::admin::delete_post` because it pairs with the
//! `Gate::authorize("delete-post", ...)` example wired in
//! `app/src/policies/post_policy.rs`.

use serde::Deserialize;
use suprnova::{
    Auth, FormRequest, FrameworkError, Gate, HttpResponse, Model, Request, Response,
    ValidationErrors, attrs, handler, serde_json, tool, tool_input, tool_output,
};

use crate::agents::scope;
use crate::models::posts::Post;
use crate::models::users::User;

/// Body for `POST /api/posts`. Its validation keeps the REST error contract.
/// `author_id` comes from the session rather than the request body.
#[derive(Deserialize)]
pub struct CreatePost {
    /// The post's title.
    pub title: String,
    /// The post's text.
    pub body: String,
    /// Private unless the caller explicitly makes it public.
    #[serde(default)]
    pub is_public: bool,
}

/// The validated input of `create_post` on `POST /api/posts/tools`.
/// `author_id` is intentionally omitted - the server derives it from the
/// signed-in user to keep the trust boundary clean (a client must not be
/// able to author a post as someone else). A missing field reads as empty,
/// so it fails the rules below, naming the field, rather than the parse.
#[tool_input]
#[form_request(custom_hooks)]
pub struct CreatePostInput {
    /// The post's title.
    #[serde(default)]
    #[validate(length(
        min = 1,
        max = 255,
        message = "Give the post a title of 1 to 255 characters."
    ))]
    pub title: String,
    /// The post's text.
    #[serde(default)]
    #[validate(length(min = 1, message = "Give the post some text."))]
    pub body: String,
    /// Whether anyone may read the post, through `list_posts` and
    /// `GET /api/posts`. Defaults to `false` when omitted, mirroring the
    /// column default in the migration.
    #[serde(default)]
    pub is_public: bool,
}

impl FormRequest for CreatePostInput {
    /// A title or text of spaces alone is none.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let blank_title = self.title.trim().is_empty();
        let blank_body = self.body.trim().is_empty();
        if !blank_title && !blank_body {
            return Ok(());
        }
        let mut errs = ValidationErrors::new();
        if blank_title {
            errs.add("title", "Give the post a title of 1 to 255 characters.");
        }
        if blank_body {
            errs.add("body", "Give the post some text.");
        }
        Err(errs)
    }
}

/// A post as `create_post` and `list_posts` answer it, with the keys
/// `GET /api/posts/{id}` answers too.
#[tool_output]
pub struct PostItem {
    /// The post's id, which `GET /api/posts/{id}` names.
    pub id: i64,
    /// The id of the user who wrote it.
    pub author_id: i64,
    /// The post's title.
    pub title: String,
    /// The post's text.
    pub body: String,
    /// Whether anyone may read it.
    pub is_public: bool,
}

impl From<Post> for PostItem {
    fn from(post: Post) -> Self {
        Self {
            id: post.id,
            author_id: post.author_id,
            title: post.title,
            body: post.body,
            is_public: post.is_public,
        }
    }
}

/// One page of public posts, as `GET /api/posts` and `list_posts` answer
/// it.
#[tool_output]
pub struct PostPage {
    /// The public posts on this page, newest first.
    pub posts: Vec<PostItem>,
    /// The page's number, from 1.
    pub current_page: u64,
    /// How many posts a page holds.
    pub per_page: u64,
    /// Whether a next page exists.
    pub has_more: bool,
}

/// `value` as a JSON answer with `status`.
fn answer(
    value: serde_json::Result<serde_json::Value>,
    status: u16,
) -> Result<HttpResponse, FrameworkError> {
    let body =
        value.map_err(|error| FrameworkError::internal(format!("encoding the posts: {error}")))?;
    Ok(HttpResponse::json(body).status(status))
}

/// `POST /api/posts` - create a post with the REST route's validation answers.
pub async fn store(req: Request) -> Response {
    store_inner(req).await.map_err(HttpResponse::from)
}

async fn store_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let current_user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let payload: CreatePost = req.json().await?;
    if payload.title.trim().is_empty() {
        return Err(FrameworkError::bad_request("title must not be empty"));
    }
    if payload.body.trim().is_empty() {
        return Err(FrameworkError::bad_request("body must not be empty"));
    }
    write_post(current_user.id, payload).await
}

async fn write_post(author_id: i64, payload: CreatePost) -> Result<HttpResponse, FrameworkError> {
    let post = Post::create(attrs! {
        author_id: author_id,
        title: payload.title,
        body: payload.body,
        is_public: payload.is_public,
    })
    .await?;
    answer(serde_json::to_value(PostItem::from(post)), 201)
}

/// `POST /api/posts/tools` - the tool `create_post`: a post owned by the
/// signed-in user.
#[tool(
    name = "create_post",
    title = "Write a post",
    description = "Write a post for the signed-in user, with a title and some text. It stays private unless `is_public` is true. Answers the post written.",
    callers = [user],
    scopes = [scope::POSTS_WRITE],
    input = CreatePostInput,
    output = PostItem,
    annotations(read_only = false, destructive = false),
)]
#[handler]
pub async fn create(form: CreatePostInput) -> Response {
    let current_user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;

    Ok(write_post(
        current_user.id,
        CreatePost {
            title: form.title,
            body: form.body,
            is_public: form.is_public,
        },
    )
    .await?)
}

/// `GET /api/posts` - the tool `list_posts`: every public post, a page at a
/// time.
///
/// Post text is user content, so the tool is marked `untrusted_content`:
/// an agent reads what it answers as data, never as instructions.
#[tool(
    name = "list_posts",
    title = "List public posts",
    description = "List the newest public posts, twenty at a time. Anyone may read them, with no sign-in.",
    callers = [public],
    output = PostPage,
    annotations(read_only, untrusted_content),
)]
#[handler]
pub async fn index(_req: Request) -> Response {
    // Paginated, not `all_public()`. This route is unauthenticated, so an
    // unbounded listing is a denial-of-service primitive as soon as the
    // table is real: at 50M posts a single request would materialise ~42M
    // rows and tens of gigabytes before the process died. Page size is
    // fixed here rather than caller-supplied - see `Post::public_page`.
    const PER_PAGE: u64 = 20;

    let page = Post::public_page(PER_PAGE).await?;
    let page = PostPage {
        posts: page.data.into_iter().map(PostItem::from).collect(),
        current_page: page.current_page,
        per_page: page.per_page,
        has_more: page.has_more,
    };
    Ok(answer(serde_json::to_value(page), 200)?)
}

/// `GET /api/posts/{id}` - fetch a single post, gated by the
/// `view-post` policy (`post.is_public`).
///
/// The Gate runs `view-post` registered automatically by the
/// `#[policy(User, Post)]` impl on `PostPolicy`. Anonymous viewers
/// (no session) are rejected by `Gate::authorize` because the gate
/// signature requires a `&User`.
pub async fn show(req: Request) -> Response {
    show_inner(req).await.map_err(HttpResponse::from)
}

async fn show_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw_id = req.param("id")?;
    let post_id: i64 = raw_id
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let current_user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;

    let post = Post::find_by_id(post_id)
        .await?
        .ok_or_else(|| FrameworkError::not_found("post"))?;

    Gate::authorize("view-post", &current_user, &post)?;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "id": post.id,
        "author_id": post.author_id,
        "title": post.title,
        "body": post.body,
        "is_public": post.is_public,
    })))
}
