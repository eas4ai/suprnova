# Component registries

Status: Draft
Prefix: REG

The developer asked on 2026-10-03 for a Suprnova Live components SDK:
"I need to create a suprnova live components sdk for third party developers
to be able to make component libraries and I want to allow users the
familiar method of adding components like shadcn-ui from a registry. So we
would have to modify how the cli pulls in components for third parties
including the rust component." The same day, of the project file the CLI
reads, "it should be lowercase"; of installing, "the third party components
need to be sandboxed for validation and then validated as part of the
'pull'. All in the same action."; and then, "It needs to not just be
outside of the app... the code needs to be 100% sandboxed with no way to
execute". Of how: "cargo check is not an acceptable way to validate the code
prior to being deployed. It needs an intelligent scan... a signature. The
manifest should have a verification hash".

Of the project file, later that day: "I forsee using this as a 'registry'
for components in the application (not framework but the user application)
and registers where the component came from, the verification signature,
version and component name", "Like a provenance record". Of trusting a
library's key: "I am not sure about the key part. That means that in order
for the user to use it they have to register something right?" Of
addressing: "I would think that the alias needs to be based on a github repo
(or similar). <owner>/<library-name>/<component>". Of the library's layout:
"the github repository scaffold shouldn't have the component in the root.
So the SDK would scaffold a tree, right? So all libraries would have the
same source tree", and "yes" to one per-directory manifest format for the
shipped library and every third-party one. Of refusing the crates a
component declares: "we can't do that but we can prompt the user to
approve/deny"; then "Any capabilities should be from suprnova really... you
are now making me rething my crate approval", and "yeah I think we should
limit them to the suprnova api".

The model is shadcn's registry, addressed by repository. A library is a
repository with one fixed tree: `library.json` at its root and one
directory per component under `components/`, each holding a
`manifest.json` that names its files, the files, and a signature. A release
is a `v<version>` tag. `suprnova live:add <owner>/<library>/<component>`
fetches one component's files at a tag, verifies its signature and scans
it without running any of it, copies it into the application, which then
owns it, and records where it came from in `suprnova.toml`. Unlike the
shipped library, whose components are views, stylesheets and scripts only,
a third-party component may also carry the Rust of a Live component: its
state, actions and view, which the CLI writes into `src/live/` and
registers.

A third-party component is written against Suprnova's API and nothing
else: its own code, the components it depends on, the framework and the
crates the framework re-exports. It declares no crates, so nothing enters
the application's build that the scan did not read. The scan admits what
it can classify and refuses everything else. Every part of Suprnova's API
that reaches the database, the network, files, mail, a queue, a cache, the
session, the environment or a process carries a capability, and the
developer approves each capability a component uses before it installs.

What a reader can rely on today, at framework v3.1.0 (`2d9b8464`), observed and
not contract:

- The shipped library is 58 components in `crates/suprnova-live/components/`.
  Each is a directory holding a `manifest.json` (`name`, integer `version`,
  `root`, flat `files`, `elements`), a view, a stylesheet and, for 10 of
  them, a script. That is the tree this set makes every library use.
  `live:add <name>` installs one from the binary, and
  `live:add --manifest <file>` installs a third-party one from a manifest
  file on disk, writing a `.suprnova-installed.json` digest record into each
  component directory (UI-017, UI-022, UI-023). `live:add` requires the
  manifest `version` to be an integer and decides nothing by it. Every
  shipped element tag carries `sn-` (UI-018).
- No component carries Rust through `live:add`, but some depend on Rust that
  ships elsewhere: the chart's view is filled by `render_chart` in the Live
  crate, which needs charts-rs, and an application writes its own island
  around the datatable. A third-party library has no way to ship either.
- Views are Askama templates compiled into the application. A template
  expression can call a Rust function by its path and invoke a Rust macro,
  and a template can include, import or extend another, so a vendored view
  is server code, as trusted as a vendored Rust file. `live:check` checks a
  view's markup, accessibility and live directives; it builds and runs the
  application's console helper to read the compiled component registry,
  which its checker needs. Of expressions it refuses only the raw `safe`
  filter. The framework re-exports `TrustedHtml`, whose public
  `framework_generated` constructor trusts any markup it is given, and
  the `trusted_html` filter that emits it unescaped; Askama's
  `escape("none")` also writes a value unescaped.
- `live:make` already writes `src/live/<name>.rs` and inserts `pub mod` and
  `.register::<T>()` into `src/live/mod.rs`, finding the builder by
  searching the text for `pub fn registry()` and `.build()`.
- The framework serves component stylesheets and scripts only at
  `/suprnova-ui/{component}/{file}`, through `try_live_ui_assets()`, which an
  application mounts itself (manual `live.md`), or
  `try_live_ui_assets_from()` over another directory at the same URL. A
  component segment or file-name stem it serves is 1 to 64 bytes of
  lowercase letters, digits and hyphens, neither starting nor ending with
  a hyphen; a file name is such a stem then `.css` or `.js`. No route
  serves a namespace's own URL.
