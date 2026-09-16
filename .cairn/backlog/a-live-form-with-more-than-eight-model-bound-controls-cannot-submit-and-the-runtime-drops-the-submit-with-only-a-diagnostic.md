# A Live form with more than eight model-bound controls cannot submit, and the runtime drops the submit with only a diagnostic

Surfaced from: LIVE-027
Outside because: The eight-proposal cap is the reviewed wire protocol's request bound (Live spec 06); no requirement in checker-proves-runtime-accepts covers the protocol limits or how a refused submit is surfaced.
Captured: 2026-09-16T16:55:59.067Z

Found 2026-09-16 while the Live gate ran checker-proves-runtime-accepts: the dogfood form gallery's save form bound ten model controls. A submit proposes every model control associated with its form (crates/suprnova-live/browser/src/models/forms.ts, prepareAction), and both request validators refuse more than eight proposals (crates/suprnova-live/browser/src/protocol.ts, validateUpdateRequestV1 and V2, too_many_model_proposals; the server's limits are also 8). The builder's error is caught in the island transport and reported as transport_failed network_failure, so the action never runs, nothing reaches the server, and with live:submit.prevent the page does nothing visible. live:check proves such a form. The gallery now keeps five model controls in its save form; an application form with more than eight has no working Live submit and no clear signal why.
