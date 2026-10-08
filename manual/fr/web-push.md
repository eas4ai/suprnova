# Web Push

Web Push livre un message court à un navigateur même quand votre site
est fermé - le Service Worker se réveille, déchiffre le payload, et
affiche une notification au niveau de l'OS. Suprnova livre le
protocole de bout en bout : génération de clé VAPID, chiffrement de
payload AES128GCM, le transport HTTP, et un `WebPushChannel` qui se
branche dans le sous-système de notifications, si bien que la même
`Notification` que vous envoyez au mail ou à la base de données
atterrit aussi comme un push.

Tournez-vous vers ceci quand vous voulez alerter les utilisateurs en
temps réel sans WebSocket ouvert - commande expédiée, demande d'ami,
mention, solde crédité. Si l'utilisateur est sur un navigateur de
bureau avec le site fermé, le push web est le seul mécanisme qui les
atteint ; s'ils sont sur le site, [Diffusion](broadcasting.md) est
généralement plus adapté.

L'API est derrière la feature Cargo `web-push`, activée par défaut.
Les applications utilisant `default-features = false` doivent activer
`web-push` explicitement.

## Les quatre éléments

Web Push a plus de pièces mobiles que le mail ou la base de données,
parce que la spec ([RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)) répartit
l'identité, le chiffrement, et le transport sur trois contrats :

| Élément | Ce que c'est |
|---|---|
| `VapidKey` / `VapidSigner` | Une paire de clés ECDSA P-256 utilisée pour signer des JWT qui prouvent que votre serveur est bien celui qu'il prétend être |
| `WebPushClient` | Le client HTTP qui chiffre un payload, signe un JWT VAPID, et POST vers le point de terminaison de l'abonnement |
| `WebPushChannel` | L'adaptateur du sous-système de notifications qui transforme une `Notification` en un appel `WebPushClient::send` |
| `SubscriptionInfo` | Le triplet opaque (`endpoint`, `p256dh`, `auth`) que le navigateur vous remet quand un utilisateur s'abonne - vous le stockez ; vous ne le générez pas |

Les trois couches du bas - `VapidKey`, `WebPushClient`, le POST
chiffré - sont réexportées depuis `suprnova::web_push`, si bien que
les applications n'ont jamais besoin de dépendre directement de la
crate sous-jacente `suprnova-web-push`.

## Générer une paire de clés VAPID

Web Push utilise VAPID (Voluntary Application Server Identification)
pour permettre aux services push de limiter le débit et de contacter
les expéditeurs qui se comportent mal. Il vous faut une paire de clés
P-256 par application ; la clé publique va dans votre frontend afin
que le navigateur puisse épingler les abonnements à votre serveur, et
la clé privée reste sur le serveur pour signer les JWT.

Générez-en une une fois, persistez-la, et réutilisez-la pour
toujours :

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// Sauvegardez le PEM quelque part de durable - un gestionnaire de secrets,
// un fichier que le pipeline de déploiement monte, un volume
// env-vars-as-files. Vous NE POUVEZ PAS régénérer ceci sans invalider
// tous les abonnements existants.
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// Le frontend a besoin de la clé publique non compressée en
// base64url sans padding. Donnez-la à votre JS pour que
// `pushManager.subscribe()` puisse l'utiliser comme
// `applicationServerKey`.
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

À l'amorçage, chargez le PEM sauvegardé :

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

Un `VapidSigner` produit des JWT mais n'envoie rien - c'est purement
une primitive de signature. La couche suivante l'enveloppe.

## Construire un WebPushClient

`WebPushClient` est la primitive côté HTTP : donnez-lui un signer et
une URI de contact (« comment le service push peut vous joindre si
vous vous comportez mal »), et récupérez un objet dont la méthode
`send` chiffre un payload, signe un JWT, et POST vers le point de
terminaison de l'abonnement.

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// Le subject DOIT être une URI mailto: ou une URL https: selon la
// RFC 8292 §2.1. Tout le reste est rejeté à la construction afin
// qu'un déploiement mal configuré échoue vite à l'amorçage - pas
// silencieusement après le premier dispatch en échec.
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

Pourquoi `Arc<WebPushClient>` ? `WebPushClient` enveloppe un
`VapidSigner` qui enveloppe un `ES256KeyPair` privé. Aucun d'eux n'est
`Clone` - les clés privées ne devraient pas être dupliquées à la
légère - et construire un nouveau signer pour chaque enregistrement de
canal signifierait N identités VAPID indépendantes pour la même
application. Envelopper dans un `Arc` permet à une seule identité
signée de servir chaque enregistrement et chaque livraison
concurrente.

