# Review - tooltip-dismissal

commitment: tooltip-dismissal
commit: 3ab4f8ec
examined:
  - The mechanism `ui-overlays` against OVL-002's revised text (LOOP-059): its tooltip checks in `.cairn/tools/ui-overlays.mjs`, the tooltip view and stylesheet they read, and the overlay gallery they scan for `aria-describedby`.
findings:
  - resolved: The check refused any `tooltip.js` outright, which the revised text admits. It now asks that a shipped script define the `sn-tooltip` element, and the revision and the check were committed together at `3ab4f8ec`, so no evidence was ever recorded against the old check for the new text.
  - resolved: The static checks cannot observe the revised falsifier's third clause, that the bubble stays hidden in a document the enhancement never reaches. The new mechanism `ui-tooltip-dismissal` speaks for OVL-002 as well, and its script-absent browser case reads the bubble's visibility on hover and on focus in each qualified engine; the two mechanisms together cover the requirement, and that division is recorded in the requirement's Mechanism line.
  - resolved: The gallery half of the check, that every `tooltip::tooltip` call is referenced by `aria-describedby`, is unchanged by the revision and still matches the agreed text.

## Mechanism review, 2026-09-17

No code changed during this review. The checks read the shipped component
directory and the dogfood gallery only, so a passing result cannot come
from a stale build. The browser half is declared where a browser can see
it, and the static half stays where a grep can.
