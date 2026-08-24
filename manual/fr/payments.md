# Paiements

La surface de paiement de Suprnova est neutre du point de vue du
fournisseur. Vous choisissez une crate d'adaptateur - Stripe, Paddle,
ou une que vous écrivez vous-même - vous l'enregistrez au démarrage,
et votre code métier appelle les quatre mêmes traits de base (plus un
cinquième, optionnel, pour la capture côté serveur), quel que soit le
fournisseur derrière. Les tables miroir de votre base de données sont
maintenues synchronisées par les webhooks, si bien que votre code
métier lit dans votre propre base plutôt que d'interroger l'API du
fournisseur à chaque requête.

Aucune fonctionnalité n'est réservée à un seul fournisseur. Le modèle
à capture directe de Stripe et le modèle Merchant-of-Record de Paddle
tiennent tous les deux dans le même contrat de traits. La seule
surface qui diffère est `Payment` (capture côté serveur), qui est
optionnelle - Paddle n'en a pas besoin, donc Paddle ne l'implémente
pas. Les fournisseurs annoncent leur capacité en surchargeant
`PaymentProvider::as_payment()` pour retourner `Some(&dyn Payment)` ;
les appelants interrogent au moment de l'exécution.

## Pourquoi Suprnova diverge

Laravel livre Cashier comme une intégration Stripe officielle,
présente dans sa documentation centrale. C'est pratique, mais réservé
à Stripe - ajouter un second fournisseur signifie forker Cashier ou
construire une surface parallèle. Suprnova traite les fournisseurs de
paiement comme il traite les drivers de cache et de stockage : un jeu
de traits générique, des adaptateurs interchangeables. Votre code
métier ne nomme jamais `StripeProvider` ni `PaddleProvider` ; il
appelle `provider.subscribe(...)` sur un `Arc<dyn PaymentProvider>`
résolu depuis un registre, et le fournisseur derrière n'est qu'à un
changement d'amorçage de devenir autre chose.

## Démarrage rapide

Ajoutez la crate d'adaptateur. Tant que Suprnova n'a pas livré sa
version v0.1, le framework et ses crates d'adaptateur se consomment
par git plutôt que par crates.io :

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1" }
```

Enregistrez le fournisseur et le routeur de webhooks à l'amorçage.
Le routeur de webhooks est un `Router` ordinaire que vous composez
dans votre `routes::register()` :

```rust,ignore
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_stripe::StripeProvider;

pub async fn register() {
    let stripe = StripeProvider::from_env().expect("Stripe env vars not set");
    PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
}
```

```rust,ignore
// src/routes.rs
use std::sync::Arc;
use suprnova::payments::webhook_routes;
use suprnova::container::App;
use suprnova::Router;
use sea_orm::DatabaseConnection;

/// `Application::routes(routes::register)` appelle ceci une fois à l'amorçage.
/// Nous partons du routeur de webhooks de paiement, puis nous empilons le
/// reste des routes de l'application par-dessus avec des appels
/// `.get(...)` / `.post(...)` ordinaires.
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ... le reste de vos routes ...
        .into()
}
```

`webhook_routes(db)` retourne un `Router` contenant seulement `POST
/webhooks/payments/{provider}`. Comme `Router::get` et
`Router::post` retournent chacun un `RouteBuilder` qui reconvertit
en `Router` via `.into()`, chaîner par-dessus le routeur de paiement
est la façon la plus directe de composer. Si vous utilisez déjà la
macro `routes!{}` pour vos routes ordinaires, déposez le POST de
webhook dans le même bloc - `webhook_routes` est une enveloppe de
commodité autour d'un seul appel `Router::new().post(...)`.

Dans votre contrôleur, récupérez le fournisseur, créez un client et
ouvrez une session de paiement :

```rust,ignore
// src/controllers/billing.rs
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("stripe")
        .ok_or_else(|| PaymentError::Internal("stripe not registered".into()))?;

    let customer = provider.create_customer(CreateCustomerRequest {
        user_id,
        email,
        name: None,
        metadata: None,
    }).await?;

    provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: customer.provider_customer_id,
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: None,
        metadata: None,
    }).await
}
```

Ce `SessionPayload` part dans les props de votre page Inertia. Le
frontend discrimine sur `payload.flow` pour afficher le bon widget -
voir [Paiements - Intégration du frontend](payments-frontend.md).

## Choisir un adaptateur

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1" }
```

