# Suprnova Live -- 21 Form and Input Components

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the official component library's form structure, labels/help/
errors, buttons, text and selection controls, toggles, advanced choices,
date/time inputs, and upload presentation. It depends on library foundations,
state binding, actions/validation, uploads, scheduling feedback, and morphing.
It does not own application validation rules or file-transfer mechanics.

## Capabilities

### Form and field composition

The library shall provide semantic form, field, fieldset, legend, label,
description, error, and action-group patterns that connect native HTML and Live
binding without hiding ownership or submission behavior.

Acceptance criteria:
- Every control has a programmatic label and stable error/help associations.
- Required, optional, readonly, disabled, invalid, dirty, loading, and success
  states are visually and semantically distinct.
- Field names, IDs, model paths, and error paths compose without collisions in
  repeated/nested components.
- Native form submission remains available for ordinary Suprnova forms; Live
  submission is explicit through its directive.
- Fieldset/group semantics cover related controls.
- Summary errors can link/focus affected fields without duplicating messages
  excessively.

UX flow:
1. Application user enters a form -> labels, help, grouping, and required state
   explain expected input.
2. Validation fails -> field and summary feedback identify and focus actionable
   corrections while preserving entered values.

### Buttons and action controls

Button, submit, reset, link-button presentation, icon-button, split-action, and
destructive-action patterns shall preserve native element semantics and Live
feedback targeting.

Acceptance criteria:
- Buttons use native `<button>` with explicit type; navigation uses anchors
  unless an action genuinely requires a button.
- Icon-only controls require accessible names.
- Loading/busy state prevents duplicate activation only for the intended target
  and does not erase the control's label.
- Destructive variants communicate consequence and integrate with explicit
  confirmation patterns where required.
- Disabled versus permission-hidden behavior remains distinct.
- Keyboard, touch target, focus-visible, and high-contrast behavior meet the
  foundation contract.

UX flow:
1. Application user invokes an action control -> target-scoped queued/loading
   feedback appears and repeated unsafe activation is prevented.
2. Action settles -> control restores or transitions to declared success/error
   state without false completion.

### Textual and numeric inputs

The library shall cover text, search, email, URL, telephone, password, number,
range, textarea, masked/prefix/suffix, and one-time-code patterns using native
controls where possible and typed Live binding. Passwords, one-time codes, and
equivalent request-only secrets shall use transient model binding by default.

Acceptance criteria:
- Appropriate input type, autocomplete, inputmode, spellcheck, min/max/step,
  maxlength, and privacy defaults are documented.
- Prefixes/suffixes do not become part of submitted value unless declared.
- Password and secret controls never dehydrate values into instanced or public
  seed snapshots and never expose them through diagnostics, events, effects, or
  generated markup; compatible morphing preserves only the browser-local
  control value.
- IME composition, selection, autofill, paste, and browser correction survive
  compatible morphs.
- Search and immediate inputs use bounded debounce/throttle defaults.
- Clear/reveal/copy affordances remain keyboard and assistive-technology usable.

UX flow:
1. Application user edits a value -> native input behavior remains immediate and
   model timing follows the declared binding.
2. Server returns validation or formatted authoritative value -> morph preserves
   compatible active editing or applies an explicit correction.

### Choice and toggle controls

Checkbox, radio group, switch, native select, multi-select, segmented control,
and toggle-group components shall map accessible selected/checked state to typed
component values.

Acceptance criteria:
- Binary, optional, enum, and collection mappings are explicit.
- Radio/toggle groups have accessible group labels and arrow-key behavior where
  required by their pattern.
- Switch uses checked state for a real binary setting and does not substitute
  for an unrelated action button.
- Disabled options and group-level disabled state retain semantic meaning.
- Reordered options use stable values/keys and do not move selection to another
  logical item.
- Empty and loading option states remain usable.

UX flow:
1. Application user selects or toggles an option -> local control state updates
   and Live synchronizes the typed value under policy.
2. Options change after a server action -> valid selections persist; removed or
   forbidden values receive explicit correction/validation.

### Combobox and autocomplete

Combobox, autocomplete, tag/token input, and searchable selection components
shall implement established keyboard/focus semantics while supporting local or
server-backed filtering, pagination, and asynchronous results.

Acceptance criteria:
- Input, popup, listbox, option identity, active descendant, selection, and
  expanded state follow the chosen accessible pattern.
- Local filtering makes no server request; server filtering uses bounded model/
  action scheduling and announces loading/empty/error states.
- Stale result responses cannot replace results for a newer query.
- Virtualization, when offered, preserves accessible option count/position and
  selected identity.
- Multi-value tokens support keyboard navigation/removal and clear labels.
- Free-form versus constrained selection is explicit.

UX flow:
1. Application user types or opens choices -> component exposes current local or
   server-backed options with truthful loading state.
2. Application user selects, clears, or receives no results -> typed model and accessible
   feedback update without focus loss.

### Date and time controls

Date, time, datetime, range, and calendar-enhanced controls shall prefer native
semantics where adequate and define locale, timezone, parsing, constraints, and
keyboard behavior where custom presentation is used.

Acceptance criteria:
- Rust value type, browser value format, display locale, and timezone authority
  are explicit.
- Min/max, unavailable dates, range ordering, and validation are server-enforced.
- Custom calendar grids have complete keyboard navigation and announced labels.
- Manual input and picker selection share validation and do not silently change
  timezone/date.
- Current date/time is injectable for deterministic tests.
- Popover/calendar morphs preserve focus and chosen range identity.

UX flow:
1. Application user types or chooses a date/time -> component displays localized
   presentation while synchronizing the canonical typed value.
2. Value is ambiguous or invalid -> feedback explains the expected format or
   constraint without guessing.

### File upload presentation

File input, drop zone, file list, preview, progress, retry, cancel, remove, and
validation components shall present the file-upload domain without inventing a
second transfer or trust model.

Acceptance criteria:
- Native file input remains reachable and labeled even when a drop zone is
  enhanced.
- Drag/drop and paste do not bypass count/type/size policy.
- Per-file and aggregate queued/transferring/verifying/complete/error states are
  visible and announced proportionately.
- Preview URLs are revoked and untrusted media is handled safely.
- Retry/cancel/remove controls act on opaque temporary references only.
- Navigation/replacement warnings reflect active upload truth.

UX flow:
1. Application user chooses or drops files -> pending list and progress expose
   the actual upload lifecycle.
2. File fails, is canceled, or completes -> row offers the correct recovery or
   final pending state without claiming durable save before form action.

## Acceptance criteria

- Form anatomy and every input family preserve native semantics and typed Live
  ownership.
- Validation, dirty/loading/success, and error feedback are accessible and
  morph-safe.
- Advanced choice/date controls implement complete keyboard and focus patterns.
- Upload UI delegates transfer and trust to the upload domain.
- Ordinary and Live forms remain explicitly distinguishable.

## Decisions and revisions

- 2026-08-21 -- Native semantic controls are the baseline; custom widgets must
  justify themselves with complete keyboard, focus, and validation behavior.
- 2026-08-21 -- The component library presents Live state but does not create a
  separate client-side form authority.
- 2026-08-21 -- Official password, one-time-code, and equivalent secret controls
  use transient request-only models and never dehydrate their values.
