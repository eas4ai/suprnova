# Component library - form and input

Status: Draft
Prefix: FORM

Refines Live spec 21
(`crates/suprnova-live/docs/specs/suprnova-live/21-form-and-input-components.md`):
form and field composition, buttons and action controls, textual and
numeric inputs, choice and toggle controls, combobox and autocomplete,
date and time controls, file upload presentation. Inherits every Agreed
block of `component-library-foundations.md`.

Components in complexity order. Presentational: field (label, control,
hint, error), label, input, textarea, number input, slider, search input,
password input with reveal, checkbox and checkbox group, radio group,
switch, select, button and link-button, button group, fieldset, form
actions bar, validation summary, file input. Behavioral: form (the
composed whole with authorize, validate, transaction), upload widget.
Custom-element enhancements: input OTP, date picker, combobox.

## Presentational tier

[FORM-001] The library MUST ship the presentational components listed
above, each built on the native control and the `live:model`,
`live:error`, and `live:loading` vocabulary.
Falsifier: one listed component is missing from the shipped set, or one
replaces a native control with a scripted stand-in.
Mechanism: `.cairn/mechanisms/ui-live-check` over a dogfood view that
mounts every listed component.

[FORM-002] Every control the library ships MUST have a programmatic label
and stable error and help associations.
Falsifier: the checker reports a shipped control without an accessible
name or with an unassociated error element.
Mechanism: `.cairn/mechanisms/ui-live-check`.

[FORM-003] A search or immediate input MUST use a bounded `live:model`
debounce or throttle default.
Falsifier: a shipped search input proposes on every keystroke with no
timing modifier.
Mechanism: `.cairn/mechanisms/ui-live-check` (the checker proves the
directive's timing form) and a grep over the shipped views.

[FORM-004] Password and one-time-code controls MUST bind through transient
model fields. The library MUST NOT dehydrate a password or one-time-code
value into a snapshot.
Falsifier: a rendered snapshot contains a password or one-time-code value.
Mechanism: the framework's Live snapshot tests
(`framework/tests/live/document_routes.rs`) extended with a library case.
Reading: Live spec 21, Decisions and revisions, 2026-08-21.

## Behavioral tier

[FORM-005] The upload widget MUST present the upload domain's states
(queued, transferring, verifying, complete, error) through the shipped
upload protocol. The upload widget MUST NOT introduce a second transfer
or trust model.
Falsifier: the widget performs a transfer outside the `__live/upload`
endpoint, or claims durable save before the finalizing action runs.
Mechanism: `app/tests/live_uploads.rs`-style end-to-end test through
`handle_request`, plus the browserless harness.

## Custom-element tier

[FORM-006] The input OTP MUST accept the code as one native input with
per-character presentation. The per-cell auto-advance MUST be an
enhancement the control works without.
Falsifier: with the element helper absent, the one-time code cannot be
entered and submitted.
Mechanism: one browserless harness case with the helper disabled.

[FORM-007] The date picker MUST accept only a date in its input. Its
year, month, and day strips MUST be native radio groups inside CSS
scroll-snap containers, so tap, click, and arrow keys work with no script.
Falsifier: a strip selection requires script, or the input accepts a
non-date value.
Mechanism: one browserless harness case with the helper disabled and one
Playwright case per qualified engine.
Ruling pending: Live spec 21 prefers native date semantics where adequate;
the developer's 2026-09-12 design is this horizontal strip. A dated
revision in spec 21 precedes agreement.

[FORM-008] The combobox MUST implement the accessible combobox pattern
(input, popup, listbox, option identity, active descendant, expanded
state) as a custom-element enhancement over a native input and list. The
combobox MUST NOT let a stale result replace results for a newer query.
Falsifier: an older response's options render after a newer query's, or
the popup lacks listbox semantics.
Mechanism: browserless harness cases for the keyboard map and stale
suppression; Playwright per engine.
Ruling pending: Live spec 21 includes tag/token input in this capability;
the developer ruled tag input out of the built-in set on 2026-09-12.
