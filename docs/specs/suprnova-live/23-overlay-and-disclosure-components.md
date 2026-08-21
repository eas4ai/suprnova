# Suprnova Live -- 23 Overlay and Disclosure Components

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns official collapsible/disclosure, accordion, menu, popover,
tooltip, hover-card, dialog, drawer/sheet, context-menu, and related layered
interaction components. It depends on local reactivity, Stimulus integration,
morph identity, navigation, and component foundations. It owns focus/layering
presentation contracts, not application action authority.

## Capabilities

### Disclosure and accordion

Disclosure and accordion components shall reveal server-rendered or lazily
declared content using local behavior by default. Expanded state, control/panel
association, keyboard behavior, and morph lifetime shall be explicit.

Acceptance criteria:
- Controls use native button semantics with `aria-expanded` and panel linkage.
- Single and multiple expansion modes are explicit.
- Local expand/collapse causes no network request unless content itself requires
  declared server completion.
- Accordion keyboard behavior follows the selected accessible pattern.
- Hidden content, focusability, animation, reduced motion, and print behavior are
  defined.
- Stable keyed scope preserves expansion across compatible morphs.

UX flow:
1. Application user activates a disclosure -> content appears immediately with
   coherent semantic state.
2. Server morph retains/removes the disclosure -> expansion survives only with
   the same keyed ownership.

### Menus and action lists

Dropdown, application menu, context menu, and action-list components shall
present registered actions or real navigation choices with appropriate menu or
ordinary-list semantics. Styling shall not force menu roles onto simple link
lists.

Acceptance criteria:
- API distinguishes navigation list, action list, and application menu patterns.
- Menu keyboard navigation, typeahead where offered, submenu behavior, focus,
  escape, outside dismissal, and disabled items are complete.
- Navigation items remain real anchors; action items are buttons/registered Live
  actions.
- Destructive actions are labeled and confirmed according to policy.
- Context-menu alternatives remain available to keyboard/touch users.
- Repositioning or morphing does not transfer active state to another keyed item.

UX flow:
1. Application user opens a menu -> focus and active item enter the declared
   pattern.
2. Application user selects/dismisses -> action/navigation runs once or focus returns to the
   invoker.

### Popovers, tooltips, and hover cards

Anchored overlays shall position supplementary or interactive content relative
to a trigger while preserving accessible names/descriptions and reliable
keyboard/touch behavior. Tooltip content shall never be the only source of
essential information or interaction.

Acceptance criteria:
- Tooltip, non-modal popover, and hover-card semantics are distinct.
- Trigger/overlay association, placement, collision handling, viewport bounds,
  and fallback position are deterministic.
- Tooltips respond to focus and hover with bounded delays and are dismissible.
- Interactive content is not placed inside tooltip-only semantics.
- Touch has an explicit accessible activation path rather than hover dependence.
- Live morph or scroll/resize repositions or closes an invalid anchor safely.

UX flow:
1. Application user focuses/activates a trigger -> appropriate supplementary
   content appears and remains reachable.
2. Trigger disappears or user dismisses -> overlay closes and focus/state
   recover coherently.

### Modal dialogs

Dialog and alert-dialog components shall establish a modal interaction boundary
with correct labeling, initial focus, focus containment, background inertness,
escape/close policy, action state, and focus return.

Acceptance criteria:
- Native dialog capabilities are used where they meet the supported contract or
  are wrapped by a tested equivalent.
- Title/description and alert-dialog consequence are programmatically exposed.
- Initial focus is deliberate; destructive confirmation does not default focus
  unsafely.
- Background content is inert and excluded from assistive navigation while
  modal.
- Escape, backdrop, close button, submit success, validation failure, and
  navigation have explicit close rules.
- Focus returns to the invoker or a safe fallback when the invoker disappeared.

UX flow:
1. Application user opens a dialog -> it becomes the active focus/context and
   background interaction stops.
2. Application user validates, submits, cancels, or navigates -> dialog remains or closes
   according to outcome and restores focus truthfully.

### Drawers and sheets

Drawer/sheet components shall provide modal or non-modal edge panels with
explicit mode, responsive behavior, focus, scroll, gesture, and navigation
semantics. They shall not disguise a route change as persistent client layout.

Acceptance criteria:
- Modal and complementary non-modal modes use appropriate semantics.
- Responsive conversion between drawer and inline/sidebar layout preserves
  content order and accessible control state.
- Swipe/drag, if offered, has keyboard/button equivalents and does not conflict
  with browser navigation gestures.
- Body/region scroll lock and restoration are bounded and nested safely.
- Route links inside navigate normally; pending action/dirty handling follows
  navigation policy.
- Reduced motion and interrupted transition reach the correct final state.

UX flow:
1. Application user opens a drawer -> local overlay state appears with declared
   focus and scroll behavior.
2. Viewport changes or destination is selected -> it converts/closes/navigates
   without losing semantic ownership.

### Layer, portal, and stacking management

Layered components shall share a framework-owned document layer system that
coordinates stacking, teleport targets, nested overlays, outside interaction,
focus, inertness, and cleanup without arbitrary global z-index escalation.

Acceptance criteria:
- Layer roots and teleport targets are declared, unique, and compatible with
  Live island ownership.
- Nested overlay stack defines which layer handles escape/outside interaction.
- Stacking tokens avoid application-wide unbounded z-index competition.
- Scroll lock, inertness, and focus guards use reference-counted/nested-safe
  cleanup.
- Server morph cannot orphan a teleported overlay or duplicate its controller.
- Document navigation disposes the complete layer stack.

UX flow:
1. Application opens nested overlays -> topmost valid layer receives interaction
   while parent state remains coherent.
2. Layers close or owner disappears -> stack, inertness, scroll, focus, and
   controllers unwind exactly once.

## Acceptance criteria

- Disclosure components remain local and accessible unless server work is
  explicitly required.
- Menus distinguish actions, navigation, and ordinary link lists.
- Anchored overlays work across keyboard, pointer, touch, resize, and morphing.
- Dialogs/drawers preserve focus, inertness, scroll, and truthful action state.
- Layer management prevents orphaned overlays and global stacking conflicts.

## Decisions and revisions

- 2026-08-21 -- Overlay state is browser-local by default; server actions own
  only authoritative effects invoked from overlays.
- 2026-08-21 -- Layering and teleportation preserve Live island ownership rather
  than creating an unrelated global client root.
