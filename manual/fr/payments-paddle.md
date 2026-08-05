# Paiements - Adaptateur Paddle

L'adaptateur Paddle (`suprnova-payments-paddle`) câble Paddle dans la
surface de paiement générique de Suprnova. Tournez-vous vers lui quand
vous voulez un fournisseur de paiement qui gère aussi pour vous la
taxe de vente, la TVA, la GST, les relances, la facturation, et les
remboursements - Paddle est un Merchant of Record (MoR), ce qui
signifie qu'il est le vendeur légal auprès de vos clients et absorbe
la surface de conformité qu'une passerelle à capture directe comme
Stripe vous laisse.

Ce choix change le modèle mental. Votre code métier ne *possède* pas
l'abonnement - Paddle le possède. Vous ouvrez un checkout, le client
le complète, et le webhook `SubscriptionCreated` vous indique que
l'abonnement existe désormais. Vous ne pouvez pas créer un abonnement
via l'API, et vous ne pouvez pas échanger son jeu de prix après coup.
Vous pouvez annuler, vous pouvez lire l'état, vous pouvez mettre à
jour les métadonnées de facturation. Le reste appartient à Paddle.

Ce chapitre suppose que vous avez lu [Paiements](payments.md) pour la
surface générique à cinq traits. Ici on couvre ce qui est vrai
*seulement* pour Paddle.

## Quand choisir Paddle

Choisissez Paddle quand une ou plusieurs de ces conditions sont
vraies :

- Vous vendez des produits numériques à l'échelle mondiale et la
  conformité fiscale (TVA, GST, taxe de vente US) est un coût réel sur
  votre feuille de route.
- Vous ne voulez pas gérer vous-même les nouvelles tentatives de
  paiement en échec, les emails de relance, ou l'émission des reçus.
- Vous voulez une facture unique d'un seul vendeur légal pour la
  comptabilité.
- Votre modèle d'affaires est subscription-first, et vous acceptez que
  le fournisseur pilote le cycle de vie de l'abonnement.

