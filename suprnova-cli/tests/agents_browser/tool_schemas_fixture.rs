// The request types of the AGENT-015 schema agreement tests, read two ways:
// `kits/schemas.rs` compiles this file with `include!`, so each struct goes
// through the real `#[tool_input]` attribute and the server derives its
// schema with `schemars`; and it hands the same text to the generator
// `suprnova generate-types` runs. Every construct the generator supports
// has a struct here, and the two schemas of each must be equal.

use validator::ValidationError;

/// Text fields.
#[tool_input]
pub struct Strings {
    pub title: String,
    pub body: String,
}

#[tool_input]
pub struct Booleans {
    pub published: bool,
}

#[tool_input]
pub struct SignedIntegers {
    pub tiny: i8,
    pub small: i16,
    pub normal: i32,
    pub wide: i64,
    pub huge: i128,
    pub pointer: isize,
}

#[tool_input]
pub struct UnsignedIntegers {
    pub tiny: u8,
    pub small: u16,
    pub normal: u32,
    pub wide: u64,
    pub huge: u128,
    pub pointer: usize,
}

#[tool_input]
pub struct Floats {
    pub ratio: f32,
    pub amount: f64,
}

#[tool_input]
pub struct Optionals {
    pub note: Option<String>,
    pub count: Option<i32>,
    pub flag: Option<bool>,
    pub level: Option<u8>,
    pub nested: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
}

#[tool_input]
pub struct Lists {
    pub tags: Vec<String>,
    pub bytes: Vec<u8>,
    pub scores: Vec<Option<i64>>,
    pub grid: Vec<Vec<bool>>,
}

#[tool_input]
pub struct FieldRenames {
    #[serde(rename = "postTitle")]
    pub title: String,
    #[serde(rename(deserialize = "bodyText"))]
    pub body: String,
    #[serde(rename(serialize = "shownOut"))]
    pub shown: String,
    pub r#type: String,
    #[serde(alias = "legacy_name")]
    pub name: String,
}

#[tool_input]
#[serde(rename_all = "camelCase")]
pub struct CamelCaseFields {
    pub first_name: String,
    pub last_name: Option<String>,
    #[serde(rename = "zip")]
    pub postal_code: String,
}

#[tool_input]
#[serde(rename_all(deserialize = "SCREAMING-KEBAB-CASE"))]
pub struct KebabCaseFields {
    pub first_name: String,
    pub is_admin: bool,
}

#[tool_input]
#[serde(rename_all = "PascalCase")]
pub struct PascalCaseFields {
    pub first_name: String,
    pub id: u32,
}

#[tool_input]
#[serde(rename = "RenamedRequest")]
pub struct RenamedContainer {
    pub value: String,
}

#[tool_input]
pub struct FieldDefaults {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub ratio: f64,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub required: String,
}

#[tool_input]
#[derive(Default)]
#[serde(default)]
pub struct ContainerDefault {
    pub title: String,
    pub count: u16,
    pub note: Option<String>,
    pub tags: Vec<String>,
}

#[tool_input]
pub struct Skipped {
    pub kept: String,
    #[serde(skip)]
    pub hidden: String,
    #[serde(skip_deserializing)]
    pub computed: bool,
}

#[tool_input]
#[serde(deny_unknown_fields)]
pub struct DenyUnknown {
    pub title: String,
    pub draft: Option<bool>,
}

/// A request with documentation.
///
/// The second paragraph keeps its own line breaks,
/// as the derive keeps them.
#[tool_input]
pub struct Documented {
    /// The post's title.
    pub title: String,
    /// # Body
    ///
    /// What the post says.
    pub body: Option<String>,
    #[doc = "Set explicitly."]
    pub explicit: bool,
    ///   Leading spaces beyond the first are kept.
    pub spaced: u32,
    // An empty doc comment, which writes no description.
    #[allow(clippy::empty_docs)]
    ///
    pub empty: String,
}

/// # Titled request
///
/// The heading is the schema's title.
#[tool_input]
pub struct TitledByDoc {
    pub value: String,
}

#[tool_input]
pub struct LengthRules {
    #[validate(length(min = 1, max = 255))]
    pub title: String,
    #[validate(length(equal = 6, message = "Six characters."))]
    pub code: String,
    #[validate(length(min = 2))]
    pub tags: Vec<String>,
    #[validate(length(max = 10000))]
    pub note: Option<String>,
    #[validate(length(min = 1, max = 3))]
    pub picks: Option<Vec<u32>>,
    #[validate(length(min = "4"))]
    pub quoted: String,
}

#[tool_input]
pub struct RangeRules {
    #[validate(range(min = 1, max = 10))]
    pub rating: i32,
    #[validate(range(min = 0.5, max = 99.5))]
    pub price: f64,
    #[validate(range(min = -5))]
    pub offset: i64,
    #[validate(range(max = 200))]
    pub level: Option<u8>,
    #[validate(range(min = 1, exclusive_max = 100))]
    pub percent: u32,
    #[validate(range(max = 1_000_000))]
    pub budget: u64,
}

#[tool_input]
pub struct Formats {
    #[validate(email)]
    pub email: String,
    #[validate(email(message = "Please enter a valid email address", code = "email"))]
    pub backup: Option<String>,
    #[validate(url)]
    pub website: String,
    #[validate(ip)]
    pub address: String,
    #[validate(email, length(min = 3, max = 254))]
    pub contact: String,
}

#[tool_input]
pub struct ContainsRules {
    #[validate(contains(pattern = "a.b+c*d?(e)|[f]{g}^h$#i&j-k~l\\m"))]
    pub marker: String,
    #[validate(contains(pattern = "@"))]
    pub handle: Option<String>,
}

#[tool_input]
pub struct RequiredRules {
    #[validate(required)]
    pub name: Option<String>,
    #[validate(required)]
    pub count: Option<u32>,
    #[validate(required)]
    #[serde(default)]
    pub defaulted: Option<String>,
    #[validate(required, length(min = 1))]
    pub tags: Option<Vec<String>>,
}

fn check_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::new("empty"));
    }
    Ok(())
}

fn check_request(_request: &RulesWithoutSchema) -> Result<(), ValidationError> {
    Ok(())
}

#[tool_input]
#[validate(schema(function = "check_request"))]
pub struct RulesWithoutSchema {
    #[validate(custom(function = "check_name"))]
    pub name: String,
    pub password: String,
    #[validate(must_match(other = "password"))]
    pub password_confirmation: String,
    #[validate(does_not_contain(pattern = "admin"))]
    pub username: String,
}

#[tool_input]
pub struct Empty {}

/// The note form's fields. A missing field reads as empty, so it fails
/// the rules below rather than the parse.
#[tool_input]
pub struct LikeTheKitNoteForm {
    #[serde(default)]
    #[validate(length(
        min = 1,
        max = 255,
        message = "Give the note a title of 1 to 255 characters."
    ))]
    pub title: String,
    #[serde(default)]
    #[validate(length(max = 10000, message = "Keep the note to 10000 characters."))]
    pub body: Option<String>,
}

#[tool_input]
pub struct LikeTheKitLoginForm {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}
