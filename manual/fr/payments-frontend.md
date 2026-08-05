# Paiements - Intégration du frontend

Le serveur retourne un `SessionPayload` comme partie des props de
votre page Inertia. Le payload porte un champ `flow` qui indique au
frontend quel widget monter ; votre frontend dispatche sur `flow` et
ne nomme jamais un fournisseur spécifique. Ce chapitre couvre les
boucles de dispatch Svelte 5, React 19, et Vue 3.5, y compris le cycle
confirm-card-payment de Stripe Elements et le handler de step-up 3DS
hors-session.

Les cinq valeurs possibles de `flow` et leurs champs associés :

| `flow` | Champs | Widget |
|---|---|---|
| `stripe_elements` | `client_secret`, `publishable_key`, `provider_session_id` | Stripe Elements (formulaire de carte intégré) |
| `stripe_checkout_redirect` | `url`, `provider_session_id` | Redirection vers le checkout hébergé par Stripe |
| `paddle_inline` | `transaction_id`, `client_token`, `customer_token?` | Overlay inline Paddle.js |
| `mobile_money_prompt` | `provider_transaction_id`, `message`, `operator` | Prompt USSD / app opérateur + interrogation |
| `redirect` | `url`, `provider_session_id` | Redirection générique (Mollie, mock, etc.) |

Le contrôleur backend appelle `Checkout::start_session` et retourne le
résultat comme props Inertia - du point de vue du frontend l'API est
la même quel que soit l'adaptateur qui tourne.

## Dispatch sur `flow`, pas sur le fournisseur

Votre page de checkout lit le champ `flow` une fois et monte le widget
correspondant. Elle ne nomme jamais « Stripe » ou « Paddle » ; seul
l'amorçage qui a choisi l'adaptateur le sait. C'est le contrat sur
lequel se construit le reste du chapitre.

### Pourquoi Suprnova diverge

Laravel Cashier livre une vue Blade pour Stripe Checkout, un chemin de
partials pour la SCA, et une convention de SDK séparée pour Paddle.
Les chemins Stripe et Paddle ne partagent pas de contrat frontend - le
widget de chaque fournisseur est câblé à une action de contrôleur
différente et à un arbre de templates différent.

Suprnova inverse cela : le backend retourne toujours la même enum
`SessionPayload` et le frontend commute toujours sur `flow`. Ajouter
un nouveau fournisseur signifie ajouter un variant côté serveur et un
`case` côté client ; le reste de votre page de checkout ne bouge pas.
Le variant Mobile Money en est la preuve - il ne produit aucun widget
du tout (le client confirme sur son téléphone), et le dispatcher
l'absorbe sans aucun cas particulier dans le composant appelant.

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayload arrive dans les props de page Inertia
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
    // Stripe.js doit être chargé - ajoutez à index.html :
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // Câblez la soumission du formulaire :
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // Afficher l'erreur à l'utilisateur
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // Paiement terminé - naviguer ou afficher la confirmation
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // Paddle.js doit être chargé - ajoutez à index.html :
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // Interrogez votre propre backend, qui lit la table miroir des transactions.
    // Le handler de webhook met à jour la ligne quand le fournisseur nous notifie.
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
  <!-- Rendu seulement pour stripe_elements ; masqué sinon -->
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

Le garde-fou `mountedRef` empêche le double montage sous le double
rendu de développement du StrictMode de React 19.

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

## Charger les SDK de paiement

Ajoutez les scripts pertinents à votre `index.html` (ou point d'entrée
équivalent). N'incluez que ceux que votre sélection de fournisseur
exige :

```html
<!-- Stripe (à ajouter si vous utilisez stripe_elements ou stripe_checkout_redirect) -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle (à ajouter si vous utilisez paddle_inline) -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

Les deux scripts sont chargés de façon asynchrone par le navigateur.
Si vous utilisez Vite avec le code-splitting, chargez-les via un
`import()` dynamique ou incluez-les comme externals dans votre
`vite.config.ts` pour éviter de bundler vous-même les SDK des
fournisseurs.

Stripe et Paddle exigent tous les deux que vous chargiez le SDK depuis
leur propre CDN - Stripe en fait une condition de conformité PCI, et
Paddle s'appuie sur cela pour la réécriture d'URL en direct. Le
Subresource Integrity (`integrity="sha384-..."`) n'est utilisable sur
aucun des deux scripts parce que les deux fournisseurs livrent en
continu et ne publient pas de hash stables ; la frontière de confiance
est la connexion HTTPS plus le CDN du fournisseur. Si votre modèle de
menace exige du SRI pour tout ce que vous embarquez, c'est un signal
pour garder toute l'UI de paiement sur un checkout hébergé par le
fournisseur (`stripe_checkout_redirect`, ou l'overlay hébergé de
Paddle invoqué depuis une redirection émise par le serveur) plutôt que
dans votre propre page.

## Types TypeScript

Le type `SessionPayload` montré dans chaque exemple ci-dessus est une
union discriminée qui correspond à la forme sérialisée de l'enum Rust.
Vous pouvez le générer automatiquement avec `suprnova generate-types`
si votre `SessionPayload` est exposé via un wrapper
`#[derive(InertiaProps)]`, ou le définir manuellement comme montré.

