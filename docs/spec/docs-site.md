# Documentation site plugin

Status: Agreed 2026-10-23
Prefix: DOCS

Drafted 2026-10-23 from the developer's direction on
`reference/larecipe-3.0.1`: "I was thinking about adding these features
... add these as commitments." LaRecipe 3 is a docs site that lives
inside the application and serves people, agents and models from one
Markdown tree; suprnova.app already runs most of that pipeline for the
manual, and this spec makes it a plugin any application installs.

## Observed at d448ed10c

Status: Observed

- `suprnova-markdown` (its own repository, git-tag distribution) renders
  Markdown through the framework's content pipeline; suprnova.app builds
  the manual with `docs:build`, indexes it for search and serves it with
  page tools for agents (`manual/agents.md`).
- The agents endpoint registers tools marked on handlers and Live actions
  (AGENT-001) and serves them over MCP (AGENT-004 onward); no tool reads
  documentation.
- No framework or plugin route set serves a `docs/` folder; no `llms.txt`
  is produced anywhere.

## Requirements

[DOCS-001] A plugin crate, `crates/suprnova-docs`, distributed as the
payment adapters are, MUST mount a documentation site at a configurable
prefix from a `docs/` folder at the application root: a set manifest
(`docs/docs.json`: title, versions, the default version, languages), one
folder per version holding Markdown pages and a navigation file, and an
`_assets` folder per version. A page MUST render through
`suprnova-markdown` in the application's own layout, with the
navigation, a version switch and a language switch when the set has
more than one, and links between pages MUST resolve inside the version.
A set, a version or a page MUST be markable private, served only to a
request the application's gate admits.
Falsifier: a page listed in a version's navigation renders 404 or outside the layout; a link to a sibling page leaves the version; a private page answers a request the gate refused; or a set with one version shows a version switch.
Mechanism: `docs-plugin`.
Rationale: LaRecipe 3's content model (`ARCHITECTURE.md` 4.1 to 4.6: the set manifest, `navigation.json` per version, page frontmatter, doc sets) and its versions, languages and private docs.
Status: Agreed 2026-10-23

[DOCS-002] Every page MUST also be served as Markdown, to a request
whose `Accept` lists `text/markdown` and at the page's `.md` address,
and the set MUST serve `llms.txt`, an index of its pages with one line
each, and `llms-full.txt`, every page's Markdown in navigation order,
for the default version and for each other version at its own address.
Falsifier: a page's `.md` address answers HTML; `llms.txt` lists a private page to a request the gate refused; or `llms-full.txt` differs from the concatenation of the pages it lists.
Mechanism: `docs-plugin`.
Rationale: LaRecipe's "Every page as Markdown" and `llms.txt` / `llms-full.txt`; the llms.txt convention (llmstxt.org).
Status: Agreed 2026-10-23

[DOCS-003] The plugin MUST index the set at boot and serve search in the
browser from that index with no service to run, and MUST register three
read tools on the application's agents endpoint, `search_docs`,
`read_page` and `list_pages`, which return Markdown and page addresses,
never HTML, and respect the same gate as the pages.
Falsifier: a search for a heading's words returns no hit for its page; the agents endpoint lists no docs tool; `read_page` returns HTML; or a tool returns a private page to an unadmitted caller.
Mechanism: `docs-plugin`.
Rationale: LaRecipe's in-browser search, its MCP read tools (`search_docs`, `read_page`, `list_pages`) and the agents domain of its architecture; AGENT-001 to AGENT-004 give the endpoint.
Status: Agreed 2026-10-23

[DOCS-004] The dogfood application MUST serve a documentation set
through the plugin, with at least two versions and one private page, and
`manual/docs-site.md` MUST document the plugin; suprnova.app adopts it
for the manual at the release.
Falsifier: the dogfood application has no `docs/` set or serves it outside the plugin; its set has one version or no private page; or the manual has no chapter for the plugin.
Mechanism: `docs-plugin`.
Rationale: the dogfood application exercises every subsystem (overview); the site's adoption is release work on another repository, which no mechanism here can observe.
Status: Agreed 2026-10-23
