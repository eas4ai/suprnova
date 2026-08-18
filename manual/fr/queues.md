# File d'attente

La façade `Queue` dispatche du travail en arrière-plan vers un driver
et laisse un processus worker séparé le vider : les handlers HTTP
retournent vite, le gros du travail s'exécute en coulisses.
Tournez-vous vers elle chaque fois qu'une requête bloquerait sinon sur
quelque chose qui peut être fait plus tard - envoyer un mail, appeler
un webhook, générer un rapport. Associez-la à [`Bus`](bus.md) quand
vous voulez que le travail s'exécute *maintenant* dans la tâche
courante et retourne un résultat typé ; associez-la à
[`Events`](events.md) quand vous voulez qu'un seul signal fasse du
fan-out vers plusieurs écouteurs.

## Démarrage rapide

Définissez un job, enregistrez-le une fois à l'amorçage, poussez-le :

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … envoie réellement le mail
        Ok(())
    }
}

// À amorcer une fois (le processus worker et le processus de dispatch en ont tous deux besoin).
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// Pousser depuis un handler :
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

Un processus worker vide le driver configuré jusqu'à l'annulation :

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

Dans une application scaffoldée, le worker est démarré par la
sous-commande `queue:work` du binaire - `cargo run -- queue:work` -
qui exécute le même amorçage que votre serveur HTTP, si bien que les
observateurs et les écouteurs enregistrés dans `bootstrap()` se
déclenchent de façon identique pour les insertions venant d'un handler
de file d'attente.

## Drivers

Cinq drivers sont livrés dans l'arbre. Configurez via la variable
d'env `QUEUE_DRIVER` ou en appelant `Queue::set_driver(...)` de façon
programmatique.

| Driver | À utiliser pour | Forces |
| --- | --- | --- |
| `MemoryQueueDriver` | tests, applications mono-processus | `tokio::time::DelayQueue` pour `available_at`, compatible horloge virtuelle |
| `RedisQueueDriver` | fan-out en production | groupes de consommateurs + `XAUTOCLAIM` + jobs différés adossés à des ZSET |
| `DatabaseQueueDriver` | applications mono-BD | `FOR UPDATE SKIP LOCKED` sur Postgres/MySQL, sérialisé par `BEGIN` sur SQLite |
| `SyncQueueDriver` | dev, CI | exécute le handler en ligne sur `push`, pas de worker |
| `NullQueueDriver` | enveloppes de test | ignore chaque push sans l'exécuter |

`Queue::bootstrap_from_env()` lit `QUEUE_DRIVER` et câble le driver
correspondant ; `Queue::bootstrap_default()` câble toujours le driver
mémoire. Le chemin d'amorçage du serveur appelle l'un des deux pour
vous - la plupart des applications ne configurent que via l'env.

### Configuration par variables d'environnement

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver base de données - DB::init() doit s'exécuter en premier
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

Le driver base de données valide `QUEUE_DB_TABLE` comme un identifiant
SQL à la construction, si bien qu'une valeur d'env malformée fait
échouer l'amorçage plutôt que d'atteindre la composition SQL. Redis
utilise sea-streamer-redis en interne avec `AutoCommit::Disabled` ; le
délai de visibilité est fixé au moment de la construction du groupe de
consommateurs, si bien que l'argument `visibility_timeout` par
extraction est ignoré sur Redis (une divergence documentée par rapport
au contrat du trait, imposée par Redis Streams).

### Pourquoi Suprnova diverge

Laravel route chaque élément mettable-en-file-d'attente à travers le
Bus, en distinguant les jobs `ShouldQueue` au moment du dispatch.
Suprnova sépare les deux : `Bus` pour le travail synchrone qui
retourne un résultat typé, `Queue` pour le travail asynchrone qui
survit à un crash de processus. PHP a besoin du routage implicite
parce que son modèle process-par-requête rend difficile de modéliser
autrement « faire ceci plus tard, dans un autre processus ». Tokio
n'en a pas besoin - le choix explicite entre `Bus::dispatch` et
`Queue::push` est plus clair, plus rapide, et fait apparaître le choix
de durabilité au site d'appel. Voir [`bus.md`](bus.md) pour la
comparaison côte à côte.

