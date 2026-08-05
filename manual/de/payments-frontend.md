# Zahlungen - Frontend Integration

Der Server liefert eine `SessionPayload` als Teil Ihrer
Inertia-Page-Props. Die Payload trägt ein Feld `flow`, das dem
Frontend sagt, welches Widget zu mounten ist; Ihr Frontend verzweigt
auf `flow` und benennt niemals einen bestimmten Provider. Dieses
Kapitel behandelt die Dispatch-Schleifen für Svelte 5, React 19 und
Vue 3.5, einschließlich des Stripe-Elements-Confirm-Card-Payment-
Zyklus und des Off-Session-3DS-Step-up-Handlers.

Die fünf möglichen Werte von `flow` und ihre zugehörigen Felder:

| `flow` | Felder | Widget |
|---|---|---|
| `stripe_elements` | `client_secret`, `publishable_key`, `provider_session_id` | Stripe Elements (eingebettetes Kartenformular) |
| `stripe_checkout_redirect` | `url`, `provider_session_id` | Redirect zum gehosteten Stripe-Checkout |
| `paddle_inline` | `transaction_id`, `client_token`, `customer_token?` | Paddle.js-Inline-Overlay |
| `mobile_money_prompt` | `provider_transaction_id`, `message`, `operator` | USSD-/Operator-App-Prompt + Polling |
| `redirect` | `url`, `provider_session_id` | Generischer Redirect (Mollie, Mock usw.) |

Der Backend-Controller ruft `Checkout::start_session` auf und gibt
das Ergebnis als Inertia-Props zurück - aus Sicht des Frontends ist
die API dieselbe, unabhängig davon, welcher Adapter läuft.

## Nach `flow` verzweigen, nicht nach Provider

Ihre Checkout-Seite liest das Feld `flow` einmal und mountet das
passende Widget. Sie benennt niemals "Stripe" oder "Paddle"; nur das
Bootstrap, das den Adapter gewählt hat, weiß das. Das ist der
Vertrag, auf dem der Rest des Kapitels aufbaut.

### Warum Suprnova abweicht

Laravel Cashier liefert eine Blade-View für Stripe Checkout, einen
Partials-Pfad für SCA und eine separate SDK-Konvention für Paddle.
Die Stripe- und Paddle-Pfade teilen sich keinen Frontend-Vertrag -
das Widget jedes Providers ist an eine andere Controller-Aktion und
einen anderen Template-Baum verdrahtet.

Suprnova dreht das um: Das Backend liefert immer dasselbe Enum
`SessionPayload`, und das Frontend verzweigt immer auf `flow`. Ein
neuer Provider bedeutet, eine Variante auf der Serverseite und einen
`case` auf der Client-Seite hinzuzufügen; der Rest Ihrer
Checkout-Seite bewegt sich nicht. Die Mobile-Money-Variante ist der
Beweis - sie erzeugt überhaupt kein Widget (der Kunde bestätigt auf
seinem Telefon), und der Dispatcher nimmt sie auf, ohne dass die
aufrufende Komponente irgendeinen Sonderfall braucht.

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayload kommt in den Inertia-Page-Props an
  let session = $derived($page.props.session as SessionPayload);

  type MobileMoneyOperator =
    | { kind: "mtn_momo" }
    | { kind: "mpesa" }
    | { kind: "airtel_money" }
    | { kind: "orange_money" }
    | { kind: "lipila" }
    | { kind: "custom"; identifier: string };

  type SessionPayload =
    | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
    | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
    | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
    | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
    | { flow: "redirect"; url: string; provider_session_id: string };

  let mobileMessage = $state("");

  $effect(() => {
    if (!session) return;
    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        mobileMessage = session.message;
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  });

  async function mountStripeElements(s: Extract<SessionPayload, { flow: "stripe_elements" }>) {
    // Stripe.js muss geladen sein - zu index.html hinzufügen:
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // Formular-Submit verdrahten:
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // Fehler dem Nutzer anzeigen
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // Zahlung abgeschlossen - navigieren oder Bestätigung anzeigen
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // Paddle.js muss geladen sein - zu index.html hinzufügen:
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // Das eigene Backend pollen, das die Mirror-Transaktionstabelle liest.
    // Der Webhook-Handler aktualisiert die Zeile, wenn der Provider uns benachrichtigt.
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }
</script>

