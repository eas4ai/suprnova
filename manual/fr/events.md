# Événements

Les événements sont le pub/sub typé in-process de Suprnova. Un
contrôleur déclenche `UserRegistered { user_id }` ; un écouteur envoie
un e-mail à l'utilisateur, un autre écrit une ligne d'audit, un
troisième publie une diffusion. Les trois voient le même payload,
s'exécutent dans l'ordre d'enregistrement, et n'ont aucune connaissance
de compilation les uns des autres.

La surface exposée au développeur est la struct `EventFacade`
(réexportée en tant que `suprnova::EventFacade`). La crate réexporte
aussi le *trait* `Event` en tant que `suprnova::Event` - même nom que
la façade de Laravel, mais en Rust le trait est le contrat typé que
chaque payload implémente. Derrière la façade se trouve un unique
`EventDispatcher` global au processus (détenu dans un `OnceLock`) :
les écouteurs enregistrés survivent à la requête qui les a
enregistrés, et les dispatches soit s'exécutent en ligne, soit
spawnent dans un ensemble de tâches borné avec réessai.

## Les bases

```rust
use suprnova::{EventFacade, Event, Listener, FrameworkError, async_trait};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct UserRegistered {
    pub user_id: i64,
}

impl Event for UserRegistered {
    fn event_name() -> &'static str {
        "UserRegistered"
    }
}

pub struct SendWelcomeEmail;

#[async_trait]
impl Listener<UserRegistered> for SendWelcomeEmail {
    async fn handle(&self, e: &UserRegistered) -> Result<(), FrameworkError> {
        // envoie l'e-mail…
        let _ = e.user_id;
        Ok(())
    }
}

// Dans bootstrap.rs :
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// Dans un contrôleur :
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` requiert `Send + Sync + Clone + 'static + Debug` afin qu'un
payload puisse traverser les frontières de tâche (écouteurs en file
d'attente) et que le dispatcher puisse le journaliser. `Listener<E>`
est `Send + Sync + 'static` afin de pouvoir survivre à l'appel
d'enregistrement. Il n'y a pas de `#[derive(Event)]` - le trait a deux
méthodes (`event_name` et `queued`, qui a un défaut), si bien qu'un
impl écrit à la main tient en deux lignes.

## Modes de dispatch

| Méthode | Sémantique |
|---|---|
| `EventFacade::dispatch(event)` | Synchrone, fail-fast - le premier `Err` d'un écouteur abandonne la chaîne |
| `EventFacade::dispatch_best_effort(event)` | Synchrone, exécute-les-tous - retourne le premier `Err` après que chaque écouteur s'est exécuté |
| `EventFacade::dispatch(event)` quand `Event::queued() = true` | Chaque écouteur spawn comme une tâche bornée avec réessai ; l'appel retourne après le spawn |

Utilisez `dispatch` (fail-fast) quand un effet de bord en aval DOIT
observer un succès en amont - la plupart des hooks de cycle de vie de
modèle tombent ici, si bien qu'un observateur qui met son veto sur une
sauvegarde peut court-circuiter. Utilisez `dispatch_best_effort` pour
le fan-out où un écouteur en échec ne devrait pas faire taire les
autres - la plupart des événements d'observabilité tombent ici.

Surchargez la méthode du trait pour opter pour la livraison en file
d'attente :

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

Les écouteurs en file d'attente sont bornés par un sémaphore à
l'échelle du processus. Le plafond par défaut est 256 tâches
concurrentes ; surchargez-le par dispatcher avec
`EventDispatcher::with_concurrency(n)` ou globalement via la variable
d'environnement `EVENT_MAX_CONCURRENCY`. Chaque tâche réessaie
jusqu'à 3 tentatives avec un backoff jitterisé de 100ms à 2s avant
d'abandonner - ce sont des réessais in-process pour défaillance
transitoire, pas le planning de plusieurs minutes de la file d'attente
durable.

## `Subscriber` - regrouper les enregistrements liés

Quand plusieurs écouteurs appartiennent à une même fonctionnalité, un
`Subscriber` les enregistre comme une unité. Reflète le motif
subscriber de l'`EventServiceProvider` de Laravel.

```rust
use suprnova::{EventFacade, EventDispatcher, Subscriber, async_trait};
use std::sync::Arc;

pub struct UserEventSubscriber {
    db: Arc<crate::Db>,
}

#[async_trait]
impl Subscriber for UserEventSubscriber {
    async fn subscribe(self: Arc<Self>, d: &EventDispatcher) {
        let db = self.db.clone();
        d.listen::<UserRegistered, _>(Arc::new(SendWelcomeEmail::new(db.clone()))).await;
        d.listen::<UserDeleted, _>(Arc::new(CleanupUserData::new(db.clone()))).await;
        d.listen::<UserPromoted, _>(Arc::new(NotifyAdmins::new(db))).await;
    }
}

// Dans bootstrap.rs - une ligne par subscriber plutôt que trois par
// écouteur :
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` prend un `Arc<S>` afin que les écouteurs qui ont besoin de
partager un état avec le subscriber puissent cloner l'`Arc` et le
capturer.

## Inspecter et supprimer des écouteurs

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` reflète le `Event::hasListeners($eventName)` de
Laravel. `forget::<E>()` abandonne chaque écouteur enregistré pour ce
type d'événement et retourne le nombre supprimé. Le code de
production a rarement besoin de `forget` - l'enregistrement des
écouteurs se fait normalement une fois à l'amorçage - mais le hot-swap
et le code de test s'en servent.

Les deux méthodes retournent des défauts sûrs quand le verrou du
registre d'écouteurs est empoisonné (`false` et `0` respectivement),
avec un `tracing::error!` journalisé afin que la défaillance soit
observable.

## Push et flush

`push` capture un événement dans un bucket par nom d'événement sans le
déclencher. `flush::<E>()` vide le bucket et dispatche tout dans
l'ordre de capture. Reflète la paire `Event::push` / `Event::flush` de
Laravel.

```rust
// À l'intérieur d'un handler qui fait son travail en deux phases :
EventFacade::push(UserRegistered { user_id: 42 }).await;
// … rendu, validation, davantage de travail …
EventFacade::flush::<UserRegistered>().await?;
```

Les événements poussés ignorent la portée `defer` - ils sont déjà
explicitement différés. `forget_pushed()` abandonne chaque événement
poussé sans le dispatcher, et retourne le nombre abandonné. Reflète
`Event::forgetPushed()`.

## defer - mettre en tampon chaque dispatch à l'intérieur d'un callback

`defer(only, async { … })` exécute le callback avec un tampon
task-local dans la portée. Chaque appel `dispatch` /
`dispatch_best_effort` fait à l'intérieur du callback est capturé et
rejoué après que le callback retourne. Reflète le
`Event::defer($callback, ?$events)` de Laravel.

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // mis en tampon
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // mis en tampon
    Ok(())
})
.await?;
// À ce stade, WorkStarted et WorkFinished se sont tous les deux
// déclenchés dans l'ordre.
// `flush_err` porte la première erreur de dispatch du replay (s'il y
// en a une).
```

Passez `Some(&["EventOne", "EventTwo"])` pour différer SEULEMENT ces
noms d'événements ; tout le reste dispatche en ligne comme
d'habitude. Une erreur de callback court-circuite - les événements
mis en tampon sont abandonnés, l'erreur se propage.

Le tampon de defer est par tâche Tokio, si bien que deux appels
`defer` concurrents ne piétinent pas l'état l'un de l'autre.

## Écouteurs en file d'attente - in-process vs durable

Deux niveaux « en file d'attente » distincts, et le nommage compte :

| Besoin | Tournez-vous vers |
|---|---|
| L'écouteur devrait s'exécuter hors tâche ; perte acceptable en cas de crash | `Event::queued() = true` sur le trait de l'événement |
| Le travail de l'écouteur DOIT survivre à un crash + redémarrage | `QueuedListener<E, J>` (relie événement → job durable) |

`Event::queued() = true` fait que le dispatcher spawn chaque écouteur
comme sa propre tâche Tokio, bornée par un sémaphore de processus,
avec un réessai borné (3 tentatives, backoff jitterisé). Le travail
s'exécute sur ce processus ; un crash abandonne les écouteurs en vol.
Le [vidage à l'arrêt gracieux](#vidage-à-l-arrêt) attend les tâches en
vol jusqu'à une échéance.

`QueuedListener<E, J>` est un écouteur prêt à l'emploi qui construit
un [`Job`](queues.md) à partir de chaque événement et le pousse sur la
file d'attente durable. L'événement se déclenche quand même de façon
synchrone ; l'écouteur ne fait qu'y mettre en file d'attente - ce qui
est rapide - si bien que la latence de requête reste basse. Le job
lui-même survit au crash parce que la file d'attente est durable.

```rust
use suprnova::{EventFacade, QueuedListener};
use std::sync::Arc;

EventFacade::listen::<UserRegistered, _>(Arc::new(
    QueuedListener::<UserRegistered, SendWelcomeEmailJob>::new(|e| SendWelcomeEmailJob {
        user_id: e.user_id,
    }),
))
.await;
```

Le `QueuedListener` a seulement besoin que l'événement soit un
événement synchrone ordinaire - la durabilité vit dans la file
d'attente, pas dans le dispatcher.

## Vidage à l'arrêt

Les écouteurs in-process en file d'attente spawnent dans un `JoinSet`
suivi par le dispatcher. La séquence d'arrêt gracieux du serveur
appelle `EventFacade::drain_queued(timeout)` pour les attendre :

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

Le vidage retourne le nombre encore en cours d'exécution quand
l'échéance s'est écoulée (`0` = entièrement vidé). Les retardataires
après l'échéance sont interrompus afin que l'arrêt ne puisse pas se
bloquer.

## Relier les événements à la diffusion

`EventFacade::broadcast::<E>(hub)` câble un pont d'une ligne entre un
événement dispatché et un `BroadcastHub`. Tout type qui implémente
`Broadcastable` et `Event` peut être diffusé de cette façon ; les
écouteurs reçoivent le payload typé, et les abonnés sur les canaux
nommés reçoivent l'enveloppe de diffusion.

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// Tout dispatch ultérieur est aussi publié sur les canaux déclarés
// par OrderShipped::broadcast_on() :
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

Voir [Diffusion](broadcasting.md) pour le modèle de canal (public /
privé / présence) et le trait `Broadcastable`.

## Événements intégrés

Le framework dispatche un ensemble fixe d'événements depuis ses
propres sous-systèmes. Vous optez en enregistrant des écouteurs ; si
aucun écouteur n'est enregistré, les événements sont sans effet.

| Sous-système | Événements | Dispatché par |
|---|---|---|
| Gestion des erreurs | `ErrorOccurred` | Chaque réponse 5xx (`FrameworkError` retournée ou panique récupérée) |
| Auth (guards) | `Auth\\Attempting`, `Auth\\Authenticated`, `Auth\\Login`, `Auth\\Logout`, `Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| Flux d'auth | `EmailVerified`, `PasswordResetLinkSent`, `PasswordResetCompleted`, `AccountLocked`, `AccountUnlocked`, `TwoFactorEnrolled`, `TwoFactorChallenged`, `TwoFactorChallengeFailed`, `TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| Base de données | `Database\\ConnectionEstablished`, `Database\\QueryExecuted`, `Database\\TransactionBeginning`, `Database\\TransactionCommitted`, `Database\\TransactionRolledBack`, `Database\\DatabaseBusy` | `DbConnection::connect`, helpers `ExecutorChoice`, `DB::transaction` |
| Mail | `Suprnova\\Mail\\MessageSending`, `Suprnova\\Mail\\MessageSent` | `MailBuilder::send` avant/après le transport |
| Notifications | `Suprnova::Notifications::Sending`, `Suprnova::Notifications::Sent`, `Suprnova::Notifications::Failed` | Chaque livraison de canal |
| File d'attente (worker) | `queue::JobQueueing`, `JobQueued`, `JobProcessing`, `JobProcessed`, `JobAttempted`, `JobExceptionOccurred`, `JobFailed`, `JobReleased`, `JobReleasedAfterException`, `JobTimedOut`, `Looping`, `WorkerStarting`, `WorkerStopping`, `WorkerInterrupted` | `Queue::push` / `run_worker` |
| Fonctionnalités | `FeatureUpdated`, `FeatureDeleted` | CRUD `features::admin` |
| Eloquent (par modèle) | 16 événements de cycle de vie - `Retrieved`, `Saving`, `Saved`, `Creating`, `Created`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Restoring`, `Restored`, `ForceDeleting`, `ForceDeleted`, `Replicating`, `Pruning`, `Pruned` - émis sous le sous-module `events::` de chaque modèle | La macro `#[suprnova::model]` les câble dans save/update/delete |

`ErrorOccurred` est le hook dédié pour expédier les exceptions 5xx
vers Sentry, Datadog, Slack, etc. Le dispatch est best-effort et
spawné, si bien qu'un écouteur Sentry cassé ne peut pas faire taire
les autres, et la conversion de réponse ne bloque jamais dessus. Voir
[Modèle d'erreur](error-model.md) pour le contrat complet de
récupération de panique et de conversion.

Les événements de cycle de vie de modèle se déclenchent en fail-fast :
un écouteur `Saving` qui retourne `EventResult::Cancel` (via le trait
`CancellableListener`) abandonne la sauvegarde. Voir [Observateurs
Eloquent et événements de cycle de vie](eloquent.md).

## DB::listen - observer les requêtes

Pour de l'observabilité par requête, vous pouvez enregistrer soit un
`Listener<QueryExecuted>` typé via le dispatcher, soit, plus
couramment, un callback `DB::listen` qui reflète la signature
`DB::listen(function ($q) { ... })` de Laravel :

```rust
use suprnova::DB;
use std::sync::Arc;

DB::listen(Arc::new(|q| {
    tracing::debug!(
        sql = %q.sql,
        time_ms = q.time.as_millis(),
        connection = %q.connection_name,
        "query"
    );
}));
```

Le callback reçoit un `QueryExecuted` portant le SQL, les bindings, la
durée en temps réel, le nom de connexion, la classification
lecture/écriture, et le `Result` final (si bien que les requêtes en
échec sont aussi observables). `QueryExecuted::to_raw_sql()` intègre
les bindings pour la commodité des journaux - format debug, PAS sûr
pour du SQL.

Deux garanties de réentrance et de coût :

- **Garde de réentrance.** Un écouteur qui émet lui-même une requête ne
  redéclenchera pas `QueryExecuted` depuis cette requête imbriquée - le
  dispatcher pose un flag task-local tandis qu'un écouteur s'exécute,
  et l'executor saute l'émission à l'intérieur de cette portée. Un
  écouteur qui journalise vers la base de données ne bouclera pas.
- **Surcoût nul quand personne n'écoute.** L'executor vérifie un
  `query_observation_active()` combiné (un écouteur direct
  quelconque, un `Listener<QueryExecuted>` enregistré quelconque, OU
  le query-log activé) avant de construire le payload de l'événement.
  Quand les trois sont désactivés, tout le chemin d'émission est
  court-circuité.

## Tests - `EventFacade::fake()`

`EventFacade::fake()` substitue le dispatcher global par un
enregistreur. Les événements dispatchés vont dans l'enregistrement au
lieu d'exécuter les écouteurs. Le fake détient un sérialiseur à
l'échelle du processus pour la durée de vie de la garde, si bien que des
`#[tokio::test]` parallèles qui l'utilisent s'exécutent un par un - les
tests n'ont plus besoin de leur propre mutex `serial_test`.

```rust
use suprnova::events::{
    EventFacade, assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_nothing_dispatched, has_dispatched, dispatched, dispatched_events,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Helper | Vérifie |
|---|---|
| `assert_dispatched::<E>(pred)` | au moins un `E` correspondant a été dispatché |
| `assert_dispatched_once::<E>()` | exactement un `E` a été dispatché |
| `assert_dispatched_times::<E>(n)` | exactement `n` `E` ont été dispatchés |
| `assert_not_dispatched::<E>(pred)` | aucun `E` correspondant n'a été dispatché |
| `assert_nothing_dispatched()` | AUCUN événement d'aucun type n'a été dispatché |
| `assert_listening::<E, L>()` | un écouteur `L` a été enregistré pour `E` |
| `has_dispatched::<E>()` | bool : un `E` quelconque enregistré |
| `dispatched::<E>(pred)` | clones `Vec<E>` des événements correspondants |
| `dispatched_count::<E>(pred)` | nombre d'événements correspondants |
| `dispatched_events()` | `HashMap<&'static str, usize>` de tous les dispatches |

### Fake sélectif

```rust
// Ne fake que ces événements ; tout le reste dispatche normalement.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Fake tous les événements SAUF ceux-ci.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Reflète le `Event::fake([…])` et l'`EventFake::except($events)` de
Laravel.

### Mute - écarter les événements sans les enregistrer

`EventFacade::muted(async { … })` exécute le callback avec un flag
task-local « dispatcher silencieux » activé ; chaque événement
dispatché à l'intérieur est écarté sans être enregistré ni invoquer
d'écouteurs. L'analogue Suprnova du `NullDispatcher` de Laravel, borné
à un callback.

```rust
EventFacade::muted(async {
    // Aucun écouteur ne se déclenche, aucun événement n'est enregistré.
    run_bulk_import().await;
})
.await;
```

À la différence de `fake()`, `muted` n'acquiert PAS le sérialiseur de
processus - deux portées muted peuvent s'exécuter en parallèle.

### `assert_listening` - vérifier qu'un écouteur est câblé

À utiliser pour tester le câblage de bootstrap sans déclencher
d'événement :

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

Le fake observe les enregistrements via la méthode `listen` du
dispatcher, si bien que l'enregistrement doit se produire À
L'INTÉRIEUR de la portée du fake - les écouteurs enregistrés avant
`EventFacade::fake()` ne sont PAS vus par `assert_listening`.

## Référence de parité Laravel

Chaque méthode de la façade `Event` et d'`EventFake` de Laravel 13 qui
a un équivalent Rust typé est livrée sous le nom le plus proche. Les
méthodes que Laravel expose et qui ne correspondent pas au Rust typé
sont omises avec une brève note.

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)` (halt arg) | utilisez `dispatch` (fail-fast sur `Err`) |
| `Event::until($event)` | `dispatch` (typé : le premier `Err` arrête) |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()` (garde) |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched` (retourne `Vec<E>`) |
| `EventFake::dispatchedEvents` | `dispatched_events` (map nom → compte) |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards` (`User.*` patterns) | non livré - utilisez des écouteurs typés, ou le trait `Observer<M>` pour les hooks de cycle de vie par modèle |
| `Event::subscribe` (string subscriber) | utilisez le trait typé `Subscriber` |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - même forme, prend `&QueryExecuted` |

### Pourquoi Suprnova diverge

Le dispatcher de Laravel s'appuie sur le runtime au typage par
chaînes de PHP : les événements sont des noms de classe passés comme
chaînes, les écouteurs sont des noms de classe recherchés via le
conteneur, et `Event::listen('User.*', ...)` fonctionne parce que les
wildcards sur des chaînes de nom de classe ont du sens en PHP. En
Rust, l'équivalent de « cet écouteur gère `User.*` » est « cet
écouteur générique sur `E: UserEvent` » - un trait, pas une
correspondance de chaîne. Suprnova abandonne donc les wildcards en
faveur du système de types, et le résultat est que les refactorings
cassés deviennent des erreurs de compilation plutôt que des mauvais
routages à l'exécution.

L'autre divergence, c'est `defer` : le defer de Laravel s'appuie sur
le modèle une-requête-par-processus pour borner la portée du report.
Suprnova sert de nombreuses requêtes concurrentes dans un seul
processus, si bien que le tampon de report est task-local. Deux
appels `defer` concurrents obtiennent chacun leur propre tampon ; les
appels ne peuvent pas se piétiner l'un l'autre, et il n'y a pas d'état
global caché qui puisse fuiter.

## Où réside chaque élément

| Élément | Fichier |
|---|---|
| `Event` trait, `Listener<E>`, `Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`, `EventFacade` (facade struct) | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`, `EventFakeGuard`, `muted` | `framework/src/events/testing.rs` |
| Payloads d'événements intégrés | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| Événements de cycle de vie par modèle | générés par macro dans le sous-module `events::` de chaque modèle |

## Suivant

- [Modèle d'erreur](error-model.md) - `ErrorOccurred` et le chemin de
  conversion des 5xx
- [File d'attente](queues.md) - les jobs durables, le niveau tolérant
  aux crashs ; `QueuedListener` fait le pont vers celui-ci
- [Diffusion](broadcasting.md) - câbler les événements dispatchés vers
  des canaux WebSocket via `EventFacade::broadcast::<E>(hub)`
- [Eloquent API](eloquent.md) - les événements de cycle de vie de
  modèle et le trait `Observer<M>`
- [Base de données](database.md) - `DB::listen` et l'événement
  `Database\\QueryExecuted`
