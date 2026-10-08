//! TS extraction across Data derives:
//!   - Field<T>  → `field?: T | null`
//!   - Prop<T>   → `field?: T`         (lazy/deferred - may be absent)
//!   - input_only → excluded from generated output type
//!   - output_only → included in output type, excluded from input type
//!   - allow_include → no TS effect (runtime-only)

use suprnova_cli::commands::generate_types::{
    GenerateOptions, ScanInput, generate_types_string, generate_types_string_with,
};

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

/// `InertiaProps` honors serde's renames and skips in the `Serialize` it
/// writes, and drops a raw identifier's `r#`, as serde does, so a derived
/// struct's interface declares exactly the keys the derive sends. Checked
/// against the derive's own output.
#[test]
fn an_inertia_props_struct_declares_the_keys_its_derive_sends() {
    scanned! {
        root: "";
        #[derive(suprnova::InertiaProps)]
        #[serde(rename_all = "camelCase")]
        pub struct ProfileProps {
            pub display_name: String,
            #[serde(skip)]
            pub revision_id: i64,
            #[serde(rename = "kind")]
            pub r#type: String,
            pub r#match: String,
        }
    }
    let ts = generate_types_string(ScanInput::Source(SCANNED_SRC));
    let props = ProfileProps {
        display_name: "Ada".into(),
        revision_id: 1,
        r#type: "admin".into(),
        r#match: "exact".into(),
    };
    assert_interface_matches_serde(&ts, "ProfileProps", &props);
    let declared = declared_keys(&ts, "ProfileProps");
    assert_eq!(
        declared.keys().map(String::as_str).collect::<Vec<_>>(),
        ["displayName", "kind", "match"]
    );
    assert_eq!(props.revision_id, 1);
}

