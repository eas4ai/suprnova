# Roadmap

Status: Draft. Not normative.

Order lives here. Filenames carry meaning, never sequence.

Current: tooltip-dismissal

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
2. live-issuance-credentials - DONE 2026-09-14 (`6546bb31` and `a62136ab`,
   receipts on `a62136ab`; widened once for LIVE-023 after the developer's
   `ok` on escalation `live-022`, the issuance context race the baseline
   exposed) - the backlog observation from the LIVE-018 probe: concurrent
   issuances of one scope in the same millisecond mint identical
   descriptors and the later one's credential replaced the earlier one's,
   so 39 of 512 admitted requests answered 403 (LIVE-022). The developer
   named it the next commitment at 09:05 on 2026-09-14, ahead of the
   library.
3. component-library-foundations - DONE 2026-09-14 (built between
   `781b63c1` and `44e7bed7`; receipts on the closing tree; resumed after
   the developer's `ok` on escalation `loop-035`, keeping the v2.0.2
   release history as the base) - tokens, base layer, state-attribute
   styling, asset delivery, explicit registration and reserved
   namespaces, the checker and baseline mechanisms, and the form
   family's presentational tier (`component-library-foundations.md`,
   `component-library-forms.md`).
4. component-library-overlays - DONE 2026-09-14 (built between
   `e692a480` and `1efccb1d` after the developer's `ok` on escalation
   `ovl-001`, which agreed OVL-001 to OVL-006 and raised the browser
   baseline to the `popover` floor; receipts and review on `0ed49968`) -
   dialog, sheet, drawer,
   single-level dropdown menu, popover, tooltip, collapsible and
   accordion; opened with the anchor-positioning and popover-continuity
   spike in the qualification host (`component-library-overlays.md`).
   Tabs, listed here at first, was never in OVL-001 to OVL-006 and moves
   to item 5, where NAV-002 specifies it.
5. component-library-feedback-and-navigation - DONE 2026-09-15 (built
   between `7aef377a` and `cdc648b5` after the developer's `ok` on
   escalation `fdb-001`, which agreed FDB-001 to FDB-004, FDB-006,
   NAV-001 to NAV-004 and NAV-006; receipts and review on `344ff655`) -
   alert, toast and flash region, skeleton, progress, spinner, empty
   state; header bar, footer, sidebar, breadcrumbs, tabs, pagination,
   load more; the `live_key` view filter landed with them
   (`component-library-feedback.md`, `component-library-navigation.md`).
6. component-library-data-display - DONE 2026-09-15 (built between
   `2d9bec74` and `41e6a04b`; DATA-001 to DATA-005 agreed on escalation
   `data-001` under the developer's 2026-09-14 ruling that every
   remaining commitment is agreed; receipts and review on `cea49611`) -
   card, image and aspect ratio, scroll area, separator, badge, avatar,
   list group, description list, stat card; the chart through
   `charts-rs`; the datatable last (`component-library-data-display.md`).
7. component-library-live-native - DONE 2026-09-15 (built between
   `9c9883c0` and `dfeb91bb`; FORM-005 to FORM-008, FDB-005 and NAV-005
   agreed on escalation `form-005` under the developer's 2026-09-14 ruling;
   the Live gate failed four times and each cause was fixed on the tree,
   `e1a3492a`, `824c3919`, `3478e659` with `14efccaa`, and `2dc59f5a`;
   the review's two open findings were routed on escalation `form-008`;
   receipts `e81fce64`, review `9300e61a`) - upload widget, live feed,
   notification bell, account menu, input OTP, date picker, combobox; the
   custom-element enhancement layer landed here (`component-library-forms.md`
   custom-element tier, `-feedback.md`, `-navigation.md` behavioral tiers);
   the server-rendered chart had landed under DATA-004 in item 6.
8. session-request-serialization - DONE 2026-09-15 (built at `36287efb`
   after the race was pinned at `1a6c1dda`; the combined check after the
   kernel change exposed a webkit race in the input OTP's mirror and a
   PKG-007 lint on LIVE-020, fixed at `fa3e572e` and `ee60c838`; receipts
   `04be1af0`, `e5607e60`, `5c561a51`, review `7103d526`) - session
   blocking through the cache lock driver, the session write race the
   live-native review recorded as an open finding and the developer routed
   here on escalation `form-008` (`sessions.md`,
   `docs/commitments/session-request-serialization.md`); `SessionBlock`,
   `SESSION_BLOCK`, `block_session` on every route and group builder, the
   dogfood application blocking on every route.
