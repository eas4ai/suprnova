# Live library schema 1

Prefix: LCS

Status: Agreed 2026-10-01

This detail is incorporated by LCT-002 through LCT-005. It adopts the package,
closed schema, and diagnostic rules from revision 2 of the SDK's framework
design. The source document's Git blob is
`ffc5fb1e2bedb41b9f05283771ef7d49dd39fc0b`.
It does not adopt the runtime evaluator, installer, or reload implementation.

## Package contract, schema 1

A release is an unpacked directory containing `library.json` and component
directories. The first release accepts local directories only. Archive extraction,
remote fetching, registries, publisher accounts, and signing are outside scope.
Authors may distribute the directory through their normal source release process.

`library.json` has exactly these fields; all are required:

```json
{
  "schema": 1,
  "name": "acme-ui",
  "version": "1.0.0",
  "license": "MIT",
  "compatibility": {
    "suprnova": ">=0.1.0, <0.2.0",
    "template_profile": 1
  },
  "components": [
    {
      "name": "acme-ui.badge",
      "manifest": "components/badge/manifest.json",
      "files": [
        { "path": "badge.html", "role": "template", "sha256": "664b3a03d897c45aea653fa10f4cce64332188cccd9bd848b0d8ea3e3208a557" },
        { "path": "badge.css", "role": "stylesheet", "sha256": "ccbe80977b7174ff27f3cf251a85061f38544666e07eef5736e041eed7fa8596" }
      ],
      "views": [
        {
          "template": "badge.html",
          "entry": "badge",
          "context": { "kind": "record", "fields": { "label": { "kind": "string" } } }
        }
      ],
      "dependencies": [],
      "rust": null
    }
  ]
}
```

The example version range is illustrative, not a claim about a supported release.
Executable fixtures use the framework version actually qualified by their tests.
The example hashes correspond to these exact UTF-8 payloads, each with one final
LF newline:

```html
{% macro badge(label) %}<span class="acme-badge">{{ label }}</span>{% endmacro %}
```

```css
@layer acme-ui { .acme-badge { display: inline-block; } }
```

Rules:

1. `schema` is the integer 1. Unknown schema versions, duplicate JSON keys, and
   unknown fields are errors. JSON is UTF-8.
2. `name` is the library's local namespace: 1–48 ASCII lowercase letters, digits,
   and hyphens, starting with a letter and ending with a letter or digit.
   `suprnova`, `suprnova-ui`, and names starting `suprnova-` are reserved.
   Names convey no verified publisher identity.
3. `version` is a SemVer release version without build metadata. The compatibility
   range uses Cargo version-requirement syntax, must include an upper bound, and
   must match the consumer's framework version. Prereleases require an explicit
   matching prerelease requirement. `license` is a nonempty SPDX expression;
   identifiers and expressions are validated against a versioned SPDX list.
   `template_profile` is the exact supported integer profile, initially 1.
4. Each component is `<library>.<slug>`, with a slug following the namespace
   grammar. Its manifest path is exactly `components/<slug>/manifest.json`.
5. That component manifest retains the existing format: `name`, integer
   `version: 1`, `root`, `files`, and `elements`. Its name must match the catalog;
   root must equal `<library>/<slug>`. Manifest `version` is a schema version,
   not the library release version. This new workflow requires all five fields
   and rejects other fields and duplicate keys.
6. Each catalog file is a direct child of the component directory. File names
   are 1–128 ASCII lowercase letters, digits, hyphens, and dots, start with a
   letter, and contain no consecutive dots. Roles are `template` (`.html`),
   `stylesheet` (`.css`), and `script` (`.js`). `manifest.json` is not a payload.
   The catalog and component manifest must list exactly the same payload files,
   once each and in identical order. The catalog is the runtime ordering authority;
   differing orders are rejected. A component has at least one template.
   Payloads must be UTF-8.
7. Every payload has a SHA256 digest of its exact bytes. Changes in whitespace
   count as changes. The package digest also binds both levels of metadata:
   hash each manifest and payload, then SHA256 the lexically sorted UTF-8 list
   of `relative-path`, a tab, lowercase file digest, and a newline. Include
   `library.json` itself; store the resulting package digest outside the package.
   Paths cannot contain tabs or newlines. The contract crate owns this algorithm.
8. `dependencies` names other components in the same library, without version
   ranges. They must exist; cycles and duplicates are errors. Cross-library
   dependency resolution is deferred. Selection installs the transitive closure.
9. `rust` is null or an object with exactly `package`, `version`, and `type`.
   These name a Cargo package, exact SemVer version, and its fully qualified
   exported Rust type, for example `acme_live::Counter`. Compilation later checks
   that the type implements the public component contract and its registered
   identity matches this entry. No source is executed to inspect these fields.
10. Element tags listed by a component must start with `<library>-`, meet the
    HTML custom-element name grammar, and be unique throughout the library and
    enabled application catalog. `sn-` remains reserved. A nonempty `elements`
    list requires a script. Declarations do not prove what arbitrary JS defines.
11. No undeclared files or directories are allowed in a distributable package.
    Library documentation, Rust sources, tests, and author tooling remain in
    the source repository or separate Cargo package. The SDK exports only the
    catalog, component manifests, and declared payloads.
12. `views` is a nonempty list. Each entry has exactly `template`, `entry`, and
    `context`. `template` names a declared template file; `entry` is null for a
    whole-file entry or the name of an exported macro. Duplicate file/entry pairs
    are errors. Every externally callable entry is declared. Private includes
    and macros are checked as part of an entry's transitive dependency graph.
    `context` follows the view-data schema below, with a record at its root.

Default hard limits: 1 MiB per manifest or payload, 64 MiB total package bytes,
256 components, 64 payloads and 32 element tags per component, 32 dependencies
per component, and JSON nesting depth 32. Checks reject over-limit input while
reading, before allocating its declared size. These are release-contract defaults;
changing them requires a documented contract version change.

## Declared view-data schema

Schema nodes are closed objects: primitive `{ "kind": "string" }`, `boolean`,
`integer` (signed 64-bit), or `number` (finite JSON number); a record has `kind`
and a `fields` map; a list has `kind` and an `item` schema; a nullable node has
`kind` and an `item` schema. All record fields are required; nullable values may
be null, but cannot be absent. Extra fields, duplicate keys, unknown kinds, and
wrong value types fail. U64 identifiers outside the signed range use strings.
Schemas share the manifest depth/size limits. No reference cycles, Rust type
names, executable expressions, or external schema references are allowed.

## Diagnostic reports

Diagnostics have stable codes, severity, relative path, JSON pointer or source
location when available, and remediation text. JSON output includes validator
version, package digest when available, checked stages, and skipped stages.
Malformed packages and failed compatibility checks return nonzero. Integration
qualification requires complete requested-entry coverage; zero checked entries
cannot pass. Presentational-only libraries qualify through test pages without
behavioral registration. Selected Rust-backed components require registration.
Terminal output escapes control characters supplied by package metadata.

The structural validator names runtime-template, binary, installation, and
rendering checks as skipped when it has not executed them. Later qualification
must supply their evidence; parsing a profile identifier is not qualification.
