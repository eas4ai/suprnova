# Paiements - Adaptateur Stripe

`suprnova-payments-stripe` est l'adaptateur de référence pour la
surface de paiement neutre du point de vue du fournisseur de Suprnova.
Il implémente les cinq traits de paiement (`Checkout`, `Payment`,
`Subscription`, `CustomerStore`, `WebhookHandler`) contre l'API Stripe
via `async-stripe` 1.0.0-rc.5. Tournez-vous vers ce chapitre quand
vous devez savoir exactement quel endpoint Stripe une méthode appelle,
comment le format de signature du webhook est vérifié, comment les
PaymentIntents circulent à travers `ChargeResult`, ou quels types
d'événement se mappent sur l'enum d'événement neutre.

Pour les formes de trait elles-mêmes, la configuration des variables
d'environnement, et le motif d'amorçage, lisez d'abord
[Paiements](payments.md). Ce chapitre est l'exploration approfondie
spécifique à Stripe.

## Passerelle, pas Merchant of Record

Stripe est par défaut une **passerelle de paiement** : vous recevez
les fonds directement sur votre propre compte bancaire, et vous êtes
responsable de la collecte et du versement de la taxe, de la
facturation, des relances, et de la gestion des rétrofacturations. À
l'opposé, avec Paddle ([Paiements - Paddle](payments-paddle.md)),
Paddle est le Merchant of Record - il collecte les fonds, dépose la
taxe, et vous reverse net de ses frais.

La conséquence pratique pour ce chapitre : `StripeProvider` implémente
`Payment` (vous pouvez autoriser, capturer, rembourser, et annuler une
carte côté serveur). `PaddleProvider` ne le fait pas. La séparation
des traits existe parce que les deux flux sont réellement
différents - pas par manque de temps.

### Stripe Managed Payments (opt-in Merchant of Record)

Le programme **Managed Payments** de Stripe déplace Stripe dans le
rôle de Merchant of Record pour les transactions éligibles - Stripe
devient le vendeur légal, calcule, collecte, dépose et reverse la taxe
de vente/TVA/GST, et prend en charge les litiges. Le programme a des
contraintes d'intégration strictes :

- **Checkout hébergé uniquement.** Les sessions doivent tourner sur la
  page hébergée de Stripe. Les flux Elements/personnalisés sont
  exclus - c'est pourquoi le chemin ponctuel hébergé de l'adaptateur
  (ci-dessous) est la seule forme `OneOff` qui compose avec lui.
- **Prix prédéfinis avec des codes de taxe éligibles.** Les lignes
  doivent référencer des objets `price_…` dont les produits portent un
  code de taxe marqué éligible à Managed Payments dans le tableau de
  bord Stripe. Les montants ad hoc sont rejetés.
- **Inscription du compte.** Le compte Stripe doit être intégré au
  programme ; les sessions portant le flag sur un compte non inscrit
  échouent.

Activez-le par fournisseur avec `.with_managed_payments(true)` ou
`STRIPE_MANAGED_PAYMENTS=true` - l'adaptateur envoie alors
`managed_payments[enabled]=true` lors de la création de sessions
ponctuelles hébergées. Quand c'est désactivé (le défaut), le champ est
omis entièrement.

### Pourquoi Suprnova diverge

Laravel livre Cashier comme une intégration Stripe officielle,
présente dans sa documentation centrale. C'est pratique, mais réservé
à Stripe - et ajouter un second fournisseur signifie soit forker
Cashier, soit construire une surface parallèle.

Suprnova garde Stripe à distance. L'adaptateur Stripe est une seule
crate qui s'enregistre contre les cinq mêmes traits que n'importe quel
autre fournisseur implémente. Votre code métier ne nomme jamais
`StripeProvider` ; il appelle `provider.charge(...)` sur un
`Arc<dyn PaymentProvider>` résolu depuis le registre, et le
comportement Stripe n'est qu'à un échange du comportement Paddle.
Quand vous ajoutez plus tard Mollie, ou câblez une passerelle
régionale qui n'existe pas encore, vous implémentez les cinq mêmes
traits et le reste de votre app ne bouge pas.