## Interrogation de Mobile Money

`mobile_money_prompt` est le seul flow où le client ne touche jamais
plus votre page après l'arrivée du prompt. Il confirme sur son
téléphone (menu USSD ou push de l'app opérateur), le fournisseur
notifie votre handler de webhook, et votre frontend doit découvrir que
la transaction s'est réglée.

Câblez un petit endpoint de statut qui lit la table miroir
`payments_transactions` par `provider_transaction_id`. Le handler de
webhook installé par `webhook_routes(db)` garde la colonne de statut
de la ligne à jour ; votre endpoint ne fait que la refléter en
retour :

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

Le helper `pollMobileMoney` du frontend montré dans chaque exemple
ci-dessus frappe cet endpoint toutes les trois secondes avec un
plafond de cinq minutes. Les chaînes de statut viennent de l'enum
`PaymentStatus` et se sérialisent en snake_case : `created`,
`requires_action`, `pending`, `processing`, `authorized`, `expired`,
`succeeded`, `failed`, `canceled`, `refunded`, `partially_refunded`,
`disputed`.

## Gestion des erreurs - `RequiresClientAction`

Quand `Payment::charge` (capture côté serveur) retourne
`ChargeResult::RequiresClientAction`, le backend sérialise le résultat
en JSON et le retourne au frontend. Cela arrive pour les flux de
step-up 3DS hors-session où l'émetteur de la carte exige une
authentification supplémentaire.

Le JSON ressemble à ceci :

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` et `publishable_key` sont des `Option<String>` côté
Rust et seront absents du JSON quand une action n'en a pas besoin.
Vérifiez toujours les deux contre null avant de les passer à un SDK de
fournisseur, et laissez `action_kind` piloter le dispatch - ce champ
est toujours présent.

Votre contrôleur backend devrait détecter cela et le retourner comme
une prop Inertia distincte ou comme une réponse HTTP que le frontend
lit. Motif de contrôleur exemple :

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // Rediriger vers la page de succès
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // Rediriger le navigateur
    }
}
```

Côté frontend, dispatchez sur `action_kind` :

**Svelte 5 :**

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
      // Afficher le message d'échec 3DS
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
</script>
```

**React 19 :**

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

**Vue 3.5 :**

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

Le champ `action_kind` est une chaîne spécifique au fournisseur.
Actuellement `"stripe_3ds"` est la seule valeur produite par
l'adaptateur Stripe livré. Quand des adaptateurs supplémentaires
exigeront des actions client, ils ajouteront leurs propres valeurs
`action_kind` en suivant le même motif - écrivez une branche par
défaut (`console.warn("Unknown action_kind:", k)`) pour qu'une valeur
non reconnue échoue explicitement plutôt que de silencieusement
abandonner le paiement.

## Suivant

- [Paiements](payments.md) - la surface à cinq traits, le registre, et
  le motif d'amorçage qui produit le `SessionPayload`.
- [Paiements - Stripe](payments-stripe.md) - configuration côté
  serveur pour les flux `stripe_elements`, `stripe_checkout_redirect`,
  et `stripe_3ds`.
- [Paiements - Paddle](payments-paddle.md) - configuration côté
  serveur pour le flux `paddle_inline` et la répartition des
  responsabilités du Merchant-of-Record.
- [Paiements - Guide
  fournisseur](payments-provider-guide.md) - ajoutez un nouveau
  variant `SessionPayload` quand vous écrivez un adaptateur pour une
  passerelle que Suprnova ne livre pas.
- [Frontend](frontend.md) - configuration de page Inertia, typage de
  props, et comment `usePage` se branche sur votre starter Svelte /
  React / Vue.