<div id="payment-form">
  <div id="card-element"></div>
  <!-- Nur für stripe_elements gerendert; sonst versteckt -->
  {#if session?.flow === "stripe_elements"}
    <button type="submit">Pay now</button>
  {/if}
  {#if session?.flow === "mobile_money_prompt"}
    <p>{mobileMessage}</p>
    <p>Waiting for confirmation…</p>
  {/if}
</div>
```

## React 19

```tsx
// src/pages/Billing/Checkout.tsx
import { useEffect, useRef, useState } from "react";
import { usePage } from "@inertiajs/react";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

export default function Checkout() {
  const { session } = usePage<{ session: SessionPayload }>().props;
  const mountedRef = useRef(false);
  const [mobileMessage, setMobileMessage] = useState("");

  useEffect(() => {
    if (!session || mountedRef.current) return;
    mountedRef.current = true;

    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        setMobileMessage(session.message);
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  }, [session]);

  async function mountStripeElements(
    s: Extract<SessionPayload, { flow: "stripe_elements" }>
  ) {
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });
    const card = elements.create("card");
    card.mount("#card-element");

    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(
    s: Extract<SessionPayload, { flow: "paddle_inline" }>
  ) {
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }

  return (
    <form id="payment-form">
      <div id="card-element" />
      {session?.flow === "stripe_elements" && (
        <button type="submit">Pay now</button>
      )}
      {session?.flow === "mobile_money_prompt" && (
        <div>
          <p>{mobileMessage}</p>
          <p>Waiting for confirmation…</p>
        </div>
      )}
    </form>
  );
}
```

Die `mountedRef`-Guard verhindert doppeltes Mounten unter React 19s
Doppel-Render in der StrictMode-Entwicklungsumgebung.

## Vue 3.5

```vue
<!-- src/pages/Billing/Checkout.vue -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { usePage } from "@inertiajs/vue3";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

const page = usePage<{ session: SessionPayload }>();
const session = page.props.session;
const isStripeElements = ref(session?.flow === "stripe_elements");
const isMobileMoney = ref(session?.flow === "mobile_money_prompt");
const mobileMessage = ref(
  session?.flow === "mobile_money_prompt" ? session.message : ""
);

onMounted(() => {
  if (!session) return;
  switch (session.flow) {
    case "stripe_elements":
      mountStripeElements(session);
      break;
    case "stripe_checkout_redirect":
      window.location.href = session.url;
      break;
    case "paddle_inline":
      mountPaddleInline(session);
      break;
    case "mobile_money_prompt":
      pollMobileMoney(session.provider_transaction_id);
      break;
    case "redirect":
      window.location.href = session.url;
      break;
  }
});

async function mountStripeElements(
  s: Extract<SessionPayload, { flow: "stripe_elements" }>
) {
  const stripe = (window as any).Stripe(s.publishable_key);
  const elements = stripe.elements({ clientSecret: s.client_secret });
  const card = elements.create("card");
  card.mount("#card-element");

  const form = document.getElementById("payment-form") as HTMLFormElement;
  form?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
      payment_method: { card },
    });
    if (error) {
      console.error(error.message);
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  });
}

function mountPaddleInline(
  s: Extract<SessionPayload, { flow: "paddle_inline" }>
) {
  const Paddle = (window as any).Paddle;
  Paddle.Initialize({ token: s.client_token });
  Paddle.Checkout.open({
    transactionId: s.transaction_id,
    customerToken: s.customer_token,
  });
}

async function pollMobileMoney(txId: string) {
  const deadline = Date.now() + 5 * 60_000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 3000));
    const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
    const { status } = await res.json();
    if (status === "succeeded") {
      window.location.href = "/billing/success";
      return;
    }
    if (status === "failed" || status === "canceled" || status === "expired") {
      window.location.href = "/billing/failed";
      return;
    }
  }
}
</script>

<template>
  <form id="payment-form">
    <div id="card-element" />
    <button v-if="isStripeElements" type="submit">Pay now</button>
    <div v-if="isMobileMoney">
      <p>{{ mobileMessage }}</p>
      <p>Waiting for confirmation…</p>
    </div>
  </form>
