# Notifications

Une notification est un petit message que vous voulez qu'un
utilisateur (ou « n'importe qui ayant une adresse e-mail ») reçoive
sur un ou plusieurs canaux - mail, boîte de réception in-app, push
navigateur, WebSocket en temps réel - depuis un seul site d'appel.
Vous écrivez `Notify::send(&user, &OrderShipped { … })` ; le
dispatcher répartit cette notification unique sur chaque canal que la
notification a déclaré, en adressant chacun via le destinataire.

Utilisez les notifications quand le *quoi* (une commande a été
expédiée, une facture a été payée) intéresse davantage votre code que
le *comment* (quel transport a fini par la livrer). Pour un accès
direct au transport - composer un corps de mail personnalisé, publier
sur un canal de diffusion spécifique, envoyer un web push ponctuel -
passez directement par [mail](mail.md), [diffusion](broadcasting.md),
ou [web push](web-push.md).

## Démarrage rapide

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // macro derive
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` dispatche à la fois vers le canal mail et le canal
database dans un seul appel. Le destinataire décline un canal en
retournant `None` depuis `route_for` - utile pour les utilisateurs
« e-mail seulement » ou « push seulement ».

## Les trois traits

| Trait | Ce qu'il représente | Implémenté par |
|---|---|---|
| `Notification` | Un message typé + les canaux vers lesquels il dispatche | Vos structs de notification |
| `Notifiable` | Un destinataire - expose un `route_for` par canal | Votre `User`, `Order`, tout ce qui est adressable |
| `Channel` | Un transport - sait comment livrer vers une route | Intégrés : `MailChannel`, `DatabaseChannel`, `BroadcastChannel`, `WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

Le destinataire possède l'adressage par canal. `route_for("mail")`
retourne l'adresse e-mail ; `route_for("database")` retourne l'id de
l'entité sous forme de chaîne ; `route_for("webpush")` retourne un
JSON `SubscriptionInfo` sérialisé ; `route_for("broadcast")` retourne
le nom du canal de diffusion. Retournez `None` pour ignorer un canal
pour ce destinataire.

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }
}
```

| Méthode | Objet |
|---|---|
| `notification_name()` | Identifiant stable persisté par le canal database, utilisé comme clé d'enveloppe de file d'attente, et comme clé de recherche pour le registre de renderers mail. |
| `channels(&self)` | Noms des canaux vers lesquels cette notification dispatche. L'ordre est l'ordre d'itération. |
| `data(&self)` | Payload sérialisable en JSON que les canaux livrent / persistent. Typiquement `serde_json::to_value(self)` du sous-ensemble de champs dont les canaux ont besoin. |
| `should_send(&self, channel)` | Veto par canal consulté à la fois sur le chemin synchrone et sur le chemin en file d'attente. Retourner `false` ignore ce canal pour ce dispatch. Défaut : toujours envoyer. |
| `after_sending(&self, channel)` | Hook post-succès invoqué une fois par canal qui s'est terminé, à la fois sur le chemin synchrone et sur le chemin en file d'attente. Retourner `Err` se propage de la même façon qu'une erreur de canal. Défaut : sans effet. |

`should_send` et `after_sending` sont honorés sur les **deux**
chemins. `Notify::send` les consulte dans le dispatcher ;
`Notify::queue` vérifie `should_send` avant de mettre en file
d'attente chaque job par canal, et le worker revérifie `should_send`
avant la livraison (l'état peut changer entre la mise en file
d'attente et l'exécution) et exécute `after_sending` après un envoi
réussi. Les trois *événements* de cycle de vie (`NotificationSending`
/ `NotificationSent` / `NotificationFailed`) ne se déclenchent quand
même que sur le chemin synchrone.

## Canaux

### E-mail

Le canal mail livre via le transport mail lié (voir
[E-mail](mail.md)). Une notification y participe en implémentant
`NotificationMailable` :

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` est l'enveloppe de rendu - `subject` (requis), `html`
et/ou `text` (au moins un requis), et en option `from`, `cc`, `bcc`,
`reply_to`, et `attachments`. Le canal mail assemble un message
sortant à partir de ce rendu plus le `route_for("mail")` du
destinataire, applique les défauts d'expéditeur configurés
(`Mail::always_from(...)`, `always_to(...)`, etc.), et dispatche via
`Mail::current_transport`.