## Variantes de push

Chaque variante de push prend une valeur typée `J: Job` et retourne
quand l'enveloppe est commitée au driver - pas quand le handler
s'exécute.

| Méthode | Comportement |
| --- | --- |
| `Queue::push(job)` | met en file immédiatement |
| `Queue::push_later(job, at)` | disponible à un `DateTime<Utc>` précis |
| `Queue::later(delay, job)` | disponible après `delay` à partir de maintenant |
| `Queue::push_unique(job)` | déduplique par `J::unique_id` pendant `J::unique_for`, retourne `Ok(true)` quand l'enveloppe a été poussée, `Ok(false)` quand une clé de déduplication vivante l'a supprimée |
| `Queue::push_unique_later(job, at)` | unique + planifié |
| `Queue::later_unique(delay, job)` | unique + différé |
| `Queue::bulk(vec![job1, job2, ...])` | pousse chaque job (le driver peut utiliser un chemin `bulk` natif) |

`push_unique` exige que la couche de cache soit amorcée - le verrou
de déduplication vit dans [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). Un push en échec
libère la clé de déduplication pour que l'appelant puisse réessayer ;
un push réussi la conserve pendant `J::unique_for` secondes. Le
job doit surcharger `Job::unique_id(&self)` pour retourner
`Some(id)` - `None` retourne une erreur interne.

Le booléen répond à une seule question - « ce job est-il dans la
file ? » - et il y a un troisième cas derrière. Si le bail du verrou
de déduplication est perdu pendant que le push est en vol, le push
aboutit quand même (la couche d'idempotence n'annule jamais un corps
qui a pu déjà avoir un effet) et vous obtenez tout de même
`Ok(true)`, avec un journal de niveau `warn` nommant le job et sa
clé unique. Le job est en file ; ce qui n'est pas prouvé, c'est que
personne d'autre n'a mis le même en file en parallèle. Votre handler
doit de toute façon tolérer la redélivrance, donc cela ne demande
aucun traitement supplémentaire - mais le journal est là parce
qu'une rafale de ces messages signifie que le cache qui soutient
votre verrou de déduplication peine.

## Configuration du job

Redéfinissez les fonctions associées de `Job` pour ajuster le
comportement par impl :

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn max_tries() -> u32 { 5 }                            // défaut : 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // défaut : false (le timeout réessaie)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // défaut : 5 minutes
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* voir « Middleware de job » ci-dessous */]
    }
}
```

## Routage de file d'attente

Par défaut, chaque job va dans une seule file d'attente et chaque
worker la vide entièrement. Une fois que certains jobs sont plus lents
ou plus importants que d'autres, vous voulez des pools de workers
dédiés : un export de longue durée ne devrait pas attendre derrière un
millier d'emails de bienvenue.

Un job peut déclarer où il appartient :

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // connexion par défaut
}
```

… et un opérateur peut surcharger cela de façon centrale, sans toucher
au job :

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

La résolution s'exécute en commençant par la priorité la plus haute :

1. une route enregistrée avec `Queue::route`
2. le `Job::queue` / `Job::connection` propre au job
3. le défaut du driver / global

Passer `None` pour un champ laisse cette dimension inchangée, si bien
que router la connexion d'un job ne perturbe pas la file d'attente
qu'il a déjà déclarée.

Les deux dimensions s'exécutent à des profondeurs différentes
aujourd'hui. La **file d'attente** est honorée de bout en bout -
estampillée sur l'enveloppe, stockée par le driver, filtrée par
`--queue`. La **connexion** résout le *nom* de connexion porté par les
événements de cycle de vie `JobQueueing` / `JobQueued`, ce que voient
les écouteurs et les tableaux de bord ; un unique driver global au
processus reçoit encore chaque push, si bien que router la connexion
d'un job ne sélectionne pas encore un driver différent. Déclarer des
connexions dès maintenant est compatible avec les évolutions futures,
pour le jour où des drivers par connexion arriveront, pas comportemental.