/// A `Data` struct is read as well as written, so its `Input` interface
/// declares the keys its derive reads: the deserialize names, without the
/// fields serde skips when deserializing. The derive's keys are pinned
/// against serde's own derive in `suprnova-macros/tests/serde_names.rs`.
#[test]
fn a_data_struct_declares_the_keys_it_writes_and_reads() {
    const DATA_SRC: &str = r#"
#[derive(suprnova::Data)]
#[serde(rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
pub struct ProfileDto {
    pub display_name: String,
    #[serde(rename = "id")]
    pub profile_id: i64,
    #[serde(skip_deserializing)]
    pub computed: String,
    #[serde(skip_serializing)]
    pub secret: String,
}
"#;
    let ts = generate_types_string(ScanInput::Source(DATA_SRC));
    let output = declared_keys(&ts, "ProfileDto");
    assert_eq!(
        output.keys().map(String::as_str).collect::<Vec<_>>(),
        ["computed", "displayName", "id"]
    );
    let input = declared_keys(&ts, "ProfileDtoInput");
    assert_eq!(
        input.keys().map(String::as_str).collect::<Vec<_>>(),
        ["display-name", "id", "secret"]
    );
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

// PAR-069: the wide integers - `i64`, `u64`, `i128`, `u128`, `isize` and
// `usize` - can pass 2^53, and with `preserve_big_integers` on, one that
// does travels as a `{"$bigint": ".."}` marker the Inertia client turns
// into a `BigInt`. So they are `number | bigint` exactly when the project
// preserves big integers, and `number` otherwise. Narrower integers and
// floats are always `number`.

/// One prop struct with every numeric primitive, wrapped where the wide
/// type changes how it composes.
const NUMBERS: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Numbers {
    pub a_i8: i8,
    pub a_i16: i16,
    pub a_i32: i32,
    pub a_u8: u8,
    pub a_u16: u16,
    pub a_u32: u32,
    pub a_f32: f32,
    pub a_f64: f64,
    pub a_i64: i64,
    pub a_u64: u64,
    pub a_i128: i128,
    pub a_u128: u128,
    pub a_isize: isize,
    pub a_usize: usize,
    pub maybe: Option<u64>,
    pub list: Vec<i64>,
    pub by_id: std::collections::HashMap<u64, i64>,
}
"#;

const NARROW: [&str; 8] = [
    "  a_i8: number;",
    "  a_i16: number;",
    "  a_i32: number;",
    "  a_u8: number;",
    "  a_u16: number;",
    "  a_u32: number;",
    "  a_f32: number;",
    "  a_f64: number;",
];

const WIDE: [&str; 6] = ["a_i64", "a_u64", "a_i128", "a_u128", "a_isize", "a_usize"];

fn assert_narrow_stay_number(block: &str) {
    for line in NARROW {
        assert!(block.contains(line), "{line} in:\n{block}");
    }
}

fn assert_wide(block: &str, ts_type: &str) {
    for field in WIDE {
        let line = format!("  {field}: {ts_type};");
        assert!(block.contains(&line), "{line} in:\n{block}");
    }
}

#[test]
fn intt_wide_integers_are_number_when_nothing_preserves_big_integers() {
    let ts = generate_types_string(ScanInput::Source(Box::leak(
        format!("{NUMBERS}\nfn boot(config: InertiaConfig) -> InertiaConfig {{ config.preserve_big_integers(false) }}\n")
            .into_boxed_str(),
    )));
    let block = extract_block(&ts, "Numbers");
    assert_wide(&block, "number");
    assert_narrow_stay_number(&block);
    assert!(block.contains("  maybe: number | null;"), "{block}");
    assert!(block.contains("  list: Array<number>;"), "{block}");
    assert!(
        !ts.contains("bigint"),
        "a literal `false` preserves nothing:\n{ts}"
    );
}

#[test]
fn intt_wide_integers_widen_when_src_preserves_big_integers() {
    for call in [
        "InertiaConfig::new().preserve_big_integers(true)",
        "response.preserve_big_integers(on)",
        "InertiaConfig::preserve_big_integers(config, true)",
    ] {
        let source = format!("{NUMBERS}\nfn boot() {{ let _ = {call}; }}\n");
        let ts = generate_types_string(ScanInput::Source(Box::leak(source.into_boxed_str())));
        let block = extract_block(&ts, "Numbers");
        assert_wide(&block, "number | bigint");
        assert_narrow_stay_number(&block);
        assert!(
            block.contains("  maybe: number | bigint | null;"),
            "{call}:\n{block}"
        );
        assert!(
            block.contains("  list: Array<number | bigint>;"),
            "{call}:\n{block}"
        );
        // A map key is always a JSON string on the wire and never a
        // `$bigint` marker, and TypeScript refuses `bigint` as a key type.
        assert!(
            block.contains("  by_id: Record<number, number | bigint>;"),
            "{call}:\n{block}"
        );
    }
}

#[test]
fn intt_a_preserve_call_inside_a_macro_counts_too() {
    let source = format!(
        "{NUMBERS}\nfn boot() {{ bind!(InertiaConfig::new().preserve_big_integers(true)); }}\n"
    );
    let ts = generate_types_string(ScanInput::Source(Box::leak(source.into_boxed_str())));
    assert_wide(&extract_block(&ts, "Numbers"), "number | bigint");
}

/// The wide integers of `NUMBERS` with `boot`, a function holding `body`,
/// beside them.
fn numbers_with_boot(body: &str) -> String {
    let source =
        format!("{NUMBERS}\nfn boot(config: InertiaConfig) -> InertiaConfig {{ {body} }}\n");
    generate_types_string(ScanInput::Source(Box::leak(source.into_boxed_str())))
}

#[test]
fn intt_a_parenthesized_false_preserves_nothing() {
    for call in [
        "InertiaConfig::new().preserve_big_integers((false))",
        "config.preserve_big_integers(((false)))",
        "InertiaConfig::preserve_big_integers(config, (false))",
    ] {
        let ts = numbers_with_boot(call);
        assert_wide(&extract_block(&ts, "Numbers"), "number");
        assert!(!ts.contains("bigint"), "`{call}` preserves nothing:\n{ts}");
    }
}

#[test]
fn intt_a_parenthesized_false_inside_a_macro_preserves_nothing() {
    for call in [
        "bind!(InertiaConfig::new().preserve_big_integers((false))); config",
        "bind!(config.preserve_big_integers(((false)))); config",
    ] {
        let ts = numbers_with_boot(call);
        assert_wide(&extract_block(&ts, "Numbers"), "number");
        assert!(!ts.contains("bigint"), "`{call}` preserves nothing:\n{ts}");
    }
}

/// A temporary project for the binary: a manifest, so `generate-types`
/// takes the directory as a project, and `files` under `src/`.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("create workspace tempdir");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"types\"\n",
    )
    .expect("write manifest");
    for (path, body) in files {
        let path = dir.path().join("src").join(path);
        std::fs::create_dir_all(path.parent().expect("a file under src/"))
            .expect("create source directory");
        std::fs::write(path, body).expect("write source");
    }
    dir
}

