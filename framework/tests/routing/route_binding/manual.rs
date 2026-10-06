//! BIND-014: the manual and the rustdoc describe binding as the contract
//! specifies. No page or doc comment names the removed `route_binding!` or
//! the `param_name` trait; no page shows the bare `x::Model` form as the way
//! to bind a `#[model]` outside the section that documents that older form;
//! and the routing chapter carries a "Why Suprnova diverges" section for
//! strict value parsing and the 404 that never repeats the value, the enum
//! name fallback, and the startup checks. The tutorial's code is compiled
//! by `suprnova-macros/tests/route_binding_build.rs`.

use std::path::{Path, PathBuf};

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

#[test]
fn bind_014_no_page_or_doc_comment_names_the_removed_surface() {
    let mut offenders = Vec::new();
    for (path, text) in manual_pages() {
        for (line, content) in text.lines().enumerate() {
            if content.contains("route_binding!") || content.contains("param_name()") {
                offenders.push(format!("{}:{}: {content}", path.display(), line + 1));
            }
        }
    }
    let mut sources = Vec::new();
    rust_files(&root().join("framework/src"), &mut sources);
    rust_files(&root().join("suprnova-macros/src"), &mut sources);
    for path in sources {
        let text = std::fs::read_to_string(&path).expect("read a source file");
        for (line, content) in text.lines().enumerate() {
            let trimmed = content.trim_start();
            let is_doc = trimmed.starts_with("///") || trimmed.starts_with("//!");
            if is_doc && (content.contains("route_binding!") || content.contains("param_name()")) {
                offenders.push(format!("{}:{}: {content}", path.display(), line + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "stale binding docs:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn bind_014_no_page_teaches_the_bare_form_as_the_way_to_bind_a_model() {
    // A handler argument typed `something::Model` is the bare SeaORM form.
    // The routing chapter documents it once, as the older form; every other
    // page binds the `#[model]` struct.
    let bare = regex::Regex::new(r"\b[a-z_][a-z0-9_]*: [a-z_][a-z0-9_]*::Model\b")
        .expect("a valid pattern");
    let mut offenders = Vec::new();
    for (path, text) in manual_pages() {
        let page = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        for (line, content) in text.lines().enumerate() {
            let in_signature = content.contains("fn ") && bare.is_match(content);
            if in_signature && page != "routing.md" {
                offenders.push(format!("{page}:{}: {content}", line + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "pages that bind through `x::Model`:\n{}",
        offenders.join("\n")
    );
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