Variables d'environnement requises :

| Variable | Description |
|---|---|
| `STRIPE_SECRET_KEY` | Clé secrète (`sk_live_…` / `sk_test_…`) |
| `STRIPE_PUBLISHABLE_KEY` | Clé publiable (`pk_live_…` / `pk_test_…`) |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | Secret de signature du point de terminaison de webhook (`whsec_…`) |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Depuis l'environnement (recommandé en production) :
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// Ou construisez directement :
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripe implémente tous les traits, y compris les traits optionnels
`Payment` (capture côté serveur via les PaymentIntents) et
`Promotions` (émission de codes promo via `/v1/promotion_codes`).
`provider.as_payment()` et `provider.as_promotions()` retournent
tous deux `Some`.

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1" }
```

Variables d'environnement requises :

| Variable | Description |
|---|---|
| `PADDLE_API_KEY` | Clé d'API (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) |
| `PADDLE_WEBHOOK_KEY` | Secret de la destination de notification (`pdl_ntfset_…`) |
| `PADDLE_CLIENT_TOKEN` | Jeton côté client (`live_…` / `test_…`) |
| `PADDLE_ENVIRONMENT` | Optionnelle, vaut `"sandbox"` par défaut |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Depuis l'environnement :
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// Ou construisez directement :
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddle est un Merchant of Record - il gère la taxe, les relances et
tout le cycle de vie des abonnements. Il n'expose pas de capture
côté serveur, donc `Payment` n'est pas implémenté. Appeler
`provider.as_payment()` retourne `None`. Les abonnements sont créés
indirectement : appelez `Checkout::start_session`, complétez le
widget Paddle, et le webhook `SubscriptionCreated` arrive pour
confirmer l'ID d'abonnement.

## La séparation des traits

`PaymentProvider` est un trait chapeau qui regroupe quatre traits
universels - `Checkout`, `Subscription`, `CustomerStore`,
`WebhookHandler` - que chaque adaptateur implémente. Deux traits
supplémentaires sont optionnels : `Payment` (la capture côté serveur
n'a de sens que pour des passerelles comme Stripe) et `Promotions`
(émission de codes promo). Les adaptateurs y adhèrent en surchargeant
`PaymentProvider::as_payment()` / `PaymentProvider::as_promotions()`.

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// Retourne `Some` si ce fournisseur implémente aussi `Payment` (server-capture).
    /// Retourne `None` par défaut.
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// Retourne `Some` si ce fournisseur implémente aussi `Promotions`
    /// (émission de codes promo). Retourne `None` par défaut.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - universel, ouvre le widget client

Chaque fournisseur implémente `Checkout`. Appelez `start_session` pour
obtenir un `SessionPayload` marqué par un flow, que votre frontend
affiche. `session_status` (par défaut : `NotSupported` ; surchargé par
les fournisseurs dont les sessions peuvent être interrogées, par ex.
Stripe) rapporte l'état côté fournisseur, faisant autorité, d'une
session que vous avez démarrée plus tôt.

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

Champs de `StartSessionRequest` :

| Champ | Type | Description |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` ou `Subscription` |
| `customer_ref` | `String` | Id client fournisseur venant de `CustomerStore::create_customer` |
| `price_refs` | `Vec<String>` | Ids de prix/produit du fournisseur |
| `success_return_url` | `String` | Où renvoyer l'utilisateur après le paiement |
| `cancel_return_url` | `String` | Où renvoyer l'utilisateur s'il abandonne |
| `amount_hint` | `Option<Money>` | Override ou indication pour les montants ponctuels |
| `idempotency_key` | `Option<String>` | Pour des nouvelles tentatives sûres |

