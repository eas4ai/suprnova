//! What the React, Vue and Svelte kits add for agents: the approval page,
//! the claim page and the connected-agents page, their controller and
//! routes, the browser helpers, and what the consent pages never do.
//!
//! These read what `suprnova new` writes; none compiles a scaffold. The
//! compiled proof is `agent_017_a_generated_inertia_application_serves_the_agent_pages`
//! in `scaffold_snapshot`, and the browser proof is the kit cases of the
//! `agents_browser` binary.

use std::path::Path;

use crate::shared::{INERTIA_KITS, Project, contains, item};

/// The page file extension of `kit`.
fn extension(kit: &str) -> &'static str {
    match kit {
        "react" => "tsx",
        "vue" => "vue",
        _ => "svelte",
    }
}

/// The three consent pages, by component name.
const PAGES: [&str; 3] = ["Approve", "Claim", "Connected"];

/// The scaffolded page `Agents/<page>` of `project`.
fn page(project: &Project, page: &str) -> String {
    project.read(&format!(
        "frontend/src/pages/Agents/{page}.{}",
        extension(project.kit)
    ))
}

/// The routes after the authenticated group in a scaffolded
/// `src/routes.rs`: where the agents' groups sit, up to the static files.
fn agents_routes(routes: &str) -> &str {
    let auth_end = routes
        .find(".middleware(middleware::authenticate::auth())")
        .expect("routes.rs has an authenticated group");
    let static_files = routes
        .find("// Static files.")
        .expect("routes.rs has the static files");
    assert!(auth_end < static_files, "the static files come last");
    &routes[auth_end..static_files]
}

/// The template a scaffolded file must equal byte for byte.
fn template(relative: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/templates/files")
            .join(relative),
    )
    .unwrap_or_else(|error| panic!("read the template {relative}: {error}"))
}

