# Diffusion

La diffusion est la couche de notification serveur-vers-client
par-dessus la [primitive WebSocket](websockets.md) de Suprnova. Vous
dispatchez un événement `Broadcastable` via `EventFacade` ; le
framework répartit l'enveloppe JSON de l'événement vers chaque abonné
WebSocket sur les canaux que l'événement nomme. Vous ne gérez jamais
les connexions individuelles - vous gérez des abonnements à des
canaux, et le hub fait le reste.

Le `BroadcastHub` est le bus. L'`InMemoryBroadcastHub` par défaut
s'exécute entièrement in-process - parfait pour les déploiements à
réplique unique et la suite de tests. Derrière la feature Cargo
`broadcasting-fanout`, `SeaStreamerBroadcastHub` route les mêmes
événements via un broker de stream (Redis Streams, Kafka, fichier,
stdio) si bien qu'une publication dans un processus atteint les
abonnés de tous les autres processus.

Tout ce qui vient du chapitre [WebSocket](websockets.md) s'applique
toujours - les pings de battement de cœur, `max_missed_pings`,
`WsConfig`, le middleware par route, les paramètres de chemin. La
diffusion ajoute juste un protocole réseau et un registre de canaux
par-dessus.

## Démarrage rapide

Quatre fichiers et le navigateur voit un événement.

`src/channels/order_updates.rs` :

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs` :

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs` :

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. Lie le hub derrière le trait - les handlers le résolvent de
    //    façon uniforme.
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. Enregistre chaque canal d'avance ; le handler WS résout par nom.
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. Câble le pont événement → hub une fois par type Broadcastable.
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - construisez un `BroadcastingWsHandler` par route en
résolvant le hub et le registre amorcés depuis le conteneur :

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // Conteneur en priorité ; retombe sur un hub in-process neuf + un
    // registre vide afin que les tests unitaires qui assemblent le
    // routeur sans bootstrap fonctionnent quand même.
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

Connectez-vous et observez :

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

Dispatchez depuis n'importe quel contrôleur, worker, ou tâche
planifiée :

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## Canaux

Un canal est une cible d'abonnement nommée. Les clients s'abonnent par
nom ; le hub livre les événements à chaque abonné actif sur ce nom. Le
trait `Channel` a des défauts asymétriques qui échouent fermé en
écriture et ouvert en lecture - voir [Pourquoi Suprnova
diverge](#pourquoi-suprnova-diverge) ci-dessous.

### Canaux publics

Le défaut. N'importe quel client peut s'abonner.

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() vaut true par défaut - ouvert à tous les abonnés.
}
```

### Canaux privés

Surchargez `authorize` pour filtrer les abonnements. Un abonnement
rejeté produit une frame `error` avec `reason: "unauthorized"` ;
aucune frame `subscribed` n'est envoyée.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` est ce que le client a envoyé dans le champ `data` de la frame
subscribe - un token bearer, un channel-bind signé, n'importe quoi
défini par l'application. `Request` est la requête de mise à niveau
HTTP d'origine (les en-têtes et cookies sont lisibles directement).
`params` porte les valeurs capturées depuis un nom paramétré et est
vide pour les noms fixes.

`PrivateChannel` est un trait marqueur. Le framework ne le vérifie pas
à l'exécution - c'est un signal de niveau type indiquant que le canal
surcharge `authorize`, destiné à de l'outillage futur (un lint
clippy, une passe d'audit).

### Canaux paramétrés

