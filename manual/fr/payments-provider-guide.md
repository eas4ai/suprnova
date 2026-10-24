# Écrire un adaptateur de fournisseur de paiement

Ce guide parcourt la construction d'une crate d'adaptateur
tierce - `suprnova-payments-mollie` - qui se branche sur la surface de
paiement neutre du point de vue du fournisseur de Suprnova. À la fin,
vous aurez une crate qui s'enregistre elle-même, passe le flux du
discriminant, et peut être intégrée à n'importe quelle app Suprnova
avec un simple `cargo add`.

La même structure s'applique à n'importe quel fournisseur : Square,
Braintree, Adyen, ou tout autre service avec une API HTTP.

### Pourquoi Suprnova diverge

Laravel livre Cashier comme une intégration Stripe officielle. C'est
excellent pour le chemin Stripe, mais cela codifie le vocabulaire d'un
seul fournisseur dans le framework - ajouter un second fournisseur
signifie soit forker Cashier, soit construire une surface parallèle à
côté.

Suprnova garde chaque fournisseur sur le même contrat à cinq traits :
`Checkout`, `Subscription`, `CustomerStore`, `WebhookHandler`, et le
`Payment` optionnel pour les fournisseurs à capture côté serveur. Le
code métier ne détient jamais que `Arc<dyn PaymentProvider>` depuis le
registre. Échanger Stripe pour Paddle (ou pour l'adaptateur Mollie que
vous allez écrire) est un changement d'amorçage, pas un changement de
code. Les adaptateurs de référence à
`crates/suprnova-payments-stripe/` et
`crates/suprnova-payments-paddle/` prouvent que le contrat de traits
tient pour deux modèles commerciaux très différents - passerelle à
capture directe et Merchant of Record - et votre adaptateur s'insère
dans la même forme.

## 1. Créer la crate membre du workspace

Depuis la racine du dépôt :

```bash
cargo new --lib crates/suprnova-payments-mollie
```

Ajoutez-la à votre `Cargo.toml` racine :

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # ajoutez cette ligne
]
```

(Les adaptateurs de référence - `crates/suprnova-payments-stripe` et
`crates/suprnova-payments-paddle` - vivent dans ce même répertoire
`crates/` et sont de bons modèles à lire à côté de ce guide.)

**`crates/suprnova-payments-mollie/Cargo.toml` :**

```toml
[package]
name = "suprnova-payments-mollie"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Mollie payment adapter for Suprnova"

[dependencies]
suprnova = { path = "../../framework" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
inventory = "0.3"
tracing = "0.1"
tokio = { version = "1", features = ["macros", "rt"] }
# Votre SDK Mollie :
mollie-rs = "0.1"
hmac = "0.12"   # pour la vérification HMAC du webhook
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. Disposer les fichiers source

Reprenez la structure utilisée par les adaptateurs livrés :

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # struct MollieProvider, impl PaymentProvider, from_env
├── checkout.rs     # impl Checkout
├── customer.rs     # impl CustomerStore
├── subscription.rs # impl Subscription
├── webhook.rs      # impl WebhookHandler
├── event_map.rs    # chaîne d'événement du fournisseur → NeutralEventKind
└── payment.rs      # impl Payment (si Mollie supporte la capture côté serveur)
```

## 3. `lib.rs` - la struct du fournisseur

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{Payment, PaymentProvider};

mod checkout;
mod customer;
mod event_map;
mod payment;
mod subscription;
mod webhook;

pub use event_map::mollie_event_to_neutral;

/// Adaptateur Mollie pour la surface de paiement neutre du point de vue
/// du fournisseur de Suprnova.
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Clé API Mollie (`test_…` / `live_…`).
    api_key: String,
    /// Secret de signature du webhook - utilisé dans la vérification HMAC.
    webhook_secret: String,
    /// Client HTTP - partagé entre les requêtes.
    client: reqwest::Client,
}

impl MollieProvider {
    pub fn new(api_key: impl Into<String>, webhook_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            webhook_secret: webhook_secret.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Construit depuis les variables d'environnement.
    ///
    /// Lit :
    /// - `MOLLIE_API_KEY`
    /// - `MOLLIE_WEBHOOK_SECRET`
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("MOLLIE_API_KEY")
            .map_err(|_| "MOLLIE_API_KEY not set".to_string())?;
        let webhook_secret = std::env::var("MOLLIE_WEBHOOK_SECRET")
            .map_err(|_| "MOLLIE_WEBHOOK_SECRET not set".to_string())?;
        Ok(Self::new(api_key, webhook_secret))
    }
}

impl PaymentProvider for MollieProvider {
    fn name(&self) -> &'static str {
        "mollie"
    }

    // Ne surchargez `as_payment()` que si vous implémentez aussi `Payment` (server-capture).
    // L'impl par défaut sur `PaymentProvider` retourne `None` - omettez cette
    // surcharge entièrement si Mollie est checkout-only / façon MoR.
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` est le trait chapeau - la clause de supertrait est
`Checkout + Subscription + CustomerStore + WebhookHandler`, donc le
compilateur refusera de lier votre fournisseur jusqu'à ce que les
quatre soient implémentés. Le cinquième trait, `Payment`, est
**optionnel** - seuls les fournisseurs qui exposent une capture côté
serveur l'implémentent, et `as_payment()` en rapporte le résultat au
framework. Le `as_payment()` par défaut retourne `None`, donc omettez
entièrement la surcharge si votre fournisseur ne fait pas de capture
côté serveur.

## 4. Implémenter les quatre traits requis

### `checkout.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    Checkout, PaymentError, PaymentResult, SessionMode, SessionPayload, StartSessionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Checkout for MollieProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        // Appelez l'API Mollie pour créer un paiement ou une commande.
        // Mappez la réponse sur l'un des variants de SessionPayload.
        // Mollie utilise des pages de checkout hébergées, donc Redirect est le choix naturel.
        let checkout_url = self.create_mollie_payment(&req).await
            .map_err(|e| PaymentError::Internal(format!("Mollie checkout error: {e}")))?;

        Ok(SessionPayload::Redirect {
            url: checkout_url,
            provider_session_id: "mollie_session_id_here".into(),
        })
    }
}

