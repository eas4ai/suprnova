//! TS extraction across Data derives:
//!   - Field<T>  → `field?: T | null`
//!   - Prop<T>   → `field?: T`         (lazy/deferred - may be absent)
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

    // Output type - what the frontend RECEIVES
    let output = extract_block(&ts, "UserDto");
    assert!(output.contains("id: number"));
    assert!(output.contains("name: string"));
    assert!(!output.contains("password")); // input_only excluded
    assert!(output.contains("computed_handle: string"));
    assert!(output.contains("bio?: string | null")); // Field<T>
    assert!(output.contains("favorite_song?: string")); // Prop<T>
    assert!(!output.contains("favorite_song?: string | null"));
    assert!(!output.contains("Prop<")); // never leak Rust-only types

    // Input type - what the frontend SENDS
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
// real interface - the definition is right there in the source. Only types
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
// children). The generator must still EMIT the interface - a self-edge is not a
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
// `unknown` - regression coverage for the v0.7.1 behavior that clobbered
// committed types files with weaker output.

const NESTED_SRC: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct AdminArticlesIndexProps {
    pub articles: Vec<AdminArticleRow>,
    pub external: uuid::Uuid,
    pub json: serde_json::Value,
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

/// `serde_json::Value` used to be this file's example of an external type
/// that degrades to `unknown`. It is the one external type the generator
/// does know, so it moved out of that test and into this one.
#[test]
fn serde_json_value_resolves_to_the_json_alias() {
    let ts = generate_types_string(ScanInput::Source(NESTED_SRC));
    assert!(
        ts.contains("export type JsonValue ="),
        "the alias must be declared when something references it: {ts}"
    );
    let props = extract_block(&ts, "AdminArticlesIndexProps");
    assert!(
        props.contains("json: JsonValue"),
        "a JSON document is not an unknown type: {props}"
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

// A plain struct reached through a prop is serialized by serde's own derive,
// so its `#[serde(...)]` attributes decide the keys on the wire. These tests
// use serde itself as the oracle: `scanned!` declares the items for real and
// keeps their source text after a props root that reaches them, the generator
// scans that text, and the keys of the emitted interface are compared with the
// keys `serde_json` actually sends.
macro_rules! scanned {
    (root: $root:literal; $($item:item)*) => {
        $($item)*
        const SCANNED_SRC: &str = concat!($root, stringify!($($item)*));
    };
}

/// The keys of one emitted interface, each with whether it is optional. A
/// quoted key is read as one JSON string, so a key holding `: ` stays whole.
fn declared_keys(ts: &str, name: &str) -> std::collections::BTreeMap<String, bool> {
    extract_block(ts, name)
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "}")
        .map(|line| {
            let (key, rest) = if line.starts_with('"') {
                let mut stream = serde_json::Deserializer::from_str(line).into_iter::<String>();
                let key = stream
                    .next()
                    .expect("a quoted key")
                    .expect("a quoted key is a JSON string");
                (key, &line[stream.byte_offset()..])
            } else {
                let end = line.find(['?', ':']).expect("a key ends at `?` or `:`");
                (line[..end].to_owned(), &line[end..])
            };
            (key, rest.starts_with('?'))
        })
        .collect()
}

/// Every key serde sends is declared, and every key declared as required is
/// sent.
fn assert_interface_matches_serde<T: serde::Serialize>(ts: &str, name: &str, value: &T) {
    let declared = declared_keys(ts, name);
    let json = serde_json::to_value(value).expect("the value serializes");
    let sent: std::collections::BTreeSet<String> = json
        .as_object()
        .expect("a struct serializes to an object")
        .keys()
        .cloned()
        .collect();
    for key in &sent {
        assert!(
            declared.contains_key(key),
            "serde sends `{key}` but `{name}` does not declare it: {declared:?}"
        );
    }
    for (key, optional) in &declared {
        assert!(
            *optional || sent.contains(key),
            "`{name}` requires `{key}` but serde does not send it: {sent:?}"
        );
    }
}

#[test]
fn a_plain_struct_declares_exactly_the_keys_serde_sends() {
    scanned! {
        root: "#[derive(suprnova::InertiaProps)] pub struct CardsProps { pub cards: Vec<Card> }";
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "camelCase")]
        pub struct Card {
            pub title: String,
            #[serde(skip)]
            pub revision_id: i64,
            #[serde(skip_serializing)]
            pub internal_note: String,
            #[serde(rename = "display-name")]
            pub display_name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub badge_count: Option<u32>,
            #[serde(rename(serialize = "kind", deserialize = "type"))]
            pub r#type: String,
            pub r#loop: bool,
            pub created_at: String,
            #[serde(rename = "note")]
            #[serde(default, skip_serializing_if = "String::is_empty", alias = "memo")]
            pub note_text: String,
            #[serde(rename(deserialize = "legacy_owner"))]
            pub owner_id: i64,
        }
    }
    let ts = generate_types_string(ScanInput::Source(SCANNED_SRC));

    // The skipped fields carry data serde must leave out, so the comparison
    // below is not passing over empty values.
    let card = Card {
        revision_id: 918_273_645,
        internal_note: "draft only".into(),
        ..Card::default()
    };
    let Card {
        revision_id,
        internal_note,
        ..
    } = &card;
    let sent = serde_json::to_string(&card).expect("the card serializes");
    assert!(!sent.contains(&revision_id.to_string()), "sent: {sent}");
    assert!(!sent.contains(internal_note.as_str()), "sent: {sent}");

    assert_interface_matches_serde(&ts, "Card", &card);
    assert_interface_matches_serde(
        &ts,
        "Card",
        &Card {
            badge_count: Some(3),
            note_text: "pinned".into(),
            ..Card::default()
        },
    );

    let card = extract_block(&ts, "Card");
    assert!(card.contains("  \"display-name\": string;"), "got: {card}");
    assert!(
        card.contains("  badgeCount?: number | null;"),
        "got: {card}"
    );
    assert!(card.contains("  kind: string;"), "got: {card}");
    assert!(card.contains("  loop: boolean;"), "got: {card}");
    assert!(card.contains("  note?: string;"), "got: {card}");
    assert!(card.contains("  ownerId: number;"), "got: {card}");
    assert!(!card.contains("revision"), "got: {card}");
    assert!(!card.contains("internal"), "got: {card}");
}