- `live:make` and `live:add` write into the scaffold's `src/live/mod.rs`,
  whose `registry()` binds `let registry = LiveRegistry::builder()` ...
  `.build();` and returns `Ok(registry)`. `suprnova new` writes no
  `Cargo.lock`; the scaffold's `Cargo.toml` names the framework as a git
  dependency at a `v<version>` tag.
- No JavaScript parser is in the workspace lock. `cssparser` is, through
  the Live crate, and Askama's parser is.
- The CLI has no HTTP client. The workspace lock already holds `reqwest`
  with rustls, which the framework uses, and the CLI already depends on
  Tokio. The lock also holds `toml_edit`, through `proc-macro-crate`; the
  CLI parses TOML with `toml`, which drops comments when it writes.
- `suprnova serve` reads extra dev processes from an optional
  `Suprnova.toml`, which no scaffold writes, and ignores every table but
  `serve`. It watches `src/`, `Cargo.toml` and `Cargo.lock` and rebuilds on
  a change.

Out of scope for this set: private repositories (authentication headers or
tokens), forge groups deeper than one owner segment, a library that does
not sit at its repository's root, a directory of libraries on
suprnova.app, theme and token components, `live:remove`, `live:diff` and
`live:update`, npm dependencies for scripts, and an MCP server over
libraries.

## Project file and provenance

[REG-001] The project file the CLI reads MUST be `suprnova.toml`, all
lowercase, at the project root. `suprnova serve` MUST read its extra dev
processes from it. A project holding a file named `Suprnova.toml` and none
named `suprnova.toml`, by its exact directory entry, MUST make `serve` and
`live:add` fail with a message that names the rename. `live:add` MUST
create the file when it is absent and MUST edit it in place, keeping every
comment, key order and table it does not own byte for byte. The manual MUST
name `suprnova.toml` as the file.
Falsifier: `serve` reads dev processes from a file named `Suprnova.toml`; a project holding only `Suprnova.toml` starts `serve` or runs `live:add` without naming the rename, on a case-sensitive or a case-insensitive file system; an install changes a comment or a `serve` entry in `suprnova.toml`; or a manual chapter presents `Suprnova.toml` as the file to write.
Mechanism: `registries`.
Status: Draft

[REG-013] `suprnova.toml` MUST be the application's provenance record.
For each installed component, `live:add` MUST write one
`[live.components."<address>"]` table, keyed by the component's canonical
address (REG-008), holding exactly what arrived: the library's `source`
and version (REG-025), the commit the tag resolved to (REG-026), the sha256 digests of
`library.json`, of `manifest.json` and of each named file by file name,
the verification hash (REG-023), the signature (REG-024), the components
it depends on with the version each resolved to (REG-010), the full path
of each type it registered exactly as the
registration line it wrote (REG-005), and each capability the scan found
it uses with the developer's approval of it (REG-006). When an install
keeps an edited Rust file (REG-028), the capabilities MUST be those of
the Rust that ends up installed, the kept file included, and the table
MUST name each kept file. A shipped component's table MUST hold the CLI's
version and its file digests, and no hash or signature.
Falsifier: an installed component has no table, or a table whose fields differ from what arrived; a recorded registration differs from the line written into `src/live/`; a capability the scan found is missing from the table; an install that keeps an edited Rust file records the incoming file's capabilities in place of the kept file's, or does not name the kept file; or the table holds the digest of a file the application edited.
Mechanism: `registries`.
Rationale: The developer's "provenance record" in the project file; recording the signed statement's own fields is what lets REG-027 verify it again offline.
Status: Draft

[REG-028] `live:add` MUST keep, in `.suprnova-installed.json` in the
component's directory under `templates/<namespace>-ui/`, the sha256 digest
of the upstream bytes it last wrote at each installed path, Rust files
included, keyed by the path from the project root. UI-022's behavior MUST
hold for every file type, judged against that digest: an unedited file is
replaced, an edited one is kept, and `--force` replaces it. A kept file
MUST keep its previous digest. A record keyed by bare file names, as
today's are, MUST be read as files of its directory and rewritten in the
new form.
Falsifier: an edited vendored file, Rust included, is overwritten without `--force`; an unedited one is kept after the library changes it; after an edited file is kept once, a second install replaces it without `--force`; or a record in today's form is ignored.
Mechanism: `registries`.
Rationale: Edit detection is not provenance: what arrived (REG-013) and what was last written part ways as soon as an edited file is kept.
Status: Draft

[REG-027] `suprnova live:check` MUST verify every third-party component
`suprnova.toml` records, without the network: the recorded signature
against the key pinned for its library, over the hash recomputed from the
recorded fields. A bad signature or a missing pin MUST fail the check. An
installed file whose bytes differ from what arrived MUST be reported as
changed since install, and MUST NOT fail it.
Falsifier: a table whose `source`, version, a digest or the signature was altered by hand passes `live:check`; a vendored file the application edited fails it; or the check makes a network request.
Mechanism: `registries`.
Rationale: Ruled 2026-10-05: the check lives in `live:check`, so it runs wherever `live:check` runs, the release gate included, and nobody has to remember a second command.
Status: Draft

## Libraries and components