impl MollieProvider {
    async fn create_mollie_payment(&self, req: &StartSessionRequest) -> Result<String, mollie_rs::Error> {
        // Câblez l'appel au SDK Mollie ici.
        // Retournez l'URL de checkout hébergée.
        todo!("Mollie payment creation")
    }
}
```

### `customer.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::MollieProvider;

#[async_trait]
impl CustomerStore for MollieProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        // POST /v2/customers vers Mollie
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        // PATCH /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        // GET /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        // DELETE /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

### `subscription.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    PaymentError, PaymentResult, SubscribeRequest, Subscription, SubscriptionResult,
    UpdateSubscriptionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Subscription for MollieProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        // POST /v2/customers/{id}/subscriptions
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        // PATCH /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // Positionner la date d'annulation à la fin de la période
        } else {
            // DELETE /v2/customers/{id}/subscriptions/{sub_id}
        }
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        // GET /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

Si votre fournisseur ne supporte pas une méthode, retournez
`PaymentError::NotSupported` :

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - capture côté serveur (optionnel)

Implémentez ceci seulement si votre fournisseur supporte des charges
directes côté serveur contre un moyen de paiement stocké. Retirez la
surcharge `as_payment()` dans `lib.rs` si vous sautez cette étape.

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    ChargeRequest, ChargeResult, Payment, PaymentError, PaymentResult, PaymentStatus,
    RefundRequest, RefundResult,
};

use crate::MollieProvider;

#[async_trait]
impl Payment for MollieProvider {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult> {
        // POST /v2/payments/{id}/refunds
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

## 5. Mapper les événements du fournisseur sur `NeutralEventKind`

**`event_map.rs` :**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// Mappe une chaîne de type d'événement webhook Mollie sur la taxonomie
/// neutre du framework. Retourne `None` pour les événements spécifiques
/// au fournisseur qui n'ont pas d'équivalent neutre.
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Paiements Mollie
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Abonnements Mollie
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Commandes/factures Mollie
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // Événements client
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // Spécifique au fournisseur - retombe sur raw_payload
        _                       => None,
    }
}
```

Couvrez au minimum les événements listés ci-dessus. Pour tout
événement hors de la taxonomie neutre, retournez `None` - il est
malgré tout persisté dans `payments_webhook_events` sous
`provider_event_type` + `raw_payload`, si bien que le code métier peut
le lire.

## 6. Implémenter la vérification de signature de webhook

**`webhook.rs` :**

Mollie signe les payloads de webhook avec HMAC-SHA256. Comparez
toujours les signatures en temps constant pour empêcher les attaques
temporelles.

```rust,ignore
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    NeutralEventKind, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{MollieProvider, event_map::mollie_event_to_neutral};

type HmacSha256 = Hmac<Sha256>;