### Politique de point de terminaison

Les points de terminaison d'abonnement sont des données dérivées de
l'utilisateur : le navigateur reçoit l'URL d'un service push distant
quand un utilisateur s'abonne, et votre serveur stocke tout ce que le
navigateur a renvoyé. Un abonnement stocké de façon malveillante peut
pointer le POST HTTP n'importe où d'accessible, transformant
l'expéditeur push en gadget SSRF.

`WebPushClient` a pour défaut `EndpointPolicy::Strict` :

- Le schéma doit être `https`
- L'hôte doit être un domaine nommé, pas un littéral IP
- Les noms d'hôte de métadonnées cloud et les TLD réservés par la RFC
  2606 (`.localhost`, `.local`, `.internal`, `.test`, `.example`,
  `.invalid`) sont rejetés

Cela bloque les sondes SSRF évidentes sans casser les vrais services
push (FCM, Mozilla Autopush, le `web.push.apple.com` d'Apple).

Pour des tests d'intégration locaux contre un serveur mock `wiremock`,
vous devez désactiver ça :

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

N'utilisez pas `AllowAny` en production. Les vérifications strictes
existent pour empêcher une table d'abonnements altérée d'être
transformée en arme.

### Transport personnalisé

`WebPushClient::new` applique un délai d'attente de 30 secondes par
requête. Si vous avez besoin d'une politique de transport différente -
proxy d'entreprise, TLS épinglé, délai d'attente plus court - passez
un `reqwest::ClientBuilder` à `WebPushClient::with_client_builder`.
Toutes les options du builder sont honorées, mais la politique de
redirection est désactivée de force : un endpoint validé qui répond
3xx ne doit pas faire rebondir le POST vers une URL non validée, la
bibliothèque n'accepte donc pas le réglage de redirection de
l'appelant.

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let client = WebPushClient::with_client_builder(
    Client::builder().timeout(Duration::from_secs(10)),
    signer,
    "mailto:ops@example.org",
)?;
```

`WebPushClient::with_client` prend un client déjà construit dont la
bibliothèque ne peut pas inspecter la politique de redirection. Les
envois sous la politique `Strict` par défaut sont refusés pour un
tel transport avant toute I/O - passez à `with_client_builder`, ou
acceptez explicitement le risque avec
`.allow_unconfined_redirects()` quand il est établi que le client ne
suit pas les redirections.

## Câbler WebPushChannel dans les notifications

Le `WebPushClient::send` brut fonctionne - mais la façon dont vous
envoyez réellement des notifications push dans Suprnova, c'est via le
sous-système [Notifications](notifications.md). Une `Notification`
déclare `vec!["webpush"]` dans son `channels()`, un destinataire
`Notifiable` retourne un `SubscriptionInfo` encodé en JSON depuis
`route_for("webpush")`, et le `NotificationDispatcher` lié fait le
fan-out.

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs : combien de temps le service push garde un message non
// livré. 86_400 (24h) est un défaut raisonnable pour les
// notifications non urgentes ; réduisez à 60 pour les alertes
// « agir tout de suite » où un message périmé est pire que pas de
// message.
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` fonctionne en dernier-écrit-gagne sur le `name()`
du canal, si bien que les tests peuvent substituer un stub sans
affecter la liaison de production.

## Définir une notification

Une notification destinée au push a la même forme que n'importe
quelle autre notification Suprnova - déclarez `"webpush"` dans
`channels()` et mettez le JSON que vous voulez livrer dans `data()` :

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

Le JSON de `data()` est ce que votre Service Worker reçoit. Choisissez
une forme stable et documentez-la pour le frontend - Suprnova n'en
impose aucune, parce que l'UI de notification est une préoccupation du
frontend.

## Router le destinataire

Un `Notifiable` retourne la route pour chaque canal qu'il supporte.
Pour Web Push, cette route est le `SubscriptionInfo` encodé en JSON -
exactement ce que le navigateur a produit via
`PushSubscription.toJSON()`, stocké tel quel :

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

Retourner `None` fait que le dispatcher ignore le canal
silencieusement - utile pour les utilisateurs qui ne se sont pas
abonnés au push mais reçoivent quand même des e-mails.

## L'envoyer

Synchrone :

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

En file d'attente - pré-résout la route d'abonnement au moment de la
mise en file d'attente, si bien que le worker n'a pas besoin de
recharger l'utilisateur :

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Pour que `Notify::queue` fonctionne, enregistrez la factory de la
notification à l'amorçage afin que le worker puisse reconstruire le
payload JSON en notification typée :

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

En coulisses, le dispatch en file d'attente construit un
`SendNotificationJob` portant `(notification_name, payload,
per_channel_routes, channels)`. Le worker réhydrate la notification,
recherche `WebPushChannel` par nom sur le dispatcher lié, et appelle
`deliver(route, &notification)` - le même chemin de code que le
`Notify::send` synchrone.

## Le côté navigateur

Suprnova ne livre pas de SDK JavaScript - le côté navigateur, c'est
l'API Web Push nue. Le flux que votre frontend doit implémenter :

1. Enregistrer un Service Worker.
2. Demander la permission à l'utilisateur.
3. S'abonner via `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <your VAPID public key> })`.
4. POST `subscription.toJSON()` vers un point de terminaison Suprnova
   qui le stocke sur la ligne utilisateur.

```js
// Enregistrement du Service Worker (quelque part dans le point
// d'entrée de votre app)
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

Votre point de terminaison Suprnova reçoit le JSON, valide la forme,
et le stocke sur l'utilisateur - la chaîne est opaque pour votre
serveur, mais elle doit être exactement le JSON que le navigateur a
produit (le type `SubscriptionInfo` utilise `Deserialize` pour le
parser plus tard) :

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // Parse pour valider la forme - endpoint, keys.p256dh, keys.auth.
    // Si le parsing échoue, le navigateur nous a remis quelque chose
    // de malformé.
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // Persistez `raw` tel quel - c'est exactement la chaîne que
    // WebPushChannel remettra à serde_json::from_str au moment du
    // dispatch.
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

Le Service Worker déchiffre le payload push et affiche la
notification :

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## Limites de payload

La spec Web Push plafonne chaque payload chiffré à 4096 octets au
total. Suprnova rejette les textes en clair plus grands que 3992
octets (le plafond moins la surcharge de chiffrement AES128GCM
d'environ 85 octets) au moment du chiffrement, si bien que l'échec
apparaît dans votre code, pas dans un 413 venant du service push. Une
`Notification` dont le `data()` sérialisé dépasse cette limite
retourne `WebPushError::Encryption` depuis le `deliver` du canal.

Pour tout ce qui est plus grand - un long corps de message, une
miniature - envoyez une notification courte portant une URL que le
Service Worker récupère au clic. C'est à la fois plus rapide (pas de
chiffrement sur un payload de plusieurs Ko) et plus flexible (le fetch
peut retourner la forme que vous voulez).

## Abonnements morts

Quand le service push retourne 404 ou 410, l'abonnement est mort -
l'utilisateur a désinstallé le navigateur, révoqué la permission, ou
vidé le stockage. `WebPushChannel` traite cela comme un warn non
fatal :

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

Le dispatch retourne `Ok(())` parce que la notification a atteint un
état terminal - il n'y a pas de destinataire contre qui réessayer.
Votre application est censée agir sur le warn : parser `endpoint`
depuis le journal (ou accrocher un écouteur `NotificationFailed` qui
classifie via `WebPushError`) et supprimer la ligne d'abonnement.
Suprnova livre le warn ; il n'élague pas automatiquement la table des
abonnements pour vous.

## Réessais et Retry-After

Quand le service push retourne un 5xx, 408, ou 429 transitoire, le
`WebPushError::PushServiceRejected` sous-jacent porte l'indication
`Retry-After` parsée (forme delta-seconds seulement - la forme
HTTP-date retourne `None`) :

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...réessayez, ou remettez-la dans la file d'attente avec un délai
    }
    Err(WebPushError::SubscriptionGone) => {
        // supprimer l'abonnement
    }
    Err(e) => return Err(e.into()),
}
```

L'indication `Retry-After` est plafonnée à 24 heures, si bien qu'un
serveur hostile ne peut pas garer un worker sur un sommeil de plusieurs
années.

Quand vous utilisez `Notify::queue`, le réessai/backoff propre à la
file d'attente s'applique - une `WebPushError` qui se propage hors de
`WebPushChannel::deliver` remonte comme une erreur de job et
l'enveloppe gère la remise en file d'attente selon la politique de
backoff du job. L'indication `Retry-After` est journalisée mais n'est
pas (encore) réinjectée dans le calcul du délai de la file d'attente ;
si vous en avez besoin, accrochez un écouteur `NotificationFailed` qui
remet en file d'attente avec le délai indiqué.

## Télémétrie

Le dispatcher de notifications enveloppe le fan-out dans un span info
`notification.dispatch` étiqueté avec le nom de la notification et le
nombre de canaux. Chaque livraison réussie émet un événement
`NotificationSent` ; les échecs émettent `NotificationFailed` portant
le nom du canal, la route, et la chaîne d'erreur. Câblez n'importe
lequel de ceux-là dans votre pipeline de métriques/journaux de la
même façon que vous câblez les autres événements du framework - voir
[Événements](events.md).

Un abonnement mort émet un WARN structuré avec `channel="webpush"`, le
endpoint, et le nom de la notification. C'est le signal à scraper pour
un job automatisé de nettoyage des abonnements.

### Pourquoi Suprnova diverge

Le driver `WebPush` de Laravel est un package communautaire
(`laravel-notification-channels/webpush`) - pas dans le cœur,
versionné séparément, opinionné sur l'ORM. Suprnova intègre Web Push
directement dans le framework parce que le protocole est bien défini
et que le POST HTTP chiffré est un contrat trop petit pour
l'envelopper dans une abstraction tierce. Le sous-système de
notifications garde la surface uniforme : la même `Notification` que
vous envoyez au mail ou à la base de données atterrit aussi comme un
push, pas de matrice de drivers, pas d'arbre de config séparé.

Nous remontons aussi la politique de point de terminaison strict par
défaut. Le package communautaire de Laravel laisse la protection SSRF
à l'application ; nous prenons la position que « le point de
terminaison vient de données utilisateur » est la forme de tout
abonnement Web Push, et que le défaut sûr appartient au framework, pas
à votre code.

La classification du réessai (`is_retryable`, `retry_after`) est
exposée comme des méthodes typées sur `WebPushError` plutôt que comme
une table de constantes magiques dans la couche de file d'attente. La
file d'attente possède toujours la politique de réessai - l'erreur
vous dit si un réessai pourrait réussir et combien de temps attendre ;
la file d'attente décide si et quand le rejouer. Séparer les deux
signifie que vos stratégies de réessai personnalisées (backoff
exponentiel, jitterisé, plafonné) n'ont pas à faire de cas spécial pour
Web Push.

## Tests

Montez un serveur `wiremock`, pointez un `WebPushClient` vers lui avec
`EndpointPolicy::AllowAny`, et vérifiez les requêtes qu'il reçoit :

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() contient maintenant le POST chiffré.
}
```

Pour des tests de bout en bout qui ne se soucient pas des octets
chiffrés, `Notify::fake()` (couvert dans
[Notifications](notifications.md)) capture le dispatch sans exécuter
le canal - plus rapide, pas de serveur mock, pas d'aller-retour de
chiffrement.

## Référence

- Primitives : `suprnova::VapidKey`, `suprnova::VapidSigner`,
  `suprnova::VapidClaims`
- Client : `suprnova::WebPushClient`, `suprnova::EndpointPolicy`,
  `suprnova::PushResponse`, `suprnova::SubscriptionInfo`
- Erreur : `suprnova::WebPushError` - `.is_retryable()`,
  `.retry_after()`, `WebPushError::SubscriptionGone`
- Encodage : `suprnova::ContentEncoding` (Aes128Gcm ; plafond de texte
  en clair de 3992 octets)
- Canal : `suprnova::WebPushChannel`
- Façade : `suprnova::Notify`
- Job de file d'attente : `suprnova::SendNotificationJob`
- Enregistrement de factory :
  `suprnova::notifications::register_notification_factory`

## Suivant

- [Notifications](notifications.md) - le dispatcher multi-canal dans
  lequel `WebPushChannel` se branche
- [E-mail](mail.md) - l'homologue canal e-mail pour les utilisateurs
  sans push
- [Diffusion](broadcasting.md) - livraison en temps réel pour les
  utilisateurs qui sont sur le site
- [File d'attente](queues.md) - comment `Notify::queue` s'appuie sur
  `SendNotificationJob`
- [Événements](events.md) - écouter `NotificationSent` /
  `NotificationFailed` pour piloter le nettoyage des abonnements morts