Puis dédiez-lui un worker :

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # vide chaque file d'attente, comme avant
```

Un job sans route appartient à `default`, donc `--queue=default` vide
le travail non routé plutôt que de l'abandonner.

### Pourquoi Suprnova diverge

Le `Queue::route(...)` de Laravel prend une chaîne de classe ;
Suprnova prend le job comme paramètre de type, si bien qu'un job
renommé ou supprimé est une erreur de compilation plutôt qu'une route
qui arrête silencieusement de correspondre.

La divergence la plus importante concerne ce qui se passe quand un
driver ne peut pas filtrer. `QueueDriver::pop_from` **rejette** un
filtre de file d'attente qu'il ne peut pas honorer plutôt que de se
replier sur le vidage de tout. Un worker à qui l'on a dit de ne vider
que `billing` mais qui vide silencieusement toutes les files d'attente
ressemble à un déploiement qui fonctionne jusqu'à ce que le mauvais
pool consomme les mauvais jobs - si bien que la mauvaise configuration
est signalée de manière visible dès la première interrogation. Les
drivers mémoire et base de données filtrent nativement ; un driver qui
ne le fait pas - le driver Redis en est un, puisqu'un unique groupe de
consommateurs de stream n'a pas de stockage par file d'attente -
lèvera une erreur plutôt que de tromper.

### La table `jobs`

`DatabaseQueueDriver` attend ce schéma. La colonne `queue` est ce qui
rend possible le filtrage par `--queue` :

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` est nullable, et un job non routé stocke `NULL` plutôt que
`'default'`. C'est délibéré : une ligne écrite par un binaire plus
ancien est indistinguable d'une ligne non routée écrite par un plus
récent, si bien qu'une flotte à versions mixtes vide le même travail
pendant une mise à niveau progressive.

Ajouter la colonne à une table existante est **requis**, pas seulement
pour le filtrage : `push` nomme la colonne `queue` dans son `INSERT`
que le job soit routé ou non, donc un binaire 0.7.0+ fait échouer
chaque push contre une table qui ne l'a pas. Exécutez d'abord la
migration, puis déployez les binaires - les binaires plus anciens
listent leurs colonnes explicitement et ignorent la nouvelle, donc cet
ordre est sûr :

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### Planifications de backoff

| Variante | Comportement |
| --- | --- |
| `Fixed { secs }` | délai constant par tentative |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × aléatoire dans `[1±jitter]` |
| `Sequence { secs }` | une entrée par tentative ; la dernière entrée se répète une fois épuisée |

Le défaut est
`Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` - de
2 secondes à 5 minutes avec ±25% de jitter.

## Middleware de job

Six middleware sont livrés dans l'arbre, tous à l'image de
`Illuminate\Queue\Middleware\*` :

| Middleware | Comportement |
| --- | --- |
| `WithoutOverlapping` | détient un `Cache::lock` pendant la durée ; libère avec délai en cas de contention |
| `RateLimited` | filtré sur le budget du `RateLimiter` ; libère jusqu'à la réinitialisation de la fenêtre |
| `ThrottlesExceptions` | limite le débit sur les *échecs* consécutifs, pas sur les requêtes |
| `Skip::when(cond)` / `Skip::unless(cond)` | abandonne le job quand la condition est remplie |
| `FailOnException` | promeut les erreurs correspondantes en échecs permanents (pas de réessai) |
| `SkipIfBatchCancelled` | abandonne le job si son batch propriétaire a été annulé |

