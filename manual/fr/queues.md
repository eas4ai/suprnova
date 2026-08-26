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
    queues: Vec::new(),
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

Cinq drivers sont livrés dans l'arbre. Configurez-les via la variable
d'environnement `QUEUE_DRIVER` ou en appelant `Queue::set_driver(...)` par
programme.

| Driver | À utiliser pour | Points forts |
| --- | --- | --- |
| `MemoryQueueDriver` | tests, applications mono-processus | `tokio::time::DelayQueue` pour `available_at`, compatible avec l'horloge virtuelle |
| `RedisQueueDriver` | fan-out en production | groupes de consommateurs + `XAUTOCLAIM` + jobs différés adossés à un ZSET |
| `DatabaseQueueDriver` | applications à base de données unique | `FOR UPDATE SKIP LOCKED` sur Postgres/MySQL, sérialisé par `BEGIN` sur SQLite |
| `SyncQueueDriver` | dev, CI | exécute le handler en ligne au `push`, sans worker |
| `NullQueueDriver` | wrappers de test | jette chaque push sans l'exécuter |

`Queue::bootstrap_from_env()` lit `QUEUE_DRIVER` et câble le driver
correspondant ; `Queue::bootstrap_default()` câble toujours le driver
mémoire. Le chemin d'amorçage du serveur appelle l'un des deux pour vous -
la plupart des applications se contentent de configurer par
l'environnement.