9. live-key-vocabulary - DONE 2026-09-16 (built at `159dd219`, the
   dogfood needles and one unformatted file fixed at `3fc2a965`
   after the first stale check; receipts `0d0d3ce8`, review
   `9c4ffc68`) - the runtime reads `live:key`, the attribute the checker
   validates and the manual names, for morph identity, controls and
   preservation scopes, and the library writes the key once; promoted
   from the backlog on escalation `ovl-006` (`live.md` LIVE-024,
   `docs/commitments/live-key-vocabulary.md`). The form gallery's
   `.prevent` rode along, and its first browser case found a runtime
   defect that refused any Live submit from a form holding an empty
   number input or an unselected select, fixed in the same build.
10. checker-proves-runtime-accepts - DONE 2026-09-16 (built at
   `f7ca27ca`; the first Live gate found a session lock held past an
   abandoned request, kept on escalation `loop-035-sess-001` at `0e51a13e`,
   and a save form over the eight-proposal cap, fixed at `e88a9596`;
   receipts `fc6946b5`, review `75e095a2`) - `live:check` proves a component only
   after checking every element its view renders, a declared debounce is
   one the grammar and the runtime accept, and error feedback may target
   an action; promoted on escalation `live-024` and widened on `live-008`
   after the build found an empty call block hiding 24 errors in the form
   gallery (`live.md` LIVE-025 to LIVE-027,
   `docs/commitments/checker-proves-runtime-accepts.md`).
11. live-protocol-bounds - DONE 2026-09-16 (built at `dc031353`, receipts
   `7b4918d7`, review `46bbc3da`; the browser caps turned out to be
   unspecified and below the server's, so the browser was aligned to 128)
   - the browser admits the message counts the
   framework's protocol limits admit, `live:check` refuses a submit form
   one request cannot carry, and a request refused for size fails visibly;
   promoted on escalation `live-027` after the dogfood save form's ten
   fields never submitted (`live.md` LIVE-028 to LIVE-030,
   `docs/commitments/live-protocol-bounds.md`).

12. live-library-review-remediation - DONE 2026-09-17 (built between
   `5fc3d651` and `c7c12fdd`, receipts `92e2703b`, review `53309c54`; an
   independent review of the build found three more defects, fixed in the
   same build, and `live:check` learned to expand a control's rendered state
   once so a library form stays under its branch limit) - the adversarial
   review of Live and
   the component library before the 2.1.0 release found eighteen defects,
   among them a combobox that freezes the page, form macros that render no
   island state, undecodable proposals dropped with an accepted outcome, and
   keys the checker proves and the runtime refuses; promoted on the
   developer's direction of 2026-09-17 (`component-library-forms.md` FORM-008
   to FORM-012, `component-library-feedback.md` FDB-007,
   `component-library-navigation.md` NAV-007, `component-library-overlays.md`
   OVL-007, `component-library-data-display.md` DATA-006,
   `component-library-foundations.md` UI-020 to UI-024, `live.md` LIVE-031
   to LIVE-036, `docs/commitments/live-library-review-remediation.md`).

13. live-model-render-baseline - DONE 2026-09-17 (built at `661e7310`,
   formatted at `3c868f88` after the first check, receipts `3166159b`, review
   `4c1ba597`; the settle step also moved `dirty` onto the last applied render,
   which nothing had updated since mount) - after a refused value and a render
   that replaces it,
   the browser sent nothing when the same value was typed again, leaving a
   control and its island out of step with no error; found while proving the
   remediation's reset, promoted on the developer's `ok` to escalation
   `live-031` (`live.md` LIVE-037,
   `docs/commitments/live-model-render-baseline.md`).

14. tooltip-dismissal - a tooltip bubble that covers content cannot be dismissed
   where it is shown, which WCAG 2.2 success criterion 1.4.13 requires;
   specified 2026-09-17 in the phase between loops, which revised OVL-002
   to admit a dismissal enhancement and agreed OVL-008
   (`component-library-overlays.md` OVL-002 revised and OVL-008,
   `docs/commitments/tooltip-dismissal.md`).

Out of every commitment above, by the developer's 2026-09-12 ruling:
complete blocks (auth flows, account settings, CTAs, pricing, dashboard
layouts), tag input, nested menus, command palette, drag-reorder,
auto-infinite-scroll, carousel, image cropper, rich text editor, rating
input. They are a separate project and enter this roadmap only if the
developer writes them in.

Every commitment inherits the Live gate and the repository gate; a
commitment is Done only when its mechanisms pass on a committed tree and
the review under `.cairn/reviews/` is recorded.