## Construction

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Production : lecture depuis l'env.
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// Tests / config explicite :
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` est `Clone` (peu coûteux - le `stripe::Client`
sous-jacent est adossé à un `Arc`) et détient ces valeurs :

| Champ | Source | Usage |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | `Authorization: Bearer …` HTTP sur chaque appel API |
| `publishable_key` | `pk_live_…` / `pk_test_…` | Exposée dans `SessionPayload::StripeElements` pour que le frontend puisse monter Stripe.js sans recherche de config séparée |
| `webhook_signing_secret` | `whsec_…` | Vérification HMAC-SHA256 de l'en-tête `Stripe-Signature` |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS` (`true`/`1`) ou `.with_managed_payments(bool)` | Envoie `managed_payments[enabled]=true` à la création d'une session ponctuelle hébergée (voir [Managed Payments](#stripe-managed-payments-opt-in-merchant-of-record)) |

`from_env()` retourne `Result<Self, String>` - le message d'erreur
nomme la variable requise manquante (`STRIPE_MANAGED_PAYMENTS` est
optionnelle ; absente signifie désactivée). Il n'y a pas de chemin de
panique à l'amorçage.

## Sessions de checkout

`Checkout::start_session` choisit sa surface Stripe à partir de la
requête :

| Forme de requête | Objet Stripe | Variant `SessionPayload` |
|---|---|---|
| `OneOff` + `price_refs` non vide | Session Checkout hébergée, `mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + `price_refs` vide + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | Session Checkout hébergée, `mode=subscription` | `StripeCheckoutRedirect` |

Le chemin ponctuel hébergé envoie `allow_promotion_codes=true` (les
clients peuvent saisir des codes promo sur la page de Stripe - à
associer au trait `Promotions` ci-dessous) et, quand le fournisseur
est configuré pour cela, le flag Managed Payments. Placez le littéral
de template `{CHECKOUT_SESSION_ID}` de Stripe dans votre
`success_return_url` - Stripe substitue l'id `cs_…` réel lors de la
redirection, et votre page de retour l'alimente vers `session_status`.

`Checkout::session_status` mappe `GET /v1/checkout/sessions/{id}` sur
le `CheckoutSessionState` neutre :

| `status` / `payment_status` Stripe | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` ou `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid` (règlement différé) | `Complete { paid: false, … }` |

`payment_ref` porte l'id PaymentIntent de la session (`pi_…`) pour que
les pages de retour et les balayages puissent corréler la session avec
les opérations `Payment` et le miroir `payments_transactions`.
`amount_total` est le total réglé, remises côté fournisseur et taxe
Managed Payments déjà incluses.

## Codes promo

`StripeProvider` implémente le trait optionnel `Promotions`
(`provider.as_promotions()` retourne `Some`). `create_promotion_code`
correspond à `POST /v1/promotion_codes` : il émet un code à partir
d'un coupon pré-créé (`coupon_ref`), restreint à un client
(`customer_ref`), avec une expiration et un plafond de rédemption
optionnels. Les restrictions sont appliquées par Stripe au moment de
la rédemption - un code émis pour le client A est rejeté quand le
client B le saisit, les codes expirés sont rejetés, et
`max_redemptions: Some(1)` rend le code à usage unique. Voir la
section `Promotions` de [Paiements](payments.md) pour le motif de
campagne.

## Le cycle de vie de PaymentIntent

Stripe représente une tentative de charge unique comme un
**PaymentIntent**. L'intent traverse des statuts ; le trait `Payment`
de Suprnova pilote les transitions. Chaque méthode `Payment` de
`StripeProvider` correspond à un endpoint `/v1/payment_intents/...` :

