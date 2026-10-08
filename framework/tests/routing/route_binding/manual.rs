//! BIND-014: the manual and the rustdoc describe binding as the contract
//! specifies. No page or doc comment names the removed `route_binding!` or
//! the `param_name` trait; no page shows the bare `x::Model` form as the way
//! to bind a `#[model]` outside the section that documents that older form;
//! and the routing chapter carries a "Why Suprnova diverges" section for
//! strict value parsing and the 404 that never repeats the value, the enum
//! name fallback, and the startup checks. The tutorial's code is compiled
//! by `suprnova-macros/tests/route_binding_build.rs`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .to_path_buf()
}

/// The English manual chapters; the locale mirrors are not checked.
fn manual_pages() -> Vec<(PathBuf, String)> {
    let dir = root().join("manual");
    let mut pages: Vec<(PathBuf, String)> = std::fs::read_dir(&dir)
        .expect("read the manual")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read a chapter");
            (path, text)
        })
        .collect();
    pages.sort();
    pages
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read a source directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The directory of every workspace member, read from the root manifest.
fn workspace_members() -> Vec<PathBuf> {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("read Cargo.toml");
    let start = manifest
        .find("members = [")
        .expect("the workspace lists its members");
    let list = &manifest[start..];
    let list = &list[..list.find(']').expect("the member list is closed")];
    let members: Vec<PathBuf> = list
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|member| root().join(member))
        .collect();
    assert!(
        members.len() > 4,
        "the workspace members were not read: {members:?}"
    );
    members
}

/// A Rust string literal, its contents captured. `(?s)`: an escape may be
/// a `\` line continuation.
static STRING_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)"((?:[^"\\]|\\.)*)""#).expect("a valid pattern"));

/// The string literals in `text`, a `\` line continuation read as a space.
fn string_literals(text: &str) -> Vec<String> {
    let continued = Regex::new(r"\\\n\s*").expect("a valid pattern");
    STRING_LITERAL
        .captures_iter(text)
        .map(|captures| continued.replace_all(&captures[1], " ").into_owned())
        .collect()
}

/// The rustdoc of `source`: the text of every `///` and `//!` comment, every
/// `/** */` and `/*! */` block, and the string literals of every `#[doc =
/// ...]` and `#![doc = ...]` attribute, each on the line it starts on, and
/// every other line empty, so line numbers and multi-line text survive.
fn rustdoc_text(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut doc = vec![String::new(); lines.len()];
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if let Some(text) = trimmed
            .strip_prefix("///")
            .filter(|text| !text.starts_with('/'))
            .or_else(|| trimmed.strip_prefix("//!"))
        {
            doc[index] = text.to_owned();
            index += 1;
        } else if (trimmed.starts_with("/**")
            && !trimmed.starts_with("/***")
            && !trimmed.starts_with("/**/"))
            || trimmed.starts_with("/*!")
        {
            // A doc block runs to the first `*/`.
            let mut text = trimmed[3..].to_owned();
            let first = index;
            while !text.contains("*/") && index + 1 < lines.len() {
                index += 1;
                text.push('\n');
                text.push_str(lines[index]);
            }
            let text = text.split("*/").next().unwrap_or_default();
            for (offset, line) in text.lines().enumerate() {
                doc[first + offset] = line.to_owned();
            }
            index += 1;
        } else if trimmed.starts_with("#[doc") || trimmed.starts_with("#![doc") {
            // The attribute runs until its brackets balance.
            let first = index;
            let mut text = lines[index].to_owned();
            let balance = |text: &str| {
                let without_strings = STRING_LITERAL.replace_all(text, "\"\"");
                without_strings.matches('[').count() as i64
                    - without_strings.matches(']').count() as i64
            };
            while balance(&text) > 0 && index + 1 < lines.len() {
                index += 1;
                text.push('\n');
                text.push_str(lines[index]);
            }
            doc[first] = string_literals(&text).join(" ");
            index += 1;
        } else {
            index += 1;
        }
    }
    doc.join("\n")
}

/// The rustdoc of every source file of every workspace crate.
fn rustdoc_files() -> Vec<(PathBuf, String)> {
    let mut sources = Vec::new();
    for member in workspace_members() {
        let src = member.join("src");
        if src.is_dir() {
            rust_files(&src, &mut sources);
        }
    }
    sources.sort();
    sources
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read a source file");
            (path, rustdoc_text(&text))
        })
        .collect()
}

/// The lines of `text` that name the removed surface: the `route_binding!`
/// macro, written with its `!` or called a macro without it, and the
/// `param_name` method of the removed trait, with or without `()`.
/// `FrameworkError::ParamError` has a field of that name, so a line that
/// names `ParamError` is about the field.
fn names_removed_surface(text: &str) -> Vec<(usize, String)> {
    let removed = Regex::new(r"route_binding!|\broute_binding`?\s+macro\b|\bparam_name\b")
        .expect("a valid pattern");
    text.lines()
        .enumerate()
        .filter(|(_, line)| removed.is_match(line) && !line.contains("ParamError"))
        .map(|(index, line)| (index + 1, line.trim().to_owned()))
        .collect()
}

