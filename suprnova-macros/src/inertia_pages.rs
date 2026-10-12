//! Where `inertia_response!` looks for a page component at compile time.
//!
//! A `suprnova new` project keeps its pages at
//! `frontend/src/pages/{Component}.{svelte,tsx,jsx,vue}`, and without
//! configuration that is the only place the macro looks. An application
//! whose frontend lays pages out another way describes the layout in its
//! own `Cargo.toml`:
//!
//! ```toml
//! [package.metadata.suprnova.inertia]
//! pages_dir = "resources/angular/pages"
//! page_file = "{dir}/{name|lower}.page.ts"
//! ```
//!
//! The manifest is the one file every build of the crate already has, and it
//! sits at `CARGO_MANIFEST_DIR`, the base every page path resolves against.
//! A separate config file would need its own discovery rules for no gain.

use std::path::{Path, PathBuf};

use crate::utils::levenshtein_distance;

/// The table the lookup is read from, as error messages name it.
pub(crate) const LOOKUP_TABLE: &str = "[package.metadata.suprnova.inertia]";

/// Where a `suprnova new` project keeps its pages, relative to the crate.
const STARTER_PAGES_DIR: &str = "frontend/src/pages";

/// Page-component file extensions the macro accepts when no `page_file`
/// pattern names the file.
///
/// Ordered so that Svelte (Suprnova's default) wins ties first. The macro
/// accepts whichever extension exists, which frees the framework from
/// requiring a build-time `SUPRNOVA_FRONTEND` env var in every workspace
/// setup.
const PAGE_EXTENSIONS: &[&str] = &["svelte", "tsx", "jsx", "vue"];

/// How a component name maps to the file the macro requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageLookup {
    /// Relative to the crate directory, without a trailing separator.
    pages_dir: String,
    file: PageFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PageFile {
    /// `{Component}.{ext}` for each of [`PAGE_EXTENSIONS`].
    Extensions,
    /// The one file a `page_file` pattern names.
    Pattern(PagePattern),
}

/// A parsed `page_file` value such as `{dir}/{name|lower}.page.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagePattern {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(String),
    Placeholder {
        field: Field,
        filter: Option<CaseFilter>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The component name up to its last `/`, empty for a top-level page.
    Dir,
    /// The component name's last segment.
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseFilter {
    Lower,
    Kebab,
    Snake,
}

impl PagePattern {
    /// Parses a `page_file` value. The error names the key and the problem,
    /// because it surfaces as a compile error far from the manifest.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() {
            return Err("`page_file` must not be empty".to_string());
        }
        if is_rooted(text) {
            return Err(format!(
                "`page_file` must be relative to `pages_dir`, got `{text}`"
            ));
        }

        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = text;
        while let Some(index) = rest.find(['{', '}']) {
            let (before, after) = rest.split_at(index);
            literal.push_str(before);
            if after.starts_with('}') {
                return Err(format!(
                    "`page_file` has a `}}` with no opening `{{` in `{text}`"
                ));
            }
            let body = &after[1..];
            let close = match body.find(['{', '}']) {
                Some(close) if body[close..].starts_with('}') => close,
                _ => {
                    return Err(format!("`page_file` has an unclosed `{{` in `{text}`"));
                }
            };
            if !literal.is_empty() {
                parts.push(Part::Literal(std::mem::take(&mut literal)));
            }
            parts.push(Self::placeholder(&body[..close])?);
            rest = &body[close + 1..];
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }

        let names_the_page = parts.iter().any(|part| {
            matches!(
                part,
                Part::Placeholder {
                    field: Field::Name,
                    ..
                }
            )
        });
        if !names_the_page {
            return Err(format!(
                "`page_file` must contain `{{name}}`, or every component in a \
                 directory names the same file, in `{text}`"
            ));
        }
        Ok(Self { parts })
    }

    fn placeholder(body: &str) -> Result<Part, String> {
        let whole = format!("{{{body}}}");
        if body.is_empty() {
            return Err("`page_file` has an empty placeholder `{}`".to_string());
        }
        let mut pieces = body.split('|');
        let field = match pieces.next() {
            Some("dir") => Field::Dir,
            Some("name") => Field::Name,
            _ => {
                return Err(format!(
                    "`page_file` uses the unknown placeholder `{whole}`; the \
                     placeholders are `{{dir}}` and `{{name}}`"
                ));
            }
        };
        let filter = match pieces.next() {
            None => None,
            Some("") => {
                return Err(format!("`page_file` has an empty filter in `{whole}`"));
            }
            Some("lower") => Some(CaseFilter::Lower),
            Some("kebab") => Some(CaseFilter::Kebab),
            Some("snake") => Some(CaseFilter::Snake),
            Some(other) => {
                return Err(format!(
                    "`page_file` uses the unknown filter `{other}` in `{whole}`; \
                     the filters are `lower`, `kebab` and `snake`"
                ));
            }
        };
        if pieces.next().is_some() {
            return Err(format!(
                "`page_file` allows one filter per placeholder, got `{whole}`"
            ));
        }
        Ok(Part::Placeholder { field, filter })
    }

    /// The page's path relative to `pages_dir`. Empty segments are dropped,
    /// so `{dir}/` leaves no leading `/` for a top-level component.
    pub(crate) fn render(&self, component: &str) -> String {
        let (dir, name) = component.rsplit_once('/').unwrap_or(("", component));
        let mut path = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => path.push_str(text),
                Part::Placeholder { field, filter } => {
                    let value = match field {
                        Field::Dir => dir,
                        Field::Name => name,
                    };
                    match filter {
                        None => path.push_str(value),
                        Some(filter) => {
                            let segments: Vec<String> = value
                                .split('/')
                                .map(|segment| filter.apply(segment))
                                .collect();
                            path.push_str(&segments.join("/"));
                        }
                    }
                }
            }
        }
        path.split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The literal text after the last placeholder, which every page file
    /// the pattern can name ends with.
    fn suffix(&self) -> &str {
        match self.parts.last() {
            Some(Part::Literal(text)) => text,
            _ => "",
        }
    }
}