[REG-025] A library MUST be one tree, the same for every library:
`library.json` at its root, and under `components/` one directory per
component, named for it, holding `manifest.json`, the files the manifest
names and `manifest.sig`. A component directory's name MUST be 1 to 64
bytes of lowercase letters, digits and hyphens, neither starting nor
ending with a hyphen, the asset route's component segment. `library.json`
MUST be one JSON object holding `namespace`, `source`, `version`,
`framework` and `publicKey`, and optionally `title` and `description`, and
nothing else. `source` MUST be the library's canonical address (REG-008)
where it is published; `version` a semver version; `framework` a semver
requirement; `publicKey` the library's signing key (REG-024).
Falsifier: `live:add` installs from a tree with no `library.json`, with a component outside `components/`, with a component directory of 65 bytes or one ending in a hyphen, or with a `library.json` key outside that set.
Mechanism: `registries`.
Status: Draft

[REG-002] A component's `manifest.json` MUST be one JSON object of at
most 1 MiB holding `name` and `files`, and optionally `root`, `title`,
`description`, `elements`, `register` and `dependencies`, and nothing
else; it carries no version, which is the library's, and declares no
crates. `name` MUST be `<namespace>.<directory>`, and `root`, when
given, MUST be `<namespace>-ui/<directory>`. `live:add` MUST refuse any
other manifest. The shipped library's manifests MUST hold to the same
format, their integer `version` removed.
Falsifier: a manifest with a key outside that set, `cargoDependencies` included, a body of more than 1 MiB, or a `name` or `root` that does not match its directory installs, or a shipped manifest carries a key a third-party manifest could not.
Mechanism: `registries`.
Rationale: UI-017 holds as Agreed: a component is one directory described by one JSON manifest that names its files, and `live:add` accepts a third-party manifest in the same format; this set adds the library around it.
Status: Draft

[REG-003] Every file a manifest names MUST be a single file name in the
component's directory, named once, of at most 1 MiB, and its extension
MUST give its type and where it lands: `.html`, `.css` and `.js` under
`templates/<namespace>-ui/<directory>/`, and `.rs` under
`src/live/<namespace_module>/`, where `<namespace_module>` is the namespace
with each hyphen written as an underscore. A component MUST name at least
one `.html` view. A `.css` or `.js` name MUST be one the asset route
serves: a stem of lowercase letters, digits and hyphens of at most 64
bytes. A `.html` name MUST come from the closed character set `live:add`
enforces today. A `.rs` name MUST be a Rust identifier that is not a
keyword, `mod` or `lib`. Two components of one library MUST NOT install
the same path.
Falsifier: a component installs a file outside its type's directory, a file of any other extension, a name listed twice, a stylesheet or script the asset route would not serve, `mod.rs`, or a path holding `..`, a leading dot, an uppercase letter or a separator; or a second component of the same library overwrites a Rust file the first one installed.
Mechanism: `registries`.
Status: Draft

[REG-004] A namespace MUST be one segment of lowercase letters, digits and
hyphens, starting with a letter, of at most 32 bytes, whose module form
is not a Rust keyword. The namespaces `suprnova`, `sn` and `live` MUST be
refused from any library but the shipped one. Every element tag a
third-party manifest declares MUST start with `<namespace>-`; the shipped
library's tags keep `sn-` (UI-018). Every Live component name a
component's Rust defines MUST start with `<namespace>.`.
Falsifier: a third-party component installs under `templates/suprnova-ui/` or `src/live/suprnova/`, uses the namespace `self`, or declares an element or defines a Live component name without its namespace prefix; or a shipped tag loses `sn-`.
Mechanism: `registries`.
Rationale: Refines UI-015, UI-016 and UI-018 for libraries other than the shipped one.
Status: Draft

[REG-005] A manifest's `register` MUST name, as `<module>::<Type>`, each
Live component its Rust files define, relative to the library's namespace
module: `<module>::<Type>` is
`crate::live::<namespace_module>::<module>::<Type>`, and that full path is
what the registration line and the record (REG-013) both write. `live:add`
MUST declare the component's modules, adding `pub mod <namespace_module>;`
to `src/live/mod.rs` and `pub mod <module>;` to
`src/live/<namespace_module>/mod.rs`, and MUST register each named type in
the application's registry builder exactly once, removing the registration
of a type the incoming `register` drops. The builder `live:add` can find
MUST be the scaffold's form, found by parsing `src/live/mod.rs` with
`syn`: one `pub fn registry()` whose body is either one
`LiveRegistry::builder()` chain ending in `.build()`, or, as the scaffold
writes it, one `let` binding of that chain followed by `Ok(<binding>)`,
with any number of `.register::<T>()` calls in the chain. Any other form,
or more than one, MUST get the lines to add reported, with nothing
written for the whole install. When
an install keeps an edited Rust file of a component (REG-028) and the
incoming `register` differs from the recorded one, `live:add` MUST refuse
before writing, naming the file to reconcile.
Falsifier: a component with Rust gets its lines reported instead of written in an application `suprnova new` just generated; after a well-typed component with Rust installs into a scaffolded application, the application does not compile, the registration line names a path other than `crate::live::<namespace_module>::<module>::<Type>`, the component is not reachable through its Live route, or installing it again adds a second module declaration or registration; an unrecognized builder form gets any line written; or an update that renames a type in a kept file writes its registration.
Mechanism: `registries-compile`.
Rationale: UI-014 holds: the application still registers every component explicitly; the CLI writes the registration into the application's code, and nothing registers itself.
Status: Draft