Intégrez des segments `{param}` dans `name()` et un seul enregistrement
sert chaque abonnement concret qui correspond au motif - le même
modèle que le `Broadcast::channel('orders.{id}', …)` de Laravel. Les
valeurs capturées atteignent chaque hook sous forme d'une map
`ChannelParams`.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // Filtre sur l'id capturé - l'utilisateur de la session
        // possède-t-il cette commande ?
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// Un seul enregistrement sert orders.42, orders.99, orders.featured, …
registry.register(OrderChannel);
```

Chaque `{param}` se lie à exactement un segment séparé par un point :
`orders.{id}` correspond à `orders.42` mais pas à `orders` ni à
`orders.42.line`. La résolution préfère un enregistrement à nom fixe
exact à n'importe quel motif (`orders.featured` l'emporte sur
`orders.{id}` pour ce nom précis), puis le motif le plus spécifique
(le plus de segments littéraux), avec le motif lexicographiquement le
plus petit comme départage déterministe.

### Canaux de présence

Les canaux de présence suivent l'appartenance. Quand un client
s'abonne, le hub livre à ce client un instantané `presence.here` et
diffuse `presence.joined` à chaque autre abonné. Quand un client part,
le hub diffuse `presence.left`.

Le contrat en deux parties est facile à moitié implémenter : vous
devez à la fois surcharger `Channel::presence_info` pour retourner
`Some(self)` ET implémenter `PresenceChannel::member_info`. Oublier
`presence_info` câble le canal comme non-présence - les abonnements
fonctionnent, mais `presence.joined` / `presence.here` /
`presence.left` ne se déclenchent jamais.

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // Requis - sans cette surcharge, PresenceChannel est câblé mais inerte.
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // Retourne ce dont les autres abonnés ont besoin pour
        // identifier ce membre - typiquement un id utilisateur.
        // N'incluez jamais de secrets ni de données personnelles privées.
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

Voir [Présence](#présence) pour le flux d'événements complet et
l'écho du self-join.

### Noms réservés

Les noms commençant par `__` sont réservés aux méta-canaux du
framework (`__presence__` porte la réplication de présence
inter-processus). Appeler `registry.register(channel)` sur un nom
préfixé par `__` panique à l'enregistrement, si bien que l'erreur est
attrapée à l'amorçage, pas à l'exécution.

### Pourquoi Suprnova diverge

Laravel lie l'autorisation de canal à un paramètre de callback `$user`
parce que PHP injecte implicitement l'utilisateur authentifié courant.
L'`authorize` de Suprnova prend à la place la `Request` brute, les
`ChannelParams` capturés, et un `data: Value` arbitraire - trois
entrées orthogonales, toutes disponibles, sans contexte implicite.
Vous lisez le cookie de session ou le token bearer depuis `Request` et
les params façon routage depuis `ChannelParams` ; le payload `data`
est un emplacement libre pour les tokens que le client fournit au
moment de l'abonnement.

Les défauts du trait `Channel` sont **asymétriques
intentionnellement** : `authorize` vaut `true` par défaut (s'abonner
est public par défaut), `authorize_publish` vaut `false` par défaut
(le publish initié par le client est refusé par défaut). L'action
dangereuse échoue fermée ; la sûre échoue ouverte. En cas de doute, ne
touchez à aucun des deux.

## Le trait Broadcastable

`Broadcastable: Event + Serialize` - chaque `Broadcastable` est aussi
un `Event`. Dispatcher via `EventFacade::dispatch(event)` exécute
chaque écouteur in-process ET envoie le payload sérialisé en JSON à
chaque abonné WebSocket sur les canaux que l'événement nomme.

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // Un événement, plusieurs canaux. Chaque abonné de chaque
        // canal reçoit la même enveloppe.
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

Câblez le pont une fois par type Broadcastable à l'amorçage :

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

Après cela, `EventFacade::dispatch(event).await?` est toute la partie
envoi - pas d'appel `publish` séparé.

Par défaut, l'événement est sérialisé via
`serde_json::to_value(&event)` et envoyé à chaque abonné. Les canaux
sans aucun abonné sont ignorés silencieusement sur le hub in-process ;
le hub cross-process les publie quand même afin que les autres
processus aient une chance de livrer.

Quatre méthodes optionnelles affinent le défaut :

**`broadcast_event_name(&self) -> &'static str`** - surcharge le nom
de l'événement sur le réseau. Vaut par défaut `Self::event_name()`. À
utiliser pour découpler l'identité de l'événement in-process du nom
sur le réseau.