fn run_generate_types(dir: &tempfile::TempDir, args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .arg("generate-types")
        .args(args)
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova binary");
    assert!(
        out.status.success(),
        "generate-types {args:?} must succeed; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(dir.path().join("frontend/src/types/inertia-props.ts"))
        .expect("read generated file")
}

#[test]
fn intt_the_big_integers_flag_widens_without_a_preserve_call() {
    let dir = project(&[("props.rs", NUMBERS)]);

    let without = run_generate_types(&dir, &[]);
    assert_wide(&extract_block(&without, "Numbers"), "number");

    let with = run_generate_types(&dir, &["--big-integers"]);
    let block = extract_block(&with, "Numbers");
    assert_wide(&block, "number | bigint");
    assert_narrow_stay_number(&block);
}

/// The PAR-069 falsifier as written: the call sits in `src/bootstrap.rs`
/// and the struct in another file; preservation is a project-wide fact.
#[test]
fn intt_a_preserve_call_in_bootstrap_widens_every_file() {
    let dir = project(&[
        ("props.rs", NUMBERS),
        (
            "bootstrap.rs",
            "pub fn register() {\n    Inertia::install(&InertiaConfig::new().preserve_big_integers(true))\n        .expect(\"install\");\n}\n",
        ),
    ]);
    let block = extract_block(&run_generate_types(&dir, &[]), "Numbers");
    assert_wide(&block, "number | bigint");
    assert_narrow_stay_number(&block);
}

#[test]
fn intt_the_big_integers_option_widens_an_in_memory_scan() {
    let ts = generate_types_string_with(
        ScanInput::Source(NUMBERS),
        GenerateOptions { big_integers: true },
    );
    let block = extract_block(&ts, "Numbers");
    assert_wide(&block, "number | bigint");
    assert_narrow_stay_number(&block);
}

// PAR-068: beside the props interfaces, `inertia-props.ts` maps each page
// component to its props (`Pages`), types the props every page shares
// (`SharedProps`), and augments `@inertiajs/core`'s `InertiaConfig` so
// `usePage()` is typed with no argument.

/// Generate the types of a project holding `files` under `src/`, the way
/// `generate-types` does: the file it writes, or the error it reports.
fn project_types(files: &[(&str, &str)]) -> Result<String, String> {
    let dir = project(files);
    let out = dir.path().join("frontend/src/types/inertia-props.ts");
    suprnova_cli::commands::generate_types::generate_types_to_file(
        dir.path(),
        &out,
        GenerateOptions::default(),
    )?;
    Ok(std::fs::read_to_string(&out).expect("read generated file"))
}

fn generated(files: &[(&str, &str)]) -> String {
    project_types(files).unwrap_or_else(|error| panic!("generation failed: {error}"))
}

/// The declaration that starts with `head` and ends at its closing line.
fn declaration(ts: &str, head: &str) -> String {
    let start = ts
        .find(head)
        .unwrap_or_else(|| panic!("`{head}` not found in:\n{ts}"));
    let after = &ts[start..];
    let end = after.find("\n}\n").map_or(after.len(), |end| end + 3);
    after[..end].to_string()
}

const HOME: &str = r#"
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps { title: "Hi".into() })
}

#[handler]
pub async fn about(req: Request) -> Response {
    inertia_response!(&req, "About", { "team_size": 4 })
}
"#;

