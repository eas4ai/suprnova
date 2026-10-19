//! AGENT-015: the schema a kit form tool registers comes from the form's
//! server request type through `suprnova generate-types`, and agrees with
//! the schema the server derives for that type exactly.
//!
//! `tool_schemas_fixture.rs` holds one request type per construct the
//! generator supports. It is compiled here through the real `#[tool_input]`
//! attribute, which derives the server's schema with `schemars`, and the
//! same text goes to the generator. A construct outside the supported set
//! gets no schema: the generator names the type and the reason instead of
//! writing an approximation.
//!
//! The `agents-browser` mechanism runs this binary with `--run-ignored
//! only`, so these tests are ignored to be counted by it, although none of
//! them needs a browser.

use suprnova_cli::commands::tool_schemas::{ToolSchemas, scan_source};

#[allow(dead_code)]
mod fixture {
    use suprnova::tool_input;

    include!("tool_schemas_fixture.rs");
}

const FIXTURE: &str = include_str!("tool_schemas_fixture.rs");

/// Every request type of the fixture, which the generator must describe.
const FIXTURE_TYPES: &[&str] = &[
    "Booleans",
    "CamelCaseFields",
    "ContainerDefault",
    "ContainsRules",
    "DenyUnknown",
    "Documented",
    "Empty",
    "FieldDefaults",
    "FieldRenames",
    "Floats",
    "Formats",
    "KebabCaseFields",
    "LengthRules",
    "LikeTheKitLoginForm",
    "LikeTheKitNoteForm",
    "Lists",
    "Optionals",
    "PascalCaseFields",
    "RangeRules",
    "RenamedContainer",
    "RequiredRules",
    "RulesWithoutSchema",
    "SignedIntegers",
    "Skipped",
    "Strings",
    "TitledByDoc",
    "UnsignedIntegers",
];

fn scan() -> ToolSchemas {
    scan_source(FIXTURE).expect("the fixture parses")
}

/// The generator's schema of `name`, as compact JSON text: with the
/// framework's `serde_json` order-preserving maps, the text compares key
/// order as well as content.
fn generated(scan: &ToolSchemas, name: &str) -> String {
    let refused: Vec<String> = scan
        .refusals
        .iter()
        .map(|refusal| refusal.to_string())
        .collect();
    let schema = scan
        .schemas
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, schema)| schema)
        .unwrap_or_else(|| panic!("no schema for `{name}`; refusals: {refused:#?}"));
    serde_json::to_string(&schema.to_value()).expect("serialize the generated schema")
}

/// The server's schema of `T`: what `#[tool(input = T)]` publishes.
fn server<T>() -> String
where
    T: suprnova::JsonSchema + serde::de::DeserializeOwned + suprnova::Validate,
{
    serde_json::to_string(&suprnova::agents::__input_schema::<T>())
        .expect("serialize the server schema")
}

macro_rules! agreement {
    ($($test:ident => $ty:ident),* $(,)?) => {
        /// The pairs of generated and server schemas, one per fixture type.
        fn every_pair(scan: &ToolSchemas) -> Vec<(&'static str, String, String)> {
            vec![$((stringify!($ty), generated(scan, stringify!($ty)), server::<fixture::$ty>())),*]
        }

        $(
            #[test]
            #[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
            fn $test() {
                let scan = scan();
                assert_eq!(
                    generated(&scan, stringify!($ty)),
                    server::<fixture::$ty>(),
                    "`{}`: the generated schema (left) differs from the server's (right)",
                    stringify!($ty),
                );
            }
        )*
    };
}

agreement! {
    agent_015_schema_agrees_for_strings => Strings,
    agent_015_schema_agrees_for_booleans => Booleans,
    agent_015_schema_agrees_for_signed_integers => SignedIntegers,
    agent_015_schema_agrees_for_unsigned_integers => UnsignedIntegers,
    agent_015_schema_agrees_for_floats => Floats,
    agent_015_schema_agrees_for_options => Optionals,
    agent_015_schema_agrees_for_lists => Lists,
    agent_015_schema_agrees_for_field_renames => FieldRenames,
    agent_015_schema_agrees_for_camel_case_fields => CamelCaseFields,
    agent_015_schema_agrees_for_kebab_case_fields => KebabCaseFields,
    agent_015_schema_agrees_for_pascal_case_fields => PascalCaseFields,
    agent_015_schema_agrees_for_a_renamed_container => RenamedContainer,
    agent_015_schema_agrees_for_field_defaults => FieldDefaults,
    agent_015_schema_agrees_for_a_container_default => ContainerDefault,
    agent_015_schema_agrees_for_skipped_fields => Skipped,
    agent_015_schema_agrees_for_deny_unknown_fields => DenyUnknown,
    agent_015_schema_agrees_for_doc_comments => Documented,
    agent_015_schema_agrees_for_a_doc_heading_title => TitledByDoc,
    agent_015_schema_agrees_for_length_rules => LengthRules,
    agent_015_schema_agrees_for_range_rules => RangeRules,
    agent_015_schema_agrees_for_format_rules => Formats,
    agent_015_schema_agrees_for_contains_rules => ContainsRules,
    agent_015_schema_agrees_for_required_rules => RequiredRules,
    agent_015_schema_agrees_for_rules_schemars_does_not_read => RulesWithoutSchema,
    agent_015_schema_agrees_for_a_struct_without_fields => Empty,
    agent_015_schema_agrees_for_the_kit_note_form_shape => LikeTheKitNoteForm,
    agent_015_schema_agrees_for_the_kit_login_form_shape => LikeTheKitLoginForm,
}

