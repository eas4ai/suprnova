# Identical descriptors from concurrent issuance are told apart in the host credential store, not by a nonce in the claims

Level: Consequential
Decided by: Shawn
Rests on: LIVE-022
Would be wrong if: A binding's unconsumed secrets outgrow the entry cap under sustained concurrent issuance, or a consumer needs each descriptor to be unique for a reason other than connecting once.

## Decision

The host's SuprnovaSubscriptionCredentials keeps every secret issued for a binding until each is consumed or expires, so two issuances that mint the same signed descriptor in one millisecond each connect once with their own secret. The alternative, a per-issuance nonce inside the signed claims, makes every descriptor unique but changes the descriptor schema, the browser's generated contract and the conformance fixture for a collision that only same-millisecond issuance of one scope produces. The developer accepted the recommendation at 09:05 on 2026-09-14 ("confirmed").

## Realized by

- 6546bb31  fix(live): the credential store keeps every unconsumed secret per binding (LIVE-022)