const AUTH: &str = r#"
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
pub struct LoginProps {}

impl LoginProps {
    pub fn new() -> Self { Self {} }
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps::new())
}
"#;

const USERS: &str = r#"
use suprnova::{handler, Data, Inertia, InertiaResponse, Request, Response};

#[derive(Data)]
pub struct UserDto {
    pub id: i64,
    pub name: String,
}

#[handler]
pub async fn show(req: Request) -> Response {
    InertiaResponse::new("Users/Show")
        .with_data(UserDto { id: 1, name: "Ada".into() })
        .title("Ada")
        .resolve(&req)
        .await
}

#[handler]
pub async fn edit(req: Request) -> Response {
    Inertia::data("Users/Edit", UserDto { id: 1, name: "Ada".into() }).resolve(&req).await
}

#[handler]
pub async fn index(req: Request) -> Response {
    InertiaResponse::new("Users/Index").with("users", Vec::<String>::new()).resolve(&req).await
}
"#;

#[test]
fn intt_pages_map_each_component_to_the_struct_it_renders_with() {
    let ts = generated(&[
        ("controllers/home.rs", HOME),
        ("controllers/auth.rs", AUTH),
        ("controllers/users.rs", USERS),
    ]);
    assert_eq!(
        declaration(&ts, "export interface Pages {"),
        "export interface Pages {\n  \"Home\": HomeProps;\n  \"Users/Edit\": UserDto;\n  \
         \"Users/Show\": UserDto;\n  \"auth/Login\": LoginProps;\n}\n",
        "one quoted entry per typed component, sorted by name; `About` and \
         `Users/Index` render JSON props only:\n{ts}"
    );
}

#[test]
fn intt_page_props_join_a_page_its_shared_props_and_the_errors() {
    let ts = generated(&[("controllers/home.rs", HOME)]);
    assert!(
        ts.contains("export type Errors = Record<string, string>;\n"),
        "{ts}"
    );
    assert!(
        ts.contains(
            "export type PageProps<C extends keyof Pages> = Pages[C] & SharedProps & { errors: Errors };\n"
        ),
        "{ts}"
    );
}

#[test]
fn intt_the_augmentation_types_inertias_config() {
    let ts = generated(&[("controllers/home.rs", HOME)]);
    assert!(
        ts.starts_with(
            "// This file is auto-generated by Suprnova. Do not edit manually.\n\
             // Run `suprnova generate-types` to regenerate.\n\n\
             import '@inertiajs/core';\n\n"
        ),
        "the import opens the file, after the header:\n{ts}"
    );
    assert!(
        ts.ends_with(
            "declare module '@inertiajs/core' {\n  export interface InertiaConfig {\n    \
             sharedPageProps: SharedProps;\n    errorValueType: string;\n  }\n}\n"
        ),
        "without a flash struct, Inertia's own flash type applies:\n{ts}"
    );
}

#[test]
fn intt_shared_props_carry_root_and_the_struct_share_data_is_given() {
    let ts = generated(&[
        ("controllers/home.rs", HOME),
        (
            "bootstrap.rs",
            r#"
use suprnova::{Data, Inertia, inertia::Prop};

#[derive(Data)]
pub struct AppShared {
    pub app_name: String,
    pub root: String,
    #[data(lazy)]
    pub stats: Prop<i64>,
}

pub fn register() {
    Inertia::share_data(AppShared { app_name: "Suprnova".into(), root: String::new(), stats: Prop::default() })
        .expect("shared");
    Inertia::share("appVersion", "1.0").expect("shared");
}
"#,
        ),
    ]);
    assert_eq!(
        declaration(&ts, "export interface SharedProps {"),
        "export interface SharedProps {\n  root: string;\n  app_name: string;\n}\n",
        "the framework's `root`, then the struct's eager fields; a lazy field is \
         never shared and a per-key share is not typed:\n{ts}"
    );
}

