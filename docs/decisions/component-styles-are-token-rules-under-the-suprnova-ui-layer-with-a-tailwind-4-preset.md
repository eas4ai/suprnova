# Component styles are token rules under the suprnova-ui layer with a Tailwind 4 preset

Level: Consequential
Decided by: Shawn
Rests on: UI-001, UI-002, UI-004, UI-007
Would be wrong if: an application without Tailwind cannot use a library component with its shipped look, or a Tailwind application cannot restyle a component with utilities because the library's rules are unlayered

## Decision

Option (c) of three, chosen 2026-09-13 10:22. Component styles ship as token-driven rules inside the suprnova-ui cascade layer, so unlayered application CSS wins by cascade order; a documented Tailwind CSS 4 theme preset maps Tailwind's namespaces to the --sn- tokens so a Tailwind application feels first-class without utilities in library markup. Live spec 20's 2026-08-21 decision named Tailwind utilities; the 2026-09-13 revision entry in that spec supersedes it. Rejected: utilities in markup (every library user must run a Tailwind build; heavier morphs; a less readable fetched document) and tokens with no Tailwind story (spec 20's intent lost).

## Realized by

- f241aba8  docs(cairn,live): apply the owner's component-library rulings to the specs
- 781b63c1  feat(live): ship the suprnova-ui token stylesheet, base layer and Tailwind preset (UI-001)
- 43f70aa0  feat(live): deliver the suprnova-ui base as the ui-styles runtime artifact (UI-008, UI-019)
