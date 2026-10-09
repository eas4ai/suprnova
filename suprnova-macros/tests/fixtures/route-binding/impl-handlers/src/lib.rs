//! BIND-003: a `#[handler(Self = Posts)]` function inside `impl Posts`
//! takes every binding form a free handler takes, beside every other form
//! of argument and `#[authorize]` above or below it, and registers as
//! `Posts::show` wherever a handler registers. This crate must compile.

use suprnova::database::EntityExt;
use suprnova::{FromRequest, Request, Response, RouteParam, Router, get, handler, model, request};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
}

// The bare `x::Model` form binds a SeaORM row whose entity opts in.
impl EntityExt for post::Entity {}

#[request]
pub struct UpdatePost {
    pub title: String,
}

pub struct Posts;

impl Posts {
    /// A `#[model]` type.
    #[handler(Self = Posts)]
    pub async fn show(post: Post) -> Response {
        suprnova::http::text(post.title)
    }

    /// `RouteParam<T>`, whole and destructured.
    #[handler(Self = Posts)]
    pub async fn wrapped(post: RouteParam<Post>) -> Response {
        suprnova::http::text(post.title.clone())
    }

    #[handler(Self = Posts)]
    pub async fn destructured(RouteParam(post): RouteParam<Post>) -> Response {
        suprnova::http::text(post.title)
    }

    /// The bare `x::Model` form.
    #[handler(Self = Posts)]
    pub async fn bare(post: post::Model) -> Response {
        suprnova::http::text(post.title)
    }

    /// A primitive `#[authorize]` target, the attribute below `#[handler]`.
    #[handler(Self = Posts)]
    #[suprnova::authorize("show", id)]
    pub async fn guarded(id: i64) -> Response {
        suprnova::http::text(id.to_string())
    }

    /// A bound `#[authorize]` target, the attribute above `#[handler]`, and
    /// a form request read after the check.
    #[suprnova::authorize("update", post)]
    #[handler(Self = Posts)]
    pub async fn update(post: Post, form: UpdatePost) -> Response {
        suprnova::http::text(format!("{} {}", post.title, form.title))
    }

    /// Optional values, a path value and a bound one.
    #[handler(Self = Posts)]
    pub async fn maybe(page: Option<u32>, post: Option<Post>) -> Response {
        suprnova::http::text(format!("{page:?} {}", post.is_some()))
    }

    /// No argument, the request itself, and a sync handler, which takes
    /// `Self` as a free sync function takes `#[handler]`.
    #[handler(Self = Posts)]
    pub async fn index() -> Response {
        suprnova::http::text("index")
    }

    #[handler(Self = Posts)]
    pub async fn raw(req: Request) -> Response {
        suprnova::http::text(req.path().to_owned())
    }

    #[handler(Self = Posts)]
    pub fn ping() -> Response {
        suprnova::http::text("pong")
    }

    /// A generic handler, which has no record, as a free one has none.
    #[handler(Self = Posts)]
    pub async fn store<T: FromRequest + Send + 'static>(form: T) -> Response {
        let _ = form;
        suprnova::http::text("stored")
    }

    /// The type named by a path.
    #[handler(Self = crate::Posts)]
    pub async fn by_path(id: i64) -> Response {
        suprnova::http::text(id.to_string())
    }
}

/// A free handler beside the associated ones keeps compiling as before.
#[handler]
pub async fn free(post: Post) -> Response {
    suprnova::http::text(post.title)
}

pub mod nested {
    use super::*;

    /// A child module that glob-imports its parent, where a free handler
    /// named `show` lives, still names its own type.
    pub struct Comments;

    impl Comments {
        #[handler(Self = Comments)]
        pub async fn show(id: i64) -> Response {
            suprnova::http::text(id.to_string())
        }
    }

    #[handler]
    pub async fn show(id: i64) -> Response {
        suprnova::http::text(id.to_string())
    }
}

pub fn router() -> Router {
    let router = Router::new()
        .get("/posts/{post}", Posts::show)
        .get("/wrapped/{post}", Posts::wrapped)
        .get("/destructured/{post}", Posts::destructured)
        .get("/bare/{post}", Posts::bare)
        .get("/guarded/{id}", Posts::guarded)
        .put("/posts/{post}", Posts::update)
        .get("/maybe/{page?}/{post?}", Posts::maybe)
        .get("/posts", Posts::index)
        .get("/raw", Posts::raw)
        .post("/store", Posts::store::<UpdatePost>)
        .get("/by-path/{id}", Posts::by_path)
        .get("/free/{post}", free)
        .get("/comments/{id}", nested::Comments::show)
        .get("/nested/{id}", nested::show)
        .group("/grouped", |r| r.get("/{post}", Posts::show))
        .into();
    get!("/macro/{post}", Posts::show).register(router)
}