`session_status` est la primitive de vérification côté serveur pour
les flux de redirection. Quand le client revient sur votre page de
retour, ne faites PAS confiance aux paramètres de requête portés par
son navigateur - passez le `provider_session_id` que vous avez
enregistré au moment de `start_session` et branchez sur le résultat :

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // Honorez la commande. `payment_ref` (par ex. le `pi_…` de Stripe)
        // fait le lien avec les opérations `Payment` et le miroir payments_transactions.
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* règlement en attente */ }
    CheckoutSessionState::Open => { /* le client n'a pas fini de payer */ }
    CheckoutSessionState::Expired => { /* session expirée - fermez la commande */ }
}
```

Le même appel alimente les balayages de réconciliation : réinterrogez
les commandes encore ouvertes dans votre base de données et concluez
celles dont la session s'est complétée après que le client a fermé
l'onglet.

### `Payment` - optionnel, capture côté serveur

Seuls les fournisseurs qui exposent une capture côté serveur
implémentent `Payment`. Stripe le fait ; Paddle non. Pour vérifier au
moment de l'exécution :

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(payment) = provider.as_payment() {
    let result = payment.charge(ChargeRequest {
        customer_ref: "cus_...".into(),
        payment_method_ref: "pm_...".into(),
        amount: Money::from_minor_units(2999, Currency::USD),
        description: Some("Pro plan one-off".into()),
        idempotency_key: Some("charge_user42_order99".into()),
        metadata: None,
    }).await?;
}
```

Interface complète de `Payment` :

```rust,ignore
#[async_trait]
pub trait Payment: Send + Sync {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult>;
    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult>;
    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult>;
    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()>;
    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus>;
}
```

`ChargeResult` est une enum marquée par `kind` - voir la section
[Money et ChargeResult](#chargeresult).

### `Promotions` - optionnel, émettre des codes promo

Les fournisseurs qui ont une surface de codes promo implémentent
`Promotions`. L'objet de remise lui-même (un coupon en pourcentage ou
en montant fixe) est créé à l'avance - typiquement une fois, dans le
tableau de bord du fournisseur - et ce trait émet des *codes* à partir
de lui, chacun restreint à un client et à une fenêtre de rédemption.
C'est la forme dont ont besoin les campagnes de reconquête et de
montée en gamme : chaque destinataire reçoit un code personnel,
inutilisable par quiconque d'autre et mort une fois la fenêtre fermée.

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // coupon pré-créé
        customer_ref: "cus_...".into(),             // seul ce client peut le rédimer
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // usage unique
    }).await?;
    // Envoyez `minted.code` par email au client ; il le saisit au checkout et
    // le fournisseur applique toutes les restrictions.
}
```

`MockPaymentProvider` implémente `Promotions` (les codes s'émettent
sous la forme `PROMO_MOCK_n`) et enregistre chaque requête - faites
vos assertions sur `recorded_promotion_requests()` dans les tests.

### `Subscription` - s'abonner, mettre à jour, annuler, récupérer

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

Annuler à la fin de la période (garde l'accès jusqu'à la fin du cycle
de facturation) :

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// Annulation immédiate :
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

Remarque : `Paddle::subscribe` retourne
`PaymentError::NotSupported` - Paddle crée les abonnements par la
complétion d'un checkout, pas par des appels API directs. Utilisez
`Checkout::start_session` et attendez le webhook
`SubscriptionCreated`.

### `CustomerStore` - créer, mettre à jour, récupérer, supprimer

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` prend `user_id`, `email`,
`name: Option<String>`, et `metadata: Option<Value>`. `CustomerRef`
revient avec `provider_customer_id` - conservez-le à côté de votre
enregistrement utilisateur pour l'utiliser dans les appels suivants.

### `WebhookHandler` - vérifier, analyser et extraire

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// Tire les ids d'entité du payload brut pour que le framework sache
    /// quelles lignes miroir hydrater. Retourne un `PayloadIds` vide par défaut.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// Construit un `PaymentSnapshot` à partir d'un événement de paiement /
    /// facture. Retourne `None` par défaut, ce qui saute l'upsert de `payments_transactions`.
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// Construit un `CustomerSnapshot` à partir d'un événement client.
    /// Retourne `None` par défaut, ce qui saute le rafraîchissement de
    /// l'email / des métadonnées sur la ligne existante.
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