/// AGENT-017: every Inertia kit ships the approval, claim and
/// connected-agents pages in its own frontend, the controller that renders
/// them, and their routes after the authenticated group: the consent pages
/// for anyone, who signs in from there, and the connected agents behind
/// sign-in. Every one of them runs `ConsentPageHeaders`.
#[test]
fn agent_017_inertia_kits_ship_the_three_pages() {
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        for name in PAGES {
            let source = page(&project, name);
            assert!(
                source.contains("../../types/inertia-props"),
                "{kit}: Agents/{name} types its props from the generated types:\n{source}"
            );
        }

        let controller = project.read("src/controllers/agents.rs");
        assert_eq!(
            controller,
            template("backend/controllers/agents.rs.tpl"),
            "{kit}: src/controllers/agents.rs is the template"
        );
        let modules = project.read("src/controllers/mod.rs");
        assert!(
            modules.lines().any(|line| line == "pub mod agents;"),
            "{kit}: src/controllers/mod.rs must declare `pub mod agents;`:\n{modules}"
        );
        for (handler, component) in [
            ("approval", "Agents/Approve"),
            ("claim", "Agents/Claim"),
            ("connected", "Agents/Connected"),
        ] {
            let body = item(&controller, handler);
            assert!(
                body.contains(&format!("\"{component}\"")),
                "{kit}: `{handler}` must render {component}:\n{body}"
            );
        }
        for (handler, facade) in [
            ("approval", "AgentApprovals::of(&req)?"),
            ("decide", "AgentApprovals::of(&req)?"),
            ("claim", "AgentClaims::of(&req)?"),
            ("confirm", "AgentClaims::of(&req)?"),
            ("connected", "ConnectedAgents::client_grants(&req)"),
            ("connected", "ConnectedAgents::agent_accounts(&req)"),
            (
                "revoke_client",
                "ConnectedAgents::revoke_client_grant(&req, &grant)",
            ),
            (
                "revoke_agent",
                "ConnectedAgents::revoke_agent_account(&req, &registration)",
            ),
        ] {
            let body = item(&controller, handler);
            assert!(
                contains(body, facade),
                "{kit}: `{handler}` must act through `{facade}`:\n{body}"
            );
        }

        let routes = project.read("src/routes.rs");
        let agents = agents_routes(&routes);
        let consent_end = agents
            .find(".middleware(ConsentPageHeaders),")
            .unwrap_or_else(|| panic!("{kit}: no consent group:\n{routes}"));
        let (consent, connected) = agents.split_at(consent_end + 1);
        for route in [
            r#"get!("/oauth/approve", controllers::agents::approval).name("agents.approval")"#,
            r#"post!("/oauth/approve", controllers::agents::decide).name("agents.decide")"#,
            r#"get!("/agent/claim", controllers::agents::claim).name("agents.claim")"#,
            r#"post!("/agent/claim", controllers::agents::confirm).name("agents.confirm")"#,
        ] {
            assert!(
                contains(consent, route),
                "{kit}: `{route}` must sit in the consent group, open to a guest who then \
                 signs in:\n{routes}"
            );
        }
        for route in [
            r#"get!("/connected-agents", controllers::agents::connected).name("agents.connected")"#,
            r#"post!("/connected-agents/clients/{grant}/revoke", controllers::agents::revoke_client).name("agents.clients.revoke")"#,
            r#"post!("/connected-agents/agents/{registration}/revoke", controllers::agents::revoke_agent).name("agents.accounts.revoke")"#,
        ] {
            assert!(
                contains(connected, route),
                "{kit}: `{route}` must sit in the signed-in agents group:\n{routes}"
            );
        }
        assert!(
            contains(
                connected,
                "}).middleware(middleware::authenticate::auth()).middleware(ConsentPageHeaders),"
            ),
            "{kit}: the connected agents need sign-in and the consent headers:\n{routes}"
        );
        assert!(
            routes
                .lines()
                .any(|line| line.starts_with("use suprnova::{")
                    && line.contains("ConsentPageHeaders")),
            "{kit}: routes.rs imports ConsentPageHeaders:\n{routes}"
        );

        let types = project.read("frontend/src/types/inertia-props.ts");
        for entry in [
            r#""Agents/Approve": ApprovalProps;"#,
            r#""Agents/Claim": ClaimProps;"#,
            r#""Agents/Connected": ConnectedAgentsProps;"#,
        ] {
            assert!(
                types.contains(entry),
                "{kit}: inertia-props.ts must type `{entry}`:\n{types}"
            );
        }

        let layout = project.read(&format!(
            "frontend/src/layouts/AppLayout.{}",
            extension(kit)
        ));
        assert!(
            layout.contains("${root}/connected-agents"),
            "{kit}: the app layout links the connected agents:\n{layout}"
        );
    }
}

