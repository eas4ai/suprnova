//! The Live kit's agent pages (AGENT-017): the approval, claim,
//! connected-agents and returning pages, plain server-rendered documents
//! with native forms, and the head every page of the kit carries.
//!
//! These read what `suprnova new --frontend live` writes; none compiles
//! it. `agent_017_a_generated_live_application_serves_the_agent_pages` in
//! `scaffold_snapshot.rs` builds the kit and drives the pages.

use crate::shared::{Project, contains, item};

/// The kit's agent pages: the template under `templates/live/pages/` and
/// the view that renders it.
const PAGES: [(&str, &str); 4] = [
    ("agents-approve", "AgentsApprovePage"),
    ("agents-claim", "AgentsClaimPage"),
    ("agents-connected", "AgentsConnectedPage"),
    ("agents-returning", "AgentsReturningPage"),
];

/// The approval and claim routes. A guest who opens either page signs in
/// and comes back, so they sit outside the authenticated group.
const CONSENT_ROUTES: [&str; 4] = [
    r#"get!("/oauth/approve", controllers::agents::approval).name("agents.approval")"#,
    r#"post!("/oauth/approve", controllers::agents::decide).name("agents.decide")"#,
    r#"get!("/agent/claim", controllers::agents::claim).name("agents.claim")"#,
    r#"post!("/agent/claim", controllers::agents::confirm).name("agents.confirm")"#,
];

/// The connected-agents routes, for the signed-in user only.
const CONNECTED_ROUTES: [&str; 3] = [
    r#"get!("/connected-agents", controllers::agents::connected).name("agents.connected")"#,
    r#"post!("/connected-agents/clients/{grant}/revoke", controllers::agents::revoke_client).name("agents.clients.revoke")"#,
    r#"post!("/connected-agents/agents/{registration}/revoke", controllers::agents::revoke_agent).name("agents.accounts.revoke")"#,
];

/// The values on a consent page that someone other than the application
/// chose: the client, the agent, the provider or the user.
const CHOSEN_FIELDS: [&str; 7] = [
    "client_name",
    "client_host",
    "redirect_uri",
    "signed_in_as",
    "email",
    "provider",
    "redirect",
];

/// The `group!` of `routes` that holds `route`: from the `group!(` before
/// it to the end of the line that closes the group, its middleware
/// included.
fn group_of<'a>(routes: &'a str, route: &str) -> &'a str {
    let at = routes
        .find(route)
        .unwrap_or_else(|| panic!("live: src/routes.rs lacks `{route}`:\n{routes}"));
    let start = routes[..at]
        .rfind("group!(")
        .unwrap_or_else(|| panic!("live: `{route}` is in no group:\n{routes}"));
    let close = routes[at..]
        .find("})")
        .map(|close| close + at)
        .unwrap_or_else(|| panic!("live: the group of `{route}` does not close:\n{routes}"));
    let end = routes[close..]
        .find('\n')
        .map_or(routes.len(), |end| end + close);
    &routes[start..end]
}

/// Each `{{ ... }}` in `template` that sits in the text of the page rather
/// than inside a tag, with the text before it.
fn text_expressions(template: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = 0;
    while let Some(open) = template[rest..].find("{{") {
        let open = open + rest;
        let close = template[open..]
            .find("}}")
            .map(|close| close + open + 2)
            .expect("an expression closes");
        let before = &template[..open];
        let in_tag = before.rfind('<') > before.rfind('>');
        if !in_tag {
            found.push((before, &template[open..close]));
        }
        rest = close;
    }
    found
}