En pratique vous n'appelez jamais aucune de ces méthodes
directement - `webhook_routes` les invoque pour chaque webhook
entrant. Elles vivent sur le trait pour que les crates d'adaptateur
puissent implémenter la vérification de signature, l'analyse
d'événement et l'extraction de payload spécifiques au fournisseur, de
façon testable. Les méthodes `extract_*` ont toutes des défauts
raisonnables ; les adaptateurs Stripe et Paddle livrés les surchargent
avec des implémentations sensibles à la forme du fournisseur (Stripe
va chercher dans `data.object.*`, Paddle dans `data.*`).

## Le payload Inertia marqué par flow

`start_session` retourne une enum `SessionPayload` qui se sérialise en
JSON avec un champ discriminant `flow`. Votre frontend commute sur
`flow` pour afficher le bon widget :

```rust,ignore
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum SessionPayload {
    StripeElements {
        client_secret: String,
        publishable_key: String,
        provider_session_id: String,
    },
    StripeCheckoutRedirect {
        url: String,
        provider_session_id: String,
    },
    PaddleInline {
        transaction_id: String,
        customer_token: Option<String>,
        client_token: String,
    },
    /// Flow Mobile Money - pas de redirection ni d'intégration embarquée.
    /// Le frontend affiche un message destiné au client lui demandant de
    /// confirmer sur son téléphone (prompt USSD ou app opérateur), puis
    /// interroge le fournisseur via `provider_transaction_id` pour les mises à jour de statut.
    MobileMoneyPrompt {
        provider_transaction_id: String,
        message: String,
        operator: MobileMoneyOperator,
    },
    Redirect {
        url: String,
        provider_session_id: String,
    },
}
```

Forme sérialisée d'un payload `StripeElements` :

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

Un payload `MobileMoneyPrompt` ressemble à ceci - il n'y a pas d'URL
parce que le client ne quitte jamais votre page ; le frontend affiche
`message` et se met à interroger :

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

Retournez, depuis votre contrôleur, quel que soit le variant produit
par le fournisseur, comme props Inertia. L'intégration frontend est
décrite dans [Paiements - Intégration du
frontend](payments-frontend.md).

## Tables miroir

Six tables sont créées par la migration du framework. Importez l'alias
public et incluez-le dans le migrateur de votre application :

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... vos autres migrations ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

Le même module exporte aussi un helper
`pub fn migrations() -> Vec<Box<dyn MigrationTrait>>` si vous préférez
l'appeler et répandre le résultat dans votre propre liste.

### Vue d'ensemble des tables

| Table | Rôle |
|---|---|
| `payments_customers` | Une ligne par paire `(provider, user_id)` |
| `payments_payment_methods` | Moyens de paiement stockés par client |
| `payments_subscriptions` | État du cycle de vie de l'abonnement |
| `payments_subscription_items` | Lignes composant un abonnement |
| `payments_transactions` | Charges ponctuelles et factures d'abonnement |
| `payments_webhook_events` | Journal d'audit et garde-fou d'idempotence |

Chaque table a une colonne JSON `provider_metadata`. Quand la
représentation neutre du framework ne couvre pas un champ spécifique
au fournisseur, lisez-le depuis là.

### Table des transactions

`payments_transactions` scinde les montants en `amount_total_minor` et
`amount_tax_minor`. Stripe rapporte des montants hors taxe - la taxe
est à zéro sur la ligne de transaction, et toute donnée de taxe vit
dans `provider_metadata`. Paddle rapporte des montants taxe incluse et
positionne `amount_tax_minor` sur la composante de taxe. Les deux
représentations fonctionnent ; additionnez
`amount_total_minor - amount_tax_minor` pour obtenir le montant net.

### Table des événements de webhook

`payments_webhook_events` a un index
`UNIQUE(provider, provider_event_id)`. Chaque webhook entrant est
vérifié contre celui-ci avant traitement - les doublons retournent 200
OK sans être retraités. C'est structurant : Stripe, Paddle, et la
plupart des fournisseurs relancent agressivement les webhooks en
échec.

### Mises en garde

