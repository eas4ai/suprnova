# Installing Live Components

`suprnova live:add` installs a Live component into your application from the
shipped library or from a third-party library. A third-party component can
carry Rust as well as a view, a stylesheet and a script, so installing one
adds its author's code to your application. `live:add` treats it that way:
it verifies the component's signature, scans every file without running any
of it, shows you everything it will write and every capability the component
uses, and writes nothing until you approve. Then the copy is yours, and
`suprnova.toml` records where it came from.

This chapter follows one install from the command to a rendered component.
To publish a library of your own, see [Live Component
Libraries](live-libraries.md).

## Quick start

```bash
suprnova live:add acme/acme-ui/counter
```

`live:add` fetches the component from the newest release of the library at
`github.com/acme/acme-ui`, verifies it, scans it, and prints its plan. On a
terminal it asks you to approve each capability the component uses, to pin
the library's key, and to confirm the plan. Then it writes the files,
declares the component's Rust module, registers its Live component, and
records the install. A shipped component installs the same way, by its
name, with no questions:

```bash
suprnova live:add field
```

## Sources

`live:add` takes one of four sources:

| Source | Example | Installs from |
|---|---|---|
| A name | `field` | The shipped library, embedded in the CLI. `suprnova.field` is the same. |
| A repository address | `acme/acme-ui/counter`, `gitlab.com/acme/acme-ui/counter@1.2.0` | The library at the root of that repository, at a release tag. The host is `github.com` when you omit it, or `gitlab.com` or `codeberg.org`. |
| An `https://` URL | `https://ui.acme.test/acme-ui/components/counter` | The library two levels above the component directory. |
| A path | `../acme/components/counter` | A library tree on disk. The path starts with `./`, `../` or `/`, and names a component directory or its `manifest.json`. `--manifest <file>` is the same source. |

A repository address is `[<host>/]<owner>/<library>/<component>[@<version>]`.
With no version, `live:add` takes the highest `v<semver>` tag the forge
lists, pre-releases excluded, and refuses a repository with no release tag.
`@1.2.0` takes the tag `v1.2.0`.

Every source resolves to one library address and one component address
before anything is fetched. A repository's library address is
`<host>/<owner>/<library>`, lowercase, so `Acme/Acme-UI/counter` and
`acme/acme-ui/counter` are one component. A URL's is its lowercase scheme
and host, a port that is not the default, and the path two segments above
the component, with no trailing slash. A path's is the absolute library root
with symbolic links resolved. A library fetched by repository address or URL
must name that address as the `source` in its `library.json`, so a fork that
copies another library's files and signatures is refused. A path source is
an author's tree on disk, whose `source` names where it will be published.

Fetching follows fixed rules. Every request is HTTPS, except to a loopback
host, and carries no credentials. A response over 2 MiB, one that takes over
30 seconds, a redirect to another origin, and a JSON document that is not one
object or holds a duplicate key are refused. Repository files are read raw
at the commit the tag names, never as release assets, and a redirect to
another repository path, as a renamed repository answers, is refused with
the new address named. Only public repositories are supported.

## The plan

Before it writes anything, `live:add` fetches, verifies and scans the whole
plan, dependencies included, and prints it. This is the plan for a first
install of the example component of a library `live:registry new acme`
scaffolded and published at `github.com/acme/acme-ui`:

```text
framework: suprnova 3.3.0
github.com/acme/acme-ui/counter 0.1.0
  commit      5b0f3c9e2a7d41e8c6b1f0a9d3e5c7b2a4f6e8d0
  hash        sha256:58982952bdf9cfc32fe817ffc459ab3a9302e68581b1eaef1a77b59d2049653e
  templates/acme-ui/counter/manifest.json  new
  templates/acme-ui/counter/counter.html  new
  templates/acme-ui/counter/counter.css  new
  templates/acme-ui/counter/counter.js  new
  src/live/acme/counter.rs  new
  module      src/live/mod.rs: pub mod acme;
  module      src/live/acme/mod.rs: pub mod counter;
  register    crate::live::acme::counter::Counter
github.com/acme/acme-ui/counter: capabilities: none
key: pin sha256:0d7344c941b351c781be6ee0ce4c3146a85a2df7d526d1f2c45f2160a21d27c7 for github.com/acme/acme-ui (no key is pinned for it yet)
serve its assets: call `router.try_live_ui_assets_for("acme")` when you build the router
```