| Méthode `Payment` | Endpoint Stripe | Ce qu'elle fait |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | Crée et confirme en un seul appel contre un moyen de paiement sauvegardé. `capture_method: "manual"`, si bien que l'intent passe à `requires_capture`, **pas** à `succeeded`. |
| `capture` | `POST /v1/payment_intents/{id}/capture` | Règle un intent préalablement autorisé. Statut `requires_capture` → `succeeded`. |
| `refund` | `POST /v1/refunds` | Inverse totalement ou partiellement un intent capturé. |
| `void` | `POST /v1/payment_intents/{id}/cancel` | Libère une autorisation avant capture. Statut `requires_capture` → `canceled`. |
| `status` | `GET /v1/payment_intents/{id}` | Récupère le statut courant (retourne `PaymentStatus`). |

### Autoriser d'abord, capturer ensuite

`StripeProvider::charge` ne règle **pas** les fonds immédiatement. Il
envoie `capture_method=manual` + `confirm=true`, ce qui autorise la
carte et réserve les fonds, puis attend un appel `capture` explicite.
C'est le flux canonique en deux étapes :

```rust
use suprnova::payments::{
    PaymentProviderRegistry, ChargeRequest, ChargeResult,
    Money, Currency, PaymentStatus,
};

let provider = PaymentProviderRegistry::get("stripe").unwrap();
let payment = provider.as_payment()
    .expect("Stripe implements Payment");

let result = payment.charge(ChargeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    payment_method_ref: "pm_card_visa".into(),
    amount: Money::from_minor_units(2999, Currency::USD),
    description: Some("Pro plan, manual capture".into()),
    idempotency_key: Some("order-12345".into()),  // voir « Idempotence » ci-dessous
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // Autorisé - réglez quand la commande est expédiée.
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // Step-up 3DS nécessaire - voir « 3DS et SCA » ci-dessous.
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

Si vous voulez une capture **immédiate** - le cas ponctuel courant en
e-commerce - utilisez plutôt `Checkout::start_session` avec
`SessionMode::OneOff`. Ce chemin crée un PaymentIntent avec
`automatic_payment_methods` activé et remet le secret côté client au
frontend pour que le navigateur du client confirme l'intent sur place.
`Payment::charge` est pour les flux pilotés par le serveur où vous
détenez déjà le moyen de paiement sauvegardé du client et voulez un
contrôle explicite autoriser-puis-capturer (typique des places de
marché, du SaaS à exécution différée, ou du commerce à expédition
scindée).

### Correspondance des statuts

Les statuts Stripe se replient dans l'enum `PaymentStatus` de
Suprnova :

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending` (autorisé, en attente de capture) |
| `RequiresAction` | `Pending` (retourné comme `RequiresClientAction` par `charge`) |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _nouveau statut Stripe (l'enum est `#[non_exhaustive]`)_ | `Failed` |

Le repli `non_exhaustive` est intentionnel. Stripe ajoute
occasionnellement des états (par ex. lors de l'introduction de
nouveaux types de moyens de paiement). Les remonter comme `Failed` est
le défaut prudent - votre app traite la charge comme
pas-encore-confirmée jusqu'à ce que vous mettiez à jour l'adaptateur.

### 3DS et SCA

L'authentification forte du client européenne, les règles de la RBI
indienne, et plusieurs autres régulateurs exigent que le porteur de
carte s'authentifie dans un contexte de navigateur séparé. Stripe
remonte cela comme `requires_action` avec un bloc `next_action`.

`StripeProvider::charge` traduit cela en l'un de deux variants
`ChargeResult` :

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - conservez-le
    action_kind: "stripe_3ds", // tag spécifique à Stripe
    client_secret,             // à remettre à Stripe.js
    publishable_key,           // à remettre à Stripe.js
}
```

Quand le `next_action` de l'intent contient une URL de redirection
(certains flux d'authentification sont à redirection d'URL plutôt qu'à
modale sur place), le résultat est réécrit comme :

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // redirigez le navigateur ici
    return_to: None,
}
```