Choisissez [Stripe](payments.md#stripe) à la place quand vous voulez
un contrôle direct sur la capture des charges, que vous gérez votre
propre taxe, ou que vous avez besoin d'appels
`charge`/`capture`/`refund` côté serveur depuis vos propres chemins de
code.

## Configuration

Ajoutez la crate :

```bash
cargo add suprnova-payments-paddle
```

Positionnez les quatre variables d'environnement :

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| Variable | Ce qu'elle est | D'où elle vient |
|---|---|---|
| `PADDLE_API_KEY` | Clé API côté serveur (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) | Tableau de bord Paddle → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | Secret de la destination de notification (`pdl_ntfset_…`) | Tableau de bord Paddle → Developer Tools → Notifications → votre endpoint |
| `PADDLE_CLIENT_TOKEN` | Jeton sûr pour le navigateur (`live_…` / `test_…`) | Tableau de bord Paddle → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox` (défaut) ou `production` | Votre choix |

Enregistrez le fournisseur à l'amorçage. Les deux formes sont
valides :

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // Depuis l'env (recommandé) :
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // Ou construisez directement :
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

La route d'entrée des webhooks est enregistrée par le helper
`webhook_routes(db.clone())` du framework - voir
[Paiements](payments.md#webhook-handling). `from_env()` et `new()`
retournent tous les deux un `Result` parce que le
`paddle_rust_sdk::Paddle::new` sous-jacent valide la forme de la clé
API et l'URL de l'endpoint au moment de la construction.

## Le modèle mental du MoR

La forme qui surprend les utilisateurs de Stripe :

```
Stripe (passerelle) :
    votre app  ─────────►  Stripe  ──►  réseau carte
        │                     ▲
        └────── webhook ──────┘
    vous possédez l'état de l'abonnement dans votre DB ; Stripe est l'exécutant

Paddle (Merchant of Record) :
    votre app  ─►  lien checkout  ─►  client  ──►  Paddle  ──►  réseau carte
                                                      │
       ◄─────────────────  webhook  ──────────────────┘
    Paddle possède l'état de l'abonnement ; votre DB est le miroir
```

En code, la différence apparaît à trois endroits :

1. **Vous ne pouvez pas créer un abonnement via l'API.** Appelez
   `Checkout::start_session` avec un prix récurrent ; le client
   complète le widget Paddle ; le webhook `SubscriptionCreated`
   hydrate votre miroir.
2. **Vous ne pouvez pas échanger le jeu de prix d'un abonnement via
   l'API.** Paddle réserve les changements de plan à son propre
   tableau de bord ou aux flux de migration qu'il possède.
3. **Vous ne pouvez pas supprimer un client.** Archiver via update est
   le contournement supporté.

Suprnova remonte ces contraintes comme `PaymentError::NotSupported`
plutôt que de les masquer - voir la [matrice des
capacités](#matrice-des-capacités) ci-dessous.

## Flux de checkout

`Checkout::start_session` est le seul moyen de démarrer un paiement
avec Paddle. Le frontend ouvre le `transaction_id` résultant avec
paddle.js en utilisant le `client_token` que vous avez positionné à
l'amorçage :

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. Créez le client dans Paddle (ou réutilisez-en un existant).
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. Ouvrez une session de checkout. Paddle dispatche ponctuel vs abonnement
    //    sur le *type de prix*, pas sur le champ SessionMode ci-dessous.
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // ignoré par Paddle (voir la remarque)
        customer_ref: cus.provider_customer_id,
        price_refs: vec!["pri_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some(format!("checkout_{user_id}")),
        metadata: None,
    }).await?;

    Ok(session)
}
```

Le `SessionPayload::PaddleInline` retourné porte tout ce dont le
frontend a besoin :

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Voir [Paiements - Intégration du frontend](payments-frontend.md) pour
le code de montage paddle.js en Svelte / React / Vue.

### Paddle dispatche sur le type de prix, pas sur `SessionMode`

Un vrai piège spécifique à Paddle : le champ `SessionMode::OneOff` /
`SessionMode::Subscription` sur `StartSessionRequest` est **ignoré par
l'adaptateur Paddle**. L'API de Paddle a un unique endpoint
`transaction_create`, et le fournisseur inspecte les ids de prix
fournis pour déduire le flow - un prix récurrent démarre un
abonnement, un prix ponctuel démarre une charge unique. Avec Stripe,
c'est le champ qui pilote le flow ; avec Paddle, c'est le *prix*.
Configurez votre catalogue Paddle avec les bons types de prix avant de
pointer l'adaptateur vers eux.

## Les abonnements arrivent par webhook

Parce que Paddle possède le cycle de vie de l'abonnement, votre code
métier ne fait qu'*apprendre* l'existence d'un abonnement quand Paddle
vous le dit. Le flux :

```
votre app                       Paddle                    client
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │   termine le checkout    │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   subscription.created webhook                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 tables miroir hydratées ;        │                          │
 ligne payments_subscriptions     │                          │
 a provider_subscription_id       │                          │
```

Le handler `webhook_routes(db)` du framework fait l'hydratation pour
vous : il appelle `WebhookHandler::extract_payload_ids` pour trouver
le `subscription_id`, appelle `Subscription::get(id)` pour lire l'état
canonique, et upsert `payments_subscriptions` +
`payments_subscription_items` à l'intérieur d'une seule transaction.
Au moment où le webhook retourne 200, votre miroir est cohérent avec
Paddle.

Il existe une brève fenêtre entre le moment où le client complète le
widget et l'arrivée du webhook, pendant laquelle
`payments_subscriptions` n'a aucune ligne pour le nouvel abonnement.
Deux motifs la couvrent :

- **Utilisez l'URL de redirection pour une UX immédiate.**
  `success_return_url` se déclenche côté client dès que Paddle
  confirme la transaction, si bien que vous pouvez afficher «
  Abonnement actif » sans attendre le webhook côté serveur.
- **Interrogez-et-affichez.** Après la redirection, rafraîchissez la
  page après un court délai pour que le contrôleur Inertia puisse lire
  le miroir désormais hydraté.

## Matrice des capacités

Toutes les méthodes de tous les traits ne font pas ce que fait leur
équivalent Stripe. Le tableau ci-dessous est la vérité. `subscribe()`
et `update()` avec `new_price_refs.is_some()` sont les seules méthodes
qui échouent *toujours* ; le reste fonctionne, avec les mises en garde
notées.

| Méthode de trait | Comportement |
|---|---|
| `Checkout::start_session` | Fonctionne. Dispatche ponctuel vs abonnement sur le type de prix, pas sur `SessionMode`. |
| `Subscription::subscribe` | Toujours `NotSupported`. Les abonnements naissent de la complétion du checkout + webhook. |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | Fonctionne. Se câble à `subscription_cancel` avec le défaut `EffectiveFrom::NextBillingPeriod`. |
| `Subscription::update(new_price_refs: Some(...))` | `NotSupported` en v1. Paddle réserve le remplacement de jeu de prix à ses propres flux de migration. |
| `Subscription::update` (no-op) | Fonctionne. Re-récupère l'état courant via `subscription_get`. |
| `Subscription::cancel` | Fonctionne, mais `at_period_end` est **ignoré** - planifie toujours à la prochaine période de facturation. Voir [ci-dessous](#l-annulation-est-toujours-planifiée). |
| `Subscription::get` | Fonctionne. |
| `CustomerStore::create_customer` | Fonctionne. |
| `CustomerStore::update_customer` | Fonctionne. |
| `CustomerStore::get_customer` | Fonctionne. |
| `CustomerStore::delete_customer` | `NotSupported`. Utilisez `update_customer` avec le statut `archived` si besoin. |
| `Payment::*` | Le trait n'est pas implémenté. `provider.as_payment()` retourne `None`. |
| `WebhookHandler::*` | Fonctionne. |

Les invariants - `Payment` non implémenté,
`subscribe`/`delete_customer` retournant `NotSupported`, et le rejet
de signature de webhook - sont fixés par des tests toujours actifs
dans `crates/suprnova-payments-paddle/tests/integration.rs`, si bien
que la matrice ci-dessus ne dérivera pas silencieusement.

### L'annulation est toujours planifiée

`Subscription::cancel(id, at_period_end)` accepte le bool pour la
compatibilité du trait mais **se comporte toujours comme une
annulation planifiée** - l'enum `EffectiveFrom` de Paddle est privée
dans `paddle_rust_sdk` 0.18, donc l'annulation immédiate n'est pas
viable en v1. L'utilisateur garde l'accès jusqu'à la fin de la période
de facturation courante, moment auquel Paddle déclenche
`subscription.canceled` et le miroir fait passer `status` à
`Canceled`.

Si vous voulez un « annuler maintenant » au niveau UX qui révoque
l'accès à l'app immédiatement tout en laissant Paddle éteindre la
facturation en arrière-plan, filtrez l'accès sur votre propre flag
`subscription.status != Canceled && subscription.cancel_at_period_end == false`
et mettez à jour l'UI juste après que `cancel()` retourne - le
prochain webhook confirmera.

### La suppression client, c'est « archiver via update »

`delete_customer` retourne `PaymentError::NotSupported` parce que
l'API publique de Paddle n'expose aucun endpoint de suppression. Si
vous devez supprimer un enregistrement client dans Paddle, appelez
`update_customer` avec le statut `archived`. L'adaptateur du framework
n'encapsule pas cela directement - le champ metadata est
l'échappatoire :

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

Confirmez le chemin de champ exact contre votre version de l'API
Paddle avant de livrer ceci - le SDK ne modélise pas directement
l'enum `status` actuellement.

## Vérification de signature de webhook

Paddle signe chaque webhook avec HMAC. L'en-tête `Paddle-Signature`
ressemble à `ts=1716000000,h1=abcdef…`. L'adaptateur délègue la
vérification à `Paddle::unmarshal` du SDK, qui :

- Analyse l'en-tête
- Recalcule le HMAC en utilisant votre `PADDLE_WEBHOOK_KEY`
- Rejette les signatures dont le timestamp est hors de
  `MaximumVariance::default()` (5 secondes au moment de
  l'écriture - les rejeux plus anciens que cela sont abandonnés)

Le handler `webhook_routes` du framework appelle `verify` avant tout
le reste ; un échec retourne `401 invalid-signature` sans fuite de
corps. Vous n'écrivez rien de ce code vous-même, mais il est utile de
savoir que la vérification est HMAC + tolérance de timestamp, pas une
comparaison de secret statique.

## Forme du payload de webhook

Les méthodes `extract_payload_ids`, `extract_payment_snapshot`, et
`extract_customer_snapshot` de l'adaptateur connaissent la forme du
payload de Paddle pour que le framework puisse hydrater les tables
miroir. Correspondance rapide :

| event_type de webhook | `NeutralEventKind` | Effet miroir |
|---|---|---|
| `transaction.completed`, `transaction.paid` | `PaymentSucceeded` | Upsert `payments_transactions` |
| `transaction.payment_failed` | `PaymentFailed` | Upsert `payments_transactions` (en échec) |
| `transaction.billed` | `InvoicePaid` | Upsert `payments_transactions` avec `provider_subscription_id` lié |
| `adjustment.created`, `adjustment.updated` | `PaymentRefunded` | Upsert `payments_transactions` (remboursé) |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → upsert `payments_subscriptions` + lignes |
| `subscription.updated`, `.activated`, `.paused`, `.resumed`, `.trialing` | `SubscriptionUpdated` | Comme ci-dessus |
| `subscription.canceled` | `SubscriptionCanceled` | Idem ; positionne `canceled_at`, fait basculer le statut |
| `customer.created` | `CustomerCreated` | Mise à jour seule : rafraîchit `email`/`metadata` si la ligne miroir existe |
| `customer.updated` | `CustomerUpdated` | Idem |
| tout le reste | `None` (non mappé) | Ligne d'audit seulement - aucun changement du miroir |

Paddle place l'objet entité directement sous `data` (pas `data.object`
comme Stripe). Les montants arrivent comme des **chaînes d'unités
mineures** (`"1234"` = 12,34 dans l'unité majeure), pas des
décimaux - l'adaptateur analyse les deux formes, chaîne et numérique,
pour la compatibilité future. La devise arrive comme `currency_code`,
en minuscules, et l'instantané la met en majuscules.

### Montants taxe incluse

Paddle rapporte les montants de transaction **taxe incluse**. Le
miroir `payments_transactions` du framework scinde cela :

- `amount_total_minor` - le montant total payé par le client (taxe
  incluse)
- `amount_tax_minor` - la composante de taxe

Le net de taxe est `amount_total_minor - amount_tax_minor`. Cela
diffère de Stripe (qui rapporte hors taxe avec
`amount_tax_minor = 0`). Le code qui additionne les revenus à travers
les deux fournisseurs doit être conscient de la taxe :

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## Création de client

`CreateCustomerRequest` se mappe directement sur le `customer_create`
de Paddle :

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // id utilisateur de votre app
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // non transmis à Paddle en v1
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

Stockez `cus.provider_customer_id` à côté de votre enregistrement
utilisateur. Chaque appel suivant (démarrer un checkout, chercher un
abonnement, etc.) prend l'id client Paddle, pas l'id utilisateur de
l'app. La table miroir `payments_customers` porte les deux colonnes,
si bien qu'une seule recherche par index vous donne l'une ou l'autre
direction.

`update_customer` et `get_customer` transitent vers les méthodes SDK
équivalentes. `update_customer` accepte les mises à jour `email` /
`name` et retourne le `CustomerRef` rafraîchi. `get_customer` récupère
un instantané depuis Paddle (pas depuis le miroir) - utilisez-le quand
vous avez besoin d'une lecture fraîche après un changement hors bande
dans le tableau de bord Paddle.

## La forme intentionnelle de `NotSupported`

Un lecteur peu familier avec la base de code pourrait supposer que
`PaymentError::NotSupported` sur `subscribe()` et `delete_customer()`
est un TODO différé. Ce n'est pas le cas. Les contraintes font partie
de la surface produit de Paddle, et Suprnova les encode plutôt que de
simuler des mutations locales que le fournisseur n'honorera jamais.

Chaque message d'erreur `NotSupported` pointe vers le flux supporté :

- `subscribe` : « use `Checkout::start_session` with
  `SessionMode::Subscription` and await the `SubscriptionCreated`
  webhook »
- `update` avec `new_price_refs` : « Paddle price-set replacement on
  existing subscription not in v1 »
- `delete_customer` : « use `UpdateCustomer` with `archived` status »

Branchez explicitement sur cette erreur quand vous écrivez du code
métier agnostique du fournisseur :

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* chemin Stripe */ }
    Err(PaymentError::NotSupported(_)) => {
        // Chemin Paddle - archiver via update à la place
        provider.update_customer(UpdateCustomerRequest {
            provider_customer_id: cus_id,
            email: None,
            name: None,
            metadata: Some(serde_json::json!({ "status": "archived" })),
        }).await?;
    }
    Err(e) => return Err(e),
}
```

### Pourquoi Suprnova diverge

Laravel Cashier est réservé à Stripe et modélise les abonnements comme
possédés par l'app :
`$user->newSubscription('default', 'pri_pro')->create()` est formé
comme si l'application initiait l'abonnement. Avec une passerelle à
capture directe, c'est exact. Avec un MoR, c'est un mensonge - le
fournisseur est l'acteur, pas votre app.

La surface de paiement de Suprnova est neutre du point de vue du
fournisseur, donc elle ne prend pas parti. La surface de traits
(`subscribe`, `update`, `cancel`, `get`) est la forme générique ;
chaque adaptateur implémente ce que son fournisseur expose et retourne
`NotSupported` là où le modèle produit du fournisseur diffère.
L'adaptateur Stripe implémente `subscribe`. L'adaptateur Paddle ne le
fait pas, parce que Paddle ne le lui permet pas. Masquer la différence
derrière un faux « create » local ferait mentir
l'adaptateur - Suprnova préfère le `NotSupported` typé avec un message
de migration dans la chaîne d'erreur.

La même divergence s'applique à `Payment` (capture côté serveur).
Stripe l'implémente ; Paddle non, et `provider.as_payment()` retourne
`None`. Le code qui a besoin de charge/capture/remboursement doit
vérifier `as_payment().is_some()` plutôt que d'appeler à
l'aveugle - voir
[Paiements](payments.md#payment--optional-server-side-capture).

## Tester votre intégration

La crate inclut des tests d'invariants toujours actifs (aucun accès
réseau nécessaire) plus un test d'intégration conditionné par l'env
contre l'API sandbox de Paddle :

```bash
# Invariants toujours actifs (rejet de signature, formes NotSupported) :
cargo test -p suprnova-payments-paddle

# Plus intégration sandbox (exige PADDLE_API_KEY etc.) :
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

Les tests d'invariants sont ceux à reprendre dans votre propre code si
vous construisez des abstractions spécifiques à l'adaptateur. Trois
formes de test qui valent la peine d'être copiées :

```rust
use suprnova::payments::*;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

#[test]
fn paddle_does_not_implement_payment_trait() {
    let p = PaddleProvider::new(
        "pdl_sdbx_apikey_test",
        "pdl_ntfset_test",
        "test_client",
        PaddleEnvironment::Sandbox,
    ).expect("provider construction");
    assert!(p.as_payment().is_none());
}

#[tokio::test]
async fn paddle_subscribe_returns_not_supported() {
    let p = /* ...comme ci-dessus... */;
    let err = p.subscribe(SubscribeRequest {
        customer_ref: "ctm_test".into(),
        price_refs: vec!["pri_test".into()],
        trial_days: None,
        idempotency_key: None,
        metadata: None,
    }).await.unwrap_err();
    assert!(matches!(err, PaymentError::NotSupported(_)));
}

#[test]
fn webhook_verify_rejects_bad_signature() {
    let p = /* ...comme ci-dessus... */;
    let mut headers = http::HeaderMap::new();
    headers.insert("paddle-signature", "ts=1234,h1=deadbeef".parse().unwrap());
    let ctx = WebhookContext {
        body: b"{}",
        headers: &headers,
        remote_addr: None,
    };
    assert!(matches!(p.verify(&ctx).unwrap_err(), PaymentError::WebhookSignature(_)));
}
```

Pour des tests locaux de bout en bout sans jamais frapper Paddle, le
framework livre `MockPaymentProvider`. Comme Paddle, le `as_payment()`
du mock retourne `None` (pas de capture côté serveur), si bien que le
code qui branche sur `as_payment().is_some()` suit le même chemin sous
le mock que sous Paddle. Le `subscribe()` du mock retourne `Ok`
(contrairement à Paddle), donc les tests qui doivent vérifier la
branche `NotSupported` devraient utiliser le vrai `PaddleProvider`.
Liez le mock dans les tests plutôt que le vrai fournisseur :

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ...exercez votre contrôleur contre le mock...
}
```

## Checklist de production

Avant de basculer `PADDLE_ENVIRONMENT=production` :

- [ ] Les quatre variables d'env sont positionnées dans les secrets de
  production, pas commitées
- [ ] L'URL de l'endpoint de webhook est enregistrée dans les réglages
  *Notifications* du tableau de bord Paddle, et le secret de
  destination que vous y avez généré correspond à `PADDLE_WEBHOOK_KEY`
- [ ] Le catalogue a des ids de prix live (pas sandbox), et les ids
  que vous référencez dans `price_refs` existent dans le catalogue
  live
- [ ] Vos `success_return_url` et `cancel_return_url` pointent vers
  des endpoints HTTPS (Paddle rejette le HTTP en production)
- [ ] Vous avez décidé comment votre app répond quand `subscribe()`,
  `delete_customer()`, ou `update(price_refs)` retournent
  `NotSupported` - soit en branchant dans le code, soit en documentant
  que ces flux sont réservés au MoR
- [ ] Vous avez testé sous stress l'UX d'annulation : l'annulation est
  toujours planifiée, donc « vous avez annulé mais vous avez encore
  accès jusqu'à DATE » est le message que votre UI devrait afficher
- [ ] Vous avez testé sous stress le webhook d'arrivée d'abonnement :
  il existe une fenêtre où le client a payé mais le miroir n'a encore
  aucune ligne
- [ ] Vous agrégez le revenu correctement : les montants Paddle sont
  taxe incluse, les montants Stripe sont hors taxe

## Suivant

- [Paiements](payments.md) - la surface générique à cinq traits et le
  contrat d'hydratation du miroir du handler de webhook
- [Paiements - Intégration du
  frontend](payments-frontend.md) - checkout inline paddle.js en
  Svelte / React / Vue
- [Paiements - Guide
  fournisseur](payments-provider-guide.md) - écrivez votre propre
  crate d'adaptateur de bout en bout
- [Configuration](configuration.md) - l'enregistrement de config typée
  dans lequel les variables d'env Paddle se branchent
- [Amorçage de l'application](bootstrap.md) - où
  `PaymentProviderRegistry::bind` vit réellement dans votre app
