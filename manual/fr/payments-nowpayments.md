# Paiements - NOWPayments

L'adaptateur `suprnova-payments-nowpayments` crée des factures hébergées,
vérifie les notifications de paiement et lit le statut d'un paiement à
l'aide de la clé d'API marchand. Il s'enregistre sous le nom
`nowpayments` dans le registre de fournisseurs de paiement habituel.

## Installation et configuration

Utilisez une révision de Suprnova qui contient cet adaptateur pour les
deux dépendances. Pour une application locale à côté d'un checkout des
sources de Suprnova :

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

Configurez ces valeurs dans l'environnement protégé de l'application :

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

L'environnement accepte `sandbox` ou `production` et vaut `sandbox` par
défaut. Un environnement inconnu ou vide fait échouer la configuration.
Des identifiants vides échouent avant toute requête HTTP. Utilisez des
identifiants distincts pour chaque environnement.

Enregistrez le fournisseur au démarrage, en propageant une erreur de
configuration :

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

Ajoutez les migrations de paiement et composez `webhook_routes(db)` dans
le routeur de l'application comme le montre
[l'aperçu des paiements](payments.md). Le point d'entrée est
`POST /webhooks/payments/nowpayments`. Gardez cette route authentifiée
par le fournisseur en dehors du middleware CSRF des sessions
navigateur. Configurez le callback avec l'URL HTTPS publique exacte ;
les corps de requête doivent parvenir à l'adaptateur sans modification.

## Démarrer une facture hébergée

Persistez une tentative de checkout avec une référence de commande
marchand unique avant d'émettre la requête. Le montant provient de la
commande de confiance de l'application, jamais d'un total fourni par le
navigateur.

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` accepte la même requête et retourne le
`SessionPayload::Redirect` générique. Stockez son
`provider_session_id` comme **identifiant de facture**, puis redirigez
vers son URL validée. La méthode concrète `create_invoice` retourne en
plus la référence de commande marchand et distingue les erreurs de
création incertaines.

L'adaptateur de facture exige le mode paiement unique, des références
client et prix vides, et un montant `Money` positif dont l'exposant de
devise fiduciaire est défini. Il envoie le montant décimal exact en
unité principale et le code de devise en minuscules. Les clients
choisissent une cryptomonnaie sur la page hébergée, sauf si
`pay_currency` est fourni.

Les métadonnées forment un objet strict avec ces champs :

| Champ | Signification |
| --- | --- |
| `order_id` | Référence marchand obligatoire et non vide, 128 octets au plus |
| `order_description` | Description optionnelle, 500 octets au plus |
| `pay_currency` | Ticker fournisseur optionnel en minuscules, par exemple `btc` |
| `is_fixed_rate` | Réglage optionnel du taux de change du fournisseur |
| `is_fee_paid_by_user` | Réglage optionnel des frais du fournisseur |

Les clés de métadonnées inconnues sont rejetées. Ne placez pas de
données applicatives ou client arbitraires dans cet objet. Les URL de
retour doivent utiliser la même origine HTTPS que le callback, sans
identifiants ni fragment. Les redirections de facture doivent viser
l'environnement NOWPayments sélectionné et contenir l'identifiant de
facture retourné.

## Échecs de création et reprises

Le point d'entrée de facturation de NOWPayments ne documente aucune
garantie de clé d'idempotence. L'adaptateur rejette un
`idempotency_key` différent de `None` ; `order_id` est une donnée de
corrélation, pas une déduplication côté fournisseur. Ni les
redirections HTTP ni les reprises automatiques ne sont activées. Les
requêtes ont une échéance de 30 secondes et une limite de réponse de
64 KiB.

`InvoiceCreationError::Rejected` représente une validation d'entrée ou
un rejet net de l'API. `Unknown` signifie qu'une facture existe
peut-être déjà : par exemple, la requête a expiré, le fournisseur a
retourné une erreur serveur, ou sa réponse de succès était invalide.
Marquez cette tentative comme incertaine et rapprochez la commande dans
le tableau de bord du fournisseur avant de créer une autre facture. Ne
placez pas la création de facture dans une boucle de reprise
générique. Via `Checkout::start_session`, un résultat inconnu est un
`PaymentError::Provider` porteur de cette instruction de récupération.

## Vérifier et rapprocher les paiements

Une facture peut produire un identifiant de paiement distinct.
`payment_status(payment_id)` appelle le point d'entrée de paiement
authentifié et rejette une réponse portant un autre identifiant.
`Checkout::session_status(invoice_id)` retourne `NotSupported` ; un
identifiant de facture ne peut pas être substitué dans le point
d'entrée de consultation de paiement. Cet adaptateur n'utilise pas les
identifiants e-mail et mot de passe du tableau de bord pour lister les
paiements par facture.

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

L'identifiant de paiement doit provenir d'un IPN vérifié ou d'un autre
enregistrement côté serveur de confiance. Un retour navigateur ne prouve
pas le paiement. Une consultation réussie n'est pas non plus un contrôle
d'autorisation : rapprochez-la de la commande, de la facture, du montant
et de la devise stockés avant de modifier un accès. `price` est le prix
fiduciaire demandé, pas la quantité de crypto reçue. Passez en revue les
réglages d'acceptation des paiements partiels du marchand ; cet
adaptateur ne convertit jamais `partially_paid` en succès.

| État fournisseur | Événement neutre |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`, inconnu | Aucune classification neutre |

Les signatures d'IPN utilisent un JSON trié récursivement et HMAC
SHA-512. La vérification emploie une comparaison de MAC à temps
constant. Les signatures absentes, dupliquées ou invalides, les clés
JSON dupliquées, les charges utiles surdimensionnées et les champs
obligatoires malformés sont rejetés. Le fournisseur ne signe aucun
horodatage de livraison indépendant, il n'y a donc pas de fenêtre de
rejeu par horodatage inventée. La protection contre le rejeu s'appuie
sur des reçus persistés.

Les IPN de NOWPayments n'ont pas d'identifiant d'événement indépendant.
L'adaptateur identifie un reçu par identifiant de paiement et par
statut, si bien qu'une livraison répétée avec un `updated_at` différent
ne répète pas le même événement terminal. Le framework stocke les
événements vérifiés dans `payments_webhook_events` et réessaie les
traitements échoués. Un événement en attente arrivé tardivement ne
modifie pas une table miroir de transaction déjà réglée.

### Factures sans client et état applicatif

L'API de facturation ne fournit pas l'identité client qu'exigent les
tables miroir de transactions de Suprnova. Cet adaptateur retourne
`false` depuis `WebhookHandler::mirrors_payment_transactions` : tous les
événements vérifiés, remboursements compris, restent dans le journal
d'audit, mais il ne crée aucune table miroir de client ni de
transaction. Stripe et Paddle conservent le comportement miroir par
défaut.

La route webhook générique enregistre et traite les événements du
fournisseur ; elle n'honore pas les commandes de l'application. Une
tâche de rapprochement de l'application devrait consommer les reçus
vérifiés, lire le statut de paiement courant, et mettre à jour sa
commande dans une transaction idempotente. Stockez une clé d'exécution
unique par fournisseur et paiement, ou par commande, pour que les
reprises et les livraisons désordonnées ne puissent pas accorder l'accès
deux fois. Suivez l'état d'achèvement durable propre à la tâche
séparément du `processed_at` du framework. Utilisez l'état authentifié
courant pour les événements tardifs et traitez les remboursements selon
la politique de l'application.

## Limites de capacités

Les opérations de `CustomerStore` et `Subscription` retournent
`NotSupported`. `as_payment()` et `as_promotions()` retournent `None`.
Cet adaptateur n'implémente ni soldes en conservation, ni facturation
récurrente, ni checkout direct par adresse de dépôt, ni versements, ni
déclenchement de remboursement, ni gestion des clients chez le
fournisseur. NOWPayments propose des produits distincts pour certaines
de ces opérations ; cet adaptateur de facturation ne prétend pas les
implémenter.

### Pourquoi Suprnova diverge

Les traits de fournisseur communs restent disponibles, mais les
opérations non supportées échouent explicitement. Les factures sans
client fournisseur s'appuient sur des notifications auditées et des
commandes détenues par l'application plutôt que sur des enregistrements
client ou des abonnements fabriqués.

## Vérification

Lancez les tests de l'adaptateur et les régressions de paiement
partagées depuis le checkout des sources :

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

La suite locale utilise de fausses réponses HTTP, une fixture de
signature JavaScript générée de façon indépendante, et une véritable
entrée du framework avec SQLite. Elle couvre le checkout réussi, les
échecs d'authentification, les réponses malformées ou volumineuses, les
expirations, les redirections non fiables, les signatures invalides, la
correspondance des statuts, les livraisons dupliquées, et la reprise
après une panne de base de données. Les fixtures locales n'établissent
aucune interopérabilité avec le fournisseur.

Avant d'activer la production, utilisez un compte sandbox NOWPayments et
un callback HTTPS joignable. Créez une facture, exercez les cas de
succès, partiels et d'échec proposés par le fournisseur, confirmez que
sa signature est acceptée, et rapprochez l'identifiant de paiement de la
facture et de la commande stockées. Rejouez une livraison et confirmez
que la clé d'exécution de l'application empêche un second octroi. Les
tests locaux n'impliquent aucune exécution en sandbox ou en production
sur un compte réel.

Références fournisseur : [points d'entrée de l'API](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description),
[authentification IPN](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)
et le [SDK officiel](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs).

## Suivant

Voir [Intégration frontend](payments-frontend.md) pour le rendu des
payloads de redirection, ou le
[Guide du fournisseur](payments-provider-guide.md) pour les contrats
partagés.