</template>
```

## Die Payment-SDKs laden

Fügen Sie die relevanten Skripte zu Ihrer `index.html` hinzu (oder
dem entsprechenden Einstiegspunkt). Binden Sie nur die ein, die Ihre
Provider-Auswahl braucht:

```html
<!-- Stripe (hinzufügen, falls stripe_elements oder stripe_checkout_redirect verwendet wird) -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle (hinzufügen, falls paddle_inline verwendet wird) -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

Beide Skripte werden vom Browser asynchron geladen. Wenn Sie Vite
mit Code-Splitting verwenden, laden Sie diese über dynamisches
`import()` oder nehmen Sie sie als Externals in Ihre `vite.config.ts`
auf, um die Provider-SDKs nicht selbst zu bundlen.

Stripe und Paddle verlangen beide, dass Sie das SDK von ihrem
eigenen CDN laden - bei Stripe ist das eine PCI-Compliance-Vorgabe,
und Paddle stützt sich dafür auf das Live-URL-Rewriting. Subresource
Integrity (`integrity="sha384-..."`) ist bei keinem der beiden
Skripte nutzbar, weil beide Anbieter kontinuierlich ausliefern und
keine stabilen Hashes veröffentlichen; die Vertrauensgrenze ist die
HTTPS-Verbindung plus das CDN des Anbieters. Wenn Ihr Bedrohungsmodell
SRI für alles verlangt, was Sie einbetten, ist das ein Signal, alle
Zahlungs-UI auf einem vom Anbieter gehosteten Checkout zu halten
(`stripe_checkout_redirect`, oder Paddles gehostetes Overlay, von
einem serverseitig ausgelösten Redirect aufgerufen), statt auf Ihrer
eigenen Seite.

## TypeScript Types

Der in jedem Beispiel oben gezeigte Typ `SessionPayload` ist eine
diskriminierte Union, die der serialisierten Form des Rust-Enums
entspricht. Sie können ihn automatisch mit `suprnova generate-types`
erzeugen, wenn Ihre `SessionPayload` über einen
`#[derive(InertiaProps)]`-Wrapper exponiert wird, oder ihn wie
gezeigt manuell definieren.

## Mobile-Money-Polling

`mobile_money_prompt` ist der einzige Flow, bei dem der Kunde Ihre
Seite nie wieder berührt, nachdem der Prompt eintrifft. Er bestätigt
auf seinem Telefon (USSD-Menü oder Operator-App-Push), der Provider
benachrichtigt Ihren Webhook-Handler, und Ihr Frontend muss
entdecken, dass die Transaktion abgerechnet wurde.

Verdrahten Sie einen kleinen Status-Endpunkt, der die
Mirror-Tabelle `payments_transactions` nach
`provider_transaction_id` liest. Der von `webhook_routes(db)`
installierte Webhook-Handler hält die Statusspalte der Zeile
aktuell; Ihr Endpunkt gibt sie nur zurück:

```rust,ignore
use suprnova::{Json, Query, json_response};
use suprnova::payments::entities::transaction;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    pub transaction_id: String,
}

pub async fn status(Query(q): Query<StatusQuery>) -> Json<serde_json::Value> {
    let db = suprnova::db().await;
    let row = transaction::Entity::find()
        .filter(transaction::Column::ProviderTransactionId.eq(q.transaction_id))
        .one(&db)
        .await
        .unwrap();
    let status = row.map(|r| r.status).unwrap_or_else(|| "pending".into());
    Json(serde_json::json!({ "status": status }))
}
```

Der Frontend-Helfer `pollMobileMoney`, der in jedem Beispiel oben
gezeigt wird, trifft diesen Endpunkt alle drei Sekunden mit einer
Obergrenze von fünf Minuten. Status-Strings kommen aus dem Enum
`PaymentStatus` und serialisieren als snake_case: `created`,
`requires_action`, `pending`, `processing`, `authorized`, `expired`,
`succeeded`, `failed`, `canceled`, `refunded`,
`partially_refunded`, `disputed`.

## Fehlerbehandlung - `RequiresClientAction`

Wenn `Payment::charge` (serverseitige Erfassung)
`ChargeResult::RequiresClientAction` liefert, serialisiert das
Backend das Ergebnis zu JSON und gibt es an das Frontend zurück. Das
passiert bei Off-Session-3DS-Step-up-Flows, bei denen der
Kartenaussteller zusätzliche Authentifizierung verlangt.