`FailoverQueueDriver` n'est pas un sixième backend. Il enveloppe une liste
ordonnée des drivers ci-dessus, si bien qu'un push refusé par une
connexion passe à la suivante. Voir [Connexions en
failover](#connexions-en-failover).

### Configuration par l'environnement

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Driver base de données - DB::init() doit s'exécuter d'abord
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

Le driver base de données valide `QUEUE_DB_TABLE` comme identifiant SQL à
la construction, si bien qu'une valeur d'environnement malformée fait
échouer l'amorçage plutôt que d'atteindre la composition du SQL. Redis
utilise sea-streamer-redis en dessous, avec `AutoCommit::Disabled` ; le
délai de visibilité est fixé à la construction du groupe de consommateurs,
si bien que l'argument `visibility_timeout` par pop est ignoré sur Redis
(une divergence documentée par rapport au contrat du trait, imposée par
les Redis Streams).

### Pourquoi Suprnova diverge

Laravel fait passer par le Bus tout ce qui peut être mis en file, en
distinguant les jobs `ShouldQueue` au moment du dispatch. Suprnova sépare
les deux : `Bus` pour le travail synchrone qui retourne un résultat typé,
`Queue` pour le travail asynchrone qui survit à un plantage du processus.
PHP a besoin de ce routage implicite parce que son modèle « une requête,
un processus » rend difficile de modéliser autrement « fais ceci plus
tard, dans un autre processus ». Tokio, lui, n'en a pas besoin : un
`Bus::dispatch` explicite face à un `Queue::push` est plus clair, plus
rapide, et fait apparaître le choix de durabilité sur le site d'appel.
Voir [`bus.md`](bus.md) pour la comparaison côte à côte.

## Connexions en failover

`FailoverQueueDriver` enveloppe une liste ordonnée de connexions. Un push
que la première connexion refuse est réessayé sur la suivante, et ainsi de
suite le long de la liste, si bien qu'une panne de Redis ne transforme pas
chaque dispatch en job perdu.

Configurez-le depuis l'environnement :

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# Chaque connexion lit ses propres variables, exactement comme elle le
# ferait si elle était QUEUE_DRIVER à elle seule.
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

Ou câblez-le vous-même, quand les connexions ont besoin d'une
configuration à l'exécution que l'environnement ne peut pas exprimer :

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

Le `String` de chaque entrée est le label de connexion rapporté sur
l'événement `QueueFailedOver`. Il n'est pas dérivé du type de driver, car
deux connexions peuvent faire tourner le même driver.

`QUEUE_FAILOVER_CONNECTIONS` est requis quand `QUEUE_DRIVER=failover`, et
la liste ne peut pas contenir `failover` elle-même. Une entrée nommant un
driver qui n'existe pas est une erreur d'amorçage, plutôt que le repli
« avertir et utiliser la mémoire » que `QUEUE_DRIVER` s'applique à
lui-même : à l'intérieur d'une chaîne de failover, une faute de frappe
devenue silencieusement une connexion en mémoire placerait un backend
éphémère dans une liste durable.

### Les écritures basculent, pas les lectures

Seuls `push` et `bulk_push` parcourent la liste de connexions. Toutes les
autres opérations - `pop`, `ack`, `nack`, `release`, `settle`, `clear`,
les quatre compteurs et les trois listings d'inspection - vont à la
**première** connexion et à aucune autre.

Cette asymétrie est le contrat, pas un oubli. Un token de réservation n'a
de sens que pour le driver qui l'a émis : acquitter contre une autre
connexion ne clôturerait rien et corromprait les deux. Les compteurs et
les listings suivent la même règle, pour que ce que vous inspectez soit ce
que vide le worker de cette connexion, plutôt qu'une somme entre backends
qui ne correspond à la vue d'aucun worker.

**Un worker sur la connexion de failover ne vide que la primaire.** Les
jobs qui ont basculé vers un repli ont besoin d'un worker tournant
directement contre cette connexion de repli :

```bash
# Vide la primaire de la chaîne de failover.
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# Vide ce qui a basculé vers la base de données. Lancez celui-ci aussi.
QUEUE_DRIVER=database ./app queue:work
```

La documentation de Laravel porte le même avertissement pour la même
raison.

Cela atteint les chaînes, mais par une seule porte. Un worker clôture un
job et met en file le maillon suivant d'une [chaîne en file
d'attente](#chaînes-en-file-d-attente) en un seul appel, `settle`, et le
décorateur délègue cet appel à la seule primaire. Ainsi, avec une primaire
transactionnelle comme le driver base de données, une primaire en panne
fait échouer la clôture et rien ne bascule : le worker laisse la
réservation intacte et l'expiration de visibilité redélivre le job. Le
basculement se produit quand la primaire répond `Settled::Unsupported`, ce
que font les drivers mémoire et Redis, car le worker pousse alors le
maillon suivant à travers le driver lié comme n'importe quel autre push -
et ce push, lui, bascule. Le reste de la chaîne attend alors un worker sur
la connexion de repli. Sans lui, la chaîne cale : le maillon est durable
et rien n'est perdu, mais rien ne l'exécute non plus.

### L'événement `QueueFailedOver`

Chaque connexion qui refuse un push dispatche
`queue::events::QueueFailedOver { connection, job_name, exception }`, mais
uniquement sur le push qui fait *basculer* cette connexion en échec. Une
connexion déjà connue comme défaillante reste silencieuse jusqu'à ce qu'un
push ultérieur y réussisse, ce qui la réarme. Une panne de quatre heures
produit un événement, pas un par dispatch, et c'est ce qui la rend
utilisable comme alerte.

`connection` est le label de la connexion qui a échoué, pas celui de celle
qui a accepté le job.

Quand toutes les connexions refusent un push, le push retourne l'erreur de
la dernière connexion. `bulk_push` pousse chaque enveloppe séparément, si
bien que chacune bascule pour son propre compte : un lot à moitié accepté
par la primaire n'est jamais repoussé en bloc sur le repli, et chaque
enveloppe conserve l'`available_at` avec lequel elle a été construite. Un
lot n'est pas atomique. Si une enveloppe est refusée par toutes les
connexions, `bulk_push` retourne l'erreur de cette enveloppe alors que les
enveloppes précédentes sont déjà mises en file.

Basculer n'est pas dédupliquer. Le décorateur ne retente jamais une
enveloppe qu'une connexion a acceptée, mais une connexion qui écrit
l'enveloppe *puis* signale un échec produit un doublon sur la connexion
suivante, car « je l'ai écrite et j'ai perdu l'acquittement » est
indiscernable de « je ne l'ai jamais prise ». Les deux copies portent le
même id de job. C'est le contrat de livraison au moins une fois du
framework, celui-là même qui fait de l'idempotence du handler une exigence
partout ailleurs - voir [L'idempotence est le contrat du worker envers
vous](#l-idempotence-est-le-contrat-du-worker-envers-vous).

### Pourquoi Suprnova diverge

La connexion de failover de Laravel est un tableau `connections` dans
`config/queue.php`, résolu à travers le registre de connexions. Suprnova
n'a pas de registre de drivers par connexion - un seul driver est lié pour
tout le processus - si bien que les labels viennent de
`QUEUE_FAILOVER_CONNECTIONS` (ou du `String` que vous passez à
`FailoverQueueDriver::new`) et que les lectures délèguent au premier
*driver* plutôt qu'à une connexion nommée.

Le `FailoverQueue::bulk` de Laravel boucle sur les jobs un par un pour que
le délai de chacun survive. Suprnova résout le délai sur l'enveloppe avant
qu'aucun driver ne la voie, si bien que la boucle par enveloppe le
préserve gratuitement - mais c'est encore cette boucle qui empêche un lot
à moitié posé d'être poussé deux fois, donc elle reste.

## Variantes de push

Chaque variante de push prend une valeur typée `J: Job` et retourne quand
l'enveloppe a été remise au driver - pas quand le handler s'exécute.

| Méthode | Comportement |
| --- | --- |
| `Queue::push(job)` | met en file immédiatement |
| `Queue::push_later(job, at)` | disponible à un `DateTime<Utc>` précis |
| `Queue::later(delay, job)` | disponible après `delay` à partir de maintenant |
| `Queue::push_with(job, overrides)` | met en file immédiatement avec des `EnvelopeOverrides` par push |
| `Queue::push_after_commit(job)` | met en file quand la `DB::transaction` englobante commite |
| `Queue::later_with(delay, job, overrides)` | disponible après `delay` à partir de maintenant, avec des `EnvelopeOverrides` par push |
| `Queue::push_unique(job)` | déduplique par `J::unique_id` dans `J::unique_for` ; retourne `Ok(true)` quand l'enveloppe a été poussée, `Ok(false)` quand une clé de déduplication vivante l'a supprimée |
| `Queue::push_unique_later(job, at)` | unique + planifié |
| `Queue::later_unique(delay, job)` | unique + différé |
| `Queue::bulk(vec![job1, job2, ...])` | pousse chaque job (le driver peut utiliser un chemin bulk natif) |

`push_unique` exige que la couche cache soit amorcée : le verrou de
déduplication vit dans [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). Un push échoué libère
la clé de déduplication pour que l'appelant puisse réessayer ; un push
réussi la garde pendant `J::unique_for` secondes. Le job doit surcharger
`Job::unique_id(&self)` pour retourner `Some(id)` - `None` retourne une
erreur interne.

Le booléen répond à une seule question - « ce job est-il sur la file ? » -
et il y a un troisième cas derrière. Si le bail du verrou de déduplication
est perdu pendant que le push est en vol, le push se termine quand même
(la couche d'idempotence n'annule jamais un corps qui a pu déjà avoir un
effet) et vous obtenez toujours `Ok(true)`, avec un log de niveau `warn`
nommant le job et sa clé unique. Le job est en file ; ce qui n'est pas
prouvé, c'est que personne d'autre n'a mis le même en file au même
moment. Votre handler doit déjà tolérer la redélivrance, donc cela ne
demande aucun traitement supplémentaire - mais le log est là parce qu'une
rafale de ces messages signifie que le cache derrière votre verrou de
déduplication peine.

### Unique jusqu'au traitement

Un verrou d'unicité dure normalement toute la fenêtre `unique_for`, même
après l'exécution du job. Quand le verrou existe pour fusionner les
doublons *en file* plutôt que pour sérialiser l'exécution, choisissez de
le libérer dès le début du traitement :

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Une reconstruction qui tourne 20 minutes n'avale plus le
        // re-dispatch qui arrive à la minute 2.
        Ok(())
    }
}
```

Le worker libère le verrou après la passe de middleware du job et
immédiatement avant l'exécution du handler. Quatre conséquences en
découlent :

- Un job qu'un middleware relâche sur la file garde son verrou. Il n'a pas
  commencé à être traité, donc rien n'a changé pour un doublon.
- Un job qu'un middleware court-circuite de toute autre façon abandonne
  son verrou, parce qu'il ne sera jamais traité du tout. Cela couvre la
  suppression du job, sa mise en lettre morte, et le fait de le déclarer
  terminé sans jamais appeler le handler.
- Un job qui échoue libère son verrou et est quand même réessayé. Le
  verrou est parti au moment où le traitement a commencé, donc un doublon
  peut se mettre en file pendant que la tentative échouée attend la fin de
  son backoff, et vous vous retrouvez avec deux enveloppes pour le même id
  unique. C'est le compromis que fait cette option. Si un réessai doit
  continuer à tenir la place, laissez `unique_until_processing` désactivé
  et laissez le TTL `unique_for` couvrir toute la chaîne de tentatives.
- La libération est liée au propriétaire. `push_unique` enregistre le
  token de propriétaire du verrou sur l'enveloppe, et le worker libère
  avec ce token, si bien qu'une tentative redélivrée ne peut jamais
  libérer un verrou qu'un dispatch plus récent a acquis depuis.

`unique_until_processing` a besoin des deux mêmes choses que
`push_unique` : un `unique_id` qui retourne `Some(id)`, et une couche
cache amorcée.

Sous le driver `sync`, le handler s'exécute en ligne à l'intérieur de
l'appel `push_unique` qui a pris le verrou, si bien que le job libère un
verrou que son propre appelant est encore nominalement en train de tenir.
Si ce handler tourne plus longtemps qu'un tiers de `unique_for`, le
renouveleur de bail de déduplication remarque que le verrou a disparu et
journalise un avertissement de bail perdu, et `push_unique` journalise
par-dessus son propre avertissement « l'exclusivité n'a pas pu être
prouvée ». Les deux sont attendus ici plutôt que fautifs : le job s'est
exécuté, le push retourne `Ok(true)`, et le verrou a disparu parce que le
job l'a lui-même libéré.

### Pourquoi Suprnova diverge

Laravel libère le verrou d'un job unique *ordinaire* dès que le handler
retourne. Suprnova laisse plutôt ce verrou expirer avec le TTL
`unique_for`, ce qui garde la fenêtre de déduplication honnête quand un
worker meurt en plein job : la fenêtre que vous avez configurée est celle
que vous obtenez, que le handler soit retourné ou non.
`unique_until_processing` se comporte de la même façon dans les deux
frameworks.

Suprnova ne force par ailleurs jamais la libération d'un verrou d'unicité.
Laravel se rabat sur une libération forcée pour une première tentative qui
ne porte aucun token de propriétaire. Les seules enveloppes qui atteignent
un worker Suprnova sans token sont celles mises en file avant l'existence
du token, et celles-là gardent l'expiration par TTL plutôt que de risquer
une libération qui supprimerait le verrou d'un dispatch plus récent.

### Remplacements par-push avec `EnvelopeOverrides`

`Queue::push_with` et `Queue::later_with` prennent un `EnvelopeOverrides`
à côté du job, pour le dispatch qui a besoin d'un comportement de file, de
connexion, de délai d'attente ou de réessai différent des valeurs par
défaut du job :

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// Le pendant différé, qui reflète la relation de `Queue::later` à `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Chaque champ vaut `None` par défaut et s'en remet à la résolution normale
que `Queue::push` exécute déjà ; un champ `Some` l'emporte sur tout cela
pour ce seul push, devançant à la fois une route enregistrée avec
[`Queue::route`](#routage-de-file-d-attente) et la déclaration `Job::*`
propre au job pour ce champ :

| Champ | Devance |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` est la primitive sur laquelle sont bâtis à la fois
`Mail::on_queue`/`.on_connection()` et le réglage de file par notification
de `Notify::queue` - voir [E-mail](mail.md#queueing) et
[Notifications](notifications.md).

### Délai déclaré par le job

Un job peut porter son propre délai par défaut, au lieu que chaque site
d'appel répète `Queue::later(Duration::from_secs(60), job)` :

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`,
`Queue::push_unique(job)` et `Queue::bulk(vec![job1, job2])` le respectent
tous - `available_at` devient `now + J::delay()` au lieu de `now`.
`Queue::bulk` résout le délai une fois par appel, puisque chaque job du
vecteur partage le même `J` concret et donc le même `Job::delay()`.

Un délai explicite au site d'appel l'emporte toujours :
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` et `Queue::later_unique(delay, job)`
utilisent tous, tel quel, l'horodatage ou le délai passé par l'appelant -
`Job::delay()` n'est consulté pour aucun d'eux. Recourez à la méthode du
trait quand chaque dispatch d'un type de job doit démarrer différé par
défaut ; recourez à l'une des variantes `later`/`push_later` pour un délai
dont un dispatch précis a besoin mais que le type ne déclare pas par
ailleurs.

Les batches et les chaînes ne le consultent pas non plus :
`Queue::batch()...add(job)` et `Queue::chain()...add(job)?` construisent
tous deux leurs enveloppes avec `available_at` fixé au moment où vous avez
appelé `add`, si bien qu'un job doté d'un `Job::delay()` déclaré est
dispatché immédiatement au sein d'un batch ou d'une chaîne, alors même
qu'un simple `Queue::push(job)` du même job attendrait. Donnez au job un
délai explicite autrement - un champ sur le job lui-même, appliqué dans
`handle()` - si une étape de batch ou de chaîne en a besoin.

### Pourquoi Suprnova diverge

Le `$job->delay` de Laravel est une propriété d'instance, définie par
dispatch (`SendDigest::dispatch($user)->delay(60)`), si bien que deux
dispatches de la même classe peuvent porter des délais différents. Ici,
`Job::delay()` est plutôt un défaut au niveau de la classe, comme
`Job::queue()` ou `Job::max_tries()` - un dispatch qui a besoin d'un délai
calculé depuis ses propres données utilise `Queue::later`/`push_later`,
qui devance déjà le défaut déclaré.

### Dispatch après commit

Un job poussé à l'intérieur d'une
[`DB::transaction`](database.md#transactions) court contre cette
transaction. Un worker sur un autre processus peut dépiler l'enveloppe,
chercher la ligne que la transaction tient encore ouverte, et échouer - ou
pire, la transaction fait un rollback et le job s'exécute contre des
données qui n'existent plus.

Faites adhérer le job à l'attente du commit :

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // La ligne de commande est garantie durable au moment où ceci s'exécute.
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // Rien n'atteint le driver ici.
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// L'enveloppe est sur la file maintenant, et seulement maintenant.
```

Trois règles couvrent tous les cas :

- **À l'intérieur d'une transaction, tout le push attend le commit.** Pas
  seulement l'écriture du driver : la construction de l'enveloppe,
  l'événement `JobQueueing` et l'événement `JobQueued` ont eux aussi lieu
  au moment du commit, si bien qu'un écouteur n'est jamais informé d'un
  job qu'un rollback jette ensuite.
- **Un rollback le jette.** Le push n'a simplement jamais lieu. S'il a
  pris un verrou d'unicité, le rollback rend ce verrou.
- **Hors transaction, le push a lieu immédiatement.** C'est ce qui rend
  cette adhésion sûre à déclarer sur le type de job : un site de dispatch
  n'a pas à savoir si le chemin de code sur lequel il se trouve est
  transactionnel.

Un rollback vers un [savepoint](database.md#savepoints) compte comme un
rollback pour tout ce qui a été enregistré à l'intérieur.
`tx.rollback_to("name")` jette les pushes différés depuis
`tx.savepoint("name")` et libère les verrous qu'ils ont pris, sur-le-champ,
si bien qu'un nouveau dispatch dans la même transaction regagne la clé.
Les pushes faits avant le savepoint sont intacts, et un savepoint que vous
n'annulez jamais conserve tout ce qui a été enregistré à l'intérieur.

Par dispatch plutôt que par type de job, utilisez
`EnvelopeOverrides::after_commit`. `Some(true)` est l'`afterCommit()` de
Laravel et a pour raccourci `Queue::push_after_commit(job)` ; `Some(false)`
est le `beforeCommit()` de Laravel, pour le dispatch qui doit être visible
d'un worker avant que le commit ne se pose :

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// Diffère un job dont le type n'y adhère pas.
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// Pousse immédiatement, bien que le type de job y adhère.
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

Un `Queue::push` différé résout à nouveau
[`Job::delay()`](#délai-déclaré-par-le-job) par rapport au commit et non
par rapport au push, parce que le délai signifie « attends ce temps après
le dispatch » et que, pour un job différé, le dispatch *est* le commit. Un
horodatage explicite est l'intention de l'appelant à propos d'un instant
précis : `Queue::push_later`, `Queue::later` et `Queue::later_with`
portent donc le leur à travers le report, inchangé.

`Queue::push_unique` diffère avec une asymétrie délibérée : le verrou de
déduplication est pris immédiatement, si bien qu'un second `push_unique`
pour le même id unique dans la même transaction est toujours supprimé et
rapporte toujours `Ok(false)`. Seule l'enveloppe attend. Le gagnant
rapporte `Ok(true)` même si son push est en attente, parce que ce push va
avoir lieu. Un rollback libère le verrou qu'il a pris, en le liant au
propriétaire, si bien que la fenêtre `unique_for` n'est jamais bloquée par
un dispatch qui n'a jamais eu lieu - et il en va de même pour toute autre
issue où le commit ne se pose pas, y compris un `COMMIT` refusé. La seule
borne de cette garantie est le TTL lui-même : une transaction qui reste
ouverte plus longtemps que `unique_for` peut voir son verrou expirer et
être repris par un autre dispatch en cours de route ; donnez donc à
`unique_for` de la marge au-dessus de votre plus longue transaction si la
déduplication compte. La famille `push_unique*` ne prend pas
d'`EnvelopeOverrides`, si bien que `Job::after_commit()` est la seule
chose qui décide si un push unique diffère - il n'y a pas de surcharge par
push pour cela.

Les batches et les chaînes ne diffèrent pas, de la même façon qu'ils ne
consultent pas `Job::delay()` : `Queue::batch()` et `Queue::chain()`
construisent et poussent leurs enveloppes directement. Enveloppez l'appel
à `.dispatch()` pour qu'il s'exécute après le retour de la transaction si
un batch doit attendre un commit.

Les [e-mails](mail.md#queueing) et les [notifications](notifications.md)
mis en file ne diffèrent pas non plus. Chacun voyage sur un unique type de
job partagé (`SendMailJob` / `SendNotificationJob`), et il n'existe pas
encore d'équivalent de `ShouldQueueAfterCommit` sur `Mailable` ou
`Notification`, si bien qu'un appel `Mail::queue` ou `Notify::queue` à
l'intérieur d'une transaction atteint le driver immédiatement. Envoyez-les
après le retour de la transaction.

Sous `Queue::fake()`, un push est enregistré immédiatement, report compris,
si bien qu'un test peut faire des assertions dessus sans rien commiter.
Cela correspond au `Bus::fake` de Laravel, et c'est ce qui permet à un
test de piloter un handler transactionnel et d'affirmer ses dispatches
dans le même souffle.

### Pourquoi Suprnova diverge

`Queue::bulk` est monomorphe - chaque élément partage un unique `J`
concret - si bien que son partitionnement après commit est tout ou rien
pour l'appel. Laravel partitionne un tableau hétérogène en deux moitiés,
différée et immédiate ; ici, il n'y a rien à partitionner.

Le report est lié à la forme par fermeture. Un push à l'intérieur d'un
[`DB::begin_transaction`](database.md#manual-form) manuel a lieu
**immédiatement**, parce que le mode manuel n'installe aucune transaction
ambiante et n'a donc aucun commit auquel accrocher un callback. Y différer
mettrait en file un callback que rien n'exécuterait jamais, et un dispatch
qui disparaît silencieusement est pire qu'un dispatch qui a lieu trop tôt.
Recourez à `DB::transaction` quand un dispatch doit attendre le commit.

Laravel lit par ailleurs une clé de configuration `after_commit` au niveau
de la connexion comme dernier repli de sa chaîne de priorité. Suprnova
s'arrête à la surcharge par push, puis au `Job::after_commit()` propre au
job : ici, les connexions de file ne portent pas leur propre politique de
dispatch.

## Configuration des jobs

Surchargez les fonctions associées de `Job` pour ajuster le comportement
par impl :

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // défaut : aucun délai
    fn max_tries() -> u32 { 5 }                            // défaut : 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // défaut : false (le délai d'attente réessaie)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // défaut : 5 minutes
    fn unique_until_processing() -> bool { true }          // défaut : false (le TTL est la fenêtre)
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

1. une surcharge par push passée à `Queue::push_with` /
   `Queue::later_with` (voir [Remplacements par-push avec
   `EnvelopeOverrides`](#remplacements-par-push-avec-envelopeoverrides))
2. une route enregistrée avec `Queue::route`
3. le `Job::queue` / `Job::connection` propre au job
4. le défaut du driver / global

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
| `UniqueJobSkipped` | `push_unique` a supprimé un doublon pendant la fenêtre `unique_for` |
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
| `QueuePaused` | `Queue::pause` a défini le commutateur propre à une file |
| `QueueResumed` | `Queue::resume` a effacé le commutateur propre à une file |
| `QueuesPaused` | `Queue::pause_all` a défini le commutateur global |
| `QueuesResumed` | `Queue::resume_all` a effacé le commutateur global |

Abonnez-vous avec l'API normale `Event::listen`. Les événements sont
best-effort - `Event::dispatch` sans écouteur est un `Ok(())` sans
effet, si bien que les workers dans les déploiements sans
`Event::init()` ne paient rien.

`UniqueJobSkipped` est le seul événement déclenché du côté du *push*
plutôt que du côté du worker, et le seul qui signale une non-défaillance.
Il porte `job_name`, `unique_id` et `connection` - la décision de
déduplication a lieu avant qu'une enveloppe existe, si bien qu'il n'y a
aucun id d'enveloppe à signaler. Le push retourne tout de même
`Ok(false)` ; l'événement rend observable une suppression autrement
invisible.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` se
déclenchent de la même façon - depuis `Queue::pause` / `resume` /
`pause_all` / `resume_all` eux-mêmes, et non depuis la boucle du worker.
Ils ne portent pas non plus d'identité d'enveloppe ; voir « Mettre les
files en pause » ci-dessous pour le contrat complet.

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
Queue::pending_size().await?;    // available_at <= maintenant, non réservé
Queue::delayed_size().await?;    // available_at > maintenant
Queue::reserved_size().await?;   // dépilé, pas encore acquitté
Queue::clear().await?;           // jette chaque enveloppe, retourne le compte
Queue::driver_name()?;           // nom du driver configuré, pour les logs / l'admin
```

Le trait `QueueDriver` déclare des implémentations par défaut pour `size`
/ `pending_size` / `reserved_size` / `delayed_size` / `clear` ;
`MemoryQueueDriver`, `DatabaseQueueDriver` et `RedisQueueDriver` les
implémentent tous nativement.

### Inspecter les files

Les compteurs vous disent combien il y a de travail en file ; parfois vous
avez besoin de voir les enveloppes elles-mêmes - un tableau de bord
d'administration, une session de débogage, une question du type
« qu'est-ce qui est bloqué, exactement ». `Queue::pending_jobs` /
`delayed_jobs` / `reserved_jobs` retournent la même information que
comptent les compteurs de taille, sous la forme d'un listing de DTO
`InspectedJob` :

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` porte `id`, `queue`, `name`, `attempts`, `payload` et
`created_at`. `id` et `created_at` sont des `Option` : les listings du
driver base de données rapportent quand même une ligne dont
l'`envelope_json` n'a pas pu être décodé - sous la forme `id: None` et
`payload: {"unparseable": true}` - plutôt que de la laisser tomber et de
cacher un job toxique à qui vient regarder ; la projection de
`Queue::fake()` n'enregistre jamais d'horodatage de dispatch distinct
d'`available_at`, si bien que `created_at` y vaut toujours `None`.

Sur le driver mémoire, `delayed_size()` lit directement la longueur du
magasin des différés, tandis que `delayed_jobs()` et `pending_jobs()`
promeuvent d'abord toute entrée dont l'`available_at` est déjà passé. Dans
l'étroite fenêtre entre le moment où un job devient échu et le tic suivant
du ramasseur de fond, toutes les 50 ms, `delayed_size()` peut encore
compter un job que `delayed_jobs()` a déjà promu dans `pending_jobs()` -
les listings sont la vue la plus à jour ; un écart là-dessus est attendu,
ce n'est pas un bug.

Une réservation dont le délai de visibilité a expiré continue d'apparaître
dans `reserved_jobs()` jusqu'à ce qu'un `pop` ou le ramasseur de fond la
reprenne. Seuls ces deux-là reprennent, et c'est la reprise qui dépense
une tentative, si bien qu'un appel de listing ne change jamais le compte
de tentatives d'un job, quel que soit le nombre de fois où vous l'appelez.

#### Pourquoi Suprnova diverge

- **Une seule méthode avec `Option<&str>`, pas une paire par listing.**
  Laravel livre `pendingJobs($queue)` à côté d'un `allPendingJobs()`
  distinct ; ici, `queue: None` réduit les deux à un seul appel. Même
  forme pour `delayedJobs`/`allDelayedJobs` et
  `reservedJobs`/`allReservedJobs`.
- **Le défaut du trait est un `Err` honnête, pas une collection vide.**
  Les drivers Beanstalkd et SQS de Laravel retournent `[]` depuis ces
  méthodes même pour une file qui a manifestement des jobs : un mensonge
  par omission qu'un auteur de driver tiers pourrait copier sans s'en
  rendre compte. Un driver Suprnova qui n'a pas implémenté l'inspection le
  dit ; `sync` et `null` le redéfinissent avec `Ok(vec![])` parce que,
  pour eux, « il n'y a jamais rien à lister » est la stricte vérité, et
  non une méthode non implémentée.
- **Le `reserved_jobs` de Redis est par consommateur.** Le driver ne
  connaît que les réservations qu'il a personnellement distribuées dans le
  processus ; les entrées en vol d'un autre consommateur ne sont visibles
  qu'à travers le `XPENDING` de Redis, pas à travers cet appel.
- **Le `pending_jobs` de Redis signifie « jamais délivré à aucun
  consommateur de ce groupe ».** Il balaie `XRANGE (<last-delivered-id> +`,
  c'est-à-dire tout ce qui suit le curseur de livraison du groupe
  (`XINFO GROUPS`), plutôt que le stream entier, parce qu'`ack` ne fait
  qu'un `XACK` sur une entrée (ce driver ne fait jamais de `XDEL`/`XTRIM`
  sur le stream) : un balayage qui se contenterait d'exclure les
  réservations en mémoire d'un consommateur rapporterait donc chaque job
  acquitté comme en attente pour toujours. Un job relâché ou nacké est
  republié sous un id neuf au-dessus du curseur, si bien qu'il réapparaît
  dès que son réessai est vivant. Même registre de « borne supérieure »
  que `pending_size` : le curseur est lu une fois, donc un `pop` concurrent
  peut réclamer une entrée entre cette lecture et le balayage. En pratique,
  la tâche de lecture anticipée en arrière-plan d'un consommateur en cours
  d'exécution a tendance à réclamer une entrée fraîchement poussée dans
  les millisecondes qui suivent le push, bien avant qu'une application
  n'appelle `pop` - si bien que `pending_jobs` reflète surtout le travail
  poussé pendant qu'aucun consommateur de ce stream n'interroge
  activement, et non « toute enveloppe que personne n'a explicitement
  dépilée ».

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

## Mettre les files en pause

`php artisan queue:pause` / `queue:resume` se traduisent par :

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

ou depuis la CLI :

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # alias: queue:continue
```

Un worker en pause termine tout ce qu'il a déjà extrait - la mise en
pause n'interrompt jamais un job en cours - puis cesse de réclamer du
travail jusqu'à la reprise. `pause_all` / `resume_all` sont le
commutateur global ; mettre en pause (ou reprendre) une file nommée
n'affecte que cette file. **`resume_all` n'efface pas une pause propre
à une file** - une file mise en pause individuellement le reste après
une reprise globale, comme dans Laravel. Effacez-la explicitement avec
`Queue::resume(&connection, "billing")`.

Les deux signaux vivent dans `Cache`, à côté du signal de redémarrage
ci-dessus :

| Clé | Signification |
| --- | --- |
| `suprnova:queues:paused` | commutateur global, défini par `pause_all` |
| `suprnova:queue:paused:{connection}:{queue}` | commutateur d'une file, défini par `pause` |

Vérifiez l'état avec
`Queue::is_paused(&connection, "billing").await?` (vrai si l'une ou
l'autre clé est définie) ou
`Queue::paused_queues(&connection, &queues).await?` (quelles files de
`queues` sont actuellement en pause).

### La mise en pause par file exige un `--queue` nommé

Un worker démarré avec `--queue=billing,exports` ne réclame que ces deux
files, si bien que mettre `billing` en pause réduit cette liste à
`exports` tant que la pause dure. Un worker démarré sans `--queue` du
tout vide chaque file que détient le driver, et il n'existe aucun moyen
de demander « mettre seulement `billing` en pause » dans ce cas -
`QueueDriver::pop_from` ne signale jamais les noms de files existants,
si bien qu'il n'y a rien à confronter à une clé de pause par file.
`pause_all` arrête quand même complètement un worker non filtré ; une
pause nommée par file ne prend effet qu'une fois les files de ce worker
nommées également.

### Désactiver l'interrogation de pause

Définissez `QUEUE_PAUSABLE=false` et chaque worker de ce processus
ignore entièrement les signaux de pause, sans coût supplémentaire de
lecture du cache par boucle. `queue:pause` (mais pas `queue:resume`)
refuse également de s'exécuter et sort avec un code non nul, si bien
qu'un opérateur qui a désactivé les pauses le découvre immédiatement au
lieu d'envoyer une pause qui ne ferait tranquillement rien. Cela reflète
`Worker::$pausable` de Laravel.

### Pourquoi Suprnova diverge

Un cache inaccessible applique une politique **fail-open** : un worker
qui ne peut pas lire les clés de pause se comporte comme s'il n'était pas
en pause et continue de vider la file - le même contrat **fail-open** que
celui du signal de redémarrage ci-dessus. Une panne temporaire du cache
doit dégrader un parc de workers en « ignore la pause », jamais en
« chaque worker se fige silencieusement » - l'état de pause est un
signal explicitement choisi, et son indisponibilité ne doit pas devenir
un coupe-circuit caché.

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
l'échelle du processus ; elle capture `(payload, available_at, overrides)`
par push et s'efface au `Drop`. Le champ `overrides` vaut
`EnvelopeOverrides::default()` pour chaque point d'entrée sauf
`push_with`/`later_with` - voir [Mocking](mocking.md#queue---queuetestinginstall_fake)
pour `assert_pushed_on_queue`/`assert_pushed_on_connection` et
`pushed_with_overrides`, les assertions qui le couvrent. En mode fake,
`push_unique` enregistre toujours le push comme nouveau - la
déduplication n'a pas de sens quand aucun driver n'est câblé.

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