[REG-006] A third-party component MUST reach effects only through
Suprnova's API, and `live:add` MUST NOT add, change or remove any
dependency of the application. Every part of Suprnova's public API that
reaches a database, the network, files, mail, a queue, a cache, the
session, the environment or a process MUST carry one capability from a
closed list the allowlist names. The plan MUST show each capability the
scan (REG-030) finds a component uses, and `live:add` MUST install only
after the developer approves every one, by name, on a terminal or with
`--allow <capability>`; `--yes` MUST NOT approve a capability, and denying
one MUST refuse the install with nothing written. An update that uses a
capability the recorded approval does not hold MUST ask again. A
component that uses no capability needs no approval.
Falsifier: `live:add` changes `Cargo.toml`; a component installs whose Rust uses a capability the developer did not approve, or a capability the plan did not show; `--yes` alone approves a capability; a denial leaves any file written; or an update that adds a capability installs without asking.
Mechanism: `registries`.
Rationale: The developer, 2026-10-03: "Any capabilities should be from suprnova really" and "yeah I think we should limit them to the suprnova api", after first ruling that declared crates be approved rather than refused ("we can prompt the user to approve/deny"); the approval now covers capabilities, and no crate enters the build.
Status: Draft

[REG-007] A library's `framework` MUST be checked against the version of
the `suprnova` package the application's own package depends on in
`Cargo.lock`, and a component from a library it does not admit MUST be
refused, naming both. With no `Cargo.lock`, the version MUST be the one
the `v<version>` tag of that git dependency in the package's `Cargo.toml`
names, as the scaffold writes it; with neither, `live:add` MUST refuse,
saying to run `cargo generate-lockfile`. `live:add` MUST NOT create or
change `Cargo.lock`.
Falsifier: a component from a library requiring `>=4.0.0` installs into an application locked to `3.0.0`, or into one with no lock whose `Cargo.toml` names the tag `v3.0.0`; an application with no lock and a framework dependency on a branch installs without the check; or `live:add` writes `Cargo.lock`.
Mechanism: `registries`.
Status: Draft

## Addressing and fetching

[REG-008] `live:add` MUST accept four sources. A bare name is a component
of the shipped library. `[<host>/]<owner>/<library>/<component>[@<version>]`
is a component of the library at the root of that repository, where
`<host>` is `github.com` when omitted, or `gitlab.com` or `codeberg.org`,
and `<owner>` and `<library>` are one segment each. An `https://` URL of a
component directory is a component of the library two levels up, served as
the same tree. A path starting with `./`, `../` or `/` to a component
directory or its `manifest.json` is a component on disk inside a library
tree, and `--manifest <file>` MUST remain a spelling of it.
`<owner>` and `<library>` MUST be ASCII letters, digits, `-`, `_` and
`.`, not starting with `.`, and a URL's path segments the same; a URL
MUST carry no user, query, fragment, percent escape, `.` or `..` segment.
Every source MUST be resolved to one library address and one component
address before anything is fetched. For a repository, the library address
is `<host>/<owner>/<library>`, lowercase. For a URL, it is the URL's
scheme, lowercase host, port when not the default, and the path two
segments above the component directory, with no trailing slash. For a
path, it is the absolute library root with symbolic links resolved, and a
path to `manifest.json` names its directory. A component address is its
library address, `/`, and its directory name; neither carries `@<version>`.
A redirect that changes the repository path, as a renamed repository's
does, MUST be refused, naming the new address.
Falsifier: `live:add acme/acme-ui/date-picker` fetches from any host but GitHub's; a URL source fetches outside its library's base; `Acme/Acme-UI/date-picker` and `acme/acme-ui/date-picker`, a URL with and without a trailing slash, or a component directory and its `manifest.json` record two addresses; or an install follows a redirect to another repository path.
Mechanism: `registries`.
Status: Draft

[REG-026] A repository source MUST be fetched at a tag: `@<version>` names
the tag `v<version>`, and with no version `live:add` MUST take the highest
`v<semver>` tag, pre-releases excluded, that the forge lists, and MUST
refuse a repository with none, saying to tag a release. Each tag a plan
uses MUST be resolved once to a commit, and every file of that library
at that version, dependencies included, MUST be fetched at that commit.
The library's `library.json` `version` MUST equal the tag's version.
`live:add` MUST refuse a version lower than the one `suprnova.toml`
records for the component unless `--force` is given, and MUST refuse the
recorded version when its verification hash differs from the recorded
one, reporting that the library changed a released version, unless
`--force` is given.
Falsifier: an install with no version takes a branch head or a pre-release; a tag whose `library.json` says another version installs; two files of one plan come from different commits of one tag; an older signed version replaces a newer recorded one without `--force`; or a tag moved to different signed content at the recorded version installs without `--force`.
Mechanism: `registries`.
Rationale: Ruled 2026-10-05: the newest release tag is the default; the record in `suprnova.toml` pins the version afterwards, and the refusals above cover a downgrade or a moved tag.
Status: Draft

