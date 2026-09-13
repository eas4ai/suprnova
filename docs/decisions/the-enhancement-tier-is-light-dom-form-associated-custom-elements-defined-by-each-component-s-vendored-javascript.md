# The enhancement tier is light-DOM, form-associated custom elements defined by each component's vendored JavaScript

Level: Consequential
Decided by: Shawn
Rests on: UI-010, UI-011, UI-018
Would be wrong if: a shipped element carries a value the surrounding form cannot submit, a fetched document lacks content a component displays, or a document that never added a component defines its tag

## Decision

Option (b') of three, chosen 2026-09-13 10:36. The three components the platform cannot give natively (combobox, one-time code, date strips) ship as light-DOM custom elements with sn- tags, form-associated through ElementInternals, each defined by its own component's vendored JavaScript on a small reviewed helper artifact; Live local primitives stay the first choice and Stimulus stays an application-supplied role. Live spec 20's 2026-08-21 text named local primitives or Stimulus controllers as the only JavaScript paths; the 2026-09-13 revision entry supersedes it. Fallback recorded: hand-written HTMLElement subclasses if the helper misfits the first widget. Rejected: Stimulus for library behavior (an app-supplied dependency for the library's most valuable component; not form-associated).

## Realized by

- f241aba8  docs(cairn,live): apply the owner's component-library rulings to the specs
