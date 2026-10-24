//! Documentation catalog builder.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::headings::{Heading, slugify_heading};
use super::markdown::{ContentError, ContentResult, MarkdownRenderer};

/// Input and output paths for a documentation build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocsBuildConfig {
    /// Directory containing Markdown chapter files.
    pub source_dir: PathBuf,
    /// Directory where JSON artifacts are written.
    pub output_dir: PathBuf,
    /// Markdown table-of-contents file, usually `documentation.md`.
    pub toc_file: PathBuf,
}

/// A rendered documentation chapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocsChapter {
    /// URL slug and JSON filename stem for this chapter.
    pub slug: String,
    /// Display title for this chapter.
    pub title: String,
    /// Rendered chapter HTML.
    pub html: String,
    /// Short chapter preview.
    pub excerpt: String,
    /// Ordered headings extracted from the chapter.
    pub headings: Vec<Heading>,
    /// Previous chapter slug, if one exists.
    pub previous: Option<String>,
    /// Next chapter slug, if one exists.
    pub next: Option<String>,
}

/// Catalog artifact used by docs index and search pages.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocsCatalog {
    /// Ordered chapter entries.
    pub chapters: Vec<DocsCatalogEntry>,
    /// Search payload for client-side filtering.
    pub search: Vec<DocsSearchEntry>,
}

/// Lightweight catalog entry for one chapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocsCatalogEntry {
    /// Chapter slug.
    pub slug: String,
    /// Chapter display title.
    pub title: String,
    /// Chapter preview.
    pub excerpt: String,
    /// Ordered headings extracted from the chapter.
    pub headings: Vec<Heading>,
    /// Previous chapter slug, if one exists.
    pub previous: Option<String>,
    /// Next chapter slug, if one exists.
    pub next: Option<String>,
}

/// Search index entry for one chapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocsSearchEntry {
    /// Chapter slug.
    pub slug: String,
    /// Chapter display title.
    pub title: String,
    /// Chapter preview.
    pub excerpt: String,
    /// Ordered headings extracted from the chapter.
    pub headings: Vec<Heading>,
    /// Plain text extracted from the rendered Markdown.
    pub plain_text: String,
}

/// The slug of the catalog artifact [`build_docs`] writes beside the
/// chapters; no chapter may take it.
const CATALOG_SLUG: &str = "catalog";