Votre contrôleur remet le payload `RequiresClientAction` à la page
Inertia ; le frontend appelle
`stripe.confirmCardPayment(client_secret, ...)` et le client complète
le 3DS. Quand la confirmation réussit, Stripe déclenche
`payment_intent.succeeded` et la route de webhook écrit la ligne
miroir. Voir [Paiements - Intégration du
frontend](payments-frontend.md) pour les extraits Svelte / React /
Vue.

### Annulation vs remboursement

`void` libère une autorisation **avant** la capture ; `refund` inverse
un paiement capturé. Appeler `void` sur un intent capturé
échouera - Stripe rejette avec un message contenant
`"already succeeded"` ou `"You cannot cancel"`, et l'adaptateur
remonte cela comme `PaymentError::Validation` pour que votre handler
puisse distinguer une erreur utilisateur récupérable (utilisez
`refund` à la place) d'une véritable panne du fournisseur. Tout autre
échec est `PaymentError::Provider`.

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* autorisation libérée */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // Déjà capturé - appelez refund à la place.
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // remboursement complet
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund() ne transmet pas ceci - voir « Idempotence »
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Clients

`StripeProvider` implémente `CustomerStore` contre `/v1/customers`.
L'adaptateur mappe un `Customer` retourné vers le `CustomerRef`
neutre, en préservant l'email et le `user_id` de votre application :

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // id utilisateur de votre app
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// Persistez ceci à côté de votre ligne User pour que les charges,
// abonnements et webhooks suivants s'y résolvent en retour.
```

`update_customer`, `get_customer`, et `delete_customer` frappent
`POST /v1/customers/{id}`, `GET /v1/customers/{id}`, et
`DELETE /v1/customers/{id}` respectivement. Le delete de Stripe
retourne une enveloppe `DeletedCustomer` que l'adaptateur
écarte - seuls le succès/l'échec de l'appel est propagé.

## Abonnements

`StripeProvider::subscribe` poste vers `/v1/subscriptions` avec la
référence client, un tableau `items[]`, et un `trial_period_days`
optionnel :

```rust
use suprnova::payments::{SubscribeRequest, SubscriptionStatus};

let sub = provider.subscribe(SubscribeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    price_refs: vec!["price_pro_monthly".into()],
    trial_days: Some(14),
    idempotency_key: None,
    metadata: None,
}).await?;

assert!(matches!(
    sub.status,
    SubscriptionStatus::Trialing | SubscriptionStatus::Active
));

println!("Period ends at {}", sub.current_period_end);
for item in &sub.items {
    println!(
        "  {} × {} @ {:?}",
        item.quantity, item.provider_price_id, item.unit_amount,
    );
}
```

### Bornes de période

Stripe a déplacé les timestamps `current_period_start` /
`current_period_end` du Subscription parent vers chaque
`SubscriptionItem` dans la version d'API `2023-08-16`. Les abonnements
multi-lignes peuvent en théorie avoir des périodes de ligne
divergentes, mais en pratique chaque ligne d'un même abonnement
partage le cycle de facturation du parent. L'adaptateur prend la
période de la **première ligne** comme période parente dans le
`SubscriptionResult` retourné. Si vous avez vraiment besoin de
périodes par ligne, lisez-les depuis `sub.items[n]` - elles sont
préservées sur l'instantané.

### Annuler à la fin de la période vs immédiatement

```rust
// Annulation douce - garde l'accès jusqu'à current_period_end :
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// Annulation immédiate - Stripe DELETE /v1/subscriptions/{id} :
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

Les deux chemins frappent des endpoints Stripe différents. Le soft
cancel est `POST /v1/subscriptions/{id}` avec
`cancel_at_period_end=true` - l'abonnement reste actif jusqu'à la fin
de la période de facturation, puis Stripe le finalise. Le cancel
immédiat est `DELETE /v1/subscriptions/{id}` avec `prorate=false` et
`invoice_now=false`.