[REG-009] `live:add` MUST fetch over HTTPS only, except from a loopback
host, MUST send no credentials, and MUST refuse a redirect to another
origin, a response of more than 2 MiB, one that takes more than 30
seconds, and a JSON document that is not one object or holds a duplicate
key. A repository's files MUST be fetched as raw files at the tag, never
as release assets.
Falsifier: a component installs from `http://example.test/`, after a redirect to another origin, from a 3 MiB file, from a response that stalls, or from a manifest that names `files` twice.
Mechanism: `registries`.
Status: Draft

[REG-010] `live:add` MUST install a component's `dependencies` before the
component, transitively, each at most once, and MUST refuse a plan of
more than 64 components. A bare dependency is a shipped component;
`./<component>` is a component of the same library at the same version;
any other dependency is a full address, which MAY carry `@<version>` and
otherwise resolves as REG-026 does. The version each dependency resolved
to MUST be recorded. A plan that resolves one component address to two
versions MUST be refused before anything is written, naming the
components that require each. A plan that replaces a component with a
version other than the one an installed dependent recorded MUST name that
dependent and be refused unless `--force` is given.
Falsifier: a component whose dependency names it back hangs or installs twice, a chain of 65 components installs, a `./` dependency installs from another version of its library, or two parents requiring one helper at `1.0.0` and `2.0.0` install.
Mechanism: `registries`.
Status: Draft

[REG-011] A namespace MUST stay with the library it was first installed
from: a component whose namespace `suprnova.toml` records for a different
library address MUST be refused, naming both.
Falsifier: two libraries that both claim namespace `acme`, including two on one host under different ports or paths, install into the same directories.
Mechanism: `registries`.
Status: Draft

## Installing

[REG-012] Before it writes anything, `live:add` MUST report its plan:
every file with its destination and whether it is new, replaced, kept or
changed since the record, every module declaration and registration, every
capability with its approval (REG-006), and, for a library with no pinned key,
the key it would pin by fingerprint. For a component from any library but
the shipped one, it MUST then ask for confirmation on a terminal, and
without a terminal it MUST refuse unless `--yes` is given.
Falsifier: a third-party component writes a file before the plan is shown, installs without confirmation or `--yes`, or a library that changed an installed component's script has the new script written with no report of the change.
Mechanism: `registries`.
Rationale: A view, a script and a Rust file are all code the application then runs, so installing a component is adding its author's code.
Status: Draft

[REG-029] `live:add` MUST fetch, verify and scan the whole plan,
dependencies included, before its first write, so a watcher or editor that
compiles on change only ever sees validated files. It MUST hold an
exclusive lock on the project from before it reads the project's records
until it finishes, refusing to start while another `live:add` holds it,
so no plan is made against records another install is changing. Before
its first write it MUST write a journal naming every path it will change
or create, with the prior bytes of each it will change. When any step
after the first write fails, it MUST restore every file it changed,
`suprnova.toml` included, to its prior bytes, and remove every file it
created. A `live:add` or `serve` that finds a journal with no lock held
MUST restore from it before anything else, and report that it did.
`suprnova serve` MUST NOT start a build while the lock is held, and MUST
build once after it is released.
Falsifier: a dependency that fails validation leaves its parent's or its own files written; a failed write leaves component files, module declarations or record entries behind; two concurrent installs both write, or the second plans against records the first is changing; an install killed after its first write leaves files the next `live:add` or `serve` does not restore; or `serve` builds during an install.
Mechanism: `registries`.
Status: Draft

[REG-014] `live:add --dry-run` MUST fetch, resolve, verify, scan and
report the whole plan, dependencies included, making the framework check
(REG-007) as an install would, and MUST write nothing, `suprnova.toml`, the
journal and the lock included.
Falsifier: a dry run changes or creates any file, or reports a framework check other than the one an install of the same plan would make.
Mechanism: `registries`.
Status: Draft

[REG-015] Nothing a component carries MAY execute while `live:add`
fetches, validates or installs it, and no code the scan did not read MAY
enter the application's build. `live:add` MUST start no process.
Falsifier: installing or validating a component runs any of its code, a script, a hook, a build script or a procedural macro; `live:add` starts a process; or an install adds a crate to the application's build.
Mechanism: `registries`.
Status: Draft

## Validation