/// `Inertia::share_data` is the struct form of sharing. `Inertia::share`
/// takes a key and a value, so a struct given as that value is one shared
/// prop under its key, never the shared props themselves.
#[test]
fn intt_a_struct_shared_under_a_key_is_not_the_shared_struct() {
    let ts = generated(&[
        ("controllers/home.rs", HOME),
        (
            "bootstrap.rs",
            r#"
use suprnova::{Data, Inertia};

#[derive(Data)]
pub struct Settings {
    pub theme: String,
}

pub fn register() {
    Inertia::share("settings", Settings { theme: "dark".into() }).expect("shared");
    let settings = Settings { theme: "light".into() };
    Inertia::share("fallback", settings).expect("shared");
}
"#,
        ),
    ]);
    assert_eq!(
        declaration(&ts, "export interface SharedProps {"),
        "export interface SharedProps {\n  root: string;\n}\n",
        "a per-key share types nothing; only `Inertia::share_data` names the \
         shared struct:\n{ts}"
    );
}

#[test]
fn intt_shared_props_read_the_shared_marker() {
    let ts = generated(&[
        ("controllers/home.rs", HOME),
        (
            "shared.rs",
            r#"
#[derive(suprnova::InertiaProps)]
#[inertia_props(shared)]
pub struct AppShared {
    pub app_name: String,
    pub user_id: Option<u64>,
}
"#,
        ),
    ]);
    assert_eq!(
        declaration(&ts, "export interface SharedProps {"),
        "export interface SharedProps {\n  root: string;\n  app_name: string;\n  user_id: number | null;\n}\n",
        "{ts}"
    );
}

#[test]
fn intt_shared_props_hold_root_alone_without_a_shared_struct() {
    let ts = generated(&[("controllers/home.rs", HOME)]);
    assert_eq!(
        declaration(&ts, "export interface SharedProps {"),
        "export interface SharedProps {\n  root: string;\n}\n",
        "{ts}"
    );
}

const TOAST: &str = r#"
#[derive(suprnova::InertiaProps)]
#[inertia_props(flash)]
pub struct Toast {
    pub message: String,
}
"#;

#[test]
fn intt_the_flash_marker_names_the_flash_data_type() {
    let with = generated(&[("controllers/home.rs", HOME), ("flash.rs", TOAST)]);
    assert!(
        with.contains(
            "    sharedPageProps: SharedProps;\n    errorValueType: string;\n    flashDataType: Toast;\n"
        ),
        "{with}"
    );
    assert!(with.contains("export interface Toast {"), "{with}");

    let without = generated(&[("controllers/home.rs", HOME)]);
    assert!(!without.contains("flashDataType"), "{without}");
}

#[test]
fn intt_a_component_rendered_with_two_structs_is_an_error_naming_both() {
    let error = project_types(&[
        ("controllers/home.rs", HOME),
        (
            "controllers/landing.rs",
            r#"
#[derive(suprnova::InertiaProps)]
pub struct LandingProps { pub hero: String }

pub async fn landing(req: Request) -> Response {
    inertia_response!(&req, "Home", LandingProps { hero: "x".into() })
}
"#,
        ),
    ])
    .expect_err("one page has one props type");
    for part in [
        "`Home`",
        "`HomeProps`",
        "`LandingProps`",
        "src/controllers/home.rs",
        "src/controllers/landing.rs",
    ] {
        assert!(error.contains(part), "{part} in: {error}");
    }
}

#[test]
fn intt_two_shared_structs_are_an_error_naming_both() {
    let error = project_types(&[(
        "shared.rs",
        r#"
#[derive(suprnova::InertiaProps)]
#[inertia_props(shared)]
pub struct AppShared { pub app_name: String }

#[derive(suprnova::Data)]
pub struct OtherShared { pub theme: String }

pub fn register() {
    Inertia::share_data(OtherShared { theme: "dark".into() }).expect("shared");
}
"#,
    )])
    .expect_err("one struct types the shared props");
    for part in ["`AppShared`", "`OtherShared`", "shared"] {
        assert!(error.contains(part), "{part} in: {error}");
    }
}

