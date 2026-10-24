//! Validate and snapshot the source tree before routes can serve it.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, format_commonmark, parse_document};
use suprnova::{FrameworkError, MarkdownOptions, MarkdownRenderer};

use crate::{Asset, DocsAccess, DocsConfig, Manifest, Navigation, Page, Site, invalid};

pub(super) fn load(config: DocsConfig) -> Result<Site, FrameworkError> {
    let root = fs::canonicalize(&config.root)
        .map_err(|e| invalid(format!("{}: {e}", config.root.display())))?;
    let manifest: Manifest = read_json(&root, "docs.json")?;
    validate_manifest(&manifest)?;
    let mut site = Site {
        config,
        manifest,
        pages: Vec::new(),
        navigation: HashMap::new(),
        assets: Vec::new(),
    };
    let renderer = MarkdownRenderer::new(MarkdownOptions::default());
    for version in &site.manifest.versions {
        let directory = checked_file(&root, &version.id)?;
        let navigation: Vec<Navigation> = read_json(&directory, "navigation.json")?;
        let mut paths = HashSet::new();
        let mut entries = Vec::new();
        flatten(
            &navigation,
            site.manifest.private || version.private,
            &mut paths,
            &mut entries,
        )?;
        if entries.is_empty() {
            return Err(invalid(format!("{} has no navigation pages", version.id)));
        }
        for language in &site.manifest.languages {
            let folder = if site.manifest.languages.len() == 1 {
                directory.clone()
            } else {
                checked_file(&directory, language)?
            };
            for (title, path, private) in &entries {
                let file = checked_file(&folder, &format!("{path}.md"))?;
                let markdown = fs::read_to_string(&file)
                    .map_err(|e| invalid(format!("{}: {e}", file.display())))?;
                let base = site.base(&version.id, language);
                let linked = resolve_links(
                    &markdown,
                    &base,
                    path,
                    &format!("{}/{}", site.config.prefix, version.id),
                )?;
                let rendered = renderer.render(&linked).map_err(invalid)?;
                site.pages.push(Page {
                    access: DocsAccess {
                        version: version.id.clone(),
                        language: language.clone(),
                        path: Some(path.clone()),
                    },
                    title: title.clone(),
                    address: format!("{base}/{path}"),
                    markdown,
                    rendered,
                    private: *private,
                });
            }
        }
        let scope = AssetScope {
            version: &version.id,
            private: site.manifest.private || version.private,
            prefix: &site.config.prefix,
        };
        load_assets(
            &directory,
            &directory.join("_assets"),
            &scope,
            &mut site.assets,
        )?;
        site.navigation.insert(version.id.clone(), navigation);
    }
    Ok(site)
}

fn validate_manifest(manifest: &Manifest) -> Result<(), FrameworkError> {
    if manifest.title.trim().is_empty()
        || manifest.title.chars().any(char::is_control)
        || manifest.versions.is_empty()
        || manifest.languages.is_empty()
    {
        return Err(invalid("title, versions and languages must not be empty"));
    }
    unique(manifest.versions.iter().map(|v| v.id.as_str()))?;
    unique(manifest.languages.iter().map(String::as_str))?;
    if !manifest
        .versions
        .iter()
        .any(|v| v.id == manifest.default_version)
    {
        return Err(invalid("default_version must name a declared version"));
    }
    Ok(())
}

fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), FrameworkError> {
    let mut seen = HashSet::new();
    for id in ids {
        validate_path(id)?;
        if id.contains('/')
            || id.ends_with(".md")
            || matches!(
                id,
                "llms.txt" | "llms-full.txt" | "search.json" | "_assets" | "_tools"
            )
            || !seen.insert(id)
        {
            return Err(invalid(format!(
                "duplicate or reserved version/language: {id}"
            )));
        }
    }
    Ok(())
}

fn flatten(
    nodes: &[Navigation],
    inherited: bool,
    paths: &mut HashSet<String>,
    entries: &mut Vec<(String, String, bool)>,
) -> Result<(), FrameworkError> {
    for node in nodes {
        if node.title.trim().is_empty() || node.title.chars().any(char::is_control) {
            return Err(invalid("navigation titles must not be empty"));
        }
        let private = inherited || node.private;
        if let Some(path) = &node.path {
            validate_path(path)?;
            let first = path.split('/').next().unwrap_or_default();
            if path.ends_with(".md")
                || matches!(
                    first,
                    "llms.txt" | "llms-full.txt" | "search.json" | "_assets" | "_tools"
                )
                || !paths.insert(path.clone())
            {
                return Err(invalid(format!("duplicate or reserved page path: {path}")));
            }
            entries.push((node.title.clone(), path.clone(), private));
        } else if node.children.is_empty() {
            return Err(invalid("a navigation node needs a path or children"));
        }
        flatten(&node.children, private, paths, entries)?;
    }
    Ok(())
}