**`broadcast_with(&self) -> Option<Value>`** - retournez `Some(value)`
pour envoyer un payload sélectionné au lieu de la sérialisation
complète de l'événement (le `broadcastWith()` de Laravel). Omettez les
secrets ou remodelez pour le client sans changer le type de
l'événement :

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // Ne mettez jamais le solde sur le réseau - seulement l'id public.
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - retournez `false` pour
dispatcher l'événement aux écouteurs in-process mais sauter l'envoi
WebSocket (le `broadcastWhen()` de Laravel). Seule la diffusion est
filtrée ; le reste du pipeline d'événement s'exécute sans changement :

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // diffuse seulement à la publication
}
```

**`broadcast_to_others(&self) -> bool`** - retournez `true` pour
exclure la connexion qui a déclenché la diffusion (le `toOthers()` de
Laravel). Le framework assigne à chaque connexion de diffusion un
`socket_id` à la connexion (envoyé dans la frame `connected`) ; le
navigateur le renvoie en écho comme en-tête `X-Socket-ID` sur les
requêtes HTTP ; un événement `broadcast_to_others` dispatché pendant
le traitement de cette requête ignore la connexion d'origine. Hors
requête (un worker ou un job) ou quand aucun `X-Socket-ID` n'est
présent, cela dégrade en diffusion à tout le monde :

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // l'expéditeur l'a déjà
}
```

C'est un choix par type d'événement. Pour une exclusion par dispatch,
publiez directement :

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### Ordre de dispatch avec d'autres écouteurs

`EventFacade::dispatch` est **fail-fast** : si un publish sur le hub
retourne `Err` (par ex. une déconnexion du broker sur un hub
cross-process), le `BroadcastListener` retourne `Err` et tout autre
écouteur enregistré **après** lui ne s'exécute pas. Deux façons de
gérer ceci :

- Enregistrez le pont de diffusion APRÈS les écouteurs in-process dont
  les effets de bord (écritures DB, émission de journaux) doivent
  s'exécuter indépendamment du résultat de la diffusion.
- Passez à `EventFacade::dispatch_best_effort(event)` quand chaque
  écouteur doit s'exécuter indépendamment du fait que l'un d'eux
  retourne `Err`.

Les hubs en mémoire ne retournent jamais `Err` - seule la variante
cross-process fait remonter les défaillances du broker.

## Le protocole réseau

Chaque message sur la route de diffusion est une frame JSON UTF-8.
Deux formes : `ClientFrame` (client → serveur) et `ServerFrame`
(serveur → client).

### Frames client

| `action` | Champs requis | Champs optionnels | Signification |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | S'abonner à `channel`. `data` est transmis à `Channel::authorize`. |
| `unsubscribe` | `channel` | | Se détacher de `channel`. |
| `publish` | `channel`, `event`, `data` | | Envoie un événement à chaque abonné sur `channel`. Filtré par `Channel::authorize_publish` ET nécessite un abonnement actif. |

Le `publish` initié par le client est filtré par **deux**
vérifications : la connexion DOIT détenir un abonnement autorisé au
canal cible, ET `Channel::authorize_publish` doit retourner `true`
(il vaut `false` par défaut). Cela reflète le contrat d'événement
client de Pusher - les canaux qui veulent des publishs client y
participent explicitement en surchargeant le hook. La plupart des
canaux de diffusion côté serveur ne veulent jamais d'événements
initiés par le client, et la forme de refus par défaut correspond à
cette intention.

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### Frames serveur

| `action` | Champs | Signification |
|----------|--------|---------|
| `connected` | `socket_id` | Envoyé une fois, en premier. Renvoyez `socket_id` en écho comme en-tête HTTP `X-Socket-ID` afin que `broadcast_to_others` côté serveur puisse exclure cette connexion. |
| `subscribed` | `channel` | Abonnement accepté. |
| `unsubscribed` | `channel` | Désabonnement confirmé. |
| `event` | `channel`, `event`, `data` | Un événement a été diffusé sur `channel`. |
| `lagged` | `channel`, `skipped` | L'abonné a pris du retard sur le ring buffer par canal du serveur et `skipped` enveloppes ont été abandonnées sur cette connexion. L'état local du client sur `channel` est périmé ; refetchez avant de traiter d'autres événements. |
| `error` | `channel` (nullable), `reason` | La dernière action a échoué. `channel` est `null` pour les erreurs de niveau enveloppe non liées à un canal. |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### À propos de `lagged`