| Line | Means |
|---|---|
| `framework:` | The `suprnova` version your application depends on, which each library's `framework` requirement was checked against. It comes from `Cargo.lock`, or, with no lock, from the `v<version>` tag of the `suprnova` git dependency in `Cargo.toml`. With neither, `live:add` refuses and asks you to run `cargo generate-lockfile`. |
| `<address> <version>` | One component and the library version it comes from. Dependencies come first. |
| `commit` | The commit the release tag resolved to. Every file of the library comes from it. |
| `hash` | The component's verification hash. The signature was verified over it. |
| `<path>  <outcome>` | Each file, where it lands, and what happens to it: `new`, `unchanged`, `replaced`, `kept, edited locally`, or `kept, changed since the record`. See [Updates, edits and --force](#updates-edits-and---force). |
| `module` | A module declaration `live:add` adds, and the file it goes in. |
| `register`, `unregister` | A Live component it registers in your registry builder, or removes because the new version no longer defines it. |
| `depends on` | A dependency and the version it resolved to. |
| `capabilities:` | Each capability the scan found the component uses, or `none`. |
| `approval` | Each capability's approval: given at an earlier install, given by `--allow`, or needed from you. |
| `key:` | A key `live:add` would pin, or re-pin after a key change, by fingerprint. |
| `serve its assets:` | The router call that serves the library's stylesheets and scripts, shown at the library's first install. |

A library's files land under `templates/<namespace>-ui/<component>/` (views,
stylesheets, scripts, the manifest and the install record) and
`src/live/<namespace_module>/` (Rust), where `<namespace_module>` is the
namespace with each hyphen written as an underscore.

## Capabilities

A component reaches a database, the network, files, mail, a queue, a cache,
the session, the environment or a process only through Suprnova's API, and
every part of that API carries a capability. The scan reports each
capability a component uses, and you approve each one by name before it
installs:

| Capability | Allows the component to |
|---|---|
| `database` | Read or write a database. |
| `network` | Open a connection to another host. |
| `files` | Read or write files, a storage disk included. |
| `mail` | Send mail. |
| `queue` | Push to or work a queue. |
| `cache` | Read or write a cache. |
| `session` | Read or write the session. |
| `environment` | Read the environment or the configuration. |
| `process` | Start a process. |

On a terminal, `live:add` asks about each capability in turn. Without a
terminal, approve them with `--allow`, once per capability:

```bash
suprnova live:add acme/acme-ui/invoice-table --yes --allow database --allow cache
```

Denying a capability, or leaving one unapproved without a terminal, refuses
the install with nothing written. `suprnova.toml` records each approval, so
an update that uses the same capabilities installs without asking again, and
an update that uses a new one asks for that one. A component that uses no
capability needs no approval.

## Confirm the plan

`live:add` asks you to confirm a plan that holds any component from a
library but the shipped one. A terminal here means standard input and
standard error are both a terminal, and no is the default answer. Without a
terminal, `live:add` refuses unless you pass `--yes`.

`--yes` confirms the plan and does nothing else. It never approves a
capability, which takes `--allow`, and never pins or re-pins a key, which
takes a terminal or a pin you write yourself.

## Trust on first use

The first install from a library pins the key its `library.json` names, in
`suprnova.toml`, once you confirm on a terminal. Every later install from
that library must be signed by the pinned key. A library whose key differs
from its pin is refused, with both fingerprints named, unless the pinned key
handed the library to the new key: then the plan shows both fingerprints, and
`live:add` re-pins only when you confirm on a terminal.

To install without a terminal, pin the key yourself first. Add the library's
table to `suprnova.toml`, with the key from its `library.json`:

```toml
[live.libraries."github.com/acme/acme-ui"]
key = "ed25519:6HedMVA4Z2cro3wHx0Fh7Vqb3WLDpUtZCsqhG7DpkJA="
```

A pinned key proves continuity, not identity: every version you install
later comes from whoever held the key when you first installed, or from
whoever that holder handed it to. It does not prove who that is. Decide
whether to trust a library before its first install, from its address and
its author.

A namespace stays with the library you first installed it from. A second
library that claims the same namespace is refused, with both addresses named.

## The provenance record

`suprnova.toml`, at your project root, is your application's record of
every component it installed. `live:add` creates it when it is missing and
edits it in place: every comment, key and table it does not own stays as it
was. This is the record the plan above wrote:

```toml
[live.components."github.com/acme/acme-ui/counter"]
source = "github.com/acme/acme-ui"
version = "0.1.0"
commit = "5b0f3c9e2a7d41e8c6b1f0a9d3e5c7b2a4f6e8d0"
library_json = "sha256:867b1291d119d68bee3e3a46ce007a7ca8ab60bbae23a04892ad7a6eb9e9ec81"
manifest = "sha256:7ff819c5e8fb1d4439ca865cbddd2a690f4d3b31433c56ebed0850e8c3cb99e8"
hash = "sha256:58982952bdf9cfc32fe817ffc459ab3a9302e68581b1eaef1a77b59d2049653e"
signature = "FeJtqDThMDVmGseOl4KmygWPQghVwD0dUyDWzQw9/+xBzWmLWfvlopI4n2O9LKVuNYhshCLDgqeKSdVoR1SuAA=="
files."counter.css" = "sha256:c44ae7cd896d867174fb62154a80e113c4a67b5bb56bf3d578ca4c3650a7bca2"
files."counter.html" = "sha256:174ccd0130ffc0e366ddc8a25888113e392e185f203d6d15d68d6120bd615f42"
files."counter.js" = "sha256:49a53e15ede8c5a59bf1bd577ad333211aac956aee507f7ad412b17a7ce19acd"
files."counter.rs" = "sha256:048febcec71d261406a1931ba59b42f9a2df630d0df8a093be448928cbd85c73"
registered = ["crate::live::acme::counter::Counter"]

[live.libraries."github.com/acme/acme-ui"]
namespace = "acme"
key = "ed25519:6HedMVA4Z2cro3wHx0Fh7Vqb3WLDpUtZCsqhG7DpkJA="
```

Each `[live.components."<address>"]` table holds exactly what arrived:

| Key | Holds |
|---|---|
| `source`, `version` | The library's `source` and the version installed. |
| `commit` | The commit the release tag resolved to. A URL or path source has none. |
| `library_json`, `manifest` | The digests of `library.json` and `manifest.json` as they arrived. |
| `hash`, `signature` | The verification hash and the signature over it. |
| `files` | The digest of each file the manifest names, as it arrived. |
| `dependencies` | Each dependency's address and the version it resolved to. |
| `registered` | The full path of each Live component it registered, as the registration line names it. |
| `capabilities` | Each capability the scan found, with how you approved it: `terminal` or `flag`. |
| `kept` | Each file the install kept because you had edited it. |

Each `[live.libraries."<address>"]` table holds the namespace the library
owns in your application and the key pinned for it. A shipped component's
table holds only the CLI's version and digests, since the shipped library is
neither signed nor hashed:

```toml
[live.components."suprnova/field"]
source = "suprnova"
version = "3.3.0"
manifest = "sha256:9540febdf7b22bd79c2fdcede86e14b6e9693201de58620e564ced94f7e85620"
files."field.css" = "sha256:3b552944e95a30c31439afd0248491e77b301e9d26be9bf73beb78991a97b7c5"
files."field.html" = "sha256:f576888a8dbad260b3e42cf6be7981fc7d69a9c40cbadedddb95800be4d26d4b"
```

The record holds what arrived, never what is on disk now: editing a vendored
file changes no provenance. Each component directory's
`.suprnova-installed.json` holds the digest of the bytes `live:add` last
wrote at each path, which is how it tells your edits from its own files.

## Render the component

An installed component is registered, but nothing renders it until a page
mounts it. Serve the library's stylesheets and scripts with the call the
plan named, and render the component as its own island. This page module,
`src/counter_page.rs`, does both:

```rust
//! The installed counter, rendered by its own island.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

use crate::live::acme::counter::Counter;

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "counter_page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

/// Serves the acme library's stylesheets and scripts and the counter page.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let router = router.try_live_ui_assets_for("acme")?;
    let mount = LiveMount::<Counter>::public_seed("/counter", "counter", "acme-counter")?;
    let handler = mount.clone();
    let router: Router = router
        .get("/counter", move |request: Request| {
            let mount = handler.clone();
            async move { render(request, &mount).await }
        })
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("counter_page.html")
                    .map_err(|_| FrameworkError::internal("view name"))?,
                &Page {
                    bootstrap: bootstrap.html(),
                    counter: counter.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("response intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("counter page: {error}")).status(500))
}
```