Le code métier lit depuis les tables miroir, pas directement depuis
l'API du fournisseur. Les mutations (créer un abonnement, annuler,
etc.) partent vers le fournisseur ; le webhook qui en résulte
resynchronise les tables miroir en retour. Cela signifie qu'il existe
une brève fenêtre entre une mutation et l'arrivée du webhook pendant
laquelle vos tables miroir sont en retard. Concevez votre UX en tenant
compte de cela (affichez des états « en cours », appuyez-vous sur les
URL de redirection du fournisseur pour une confirmation immédiate).

## Gestion des webhooks

Montez la route d'entrée des webhooks une fois à l'amorçage - voir
l'exemple de routes de [Démarrage rapide](#démarrage-rapide) pour le
motif de composition. `webhook_routes(db)` retourne un `Router`
portant l'unique handler `POST /webhooks/payments/{provider}` intégré
au framework. Vous chaînez vos propres routes par-dessus (ou appelez
directement les primitives sous-jacentes de la route à l'intérieur de
votre propre bloc `routes!{}`).

Le handler du framework fait ceci pour chaque requête :

1. Recherche le fournisseur nommé dans `PaymentProviderRegistry`.
2. Appelle `WebhookHandler::verify` pour vérifier la signature.
   Retourne 401 en cas d'échec.
3. Appelle `WebhookHandler::parse_event` pour construire un
   `WebhookEvent`. Retourne 400 en cas d'échec d'analyse.
4. Vérifie `payments_webhook_events` pour une ligne existante avec le
   même `(provider, provider_event_id)`. Si elle est trouvée, retourne
   200 immédiatement - c'est le garde-fou d'idempotence.
5. Insère la ligne d'audit.

### Structure de WebhookEvent

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // chaîne brute du fournisseur, p. ex. "customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // mappé sur la taxonomie du framework, ou None si spécifique au fournisseur
    pub raw_payload: Value,                 // corps JSON complet pour le fallthrough
}
```

`NeutralEventKind` couvre le chemin courant :

```rust,ignore
pub enum NeutralEventKind {
    PaymentSucceeded,
    PaymentFailed,
    PaymentRefunded,
    PaymentDisputed,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    InvoicePaid,
    InvoiceFailed,
    CustomerCreated,
    CustomerUpdated,
}
```

Quand `neutral` vaut `None`, l'événement est spécifique au
fournisseur. Lisez `provider_event_type` et `raw_payload` pour la
donnée complète.

### Hydratation des tables miroir

Une fois la ligne d'audit persistée, le framework dispatche
l'événement vers la table miroir concernée selon `neutral`. **Toutes
les écritures miroir d'un même événement ont lieu dans une seule
transaction DB, avec `mark_processed`** - un état miroir partiel n'est
jamais observable. Soit tout est validé ensemble, soit tout est
annulé.

| `NeutralEventKind`               | Effet miroir                                                                                       |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated`    | Appelle `Subscription::get(id)` sur le fournisseur, upsert `payments_subscriptions`, synchronise les lignes.       |
| `SubscriptionCanceled`           | Comme ci-dessus ; positionne aussi `canceled_at` et fait passer `status` à `canceled` sur la ligne existante.        |
| `PaymentSucceeded / Failed / Refunded / Disputed` | Upsert `payments_transactions` à partir de l'instantané que le fournisseur produit depuis `raw_payload`.        |
| `InvoicePaid / InvoiceFailed`    | Upsert `payments_transactions` avec `provider_subscription_id` lié.                              |
| `CustomerCreated / CustomerUpdated` | Met à jour `email` / `provider_metadata` de la ligne `payments_customers` existante, à partir du `CustomerSnapshot` du fournisseur. **N'insère jamais.**   |
| `None` (non mappé)                | Ligne d'audit seulement - aucun changement du miroir.                                                                   |

Le miroir client est délibérément en mise à jour seule sur le chemin
webhook. `user_id` est `NOT NULL` et seule l'app sait à quel
utilisateur appartient un client fournisseur (le lien est créé par
votre code juste après `CustomerStore::create_customer`). Les clients
créés hors bande - créés dans le tableau de bord Stripe, disons - sont
journalisés mais jamais synthétisés dans le miroir.

### Contrat de reprise sur échec

Le handler traite les nouvelles tentatives du fournisseur comme le
mécanisme de reprise :