Das JSON sieht so aus:

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` und `publishable_key` sind auf der Rust-Seite
`Option<String>` und fehlen im JSON, wenn eine Aktion sie nicht
braucht. Prüfen Sie beide immer auf `null`, bevor Sie sie an ein
Provider-SDK übergeben, und lassen Sie `action_kind` den Dispatch
steuern - dieses Feld ist immer vorhanden.

Ihr Backend-Controller sollte das erkennen und als eigenständige
Inertia-Prop oder als HTTP-Antwort zurückgeben, die das Frontend
liest. Beispiel-Controller-Muster:

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // Auf die Erfolgsseite weiterleiten
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // Den Browser umleiten
    }
}
```

Verzweigen Sie im Frontend auf `action_kind`:

**Svelte 5:**

```svelte
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  let props = $derived($page.props as {
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  });

  $effect(() => {
    if (!props.action_kind) return;
    switch (props.action_kind) {
      case "stripe_3ds":
        handleStripe3DS(props.client_secret!, props.publishable_key!);
        break;
      default:
        console.warn("Unknown action_kind:", props.action_kind);
    }
  });

  async function handleStripe3DS(clientSecret: string, publishableKey: string) {
    const stripe = (window as any).Stripe(publishableKey);
    const { error, paymentIntent } = await stripe.handleNextAction({ clientSecret });
    if (error) {
      // 3DS-Fehlschlagsmeldung anzeigen
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
</script>
```

**React 19:**

```tsx
import { usePage } from "@inertiajs/react";
import { useEffect } from "react";

export default function ThreeDSChallenge() {
  const { action_kind, client_secret, publishable_key } = usePage<{
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  }>().props;

  useEffect(() => {
    if (!action_kind) return;
    if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
      const stripe = (window as any).Stripe(publishable_key);
      stripe.handleNextAction({ clientSecret: client_secret }).then(
        ({ error, paymentIntent }: any) => {
          if (!error && paymentIntent?.status === "succeeded") {
            window.location.href = "/billing/success";
          }
        }
      );
    }
  }, [action_kind]);

  return <div>Completing payment authentication...</div>;
}
```

**Vue 3.5:**

```vue
<script setup lang="ts">
import { onMounted } from "vue";
import { usePage } from "@inertiajs/vue3";

const { action_kind, client_secret, publishable_key } = usePage<{
  action_kind: string;
  client_secret?: string;
  publishable_key?: string;
}>().props;

onMounted(async () => {
  if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
    const stripe = (window as any).Stripe(publishable_key);
    const { error, paymentIntent } = await stripe.handleNextAction({
      clientSecret: client_secret,
    });
    if (!error && paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
});
</script>

<template>
  <p>Completing payment authentication...</p>
</template>
```

Das Feld `action_kind` ist ein providerspezifischer String. Derzeit
ist `"stripe_3ds"` der einzige Wert, den der mitgelieferte
Stripe-Adapter erzeugt. Wenn weitere Adapter clientseitige Aktionen
brauchen, fügen sie ihre eigenen `action_kind`-Werte nach demselben
Muster hinzu - schreiben Sie einen Default-Zweig
(`console.warn("Unknown action_kind:", k)`), damit ein nicht
erkannter Wert sichtbar fehlschlägt, statt die Zahlung
stillschweigend fallenzulassen.

## Nächste Schritte

- [Zahlungen](payments.md) - die Fünf-Trait-Oberfläche, die
  Registry und das Bootstrap-Muster, das die `SessionPayload`
  erzeugt.
- [Zahlungen - Stripe](payments-stripe.md) - serverseitige
  Konfiguration für die Flows `stripe_elements`,
  `stripe_checkout_redirect` und `stripe_3ds`.
- [Zahlungen - Paddle](payments-paddle.md) - serverseitige
  Konfiguration für den Flow `paddle_inline` und den
  Merchant-of-Record-Zuständigkeits-Split.
- [Zahlungen - Provider-Leitfaden](payments-provider-guide.md) -
  fügen Sie eine neue `SessionPayload`-Variante hinzu, wenn Sie
  einen Adapter für ein Gateway schreiben, das Suprnova nicht
  mitliefert.
- [Frontend](frontend.md) - Inertia-Page-Setup, Prop-Typisierung und
  wie `usePage` in Ihren Svelte-/React-/Vue-Starter einklinkt.