Its view, `templates/counter_page.html`, links the component's stylesheet
and script from the namespace's route:

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Counter</title>
<link rel="stylesheet" href="/acme-ui/counter/counter.css">
<script type="module" src="/acme-ui/counter/counter.js"></script>
{{ bootstrap|trusted_html }}
</head>
<body>
<main>
{{ counter|trusted_html }}
</main>
</body>
</html>
```

Declare the module in `src/lib.rs` with `pub mod counter_page;`, and install
its routes in `src/live/mod.rs`, inside the RenderCache install:

```rust
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    RenderCache::install(
        crate::counter_page::routes(routes(router)?)?,
        RenderCacheConfig::from_env()?,
    )
    .await
}
```

Build and start the application, and `/counter` renders the count and its
button through the component's own island, with its stylesheet served from
`/acme-ui/counter/counter.css`. The reserved Live routes of an application
`suprnova new` generated take `AuthMiddleware::new()`, so the button's action
runs for a signed-in visitor; [Routes](live.md#routes) shows the guard with
`AuthMiddleware::optional()` for anonymous actions.

`try_live_ui_assets_for("acme")` serves `templates/acme-ui/` at
`/acme-ui/{component}/{file}`: stylesheets and scripts only, never a view,
a Rust file, a manifest or the install record. Each library's namespace takes
one call; the shipped library's is `try_live_ui_assets()`. See [Component
library](live.md#component-library) for the shared rules.

## Updates, edits and --force

Run `live:add` again to update a component. Each file is judged against the
install record: a file you never edited is replaced when the library changes
it, a file you edited is kept, and the plan says which. A Rust file is judged
the same way. When a kept Rust file belongs to a component whose new version
registers different types, `live:add` refuses and names the file to
reconcile. `--force` replaces every edited file.

`suprnova.toml` pins the version you installed, and `live:add` guards it:

- A version lower than the recorded one is refused unless you pass `--force`.
- The recorded version with a different hash is refused, reporting that the
  library changed a released version, unless you pass `--force`.
- An install that would move a dependency to another version than an
  installed component recorded names that component and is refused unless
  you pass `--force`.

## Dependencies

A component's `dependencies` install first, each at most once, a cycle
included. A bare dependency is a shipped component, `./<component>` is a
component of the same library at the same version, and a full address
resolves as an address you type does. A plan that needs one component at two
versions is refused before anything is written, naming the components that
require each, and a plan of more than 64 components is refused.

## Dry run

```bash
suprnova live:add acme/acme-ui/counter --dry-run
```

`--dry-run` fetches, resolves, verifies, scans and reports the whole plan,
dependencies included, and makes the same framework check an install makes.
It writes nothing: not `suprnova.toml`, not the lock, not the journal.

## The lock and the journal

An install holds an exclusive lock on the project, the file
`.suprnova-live.lock` at its root, from before it reads your records until it
finishes. A second `live:add` refuses to start while one runs.

Before its first write, an install writes `.suprnova-live-journal.json`,
naming every path it will change or create and the prior bytes of each it
will change. When any step after that fails, it puts every changed file back,
`suprnova.toml` included, removes every file it created, and deletes the
journal. When an install is killed partway, the next `live:add` or `suprnova
serve` finds the journal with no lock held, restores from it before anything
else, and says so.

`suprnova serve` runs `suprnova live:wait` before each build. It returns at
once unless an install holds the lock, and otherwise waits for the install
to finish, so the dev server never builds a half-written install. Both files
are this machine's state: a scaffold's `.gitignore` names them.

## Verify installed components

```bash
suprnova live:check
```

Before it checks your views, `live:check` verifies every third-party
component `suprnova.toml` records, with no network: it recomputes each hash
from the recorded fields and verifies the recorded signature against the
key pinned for its library. A record altered by hand, a bad signature or a
missing pin fails the check. A vendored file whose bytes differ from what
arrived is reported as changed since install and does not fail it, since the
copy is yours to edit.

## The project file

The CLI reads one project file, `suprnova.toml`, all lowercase, at the
project root. `suprnova serve` reads its [extra dev
processes](cli-serve.md#extra-dev-processes) from it, and `live:add` writes
its records there.

An earlier CLI read `Suprnova.toml`; rename that file to `suprnova.toml`.
Until you do, `serve` and `live:add` refuse to run and name the rename. On a
file system that ignores case, rename it in two steps:

```bash
git mv Suprnova.toml suprnova.toml.tmp  # rename, step one
git mv suprnova.toml.tmp suprnova.toml
```

## What live:add never does

- Run anything a component carries: no build script, procedural macro,
  script or view runs while it fetches, verifies, scans or installs.
- Start a process.
- Change `Cargo.toml` or `Cargo.lock`, or add a crate to your build.
- Write anything before the whole plan is fetched, verified, scanned and
  approved.

A component that passes the scan can still fail to compile, for example
against another framework version than its author built it for. That failure
runs none of its code; fix or remove the component as you would your own.

### Why Suprnova diverges

shadcn's CLI copies a component's source from a registry into your project
with no signature and no scan. Laravel installs packages through Composer,
whose plugins run during the install, and a package's service provider runs
on every boot. `live:add` copies like shadcn, so the component is yours, but
trusts nothing it has not checked: the signature against a pinned key, every
file scanned as data, every capability approved by name, and a record in
`suprnova.toml` that `live:check` verifies again offline.

## Next

- [Live Component Libraries](live-libraries.md) - write, sign and publish a
  library of your own
- [Live](live.md) - components, documents, islands and the shipped
  component library
- [CLI Overview](cli.md) - every `live:*` command
- [suprnova serve](cli-serve.md) - the dev runner and `suprnova.toml`