/// AGENT-017: the Live kit ships the approval page, the claim page and the
/// connected-agents page, with the page that returns the user to a client,
/// as views, templates, a controller and routes `suprnova new` writes.
#[test]
fn agent_017_live_kit_ships_the_three_pages() {
    let project = Project::new("live");
    let views = project.read("src/views/mod.rs");
    for (page, view) in PAGES {
        let template = project.read(&format!("templates/live/pages/{page}.html"));
        assert!(
            template.starts_with("{% extends \"live/layouts/app.html\" %}"),
            "live: {page} must extend the application layout:\n{template}"
        );
        assert!(
            views.contains(&format!(
                "#[suprnova::view(path = \"live/pages/{page}.html\")]\npub struct {view} {{"
            )),
            "live: src/views/mod.rs declares no {view} for {page}:\n{views}"
        );
    }
    assert!(
        views.contains("    Agents,\n"),
        "live: Nav::Agents:\n{views}"
    );

    let modules = project.read("src/controllers/mod.rs");
    assert!(
        modules.lines().any(|line| line == "pub mod agents;"),
        "live: src/controllers/mod.rs must declare `pub mod agents;`:\n{modules}"
    );
    let controller = project.read("src/controllers/agents.rs");
    for part in [
        "AgentApprovals::of(&req)?",
        "AgentClaims::of(&req)?",
        "ConnectedAgents::client_grants(&req)",
        "ConnectedAgents::agent_accounts(&req)",
        "ConnectedAgents::revoke_client_grant(&req, &grant)",
        "ConnectedAgents::revoke_agent_account(&req, &registration)",
        "const SIGN_IN: &str = \"/login\";",
        "(ApprovalLookup::SignIn, _) => Redirect::guest(&req, SIGN_IN).into(),",
        "(ClaimLookup::Refused(ClaimRefusal::SignIn), _) => Redirect::guest(&req, SIGN_IN).into(),",
    ] {
        assert!(
            contains(&controller, part),
            "live: src/controllers/agents.rs lacks `{part}`:\n{controller}"
        );
    }

    let routes = project.read("src/routes.rs");
    let consent = group_of(&routes, CONSENT_ROUTES[0]);
    for route in CONSENT_ROUTES {
        assert!(
            consent.contains(route),
            "live: the approval and claim routes share one group:\n{consent}"
        );
    }
    assert!(
        consent.ends_with("}).middleware(ConsentPageHeaders),"),
        "live: the approval and claim pages run the consent headers and no \
         sign-in guard of their own:\n{consent}"
    );
    let connected = group_of(&routes, CONNECTED_ROUTES[0]);
    for route in CONNECTED_ROUTES {
        assert!(
            connected.contains(route),
            "live: the connected-agents routes share one group:\n{connected}"
        );
    }
    assert!(
        connected.ends_with(
            "}).middleware(middleware::authenticate::auth()).middleware(live::tenant()).middleware(ConsentPageHeaders),"
        ),
        "live: the connected-agents page is the signed-in user's, with the consent \
         headers:\n{connected}"
    );
    // After the authenticated group, so the guest and account routes keep
    // their places, and before the static files, which answer what no
    // route matched.
    let authenticated = routes
        .find(".middleware(middleware::authenticate::auth()).middleware(live::tenant()),")
        .expect("live: the authenticated group");
    let fallback = routes
        .find("fallback!(StaticFiles::public().handler())")
        .expect("live: the static files");
    for group in [consent, connected] {
        let at = routes.find(group).expect("the group is in the routes");
        assert!(
            authenticated < at && at < fallback,
            "live: the agent routes go after the authenticated group and before the \
             static files:\n{routes}"
        );
    }

    let layout = project.read("templates/live/layouts/app.html");
    for part in [
        "header::header_link(frame.href(\"/connected-agents\"), \"Agents\", current=frame.is_current(Nav::Agents))",
        "menu::account_link(frame.href(\"/connected-agents\"), \"Connected agents\")",
    ] {
        assert!(
            layout.contains(part),
            "live: the application layout lacks `{part}`:\n{layout}"
        );
    }
}

/// AGENT-017: every value on a consent page that the client, the agent,
/// a provider or the user chose is isolated in `<bdi>`, so its direction
/// marks cannot reorder the sentence around it, and no page places agent
/// data as raw HTML or loads a URL the client chose.
#[test]
fn agent_017_live_consent_pages_isolate_client_text() {
    let project = Project::new("live");
    for (page, part) in [
        ("agents-approve", "<bdi>{{ approval.client_name }}</bdi>"),
        ("agents-approve", "<bdi>{{ approval.client_host }}</bdi>"),
        (
            "agents-approve",
            "<bdi><code>{{ approval.redirect_uri }}</code></bdi>",
        ),
        ("agents-claim", "<bdi>{{ claim.signed_in_as }}</bdi>"),
        ("agents-claim", "<bdi>{{ email }}</bdi>"),
        ("agents-connected", "<bdi>{{ client.client_name }}</bdi>"),
        ("agents-connected", "<bdi>{{ client.client_host }}</bdi>"),
        ("agents-returning", "<bdi>{{ client_name }}</bdi>"),
    ] {
        let template = project.read(&format!("templates/live/pages/{page}.html"));
        assert!(
            template.contains(part),
            "live: {page} must isolate `{part}`:\n{template}"
        );
    }
    for (page, _) in PAGES {
        let template = project.read(&format!("templates/live/pages/{page}.html"));
        for (before, expression) in text_expressions(&template) {
            if CHOSEN_FIELDS.iter().any(|field| expression.contains(field)) {
                assert!(
                    before.ends_with("<bdi>") || before.ends_with("<bdi><code>"),
                    "live: {page} shows `{expression}` outside <bdi>:\n{template}"
                );
            }
        }
        for forbidden in ["trusted_html", "|safe", "logo_uri", "client_uri"] {
            assert!(
                !template.contains(forbidden),
                "live: {page} must not use `{forbidden}`:\n{template}"
            );
        }
    }
}