Chaque canal a un ring buffer par processus (256 enveloppes). Un
abonné qui ne vide pas assez vite - un client lent, un forwarder
bloqué - prend du retard, et le buffer écrase les événements les plus
anciens. Quand cela arrive, le serveur envoie une frame `lagged`
nommant le canal et le nombre d'événements abandonnés, puis continue
à livrer les frames suivantes normalement. L'écart n'est **pas**
récupérable côté serveur ; le client doit refetcher ou se
resynchroniser avant de traiter d'autres événements sur ce canal.
Abandonner silencieusement des événements laisserait des bugs se
cacher comme « on a perdu un tick » plutôt que « l'état du client a
divergé de celui du serveur ».

#### Échecs de publish

Quand un `publish` initié par le client est accepté par
`authorize_publish` mais que le publish sur le hub lui-même échoue
(déconnexion du broker sur le hub cross-process), le client d'origine
reçoit une frame `error` avec `reason: "publish failed: …"` afin qu'il
sache que l'événement n'a pas atteint les autres processus. Les
autres abonnés ne sont pas notifiés.

### Exemple de session

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# Le serveur dispatche OrderPlaced :
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## Middleware par route

Les routes de diffusion supportent le même chaînage `.middleware(M)`
que les routes WebSocket ordinaires :

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

Une réponse non-2xx de n'importe quel middleware court-circuite la
mise à niveau - le client reçoit la réponse d'erreur HTTP et aucun
handshake WebSocket n'a lieu. C'est le bon endroit pour appliquer
l'authentification au niveau transport (validité de session,
vérifications d'origine, limitation de débit au moment de la
connexion) sans dupliquer la vérification à l'intérieur de
l'`authorize` de chaque canal.

Plusieurs middleware se composent de gauche à droite :

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

