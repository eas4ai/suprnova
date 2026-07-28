//! TS extraction across Data derives:
//!   - Field<T>  → `field?: T | null`
//!   - Prop<T>   → `field?: T`         (lazy/deferred — may be absent)
//!   - input_only → excluded from generated output type
//!   - output_only → included in output type, excluded from input type
//!   - allow_include → no TS effect (runtime-only)

use suprnova_cli::commands::generate_types::{ScanInput, generate_types_string};

const SRC: &str = r#"
use suprnova::data::Field;
use suprnova::inertia::Prop;

#[derive(suprnova::Data, validator::Validate)]
pub struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    pub bio: Field<String>,

    #[data(lazy)]
    pub favorite_song: Prop<String>,
}
"#;

fn extract_block(ts: &str, name: &str) -> String {
    let start = ts
        .find(&format!("export interface {} {{", name))
        .or_else(|| ts.find(&format!("export interface {}<", name)))
        .expect("interface block not found");
    let after = &ts[start..];
    let end = after.find("}\n").expect("block close not found") + 1;
    after[..end].to_string()
}

#[test]
fn user_dto_emits_output_and_input_types() {
    let ts = generate_types_string(ScanInput::Source(SRC));

    // Output type — what the frontend RECEIVES
    let output = extract_block(&ts, "UserDto");
    assert!(output.contains("id: number"));
    assert!(output.contains("name: string"));
    assert!(!output.contains("password")); // input_only excluded
    assert!(output.contains("computed_handle: string"));
    assert!(output.contains("bio?: string | null")); // Field<T>
    assert!(output.contains("favorite_song?: string")); // Prop<T>
    assert!(!output.contains("favorite_song?: string | null"));
    assert!(!output.contains("Prop<")); // never leak Rust-only types

    // Input type — what the frontend SENDS
    let input = extract_block(&ts, "UserDtoInput");
    assert!(input.contains("password: string")); // input_only included
    assert!(!input.contains("computed_handle")); // output_only excluded
    assert!(!input.contains("favorite_song")); // lazy props are output-only
}

const GENERIC_SRC: &str = r#"
use suprnova::data::Field;

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,
    pub cursor: Field<String>,
}
"#;

#[test]
fn generic_struct_emits_typescript_generic() {
    let ts = generate_types_string(ScanInput::Source(GENERIC_SRC));
    assert!(ts.contains("export interface Paginated<T>"));
    assert!(ts.contains("items: Array<T>"));
    assert!(ts.contains("total: number"));
    assert!(ts.contains("cursor?: string | null"));
}

// A prop type that isn't an InertiaProps/Data struct but IS defined in the
// project (here `UserInfo`, which only derives Serialize) resolves to its
// real interface — the definition is right there in the source. Only types
// the project doesn't define degrade to `unknown` (see
// `external_and_tuple_types_still_degrade_to_unknown`).
const UNRESOLVED_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct DashboardProps {
    pub user: UserInfo,
    pub tags: Vec<UserInfo>,
    pub note: Option<UserInfo>,
}

#[derive(serde::Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub name: String,
}
"#;

#[test]
fn underived_local_struct_resolves_to_real_interface() {
    let ts = generate_types_string(ScanInput::Source(UNRESOLVED_SRC));

    let user = extract_block(&ts, "UserInfo");
    assert!(user.contains("id: number"), "got: {user}");
    assert!(user.contains("name: string"), "got: {user}");

    let block = extract_block(&ts, "DashboardProps");
    assert!(block.contains("user: UserInfo"), "got: {block}");
    assert!(block.contains("tags: Array<UserInfo>"), "got: {block}");
    assert!(block.contains("note: UserInfo | null"), "got: {block}");
}

const RESOLVED_NESTED_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Page {
    pub author: Author,
    pub coauthors: Vec<Author>,
}

#[derive(suprnova::InertiaProps)]
pub struct Author {
    pub name: String,
}
"#;

#[test]
fn resolved_nested_inertia_type_keeps_named_reference() {
    let ts = generate_types_string(ScanInput::Source(RESOLVED_NESTED_SRC));
    // Author IS an InertiaProps struct, so it's emitted and the reference stays
    // a precise named type (not degraded to `unknown`).
    assert!(ts.contains("export interface Author"));
    let page = extract_block(&ts, "Page");
    assert!(page.contains("author: Author"), "got: {page}");
    assert!(page.contains("coauthors: Array<Author>"), "got: {page}");
}