- **L'hydratation réussit :** la transaction est validée,
  `processed_at` est positionné, `process_error` est effacé. Réponse :
  `200 ok`.
- **L'hydratation échoue :** la transaction est annulée (aucun état
  miroir partiel), la ligne d'audit garde `processed_at = NULL` et
  `process_error` enregistre l'échec. Réponse :
  `503 hydration-failed` - le fournisseur relancera avec un backoff.
- **Le fournisseur relance l'événement en échec :** la vérification
  d'idempotence voit la ligne d'audit existante mais
  `processed_at IS NULL`, donc l'hydratation s'exécute à nouveau. La
  nouvelle tentative remplace le `process_error` périmé par le
  résultat de la tentative en cours.
- **Le fournisseur relance un événement réussi :** la vérification
  d'idempotence voit `processed_at IS NOT NULL`, retourne
  `200 duplicate` immédiatement. Pas de nouvelle hydratation.

Un événement d'abonnement/client avec un `subscription_id` /
`customer_id` manquant dans le payload est traité comme une erreur
`Validation` (également 503 + `process_error` enregistré). Un succès
silencieux sur un payload malformé laisserait le miroir périmé sans
visibilité pour l'opérateur.

Les éléments retirés d'un abonnement côté fournisseur (par ex.
l'utilisateur abandonne un module complémentaire de siège) sont
retirés de `payments_subscription_items` quand le prochain webhook
`subscription.updated` arrive. La réponse `Subscription::get(id)` du
fournisseur est la source de vérité à chaque synchronisation.

## Moyens de paiement au-delà des cartes