La séparation est intentionnelle : le **niveau transport** (qui a le
droit d'ouvrir la connexion) vit dans le middleware ; le **niveau
canal** (qui a le droit de s'abonner à quel canal) vit dans
`Channel::authorize`.

### `WsConfig` par route

Surchargez les défauts WebSocket à l'échelle du processus par route.
Chaînez `.config(WsConfig { ... })` après le handler - avant ou après
`.middleware(M)` (l'ordre n'a pas d'importance) :

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

Les cinq champs configurables et où chacun compte :

| Champ | Défaut | Cas d'usage |
|-------|---------|----------|
| `ping_interval` | 30s | Chat / présence : réduisez à 5-10s pour détecter rapidement les connexions mobiles mortes. Streaming de données en masse : allongez pour réduire le surcoût. |
| `max_missed_pings` | 2 | Réglez à `1` pour le chat où un Pong manqué devrait fermer immédiatement. Réglez à `3+` pour les réseaux mobiles instables. Réglez à `usize::MAX` pour désactiver la fermeture en l'absence de pong. |
| `max_message_size` | 1 Mio | Défaut sûr pour un point de terminaison public. Partez de `WsConfig::generous()` (64 Mio) pour les flux internes de confiance. |
| `max_frame_size` | 64 Kio | Dimensionné pour les frames de chat / notification avec de la marge. Partez de `WsConfig::generous()` (16 Mio) pour les grosses frames non fragmentées. |
| `origin_policy` | `SameOrigin` | Le défaut rejette les mises à niveau cross-origin - la seule protection CSRF qu'un handshake WS de navigateur possède. Utilisez `AllowList(vec![...])` pour des frontends cross-origin explicites, ou `AllowAny` seulement pour les points de terminaison non-navigateur. |

Quand aucun `.config(...)` n'est fourni, la route hérite de
`WsConfig::default()`. Une config explicite par route l'emporte
toujours sur le défaut.

Pour les routes qui servent des flux internes de confiance (fan-out
serveur-à-serveur, gros transferts binaires), partez de la factory des
flux de confiance et ajustez selon vos besoins :

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // message 64 Mio / frame 16 Mio
    })
```

## Présence

Quand un client s'abonne avec succès à un canal de présence, le hub :

1. Appelle `PresenceChannel::member_info` avec la `Request` de mise à
   niveau et les `ChannelParams` capturés pour recueillir les données
   du membre qui rejoint.
2. Envoie une frame d'événement `presence.here` au nouvel abonné avec
   `data: { "members": [...] }` - un instantané de tous les membres
   actuellement suivis (excluant celui qui vient de rejoindre).
3. Publie un événement `presence.joined` avec `data: <member_info>`
   sur le canal. Chaque abonné - y compris le nouveau via son propre
   forwarder - le reçoit ; les clients filtrent le self-join en
   comparant l'identité du membre qui rejoint à la leur.

Quand un abonné se déconnecte ou envoie une frame unsubscribe :

4. Le hub publie un événement `presence.left` avec les données du
   membre qui part. Chaque abonné restant le reçoit.

Les trois frames arrivent comme des frames d'action `event` avec des
noms `event` réservés :

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

À travers les processus, l'état de présence est répliqué via le
méta-canal réservé `__presence__` (voir [Fan-out
inter-processus](#fan-out-inter-processus)). Les opérations track et
untrack sur n'importe quel processus se propagent à tous les abonnés ;
`list_members` retourne la vue fusionnée (locale + distante). Les
processus morts dont `untrack_member` ne s'est jamais déclenché voient
leurs membres élagués via TTL - 60 s par défaut.

## Fan-out inter-processus

L'`InMemoryBroadcastHub` par défaut ne fait du fan-out que vers les
abonnés du processus courant. Pour les déploiements multi-répliques,
activez la feature Cargo `broadcasting-fanout` et substituez
`SeaStreamerBroadcastHub` :

`Cargo.toml` :

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.3", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs` :

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // URI du streamer (backend choisi d'après le schéma)
            "suprnova-broadcast",    // clé de stream (partagée par tous les processus du cluster)
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ... reste du bootstrap inchangé
}
```

Le constructeur prend deux arguments : l'URI du streamer (sélectionne
le backend à l'exécution d'après le schéma) et la clé de stream (le
nom de topic partagé par tous les processus du cluster). Utilisez la
même clé de stream sur chaque réplique, sinon elles ne verront pas
les événements des autres.

`new_with_presence_ttl(uri, key, ttl)` remplace le TTL de présence de
60 s par défaut - utile pour les tests qui doivent exercer rapidement
le chemin de reprise après crash. `new_loopback(uri, key)` active le
loopback stdio pour les tests d'intégration mono-processus ; la garde
anti-doublon assure que chaque événement applicatif est tout de même
livré exactement une fois en local.

### Backends

Le backend est sélectionné à l'exécution d'après le schéma de l'URI :

| Schéma d'URI | Backend | Prêt pour la production | Remarques |
|------------|---------|------------------|-------|
| `redis://`, `rediss://` | Redis Streams | **Oui** | Recommandation par défaut. `rediss://` utilise TLS. Activé dans le build par défaut. |
| `kafka://`, `kafka+ssl://` | Kafka | **Oui** | Nécessite `kafka` dans l'ensemble de features de `sea-streamer` (`framework/Cargo.toml`). |
| `stdio://` | pipes stdin/stdout | Non - tests uniquement | Loopback mono-processus. |
| `file://` | Fichier local | Non - hôte unique | Nécessite `file` dans l'ensemble de features de `sea-streamer`. |

Le build Suprnova par défaut active `stdio` + `redis` + `socket`. Pour
activer Kafka ou le fichier, éditez `framework/Cargo.toml` et ajoutez
la feature `sea-streamer` correspondante.

### Architecture

Chaque `publish(envelope)` fait deux choses en parallèle :

1. **Fan-out local** - l'`InMemoryBroadcastHub` interne livre
   immédiatement aux abonnés de ce processus. Les abonnés locaux
   n'attendent jamais le réseau.
2. **Écriture dans le stream** - la même enveloppe est sérialisée et
   poussée dans le stream sea-streamer afin que la pompe de
   consommation de chaque autre processus la récupère et la livre en
   local.

Une garde anti-doublon empêche de voir deux fois chaque événement de
données applicatives : l'instance du hub possède un UUID aléatoire,
chaque enveloppe qu'elle produit porte cet UUID, et la pompe de
consommation ignore les enveloppes entrantes dont l'identifiant
d'instance correspond à celui du hub local. Les messages du méta-canal
de présence font exception - chaque hub a besoin de ses propres
événements dans la vue inter-processus pour que le chemin de lecture
soit unifié.