/// AGENT-017: every page of the kit carries the Live bootstrap tags and
/// the session's CSRF token as the `csrf-token` meta tag, once. The Live
/// runtime's server tools read the token there before the `XSRF-TOKEN`
/// cookie.
#[test]
fn agent_017_live_head_carries_the_bootstrap_and_the_csrf_meta() {
    let project = Project::new("live");
    let head = project.read("templates/live/layouts/head.html");
    assert!(
        head.contains(
            "{% if let Some(bootstrap) = frame.bootstrap %}{{ bootstrap|trusted_html }}{% endif %}"
        ),
        "live: the head must carry the bootstrap tags:\n{head}"
    );
    assert_eq!(
        head.matches("<meta name=\"csrf-token\"").count(),
        1,
        "live: the head must carry one csrf-token meta tag:\n{head}"
    );
    assert!(
        head.contains("<meta name=\"csrf-token\" content=\"{{ frame.csrf }}\">"),
        "live: the csrf-token meta tag carries the session's token:\n{head}"
    );
    for layout in ["app", "guest"] {
        let template = project.read(&format!("templates/live/layouts/{layout}.html"));
        assert!(
            template.contains("{% include \"live/layouts/head.html\" %}"),
            "live: the {layout} layout must include the head:\n{template}"
        );
    }
}

/// AGENT-017: no consent page offers an agent a tool. Consent comes from
/// the person, so no agent route declares `with_tools`, and the pages are
/// plain documents: they mount no island, load no Live runtime and run no
/// script.
#[test]
fn agent_017_live_consent_pages_offer_no_tools() {
    let project = Project::new("live");
    let routes = project.read("src/routes.rs");
    for group in [
        group_of(&routes, CONSENT_ROUTES[0]),
        group_of(&routes, CONNECTED_ROUTES[0]),
    ] {
        assert!(
            !group.contains("with_tools"),
            "live: an agent route offers a tool:\n{group}"
        );
    }
    let controller = project.read("src/controllers/agents.rs");
    assert!(
        controller.contains("Frame::plain("),
        "live: the agent pages render plain frames:\n{controller}"
    );
    for live_only in ["Frame::live(", "LiveDocument", "with_tools"] {
        assert!(
            !controller.contains(live_only),
            "live: the agent pages must not use `{live_only}`:\n{controller}"
        );
    }
    for (page, _) in PAGES {
        let template = project.read(&format!("templates/live/pages/{page}.html"));
        for live_only in ["<script", "live:", "suprnova-live"] {
            assert!(
                !template.contains(live_only),
                "live: {page} must not carry `{live_only}`:\n{template}"
            );
        }
    }
}

/// AGENT-017: every consent form is a native form that posts the session's
/// CSRF token, which the kit's global CSRF middleware checks.
#[test]
fn agent_017_live_consent_forms_post_the_csrf_token() {
    let project = Project::new("live");
    let mut forms = 0;
    for (page, _) in PAGES {
        let template = project.read(&format!("templates/live/pages/{page}.html"));
        for form in template.split("<form").skip(1) {
            let form = &form[..form.find("</form>").expect("a form closes")];
            forms += 1;
            assert!(
                form.contains("method=\"post\""),
                "live: {page} has a form that does not post:\n{form}"
            );
            assert!(
                form.contains("<input type=\"hidden\" name=\"_token\" value=\"{{ frame.csrf }}\">"),
                "live: {page} has a form without the CSRF token:\n{form}"
            );
        }
    }
    // Allow and deny, the code, and the two revoke buttons.
    assert!(forms >= 5, "live: only {forms} consent forms");
}

/// AGENT-017: the approval decision returns the user to the client by a
/// page of its own. The client's redirect URI is usually on another
/// origin, and the kit's policy (`form-action 'self'`) makes a browser
/// block a redirect that answers a form post there; a meta refresh is a
/// navigation of the page, not part of the form submission.
#[test]
fn agent_017_live_decision_returns_to_the_client_by_a_page_of_its_own() {
    let project = Project::new("live");
    let returning = project.read("templates/live/pages/agents-returning.html");
    let head = returning
        .split("{% block head %}")
        .nth(1)
        .and_then(|rest| rest.split("{% endblock %}").next())
        .unwrap_or_else(|| panic!("live: the returning page fills the head block:\n{returning}"));
    assert!(
        head.contains("<meta http-equiv=\"refresh\" content=\"0; url={{ redirect }}\">"),
        "live: the returning page navigates by a meta refresh:\n{returning}"
    );
    assert!(
        returning.contains("href=\"{{ redirect }}\""),
        "live: the returning page links the redirect too:\n{returning}"
    );
    let layout = project.read("templates/live/layouts/app.html");
    assert!(
        layout.contains("{% include \"live/layouts/head.html\" %}\n{% block head %}{% endblock %}"),
        "live: the application layout has a head block:\n{layout}"
    );

    let controller = project.read("src/controllers/agents.rs");
    let decide = item(&controller, "decide");
    assert!(
        contains(decide, "ApprovalDecision::Redirect(url) =>")
            && decide.contains("returning_page("),
        "live: a decision answers with the returning page:\n{decide}"
    );
    assert!(
        !decide.contains("Redirect::to(url"),
        "live: a decision must not redirect a form post to the client:\n{decide}"
    );
}