impl CaseFilter {
    fn apply(self, segment: &str) -> String {
        match self {
            Self::Lower => segment.to_lowercase(),
            Self::Kebab => words(segment).join("-"),
            Self::Snake => words(segment).join("_"),
        }
    }
}

/// Splits `BaixaMatricula` into `baixa` and `matricula`.
///
/// A word starts at an uppercase letter that follows a lowercase letter or a
/// digit, and at the last capital of an acronym followed by a lowercase
/// letter (`HTMLReport` is `html` and `report`). `-`, `_` and whitespace
/// separate words too.
fn words(segment: &str) -> Vec<String> {
    let chars: Vec<char> = segment.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c == '-' || c == '_' || c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_uppercase() && !current.is_empty() && index > 0 {
            let previous = chars[index - 1];
            let next_is_lower = chars.get(index + 1).is_some_and(|next| next.is_lowercase());
            if previous.is_lowercase()
                || previous.is_numeric()
                || (previous.is_uppercase() && next_is_lower)
            {
                words.push(std::mem::take(&mut current));
            }
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn is_rooted(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || Path::new(path).has_root()
        || Path::new(path).is_absolute()
}

impl PageLookup {
    /// The lookup of a `suprnova new` project, used when the manifest sets none.
    pub(crate) fn starter() -> Self {
        Self {
            pages_dir: STARTER_PAGES_DIR.to_string(),
            file: PageFile::Extensions,
        }
    }

    /// Reads the lookup from a `Cargo.toml` body: `Ok(None)` when the table
    /// is absent, an error naming the key when it is malformed.
    pub(crate) fn from_manifest(manifest: &str) -> Result<Option<Self>, String> {
        let document: toml::Table = manifest
            .parse()
            .map_err(|error| format!("could not parse the manifest: {error}"))?;
        let Some(metadata) = document
            .get("package")
            .and_then(|package| package.get("metadata"))
        else {
            return Ok(None);
        };
        let Some(suprnova) = metadata.get("suprnova") else {
            return Ok(None);
        };
        let Some(suprnova) = suprnova.as_table() else {
            return Err("`package.metadata.suprnova` must be a table".to_string());
        };
        let Some(inertia) = suprnova.get("inertia") else {
            return Ok(None);
        };
        let Some(inertia) = inertia.as_table() else {
            return Err("`package.metadata.suprnova.inertia` must be a table".to_string());
        };

        if let Some(key) = inertia
            .keys()
            .find(|key| *key != "pages_dir" && *key != "page_file")
        {
            return Err(format!(
                "unknown key `{key}`; the keys are `pages_dir` and `page_file`"
            ));
        }

        let pages_dir = match inertia.get("pages_dir") {
            None => STARTER_PAGES_DIR.to_string(),
            Some(value) => {
                let Some(text) = value.as_str() else {
                    return Err("`pages_dir` must be a string".to_string());
                };
                if text.is_empty() {
                    return Err(
                        "`pages_dir` must not be empty; use \".\" for the crate directory"
                            .to_string(),
                    );
                }
                if is_rooted(text) {
                    return Err(format!(
                        "`pages_dir` must be relative to the crate directory, got `{text}`"
                    ));
                }
                text.trim_end_matches(['/', '\\']).to_string()
            }
        };

        let file = match inertia.get("page_file") {
            None => PageFile::Extensions,
            Some(value) => {
                let Some(text) = value.as_str() else {
                    return Err("`page_file` must be a string".to_string());
                };
                PageFile::Pattern(PagePattern::parse(text)?)
            }
        };

        Ok(Some(Self { pages_dir, file }))
    }

    /// The paths, relative to the crate directory, that satisfy `component`.
    pub(crate) fn candidates(&self, component: &str) -> Vec<String> {
        match &self.file {
            PageFile::Extensions => PAGE_EXTENSIONS
                .iter()
                .map(|ext| format!("{}/{component}.{ext}", self.pages_dir))
                .collect(),
            PageFile::Pattern(pattern) => {
                vec![format!("{}/{}", self.pages_dir, pattern.render(component))]
            }
        }
    }

    /// The first candidate that is a file under `crate_dir`. A directory at
    /// the resolved path is not a page, so it does not count.
    pub(crate) fn find(&self, crate_dir: &Path, component: &str) -> Option<PathBuf> {
        self.candidates(component)
            .into_iter()
            .map(|candidate| crate_dir.join(candidate))
            .find(|path| path.is_file())
    }

    /// What exists under the pages directory, for the not-found message:
    /// component names for the extension lookup, file paths for a pattern
    /// (whose filters cannot be undone to recover a component name).
    pub(crate) fn available(&self, crate_dir: &Path) -> Vec<String> {
        let base = crate_dir.join(&self.pages_dir);
        let mut files = Vec::new();
        collect_files(&base, &base, &mut files);

        let mut names: Vec<String> = match &self.file {
            PageFile::Extensions => files
                .iter()
                .filter(|file| {
                    file.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| PAGE_EXTENSIONS.contains(&ext))
                })
                .filter_map(|file| file.with_extension("").to_str().map(forward_slashes))
                .collect(),
            PageFile::Pattern(pattern) => files
                .iter()
                .filter_map(|file| file.to_str().map(forward_slashes))
                .filter(|file| file.ends_with(pattern.suffix()))
                .collect(),
        };
        names.sort();
        names
    }

    /// The compile error for a component with no page under a configured
    /// lookup. It names every path the macro tried, since the mapping from
    /// component to file is the application's own.
    pub(crate) fn not_found_message(&self, component: &str, available: &[String]) -> String {
        let mut message = format!(
            "Inertia component '{component}' not found.\nLooked for: {}\n\
             The page lookup comes from {LOOKUP_TABLE} in Cargo.toml.",
            self.candidates(component).join(", ")
        );
        if available.is_empty() {
            message.push_str(&format!(
                "\n\nNo page files found under {}/.",
                self.pages_dir
            ));
            return message;
        }

        let (heading, target) = match &self.file {
            PageFile::Extensions => ("Available components:".to_string(), component.to_string()),
            PageFile::Pattern(pattern) => (
                format!("Page files under {}/:", self.pages_dir),
                pattern.render(component),
            ),
        };
        message.push_str("\n\n");
        message.push_str(&heading);
        for name in available {
            message.push_str("\n  - ");
            message.push_str(name);
        }
        if let Some(suggestion) = find_similar(&target, available) {
            message.push_str(&format!("\n\nDid you mean '{suggestion}'?"));
        }
        message
    }
}

/// The compile error for a missing page when the manifest sets no lookup,
/// worded as it was before the lookup became configurable.
pub(crate) fn starter_not_found_message(component: &str, available: &[String]) -> String {
    let mut message = format!(
        "Inertia component '{}' not found.\nLooked in: frontend/src/pages/\nTried extensions: {}",
        component,
        PAGE_EXTENSIONS
            .iter()
            .map(|e| format!(".{}", e))
            .collect::<Vec<_>>()
            .join(", ")
    );

    if !available.is_empty() {
        message.push_str("\n\nAvailable components:");
        for comp in available {
            message.push_str(&format!("\n  - {}", comp));
        }

        if let Some(suggestion) = find_similar(component, available) {
            message.push_str(&format!("\n\nDid you mean '{}'?", suggestion));
        }
    } else {
        message.push_str(
            "\n\nNo components found in frontend/src/pages/.\nMake sure your frontend directory structure is set up correctly.",
        );
    }
    message
}

/// Reads the lookup from the crate's `Cargo.toml`. A crate without a
/// manifest (some build systems set `CARGO_MANIFEST_DIR` without one) keeps
/// the starter lookup.
pub(crate) fn read_lookup(crate_dir: &Path) -> Result<Option<PageLookup>, String> {
    let manifest_path = crate_dir.join("Cargo.toml");
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "{LOOKUP_TABLE}: could not read {}: {error}",
                manifest_path.display()
            ));
        }
    };
    PageLookup::from_manifest(&manifest)
        .map_err(|problem| format!("{LOOKUP_TABLE} in Cargo.toml: {problem}"))
}