/// AGENT-017: a decision leaves for the client's redirect URI by a
/// location visit, which the browser follows itself, so the page's
/// `form-action 'self'` policy never sees a redirect to another origin.
/// Both forms post the decision in a hidden field.
#[test]
fn agent_017_inertia_decision_leaves_by_a_location_visit() {
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        let controller = project.read("src/controllers/agents.rs");
        let decide = item(&controller, "decide");
        let redirect = decide
            .split_once("ApprovalDecision::Redirect(url) => {")
            .and_then(|(_, arm)| arm.split_once("ApprovalDecision::Ended"))
            .map(|(arm, _)| arm)
            .unwrap_or_else(|| panic!("{kit}: `decide` has no redirect arm:\n{decide}"));
        for part in ["Inertia::clear_history();", "Ok(Inertia::location(url))"] {
            assert!(
                redirect.contains(part),
                "{kit}: `decide` must leave by a location visit (`{part}`):\n{decide}"
            );
        }
        let approve = page(&project, "Approve");
        for decision in ["approve", "deny"] {
            assert!(
                contains(&approve, &format!(r#"name="decision" value="{decision}""#)),
                "{kit}: Agents/Approve must post `{decision}` in a hidden field:\n{approve}"
            );
        }
        assert_eq!(
            approve.matches("/oauth/approve`").count(),
            2,
            "{kit}: Agents/Approve has one form per decision:\n{approve}"
        );
    }
}

/// How `kit` renders the expression `expression` as text.
fn rendered(kit: &str, expression: &str) -> String {
    if kit == "vue" {
        format!("{{{{ {expression} }}}}")
    } else {
        format!("{{{expression}}}")
    }
}

/// Whether the text at `at` in `source` sits inside a `<bdi>` element: the
/// last `<bdi>` before it opens after the last `</bdi>`, and the next
/// `</bdi>` after it closes before the next `<bdi>`.
fn inside_bdi(source: &str, at: usize) -> bool {
    let (before, after) = source.split_at(at);
    let opened = before.rfind("<bdi>");
    let closed = before.rfind("</bdi>");
    let opens_before = matches!((opened, closed), (Some(open), Some(close)) if open > close)
        || matches!((opened, closed), (Some(_), None));
    let next_close = after.find("</bdi>");
    let next_open = after.find("<bdi>");
    let closes_after = match (next_close, next_open) {
        (Some(close), Some(open)) => close < open,
        (Some(_), None) => true,
        _ => false,
    };
    opens_before && closes_after
}

/// AGENT-017: every text a client, an agent or the user chose sits in
/// `<bdi>` wherever a consent page shows it, so its reordering stays
/// inside it, and no page renders raw HTML or a URL the client chose for a
/// logo or a home page.
#[test]
fn agent_017_consent_pages_isolate_client_text() {
    let fields: [(&str, &[&str]); 3] = [
        (
            "Approve",
            &[
                "approval.client_name",
                "approval.client_host",
                "approval.redirect_uri",
            ],
        ),
        ("Claim", &["claim.signed_in_as", "claim.email"]),
        ("Connected", &["client.client_name", "client.client_host"]),
    ];
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        for (name, expressions) in fields {
            let source = page(&project, name);
            for raw in [
                "dangerouslySetInnerHTML",
                "v-html",
                "{@html",
                "logo_uri",
                "client_uri",
            ] {
                assert!(
                    !source.contains(raw),
                    "{kit}: Agents/{name} must not use `{raw}`:\n{source}"
                );
            }
            for expression in expressions {
                let shown = rendered(kit, expression);
                let places: Vec<usize> = source.match_indices(&shown).map(|(at, _)| at).collect();
                assert!(
                    !places.is_empty(),
                    "{kit}: Agents/{name} must show `{expression}`:\n{source}"
                );
                for at in &places {
                    assert!(
                        inside_bdi(&source, *at),
                        "{kit}: Agents/{name} shows `{expression}` outside <bdi>:\n{source}"
                    );
                }
                // Anywhere else it is only compared, never put in an
                // attribute or shown another way.
                for (at, _) in source.match_indices(expression) {
                    let shown_here = places
                        .iter()
                        .any(|place| at > *place && at < place + shown.len());
                    let rest = &source[at + expression.len()..];
                    let compared = rest.starts_with(" !== null") || rest.starts_with(" === null");
                    let longer = rest
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_alphanumeric() || next == '_');
                    assert!(
                        shown_here || compared || longer,
                        "{kit}: Agents/{name} uses `{expression}` other than as isolated text:\n{source}"
                    );
                }
            }
        }
    }
}

/// AGENT-017: a consent page never offers a tool, so an agent in the
/// browser can neither approve a client nor claim itself. No page
/// registers a page or form tool, no consent route declares one, and the
/// controller declares none.
#[test]
fn agent_017_consent_pages_offer_no_tools() {
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        for name in PAGES {
            let source = page(&project, name);
            for helper in ["usePageTool", "useFormTool", "lib/agents", "registerTool"] {
                assert!(
                    !source.contains(helper),
                    "{kit}: Agents/{name} must offer no tool (`{helper}`):\n{source}"
                );
            }
        }
        let routes = project.read("src/routes.rs");
        let agents = agents_routes(&routes);
        assert!(
            !agents.contains("with_tools") && !agents.contains("share_tools"),
            "{kit}: no agents route may offer a tool:\n{agents}"
        );
        let controller = project.read("src/controllers/agents.rs");
        for marker in ["#[tool", "tool_input", "ToolCaller"] {
            assert!(
                !controller.contains(marker),
                "{kit}: the agents controller must declare no tool (`{marker}`)"
            );
        }
    }
}