#[test]
fn bind_014_no_page_or_doc_comment_names_the_removed_surface() {
    let mut offenders = Vec::new();
    for (path, text) in manual_pages() {
        for (line, content) in names_removed_surface(&text) {
            offenders.push(format!("{}:{line}: {content}", path.display()));
        }
    }
    let files = rustdoc_files();
    assert!(
        files
            .iter()
            .any(|(path, _)| path.ends_with("suprnova-cli/src/main.rs")),
        "the scan must read every workspace crate"
    );
    for (path, doc) in files {
        for (line, content) in names_removed_surface(&doc) {
            offenders.push(format!("{}:{line}: {content}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "stale binding docs:\n{}",
        offenders.join("\n")
    );
}

/// The `fn` signatures in `text` whose parameters name a bare SeaORM row,
/// a type path ending in `module::Model`, with the line each starts on. The
/// parameter list may span lines and hold parentheses
/// (`RouteParam(post): RouteParam<post::Model>`).
fn bare_form_signatures(text: &str) -> Vec<(usize, String)> {
    let signature = Regex::new(r"\bfn\s+[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^()]*?>)?\s*\(")
        .expect("a valid pattern");
    let bare = Regex::new(r"\b[a-z_][a-z0-9_]*::Model\b").expect("a valid pattern");
    let mut found = Vec::new();
    for start in signature.find_iter(text) {
        let params_start = start.end();
        let mut depth = 1;
        let mut params_end = None;
        for (offset, c) in text[params_start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        params_end = Some(params_start + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(params_end) = params_end else {
            continue;
        };
        let params = &text[params_start..params_end];
        if bare.is_match(params) {
            let line = text[..start.start()].matches('\n').count() + 1;
            let shown = text[start.start()..=params_end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            found.push((line, shown));
        }
    }
    found
}

/// `text` with the section under the heading line `heading` blanked out,
/// up to the next heading outside a code block, its lines kept.
fn without_section(text: &str, heading: &str) -> String {
    let mut inside = false;
    let mut fenced = false;
    let mut kept = Vec::new();
    for line in text.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        let is_heading =
            !fenced && line.starts_with('#') && line.trim_start_matches('#').starts_with(' ');
        if is_heading {
            inside = line == heading;
        }
        kept.push(if inside { "" } else { line });
    }
    kept.join("\n")
}

#[test]
fn bind_014_no_page_teaches_the_bare_form_as_the_way_to_bind_a_model() {
    // A handler argument typed `something::Model` is the bare SeaORM form.
    // The routing chapter documents it once, under "Older binding forms";
    // every other page, the rest of the routing chapter and every rustdoc
    // comment bind the `#[model]` struct.
    let mut offenders = Vec::new();
    for (path, text) in manual_pages() {
        let page = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_owned();
        let text = if page == "routing.md" {
            let kept = without_section(&text, "### Older binding forms");
            assert!(
                text.contains("\n### Older binding forms\n")
                    && !kept.contains("### Older binding forms"),
                "routing.md has an \"Older binding forms\" section, and only it is left out"
            );
            kept
        } else {
            text
        };
        for (line, signature) in bare_form_signatures(&text) {
            offenders.push(format!("{page}:{line}: {signature}"));
        }
    }
    for (path, doc) in rustdoc_files() {
        for (line, signature) in bare_form_signatures(&doc) {
            offenders.push(format!("{}:{line}: {signature}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "pages and doc comments that bind through `x::Model`:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn bind_014_the_scans_read_what_they_claim() {
    // The rustdoc reader sees every form of doc comment.
    let source = "/// line `route_binding!`\n\
                  //! inner `param_name`\n\
                  #[doc = \"attr \\\n    route_binding macro\"]\n\
                  /** block\n  fn show(\n    post: post::Model,\n  ) */\n\
                  // plain `param_name` comment\n\
                  let s = \"param_name\";\n";
    let doc = rustdoc_text(source);
    let named: Vec<usize> = names_removed_surface(&doc)
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(named, vec![1, 2, 3], "{doc}");
    assert_eq!(
        bare_form_signatures(&doc)
            .into_iter()
            .map(|(line, _)| line)
            .collect::<Vec<_>>(),
        vec![6],
        "{doc}"
    );
    // A signature that spans lines, holds parentheses or a longer path.
    let page = "fn show(\n    RouteParam(post): RouteParam<crate::models::post::Model>,\n) {}\n\
                fn index(posts: Vec<Post>) {}\n";
    assert_eq!(bare_form_signatures(page).len(), 1);
    // The `ParamError` field is not the removed trait.
    assert!(names_removed_surface("ParamError { param_name: String }").is_empty());
    assert_eq!(names_removed_surface("the `param_name` trait").len(), 1);
    // A blanked section keeps its lines.
    let text = "a\n### Older binding forms\n```rust\n#[handler]\nfn show(post: post::Model)\n```\n### Next\nb";
    let kept = without_section(text, "### Older binding forms");
    assert_eq!(kept.lines().count(), text.lines().count());
    assert!(bare_form_signatures(&kept).is_empty());
    assert!(kept.contains("### Next"));
}

#[test]
fn bind_014_the_routing_chapter_says_why_suprnova_diverges() {
    let routing = std::fs::read_to_string(root().join("manual/routing.md")).expect("routing.md");
    let start = routing
        .find("## Route model binding")
        .expect("routing.md has a route model binding section");
    let section = &routing[start..];
    let end = section[3..]
        .find("\n## ")
        .map(|at| at + 3)
        .unwrap_or(section.len());
    let section = &section[..end];
    let diverges = section
        .find("### Why Suprnova diverges")
        .map(|at| &section[at..])
        .expect("the binding section has a `### Why Suprnova diverges`");
    // Prose wraps at any space, so compare with the line breaks folded.
    let diverges = diverges.split_whitespace().collect::<Vec<_>>().join(" ");
    for (topic, needle) in [
        ("strict value parsing", "does not parse"),
        (
            "the 404 that never repeats the value",
            "never repeats the value",
        ),
        ("the enum name fallback", "snake case"),
        ("the startup checks", "at startup"),
    ] {
        assert!(
            diverges.contains(needle),
            "the binding divergences must cover {topic} (`{needle}`)"
        );
    }
}
