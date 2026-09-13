# Library components are headless by construction: no inline styles, tokens only, skin removable

Level: Consequential
Decided by: Shawn
Rests on: UI-004, UI-005, UI-006
Would be wrong if: a theme cannot change a component's radius, font, or color by editing tokens alone, or removing the base layer breaks a behavior, an accessible name, or a state attribute

## Decision

Ruled 2026-09-13 10:36-10:37: a component as built is structure, behavior, and state; every visual value it has comes from a --sn- token, never a literal in its own CSS and never a style attribute; structural rules may be literal. The skin ships on so a scaffolded application looks right on first run, and is removable with nothing breaking, which is the property the mechanism proves. The developer's words: NO INLINE STYLES - TOKENS ONLY. Rejected: skin off by default (a bare first run; the removability property is what matters and it holds either way).

## Realized by

- f241aba8  docs(cairn,live): apply the owner's component-library rulings to the specs