/// The falsifier sweep: the generator describes every fixture type, refuses
/// none, and every one of its schemas equals the server's.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_form_tool_schema_matches_the_server_request_type() {
    let scan = scan();
    assert!(
        scan.refusals.is_empty(),
        "the generator refused fixture types: {:#?}",
        scan.refusals
            .iter()
            .map(|refusal| refusal.to_string())
            .collect::<Vec<_>>()
    );
    let mut keys: Vec<&str> = scan.schemas.iter().map(|(key, _)| key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, FIXTURE_TYPES, "the generator described other types");
    let pairs = every_pair(&scan);
    assert_eq!(pairs.len(), FIXTURE_TYPES.len());
    for (name, generated, server) in pairs {
        assert_eq!(
            generated, server,
            "`{name}`: generated (left) and server (right) differ"
        );
    }
}

/// One request type the generator cannot describe exactly, and the words
/// its refusal must contain besides the type's name.
struct Unsupported {
    name: &'static str,
    source: &'static str,
    reason: &'static str,
}

const UNSUPPORTED: &[Unsupported] = &[
    Unsupported {
        name: "WithStruct",
        source: "#[tool_input] pub struct WithStruct { pub address: Address }",
        reason: "field `address` has the type `Address`",
    },
    Unsupported {
        name: "WithChrono",
        source: "#[request] pub struct WithChrono { pub at: chrono::DateTime<chrono::Utc> }",
        reason: "field `at` has the type `chrono::DateTime<chrono::Utc>`",
    },
    Unsupported {
        name: "WithMap",
        source: "#[tool_input] pub struct WithMap { pub tags: HashMap<String, String> }",
        reason: "field `tags` has the type `HashMap<String, String>`",
    },
    Unsupported {
        name: "WithGeneric",
        source: "#[tool_input] pub struct WithGeneric<T> { pub value: T }",
        reason: "it is generic",
    },
    Unsupported {
        name: "WithTuple",
        source: "#[tool_input] pub struct WithTuple(pub String);",
        reason: "it has no named fields",
    },
    Unsupported {
        name: "WithFlatten",
        source: "#[tool_input] pub struct WithFlatten { #[serde(flatten)] pub rest: Strings }",
        reason: "field `rest` has the serde attribute `flatten`",
    },
    Unsupported {
        name: "WithDefaultPath",
        source: "#[tool_input] pub struct WithDefaultPath { #[serde(default = \"ten\")] pub n: u32 }",
        reason: "field `n` has the serde attribute `default = \"ten\"`",
    },
    Unsupported {
        name: "WithSkipIf",
        source: "#[tool_input] pub struct WithSkipIf { #[serde(skip_serializing_if = \"Option::is_none\")] pub n: Option<u32> }",
        reason: "field `n` has the serde attribute `skip_serializing_if = \"Option::is_none\"`",
    },
    Unsupported {
        name: "WithSchemars",
        source: "#[tool_input] #[schemars(description = \"x\")] pub struct WithSchemars { pub n: u32 }",
        reason: "a `schemars` attribute",
    },
    Unsupported {
        name: "WithFieldSchemars",
        source: "#[tool_input] pub struct WithFieldSchemars { #[schemars(length(min = 1))] pub n: String }",
        reason: "field `n` has a `schemars` attribute",
    },
    Unsupported {
        name: "WithRegex",
        source: "#[tool_input] pub struct WithRegex { #[validate(regex(path = *SLUG))] pub slug: String }",
        reason: "field `slug` has the validation rule `regex`, whose pattern is only known when the program runs",
    },
    Unsupported {
        name: "WithConstantBound",
        source: "#[tool_input] pub struct WithConstantBound { #[validate(length(max = MAX_TITLE))] pub title: String }",
        reason: "field `title` has the bound `MAX_TITLE`",
    },
    Unsupported {
        name: "WithManualDefault",
        source: "#[tool_input] #[serde(default)] pub struct WithManualDefault { pub n: u32 }",
        reason: "`#[serde(default)]` on the struct reads its `Default` implementation",
    },
    Unsupported {
        name: "WithTag",
        source: "#[tool_input] #[serde(tag = \"kind\")] pub struct WithTag { pub n: u32 }",
        reason: "the serde attribute `tag = \"kind\"`",
    },
    Unsupported {
        name: "WithComputedDoc",
        source: "/// Fine.\n#[doc = include_str!(\"doc.md\")]\n#[tool_input] pub struct WithComputedDoc { pub n: u32 }",
        reason: "a doc comment that is not a string literal",
    },
    Unsupported {
        name: "WithRequiredMessage",
        source: "#[tool_input] pub struct WithRequiredMessage { #[validate(required(message = \"x\"))] pub n: Option<u32> }",
        reason: "field `n` has the validation rule `required` with arguments",
    },
    Unsupported {
        name: "WithCfg",
        source: "#[tool_input] pub struct WithCfg { #[cfg(feature = \"x\")] pub n: u32 }",
        reason: "field `n` has the attribute `cfg`",
    },
    Unsupported {
        name: "WithDeprecated",
        source: "#[tool_input] pub struct WithDeprecated { #[deprecated] pub n: u32 }",
        reason: "field `n` has the attribute `deprecated`",
    },
    Unsupported {
        name: "WithUnknownRule",
        source: "#[tool_input] pub struct WithUnknownRule { #[validate(phone)] pub n: String }",
        reason: "field `n` has the validation rule `phone`",
    },
    Unsupported {
        name: "WithStr",
        source: "#[tool_input] pub struct WithStr { pub name: Box<str> }",
        reason: "field `name` has the type `Box<str>`",
    },
    Unsupported {
        name: "WithChar",
        source: "#[tool_input] pub struct WithChar { pub initial: char }",
        reason: "field `initial` has the type `char`",
    },
];