`PaymentMethod` est l'enum que le framework utilise pour les moyens
stockés dans `payments_payment_methods` et pour tout fournisseur qui
expose des métadonnées de moyen de paiement. Elle couvre les cas
évidents - cartes, virements bancaires, e-wallets - plus des moyens
régionaux qui sont de premier rang sur de nombreux marchés :

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// Payeur identifié par téléphone + opérateur + pays.
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// Crypto indexée - équivalent cash pour la plupart des fournisseurs.
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// Cryptomonnaie non indexée.
    Crypto { network: String, address: String },
    /// Échappatoire pour les moyens régionaux / spécifiques au fournisseur pas encore modélisés.
    Custom { kind: String, descriptor: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMoneyOperator {
    MtnMomo,
    Mpesa,
    AirtelMoney,
    OrangeMoney,
    Lipila,
    Custom { identifier: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StablecoinAsset {
    Usdc,
    Usdt,
    Dai,
    Custom { ticker: String },
}
```

Les opérateurs et actifs nommés sont ceux que nous avons énumérés. Les
variants `Custom { ... }` de chacun couvrent les opérateurs régionaux
et les stablecoins que nous n'avons pas encore fixés, si bien
qu'ajouter le support de l'un d'eux ne force pas une release du
framework.

`PhoneNumber` et `CountryCode` sont des DTO validés dans
`suprnova::payments` - ils rejettent une entrée malformée au moment de
la construction, ce qui est là où vous voulez que l'échec se produise
plutôt qu'à l'appel du fournisseur.

## Montants

Les montants sont représentés par `Money` - un compteur `i64` en unité
mineure plus une `Currency`. Aucun `f64` impliqué.

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// Depuis des unités mineures (cents, pence, yen, etc.)
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// Depuis une chaîne décimale
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// Devises sans décimale - 1234 mineur = 1234 JPY (pas de conversion)
let yen = Money::from_minor_units(1234, Currency::JPY);

// Arithmétique - panique sur une incompatibilité de devise
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// Les valeurs négatives représentent des remboursements ou des crédits
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// Relire
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` et `Sub` paniquent sur une incompatibilité de devise et sur un
débordement `i64`. Utilisez l'arithmétique paniquante pour la
correction - une addition silencieuse entre devises est un bug, pas
une fonctionnalité.

## ChargeResult

`Payment::charge` retourne une enum `ChargeResult`. Toute charge ne se
complète pas immédiatement - les cartes en step-up 3DS et hors-session
peuvent nécessiter une redirection ou une action côté client :

```rust,ignore
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChargeResult {
    Completed {
        provider_transaction_id: String,
        amount: Money,
        status: PaymentStatus,
        provider_metadata: Value,
    },
    RedirectRequired {
        provider_transaction_id: String,
        url: String,
        return_to: Option<String>,
    },
    RequiresClientAction {
        provider_transaction_id: String,
        action_kind: String,
        client_secret: Option<String>,
        publishable_key: Option<String>,
    },
}
```

Gérez `RequiresClientAction` en retournant le payload à votre
frontend. Le frontend affiche le défi 3DS en utilisant `client_secret` +
`publishable_key`. Voir [Paiements - Intégration du
  frontend](payments-frontend.md) pour le code de dispatch côté
  frontend.

## Clés d'idempotence

Chaque DTO de mutation a un `idempotency_key: Option<String>`
optionnel. Positionnez-en une sur les appels réseau qui peuvent être
retentés :

```rust,ignore
provider.start_session(StartSessionRequest {
    // ...
    idempotency_key: Some(format!("checkout_{}_{}", user_id, order_id)),
    // ...
}).await?;

provider.subscribe(SubscribeRequest {
    // ...
    idempotency_key: Some(format!("sub_{}_{}", user_id, plan_id)),
    // ...
}).await?;
```

Stripe honore les clés d'idempotence via l'en-tête HTTP
`Idempotency-Key`. Paddle a un mécanisme équivalent. Si une requête
échoue en plein vol et que vous retentez avec la même clé, le
fournisseur retourne la réponse d'origine au lieu de créer une charge
ou un abonnement dupliqué.

## Le motif du discriminant

Chaque adaptateur qui prétend implémenter `PaymentProvider` doit
passer le même flux E2E :

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

`MockPaymentProvider`, livré avec le framework, le passe :

```rust,ignore
use suprnova::payments::*;

#[tokio::test]
async fn discriminator_flow() {
    let provider = MockPaymentProvider::new();

    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "user_42".into(),
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        metadata: None,
    }).await.unwrap();

    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("idem_1".into()),
        metadata: None,
    }).await.unwrap();
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        trial_days: None,
        idempotency_key: Some("idem_2".into()),
        metadata: None,
    }).await.unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // Annuler à la fin de la période
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // Annuler immédiatement
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProvider omet délibérément Payment (optionnel façon Paddle)
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` n'implémente pas `Payment` - cela exerce le même
invariant que Paddle. `StripeProvider` et `PaddleProvider` passent
tous les deux le même flux contre l'API réelle dans les tests
d'intégration.

## Applications multi-fournisseurs

Enregistrez les deux adaptateurs au démarrage et dispatchez selon
l'endroit où l'enregistrement de chaque client a été créé :

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// Plus tard, par requête :
let provider_name = user.payment_provider.as_str(); // "stripe" ou "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

Usages courants : router les clients européens via Paddle (pour la
gestion de la taxe en MoR) et les clients américains via Stripe ;
tester en A/B la conversion au checkout entre fournisseurs ; utiliser
un fournisseur pour les abonnements et un autre pour les charges
ponctuelles.

## Migration depuis Laravel Cashier

Cashier est réservé à Stripe par conception. Suprnova livre le
multi-fournisseur d'origine. Correspondance rapide :

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await` (si le fournisseur le supporte) |
| `$invoice->download()` | Non intégré ; lisez `provider_metadata["invoice_pdf_url"]` depuis la table miroir des transactions |

## Suivant

- [Paiements - Adaptateur Stripe](payments-stripe.md) - le flux de
  passerelle en détail : PaymentIntents, format de signature des
  webhooks, correspondance des types d'événement
- [Paiements - Adaptateur Paddle](payments-paddle.md) - le flux MoR en
  détail : création d'abonnement pilotée par le checkout, gestion de
  la taxe, vérification des notifications
- [Paiements - Intégration du
  frontend](payments-frontend.md) - exemples de dispatch sur flow pour
  Svelte 5, React 19 et Vue 3.5
- [Écrire un adaptateur de fournisseur de
  paiement](payments-provider-guide.md) - construisez votre propre
  crate d'adaptateur de bout en bout
- [Base de données](database.md) - la couche SeaORM sur laquelle
  reposent les tables miroir