Câblez-les sur l'impl `Job` :

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` et `RateLimited` ont besoin que le sous-système
de cache soit amorcé (`Cache::init` ou
`App::bind::<dyn CacheStore>(...)` au démarrage).

### Un verrou qui ne se libère pas ne fait pas échouer le job

Si `WithoutOverlapping` ne peut pas libérer son verrou après que le
handler s'est exécuté - le backend de cache a eu un incident, la
connexion a été coupée - il journalise au niveau `warn` et retourne
malgré tout le résultat propre du handler. Le verrou expire alors à
`expire_after`.

C'est délibéré. Au moment où la libération s'exécute, le handler a
déjà commité ses effets de bord : lignes écrites, mail envoyé, charges
effectuées. Rapporter l'échec de la libération comme un échec du job
ferait réessayer le worker et refaire tout cela une seconde fois, ce
qui est un résultat pire qu'une clé de verrou détenue pour son TTL. Un
handler qui a vraiment échoué rapporte quand même son échec -
supprimer l'erreur de libération ne supprime pas celle du handler.

### Le contrat de libération sans consommer de tentative

Le middleware retourne un `JobOutcome` plutôt qu'un `Result<()>`.
Quatre variantes :

- `JobOutcome::Completed` - le handler s'est exécuté, ack.
- `JobOutcome::Released { delay }` - remise en file d'attente après `delay` **sans** incrémenter `attempts`. Utilisé par `WithoutOverlapping`, `RateLimited`. Le worker confie l'opération entière à `QueueDriver::release`, et chaque driver dans l'arbre remet en file d'attente sa propre copie stockée sur place, si bien que le message n'est jamais simultanément réservé et visible, et jamais ni l'un ni l'autre. Le compte de tentatives est préservé sans qu'aucune arithmétique dans le worker ne puisse être en désaccord avec un driver - la copie stockée n'a jamais été incrémentée pour cette exécution.
- `JobOutcome::Failed { reason }` - lettre morte immédiatement, persisté dans le magasin des jobs en échec, pas de réessai.
- `JobOutcome::Deleted` - abandonne la réservation sans lettre morte. Utilisé par `Skip`. Si le job appartenait à un batch, le `pending_jobs` du batch décrémente quand même afin que les callbacks puissent se déclencher.

Ce contrat est ce qui fait que « limité en débit parce que le seau
était plein » se distingue de « échoué parce que le handler a
produit une erreur » dans la comptabilité des réessais, les
métriques, et les événements de cycle de vie.

### Ce qui compte comme une tentative

Il y a deux façons dont un job quitte un worker sans se terminer, et
les deux consomment une tentative :

- **Le handler a échoué** - a retourné `Err`, ou a paniqué jusque dans la limite du framework. Le worker envoie un nack ; le driver remet en file d'attente avec `attempts + 1`.
- **Le worker est mort** - kill OOM, `abort()`, un segfault, `docker kill`, ou le SIGKILL qu'un superviseur envoie quand un arrêt expire. Rien ne clôture quoi que ce soit ; la réservation s'éteint simplement. Quel que soit le worker qui reprend le job, il impute la tentative à ce moment-là.

Le second cas était auparavant gratuit, et c'était une faille plutôt
qu'une faveur : un job qui tue de façon fiable son worker ne pouvait
jamais épuiser `max_tries` et ne pouvait donc jamais devenir une
lettre morte. Il tuait chaque worker qui le réclamait, revenait
identique octet pour octet, et tuait le suivant, aussi longtemps que
quelque chose continuait à redémarrer des workers.

Les trois drivers dans l'arbre l'imputent, car changer `QUEUE_DRIVER`
ne doit pas changer si un job toxique peut être arrêté. `database`
détecte un `reserved_until` expiré ; `memory` l'impute quand le
nettoyeur remet la réservation en visible ; `redis` lit le compte de
livraisons de l'entrée depuis `XPENDING`, puisqu'une entrée de stream
Redis est immuable et que son propre compteur est le seul
enregistrement.

`JobOutcome::Released` est l'exception délibérée - voir le contrat
plus haut. Un job limité en débit par `RateLimited` ne s'est jamais
exécuté, donc il ne doit rien.

**Sur Redis, la reprise a deux horloges.** `--visibility-timeout`
définit combien de temps une entrée doit rester non acquittée avant de
devenir éligible à la reprise ; un second intervalle gouverne la
fréquence à laquelle un consommateur regarde. Le driver lie le second
au premier, si bien qu'un job perdu revient en environ deux fois le
délai configuré plutôt que le délai plus 30 secondes fixes.

**Le budget est vérifié avant que le handler ne s'exécute, pas
seulement à la clôture.** Toute autre décision de lettre morte se
produit après que le handler retourne, ce qui présuppose que le
handler retourne. Un job qui tue son worker ne peut pas atteindre
cette vérification, donc le worker refuse aussi de dispatcher un job
dont les tentatives sont déjà épuisées - il le met en lettre morte à
la place, avant qu'il n'emporte un autre worker avec lui. Sans cela,
compter la tentative ferait seulement grimper un nombre pendant que le
job continuerait de tourner en boucle.

**Ce que cela signifie pour vous.** `attempts` compte les *livraisons
à un worker*, pas les *échecs de handler*. Un worker perdu pour des
raisons sans rapport avec le job - un redémarrage de l'hôte, un OOM
causé par un voisin bruyant - consomme aussi une tentative du budget
de ce job. Laravel se comporte de la même façon. Dimensionnez
`max_tries` en tenant compte de cela, et préférez des handlers
idempotents : la livraison au moins une fois a toujours été le
contrat, et cela fait que le chemin de re-livraison compte honnêtement
plutôt que silencieusement.

## Événements de cycle de vie

Les workers émettent des événements de cycle de vie à la façon de
Laravel via la façade [`Event`](events.md). Les écouteurs reçoivent
l'identité de l'enveloppe (`id`, `job_name`, `attempts`, `max_tries`,
`connection`), pas l'instance de job typée - le worker est à type
effacé sur des payloads JSON. Les erreurs voyagent sous forme de
`String` puisque `FrameworkError` ne dérive pas `Clone`.

| Événement | Se déclenche quand |
| --- | --- |
| `JobQueueing` | avant que l'enveloppe n'atteigne le driver |
| `JobQueued` | après que le driver l'accepte |
| `JobProcessing` | extrait par le worker, sur le point d'être dispatché |
| `JobProcessed` | le handler a retourné `Ok` |
| `JobAttempted` | à chaque clôture terminale (succès, échec, timeout) |
| `JobExceptionOccurred` | le handler a retourné `Err`, va réessayer |
| `JobReleasedAfterException` | la remise en file après erreur a eu lieu |
| `JobReleased` | libération pilotée par le middleware (pas d'échec) |
| `JobFailed` | mis en lettre morte |
| `JobTimedOut` | délai d'attente par tentative dépassé |
| `Looping` | à chaque itération de boucle (avant l'extraction) |
| `WorkerStarting` / `WorkerStopping` | une fois par durée de vie du worker |
| `WorkerInterrupted` | signal `Queue::restart()` observé |

Abonnez-vous avec l'API normale `Event::listen`. Les événements sont
best-effort - `Event::dispatch` sans écouteur est un `Ok(())` sans
effet, si bien que les workers dans les déploiements sans
`Event::init()` ne paient rien.

## Stockage des jobs en échec

Les jobs mis en lettre morte atterrissent dans le `FailedJobStore`
configuré :

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// Dans l'outillage admin :
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

Trois backends :

- `MemoryFailedJobStore` - `Vec` in-process, perdu au redémarrage.
- `DatabaseFailedJobStore` - persiste dans une table `failed_jobs` via SeaORM.
- `NullFailedJobStore` - abandonne chaque enregistrement. À l'image du `NullFailedJobProvider` de Laravel.

### Quand le magasin rejette un enregistrement

Si le magasin configuré retourne une erreur, le worker journalise au
niveau `error` et **laisse la réservation intacte** plutôt que
d'acquitter. Le job revient à l'expiration de la visibilité et est
réessayé - il n'est pas abandonné silencieusement.

C'est délibéré. L'alternative, acquitter malgré tout, jette un job qui
a déjà épuisé ses tentatives *et* n'a pas pu être enregistré nulle
part, ce qui est irrécupérable. Un job qui continue de revenir est
récupérable : réparez le magasin et la livraison suivante arrive.

Le cas pratique est un `DatabaseFailedJobStore` pointant vers une
table `failed_jobs` non migrée. Jusqu'à ce que vous migriez, les jobs
mis en lettre morte tournent en boucle à raison d'une re-livraison par
délai de visibilité, chacune journalisant l'erreur du magasin. Si vous
voulez vraiment que les échecs soient abandonnés, configurez
`NullFailedJobStore` - cela réussit, donc le job acquitte et
disparaît.

### Réessayer

```rust
use uuid::Uuid;