/// The other falsifier half: a request field outside the supported set
/// gets no schema, and the refusal names the type and the reason.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_generate_types_refuses_an_unsupported_request_field() {
    for case in UNSUPPORTED {
        let scan = scan_source(case.source)
            .unwrap_or_else(|error| panic!("`{}` does not parse: {error}", case.name));
        assert!(
            scan.schemas.is_empty(),
            "`{}` got a schema it cannot have: {:?}",
            case.name,
            scan.schemas.iter().map(|(key, _)| key).collect::<Vec<_>>()
        );
        let refusals: Vec<String> = scan.refusals.iter().map(|r| r.to_string()).collect();
        assert_eq!(refusals.len(), 1, "`{}`: {refusals:#?}", case.name);
        assert!(
            refusals[0].contains(&format!("`{}`", case.name)) && refusals[0].contains(case.reason),
            "`{}`: the refusal must name the type and say \"{}\"; it says: {}",
            case.name,
            case.reason,
            refusals[0]
        );
    }
}

/// Two form requests with one name cannot share one key of `toolSchemas`,
/// so both are refused rather than one silently winning.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_generate_types_refuses_two_request_types_of_one_name() {
    let source = "mod a { #[request] pub struct Save { pub n: u32 } }\n\
                  mod b { #[tool_input] pub struct Save { pub title: String } }";
    let scan = scan_source(source).expect("parses");
    assert!(scan.schemas.is_empty(), "{:?}", scan.schemas);
    let refusals: Vec<String> = scan.refusals.iter().map(|r| r.to_string()).collect();
    assert_eq!(refusals.len(), 2, "{refusals:#?}");
    assert!(
        refusals
            .iter()
            .all(|refusal| refusal.contains("`Save`") && refusal.contains("same name")),
        "{refusals:#?}"
    );
}