Si le renderer retourne un rendu sans `html` ni `text`, la livraison
échoue vite - un mail de notification vide n'est jamais envoyé
silencieusement.

#### `#[derive(NotificationMailable)]`

Le derive condense l'`impl` `to_mail` par Notification en un seul
attribut `#[mail(...)]`. Les templates utilisent
[Tera](https://keats.github.io/tera/) ; les champs sérialisés de
`self` sont le contexte.

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

Clés supportées :

| Clé | Requise ? | Objet |
|---|---|---|
| `subject` | oui | Template Tera - rendu avec `self` comme contexte. |
| `html` | dague | Template Tera de corps HTML en ligne. |
| `html_template` | dague | Chemin vers un template Tera de corps HTML (embarqué via `include_str!`). |
| `text` | dague | Template Tera de corps texte brut en ligne. |
| `text_template` | dague | Chemin vers un template Tera de corps texte brut (embarqué via `include_str!`). |
| `from` | non | E-mail de l'expéditeur - remplace le défaut `noreply@localhost`. |
| `from_name` | non | Nom d'affichage. Nécessite `from`. |
| `cc` | non | Liste CC séparée par des virgules. Les espaces et les virgules finales sont ignorés. |
| `bcc` | non | Liste BCC séparée par des virgules. |
| `reply_to` | non | Liste Reply-To séparée par des virgules. |

(dague) Au moins une variante de corps doit être présente. `html` et
`html_template` s'excluent mutuellement ; de même pour `text` et
`text_template`.

Chaque invariant est appliqué à la compilation - un `subject`
manquant, un corps vide, des variantes en conflit, `from_name` sans
`from`, ou des clés inconnues font échouer le build plutôt que
d'échouer au dispatch.

Pour les pièces jointes (payloads binaires) ou les destinataires
dynamiques par instance, implémentez `NotificationMailable` à la main
et construisez le `MailRendering` directement.

### Base de données

Le canal database persiste chaque notification comme une ligne dans
la table `notifications` :

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

Le second argument est le tag de type polymorphe du destinataire (ce
que vous stockez dans `notifiable_type` afin de pouvoir requêter les
lignes de la boîte de réception plus tard). Le `route_for("database")`
du destinataire devient le `notifiable_id`. La migration est livrée
avec le framework
(`framework/migrations/20260516_create_notifications_table.sql`) ;
exécutez `suprnova migrate` et la table apparaît.

#### Lire la boîte de réception

Les helpers de lecture vivent dans `suprnova::notifications` sous
forme de fonctions libres sur `(notifiable_type, notifiable_id)` :

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` porte `id`, `type_name` (le
`Notification::notification_name`), `notifiable_type`, `notifiable_id`,
le `data` JSON décodé, `read_at`, `created_at`, `updated_at`.
`mark_as_read` / `mark_as_unread` sont idempotents (conformément au
contrat de Laravel).

### Web Push

Le canal web push chiffre le payload et le POST vers un point de
terminaison d'abonnement push navigateur stocké, via le client de
signature VAPID du framework :

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL seconds */);
```

Le `route_for("webpush")` du destinataire retourne un JSON
`SubscriptionInfo` sérialisé (la même forme que celle que le
navigateur renvoie depuis `PushSubscription.toJSON()` - stockez-le
tel quel, retournez-le sans y toucher). Le TTL est transmis au
service push.

Quand le service push indique au canal qu'un abonnement a disparu
(HTTP 404/410), le canal journalise un WARN structuré et retourne un
succès - la notification a atteint un état terminal sans destinataire
contre qui réessayer. Les opérateurs voient le journal et suppriment
l'abonnement mort ; la livraison ne produit pas d'erreur.

Voir [Web Push](web-push.md) pour le client complet.

### Diffusion

Le canal broadcast publie chaque notification sur le `BroadcastHub` de
l'application afin que les abonnés WebSocket la reçoivent en temps
réel. Le `route_for("broadcast")` du destinataire est le nom du canal,
le type de la notification est l'événement, et `data()` est le
payload :

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// À l'amorçage - liez le hub avant tout dispatch broadcast.
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

Le canal résout le hub depuis le conteneur au moment de la livraison.
Si aucun `BroadcastHub` n'est lié quand une notification déclare
`"broadcast"`, le canal retourne une erreur - une application mal
configurée fait remonter le problème plutôt que d'abandonner le
message silencieusement. Publier sur un canal sans aucun abonné actif
n'est pas une erreur.

Voir [Diffusion](broadcasting.md) pour la configuration du hub et la
plomberie WebSocket.

## Notifications à la demande

Parfois, vous voulez notifier *quelqu'un qui n'est pas dans votre base
de données* - une alerte ops ponctuelle vers une adresse e-mail, un
récepteur de webhook, un canal de diffusion qu'aucun utilisateur ne
possède. `AnonymousNotifiable` est l'« utilisateur sans ligne » :

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// Plusieurs canaux dans un seul builder :
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` et `Notify::routes([..., ("database",
…)])` retournent `Err` - le canal database persiste une paire
`(notifiable_type, notifiable_id)` qu'un destinataire anonyme ne peut
pas fournir.

## Le dispatcher

`NotificationDispatcher` détient le registre de canaux. Construisez-le
une fois à l'amorçage et liez-le globalement :

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` fonctionne en dernier-écrit-gagne sur le nom du
canal - enregistrer deux canaux nommés `"mail"` remplace
silencieusement le premier. Cela rend les configurations de test
ergonomiques.

Une notification qui déclare un canal que le dispatcher n'a pas
enregistré journalise un WARN (`no channel registered; skipping`) et
continue vers le canal suivant - le dispatch ne produit pas d'erreur
sur un nom de canal inconnu.

`set_dispatcher` retourne `Result<(), FrameworkError>` parce que le
registre du dispatcher vit derrière un `RwLock` ; le chemin d'erreur
ne se déclenche que si le verrou est empoisonné (un writer précédent a
paniqué). En pratique, le site d'appel à l'amorçage utilise `?`.

### Événements de cycle de vie

Trois événements entourent chaque livraison de canal synchrone :

| Événement | Quand | Comportement en cas d'erreur d'écouteur |
|---|---|---|
| `NotificationSending` | Immédiatement avant que le canal ne s'exécute | Un `Err` d'écouteur met son **veto** sur le canal pour ce dispatch |
| `NotificationSent` | Après une livraison réussie | Dispatch best-effort - les erreurs d'écouteur ne se propagent pas |
| `NotificationFailed` | Quand un canal a retourné une erreur | Dispatch best-effort ; l'erreur de canal sous-jacente se propage quand même selon le contrat d'arrêt à la première erreur |

Les trois portent `(notification, channel, route, data)`. `Failed`
ajoute l'`error` sous forme de chaîne. Écoutez avec
`EventFacade::listen::<E, L>` - voir [Événements](events.md).

Ces événements ne se déclenchent que sur le chemin synchrone
`Notify::send`. Le worker en file d'attente livre les canaux
directement sans dispatcher les événements.

### Télémétrie

`NotificationDispatcher::notify` enveloppe le fan-out dans un span de
tracing `notification.dispatch` :

- `notification` - `Notification::notification_name()`
- `channel_count` - nombre de canaux déclarés
- `duration_ms` - latence du fan-out à la complétion
- journal terminal : `notification dispatched` (info) ou
  `notification dispatch failed` (warn)

Le canal mail imbrique son propre span `mail.send` à l'intérieur.

### Contrat d'arrêt à la première erreur

`Notify::send` retourne à la première erreur de canal. Les canaux qui
ont déjà réussi ne font pas de rollback ; les canaux qui n'ont pas
encore tourné ne sont pas tentés. Le même contrat s'applique au worker
en file d'attente.

Pour de l'au-moins-une-fois à travers plusieurs canaux, dispatchez
chaque canal via son propre appel `Notify::queue` - les clés
d'idempotence de l'enveloppe de la file d'attente protègent contre
les doubles envois au réessai.

## Livraison en file d'attente

`Notify::send` s'exécute in-process. `Notify::queue` pousse un
`SendNotificationJob` sur la [File d'attente](queues.md), en
pré-résolvant les routes par canal depuis le destinataire afin que le
worker n'ait pas besoin d'un handle `Notifiable` au moment de
l'exécution :

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// À l'amorçage - une fois par notification concrète atteignable via Notify::queue.
register_notification_factory::<OrderShipped>()?;

// N'importe où :
Notify::queue(&user, OrderShipped { tracking }).await?;
```

Au moment du dispatch, le worker :

1. Recherche la factory de notification par `notification_name`
2. Reconstruit la notification typée à partir du payload JSON
3. Itère sur les canaux enregistrés au moment de la mise en file d'attente
4. Pour chacun, revérifie `should_send(channel)` (en ignorant les
   canaux mis en veto), recherche le canal sur le dispatcher lié,
   appelle `deliver(route, &notification)`, puis exécute
   `after_sending(channel)`

Les canaux qui ont été déclarés au moment de la mise en file
d'attente mais qui ne sont pas enregistrés quand le worker s'exécute
journalisent un WARN et sont ignorés - même contrat que le chemin
synchrone. Les canaux sans route pré-résolue sont ignorés
silencieusement (le destinataire a retourné `None` au moment de la
mise en file d'attente).

`Notify::queue` évalue aussi `should_send` au moment de la mise en
file d'attente, si bien qu'un canal mis en veto n'est jamais mis en
file d'attente en premier lieu ; la revérification du worker couvre
l'état qui change entre la mise en file d'attente et l'exécution. Le
chemin en file d'attente **ne déclenche pas** les trois événements de
cycle de vie (`NotificationSending` / `NotificationSent` /
`NotificationFailed`) - ceux-là restent synchrones uniquement. Si vous
dépendez des événements, envoyez via `Notify::send`.

### Pourquoi Suprnova diverge

Laravel indexe les notifications en file d'attente sur l'interface
marqueur `ShouldQueue` - le même appel
`Notification::send($user, $notification)` met en file d'attente si la
notification implémente `ShouldQueue`, et envoie en ligne sinon. Le
comportement dépend d'un flag de niveau type au site de la
notification, invisible depuis le site d'appel.

Suprnova rend ce choix explicite à chaque appel : `Notify::send` est
toujours synchrone ; `Notify::queue` est toujours en file d'attente.
Il n'y a pas de commutateur de mode caché. (C'est aussi pourquoi il
n'y a pas de `send_now` - `send` est déjà la version synchrone.)

Le côté destinataire diverge aussi. Le trait `Notifiable` de Laravel
est un mixin qui tire la relation de boîte de réception, les méthodes
`routeNotificationFor*`, et la clé primaire polymorphe. Le
`Notifiable` de Suprnova est délibérément minimal - juste
`route_for(channel) -> Option<String>` - parce que les traits Rust ne
se composent pas par mixin. L'équivalent côté lecture de Laravel est
livré comme des fonctions libres sur `(notifiable_type,
notifiable_id)` (`unread_for`, `mark_as_read`, …), si bien que de
simples structs peuvent être notifiables sans hériter d'une relation
ORM.

## Tests

Deux surfaces fake, répondant à des questions différentes.

### `Notify::fake()` - « une notification a-t-elle été dispatchée ? »

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // mail + database
}
```

Tandis que la garde fake est vivante, `Notify::send` et
`Notify::queue` enregistrent tous les deux le dispatch au lieu
d'exécuter les canaux ou de mettre en file d'attente un job - aucun
canal ne tourne, aucune ligne de file d'attente n'est écrite. Le fake
détient un mutex de sérialisation à l'échelle du processus, si bien
que les tests parallèles ne peuvent pas entrelacer leurs captures ;
laissez la garde `_fake` se détruire à la fin du test pour vider
l'enregistreur.

Utilisez `recorded_notifications()` pour une pleine maîtrise des
données capturées :

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + un vrai `MailChannel` - « la notification s'est-elle *rendue* correctement ? »

`Notify::fake()` court-circuite avant le canal. Pour vérifier que le
corps du mail s'est réellement rendu comme vous l'attendez, pilotez
le vrai canal sous `Mail::fake()` :

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

Les tests qui touchent le dispatcher, le renderer, ou les globales de
transport doivent être `#[serial_test::serial]` - ce sont des statics
globales au processus.

## Bonnes pratiques

### Enregistrez chaque factory et renderer à l'amorçage

`Notify::queue` reconstruit la notification via le registre de
factories au niveau du worker, et `MailChannel` rend via
`register_mail_renderer`. Enregistrez d'avance chaque notification
mettable-en-file-d'attente / mailable :

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // Factories de notification (une par Notification atteignable via Notify::queue).
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // Renderers mail (un par NotificationMailable).
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

Une notification non enregistrée sur la file d'attente remonte comme
`unknown notification: {name}` au moment de l'exécution par le worker
et réessaie via le chemin de lettre morte. Un dispatch `MailChannel`
pour un renderer non enregistré remonte de la même façon une erreur
`register via suprnova::register_mail_renderer::<N>()`.

### Mettez en file d'attente pour les fan-out multi-canaux

Le dispatcher synchrone visite les canaux dans l'ordre et retourne à
la première erreur. Un échec sur le canal n°2 laisse le canal n°1
committé et les canaux n°3+ non tentés. Pour toute notification qui
traverse plus d'un canal, préférez `Notify::queue` afin que le worker
gère les réessais avec backoff et que le dispatch survive à un crash
de processus.

### Rendez les livraisons de canal idempotentes

Les réessais du worker signifient que le même `SendNotificationJob`
peut s'exécuter plus d'une fois. Les canaux intégrés sont
idempotence-friendly : `MailChannel` transmet à des providers qui
dédupliquent typiquement par message-id ; `DatabaseChannel` insère un
UUID frais par exécution (ce qui est le bon comportement pour une
ligne d'audit) ; `WebPushChannel` POST vers un provider qui avale les
doublons. Les canaux personnalisés devraient viser des opérations
idempotentes - des POST HTTP avec des clés de dédup stables côté
client, des upserts plutôt que des inserts à l'aveugle, aucun effet de
bord de type « incrémenter un compteur » sur le chemin de livraison.

### Liez le dispatcher en un seul endroit

`register_channel` fonctionne en dernier-écrit-gagne, si bien que les
tests peuvent substituer un canal réel par un stub dans leur setup.
Gardez la liaison de production dans `bootstrap.rs` et laissez les
tests construire leur propre dispatcher avec les stubs dont ils ont
besoin. N'appelez pas `register_channel` lazily à l'intérieur des
handlers de requête - les écritures sur le verrou global plus la
sémantique dernier-écrit-gagne deviennent surprenantes sous charge
concurrente.

## Référence

| Symbole | Chemin |
|---|---|
| `Notifiable`, `Notification`, `Channel`, `DynNotification` | `suprnova::` |
| `Notify` (facade), `NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`, `NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`, `MailRendering`, `NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`, `StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`, `NotificationSent`, `NotificationFailed` | `suprnova::` |
| `set_dispatcher`, `register_notification_factory` | `suprnova::notifications::` |
| `all_for`, `unread_for`, `read_for`, `mark_as_read`, `mark_as_unread`, `mark_all_as_read`, `delete_for` | `suprnova::notifications::` |
| `assert_sent`, `assert_sent_named`, `assert_sent_times`, `assert_sent_to`, `assert_sent_to_on`, `assert_nothing_sent`, `assert_nothing_sent_to`, `assert_count`, `recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## Suivant

- [E-mail](mail.md) - le transport et la surface `Mailable` sur
  lesquels le canal mail s'appuie
- [Diffusion](broadcasting.md) - le `BroadcastHub` par lequel le
  canal broadcast publie
- [Web Push](web-push.md) - VAPID, chiffrement, stockage des
  abonnements
- [Événements](events.md) - écouter `NotificationSending` / `Sent` /
  `Failed`
- [File d'attente](queues.md) - le worker qui pilote `Notify::queue`
- [Tests](testing.md) - les surfaces fake et les motifs de test sériels