/// AGENT-017: every Inertia kit ships the agent helpers: the shared engine
/// and the page tools installer byte for byte, the kit's own hooks over
/// them, the form request schemas `generate-types` writes for the kit, and
/// an entry that installs the page tools before the application starts.
/// The hooks name the schemas by a path the kit resolves: the kits define
/// no `@` alias. A form tool's submission carries the header a consent page
/// refuses.
#[test]
fn agent_017_inertia_kits_ship_the_browser_helpers() {
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        let file = |relative: &str| project.read(&format!("frontend/src/{relative}"));

        let package: serde_json::Value =
            serde_json::from_str(&project.read("frontend/package.json"))
                .expect("the generated package manifest");
        assert_eq!(
            package["devDependencies"]["webmcp-types"], "0.1.10",
            "{kit}"
        );
        let core = file("lib/agents/core.ts");
        assert_eq!(
            core,
            template("frontend/agents/core.ts.tpl"),
            "{kit}: core.ts"
        );
        for upstream in [
            "WebMCP.ModelContextTool",
            "WebMCP.ToolAnnotations",
            "WebMCP.ModelContext",
            "WebMCP.ToolExecuteCallbackOptions",
        ] {
            assert!(
                core.contains(upstream),
                "{kit}: browser helpers do not use {upstream}"
            );
        }
        // A form tool marks its submission with the header the consent
        // pages refuse, by the framework's own name for it.
        for needle in [
            format!(
                "export const FORM_TOOL_HEADER = '{}'",
                suprnova::agents::FORM_TOOL_HEADER
            ),
            "headers: { [FORM_TOOL_HEADER]: '1' },".to_owned(),
        ] {
            assert!(core.contains(&needle), "{kit}: core.ts lacks {needle}");
        }
        assert_eq!(
            file("lib/agents/inertia.ts"),
            template("frontend/agents/inertia.ts.tpl"),
            "{kit}: inertia.ts"
        );
        assert_eq!(
            file("types/tool-schemas.ts"),
            template("frontend/types/tool-schemas.ts.tpl"),
            "{kit}: tool-schemas.ts"
        );
        let hooks = file("lib/agents/index.ts");
        for needle in [
            "from './core'",
            "export { installPageTools } from './inertia'",
            "export function usePageTool",
            "export function useFormTool",
            "`src/types/tool-schemas.ts`",
            "`import { toolSchemas } from '../../types/tool-schemas'`",
        ] {
            assert!(
                hooks.contains(needle),
                "{kit}: index.ts lacks {needle}:\n{hooks}"
            );
        }
        // The import the comment shows resolves from a page under
        // `pages/Notes/`.
        assert!(
            project
                .root
                .join("frontend/src/pages/Notes/../../types/tool-schemas.ts")
                .is_file(),
            "{kit}: the comment's import path does not resolve"
        );
        for entry in walkdir::WalkDir::new(project.root.join("frontend/src")) {
            let entry = entry.expect("walk the frontend");
            if entry.file_type().is_file() {
                let text = std::fs::read_to_string(entry.path()).expect("read a frontend file");
                assert!(
                    !text.contains("'@/") && !text.contains("`@/"),
                    "{kit}: {} names the `@` alias, which the kit does not define",
                    entry.path().display()
                );
            }
        }

        let entry = file(if kit == "react" {
            "main.tsx"
        } else {
            "main.ts"
        });
        let install = entry.find("installPageTools(router)").unwrap_or_else(|| {
            panic!("{kit}: the entry does not install the page tools:\n{entry}")
        });
        let start = entry
            .find("createInertiaApp({")
            .unwrap_or_else(|| panic!("{kit}: the entry does not start Inertia"));
        assert!(
            install < start,
            "{kit}: page tools install after the application starts"
        );
        assert!(
            entry.contains("import { installPageTools } from './lib/agents'"),
            "{kit}: {entry}"
        );
    }
}
