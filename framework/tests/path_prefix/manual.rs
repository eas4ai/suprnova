//! PFX-009 and the manual clauses of PFX-001, PFX-002, PFX-007, PFX-010 and
//! PFX-011: route matching stays on the received path, and the manual
//! documents prefix deployment and writes component links with the root.

use std::path::{Path, PathBuf};

use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router};

use crate::support;

fn manual_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the framework sits in the workspace")
        .join("manual")
}

fn chapter(name: &str) -> String {
    let path = manual_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The prefix-deployment section of `manual/deployment.md`, from its
/// heading to the next second-level heading, with every run of whitespace
/// read as one space, so a sentence matches across the lines it wraps over.
fn prefix_section() -> String {
    prefix_section_lines()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn prefix_section_lines() -> String {
    let text = chapter("deployment.md");
    let heading = "## Serving under a path prefix";
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("deployment.md has no `{heading}` section"));
    let rest = &text[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end].to_owned()
}

#[tokio::test]
async fn pfx_009_route_matching_stays_on_the_received_path() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_009_route_matching_stays_on_the_received_path",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let router: Router = Router::new()
        .get("/invoices", |_request: Request| async {
            Ok(HttpResponse::text("invoices"))
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;
    assert_eq!(
        support::get_prefixed(address, "/invoices").await.status,
        200
    );
    assert_eq!(
        support::get_prefixed(address, "/billing/invoices")
            .await
            .status,
        404,
        "the root must not be folded into route matching"
    );
}

#[test]
fn pfx_009_the_deployment_chapter_documents_prefix_deployment() {
    let section = prefix_section();
    for needle in [
        "X-Forwarded-Prefix",
        "APP_TRUSTED_PROXIES",
        "url::root()",
        "proxy_set_header X-Forwarded-Prefix",
        "### Why Suprnova diverges",
    ] {
        assert!(
            section.contains(needle),
            "the section does not name `{needle}`"
        );
    }
    let callout = &section[section
        .find("### Why Suprnova diverges")
        .expect("the callout")..];
    for needle in ["trailing slash", "ignored", "route()", "APP_URL"] {
        assert!(
            callout.contains(needle),
            "the callout does not mention `{needle}`"
        );
    }
}

#[test]
fn pfx_009_the_urls_chapter_links_the_section_and_names_no_rewriting_middleware() {
    let urls = chapter("urls.md");
    assert!(
        urls.contains("deployment.md#serving-under-a-path-prefix"),
        "urls.md does not link the prefix-deployment section"
    );
    for claim in ["updates the request URL", "trusted-proxy middleware"] {
        assert!(!urls.contains(claim), "urls.md still says `{claim}`");
    }
}

#[test]
fn pfx_001_the_manual_says_the_proxy_sets_or_clears_the_prefix_header() {
    let section = prefix_section();
    assert!(
        section.contains("sets or clears `X-Forwarded-Prefix`"),
        "the section does not say the proxy must set or clear the header"
    );
}

#[test]
fn pfx_002_the_manual_says_mail_and_job_links_need_the_prefix_in_app_url() {
    let section = prefix_section();
    assert!(
        section.contains("mail and job links") && section.contains("in `APP_URL`"),
        "the section does not say a header-only prefix needs APP_URL for mail and jobs"
    );
}

#[test]
fn pfx_007_the_manual_says_host_cookies_sharing_a_host_need_distinct_names() {
    let section = prefix_section();
    assert!(
        section.contains("`__Host-`") && section.contains("distinct cookie names"),
        "the section does not say `__Host-` cookies on a shared host need distinct names"
    );
}

#[test]
fn pfx_010_the_manual_says_routes_must_not_begin_with_the_prefix() {
    let section = prefix_section();
    assert!(
        section.contains("routes that begin with the prefix"),
        "the section does not say an application's routes must not begin with its prefix"
    );
}

/// The value at the start of `rest`: up to the matching quote when it opens
/// with one, otherwise up to a space, `>` or `)`.
fn value_at(rest: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = rest.strip_prefix(quote) {
            return &inner[..inner.find(quote).unwrap_or(inner.len())];
        }
    }
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>' || c == ')')
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Every link `line` writes: an HTML attribute value, quoted either way or
/// not at all; a Markdown link target or link definition; and the URL of a
/// CSS `url()` or `@import`.
fn links_in(line: &str) -> Vec<&str> {
    let mut links = Vec::new();
    let bytes = line.as_bytes();
    for (at, _) in line.match_indices('=') {
        let named = at > 0
            && (bytes[at - 1].is_ascii_alphanumeric() || matches!(bytes[at - 1], b'-' | b'_'));
        if named {
            links.push(value_at(&line[at + 1..]));
        }
    }
    for (at, _) in line.match_indices("](") {
        links.push(value_at(&line[at + 2..]));
    }
    if let Some((_, target)) = line
        .trim_start()
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("]: "))
    {
        links.push(value_at(target.trim_start()));
    }
    for (at, _) in line.match_indices("url(") {
        links.push(value_at(line[at + 4..].trim_start()));
    }
    for (at, _) in line.match_indices("@import ") {
        links.push(value_at(line[at + 8..].trim_start()));
    }
    links
}