// A self-referential InertiaProps struct (a comment thread node holding its own
// children). The generator must still EMIT the interface — a self-edge is not a
// real ordering dependency. Regression for the Kahn's-algorithm self-loop that
// silently dropped self-referencing structs, leaving referencing structs with a
// dangling type name.
const SELF_REF_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct BlogShowProps {
    pub comments: Vec<CommentView>,
}

#[derive(suprnova::InertiaProps)]
pub struct CommentView {
    pub id: i64,
    pub children: Vec<CommentView>,
}
"#;

#[test]
fn self_referential_struct_is_emitted() {
    let ts = generate_types_string(ScanInput::Source(SELF_REF_SRC));

    // The self-referencing interface must be present, not dropped.
    assert!(
        ts.contains("export interface CommentView"),
        "self-referential CommentView was dropped from the output: {ts}"
    );
    let cv = extract_block(&ts, "CommentView");
    assert!(cv.contains("children: Array<CommentView>"), "got: {cv}");

    // And the struct that references it keeps the precise named type, not a
    // dangling identifier or `unknown`.
    let bsp = extract_block(&ts, "BlogShowProps");
    assert!(bsp.contains("comments: Array<CommentView>"), "got: {bsp}");
}

#[test]
fn multi_param_generic() {
    let src = r#"
        #[derive(suprnova::Data)]
        pub struct Pair<A, B>
        where
            A: serde::Serialize + for<'de> serde::Deserialize<'de>,
            B: serde::Serialize + for<'de> serde::Deserialize<'de>,
        {
            pub left: A,
            pub right: B,
        }
    "#;
    let ts = generate_types_string(ScanInput::Source(src));
    assert!(ts.contains("export interface Pair<A, B>"));
    assert!(ts.contains("left: A"));
    assert!(ts.contains("right: B"));
}

// ── Plain-struct resolution ──────────────────────────────────────────────
// A prop field naming a struct that never derived InertiaProps/Data must
// resolve to that struct's real interface (transitively), not degrade to
// `unknown` — regression coverage for the v0.7.1 behavior that clobbered
// committed types files with weaker output.

const NESTED_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct AdminArticlesIndexProps {
    pub articles: Vec<AdminArticleRow>,
    pub external: serde_json::Value,
    pub odd: TupleThing,
}

pub struct AdminArticleRow {
    pub id: i64,
    pub title: String,
    pub meta: RowMeta,
}

pub struct RowMeta {
    pub updated_at: String,
    pub linked: Option<RowMeta>,
}

pub struct Unreferenced {
    pub nobody: bool,
}

pub struct TupleThing(pub i64);
"#;

#[test]
fn plain_structs_resolve_transitively() {
    let ts = generate_types_string(ScanInput::Source(NESTED_SRC));

    let props = extract_block(&ts, "AdminArticlesIndexProps");
    assert!(
        props.contains("articles: Array<AdminArticleRow>"),
        "nested plain struct must keep its name: {props}"
    );

    let row = extract_block(&ts, "AdminArticleRow");
    assert!(row.contains("id: number"));
    assert!(row.contains("title: string"));
    assert!(
        row.contains("meta: RowMeta"),
        "second-level plain struct must resolve too: {row}"
    );

    let meta = extract_block(&ts, "RowMeta");
    assert!(meta.contains("updated_at: string"));
    assert!(
        meta.contains("linked: RowMeta | null"),
        "self-reference through Option must keep the name: {meta}"
    );
}

#[test]
fn unreferenced_plain_structs_stay_out() {
    let ts = generate_types_string(ScanInput::Source(NESTED_SRC));
    assert!(
        !ts.contains("interface Unreferenced"),
        "plain structs nothing reaches must not be emitted: {ts}"
    );
}

#[test]
fn external_and_tuple_types_still_degrade_to_unknown() {
    let ts = generate_types_string(ScanInput::Source(NESTED_SRC));
    let props = extract_block(&ts, "AdminArticlesIndexProps");
    assert!(
        props.contains("external: unknown"),
        "external crate types stay unknown: {props}"
    );
    assert!(
        props.contains("odd: unknown"),
        "tuple structs are not promotable and stay unknown: {props}"
    );
}

#[test]
fn mutually_recursive_plain_structs_both_emit() {
    const CYCLE_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct TreeProps {
    pub root: NodeA,
}

pub struct NodeA {
    pub b: Option<NodeB>,
}

pub struct NodeB {
    pub a: Option<NodeA>,
}
"#;
    let ts = generate_types_string(ScanInput::Source(CYCLE_SRC));
    let a = extract_block(&ts, "NodeA");
    assert!(a.contains("b: NodeB | null"));
    let b = extract_block(&ts, "NodeB");
    assert!(b.contains("a: NodeA | null"));
}