/// Checks that `component` names an existing page under the crate's lookup.
///
/// On success it returns the files the check read - the manifest and the
/// page - so the expansion can make rustc track them: cargo re-runs the
/// check only when a file in the crate's dep-info changes, and a proc macro
/// that reads files on its own leaves them out of it.
pub(crate) fn check_component(crate_dir: &Path, component: &str) -> Result<Vec<PathBuf>, String> {
    let configured = read_lookup(crate_dir)?;
    let lookup = configured.clone().unwrap_or_else(PageLookup::starter);

    let Some(page) = lookup.find(crate_dir, component) else {
        let available = lookup.available(crate_dir);
        return Err(match configured {
            Some(lookup) => lookup.not_found_message(component, &available),
            None => starter_not_found_message(component, &available),
        });
    };

    let manifest = crate_dir.join("Cargo.toml");
    Ok([manifest, page]
        .into_iter()
        .filter(|path| path.is_file())
        .collect())
}

fn collect_files(base_dir: &Path, current_dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base_dir, &path, files);
            continue;
        }
        if let Ok(relative) = path.strip_prefix(base_dir) {
            files.push(relative.to_path_buf());
        }
    }
}

/// Normalizes Windows-style separators so a listed name matches what
/// `inertia_response!` is called with on any platform.
fn forward_slashes(path: &str) -> String {
    path.replace(std::path::MAIN_SEPARATOR, "/")
}