[REG-022] `live:add` MUST validate every component from any library but
the shipped one in the same command, before it writes anything, and the
validation MUST give the component's code no way to execute: it verifies
the signature (REG-024) over the hash (REG-023), then scans the views,
stylesheets, scripts and Rust as data, compiling and running none of it,
`cargo check` and the application's console helper included. The scan
MUST read each file's syntax tree, not its text, and MUST admit only what
it can show performs no refused effect: a construct it cannot classify,
and a name it cannot resolve, MUST be refused, so a capability the scan
does not know is refused rather than missed. No finding can be
overridden. `live:check`'s view checks MUST run on the component's views
without compiling the application, against the Live component contracts
read from its Rust syntax. Acceptance proves the component's capabilities
and structure, not that it type-checks: a component that passes can still
fail to compile, and that failure runs none of its code. A failure MUST
write nothing into the application, MUST name the check, the file and the
line, and MUST leave nothing behind.
Falsifier: a component from the bypass corpus (REG-030, REG-031, REG-032) installs; a finding can be overridden; validation compiles or runs any of the component's code or the application; or validation leaves files on disk.
Mechanism: `registries-scan`.
Rationale: The developer asked for "an intelligent scan"; a list of refused names is bypassed by any spelling it does not list (a macro, a re-export, a computed property), so the scan lists what is admitted, limited to Suprnova's API by the developer's ruling. Still proposed: no overrides, the allowlist's exact entries, and that a model's review, if any, never decides an install. Ruled 2026-10-05: the crates Suprnova re-exports count as its API through the `suprnova::` path only, each reported in the plan as the capability it carries.
Status: Draft

[REG-030] In Rust, parsed with `syn`, a component MAY name only its own
items, the modules of the components it depends on, the documented public
API of Suprnova, including the crates it re-exports, and the effect-free
part of `std` the allowlist names, each after resolving every `use`, alias
and rename; every other path, a `#[doc(hidden)]` re-export included, MUST
be refused. The scan MUST refuse `unsafe`, `extern` blocks and functions,
`macro_rules!`, every macro, attribute and derive not on the allowlist,
`#[path]`, a `mod` declaration without a body, `env!`, `option_env!`,
`include!`, `include_str!`, `include_bytes!` and `cfg_attr`. It MUST report
every capability (REG-006) the admitted paths carry, re-exports included,
so `tokio::fs`, `opendal` and Storage all report files. A method call
MUST be classified by the method it resolves to on its receiver's type,
and refused when the scan cannot name that type. Resolving a service from
the container MUST be classified by the type it resolves; resolving a
trait the component's own library defines, which the application binds
to its own implementation, carries no capability, because what runs is
the application's code; a resolution whose type the scan cannot name MUST
be refused. `TrustedHtml`'s constructors MUST be refused, so the only
markup a view emits unescaped is markup the framework built from typed
data, as `render_chart` builds a chart's. Each type a
`#[live]` attribute defines MUST be exactly one entry of the manifest's
`register`, and the Live component name the attribute gives it MUST start
with `<namespace>.` (REG-004).
Falsifier: a component installs that expands a `macro_rules!` into a file read, names a crate Suprnova does not re-export or a `#[doc(hidden)]` re-export such as `inventory`, includes a file through `#[path]`, `mod x;` or `include_str!`, reads `env!`, reaches files through `suprnova::tokio::fs` without the files capability in the plan, names `std::fs` under any alias, resolves a database connection from the container without the database capability in the plan, constructs a `TrustedHtml`, or defines a Live component its manifest does not declare.
Mechanism: `registries-scan`.
Status: Draft

[REG-031] In a view, parsed with Askama's parser, an expression MAY read
the component's state and call only paths REG-030 admits that carry no
capability, and the framework's view helpers. The scan MUST refuse a Rust
macro in a view, a filter that is neither Askama's own nor the
framework's, every way of writing a value unescaped but `trusted_html`,
which takes only a `TrustedHtml` (REG-030), so `safe` and an `escape` or
`e` filter given any escaper but `html` included, and an
`include`, `import` or `extends` of anything but a view the component's
manifest names, one a manifest of a component it depends on names, or a
shipped component's view. The view's markup MUST hold no `script`,
`iframe`, `object`, `embed`, `base`, `link` or `meta` element, no
event handler attribute, no `javascript:` or `data:` URL, and no SVG
`animate`, `set` or `foreignObject` element. Every URL-bearing attribute
(`href`, `src`, `srcset`, `action`, `formaction`, `poster`, `data`,
`xlink:href`, `ping`) MUST hold a constant that stays on the application's
origin, or a value the scan shows comes from where the allowlist admits.
CSS, in a stylesheet, a `style` element or a `style` attribute, parsed as
CSS, MUST hold no `@import`, and every resource it names, `url()`,
`image-set()`, `@font-face` `src` and a custom property used as one
included, MUST stay on the application's origin, or be refused.
Falsifier: a component installs whose view holds `include_str!`, a call that carries a capability, a custom filter, `escape("none")`, an include of an application template or of a file its manifest does not name, an inline `script` element, a `base` or `meta` element, an `onclick` attribute, a `javascript:` link, a form whose `action` or an image whose `src` leaves the application's origin, or a `style` attribute loading a resource from another origin; or whose stylesheet imports or loads a resource from another origin, `image-set()` included.
Mechanism: `registries-scan`.
Rationale: Ruled 2026-10-05: a non-constant URL may come from a value the application passes in (as `account-menu` takes its links and form action) or from the framework's URL helpers (`route`, `url::to`, `Storage::url`), and never from a value the component computes itself. The developer named a future component registry on suprnova.app as a possible allowed source once it exists, with the same signature verification; it is not in this scope.
Status: Draft