/// Build JSON documentation artifacts from a Markdown table of contents.
///
/// Each chapter is written to `<slug>.json`, its slug taken from its file
/// name, so the table of contents is checked before anything is written:
/// two different files that map to one slug, or a chapter whose slug is
/// `catalog`, fail the build rather than overwrite each other. One file
/// listed more than once is one chapter: the catalog keeps an entry per
/// listing, all alike, and the chapter is rendered once. Its place in the
/// previous and next chain is its first listing, so every entry for it
/// agrees with its `<slug>.json` and the chain never leads back to a
/// chapter it has passed.
pub async fn build_docs(config: DocsBuildConfig) -> ContentResult<DocsCatalog> {
    let toc = tokio::fs::read_to_string(&config.toc_file).await?;
    let entries = parse_toc_entries(&toc);
    #[cfg(unix)]
    let source = {
        let source_dir = config.source_dir.clone();
        tokio::task::spawn_blocking(move || crate::confined_walk::ConfinedRoot::new(source_dir))
            .await
            .map_err(std::io::Error::other)??
    };
    #[cfg(unix)]
    let source_dir = source.path();
    #[cfg(not(unix))]
    let source_dir = &tokio::fs::canonicalize(&config.source_dir).await?;
    let chapter_paths = check_chapter_slugs(source_dir, &entries).await?;
    let renderer = MarkdownRenderer::default();

    tokio::fs::create_dir_all(&config.output_dir).await?;

    // Each chapter is written and dropped before the next is read. Only
    // the catalog and the search index, which are a chapter's summary, are
    // kept for the whole corpus; keeping every chapter's HTML too made the
    // build's peak grow with the manual.
    let mut catalog_entries: Vec<DocsCatalogEntry> = Vec::with_capacity(entries.len());
    let mut search_entries: Vec<DocsSearchEntry> = Vec::with_capacity(entries.len());

    // The reading order: each chapter once, where it is first listed.
    // Chapters are built in that same order, so the one being built sits
    // at the position counted by how many were built before it.
    let mut listed = std::collections::HashSet::with_capacity(entries.len());
    let reading_order: Vec<&str> = entries
        .iter()
        .map(|entry| entry.slug.as_str())
        .filter(|slug| listed.insert(*slug))
        .collect();
    // Where each chapter's catalog and search entries were first pushed.
    let mut built: HashMap<&str, usize> = HashMap::with_capacity(reading_order.len());

    for (entry, markdown_path) in entries.iter().zip(chapter_paths) {
        if let Some(&first) = built.get(entry.slug.as_str())
            && let (Some(catalog), Some(search)) = (
                catalog_entries.get(first).cloned(),
                search_entries.get(first).cloned(),
            )
        {
            catalog_entries.push(catalog);
            search_entries.push(search);
            continue;
        }
        let at = built.len();
        let previous = at
            .checked_sub(1)
            .and_then(|previous| reading_order.get(previous))
            .map(|&previous| previous.to_owned());
        let next = reading_order.get(at + 1).map(|&next| next.to_owned());
        #[cfg(unix)]
        let markdown = read_chapter(&source, &markdown_path).await;
        #[cfg(not(unix))]
        let markdown = tokio::fs::read_to_string(markdown_path).await;
        let markdown = markdown.map_err(|error| entry.error(error))?;
        let rewritten = rewrite_markdown_links(&markdown);
        let rendered = renderer.render(&rewritten)?;
        let title = rendered
            .headings
            .first()
            .map(|heading| heading.title.clone())
            .unwrap_or_else(|| entry.title.clone());

        let chapter = DocsChapter {
            slug: entry.slug.clone(),
            title: title.clone(),
            html: rendered.html,
            excerpt: rendered.excerpt.clone(),
            headings: rendered.headings.clone(),
            previous: previous.clone(),
            next: next.clone(),
        };

        tokio::fs::write(
            config.output_dir.join(format!("{}.json", entry.slug)),
            serde_json::to_string(&chapter)?,
        )
        .await?;

        catalog_entries.push(DocsCatalogEntry {
            slug: entry.slug.clone(),
            title: title.clone(),
            excerpt: chapter.excerpt.clone(),
            headings: chapter.headings.clone(),
            previous,
            next,
        });
        search_entries.push(DocsSearchEntry {
            slug: entry.slug.clone(),
            title,
            excerpt: chapter.excerpt.clone(),
            headings: chapter.headings.clone(),
            plain_text: rendered.plain_text,
        });
        built.insert(entry.slug.as_str(), catalog_entries.len() - 1);
    }

    let catalog = DocsCatalog {
        chapters: catalog_entries,
        search: search_entries,
    };
    tokio::fs::write(
        config.output_dir.join("catalog.json"),
        serde_json::to_string(&catalog)?,
    )
    .await?;

    Ok(catalog)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TocEntry {
    title: String,
    path: PathBuf,
    slug: String,
    line: usize,
}

impl TocEntry {
    fn error(&self, error: std::io::Error) -> ContentError {
        ContentError::Io(std::io::Error::new(
            error.kind(),
            format!(
                "docs TOC line {} (`{}`): {error}",
                self.line,
                self.path.display()
            ),
        ))
    }
}

/// Refuse chapters outside the source root or whose artifacts would overwrite
/// each other. Files are compared by their canonical path, so `setup.md`
/// and `./setup.md` are the same chapter while `guide/setup.md` and
/// `api/setup.md` are two.
async fn check_chapter_slugs(
    source_dir: &Path,
    entries: &[TocEntry],
) -> ContentResult<Vec<PathBuf>> {
    let mut owners: HashMap<&str, (PathBuf, &TocEntry)> = HashMap::new();
    let mut paths = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.path.is_absolute() {
            return Err(entry.error(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "absolute chapter paths are not allowed",
            )));
        }
        if entry
            .path
            .components()
            .any(|part| part == Component::ParentDir)
        {
            return Err(entry.error(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "parent components in chapter paths are not allowed",
            )));
        }
        if entry.slug == CATALOG_SLUG {
            return Err(ContentError::ReservedChapterSlug {
                slug: entry.slug.clone(),
                path: entry.path.display().to_string(),
            });
        }
        let file = tokio::fs::canonicalize(source_dir.join(&entry.path))
            .await
            .map_err(|error| entry.error(error))?;
        if !file.starts_with(source_dir) {
            return Err(entry.error(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "chapter resolves outside the source root",
            )));
        }
        match owners.get(entry.slug.as_str()) {
            Some((owner, first)) if *owner != file => {
                return Err(ContentError::DuplicateChapterSlug {
                    slug: entry.slug.clone(),
                    first: first.path.display().to_string(),
                    second: entry.path.display().to_string(),
                });
            }
            Some(_) => {}
            None => {
                owners.insert(entry.slug.as_str(), (file.clone(), entry));
            }
        }
        paths.push(file);
    }
    Ok(paths)
}