fn find_similar(target: &str, available: &[String]) -> Option<String> {
    let target_lower = target.to_lowercase();

    for comp in available {
        if comp.to_lowercase() == target_lower {
            return Some(comp.clone());
        }
    }

    let mut best_match: Option<(String, usize)> = None;
    for comp in available {
        let distance = levenshtein_distance(&target_lower, &comp.to_lowercase());
        let threshold = std::cmp::max(2, target.len() / 3);
        if distance <= threshold
            && best_match
                .as_ref()
                .map(|(_, d)| distance < *d)
                .unwrap_or(true)
        {
            best_match = Some((comp.clone(), distance));
        }
    }

    best_match.map(|(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ISSUE_LOOKUP: &str = r#"
[package]
name = "app"
version = "0.1.0"

[package.metadata.suprnova.inertia]
pages_dir = "resources/angular/pages"
page_file = "{dir}/{name|lower}.page.ts"
"#;

    fn pattern(text: &str) -> PagePattern {
        match PagePattern::parse(text) {
            Ok(pattern) => pattern,
            Err(problem) => panic!("`{text}` should parse: {problem}"),
        }
    }

    fn pattern_error(text: &str) -> String {
        match PagePattern::parse(text) {
            Ok(_) => panic!("`{text}` should be rejected"),
            Err(problem) => problem,
        }
    }

    fn lookup(manifest: &str) -> PageLookup {
        match PageLookup::from_manifest(manifest) {
            Ok(Some(lookup)) => lookup,
            Ok(None) => panic!("the manifest should configure a lookup"),
            Err(problem) => panic!("the manifest should be accepted: {problem}"),
        }
    }

    fn manifest_error(manifest: &str) -> String {
        match PageLookup::from_manifest(manifest) {
            Ok(_) => panic!("the manifest should be rejected"),
            Err(problem) => problem,
        }
    }

    fn with_table(body: &str) -> String {
        format!("[package]\nname = \"app\"\n\n[package.metadata.suprnova.inertia]\n{body}\n")
    }

    // ---- page_file patterns ----

    #[test]
    fn issue_pattern_maps_nested_components_to_lowercase_files() {
        let pattern = pattern("{dir}/{name|lower}.page.ts");
        assert_eq!(
            pattern.render("Tramits/BaixaMatricula/Create"),
            "Tramits/BaixaMatricula/create.page.ts"
        );
        assert_eq!(pattern.render("Tramits/Index"), "Tramits/index.page.ts");
    }

    #[test]
    fn top_level_component_leaves_no_leading_slash() {
        let pattern = pattern("{dir}/{name|lower}.page.ts");
        assert_eq!(pattern.render("Home"), "home.page.ts");
    }

    #[test]
    fn placeholders_without_filters_keep_the_component_spelling() {
        let pattern = pattern("{dir}/{name}/index.tsx");
        assert_eq!(pattern.render("Admin/Users"), "Admin/Users/index.tsx");
        assert_eq!(pattern.render("Home"), "Home/index.tsx");
    }

    #[test]
    fn kebab_and_snake_split_words_at_case_changes() {
        assert_eq!(
            pattern("{name|kebab}.ts").render("BaixaMatricula"),
            "baixa-matricula.ts"
        );
        assert_eq!(
            pattern("{name|snake}.ts").render("BaixaMatricula"),
            "baixa_matricula.ts"
        );
        assert_eq!(
            pattern("{name|kebab}.ts").render("HTMLReport"),
            "html-report.ts"
        );
        assert_eq!(
            pattern("{name|snake}.ts").render("Create2FA"),
            "create2_fa.ts"
        );
        assert_eq!(
            pattern("{name|kebab}.ts").render("already_snake-or-kebab"),
            "already-snake-or-kebab.ts"
        );
        assert_eq!(pattern("{name|snake}.ts").render("Index"), "index.ts");
    }

    #[test]
    fn filters_on_dir_apply_to_every_segment() {
        let pattern = pattern("{dir|kebab}/{name|snake}.component.ts");
        assert_eq!(
            pattern.render("AdminArea/UserProfile/EditForm"),
            "admin-area/user-profile/edit_form.component.ts"
        );
        assert_eq!(
            self::pattern("{dir|lower}/{name}.vue").render("Admin/Reports/Index"),
            "admin/reports/Index.vue"
        );
        assert_eq!(
            self::pattern("{dir|snake}/{name}.vue").render("Index"),
            "Index.vue"
        );
    }

    #[test]
    fn a_placeholder_may_repeat() {
        let pattern = pattern("{dir}/{name}/{name|kebab}.page.ts");
        assert_eq!(
            pattern.render("Users/EditProfile"),
            "Users/EditProfile/edit-profile.page.ts"
        );
    }

    #[test]
    fn malformed_patterns_name_the_key_and_the_problem() {
        let cases: &[(&str, &[&str])] = &[
            ("", &["must not be empty"]),
            ("{dir}/{name.page.ts", &["unclosed `{`"]),
            ("{dir}/name}.page.ts", &["`}`", "no opening `{`"]),
            ("{}/{name}.ts", &["empty placeholder `{}`"]),
            (
                "{file}.ts",
                &["unknown placeholder `{file}`", "`{dir}`", "`{name}`"],
            ),
            (
                "{name|upper}.ts",
                &[
                    "unknown filter `upper`",
                    "`{name|upper}`",
                    "lower",
                    "kebab",
                    "snake",
                ],
            ),
            ("{name|}.ts", &["empty filter", "`{name|}`"]),
            (
                "{name|lower|kebab}.ts",
                &["one filter", "`{name|lower|kebab}`"],
            ),
            ("{dir}/index.page.ts", &["must contain `{name}`"]),
            ("/{name}.ts", &["relative"]),
        ];
        for (text, expected) in cases {
            let problem = pattern_error(text);
            assert!(
                problem.contains("`page_file`"),
                "`{text}`: the error must name the key, got: {problem}"
            );
            for fragment in *expected {
                assert!(
                    problem.contains(fragment),
                    "`{text}`: expected `{fragment}` in: {problem}"
                );
            }
        }
    }

    // ---- the manifest table ----

    #[test]
    fn manifest_without_the_table_keeps_the_starter_lookup() {
        assert_eq!(
            PageLookup::from_manifest("[package]\nname = \"app\"\n"),
            Ok(None)
        );
        assert_eq!(
            PageLookup::from_manifest(
                "[package]\nname = \"app\"\n\n[package.metadata.docs.rs]\nall-features = true\n"
            ),
            Ok(None)
        );
        assert_eq!(
            PageLookup::from_manifest(
                "[package]\nname = \"app\"\n\n[package.metadata.suprnova.other]\nkey = 1\n"
            ),
            Ok(None)
        );
    }

    #[test]
    fn starter_lookup_tries_the_four_extensions_under_frontend_src_pages() {
        assert_eq!(
            PageLookup::starter().candidates("Users/Index"),
            vec![
                "frontend/src/pages/Users/Index.svelte",
                "frontend/src/pages/Users/Index.tsx",
                "frontend/src/pages/Users/Index.jsx",
                "frontend/src/pages/Users/Index.vue",
            ]
        );
    }

    #[test]
    fn issue_table_resolves_one_file_per_component() {
        let lookup = lookup(ISSUE_LOOKUP);
        assert_eq!(
            lookup.candidates("Tramits/BaixaMatricula/Create"),
            vec!["resources/angular/pages/Tramits/BaixaMatricula/create.page.ts"]
        );
        assert_eq!(
            lookup.candidates("Tramits/Index"),
            vec!["resources/angular/pages/Tramits/index.page.ts"]
        );
    }

    #[test]
    fn pages_dir_alone_keeps_the_starter_extensions() {
        let lookup = lookup(&with_table("pages_dir = \"resources/js/Pages/\""));
        assert_eq!(
            lookup.candidates("Admin/Dashboard"),
            vec![
                "resources/js/Pages/Admin/Dashboard.svelte",
                "resources/js/Pages/Admin/Dashboard.tsx",
                "resources/js/Pages/Admin/Dashboard.jsx",
                "resources/js/Pages/Admin/Dashboard.vue",
            ]
        );
    }

    #[test]
    fn page_file_alone_uses_the_starter_directory() {
        let lookup = lookup(&with_table("page_file = \"{dir}/{name|lower}.page.ts\""));
        assert_eq!(
            lookup.candidates("Users/Index"),
            vec!["frontend/src/pages/Users/index.page.ts"]
        );
    }

    #[test]
    fn an_empty_table_resolves_like_the_starter_lookup() {
        let lookup = lookup(&with_table(""));
        assert_eq!(
            lookup.candidates("Home"),
            PageLookup::starter().candidates("Home")
        );
    }

    #[test]
    fn malformed_tables_name_the_key_and_the_problem() {
        let cases: Vec<(String, &[&str])> =
            vec![
            (
                with_table("pagesdir = \"x\""),
                &["unknown key `pagesdir`", "`pages_dir`", "`page_file`"],
            ),
            (with_table("pages_dir = 3"), &["`pages_dir`", "must be a string"]),
            (with_table("page_file = true"), &["`page_file`", "must be a string"]),
            (with_table("pages_dir = \"\""), &["`pages_dir`", "must not be empty"]),
            (
                with_table("pages_dir = \"/srv/pages\""),
                &["`pages_dir`", "relative to the crate directory"],
            ),
            (
                with_table("page_file = \"{dir}/{name|upper}.page.ts\""),
                &["`page_file`", "unknown filter `upper`"],
            ),
            (
                "[package]\nname = \"app\"\n\n[package.metadata.suprnova]\ninertia = \"pages\"\n"
                    .to_string(),
                &["`package.metadata.suprnova.inertia`", "must be a table"],
            ),
            (
                "[package]\nname = \"app\"\n\n[package.metadata]\nsuprnova = 1\n".to_string(),
                &["`package.metadata.suprnova`", "must be a table"],
            ),
            ("[package\nname = ".to_string(), &["could not parse"]),
        ];
        for (manifest, expected) in cases {
            let problem = manifest_error(&manifest);
            for fragment in expected {
                assert!(
                    problem.contains(fragment),
                    "expected `{fragment}` in: {problem}\nfor manifest:\n{manifest}"
                );
            }
        }
    }

    // ---- error messages ----

    #[test]
    fn starter_message_is_unchanged_with_suggestions() {
        let available = vec!["Dashboard".to_string(), "Home".to_string()];
        assert_eq!(
            starter_not_found_message("Hom", &available),
            "Inertia component 'Hom' not found.\n\
             Looked in: frontend/src/pages/\n\
             Tried extensions: .svelte, .tsx, .jsx, .vue\n\
             \n\
             Available components:\n  - Dashboard\n  - Home\n\
             \n\
             Did you mean 'Home'?"
        );
    }

    #[test]
    fn starter_message_is_unchanged_without_components() {
        assert_eq!(
            starter_not_found_message("Home", &[]),
            "Inertia component 'Home' not found.\n\
             Looked in: frontend/src/pages/\n\
             Tried extensions: .svelte, .tsx, .jsx, .vue\n\
             \n\
             No components found in frontend/src/pages/.\n\
             Make sure your frontend directory structure is set up correctly."
        );
    }

    #[test]
    fn configured_message_names_the_resolved_path() {
        let lookup = lookup(ISSUE_LOOKUP);
        let available = vec![
            "Tramits/BaixaMatricula/create.page.ts".to_string(),
            "Tramits/index.page.ts".to_string(),
        ];
        let message = lookup.not_found_message("Tramits/BaixaMatricula/Crate", &available);
        assert!(
            message.starts_with("Inertia component 'Tramits/BaixaMatricula/Crate' not found.\n"),
            "{message}"
        );
        assert!(
            message.contains(
                "Looked for: resources/angular/pages/Tramits/BaixaMatricula/crate.page.ts"
            ),
            "{message}"
        );
        assert!(
            message.contains("[package.metadata.suprnova.inertia] in Cargo.toml"),
            "{message}"
        );
        assert!(
            message.contains(
                "Page files under resources/angular/pages/:\n  - Tramits/BaixaMatricula/create.page.ts\n  - Tramits/index.page.ts"
            ),
            "{message}"
        );
        assert!(
            message.contains("Did you mean 'Tramits/BaixaMatricula/create.page.ts'?"),
            "{message}"
        );
        assert!(!message.contains("frontend/src/pages"), "{message}");
    }

    #[test]
    fn configured_message_without_pages_says_so() {
        let lookup = lookup(ISSUE_LOOKUP);
        let message = lookup.not_found_message("Home", &[]);
        assert!(
            message.contains("Looked for: resources/angular/pages/home.page.ts"),
            "{message}"
        );
        assert!(
            message.contains("No page files found under resources/angular/pages/."),
            "{message}"
        );
        assert!(!message.contains("Did you mean"), "{message}");
    }

    #[test]
    fn configured_message_for_pages_dir_lists_every_candidate() {
        let lookup = lookup(&with_table("pages_dir = \"resources/js/Pages\""));
        let message = lookup.not_found_message("Admin/Dashbord", &["Admin/Dashboard".to_string()]);
        assert!(
            message.contains(
                "Looked for: resources/js/Pages/Admin/Dashbord.svelte, \
                 resources/js/Pages/Admin/Dashbord.tsx, \
                 resources/js/Pages/Admin/Dashbord.jsx, \
                 resources/js/Pages/Admin/Dashbord.vue"
            ),
            "{message}"
        );
        assert!(
            message.contains("Available components:\n  - Admin/Dashboard"),
            "{message}"
        );
        assert!(
            message.contains("Did you mean 'Admin/Dashboard'?"),
            "{message}"
        );
    }

    // ---- the filesystem ----

    fn crate_dir(files: &[&str]) -> tempfile::TempDir {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(error) => panic!("create a scratch crate directory: {error}"),
        };
        for file in files {
            let path = dir.path().join(file);
            if let Some(parent) = path.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                panic!("create {}: {error}", parent.display());
            }
            if let Err(error) = std::fs::write(&path, "export {};\n") {
                panic!("write {}: {error}", path.display());
            }
        }
        dir
    }

    fn write(dir: &Path, file: &str, body: &str) {
        if let Err(error) = std::fs::write(dir.join(file), body) {
            panic!("write {file}: {error}");
        }
    }

    #[test]
    fn find_accepts_exactly_the_file_at_the_resolved_path() {
        let dir = crate_dir(&[
            "resources/angular/pages/Tramits/BaixaMatricula/create.page.ts",
            "resources/angular/pages/Tramits/index.page.ts",
            "frontend/src/pages/Home.svelte",
        ]);
        let lookup = lookup(ISSUE_LOOKUP);
        assert_eq!(
            lookup.find(dir.path(), "Tramits/BaixaMatricula/Create"),
            Some(
                dir.path()
                    .join("resources/angular/pages/Tramits/BaixaMatricula/create.page.ts")
            )
        );
        assert_eq!(
            lookup.find(dir.path(), "Tramits/Index"),
            Some(
                dir.path()
                    .join("resources/angular/pages/Tramits/index.page.ts")
            )
        );
        assert_eq!(lookup.find(dir.path(), "Tramits/BaixaMatricula/Edit"), None);
        // The starter location no longer counts once a lookup is set.
        assert_eq!(lookup.find(dir.path(), "Home"), None);
    }

    #[test]
    fn find_does_not_accept_a_directory_at_the_resolved_path() {
        // `{dir}/{name}` resolves `Users` to `resources/pages/Users`, which is
        // a directory of pages here, not a page file.
        let dir = crate_dir(&["resources/pages/Users/Index"]);
        let lookup = lookup(&with_table(
            "pages_dir = \"resources/pages\"\npage_file = \"{dir}/{name}\"",
        ));
        assert_eq!(
            lookup.find(dir.path(), "Users/Index"),
            Some(dir.path().join("resources/pages/Users/Index"))
        );
        assert_eq!(lookup.find(dir.path(), "Users"), None);
    }

    #[test]
    fn starter_find_accepts_any_of_the_four_extensions() {
        let dir = crate_dir(&[
            "frontend/src/pages/Home.vue",
            "frontend/src/pages/Users/Index.tsx",
        ]);
        let lookup = PageLookup::starter();
        assert_eq!(
            lookup.find(dir.path(), "Home"),
            Some(dir.path().join("frontend/src/pages/Home.vue"))
        );
        assert_eq!(
            lookup.find(dir.path(), "Users/Index"),
            Some(dir.path().join("frontend/src/pages/Users/Index.tsx"))
        );
        assert_eq!(lookup.find(dir.path(), "Missing"), None);
    }

    #[test]
    fn available_lists_the_files_the_pattern_can_name() {
        let dir = crate_dir(&[
            "resources/angular/pages/Tramits/BaixaMatricula/create.page.ts",
            "resources/angular/pages/Tramits/index.page.ts",
            "resources/angular/pages/Tramits/shared.service.ts",
            "resources/angular/pages/README.md",
        ]);
        assert_eq!(
            lookup(ISSUE_LOOKUP).available(dir.path()),
            vec![
                "Tramits/BaixaMatricula/create.page.ts".to_string(),
                "Tramits/index.page.ts".to_string(),
            ]
        );
    }

    #[test]
    fn available_lists_components_for_the_extension_lookup() {
        let dir = crate_dir(&[
            "frontend/src/pages/Home.svelte",
            "frontend/src/pages/Users/Index.tsx",
            "frontend/src/pages/styles.css",
        ]);
        assert_eq!(
            PageLookup::starter().available(dir.path()),
            vec!["Home".to_string(), "Users/Index".to_string()]
        );
    }

    #[test]
    fn read_lookup_without_a_manifest_keeps_the_starter_lookup() {
        let dir = crate_dir(&[]);
        assert_eq!(read_lookup(dir.path()), Ok(None));
    }

    #[test]
    fn read_lookup_reads_the_crate_manifest() {
        let dir = crate_dir(&[]);
        write(dir.path(), "Cargo.toml", ISSUE_LOOKUP);
        assert_eq!(read_lookup(dir.path()), Ok(Some(lookup(ISSUE_LOOKUP))));
    }

    #[test]
    fn read_lookup_errors_name_the_table_and_the_manifest() {
        let dir = crate_dir(&[]);
        write(
            dir.path(),
            "Cargo.toml",
            &with_table("page_file = \"{dir}/{name|upper}.page.ts\""),
        );
        let problem = match read_lookup(dir.path()) {
            Ok(_) => panic!("an unknown filter must be rejected"),
            Err(problem) => problem,
        };
        assert!(
            problem.starts_with("[package.metadata.suprnova.inertia] in Cargo.toml: "),
            "{problem}"
        );
        assert!(problem.contains("unknown filter `upper`"), "{problem}");
    }
}