[REG-032] In a script, parsed as a JavaScript module, every call MUST
resolve to a function the script defines or a standard browser API, and a
computed property access on a global object MUST be refused unless its key
is a constant. `Reflect.apply`, `Reflect.construct`, `Function.prototype`'s
`call`, `apply` and `bind`, getters, setters, `Proxy` traps and tagged
templates are calls, and what they invoke MUST resolve as any call's must;
`globalThis["eval"]` and `Function.prototype.constructor` are `eval` and
`Function`. The scan MUST refuse `eval`, `Function`, a timer given
anything but a function, a dynamic `import`, a static `import` from outside
the component and the components it depends on, a `Worker`,
`document.write` and `document.open`, creating a `script`, `iframe`,
`object` or `embed` element or one by a name it cannot trace to a
constant, HTML parsing into the document (`innerHTML`, `outerHTML`,
`insertAdjacentHTML`, `setHTMLUnsafe`, `createContextualFragment` and
`DOMParser`), `attachShadow` (UI-010), and a change to a built-in
prototype. An attribute name `setAttribute`, `setAttributeNS` or
`setNamedItem` receives MUST be a constant or trace within the script to
constants only, and MUST NOT be an event handler attribute (`on` and a
name), `srcdoc` or `style`. A URL passed to `fetch`, `XMLHttpRequest`,
`WebSocket`, `EventSource` or `sendBeacon`, assigned to `location` or
`window.open`, or given to a URL-bearing property or attribute, MUST be a
constant that stays on the application's origin; no `javascript:` or
`data:` URL stays on it. An element `customElements.define` defines MUST
have a constant name the manifest declares.
Falsifier: a component installs whose script calls `window["ev" + "al"]`, `Reflect.apply(Function, ...)` or `Function.prototype.constructor`, passes a string to `setTimeout`, calls `setAttribute("onclick", ...)`, sets `img.src` to a computed cross-origin URL or `location` to a `javascript:` URL, creates a `script` element, assigns `innerHTML`, calls `createContextualFragment`, or defines an undeclared element; or a shipped script fails the scan.
Mechanism: `registries-scan`.
Status: Draft

