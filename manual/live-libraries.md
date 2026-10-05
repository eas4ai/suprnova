# Live Component Libraries

A Live component library is a git repository of Live components that any
Suprnova application can install with `suprnova live:add`. A component can
carry a view, a stylesheet, a script, and the Rust of a Live component: its
state, actions and view binding. The application that installs it owns the
copy, as it owns the components of the shipped library.

Every library has the same tree, the shipped one included, and every
component in it is signed with the library's key. `live:add` verifies the
signature and scans every file as data before it writes anything, so nothing
a component carries runs during an install. This chapter takes you from an
empty directory to a signed, tagged release. [Installing Live
Components](live-add.md) is the other side: what an application sees when it
installs from your library.

## Quick start

`live:registry new` scaffolds a library with one example component, a
preview application, and a signing key:

```bash
suprnova live:registry new acme --source github.com/acme/acme-ui
cd acme
suprnova live:registry check
```

`--source` is the address the library will be published at, which
`library.json` records as its `source`; see [library.json](#libraryjson).
Without `--source`, `new` writes an empty `source`, and `check`, `sign` and
`rotate-key` refuse until you set it in `library.json`.

The argument is the library's namespace. It prefixes everything the library
installs: views and assets under `templates/acme-ui/`, Rust under
`src/live/acme/`, Live component names (`acme.counter`), and custom element
tags (`acme-counter`). A namespace is 1 to 32 bytes of lowercase letters,
digits and hyphens, starts with a letter, and its module form (each hyphen
written as an underscore) is not a Rust keyword. The namespaces `suprnova`,
`sn` and `live` belong to the shipped library.

`new` prints where it wrote your private signing key. Back that file up
now; [The private key](#the-private-key) says why.

## The library tree

`live:registry new acme --source github.com/acme/acme-ui` writes this tree
into `./acme`:

```text
acme/
  library.json
  components/
    counter/
      manifest.json
      manifest.sig
      counter.html
      counter.css
      counter.js
      counter.rs
  preview/
```

`library.json` describes the library, `components/` holds one directory per
component, and `preview/` is an application that renders each component from
where it sits. The library sits at the repository's root: `live:add` reads
`library.json` there and each component under `components/`.

### library.json

`library.json` is one JSON object. This is the one `new` wrote above:

```json
{
  "namespace": "acme",
  "source": "github.com/acme/acme-ui",
  "version": "0.1.0",
  "framework": "^3.3.0",
  "publicKey": "ed25519:6HedMVA4Z2cro3wHx0Fh7Vqb3WLDpUtZCsqhG7DpkJA=",
  "title": "acme",
  "description": "Suprnova Live components."
}
```

It holds the keys in this table and no others:

| Key | Holds |
|---|---|
| `namespace` | The namespace every component installs under. |
| `source` | The library's address where you publish it: `github.com/<owner>/<repository>`, `gitlab.com/<owner>/<repository>` or `codeberg.org/<owner>/<repository>`, all lowercase, or the `https://` URL of the tree. `new --source` writes it; without `--source` it is empty, and `check`, `sign` and `rotate-key` refuse until you set it. `live:add` refuses a library whose `source` is not the address it fetched it from, so a fork that copies your files and signatures cannot install as yours. |
| `version` | The release, a semver version. It must equal the release tag: version `0.1.0` is the tag `v0.1.0`. |
| `framework` | A semver requirement on the `suprnova` version of the application. `live:add` refuses an application its requirement does not admit. `new` writes `^` and the version of the CLI that scaffolded the library. |
| `publicKey` | The library's signing key, `ed25519:` and the base64 of its 32 bytes. |
| `previousKeys` | Optional. A statement from each former key that hands the library to `publicKey`; see [Change the signing key](#change-the-signing-key). |
| `title`, `description` | Optional text for people. |

### manifest.json

Each component's `manifest.json` is one JSON object of at most 1 MiB. It
names the component's files and nothing more: a component has no version of
its own, since the library's version is the release, and it declares no
crates.

```json
{
  "name": "acme.counter",
  "root": "acme-ui/counter",
  "title": "Counter",
  "description": "A count kept on the server, raised by a button or the plus key.",
  "files": [
    "counter.html",
    "counter.css",
    "counter.js",
    "counter.rs"
  ],
  "elements": [
    "acme-counter"
  ],
  "register": [
    "counter::Counter"
  ]
}
```

| Key | Holds |
|---|---|
| `name` | `<namespace>.<directory>`. |
| `files` | Each file the component carries, by name, each once. At least one is an `.html` view. |
| `root` | Optional. `<namespace>-ui/<directory>`, which is also the default. |
| `title`, `description` | Optional text for people. |
| `elements` | Optional. Each custom element tag the component's script defines, each starting with `<namespace>-`. |
| `register` | Optional. Each Live component the component's Rust defines, as `<module>::<Type>`, where `<module>` is the stem of one of its `.rs` files. |
| `dependencies` | Optional. Components to install first: a shipped component by its name (`field`), a component of the same library and version (`./date-grid`), a repository address that may carry a version (`acme/acme-base/icon@1.2.0`), or the `https://` URL of a component on a public host. A path on disk is never a dependency. |

A file's extension decides where `live:add` writes it:

| Extension | Lands at | Name |
|---|---|---|
| `.html` | `templates/<namespace>-ui/<directory>/` | Lowercase letters, digits, hyphens and dots, at most 128 bytes |
| `.css`, `.js` | `templates/<namespace>-ui/<directory>/`, served at `/<namespace>-ui/<directory>/<file>` | A stem of 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen |
| `.rs` | `src/live/<namespace_module>/` | A lowercase Rust identifier that is not a keyword, `mod` or `lib` |

`<namespace_module>` is the namespace with each hyphen written as an
underscore. A component directory's name follows the same rule as a
stylesheet stem, because it is the asset route's component segment. Two
components of one library cannot install the same Rust file.

### The example component

The example's Rust is an ordinary Live component, named for the namespace:

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `acme-ui/counter/counter.html`.
#[derive(LiveComponent)]
#[live(name = "acme.counter", view = "acme-ui/counter/counter.html")]
pub struct Counter {
    /// The current count, rendered by the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Adds one in answer to `live:click="increment"`, stopping at the
    /// largest count rather than wrapping to zero.
    #[action]
    pub fn increment(&mut self) {
        self.count = self.count.saturating_add(1);
    }
}
```

Its view names the view path it installs at, `acme-ui/counter/counter.html`.
Every Live component name a library's Rust defines starts with
`<namespace>.`, and each `#[live]` type is exactly one entry of `register`.
When an application installs the component, `live:add` writes
`src/live/acme/counter.rs`, declares `pub mod acme;` and `pub mod counter;`,
and adds this line to the application's registry builder:

```rust
.register::<crate::live::acme::counter::Counter>()?
```

The script defines a light DOM custom element, `acme-counter`, when no
other script has defined that name. The view works without it.

## Preview your components

`preview/` is an application `suprnova new` generated, with the library's
components wired in where they sit. Nothing is copied into it:

- `askama.toml` makes `../components/` a second template root, and each
  view under `templates/acme-ui/` is a one-line stub that includes the
  library's view.
- `src/live/acme/mod.rs` compiles each component's Rust file from
  `../components/` with `#[path]`.
- `src/preview.rs` serves the stylesheets and scripts straight from
  `../components/` at `/acme-ui/<component>/<file>`, the same URL an
  application serves them at after it installs the component, and renders
  one page per component.

Run it from the preview's directory:

```bash
cd preview
suprnova serve --backend-only
```

Then open `http://localhost:8765/preview/counter`. The preview's own test
starts its server and reads the same page:

```bash
cargo test
```

`serve` rebuilds and restarts the preview when a file under its own `src/`
or `templates/` changes. The library's files under `../components/` sit
outside the preview, so `serve` does not watch them. A stylesheet or script
edit shows on the next reload, because the asset route reads the file on
each request. A view or Rust edit shows after the next rebuild: restart
`serve`, or save a file under `preview/src/`.

To preview a new component, give it a page in `src/preview.rs`: add its view
stub under `templates/acme-ui/<component>/`, a `#[path]` line in
`src/live/acme/mod.rs`, its registration in `src/live/mod.rs`, and its mount
and route, as the counter's are. `live:registry check` reads the preview's
registry, so a `register` entry the preview does not register fails the
check.

## Check the library

```bash
suprnova live:registry check
```

`check` runs, from the library's root, every check `live:add` makes for a
reason the library alone decides:

- The tree: `library.json`, the component directories, and their names.
- Each manifest and each file it names: every named file must be a regular
  file inside its component's directory, never a symbolic link.
- Each signature, over the component's verification hash.
- The scan of every view, stylesheet, script and Rust file; see [What the
  scan admits](#what-the-scan-admits).
- The names: Live component names, element tags and `register` entries.
- The dependencies, resolved as `live:add` resolves them: a `./` dependency
  must be in your tree, a bare one in the shipped library, and a full
  address must fetch and verify.

It lists each component's capabilities as `live:add` shows them to the
developer who installs it:

```text
counter: capabilities: none
```

A capability is an effect a component reaches through Suprnova's API, such
as `database` or `network`. [Installing Live
Components](live-add.md#capabilities) lists all nine.

## Sign the library

```bash
suprnova live:registry sign
```

`sign` runs every check `check` runs except the signatures it is about to
replace. Then it signs every component, verifies every new signature, and
only then writes every component's `manifest.sig`. When any component
fails a check, it writes no signature at all. Signing is deterministic:
the same tree and key always sign to the same bytes, so signing an
unchanged tree changes nothing.

Sign again after any change to a component or to `library.json`. Each
component's signature covers the digest of `library.json`, so a new
version, `source` or key leaves every signature stale until you sign
again, and `check` reports each one.

### The private key

`live:registry new` creates the key pair. It writes the public key into
`library.json` and the private key outside the library, into your
configuration directory:

| System | Key file |
|---|---|
| Linux and other Unix | `$XDG_CONFIG_HOME/suprnova/library-keys/<fingerprint>.key`, or `~/.config/...` when `XDG_CONFIG_HOME` is not set |
| macOS | `~/Library/Application Support/suprnova/library-keys/<fingerprint>.key` |
| Windows | `%APPDATA%\suprnova\library-keys\<fingerprint>.key` |

`<fingerprint>` is the hex digest of the public key, so each library has
its own file and `sign` finds it from `library.json` alone. On Unix the file
is readable only by you, and `new` never replaces a file that exists.

To sign on another machine, such as a release job, put the key file there
and name it with `SUPRNOVA_LIBRARY_KEY`:

```bash
SUPRNOVA_LIBRARY_KEY=/run/secrets/acme.key suprnova live:registry sign
```

`sign` refuses a key file inside the library, so a key never ships with a
release. Never commit it.

**Losing the private key strands every pin.** An application that installed
your library pinned its key, and from then on it installs only versions that
key signed, or that a key it handed the library to signed. Without the
private key you can sign no new version and hand the library to no new key,
so every application that pinned it stays on the version it has. Keep a
backup of the key file somewhere safe.

### What a signature covers

A component's signature covers its verification hash: the sha256 digest of
its *statement*, one JSON object with no whitespace and these keys in this
order:

| Key | Value |
|---|---|
| `format` | `suprnova-component/1` |
| `library` | The library's `source` |
| `version` | The library's `version` |
| `component` | The component's directory name |
| `libraryJson` | The digest of `library.json` |
| `manifest` | The digest of `manifest.json` |
| `files` | An object of each named file's digest by file name, names in byte order |

Every digest is `sha256:` and the lowercase hex digest of the file's bytes.
The hash is `sha256:` and the lowercase hex digest of the statement's UTF-8
bytes. `manifest.sig` holds the Ed25519 signature, by the key `library.json`
names, over the ASCII bytes of the hash, as the standard padded base64 of
its 64 bytes and nothing else, not even a final newline. A key's
fingerprint is `sha256:` and the lowercase hex digest of the key's 32 bytes.

The statement names the directory and the library's `source`, so a
component signed under one name or address does not verify under another,
and one changed byte in any file, the manifest or `library.json` changes
the hash.

### Test vector

This vector is the `widget` component of the fixture library the CLI's own
tests install, signed with a test key. A tool that builds the statement and
checks the signature the same way must reproduce the hash and accept the
signature. Never trust this key for anything else.

```text
statement {"format":"suprnova-component/1","library":"github.com/acme/acme-ui","version":"1.0.0","component":"widget","libraryJson":"sha256:94f8914ee1f7889c447b8e0b52251e4e9faeb4badf5c530b3023671972e96b02","manifest":"sha256:3a266a23a32091ed78d8ad3894166799be2bb2a2c8c7f728e5b54734d79699bf","files":{"widget.css":"sha256:5c2f8bed08981ed1f07fbdc1e9ad4d6d0536ec3f1548679b0bf6c36dfed9d81f","widget.html":"sha256:4dff021923cf2408f82e6733a90c590d004f5598d6751dffcd164261c5270471"}}
hash sha256:25a0757c3fed1fcc9ec54986400f546bc1c51f3ba49c5404900e5450dda37a8e
publicKey ed25519:5z4SbonRkzk8kbuQFeoIuaaV0iVATslPzuqhE7XuKwg=
signature OiIoP3FOXZcbC8AqnvgENWiqPucexXxto2Y+b8scJDMVTtw0bvlshIfAL3TKDtIn9dvvi8bTPNUiTVj8ueaLCA==
```

## Publish a release

A release is a `v<version>` tag in a public repository on GitHub, GitLab
or Codeberg. `live:add` reads the tag's files raw, at the commit the tag
names, and never reads release assets.

`live:registry new` creates no git repository. Run `git init` in the
library's root, and add the forge's repository as the `origin` remote. The
repository holds the whole tree, `preview/` included; the preview's own
`.gitignore` keeps its build output and `.env` out. Then, for each release:

1. Make sure `source` in `library.json` is the repository's address, such
   as `github.com/acme/acme-ui`.
2. Set `version` to the release, such as `0.1.0`.
3. Check and sign the library:

   ```bash
   suprnova live:registry check
   suprnova live:registry sign
   ```

4. Commit the signed tree, tag the commit with the version, and push both:

   ```bash
   git add .
   git commit -m "Release 0.1.0"
   git tag v0.1.0
   git push origin main v0.1.0
   ```

An application then installs a component by its address:

```bash
suprnova live:add acme/acme-ui/counter
```

With no version, `live:add` takes the highest `v<semver>` tag, pre-releases
excluded; `acme/acme-ui/counter@0.1.0` names one. A GitLab or Codeberg
library is addressed with its host first: `gitlab.com/acme/acme-ui/counter`.

For each later release, raise `version`, sign, commit, and tag again. Never
move a tag you published. An application records the hash of what it
installed, and `live:add` refuses the same version with different content,
reporting that the library changed a released version.

Keep a published repository's name. Every application that installed from
your library recorded its address and bound your namespace to it.
`live:add` refuses to follow a forge's redirect to a renamed repository, and
refuses a second address that claims a namespace an application already
gave to the first.

To publish without a forge, serve the same tree from any HTTPS host and set
`source` to its URL, such as `https://ui.acme.test/acme-ui`. An application
installs a component by its directory's URL,
`https://ui.acme.test/acme-ui/components/counter`, at the one version
`library.json` names.

## Change the signing key

To move the library to a new key, run `rotate-key` from the library's root:

```bash
suprnova live:registry rotate-key
```

It reads the current private key as `sign` does, from `SUPRNOVA_LIBRARY_KEY`
or your configuration directory, and makes a new key pair. It writes the new
private key into your configuration directory, named by its fingerprint, as
`new` does. It updates `library.json`: `publicKey` becomes the new key, and
`previousKeys` is rewritten as one statement per former key, each handing
the library to the new key. The former keys are the key you are leaving and
every key the library used before it, and each one signs its own statement:
`rotate-key` reads each earlier key from its file in your configuration
directory. It also raises the patch part of `version`, from `1.0.0` to
`1.0.1`: a rotation changes the signed content of every component, and
`live:add` refuses content that changed at a version an application
recorded. Then it signs every component again with the new key, all or
nothing, as `sign` does. It prints both fingerprints and the path of the new
key file. Back up the new key file, then commit the result and tag the new
version as for any release.

**Keep every former key file.** An application installs a release under a
new key only when the key it pinned vouches for that key. A library on its
third key needs a statement from each of its two former keys, so an
application pinned to either one can follow. When a former key's file is
missing, `rotate-key` refuses and names the key. To go ahead without it,
pass `--drop-key` with that key's fingerprint:

```bash
suprnova live:registry rotate-key --drop-key sha256:<fingerprint>
```

`--drop-key` leaves that key's statement out of `previousKeys`. An
application still pinned to the dropped key then refuses every later
release, because its pin vouches for no change, and its developer must pin
the new key by hand. Tell your users the new key's fingerprint through a
channel they already trust before you publish a release without a
statement they need.

After a change, `library.json` looks like this:

```json
{
  "namespace": "acme",
  "source": "github.com/acme/acme-ui",
  "version": "1.0.1",
  "framework": "^3.3.0",
  "publicKey": "ed25519:<the new key>",
  "previousKeys": [
    {
      "publicKey": "ed25519:<a former key>",
      "next": "sha256:<the new key's fingerprint>",
      "signature": "<that former key's signature>"
    }
  ]
}
```

`previousKeys` holds one such entry for each former key. Each former key
signs the UTF-8 bytes of one JSON object with no whitespace,
`{"format":"suprnova-key-handover/1","library":"<source>","next":"<fingerprint>"}`,
where `library` is the `source` in this `library.json` and `next` is the new
key's fingerprint. The statement names the library, so a former key that
signs for several libraries hands over only this one. `signature` holds the
signature in standard padded base64. The format is open, so other tooling
can produce a statement too.

An application that pinned a former key installs the new release only if
the statement from its pinned key verifies, and it re-pins only when its
developer confirms the change on a terminal. The application keeps the
former key in its pin table, so the components it installed under that key
still verify.

A developer whose pinned key you dropped pins the new key by hand: in the
library's table of `suprnova.toml`, set `key` to the new key and move the
dropped key into `previous_keys`, so the components installed under it still
verify:

```toml
[live.libraries."github.com/acme/acme-ui"]
namespace = "acme"
key = "ed25519:<the new key>"
previous_keys = ["ed25519:<the dropped key>"]
```

## What a pinned key proves

The first time an application installs from your library, its developer
pins the key `library.json` names. A pinned key proves continuity: every
later version that application installs comes from whoever held the key at
that first install, or from whoever that holder handed it to through
`previousKeys`. It does not prove who that is. An application trusts your
library because its developer chose to install from your address, and the
pin keeps that choice from silently changing hands.

## What the scan admits

`live:add` scans every component from any library but the shipped one, and
`live:registry check` runs the same scan. The scan reads each file's syntax
tree, compiles and runs none of it, and admits only what it can classify.
A construct it cannot classify, or a name it cannot resolve, is refused,
and no finding can be overridden. A refusal names the check, the file and
the line. Acceptance proves what a component can reach, not that it
compiles.

In Rust, a component may name its own items, the modules of the components
it depends on, Suprnova's documented public API, including the crates
Suprnova re-exports through the `suprnova::` path, and the part of `std`
that has no effects. The scan refuses:

- Any other path, under any alias, a `#[doc(hidden)]` re-export included.
- `unsafe`, `extern` blocks and functions, and `macro_rules!`.
- Every macro, attribute and derive not on its allowlist.
- `#[path]`, a `mod` declaration without a body, `env!`, `option_env!`,
  `include!`, `include_str!`, `include_bytes!` and `cfg_attr`.
- `TrustedHtml`'s constructors, so a view writes unescaped only markup the
  framework built from typed data.
- A method call whose receiver type it cannot name, and a service resolved
  from the container whose type it cannot name.

Each capability the admitted paths carry goes into the plan. Resolving a
trait your own library defines carries none, because the application binds
it to its own implementation.

In a view, parsed with Askama's parser:

- An expression reads the component's state and calls only admitted paths
  that carry no capability, and the framework's view helpers.
- No Rust macro, no filter that is neither Askama's nor the framework's, and
  no unescaped output but `trusted_html`: no `safe`, and no `escape` or `e`
  with an escaper other than `html`.
- An `include`, `import` or `extends` names only a view of the component,
  of a component it depends on, or of the shipped library.
- No `script`, `iframe`, `object`, `embed`, `base`, `link` or `meta` element,
  no event handler attribute, no `javascript:` or `data:` URL, and no SVG
  `animate`, `set` or `foreignObject`.
- A URL attribute (`href`, `src`, `srcset`, `action`, `formaction`,
  `poster`, `data`, `xlink:href`, `ping`) holds a constant on the
  application's origin, or a value the application passes in.
- CSS, in a stylesheet or a `style` element or attribute, has no `@import`,
  and every resource it names stays on the application's origin.

In a script, parsed as a JavaScript module:

- Every call resolves to a function the script defines or a standard browser
  API, including calls made through `Reflect`, `call`, `apply`, `bind`,
  getters, setters, `Proxy` traps and tagged templates.
- No `eval` or `Function` under any spelling, no timer given a string, no
  dynamic `import`, no static `import` from outside the component and its
  dependencies, no `Worker`, and no `document.write` or `document.open`.
- No `script`, `iframe`, `object` or `embed` element created, and no element
  created by a name the scan cannot trace to a constant.
- No HTML parsed into the document (`innerHTML`, `outerHTML`,
  `insertAdjacentHTML`, `setHTMLUnsafe`, `createContextualFragment`,
  `DOMParser`), no `attachShadow`, and no change to a built-in prototype.
- An attribute name given to `setAttribute` traces to constants and is not an
  event handler, `srcdoc` or `style`.
- A URL given to `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`,
  `sendBeacon`, `location`, `window.open`, or a URL property or attribute is
  a constant on the application's origin.
- `customElements.define` takes a constant name the manifest's `elements`
  declares.

A component reaches the database, the network, files, mail, a queue, a cache,
the session, the environment or a process only through Suprnova's API, and
each of those carries a capability the installing developer approves.
`live:add` never adds a crate to the application, so nothing enters its
build that the scan did not read.

### Why Suprnova diverges

Laravel packages arrive through Composer and run as soon as the application
boots: a package's service provider is code the application runs, chosen by
the package. shadcn copies a component's source into a project from a
registry, with no signature and no scan. A Suprnova library sits between the two. Like
shadcn, `live:add` copies the component into the application, which owns it
from then on. Unlike either, every component is signed by its library's key,
pinned on first use, and scanned as data before anything is written, and the
developer approves each capability by name. The library declares no
dependencies of its own, so the scan reads everything the component adds to
the build.

## Next

- [Installing Live Components](live-add.md) - the plan, capabilities, pins,
  and the provenance record an install writes
- [Live](live.md) - components, views, documents and islands
- [CLI Overview](cli.md) - every `live:*` command
- [suprnova serve](cli-serve.md) - the dev runner the preview uses