// Enregistrement unique - false si l'id n'était pas dans le magasin.
Queue::retry_failed(some_id).await?;

// En masse - seuil optionnel (ne réessaie que les enregistrements plus anciens que `before`).
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` charge l'enveloppe, réinitialise `attempts`,
`available_at`, et `idempotency_key`, la pousse via le driver
configuré, puis supprime l'enregistrement du job en échec. À l'image
de `php artisan queue:retry <id>` combiné à la sémantique de
`queue:flush` (chaque enveloppe réessayée est poussée ET retirée du
magasin).

### Schéma `failed_jobs`

`DatabaseFailedJobStore` attend cette table (gérée par vos migrations)
:

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

L'argument `table` de `DatabaseFailedJobStore::new` est validé comme
un identifiant SQL à la construction.

## Batches en file d'attente

Dispatchez un groupe de jobs avec suivi de progression et callbacks de
complétion :

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// Enregistrez des callbacks nommés à l'amorçage.
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// Inspecter la progression plus tard :
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

Chaque worker clôture son job par rapport au batch, et quand
`pending_jobs` atteint zéro, le worker déclenche les callbacks
`then`/`catch`/`finally` enregistrés. Par défaut, le premier échec
annule le batch ; `.allow_failures()` laisse les jobs restants
continuer.

### Batches durables

`MemoryBatchRepository` est perdu au redémarrage, ce qui abandonne
chaque batch en vol : ses compteurs disparaissent, `pending_jobs` ne
peut plus jamais atteindre zéro, et les callbacks ne se déclenchent
jamais. Utilisez `DatabaseBatchRepository` en production :

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

Deux tables, que le framework ne crée pas - ajoutez-les à vos
migrations, de la même façon que `jobs` et `failed_jobs` fonctionnent
:

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` vous
laisse les nommer vous-même ; les deux noms sont validés comme des
identifiants SQL à la construction.