pub(super) fn validate_path(path: &str) -> Result<(), FrameworkError> {
    if path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        })
    {
        return Err(invalid(format!("unsafe path: {path}")));
    }
    Ok(())
}

fn checked_file(root: &Path, relative: &str) -> Result<std::path::PathBuf, FrameworkError> {
    validate_path(relative)?;
    let path =
        fs::canonicalize(root.join(relative)).map_err(|e| invalid(format!("{relative}: {e}")))?;
    if !path.starts_with(root) {
        return Err(invalid(format!("{relative} escapes its content directory")));
    }
    Ok(path)
}

fn read_json<T: serde::de::DeserializeOwned>(
    root: &Path,
    relative: &str,
) -> Result<T, FrameworkError> {
    let path = checked_file(root, relative)?;
    let bytes = fs::read(&path).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(format!("{}: {e}", path.display())))
}

struct AssetScope<'a> {
    version: &'a str,
    private: bool,
    prefix: &'a str,
}

fn load_assets(
    root: &Path,
    folder: &Path,
    scope: &AssetScope<'_>,
    out: &mut Vec<Asset>,
) -> Result<(), FrameworkError> {
    match fs::symlink_metadata(folder) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(invalid(e)),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(invalid("assets must not contain symlinks"));
        }
        Ok(_) => {}
    }
    let mut entries: Vec<_> = fs::read_dir(folder)
        .map_err(invalid)?
        .collect::<Result<_, _>>()
        .map_err(invalid)?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(invalid)?;
        if kind.is_symlink() {
            return Err(invalid("assets must not contain symlinks"));
        }
        if kind.is_dir() {
            load_assets(root, &path, scope, out)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(invalid)?
                .to_str()
                .ok_or_else(|| invalid("asset paths must be UTF-8"))?;
            validate_path(relative)?;
            out.push(Asset {
                version: scope.version.to_owned(),
                address: format!("{}/{}/{relative}", scope.prefix, scope.version),
                bytes: fs::read(&path).map_err(invalid)?.into(),
                mime: mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string(),
                private: scope.private,
            });
        }
    }
    Ok(())
}

fn resolve_links(
    markdown: &str,
    base: &str,
    page: &str,
    version_base: &str,
) -> Result<String, FrameworkError> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());
    options.extension.math_code = true;
    options.extension.math_dollars = true;
    let root = parse_document(&arena, markdown, &options);
    for node in root.descendants() {
        let mut data = node.data.borrow_mut();
        if let NodeValue::Link(link) | NodeValue::Image(link) = &mut data.value {
            link.url = resolve_url(&link.url, base, page, version_base)?;
        }
    }
    let mut output = String::new();
    format_commonmark(root, &options, &mut output).map_err(invalid)?;
    Ok(output)
}

fn resolve_url(
    url: &str,
    base: &str,
    page: &str,
    version_base: &str,
) -> Result<String, FrameworkError> {
    if url.is_empty()
        || url.starts_with(['#', '?'])
        || url.starts_with("//")
        || url
            .split(['/', '#', '?'])
            .next()
            .is_some_and(|first| first.contains(':'))
    {
        return Ok(url.to_owned());
    }
    let (target, suffix) = url
        .find(['#', '?'])
        .map_or((url, ""), |at| url.split_at(at));
    let (target, canonical) = target
        .strip_prefix(&format!("{version_base}/"))
        .map_or((target, false), |target| (target, true));
    let target = target.strip_suffix(".md").unwrap_or(target);
    let mut parts: Vec<&str> = if canonical || target.starts_with('/') {
        Vec::new()
    } else {
        page.rsplit_once('/')
            .map_or(Vec::new(), |(parent, _)| parent.split('/').collect())
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let path = parts.join("/");
    if !path.is_empty() {
        validate_path(&path)?;
    }
    let base = if canonical || path.starts_with("_assets/") {
        version_base
    } else {
        base
    };
    Ok(format!("{base}/{path}{suffix}"))
}