Le dispatch des backends repose sur une énumération, pas sur un objet
de trait : le hub stocke un `SeaProducer` / `SeaConsumer` concret issu
de l'adaptateur socket de sea-streamer, qui est une énumération sur
tous les backends compilés. Aucun surcoût `dyn` sur le site d'appel de
publication.

### Présence inter-processus

`SeaStreamerBroadcastHub` réplique automatiquement l'état de présence
à travers les processus. Chaque instance reçoit un `instance_id` UUID
à la construction ; `track_member` / `untrack_member` publient des
`PresenceEvent` sur le méta-canal réservé `__presence__`. Chaque
processus maintient une `cross_process_view` mise à jour par sa tâche
de consommation ; `list_members` retourne la vue fusionnée (locale et
distante uniformément).

Vivacité : chaque processus republie ses membres tous les `ttl / 6`
(10 s au TTL de 60 s par défaut) comme battement de cœur. Les entrées
périmées - les membres dont le `last_seen` dépasse le TTL - sont
élaguées tous les `ttl / 2`. Cela couvre les crashs de processus qui
n'ont pas eu le temps de publier `MemberRemoved`.

## Fermeture en l'absence de pong

Les routes de diffusion participent au même battement de cœur
WebSocket que les routes `ws!` ordinaires. Le framework envoie un Ping
toutes les `WsConfig::ping_interval` (30 s par défaut). Si une
connexion ne répond pas avec un Pong dans `max_missed_pings`
intervalles consécutifs (2 par défaut), le framework ferme avec le
code 1011.

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

Réduire `ping_interval` détecte les connexions mortes plus vite au
prix d'un trafic de base plus élevé. `max_missed_pings: 1` ferme après
le tout premier Pong manqué - à utiliser seulement quand les accrocs
réseau sont rares et que vous voulez le nettoyage de connexion morte
le plus rapide possible. `max_missed_pings: usize::MAX` désactive
entièrement la fermeture en l'absence de pong.

## Déploiement en production