#[test]
fn intt_two_flash_structs_are_an_error_naming_both() {
    let error = project_types(&[
        ("flash.rs", TOAST),
        (
            "notice.rs",
            r#"
#[derive(suprnova::InertiaProps)]
#[inertia_props(flash)]
pub struct Notice { pub text: String }
"#,
        ),
    ])
    .expect_err("one struct types the flash data");
    for part in ["`Toast`", "`Notice`", "flash"] {
        assert!(error.contains(part), "{part} in: {error}");
    }
}

#[test]
fn intt_a_conflict_leaves_the_written_file_alone() {
    let dir = project(&[("controllers/home.rs", HOME)]);
    let before = run_generate_types(&dir, &[]);
    std::fs::write(
        dir.path().join("src/controllers/landing.rs"),
        "#[derive(suprnova::InertiaProps)]\npub struct LandingProps { pub hero: String }\n\
         pub async fn landing(req: Request) -> Response {\n    \
         inertia_response!(&req, \"Home\", LandingProps { hero: \"x\".into() })\n}\n",
    )
    .expect("add a conflicting render");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .arg("generate-types")
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova binary");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "a conflict fails the command: {printed}"
    );
    assert!(
        printed.contains("`HomeProps`") && printed.contains("`LandingProps`"),
        "the command prints both structs: {printed}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("frontend/src/types/inertia-props.ts"))
            .expect("read generated file"),
        before,
        "the last complete output stays"
    );
}

#[test]
fn intt_a_page_rendered_with_a_plain_struct_is_typed_by_it() {
    let ts = generated(&[(
        "controllers/report.rs",
        r#"
#[derive(serde::Serialize)]
pub struct ReportProps {
    pub total: u32,
}

pub async fn show(req: Request) -> Response {
    inertia_response!(&req, "Report", ReportProps { total: 3 })
}
"#,
    )]);
    assert!(
        ts.contains("export interface ReportProps {\n  total: number;\n}"),
        "a struct that derives only Serialize is emitted once a page renders it:\n{ts}"
    );
    assert!(ts.contains("  \"Report\": ReportProps;\n"), "{ts}");
}

#[test]
fn intt_a_generic_props_struct_gets_no_pages_entry() {
    let ts = generated(&[(
        "controllers/list.rs",
        r#"
#[derive(suprnova::InertiaProps)]
pub struct Listing<T> {
    pub items: Vec<T>,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Listing", Listing::<String> { items: Vec::new() })
}
"#,
    )]);
    assert!(ts.contains("export interface Listing<T> {"), "{ts}");
    assert_eq!(
        declaration(&ts, "export interface Pages {"),
        "export interface Pages {\n}\n",
        "`Pages` cannot name `Listing` without its type arguments:\n{ts}"
    );
}

// A page rendered with a local binding or a parameter is typed by the
// struct the binding holds, which is what the macro renders.

/// The `Pages` declaration of a project holding one controller.
fn pages_of(controller: &str) -> String {
    declaration(
        &generated(&[("controllers/pages.rs", controller)]),
        "export interface Pages {",
    )
}

#[test]
fn intt_a_typed_local_binding_at_a_render_site_types_the_page() {
    let pages = pages_of(
        r#"
use suprnova::{handler, inertia_response, InertiaProps, InertiaResponse, Request, Response};

#[derive(InertiaProps)]
pub struct HomeProps { pub title: String }

#[derive(InertiaProps)]
pub struct AboutProps { pub team_size: u32 }

#[derive(InertiaProps)]
pub struct ContactProps { pub email: String }

fn load_about() -> AboutProps { AboutProps { team_size: 4 } }

#[handler]
pub async fn index(req: Request) -> Response {
    let props: HomeProps = HomeProps { title: "Hi".into() };
    inertia_response!(&req, "Home", props)
}

#[handler]
pub async fn welcome(req: Request) -> Response {
    let props: HomeProps = HomeProps { title: "Hi".into() };
    InertiaResponse::new("Welcome").with_data(props).resolve(&req).await
}

#[handler]
pub async fn about(req: Request) -> Response {
    let props: AboutProps = load_about();
    inertia_response!(&req, "About", props)
}

#[handler]
pub async fn contact(req: Request) -> Response {
    let props = ContactProps::new();
    inertia_response!(&req, "Contact", props)
}
"#,
    );
    assert_eq!(
        pages,
        "export interface Pages {\n  \"About\": AboutProps;\n  \"Contact\": ContactProps;\n  \
         \"Home\": HomeProps;\n  \"Welcome\": HomeProps;\n}\n",
        "a binding is typed by its declared type, or else by the struct its \
         initializer builds"
    );
}