### `update()` est délibérément limitée

`UpdateSubscriptionRequest` a deux champs sur lesquels l'adaptateur
agit : `cancel_at_period_end` et `new_price_refs`. Le premier est
supporté ; le second retourne `PaymentError::NotSupported` :

```rust
provider.update(UpdateSubscriptionRequest {
    provider_subscription_id: "sub_1234".into(),
    new_price_refs: Some(vec!["price_team_yearly".into()]),
    cancel_at_period_end: None,
    idempotency_key: None,
}).await
// → Err(PaymentError::NotSupported(
//      "Stripe price-set replacement on existing subscription not in v1. \
//       Cancel the subscription and create a new one with the new price set."
//   ))
```

C'est l'un des rares endroits où `NotSupported` est la réponse honnête
plutôt qu'un report. Le remplacement d'un jeu de prix Stripe exige de
supprimer et recréer les lignes d'abonnement - la forme varie selon le
fournisseur (proration, ancrage du cycle de facturation, comportement
d'essai retenu) et regrouper cela dans une seule API neutre aurait
caché plus qu'aidé. Le chemin recommandé est d'annuler l'abonnement
existant et de `subscribe` à nouveau avec le nouveau jeu de prix, en
appliquant votre propre politique de proration si vous en avez besoin.

## Webhooks

Stripe envoie des webhooks signés avec HMAC-SHA256 dans le format :

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` analyse l'en-tête, recalcule le HMAC-SHA256
sur `"{timestamp}.{raw_body}"` en utilisant le secret de signature du
webhook, et fait une comparaison en **temps constant** contre chaque
valeur `v1=` de l'en-tête. Plusieurs valeurs `v1=` existent pendant la
rotation du secret de signature - Stripe superpose l'ancien et le
nouveau secret pendant une fenêtre pour que vous puissiez re-signer et
redéployer sans bascule forcée le même jour.

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

L'adaptateur accepte la requête si **une seule** valeur `v1=`
correspond. Un en-tête sans `t=` ou sans aucune valeur `v1=` est
rejeté comme `PaymentError::WebhookSignature`. Les octets non-ASCII
n'importe où dans l'en-tête sont aussi rejetés - Stripe ne les envoie
jamais, et les traiter comme invalides est plus sûr que de substituer
un caractère de remplacement.

Vous n'appelez jamais `verify` directement.
`webhook_routes(db.clone())` du framework enregistre
`POST /webhooks/payments/{provider}` et invoque `verify` +
`parse_event` + les extracteurs de payload de l'adaptateur pour chaque
requête qui y arrive. Voir [Idempotence](idempotency.md) pour le
comportement d'audit conscient des nouvelles tentatives - y compris la
règle selon laquelle les événements précédemment en échec retentent
l'hydratation quand le fournisseur relance.

### Correspondance événement → neutre

Les types d'événement Stripe se mappent sur le `NeutralEventKind` de
Suprnova via la fonction `stripe_event_to_neutral`. La table de
correspondance :

| Type d'événement Stripe | `NeutralEventKind` |
|---|---|
| `payment_intent.succeeded` | `PaymentSucceeded` |
| `payment_intent.payment_failed` | `PaymentFailed` |
| `charge.refunded` | `PaymentRefunded` |
| `charge.dispute.created` | `PaymentDisputed` |
| `customer.subscription.created` | `SubscriptionCreated` |
| `customer.subscription.updated` | `SubscriptionUpdated` |
| `customer.subscription.deleted` | `SubscriptionCanceled` |
| `customer.subscription.paused` | `SubscriptionUpdated` |
| `customer.subscription.resumed` | `SubscriptionUpdated` |
| `customer.subscription.trial_will_end` | `SubscriptionUpdated` |
| `invoice.payment_succeeded` / `invoice.paid` | `InvoicePaid` |
| `invoice.payment_failed` | `InvoiceFailed` |
| `customer.created` | `CustomerCreated` |
| `customer.updated` | `CustomerUpdated` |
| _tout le reste_ | `None` |

Les événements qui se mappent sur `None` (signaux de fraude Radar,
paiements sortants, virements de solde, événements de cycle de vie de
litige après `created`) sont malgré tout persistés dans la table
d'audit `payments_webhook_events` - ils ne pilotent simplement pas les
tables miroir. Si vous en avez besoin, lisez directement
`event.raw_payload` dans un handler personnalisé.

La correspondance est aussi ré-exportée à la racine de la crate pour
que vous puissiez l'utiliser hors de la route de webhook :

```rust
use suprnova_payments_stripe::stripe_event_to_neutral;
use suprnova::payments::NeutralEventKind;

assert_eq!(
    stripe_event_to_neutral("payment_intent.succeeded"),
    Some(NeutralEventKind::PaymentSucceeded),
);
assert_eq!(
    stripe_event_to_neutral("radar.early_fraud_warning.created"),
    None,
);
```

### Extraction du payload

Après le succès de `verify` et `parse_event`, le framework appelle
`extract_payload_ids`, `extract_payment_snapshot`, et
`extract_customer_snapshot` pour tirer les champs qui pilotent les
tables miroir (voir [Eloquent](eloquent.md) pour le motif sous-jacent
de lecture depuis votre propre DB). Stripe est structurellement
cohérent : chaque webhook place l'entité concernée à `data.object`,
avec `id` comme clé primaire.

Les extracteurs gèrent quatre familles d'événements :

- **Événements d'abonnement** - tirent `data.object.id` (l'id de
  l'abonnement) et `data.object.customer`.
- **Événements client** - tirent `data.object.id` (l'id du client).
- **Événements PaymentIntent / Charge** - tirent `data.object.id`,
  `data.object.amount`, `data.object.currency`,
  `data.object.customer`, et (pour `payment_intent.succeeded`
  seulement) `data.object.created` comme `paid_at`.
- **Événements de facture** - tirent `data.object.id`, le pointeur
  client, `data.object.subscription` (charges récurrentes seulement),
  `amount_paid` (repli sur `amount_due`), `tax`, `currency`, et
  `data.object.status_transitions.paid_at`.

Tout le reste retourne `None` depuis les extracteurs d'instantané ; la
ligne d'audit atterrit malgré tout.

## Tables miroir

Six tables portent la surface de paiement dans la base de données de
votre application. Appliquez la migration du framework à côté de la
vôtre :

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... vos migrations ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

Les tables créées sont `payments_customers`,
`payments_payment_methods`, `payments_subscriptions`,
`payments_subscription_items`, `payments_transactions`, et
`payments_webhook_events`. La route de webhook les hydrate à
l'intérieur d'une seule transaction DB par événement - un état partiel
n'est jamais observable, et la ligne d'audit porte `process_error` à
travers les nouvelles tentatives pour que les échecs restent visibles
pour les opérateurs.

## Idempotence

L'idempotence sortante sur les appels API Stripe et l'idempotence
entrante sur les livraisons de webhook sont deux histoires séparées.
Lisez-les comme telles.

### Sortante : couverture par méthode

Stripe supporte l'idempotence de requête via l'en-tête HTTP de requête
`Idempotency-Key` - la même clé avec le même corps retourne le même
objet de réponse pendant une fenêtre de rejeu de 24 heures ; un corps
non correspondant retourne une erreur. L'adaptateur Stripe de Suprnova
ne fait **pas** transiter uniformément le champ `idempotency_key` du
DTO vers cet en-tête aujourd'hui. Le comportement réel au moment de
l'écriture :

| Méthode | Champ DTO | Ce que fait l'adaptateur |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | Transmis dans le corps du POST comme `idempotency_key=...` (pas l'en-tête HTTP). L'API de Stripe ne lit **pas** les clés d'idempotence en formulaire, donc c'est à traiter comme inopérant jusqu'à ce que l'adaptateur migre vers le chemin de l'en-tête de requête. |
| `Payment::refund` | `RefundRequest::idempotency_key` | Silencieusement ignoré - le champ n'est pas transmis. |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | Silencieusement ignoré. |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | Silencieusement ignoré. |

Si vous comptez sur une sémantique au plus une fois pour les nouvelles
tentatives de charge/remboursement contre Stripe aujourd'hui, filtrez
la nouvelle tentative à votre propre site d'appel (une clé de domaine
déterministe persistée dans votre DB, avec un index unique empêchant
la seconde insertion) jusqu'à ce que l'adaptateur câble l'en-tête. Les
champs du DTO sont acceptés par l'API mais ne sont actuellement pas
honorés jusqu'à la requête réelle envoyée à Stripe - positionnez-les à
`None` en test et en production pour que la lacune soit explicite, et
ne présumez pas que Stripe déduplique vos nouvelles tentatives.

C'est une lacune connue de l'adaptateur v1 et un correctif candidat
pour la prochaine version ; la forme de la surface reste la même une
fois le câblage arrivé.

### Entrante : déduplication de webhook

L'idempotence de webhook est gérée par le framework côté entrée et est
entièrement câblée. Chaque événement atterrit dans
`payments_webhook_events` avec un index UNIQUE sur
`(provider, provider_event_id)`. Les livraisons dupliquées d'un
événement déjà traité retournent 200 à Stripe immédiatement sans
relancer l'hydratation ; les doublons d'un événement précédemment **en
échec** retentent l'hydratation, si bien que la nouvelle tentative du
fournisseur est votre mécanisme de reprise. Voir
[Idempotence](idempotency.md) pour le contrat complet d'audit et de
nouvelles tentatives.

## Tests

L'adaptateur est porté par hyper et exposé en façade par rustls. Les
tests qui construisent un `StripeProvider` ont besoin d'un fournisseur
de crypto enregistré ; on installe `ring` une seule fois dans
`#[cfg(test)]` :

```rust
#[cfg(test)]
mod tests {
    use suprnova_payments_stripe::StripeProvider;
    use std::sync::OnceLock;

    fn install_crypto_provider() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parses_subscription_webhook_ids() {
        let p = provider();
        let event = /* construire un WebhookEvent avec raw_payload */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

Pour les tests d'intégration qui frappent le sandbox Stripe réel,
positionnez `STRIPE_SECRET_KEY` et ses amies dans votre env de test.
Pour les tests unitaires de vos propres contrôleurs, préférez
`MockPaymentProvider` du framework - il implémente les cinq traits
avec des retours prévisibles et zéro réseau.

## Suivant

- [Paiements](payments.md) - la surface de traits, le registre, le
  motif d'amorçage, et le `SessionPayload` marqué par flow.
- [Paiements - Paddle](payments-paddle.md) - le pendant
  Merchant-of-Record ; les cinq mêmes traits, une répartition des
  responsabilités différente.
- [Paiements - Guide
  fournisseur](payments-provider-guide.md) - comment écrire un
  adaptateur pour une passerelle que Suprnova ne livre pas.
- [Paiements - Intégration du frontend](payments-frontend.md) - Svelte
  / React / Vue qui dispatchent sur `SessionPayload.flow`, y compris
  la boucle confirm-card-payment de Stripe.js.
- [Idempotence](idempotency.md) - le contrat d'audit et de nouvelles
  tentatives qui rend le traitement des webhooks sûr sous une
  livraison au moins une fois.
- [Eloquent](eloquent.md) - interrogez les tables miroir à côté de vos
  propres modèles ; tout n'est qu'une entité SeaORM.