Les routes de diffusion sont des connexions HTTP mises à niveau sur le
même listener TCP hyper que vos routes HTTP. La terminaison TLS se
fait en amont, exactement comme décrit dans [le chapitre
WebSocket](websockets.md#production-deployment). Les configurations
nginx et Caddy de ce chapitre s'appliquent sans changement -
étendez-les pour couvrir le chemin `/ws/broadcast`.

Les tâches de handler WebSocket actives (y compris les connexions de
diffusion) sont suivies dans l'ensemble `WS_TASKS` du framework et
vidées à l'arrêt gracieux, si bien que les livraisons d'événements en
vol se terminent avant que le processus ne se termine.

## Tester les diffusions

`RecordingBroadcastHub` est l'analogue Suprnova du `Broadcast::fake()`
de Laravel - un `BroadcastHub` qui enregistre chaque enveloppe publiée
tout en continuant à livrer aux abonnés actifs. Liez-le à la place
d'`InMemoryBroadcastHub` dans un test et vérifiez ce qui a été diffusé
sans vous abonner au préalable :

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ... exécutez du code qui publie (directement, ou via un Broadcastable dispatché) ...

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| Helper                         | Vérifie                                                  |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | au moins une enveloppe sur `ch` avec le nom d'événement `ev` |
| `assert_nothing_broadcast()`   | rien n'a été publié                                    |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - chaque enveloppe enregistrée       |
| `count()`                      | total des enveloppes enregistrées                                 |

Pour vérifier qu'un *événement* `Broadcastable` a été dispatché du
tout (plutôt que ce qui a atteint le réseau), `EventFacade::fake()`
enregistre l'événement lui-même - voir
[Événements](events.md#testing--eventfacadefake).

## Référence de parité Laravel

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | `Channel` trait impl + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`, params in `ChannelParams` |
| `PrivateChannel` (interface) | `PrivateChannel` marker trait + override `authorize` |
| `PresenceChannel` (interface) | `PresenceChannel` + override `Channel::presence_info` |
| `ShouldBroadcast` (interface) | `Broadcastable` trait |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `RecordingBroadcastHub` bound as `dyn BroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Pusher / Reverb / Ably driver | `InMemoryBroadcastHub` (single-process) or `SeaStreamerBroadcastHub` (cross-process: Redis / Kafka / file / stdio) |
| Bibliothèque client Echo | non livrée - câblez le protocole d'enveloppe JSON depuis le navigateur à la main pour l'instant |

## Référence

| Symbole | Objet |
|--------|-------|
| `suprnova::broadcasting::Channel` | Trait de canal. Surchargez `name()` (requis), `authorize`, `authorize_publish`, `presence_info`. |
| `suprnova::broadcasting::ChannelParams` | Valeurs capturées depuis un `name()` paramétré. `get(key) -> Option<&str>`. Vide pour les noms fixes. |
| `suprnova::broadcasting::PrivateChannel` | Trait marqueur sur un `Channel` qui surcharge `authorize`. Aucune méthode requise. |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`. Nécessite la surcharge de `Channel::presence_info`. |
| `suprnova::broadcasting::ChannelRegistry` | Détient chaque canal enregistré. Lié comme `Arc<ChannelRegistry>` dans le conteneur ; résolu par `BroadcastingWsHandler`. |
| `suprnova::broadcasting::Broadcastable` | Trait sur `Event + Serialize`. Requis : `broadcast_on()`. Optionnel : `broadcast_event_name`, `broadcast_with`, `broadcast_when`, `broadcast_to_others`. |
| `suprnova::broadcasting::BroadcastHub` | Trait de hub. `subscribe`, `publish`, `subscriber_count`, track/untrack/list de présence. |
| `suprnova::broadcasting::InMemoryBroadcastHub` | Hub in-process par défaut. Aucune dépendance externe. Publish retourne `Ok` inconditionnellement. |
| `suprnova::broadcasting::RecordingBroadcastHub` | Doublure de test. Enregistre chaque publish ; livre quand même aux abonnés actifs. |
| `suprnova::broadcasting::BroadcastEnvelope` | Un événement publié : `channel`, `event`, `data`, `except`. Builder `new(ch, ev, data)` ; `.with_except(socket_id)` pour une exclusion par dispatch. |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | Les types réseau à enveloppe JSON. `ServerFrame::Lagged { channel, skipped }` fait remonter les dépassements de ring buffer par canal. |
| `suprnova::broadcasting::BroadcastingWsHandler` | Le `WebSocketHandler` réutilisable du framework. Constructeur : `BroadcastingWsHandler::new(hub, registry)`. À passer à `ws!()`. |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | Hub cross-process derrière `broadcasting-fanout`. `new(uri, stream_key)`, `new_with_presence_ttl(uri, key, ttl)`, `new_loopback(uri, key)`. |
| `EventFacade::broadcast::<E>(hub)` | Enregistre le pont événement → hub pour `E`. À appeler une fois par `Broadcastable` à l'amorçage. |
| `EventFacade::dispatch(event)` | Déclenche les écouteurs in-process ET publie sur le hub sur chaque canal que `E::broadcast_on()` retourne. |
| `WsRouteDef::config(WsConfig)` | Surcharge de config WS par route. Se compose avec `.middleware(M)` dans les deux ordres. |
| `WsRouteDef::middleware(M)` | Chaîne de middleware par route. Une réponse non-2xx court-circuite la mise à niveau. |
| `WsConfig::generous()` | Factory pour flux de confiance : message 64 Mio / frame 16 Mio, autres champs inchangés. N'utilisez PAS sur des routes publiques. |

## Suivant

- [WebSockets](websockets.md) - la primitive sous-jacente, `WsSocket`, `OriginPolicy`
- [Événements](events.md) - `EventFacade`, dispatch fail-fast vs best-effort
- [Événements serveur](sse.md) - push unidirectionnel sans handshake Upgrade
- [Notifications](notifications.md) - le driver de notification `BroadcastChannel`
- [Web Push](web-push.md) - notifications poussées par le serveur vers les utilisateurs hors ligne
