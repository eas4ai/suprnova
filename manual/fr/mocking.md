# Mocking et doublures

Chaque surface externe de Suprnova est livrée avec un fake en cours de
processus qui capture ce que votre code aurait envoyé - mail,
notifications, jobs mis en file d'attente, commandes dispatchées,
événements déclenchés, fichiers écrits, appels HTTP sortants - et un
jeu d'assertions assorti que vous exécutez après coup. La forme est
toujours la même : installez le fake, exécutez le code sous test,
affirmez ce qui a été capturé. Ce chapitre est la vue d'ensemble
consolidée ; chaque chapitre de sous-système ([E-mail](mail.md),
[Notifications](notifications.md), [File d'attente](queues.md),
[Bus](bus.md), [Événements](events.md),
[Système de fichiers et stockage](filesystem.md),
[Client HTTP](http-client.md)) couvre son fake en profondeur.

## Les sept fakes

| Surface         | Point d'entrée                                    | Style d'assertion                     | Sûreté en parallèle                                | Chapitre                              |
|-----------------|---------------------------------------------------|----------------------------------------|----------------------------------------------------|--------------------------------------|
| Mail            | `Mail::fake()` → garde `MailFake`                 | méthodes sur la garde                  | a besoin de `#[serial]` - transport global, pas de sérialiseur | [mail.md](mail.md)                   |
| Notifications   | `Notify::fake()` → `NotifyFakeGuard`              | fonctions libres dans `notifications::testing` | la garde détient un sérialiseur à l'échelle du processus | [notifications.md](notifications.md) |
| Queue           | `suprnova::queue::testing::install_fake()`        | fonctions libres dans `queue::testing`    | la garde détient un sérialiseur à l'échelle du processus | [queues.md](queues.md)               |
| Bus             | `suprnova::bus::testing::install_fake()`          | fonctions libres dans `bus::testing`      | la garde détient un sérialiseur à l'échelle du processus | [bus.md](bus.md)                     |
| Events          | `EventFacade::fake()` → `EventFakeGuard`          | fonctions libres dans `events`            | la garde détient un sérialiseur à l'échelle du processus | [events.md](events.md)               |
| Storage         | `Storage::fake()` → `StorageFakeGuard`            | méthodes `DiskAssertExt` sur un disque     | la garde détient un sérialiseur à l'échelle du processus | [filesystem.md](filesystem.md)       |
| Client HTTP     | `Http::fake(\|\| async { … }).await`              | `assert_sent` / `assert_not_sent`     | task-local - véritablement concurrent entre les tests | [http-client.md](http-client.md)     |

Quelques invariants tiennent à travers les sept :

- **Le fake enregistre, le vrai backend ne s'exécute pas.** Le mail
  n'est pas envoyé, les jobs ne sont pas poussés vers le driver, les
  handlers ne s'exécutent pas, les événements sautent leurs
  écouteurs, HTTP n'atteint pas le réseau, les écritures de fichier
  vont vers un disque en mémoire. Le côté capturé porte assez
  d'information pour affirmer ce qui se serait passé.
- **La garde est RAII.** Abandonner la garde restaure ce qui était en
  place avant (le transport mail précédent, un registre de stockage
  propre, aucun enregistrement pour les événements, etc.). Les tests
  n'ont pas besoin d'une étape de démontage.
- **Le fake ne ment pas sur les erreurs.** Si votre code appelle
  `Bus::dispatch` pour une commande non enregistrée, le fake retourne
  quand même `Err(_)` - seuls les dispatches réussis sont capturés.

## Les formes, et pourquoi elles diffèrent

Trois motifs se répètent. Savoir quel motif un fake utilise vous dit
si vous devez importer une fonction libre, appeler une méthode sur la
garde, ou envelopper le corps du test dans une closure.

### Garde avec méthodes (Mail)

`Mail::fake()` retourne un `MailFake` dont les propres méthodes sont
les assertions. C'est pratique quand l'affirmeur est *le* fake
lui-même - vous l'avez déjà lié à une locale - mais c'est le seul
fake dans cette forme :

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### Garde plus fonctions libres (Notify, Queue, Bus, Events)

La garde est un jeton sans effet dont le seul travail est de garder le
fake installé ; les assertions vivent dans un sous-module `testing` à
côté des internes du fake. Importez ce dont vous avez besoin :

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

C'est la forme la plus courante parce qu'elle généralise proprement à
travers les types - chaque assertion est générique sur `J: Job` /
`C: Command` / `E: Event` plutôt que d'être figée dans un type de
garde. Le compromis est un import supplémentaire.

### Portée avec closure (HTTP)

`Http::fake` est l'exception. Le HTTP sortant s'exécute sur quelle que
soit la tâche Tokio qui se trouve être vivante, donc l'état du fake
vit dans un `tokio::task_local!`. Vous ne pouvez pas l'installer une
fois et laisser filer - vous devez envelopper le corps qui appelle le
client :

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

Le gain : chaque autre fake détient un sérialiseur à l'échelle du
processus, si bien que les tests parallèles s'exécutent un par un,
mais `Http::fake` est véritablement concurrent - chaque test obtient
son propre enregistreur task-local et ils n'entrent jamais en
collision.

### Le trait d'extension de Storage

`Storage::fake()` retourne une garde *et* un disque en mémoire par
défaut, mais ses assertions dépendent du disque lui-même à travers le
trait d'extension `DiskAssertExt` :

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

Le trait d'extension est verrouillé sur
`#[cfg(any(test, feature = "testing"))]` pour que le code de
production ne puisse pas appeler accidentellement
`disk.assert_exists(…)`.

## Sûreté en parallèle, en un paragraphe

Six des sept fakes gardent un statique global au processus. Chaque
garde, à sa construction, prend un `std::sync::Mutex` `FAKE_SERIAL`
dédié et le détient jusqu'à l'abandon. L'effet est que deux
`#[tokio::test]` quelconques qui installent le même fake s'exécutent
sérialisés sous un seul processus - pas besoin de `#[serial]` de la
crate [serial_test](https://crates.io/crates/serial_test). **Mail est
l'exception** : la garde `MailFake` échange le `TRANSPORT` global sans
prendre de sérialiseur, donc des tests `Mail::fake()` concurrents
*s'écraseraient* les uns les autres. Marquez-les `#[serial]`.
**`Http::fake` est aussi une exception** : c'est task-local, pas
global au processus, donc les tests s'exécutent véritablement en
parallèle et n'ont jamais besoin de `#[serial]`.

Si vous entrelacez du dispatch réel avec du dispatch faké pour la même
surface à l'intérieur d'un seul binaire de test, le chemin réel ne
prend pas le sérialiseur, donc il peut entrer en course avec un test
faké parallèle. Marquez les tests à dispatch réel `#[serial]` dans ce
cas - la documentation par chapitre le signale là où ça s'applique
(voir [Bus](bus.md) pour l'exemple canonique).

## Mail - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| Assertion                                  | Affirme…                                             |
|--------------------------------------------|-------------------------------------------------------|
| `fake.assert_sent(\|m\| pred)`             | au moins un message capturé correspond                |
| `fake.assert_sent_to("…")`                 | au moins un message capturé a été routé vers cet e-mail |
| `fake.assert_not_sent(\|m\| pred)`         | aucun message capturé ne correspond                    |
| `fake.assert_not_sent_to("…")`             | aucun message capturé n'est allé vers cet e-mail       |
| `fake.assert_sent_count(n)`                | exactement `n` messages capturés                       |
| `fake.assert_nothing_sent()`               | rien n'a été capturé                                   |
| `fake.assert_queued("MailableName")`       | au moins un mailable en file d'attente de ce nom       |
| `fake.assert_queued_with(name, \|q\| …)`   | un mailable en file d'attente correspond au prédicat   |
| `fake.assert_queued_to("…")`               | un mailable en file d'attente a été routé vers cet e-mail |
| `fake.assert_not_queued("MailableName")`   | aucun mailable en file d'attente de ce nom              |
| `fake.assert_queued_count(n)`              | exactement `n` mailables en file d'attente              |
| `fake.assert_nothing_queued()`             | rien n'a été mis en file d'attente                      |
| `fake.assert_outgoing_count(n)`            | envoyés + en file d'attente totalisent `n`              |
| `fake.assert_nothing_outgoing()`           | rien n'a été envoyé et rien n'a été mis en file d'attente |

`fake.captured()`, `fake.queued()`, `fake.sent(pred)`, `fake.sent_to(…)`,
`fake.queued_named(…)`, et `fake.queued_to(…)` retournent les données
correspondantes pour que vous puissiez construire des assertions sur
mesure. Voir [E-mail](mail.md) pour la surface complète, y compris
comment `Mail::queue` est reflété dans le fake même quand
`Queue::fake` n'est pas installé.

## Notifications - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| Assertion                                            | Affirme…                                          |
|--------------------------------------------------------|---------------------------------------------------|
| `assert_sent(\|r\| pred)`                            | au moins une notification dispatchée correspond    |
| `assert_sent_to(route, "Name")`                      | la notification nommée est allée vers cette route par canal |
| `assert_sent_to_on(route, channel, "Name")`          | dispatchée sur ce canal vers cette route           |
| `assert_sent_named("Name")`                          | la notification nommée dispatchée sur n'importe quel canal |
| `assert_sent_times("Name", n)`                       | exactement `n` de la notification nommée           |
| `assert_nothing_sent()`                              | aucune notification dispatchée                     |
| `assert_count(n)`                                    | exactement `n` au total, tous types et canaux confondus |
| `assert_nothing_sent_to(route)`                      | rien dispatché vers cette route                    |

`testing::recorded()` retourne chaque `FakeRecord` (nom de
notification, canal, route, données JSON) pour des assertions plus
fines. Les destinataires de notification sont indexés sur la valeur
`route_for` par canal, donc `assert_sent_to` prend la chaîne de route
(une adresse e-mail pour `"mail"`, l'id-en-chaîne pour `"database"`,
…) - voir [Notifications](notifications.md) pour le modèle de
routage.

## Queue - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| Assertion                                      | Affirme…                                                        |
|--------------------------------------------------|------------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)`               | au moins un push de `J` correspond                                |
| `assert_pushed_later::<J>(\|j, at\| pred)`     | un push de `J` a été planifié pour `at` (dispatch différé)        |

Le côté données retourne les jobs typés eux-mêmes :

- `pushed::<J>() -> Vec<J>` - chaque push capturé de `J`
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` - la
  même chose, avec l'horodatage planifié de chaque job

Chaque `Queue::push`, `Queue::push_later`, `Queue::later`,
`Queue::push_unique*`, et les dispatchers de chaîne/batch se
canalisent tous vers le même enregistreur. Voir [File d'attente](queues.md)
pour la sémantique de `push_unique` sous le fake (il enregistre
toujours et rapporte « pushed »).

## Bus - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| Assertion                                           | Affirme…                                                       |
|-----------------------------------------------------|-------------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)`                | au moins une commande dispatchée de `C` correspond                 |
| `assert_not_dispatched::<C>(\|c\| pred)`            | aucune commande dispatchée de `C` ne correspond                    |
| `assert_dispatched_times::<C>(\|c\| pred, n)`       | exactement `n` commandes dispatchées de `C` correspondent          |
| `assert_nothing_dispatched()`                       | zéro commande de quelque type que ce soit dispatchée sous le fake actif |

Sous le fake, `Bus::dispatch` retourne `Ok(Dispatched::Captured)` au
lieu d'exécuter le handler. Les vraies erreurs - échecs
d'encodage/décodage, aucun handler enregistré avant que le fake ne
soit installé - remontent toujours sous forme d'`Err(_)`. Voir
[Bus](bus.md).

## Events - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Assertion                              | Affirme…                                          |
|-------------------------------------------|-----------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)`   | au moins un `E` dispatché correspond               |
| `assert_dispatched_once::<E>()`        | exactement un `E` a été dispatché                  |
| `assert_dispatched_times::<E>(n)`      | exactement `n` `E` ont été dispatchés              |
| `assert_not_dispatched::<E>(\|e\| ..)` | aucun `E` correspondant n'a été dispatché          |
| `assert_nothing_dispatched()`          | aucun événement de quelque type dispatché          |
| `assert_listening::<E, L>()`           | l'écouteur `L` est enregistré pour `E`             |
| `has_dispatched::<E>()`                | `bool` : si un `E` quelconque a été enregistré     |
| `dispatched::<E>(\|e\| pred)`          | clones `Vec<E>` des événements correspondants      |
| `dispatched_count::<E>(\|e\| pred)`    | nombre d'événements correspondants                 |
| `dispatched_events()`                  | `HashMap<&'static str, usize>` de tous les dispatches |

Deux variantes restreignent ce qui est faké :

```rust,ignore
// Ne fake que ces événements ; tout le reste dispatche normalement.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Fake tous les événements SAUF ceux-ci.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Et une variante supprime sans enregistrer :

```rust,ignore
EventFacade::muted(async {
    // Aucun écouteur ne se déclenche, aucun événement n'est enregistré.
    run_bulk_import().await;
})
.await;
```

`muted` n'acquiert PAS le sérialiseur, si bien que les portées muted
peuvent s'exécuter en parallèle. Voir [Événements](events.md) pour la
machinerie complète, y compris `assert_listening` (qui observe
seulement les enregistrements d'écouteur qui se produisent *à
l'intérieur* de la portée du fake).

## Storage - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

La garde pré-enregistre un disque en mémoire `"default"`, donc les
tests triviaux n'ont besoin d'aucune configuration de disque.
Enregistrez des disques supplémentaires sous des noms personnalisés
avec `Storage::register_memory("audit_logs")` depuis l'intérieur du
test si le code sous test fait appel à un disque non par défaut.

| Assertion                                        | Affirme…                                          |
|-----------------------------------------------------|------------------------------------------------------|
| `disk.assert_exists(path).await`                 | le chemin existe                                   |
| `disk.assert_contents(path, &expected).await`    | le fichier correspond à `expected` octet pour octet |
| `disk.assert_missing(path).await`                | le chemin n'existe pas                             |
| `disk.assert_count(dir, n, recursive).await`     | `dir` contient exactement `n` entrées              |
| `disk.assert_directory_empty(dir).await`         | `dir` n'a aucune entrée (récursif)                 |

Les cinq paniquent en cas de non-correspondance avec le chemin du
disque dans le message. Voir [Système de fichiers et stockage](filesystem.md)
pour la façade `Storage` elle-même et l'histoire des drivers (memory /
fs / s3 / azblob / gcs).

## Client HTTP - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` met en file
d'attente une réponse prédéfinie. La méthode `"*"` correspond à
n'importe quelle méthode. Chaque entrée prédéfinie est consommée à la
première requête correspondante ; les requêtes correspondantes
suivantes retombent soit sur l'entrée prédéfinie suivante, soit
retournent un `200 {}` vide.

| Helper                                       | Objet                                                      |
|----------------------------------------------|--------------------------------------------------------------|
| `Http::fake(\|\| async { … }).await`         | installe la portée du fake task-local                       |
| `fake_response(method, url_substring, …)`    | met en file d'attente une réponse prédéfinie                 |
| `assert_sent(\|r\| pred)`                    | affirme qu'au moins une requête enregistrée correspond        |
| `assert_not_sent(\|r\| pred)`                | affirme qu'aucune requête enregistrée ne correspond            |

### Les tâches spawnées n'héritent pas du fake par défaut

`tokio::spawn` ne fait pas transiter les task-locals dans la future
spawnée, donc le travail qui s'échappe de la tâche parente s'échappe
aussi du fake. Deux outils gèrent cela :

```rust,ignore
// Ceinture et bretelles : transforme chaque appel sortant non faké en une erreur dure.
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // Opt-in explicite : cet enfant voit l'état de fake du parent.
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` est RAII - installez-le en tête d'un test et
tout appel sortant qui n'atteint pas un fake actif échoue avec une
erreur au lieu de toucher le réseau. `Http::spawn_with_fake_inheritance`
est l'opt-in explicite pour les tâches qui doivent partager l'état de
fake du parent. Voir [Client HTTP](http-client.md) pour la discussion
complète.

## Diffusion

La diffusion WebSocket a un dispositif de test parallèle, mais sa
forme diffère assez pour vivre dans son propre chapitre :
`RecordingBroadcastHub` est un vrai `BroadcastHub` qui enregistre
chaque enveloppe publiée tout en continuant à livrer aux abonnés
actifs. Liez-le à la place d'`InMemoryBroadcastHub` et appelez
`hub.broadcasts()` / `hub.assert_broadcast(channel, event)`. Voir
[Diffusion](broadcasting.md) pour le modèle de diffusion et l'usage
du hub d'enregistrement.

## Où réside chaque fake

| Surface       | Source                                | Réexport de la façade                         |
|---------------|---------------------------------------|----------------------------------------------|
| Mail          | `framework/src/mail/mod.rs`           | `suprnova::{Mail, MailFake}`                 |
| Notifications | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| Queue         | `framework/src/queue/testing.rs`      | `suprnova::queue::testing::*`                |
| Bus           | `framework/src/bus/testing.rs`        | `suprnova::bus::testing::*`                  |
| Events        | `framework/src/events/testing.rs`     | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| Storage       | `framework/src/filesystem/testing.rs` | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP          | `framework/src/http_client/fake.rs`   | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

Les modules `testing` et `fake` sont verrouillés derrière une feature
Cargo nommée `testing`. Elle est dans l'ensemble de features par
défaut, donc tout test qui dépend de `suprnova` récupère les helpers
gratuitement. Les hooks eux-mêmes sont `#[doc(hidden)]` là où ils
pourraient être atteints accidentellement depuis du code
d'application ; le garde-fou porteur est la validation `APP_KEY` de
`Server::from_config`, qui s'exécute à chaque amorçage indépendamment
des helpers de test compilés. Voir [Tests](testing.md) pour l'histoire
des builds de production.

## Pourquoi ces formes, pas une seule forme

Une forme uniforme unique serait plus nette sur la page et pire en
pratique. Chaque forme existe parce que l'état sous-jacent a des
sémantiques de concurrence différentes :

- Le transport de **Mail** est un `Arc<dyn MailTransport>` global
  échangé par la garde. Les assertions en méthode sur la garde
  retournée lient l'affirmeur à l'installation spécifique, ce qui rend
  impossible d'appeler des assertions quand aucun fake n'est actif.
- **Notify / Queue / Bus / Events** affirment sur des charges utiles
  typées hétérogènes - chaque assertion est générique sur le type
  événement/job/commande. Les fonctions libres dans un module
  `testing` se composent avec des paramètres de type plus proprement
  qu'un jeu de méthodes écrit à la main sur une garde.
- Les assertions de **Storage** sont par disque, pas par fake - le
  même `disk.assert_exists(…)` fonctionne contre un disque en mémoire
  faké ou un vrai disque `s3` dans une suite d'intégration. Les poser
  sur le disque via un trait d'extension préserve cette symétrie.
- **HTTP** doit suivre les tâches, pas la pile d'appel. `Http::fake`
  est le seul fake dont la portée ne peut pas s'exprimer comme une
  garde - la sémantique de spawn impose une closure.

Si jamais vous vous retrouvez à chercher un helper qui n'existe pas,
lisez le chapitre pertinent ; la surface de test publique est
documentée de façon exhaustive par sous-système.

## Suivant

- [Tests](testing.md) - la macro `#[suprnova_test]`, `TestDatabase`,
  `expect!`, et `TestContainer::fake`
- [Tests HTTP](http-tests.md) - piloter `handle_request` directement
  sans ouvrir de socket
- [Tests de base de données](database-testing.md) - l'histoire de la
  base de données en mémoire par test
- [Conteneur de service](container.md) - `TestContainer::fake` pour
  échanger des services injectés