Notez ce que `pending_jobs` et `failed_jobs` ne sont **pas** : des
colonnes. Ils sont dérivés des lignes de clôture à chaque lecture -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
car les files d'attente sont au moins une fois, donc le même job se
clôture plus d'une fois chaque fois qu'une re-livraison se produit,
qu'un ack est dupliqué, ou qu'un worker meurt entre le moment où il
fait le travail et celui où il l'enregistre. Un compteur décrémenté
par clôture dérive à chacune de ces occasions, et la dérive n'est pas
cosmétique : `pending_jobs` filtre les callbacks, donc un zéro
prématuré déclenche `then` pendant que d'autres jobs du batch sont
encore en cours. Avec les comptes dérivés et la clé primaire sur
`(batch_id, job_id)`, une clôture répétée n'insère rien et il n'y a
aucun compteur qui puisse se tromper - à travers les processus, pas
seulement à l'intérieur d'un seul.

### Quand un dispatch échoue à mi-chemin

Si un `driver.push` échoue en plein milieu de `dispatch()`, les jobs
qui ont déjà atteint la file d'attente sont réels et déjà estampillés
avec l'id du batch. Le batch est donc clôturé plutôt que retiré :
chaque enveloppe qui n'a *pas* été poussée est enregistrée comme un
job en échec, et le batch est annulé.

`total_jobs` compte toujours ce que vous avez demandé,
`failed_job_ids` nomme exactement les jobs qui n'ont jamais atteint la
file, ceux déjà en file d'attente se clôturent normalement, et
`SkipIfBatchCancelled` abandonne le reste - si bien que `pending_jobs`
atteint quand même zéro et que vos callbacks `catch`/`finally`
s'exécutent quand même. Si rien n'a du tout été poussé, `dispatch` les
déclenche elle-même, parce qu'il ne reste aucun worker pour le faire.
Vous récupérez l'erreur de push originale dans les deux cas.

### Options de batch

| Option | Méthode du builder | Effet |
| --- | --- | --- |
| Autoriser les échecs | `.allow_failures()` | continue à planifier après l'échec d'un job |
| Callback then | `.then(name)` | s'exécute quand tous les jobs réussissent |
| Callback catch | `.catch(name)` | s'exécute au premier échec |
| Callback finally | `.finally(name)` | s'exécute après que le batch se clôture, dans tous les cas |
| Ignorer si annulé | middleware `SkipIfBatchCancelled` sur le job | abandonne les jobs restants quand le batch est annulé |