/// What makes a struct a form request: each attribute a request type can
/// be declared with, and a hand-written `FormRequest` impl, which is how
/// the kits declare theirs. A struct that is none of these gets nothing.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_generate_types_finds_every_form_request() {
    let source = r#"
        #[request] pub struct ByRequest { pub n: u32 }
        #[suprnova::request] pub struct ByQualifiedRequest { pub n: u32 }
        #[tool_input] pub struct ByToolInput { pub n: u32 }
        #[suprnova::tool_input] pub struct ByQualifiedToolInput { pub n: u32 }
        #[derive(Deserialize, Validate, FormRequest)] pub struct ByDerive { pub n: u32 }
        #[derive(Deserialize, Validate, suprnova::FormRequestDerive)] pub struct ByQualifiedDerive { pub n: u32 }
        #[derive(Deserialize, Validate)] pub struct ByImpl { pub n: u32 }
        impl FormRequest for ByImpl {}
        #[derive(Deserialize, Validate)] pub struct ByQualifiedImpl { pub n: u32 }
        impl suprnova::FormRequest for ByQualifiedImpl {
            fn after_validation(&self) -> Result<(), ValidationErrors> { Ok(()) }
        }
        #[derive(Deserialize)] pub struct NotARequest { pub n: u32 }
        #[derive(InertiaProps)] pub struct Props { pub n: u32 }
        impl other::FormRequest for Props {}
    "#;
    let scan = scan_source(source).expect("parses");
    assert!(scan.refusals.is_empty(), "{:?}", scan.refusals);
    let keys: Vec<&str> = scan.schemas.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "ByDerive",
            "ByImpl",
            "ByQualifiedDerive",
            "ByQualifiedImpl",
            "ByQualifiedRequest",
            "ByQualifiedToolInput",
            "ByRequest",
            "ByToolInput",
        ]
    );
}

/// The generated module: one constant keyed by the struct's name, in name
/// order, each schema exactly the JSON the scan holds, and a type of the
/// keys; an empty module when the project has no form request.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_generate_types_writes_the_schemas_module() {
    let scan = scan_source(
        "#[tool_input] pub struct Zed { pub n: u32 }\n\
         /// Says \"hi\".\n#[tool_input] pub struct Alpha { pub tags: Vec<String> }",
    )
    .expect("parses");
    let module = scan.render();
    assert!(
        module.starts_with("// This file is auto-generated by Suprnova."),
        "{module}"
    );
    let alpha = module.find("  Alpha: {").expect("Alpha is a key");
    let zed = module.find("  Zed: {").expect("Zed is a key");
    assert!(alpha < zed, "keys are in name order:\n{module}");
    assert!(
        module.contains("\"description\": \"Says \\\"hi\\\".\""),
        "{module}"
    );
    assert!(module.contains("\"required\": [\"tags\"]"), "{module}");
    assert!(module.contains("\n} as const\n"), "{module}");
    assert!(
        module.ends_with("export type ToolSchemaName = keyof typeof toolSchemas\n"),
        "{module}"
    );

    let empty = scan_source("pub struct Plain { pub n: u32 }")
        .expect("parses")
        .render();
    assert!(
        empty.contains("export const toolSchemas = {} as const\n"),
        "{empty}"
    );
}

/// `suprnova generate-types` writes `frontend/src/types/tool-schemas.ts`
/// beside `inertia-props.ts`, leaves it alone on an identical rerun, and
/// names a request type it refused while still writing the others.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_generate_types_writes_tool_schemas_beside_the_props() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("create a project");
    let project = dir.path();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"smoke\"\n")
        .expect("write the manifest");
    std::fs::create_dir_all(project.join("src/controllers")).expect("create src");
    std::fs::write(
        project.join("src/controllers/posts.rs"),
        "#[derive(Deserialize, Validate)]\n\
         pub struct StorePost {\n    #[validate(length(min = 1, max = 200))]\n    pub title: String,\n}\n\
         impl FormRequest for StorePost {}\n\
         #[request]\npub struct Upload {\n    pub file: UploadedFile,\n}\n",
    )
    .expect("write the controller");
    let run = || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .arg("generate-types")
            .current_dir(project)
            .output()
            .expect("run generate-types");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "generate-types failed: {text}");
        text
    };

    let first = run();
    assert!(
        first.contains("Generated ./frontend/src/types/tool-schemas.ts"),
        "{first}"
    );
    assert!(
        first.contains("No tool schema for `Upload` (src/controllers/posts.rs): field `file` has the type `UploadedFile`"),
        "{first}"
    );
    let module = std::fs::read_to_string(project.join("frontend/src/types/tool-schemas.ts"))
        .expect("read the module");
    let expected = scan_source(
        "#[derive(Deserialize, Validate)]\n\
         pub struct StorePost {\n    #[validate(length(min = 1, max = 200))]\n    pub title: String,\n}\n\
         impl FormRequest for StorePost {}",
    )
    .expect("parses")
    .render();
    assert_eq!(module, expected);
    assert!(!module.contains("Upload"), "{module}");

    let second = run();
    assert!(
        second.contains("./frontend/src/types/tool-schemas.ts is up to date"),
        "{second}"
    );
}