#[test]
fn intt_a_typed_parameter_at_a_render_site_types_the_page() {
    let pages = pages_of(
        r#"
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
pub struct HomeProps { pub title: String }

#[derive(InertiaProps)]
pub struct DashboardProps { pub visits: u32 }

#[derive(InertiaProps)]
pub struct SettingsProps { pub theme: String }

pub async fn show(req: Request, props: HomeProps) -> Response {
    inertia_response!(&req, "Home", props)
}

pub struct Renderer;

impl Renderer {
    pub fn dashboard(&self, req: &Request, props: &DashboardProps) -> Response {
        inertia_response!(req, "Dashboard", props)
    }
}

pub async fn settings(req: Request) -> Response {
    let render = |props: SettingsProps| inertia_response!(&req, "Settings", props);
    render(SettingsProps { theme: "dark".into() })
}
"#,
    );
    assert_eq!(
        pages,
        "export interface Pages {\n  \"Dashboard\": DashboardProps;\n  \
         \"Home\": HomeProps;\n  \"Settings\": SettingsProps;\n}\n",
        "a function, method or closure parameter is typed by its declared type"
    );
}

#[test]
fn intt_a_shadowed_name_is_typed_by_the_binding_in_force() {
    let pages = pages_of(
        r#"
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
pub struct DraftProps { pub body: String }

#[derive(InertiaProps)]
pub struct PreviewProps { pub body: String }

#[derive(InertiaProps)]
pub struct PublishedProps { pub url: String }

pub async fn draft(req: Request) -> Response {
    let props = DraftProps { body: String::new() };
    if req.query("preview").is_some() {
        let props = PreviewProps { body: String::new() };
        return inertia_response!(&req, "Preview", props);
    }
    inertia_response!(&req, "Draft", props)
}

pub async fn publish(req: Request) -> Response {
    let props = DraftProps { body: String::new() };
    let props = PublishedProps { url: String::new() };
    inertia_response!(&req, "Published", props)
}

pub async fn reload(req: Request) -> Response {
    let props = DraftProps { body: String::new() };
    let props = props.into_published();
    inertia_response!(&req, "Reloaded", props)
}

pub async fn cached(req: Request, cached: Option<PublishedProps>) -> Response {
    let props = DraftProps { body: String::new() };
    match cached {
        Some(props) => inertia_response!(&req, "Cached", props),
        None => inertia_response!(&req, "Fresh", props),
    }
}
"#,
    );
    assert_eq!(
        pages,
        "export interface Pages {\n  \"Draft\": DraftProps;\n  \"Fresh\": DraftProps;\n  \
         \"Preview\": PreviewProps;\n  \"Published\": PublishedProps;\n}\n",
        "an inner block's binding holds inside the block only, a later `let` \
         shadows an earlier one, and a binding the scan cannot type (`Reloaded`, \
         `Cached`) leaves the page untyped rather than typed by the name it hides"
    );
}

#[test]
fn intt_a_typed_binding_given_to_share_data_types_the_shared_props() {
    let ts = generated(&[(
        "bootstrap.rs",
        r#"
use suprnova::{Data, Inertia};

#[derive(Data)]
pub struct AppShared {
    pub app_name: String,
}

pub fn register() {
    let shared = AppShared { app_name: "Suprnova".into() };
    Inertia::share_data(shared).expect("shared");
}
"#,
    )]);
    assert_eq!(
        declaration(&ts, "export interface SharedProps {"),
        "export interface SharedProps {\n  root: string;\n  app_name: string;\n}\n",
        "{ts}"
    );
}
