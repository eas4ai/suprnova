# Roadmap

Status: Draft. Not normative.

Order lives here. Filenames carry meaning, never sequence.

Current: component-library-foundations

The developer named the work on 2026-09-13 08:52: the Live component
library, as scoped on 2026-09-12 and reconciled against Live specs 20-25.
The order below follows the developer's sequencing rulings: platform-first
components in waves, the advanced components last, the datatable closing.

0. live-and-render-cache-hardening - DONE 2026-09-13 (`fe65703c`, receipts
   `c9471a80`; one clause of LIVE-016 left open for the owner, see the
   review) - the thirteen defects of the
   2026-09-13 adversarial audit, in the audit's order: the four that
   change browser execution or replay bytes past the handler's HTTP
   contract first (CACHE-003, CACHE-004, CACHE-001, CACHE-002), then
   write invalidation (CACHE-009), database routing (CACHE-008), the
   Live authorization and transaction contracts (LIVE-016, LIVE-017),
   then CACHE-006, CACHE-005, LIVE-018, CACHE-010, CACHE-007. The owner
   ruled at 11:13 that this precedes the library.
1. live-session-revocation - DONE 2026-09-14 (`cf7332d1` and `3700c117`,
   receipts on `3700c117`; reopened once for LIVE-021 after the developer's
   `ok` on escalation `live-019`, plain `Auth::logout` keeping the session
   row) - the session and revocation-state clauses
   of LIVE-016 that the hardening left open: a membership whose session
   is destroyed on this node or another stops receiving events
   (LIVE-019, LIVE-020). The developer answered `ok` on escalation
   `live-016` at 21:21 on 2026-09-13: this precedes the library.
2. component-library-foundations - tokens, base layer, state-attribute
   styling, asset delivery, explicit registration and reserved
   namespaces, the checker and baseline mechanisms, and the form
   family's presentational tier (`component-library-foundations.md`,
   `component-library-forms.md`).
3. component-library-overlays - dialog, sheet, drawer,
   single-level dropdown menu, popover, tooltip, tabs, collapsible and
   accordion; opens with the anchor-positioning and popover-continuity
   spike in the qualification host (`component-library-overlays.md`).
4. component-library-feedback-and-navigation - alert, toast and flash
   region, skeleton, progress, spinner, empty state; header bar, footer,
   sidebar, breadcrumbs, pagination, load more (`component-library-feedback.md`,
   `component-library-navigation.md`).
5. component-library-data-display - card, image and aspect ratio, scroll
   area, separator, badge, avatar, list group, description list, stat
   card; the datatable last (`component-library-data-display.md`).
6. component-library-live-native - upload widget, live feed, notification
   bell, account menu, server-rendered chart, input OTP, date picker,
   combobox; the custom-element enhancement layer lands here
   (`component-library-forms.md` custom-element tier, `-feedback.md`,
   `-navigation.md`, `-data-display.md` behavioral tiers).

Out of every commitment above, by the developer's 2026-09-12 ruling:
complete blocks (auth flows, account settings, CTAs, pricing, dashboard
layouts), tag input, nested menus, command palette, drag-reorder,
auto-infinite-scroll, carousel, image cropper, rich text editor, rating
input. They are a separate project and enter this roadmap only if the
developer writes them in.

Every commitment inherits the Live gate and the repository gate; a
commitment is Done only when its mechanisms pass on a committed tree and
the review under `.cairn/reviews/` is recorded.
