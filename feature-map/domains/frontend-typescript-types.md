# Feature map: `manual/frontend-typescript-types.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 2 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova generate-types` · suprnova-cli/src/main.rs:140
  - Generate TypeScript types from Rust InertiaProps structs
  - option `-o, --output <OUTPUT>`: Output file path (default: frontend/src/types/inertia-props.ts)
  - option `-w, --watch`: Watch for changes and regenerate

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::InertiaProps` · suprnova-macros/src/lib.rs:130 (re-exported as `suprnova::InertiaProps`)
  - Form: derive `#[derive(InertiaProps)]`