### Impl `BatchCallback`

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … envoie le mail
        Ok(())
    }
}
```

Enregistrez à l'amorçage avec
`batch::register_callback(Arc::new(SendSummary))`. Les callbacks sont
indexés par `name()` - les options du batch stockent des noms de
callback, si bien qu'un redémarrage de processus retrouve les
callbacks enregistrés par recherche plutôt que d'essayer de
désérialiser une closure (les closures Rust ne se sérialisent pas).

## Chaînes en file d'attente

Des flux de travail séquentiels où chaque maillon ne s'exécute
qu'après que le handler du précédent a acquitté :

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

La première enveloppe est poussée immédiatement ; le reste voyage dans
son champ de payload `chain_remaining`. À chaque clôture réussie, le
worker extrait l'entrée suivante et la dispatche. Un échec brise la
chaîne - les maillons suivants ne sont jamais mis en file d'attente.

### Clôture terminale

Terminer un job chaîné signifie deux choses : mettre en file d'attente
le successeur, et libérer le job qui vient de finir. En tant que deux
opérations séparées, il n'y a pas d'ordre sûr. Acquittez en premier,
et un crash dans l'intervalle perd le reste de la chaîne de façon
permanente - rien ne reste dans la file d'attente pour réessayer.
Poussez en premier, et le même crash re-livre le job terminé, si bien
que son handler s'exécute à nouveau et que le successeur est mis en
file d'attente deux fois.

Le worker confie donc les deux au driver en une seule fois, via
`QueueDriver::settle(token, follow_ups)` :

| Résultat | Signification |
| --- | --- |
| `Settled::Atomically` | successeur mis en file d'attente et réservation abandonnée dans une seule transaction |
| `Settled::Stale` | la réservation a été reprise par un autre consommateur ; **rien** n'a été mis en file d'attente ni abandonné |
| `Settled::Unsupported` | ce driver ne peut pas clôturer de façon transactionnelle |

`DatabaseQueueDriver` l'implémente : les deux effets forment une seule
transaction, et le `DELETE` indexé par la réservation fait aussi
office de barrière. Si votre délai de visibilité a expiré pendant que
le handler s'exécutait et qu'un autre worker a récupéré le job, le
delete ne correspond à rien, la transaction fait un rollback, et vous
obtenez `Stale` - n'ayant rien mis en file d'attente. Une clôture en
deux étapes ne peut absolument pas exprimer cela : votre push réussit,
le push du nouveau propriétaire réussit, et la chaîne se scinde en
deux.

Redis et le driver en mémoire répondent `Unsupported` et conservent
l'ordre push-avant-ack, ce qui échange une perte permanente contre un
doublon au moins une fois. C'est le contrat documenté du framework, et
c'est pourquoi les ids d'enveloppe chaînés sont dérivés de leur
prédécesseur plutôt que d'être aléatoires - une étape re-livrée
repousse l'id qu'elle avait poussé avant, si bien que le doublon est
reconnaissable comme la même étape logique.

Si vous écrivez un driver dont l'écriture de suivi et l'acquittement
partagent un domaine transactionnel, implémentez `settle`. Son défaut
retourne `Unsupported`, si bien que les drivers écrits avant que cela
n'existe continuent de fonctionner sans changement.

## Introspection

```rust
Queue::size().await?;            // total
Queue::pending_size().await?;    // available_at <= maintenant, pas réservé
Queue::delayed_size().await?;    // available_at > maintenant
Queue::reserved_size().await?;   // actuellement extrait, pas encore acquitté
Queue::clear().await?;           // abandonne chaque enveloppe, retourne le compte
Queue::driver_name()?;           // nom du driver configuré, pour les journaux / l'admin
```

Le trait `QueueDriver` déclare des défauts pour `size` /
`pending_size` / `reserved_size` / `delayed_size` / `clear` ;
`MemoryQueueDriver` et `DatabaseQueueDriver` les implémentent
nativement. `RedisQueueDriver` retourne une erreur « unsupported »
pour `size` / `clear` - utilisez le redis-cli admin pour ceux-là.

## Signal de redémarrage du worker

`php artisan queue:restart` se traduit par :

```rust
Queue::restart().await?;
```

Le signal vit dans `Cache` comme un timestamp en millisecondes. Les
workers interrogent une fois par boucle et sortent proprement quand le
timestamp est plus récent que leur heure de démarrage. Associez-le à
un superviseur (systemd, Kubernetes, le module `supervisor`) afin
qu'un worker neuf reprenne où le précédent s'est arrêté.

## Arrêt gracieux

Le `CancellationToken` du worker se déclenche à la prochaine limite
d'extraction, jamais en plein dispatch. Un handler déjà extrait
s'exécute jusqu'à sa fin (borné par son propre `Job::timeout()` si
défini) avant que le worker ne sorte. Cela signifie que les effets de
bord en vol ne sont pas interrompus en plein élan, mais un SIGTERM
peut prendre jusqu'au délai d'attente par job pour se vider.
Définissez `WorkerConfig::max_jobs` pour une stratégie de redémarrage
périodique sur les workers de longue durée ; le worker sort proprement
après ce nombre de clôtures, quel que soit le résultat.

## Métriques de clôture

Le worker émet un compteur `queue.settlement.failures` via
[`Metrics`](observability.md) à chaque échec d'ack/nack. Attributs :
`operation` (`"ack"` | `"nack"`), `driver` (le nom du driver
configuré), `job` (le job_name), `outcome` (`"success"`,
`"dead_letter"`, `"retry"`, `"deleted"`, `"timeout_dead_letter"`,
`"timeout_retry"`, `"released"`).

Un taux non nul ici signifie que la livraison au moins une fois peut
re-livrer un effet de bord déjà réussi ou perdre la comptabilité des
tentatives - alertez explicitement là-dessus.

## Erreurs typées

`MaxAttemptsExceeded`, `TimeoutExceeded`, et `ManuallyFailed` sont à
l'image de `MaxAttemptsExceededException` / `TimeoutExceededException`
/ `ManuallyFailedException` de Laravel. Le worker attache la cause
pertinente à l'événement de lettre morte `JobFailed`, afin que les
écouteurs puissent faire du pattern-matching plutôt que de chercher
une sous-chaîne dans le message d'erreur.

## Nommage des connexions

Les workers étiquettent chaque événement de cycle de vie avec un nom
de connexion. Par défaut, c'est le `name()` du driver (par ex.
`"memory"`, `"redis"`, `"database"`). Les applications qui font
tourner plusieurs connexions à la fois peuvent surcharger cela :

```rust
Queue::set_connection_name("orders-redis");
```

## Tests

La sémantique de `Queue::fake()` vit dans `queue::testing` :

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// Pour les dispatches différés, épinglez le timestamp planifié :
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

La garde du fake sérialise les tests parallèles via un mutex à
l'échelle du processus ; elle capture `(payload, available_at)` par
push et s'efface au `Drop`. En mode fake, `push_unique` enregistre
toujours le push comme nouveau - la déduplication n'a pas de sens
quand aucun driver n'est câblé.

## L'idempotence est le contrat du worker envers vous

Les drivers de file d'attente adossés à Redis ne peuvent pas rendre
`nack` atomique - `XADD` et `XACK` sont des commandes séparées. Un
crash entre les deux re-livre le message via `XAUTOCLAIM`. Les drivers
en mémoire et base de données sont exactement-une-fois-par-tentative,
mais la boucle du worker ne distingue pas les drivers, donc **chaque
handler de job dans un déploiement de production doit être
idempotent**.

Pour les jobs typiques de style commande, enveloppez le corps du
handler dans [`Idempotency::once`](idempotency.md) ou
[`Idempotency::commit_on_success`](idempotency.md), indexé par une clé
stable par opération (id d'entité, id de requête fourni par
l'appelant, etc.). Quand un réessai doit retourner le résultat
*original* plutôt que sauter la ré-exécution, utilisez
`Idempotency::remember`, qui enregistre la valeur de succès et la
rejoue lors des livraisons ultérieures.

## Suivant

- [Bus](bus.md) - dispatcher synchrone avec résultats typés
- [Événements](events.md) - fan-out pub/sub
- [Idempotence](idempotency.md) - le contrat que les handlers honorent pour la livraison au moins une fois
- [Cache](cache.md) - alimente `push_unique`, `WithoutOverlapping`, `RateLimited`
- [Mocking et doublures](mocking.md) - chaque garde de fake, y compris `Queue::fake`