/// Resolve again beneath the root and read the opened handle, so replacing a
/// checked chapter or directory with an escaping symlink cannot redirect the read.
#[cfg(unix)]
async fn read_chapter(
    source: &crate::confined_walk::ConfinedRoot,
    path: &Path,
) -> std::io::Result<String> {
    use crate::confined_walk::{Last, open_file_in};
    use std::io::Read;

    let relative = path
        .strip_prefix(source.path())
        .map_err(std::io::Error::other)?
        .to_str()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "chapter path is not UTF-8",
            )
        })?
        .to_owned();
    let source = source.clone();
    tokio::task::spawn_blocking(move || {
        let entry = source.resolve(&relative, Last::Follow, false)?;
        let (mut file, _) = open_file_in(&entry.dir, &entry.name, &relative)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "chapter is not a regular file",
            )
        })?;
        let mut markdown = String::new();
        file.read_to_string(&mut markdown)?;
        Ok(markdown)
    })
    .await
    .map_err(std::io::Error::other)?
}

fn parse_toc_entries(markdown: &str) -> Vec<TocEntry> {
    markdown
        .lines()
        .enumerate()
        .filter_map(|(line, text)| parse_toc_line(text, line + 1))
        .collect::<Vec<_>>()
}

fn parse_toc_line(line: &str, line_number: usize) -> Option<TocEntry> {
    let title_start = line.find('[')? + 1;
    let title_end = line[title_start..].find(']')? + title_start;
    let path_start = line[title_end..].find('(')? + title_end + 1;
    let path_end = line[path_start..].find(')')? + path_start;

    let title = line[title_start..title_end].trim();
    let path = line[path_start..path_end].trim();
    if title.is_empty() || path.is_empty() {
        return None;
    }

    Some(TocEntry {
        title: title.to_owned(),
        path: PathBuf::from(path),
        slug: slug_from_markdown_path(path),
        line: line_number,
    })
}

fn rewrite_markdown_links(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut rest = markdown;

    while let Some(start) = rest.find("](") {
        output.push_str(&rest[..start + 2]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find(')') else {
            output.push_str(after_open);
            return output;
        };

        output.push_str(&rewrite_link_target(&after_open[..end]));
        rest = &after_open[end..];
    }

    output.push_str(rest);
    output
}

fn rewrite_link_target(target: &str) -> String {
    let split_at = target
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index))
        .unwrap_or(target.len());
    let (url, suffix) = target.split_at(split_at);

    if should_rewrite_markdown_url(url) {
        let (path, fragment) = url.split_once('#').unwrap_or((url, ""));
        let fragment = if fragment.is_empty() {
            String::new()
        } else {
            format!("#{fragment}")
        };
        format!(
            "/docs/{}{}{}",
            slug_from_markdown_path(path),
            fragment,
            suffix
        )
    } else {
        target.to_owned()
    }
}

fn should_rewrite_markdown_url(url: &str) -> bool {
    let (path, _) = url.split_once('#').unwrap_or((url, ""));
    path.ends_with(".md")
        && !path.starts_with('/')
        && !path.starts_with('#')
        && !path.contains("://")
}

fn slug_from_markdown_path(path: impl AsRef<str>) -> String {
    let path = Path::new(path.as_ref());
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify_heading)
        .unwrap_or_else(|| "chapter".to_owned())
}