#[test]
fn every_rename_all_rule_names_the_keys_serde_sends() {
    scanned! {
        root: "#[derive(suprnova::InertiaProps)] pub struct RulesProps { \
               pub a: Lower, pub b: Upper, pub c: Pascal, pub d: Camel, pub e: Snake, \
               pub f: ScreamingSnake, pub g: Kebab, pub h: ScreamingKebab, pub i: SerializeOnly }";
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "lowercase")]
        pub struct Lower { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "UPPERCASE")]
        pub struct Upper { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "PascalCase")]
        pub struct Pascal { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "camelCase")]
        pub struct Camel { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "snake_case")]
        pub struct Snake { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
        pub struct ScreamingSnake { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "kebab-case")]
        pub struct Kebab { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all = "SCREAMING-KEBAB-CASE")]
        pub struct ScreamingKebab { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
        #[derive(serde::Serialize, Default)]
        #[serde(rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
        pub struct SerializeOnly { pub user_id: i64, pub name_2: i64, pub created_by_user: i64 }
    }
    let ts = generate_types_string(ScanInput::Source(SCANNED_SRC));

    assert_interface_matches_serde(&ts, "Lower", &Lower::default());
    assert_interface_matches_serde(&ts, "Upper", &Upper::default());
    assert_interface_matches_serde(&ts, "Pascal", &Pascal::default());
    assert_interface_matches_serde(&ts, "Camel", &Camel::default());
    assert_interface_matches_serde(&ts, "Snake", &Snake::default());
    assert_interface_matches_serde(&ts, "ScreamingSnake", &ScreamingSnake::default());
    assert_interface_matches_serde(&ts, "Kebab", &Kebab::default());
    assert_interface_matches_serde(&ts, "ScreamingKebab", &ScreamingKebab::default());
    assert_interface_matches_serde(&ts, "SerializeOnly", &SerializeOnly::default());
}

/// `InertiaProps` and `Data` write their own `Serialize` and never read
/// `#[serde(...)]`, so the keys of a derived struct stay the Rust names, the
/// `r#` of a raw identifier included. Were the generator to apply serde's
/// attributes here, the interface would name keys the server never sends.
#[test]
fn a_derived_struct_keeps_the_names_its_derive_sends() {
    const DERIVED_SRC: &str = r#"
#[derive(serde::Deserialize, suprnova::InertiaProps)]
#[serde(rename_all = "camelCase")]
pub struct ProfileProps {
    pub display_name: String,
    #[serde(skip)]
    pub revision_id: i64,
    pub r#type: String,
}

#[derive(suprnova::Data)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    pub display_name: String,
    #[serde(rename = "id")]
    pub profile_id: i64,
}
"#;
    let ts = generate_types_string(ScanInput::Source(DERIVED_SRC));
    let props = extract_block(&ts, "ProfileProps");
    assert!(props.contains("  display_name: string;"), "got: {props}");
    assert!(props.contains("  revision_id: number;"), "got: {props}");
    assert!(props.contains("  \"r#type\": string;"), "got: {props}");
    let dto = extract_block(&ts, "ProfileDto");
    assert!(dto.contains("  display_name: string;"), "got: {dto}");
    assert!(dto.contains("  profile_id: number;"), "got: {dto}");
}

#[test]
fn a_plain_struct_whose_fields_serde_all_skips_is_an_empty_interface() {
    scanned! {
        root: "#[derive(suprnova::InertiaProps)] pub struct MarkerProps { pub marker: Marker }";
        #[derive(serde::Serialize, Default)]
        pub struct Marker {
            #[serde(skip)]
            pub cache: Vec<u8>,
        }
    }
    let ts = generate_types_string(ScanInput::Source(SCANNED_SRC));

    let marker = Marker {
        cache: vec![1, 2, 3],
    };
    assert_eq!(marker.cache.len(), 3);
    assert_interface_matches_serde(&ts, "Marker", &marker);
    let props = extract_block(&ts, "MarkerProps");
    assert!(props.contains("  marker: Marker;"), "got: {props}");
}

/// A Data Object carrying a model's dates: chrono serializes each one as
/// ISO 8601 text, so each is a `string`, not `unknown`.
#[test]
fn chrono_dates_are_strings() {
    const CHRONO_SRC: &str = r#"
use chrono::{DateTime, NaiveDate, Utc};

#[derive(suprnova::Data)]
pub struct EventDto {
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub day: NaiveDate,
    pub local: chrono::NaiveDateTime,
    pub doors: chrono::NaiveTime,
}
"#;
    let ts = generate_types_string(ScanInput::Source(CHRONO_SRC));
    let dto = extract_block(&ts, "EventDto");
    for line in [
        "  starts_at: string;",
        "  ends_at: string | null;",
        "  day: string;",
        "  local: string;",
        "  doors: string;",
    ] {
        assert!(dto.contains(line), "{line} in {dto}");
    }
    assert!(!ts.contains("unknown"), "got: {ts}");
}