#[async_trait]
impl WebhookHandler for MollieProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // Lisez l'en-tête de signature que Mollie envoie.
        // Nom d'en-tête exact et schéma de signature - vérifiez la doc de Mollie pour votre version.
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // Calculez le HMAC-SHA256 attendu sur le corps brut.
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // Décodez la signature reçue, encodée en hex.
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // Comparaison en temps constant.
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // Mollie envoie du JSON - analysez-le.
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // Mollie utilise des types de ressource plutôt que des chaînes de type
        // d'événement dans certaines formes de webhook. Adaptez selon ce que
        // votre version du SDK envoie.
        let event_type = raw["resource"].as_str()
            .unwrap_or("unknown")
            .to_string();

        let neutral = mollie_event_to_neutral(&event_type);

        Ok(WebhookEvent {
            provider: "mollie".into(),
            provider_event_id: event_id,
            provider_event_type: event_type,
            neutral,
            raw_payload: raw,
        })
    }
}
```

Points clés :

- `PaymentError::WebhookSignature(String)` est le variant unique pour
  toute défaillance de signature - en-tête manquant, encodage
  malformé, non-correspondance. La route de webhook du framework
  traite chaque `WebhookSignature(_)` comme un 401.
- Utilisez `PaymentError::Validation(String)` pour les corps
  imparsables. La route de webhook retourne 400 sur tout échec
  d'analyse.
- Le handler `webhook_routes` du framework appelle `verify` avant
  `parse_event`, puis hydrate à l'intérieur d'une transaction DB. Les
  échecs d'hydratation retournent 503 pour que le fournisseur relance.
- Ne journalisez jamais le secret brut ni la signature reçue.

### Hydratation des tables miroir : `extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

Après que `parse_event` a retourné un `WebhookEvent`, la route de
webhook du framework hydrate les tables miroir. Trois méthodes de
trait optionnelles pilotent cela - toutes ont des implémentations par
défaut sûres et sans effet, si bien qu'un adaptateur peut être livré
sans elles et passer malgré tout par la couche d'audit :

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` est le pont entre l'événement analysé et la logique de
miroir du framework. Implémentez-le pour que le framework puisse
trouver la bonne entité :

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

Pour chaque valeur `neutral`, peuplez les ids que le payload du
fournisseur expose. Les événements d'abonnement devraient positionner
`subscription_id` pour que le framework puisse appeler
`Subscription::get(id)` et rafraîchir le miroir depuis l'état
canonique. Les événements client positionnent `customer_id`. Les
événements de paiement / facture positionnent `transaction_id`, plus
`subscription_id` quand c'est une charge récurrente.

`PaymentSnapshot` est construit directement depuis le payload du
webhook - il n'y a pas de callback `Payment::get`. Implémentez-le pour
les neutres de paiement / facture :

```rust,ignore
pub struct PaymentSnapshot {
    pub provider_transaction_id: String,
    pub provider_customer_id: String,
    pub provider_subscription_id: Option<String>,
    pub amount_total_minor: i64,
    pub amount_tax_minor: i64,
    pub currency: String,
    pub status: String,             // "succeeded" | "failed" | "refunded" | "disputed"
    pub paid_at: Option<DateTime<Utc>>,
    pub provider_metadata: Value,   // typiquement l'objet entité provenant du payload
}
```

L'implémentation de référence de Stripe lit
`data.object.{id,amount,currency,customer}` pour les événements
`PaymentIntent`/`Charge` et
`data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}`
pour les événements `Invoice`. Celle de Paddle lit
`data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}`.
Reprenez les conventions qui correspondent à la forme du payload de
votre fournisseur - le framework ne se soucie pas de la façon dont
vous extrayez, seulement du fait que l'instantané soit correct.

Si vous retournez `None` depuis `extract_payment_snapshot`, la ligne
d'audit est malgré tout écrite mais `payments_transactions` n'est pas
touchée. C'est le bon retour pour les événements d'abonnement /
client, ou pour tout événement de paiement où le payload ne porte pas
assez d'information pour peupler une ligne.

`CustomerSnapshot` garde la synchronisation du miroir client pilotée
par le fournisseur (pas de chemin JSON codé en dur dans le
framework) :

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

Le framework ne fera `email = Set(snapshot.email)` que quand
l'instantané en fournit un ; `provider_metadata` est toujours remplacé
par la vue du fournisseur sur le client (`updated_at` est aussi bumpé
dans tous les cas). Les lignes du miroir client ne sont jamais que
**mises à jour** - jamais insérées - parce que `user_id` est
`NOT NULL` et que l'app possède le lien utilisateur ↔ client via
`CustomerStore::create_customer`.

### Sémantique d'échec

Si `extract_payload_ids` retourne `None` pour `subscription_id` sur un
événement d'abonnement (ou pour `customer_id` sur un événement
client), le framework traite cela comme une erreur `Validation` : la
transaction d'hydratation est annulée, le `process_error` de la ligne
d'audit est positionné, et la réponse HTTP est **503
hydration-failed** pour que le fournisseur relance. Un succès
silencieux sur un payload malformé laisserait le miroir périmé sans
visibilité pour l'opérateur - les nouvelles tentatives du fournisseur
sont le mécanisme de reprise.

Ce contrat signifie que l'extracteur d'un adaptateur doit peupler
honnêtement les ids concernés. Retourner `None` est réservé aux
événements que votre fournisseur ne peut traduire du tout (par ex. un
événement de paiement sans id de charge dans le payload), pas à « je
n'ai pas pris la peine d'analyser celui-là ».

## 7. S'enregistrer au démarrage de l'app

Deux mécanismes sont disponibles - choisissez-en un :

### Enregistrement au runtime (recommandé pour les apps à config par variable d'env)

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### Enregistrement à la compilation via `inventory`

Pour les crates d'adaptateur qui veulent un enregistrement zéro
config - utile quand vous livrez une bibliothèque que les
consommateurs se contentent de `cargo add` sans aucun câblage au
démarrage :

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// Dans lib.rs, dans un initialiseur statique :
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` s'exécute avant `main`. La closure de fabrique
est appelée une fois, quand le registre est accédé pour la première
fois.