#[test]
fn pfx_011_every_component_asset_link_in_the_manual_writes_the_root() {
    let mut offenders = Vec::new();
    let entries = std::fs::read_dir(manual_dir()).expect("the manual directory");
    for entry in entries {
        let path = entry.expect("an entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a chapter");
        for (index, line) in text.lines().enumerate() {
            for value in links_in(line) {
                let component_link =
                    value.contains("-ui/") && !value.contains("://") && !value.starts_with("//");
                if component_link && !value.starts_with("{{ suprnova::url::root() }}/") {
                    offenders.push(format!(
                        "{}:{}: {value}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        index + 1
                    ));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

#[test]
fn pfx_011_the_link_reader_sees_every_link_form() {
    for (line, link) in [
        (
            r#"<link href="/suprnova-ui/a/a.css">"#,
            "/suprnova-ui/a/a.css",
        ),
        ("<link href='/suprnova-ui/a/a.css'>", "/suprnova-ui/a/a.css"),
        ("<link href=/suprnova-ui/a/a.css>", "/suprnova-ui/a/a.css"),
        (
            r#"<script src="/acme-ui/a/a.js"></script>"#,
            "/acme-ui/a/a.js",
        ),
        (
            "[a stylesheet](/suprnova-ui/a/a.css)",
            "/suprnova-ui/a/a.css",
        ),
        ("[a]: /acme-ui/a/a.css", "/acme-ui/a/a.css"),
        (
            "background: url(/suprnova-ui/a/a.png);",
            "/suprnova-ui/a/a.png",
        ),
        (
            "background: url('/suprnova-ui/a/a.png');",
            "/suprnova-ui/a/a.png",
        ),
        (r#"@import "/suprnova-ui/a/a.css";"#, "/suprnova-ui/a/a.css"),
    ] {
        assert!(
            links_in(line).contains(&link),
            "{line}: {:?}",
            links_in(line)
        );
    }
    // A path named in prose or in a view's `import` is no link.
    for line in [
        "Assets are served at `/suprnova-ui/<component>/<file>`.",
        r#"{% import "suprnova-ui/field/field.html" as field %}"#,
        r#"#[live(name = "acme.counter", view = "acme-ui/counter/counter.html")]"#,
    ] {
        assert!(
            links_in(line).iter().all(|value| !value.contains("-ui/")),
            "{line}: {:?}",
            links_in(line)
        );
    }
}

#[test]
fn pfx_011_the_manual_names_url_root_for_links_route_does_not_build() {
    let urls = chapter("urls.md");
    assert!(
        urls.contains("url::root()") && urls.contains("does not build"),
        "urls.md does not name `url::root()` as the way to write a root-relative link"
    );
    let live = chapter("live.md");
    assert!(
        live.contains("{{ suprnova::url::root() }}/suprnova-ui/"),
        "live.md does not link a component stylesheet with `url::root()`"
    );
}