[REG-033] A library MUST be able to change its signing key: `library.json`
names the new key as `publicKey` and carries, as `previousKeys`, a
statement for each former key, naming the new key's fingerprint and signed
by that former key. When `live:add` finds a pinned key that differs from
`publicKey`, it MUST accept the change only if `previousKeys` holds a
valid statement from the pinned key for the new one, MUST show both
fingerprints in the plan and MUST re-pin only once the developer confirms
on a terminal; `--yes` MUST NOT re-pin. A changed key with no such
statement MUST be refused as REG-024 says.
Falsifier: a library whose new key is not vouched for by the pinned key installs; a vouched change re-pins under `--yes` or without a terminal; or a vouched change is refused.
Mechanism: `registries`.
Rationale: The developer accepted this on 2026-10-05. The pinned key proves continuity of control (the manual's words in the documentation requirement), so only that key can hand control to the next.
Status: Draft

[REG-023] Every component's verification hash MUST be `sha256:` and the
lowercase hex sha256 digest of its statement: the UTF-8 bytes of one JSON
object with no whitespace and these keys in this order:
`"format":"suprnova-component/1"`, `"library"`, the library's `source`,
`"version"`, its `version`, `"component"`, the directory name,
`"libraryJson"` and `"manifest"`, the digests of `library.json` and
`manifest.json`, and `"files"`, an object of each named file's digest by
file name, names in byte order. Every digest is written `sha256:` and
lowercase hex, and every value is a closed-character-set string (REG-003,
REG-008, REG-025), so the statement has one spelling. The hash covers the
component exactly as it arrived, never the files on disk: an application
that edits a vendored file afterwards changes no provenance, and
REG-028's digests are what detect the edit. `live:add` MUST compute the
statement from the bytes it fetched and the directory name the address
resolved to (REG-008), and MUST refuse, before it scans the component, a
component whose `library.json` `source` differs from the library address
it was fetched from, for a repository or URL source, or whose signature
does not verify over the statement. A path source is an author's own
tree on disk, so its `source` names where the library will be published,
not where it sits. The manual MUST publish
one statement, its hash, a key and a signature as a test vector.
Falsifier: a component whose `library.json`, manifest or any file changed by one byte after signing installs; a component signed under one directory name installs when another was requested; a fork that keeps the author's `library.json`, files and signatures installs from the fork's address; or the manual's test vector does not verify.
Mechanism: `registries`.
Status: Draft

[REG-024] Every component from a library other than the shipped one MUST
carry in `manifest.sig` an Ed25519 signature, over the ASCII bytes of its
verification hash (REG-023), by the key `library.json` names. `publicKey`
MUST be `ed25519:` and the standard padded base64 of the 32 key bytes; a
key's fingerprint is `sha256:` and the lowercase hex digest of those
bytes; `manifest.sig` MUST hold only the standard padded base64 of the 64
signature bytes. `live:add` MUST verify it against the
key `suprnova.toml` pins for that library under
`[live.libraries."<library address>"]`. The first install from a library
with no pinned key MUST pin the key `library.json` names, once the
developer confirms the plan that shows it on a terminal; `--yes` MUST NOT
pin a new key, and a developer MAY pin one by hand beforehand. A library
whose key differs from its pin MUST be refused, naming both fingerprints.
An unsigned component or a bad signature MUST be refused.
Falsifier: a component installs unsigned, with a signature from another key, or from a library whose `library.json` key differs from its pin; a new key is pinned under `--yes` with no terminal; or a component signed by `live:registry sign` fails to verify.
Mechanism: `registries`.
Rationale: The developer asked whether a user must register something first; trust on first use answers it, and he accepted it on 2026-10-05 ("Yes I am accepting your recommendations"). Key rotation is REG-033.
Status: Draft

[REG-016] The shipped library MUST be the built-in library, embedded in
the binary in the tree and manifest format of REG-025 and REG-002, and the
only source exempt from REG-022, REG-023 and REG-024. Every shipped
component MUST still pass the scan of REG-031 and REG-032.
Falsifier: a shipped component installs from a format a third-party author cannot produce in a library tree, or a shipped view, stylesheet or script fails the scan.
Mechanism: `registries-scan`.
Status: Draft

## Serving

[REG-017] The framework MUST serve a namespace's stylesheets and scripts
at `/<namespace>-ui/{component}/{file}` from `templates/<namespace>-ui/`,
under the contract `/suprnova-ui/` has (closed names, `.css` and `.js`
only, a 1 MiB cap, ETag, UI-021's refusal to start on an unreadable
directory), through one explicit router call per namespace.
`live:add` MUST report that call when it installs a namespace's first
component.
Falsifier: `templates/acme-ui/widget/widget.js` is installed and `/acme-ui/widget/widget.js` answers 404 after the documented call, or the route serves a view, a Rust file, an install record or `suprnova.toml`.
Mechanism: `registries-serve`.
Status: Draft

## Authoring

[REG-018] `suprnova live:registry new <namespace>` MUST scaffold the tree
of REG-025 with one example component holding a view, a stylesheet, a
script and a Rust Live component, and a `preview/` Suprnova application
that compiles and renders each component from where it sits under
`components/`, so an edit shows on the next reload with nothing copied,
with a test of the component. It MUST create the library's key pair,
write the public key into `library.json` and the private key outside the
project, and say where the private key is and that losing it strands every
pin. As generated the tree MUST pass `live:check`, `live:registry check`
and its own tests, and hold no `TODO`.
Falsifier: the scaffold fails one of those checks or its tests, contains a `TODO`, writes the private key inside the project, or needs a file copied for an edit under `components/` to show in the preview.
Mechanism: `registries-compile`.
Status: Draft

[REG-019] `suprnova live:registry check` MUST fail on any component
`live:add` would refuse for a reason the library alone decides: its tree,
its manifests, its signature, its scan and its names. It MUST resolve
dependencies as `live:add` does, failing on a `./` dependency its own tree
does not hold, a bare one the shipped library does not hold, and a full
address that does not fetch and verify. It MUST fail on a `register` entry
the preview application does not register under that name, and MUST list
each component's capabilities as `live:add` will show them.
Falsifier: `sign` signs a component that `live:add` then refuses for a reason the library decides, a bare dependency is satisfied by the library's own component, a `register` entry the preview never registered passes, or `check` lists capabilities other than those `live:add` shows.
Mechanism: `registries`.
Status: Draft

[REG-020] `suprnova live:registry sign` MUST run every check `check`
runs except verifying the signatures it is about to replace, then sign
every component, verify every new signature, and only then write every
component's `manifest.sig`, all or nothing. `check` alone MUST still
verify signatures. It MUST read the private
key from `SUPRNOVA_LIBRARY_KEY` when set, or from the user's configuration
directory, and MUST refuse a key file inside the project. The same tree
and key MUST sign to the same bytes. A named file MUST be a regular file
inside its component's directory (UI-023).
Falsifier: `sign` follows a symbolic link out of a component directory, writes some signatures when one component is invalid, refuses to re-sign a component edited after it was signed, reads a key from inside the project, or signs the same tree to different bytes; or `check` passes a stale signature.
Mechanism: `registries`.
Status: Draft

## Documentation

[REG-021] The manual MUST carry chapters, linked from
`manual/documentation.md`, that take an author from `live:registry new`
to a tagged release on GitHub, and a consumer from
`live:add <owner>/<library>/<component>` to an installed component
rendered by its own island, with its provenance in `suprnova.toml`, in the
chapter shape the manual requires. They MUST list the capabilities and
what each allows, and MUST say what a pinned key proves:
that later versions come from whoever held the key at the first install,
not who that is.
Falsifier: following a chapter's commands in order does not produce a signed, tagged library, or an installed, served component an island renders; or a chapter presents a key pinned on first use as proof of the publisher's identity.
Mechanism: `registries-compile`.
Status: Draft