## 8. Passer le test du discriminant

Chaque crate d'adaptateur devrait inclure un test d'intégration qui
prouve que le contrat de traits est correct de bout en bout. C'est la
preuve de solidité - si ce test passe, le fournisseur se branche sur
n'importe quelle app Suprnova sans surprise.

```rust,ignore
// tests/discriminator.rs (inside crates/suprnova-payments-mollie/)

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// Exige que MOLLIE_API_KEY et MOLLIE_WEBHOOK_SECRET soient positionnées.
/// Lancez avec : cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. Créer le client
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. Démarrer la session de checkout
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("discriminator_test_checkout".into()),
        metadata: None,
    }).await.expect("start_session failed");
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    // 3. S'abonner directement (si votre fournisseur le supporte ; Mollie peut exiger un checkout)
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. Relire
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. Annuler à la fin de la période
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. Annuler immédiatement
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. Vérifier l'invariant as_payment()
    let p: &dyn PaymentProvider = &provider;
    // Si vous avez implémenté Payment : assert!(p.as_payment().is_some())
    // Si vous n'avez PAS implémenté Payment : assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

Filtrez les tests d'intégration en direct avec `#[ignore]` pour que
`cargo test` passe en CI sans identifiants. Lancez-les explicitement
avec `-- --ignored` contre un compte sandbox.

## 9. Référence des variants de `PaymentError`

L'enum complète vit dans `framework/src/payments/error.rs`. Choisissez
le variant qui correspond à ce qui s'est réellement passé :

| Variant | Quand l'utiliser |
|---|---|
| `Provider(String)` | L'API du fournisseur a retourné une erreur que vous n'avez pas besoin de traduire davantage |
| `Validation(String)` | Les champs de la requête sont invalides, ou un corps de webhook ne s'analyse pas |
| `NotSupported(String)` | La méthode ne s'applique pas à ce fournisseur (par ex. `subscribe` de Paddle) |
| `Declined { reason, decline_code }` | Carte refusée - transmettez `decline_code` quand le fournisseur en fournit un |
| `Authentication(String)` | Le fournisseur a rejeté votre clé API ou vos identifiants |
| `NotFound(String)` | L'id de client, d'abonnement, ou de transaction n'existe pas |
| `WebhookSignature(String)` | Toute défaillance de signature - en-tête manquant, encodage malformé, ou non-correspondance |
| `InvalidPhoneNumber(String)` | La validation E.164 a échoué dans les flux mobile money |
| `InvalidCountryCode(String)` | La validation ISO-3166-1 alpha-2 a échoué |
| `Internal(String)` | Erreur SDK inattendue, panne réseau, échec d'init HMAC, ou tout autre problème côté framework |

La route de webhook mappe ces variants sur des codes de statut :
`WebhookSignature(_)` → 401, `Validation(_)` depuis `parse_event` →
400, tout le reste depuis l'hydratation → 503 (pour que le fournisseur
relance).

Une fois que votre adaptateur compile et que le test du discriminant
passe :

- Ajoutez votre crate au `Cargo.toml` de votre app avec
  `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie`.
- Enregistrez-la à l'amorçage comme montré à l'étape 7.
- Montez `webhook_routes(db.clone())` une fois au démarrage de
  l'app - le même handler dispatche vers chaque fournisseur enregistré
  par nom, si bien qu'un seul montage sert Stripe, Paddle, et votre
  nouvel adaptateur.

## Suivant

- [Paiements](payments.md) - la surface neutre du point de vue du
  fournisseur et le Démarrage rapide
- [Paiements - Adaptateur Stripe](payments-stripe.md) - modèle complet
  pour un adaptateur de passerelle
- [Paiements - Adaptateur Paddle](payments-paddle.md) - modèle complet
  pour un adaptateur Merchant-of-Record
- [Paiements Frontend](payments-frontend.md) - comment afficher le
  `SessionPayload` que votre adaptateur retourne
- [Modèle d'erreur](error-model.md) - comment `PaymentError` atterrit
  comme une `HttpResponse`
