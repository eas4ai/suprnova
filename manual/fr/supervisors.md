# Superviseurs

Un superviseur est une tâche Tokio de longue durée que le framework
démarre à l'amorçage et redémarre automatiquement quand elle se termine.
Les superviseurs servent au travail « toujours actif » : battements de
cœur en arrière-plan, collecteurs de métriques, réchauffeurs de
connexions, balayeurs périodiques, ou toute boucle async qui ne devrait
jamais s'arrêter. Ils se distinguent des [workers de file
d'attente](queues.md), qui consomment des éléments `Job` discrets depuis
une file d'attente. Un superviseur n'a pas de file de jobs - il possède sa
propre boucle et décide quand dormir, attendre, ou agir.

Le `SupervisorRegistry` démarre chaque superviseur enregistré comme une
tâche Tokio détachée, surveille le `JoinHandle` de chaque tâche, et le
redémarre selon sa `RestartPolicy` quand elle se termine - que ce soit en
retournant `Err`, en retournant `Ok`, ou en paniquant. Les redémarrages
sont espacés par un backoff exponentiel qui commence à 100 ms et plafonne
à 60 secondes, de sorte qu'un superviseur qui plante ne s'emballe pas et
n'inonde pas les journaux.

## Démarrage rapide

Définissez un superviseur, enregistrez-le via `inventory::submit!`, et
appelez `SupervisorRegistry::start_all()` à l'amorçage.

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// Utilisez le `suprnova::inventory` réexporté pour qu'une application scaffoldée
// n'ait pas besoin d'ajouter `inventory` comme dépendance directe.
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

C'est toute la configuration. Le superviseur `LogHeartbeat` démarre à
l'amorçage, journalise toutes les 60 secondes, et - parce que
`RestartPolicy::Always` redémarre aussi bien sur une sortie `Ok` que sur
une sortie `Err` - est redémarré immédiatement si la boucle se termine
pour quelque raison que ce soit.

## Politiques de redémarrage

Chaque superviseur déclare sa `RestartPolicy` via la méthode du trait. La
valeur par défaut est `OnError`.

| Politique | Redémarre quand... | Cas d'usage |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` retourne `Err` ou panique | Tâches qui doivent s'exécuter jusqu'à leur terme en cas de succès (par ex., un job d'initialisation ponctuel enveloppé comme un superviseur). |
| `RestartPolicy::Always` | `run()` retourne soit `Ok` soit `Err`, ou panique | Vrais daemons - boucles qui ne devraient jamais retourner. Si la boucle se termine pour quelque raison que ce soit, c'est un bug et un redémarrage est justifié. |
| `RestartPolicy::Never` | (jamais) | Tâches ponctuelles qui doivent s'exécuter une seule fois et ne pas être redémarrées, quel que soit le résultat. |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // défaut
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // boucle de daemon
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // ponctuel
```

**Quand choisir `Always` plutôt que `OnError`.** Un superviseur à boucle
infinie (`loop { ... }`) devrait utiliser `Always` - si la boucle retourne
un jour `Ok(())`, quelque chose d'inattendu s'est produit et un
redémarrage est la bonne réponse. Un superviseur qui effectue un travail
fini et retourne `Ok` en cas de succès (par ex., rafraîchir un cache une
seule fois) devrait utiliser `OnError` pour qu'une fin propre ne déclenche
pas de redémarrage.

**`Never` pour le travail ponctuel.** Préférez les [workers de file
d'attente](queues.md) ou les [tâches planifiées](scheduling.md) pour le
travail qui s'exécute selon une planification. Utilisez
`RestartPolicy::Never` quand le motif superviseur est pratique pour
quelque chose qui doit s'exécuter une fois au démarrage et jamais plus.

## Gestion des paniques

Les paniques à l'intérieur de `run()` sont capturées par le registre et
traitées comme des erreurs - un superviseur qui panique est redémarré avec
un backoff plutôt que de faire planter le processus. Le registre surveille
le `JoinHandle` de chaque superviseur et détecte les paniques via le
mécanisme de jointure standard de Tokio.

Du point de vue de la politique de redémarrage, une panique est toujours
traitée comme une sortie `Err`, quelle que soit la politique :

- `OnError` - redémarre après une panique (la panique compte comme une erreur).
- `Always` - redémarre après une panique (comme n'importe quelle autre sortie).
- `Never` - ne redémarre pas après une panique (comme n'importe quelle autre sortie).

La panique est journalisée au niveau `error!` avec le nom du superviseur
avant que le backoff de redémarrage ne commence.

## Backoff

Quand un superviseur se termine et que sa politique indique de redémarrer,
le registre attend avant de lancer le remplaçant :

| Redémarrage consécutif | Délai |
|---------|-------|
| 1er | 100 ms |
| 2e | 200 ms |
| 3e | 400 ms |
| 4e | 800 ms |
| ... | double chaque fois |
| Plafonné | 60 s |

Le backoff se réinitialise après une exécution saine. Le délai double à
chaque redémarrage *consécutif* jusqu'au plafond de 60 s, mais une
exécution qui reste active au moins 60 s (la durée du plafond) est traitée
comme saine : le redémarrage suivant retombe au plancher de 100 ms au lieu
d'hériter du backoff qui avait grimpé pendant une rafale d'échecs
antérieure. Ainsi, un daemon qui a fonctionné proprement pendant des
heures puis a un incident redémarre promptement, pas après une attente de
60 s accumulée longtemps auparavant.

La réinitialisation est basée sur la vivacité, et délibérément
conservatrice : seule une exécution qui *survit plus longtemps que le
backoff maximal possible* compte comme saine. Une exécution qui se termine
avant ce seuil transmet le backoff courant à la suivante, si bien qu'un
superviseur véritablement instable - dont les exécutions n'atteignent
jamais le seuil - continue de grimper jusqu'au plafond de 60 s et y reste.
La réinitialisation ne masque jamais un superviseur qui plante en boucle.

Le plafond de 60 secondes empêche un superviseur définitivement cassé de
dormir indéfiniment ou de marteler des dépendances externes à chaque
réessai. Combinez avec la journalisation au niveau `error!` pour alerter
quand un superviseur entre dans la tranche de backoff élevé.

## Arrêt gracieux

Les superviseurs reçoivent un `CancellationToken` comme paramètre de
`run()`. Le framework annule ce jeton sur Ctrl-C / SIGTERM dans le cadre
de la séquence d'arrêt de `Server::run`. Les superviseurs qui veulent
vider leur état, terminer le travail en vol, ou sinon sortir proprement
devraient faire un `tokio::select!` sur `cancel.cancelled()` :

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

Le framework vide le `JoinSet` des superviseurs avec une fenêtre de grâce
de 5 secondes après l'annulation. Les superviseurs qui n'honorent pas le
jeton dans cette fenêtre sont interrompus via `JoinSet::abort_all`. Ce
vidage s'exécute après le vidage des handlers WebSocket (pour que les
connexions WS se nettoient d'abord) et avant le vidage des buffers de
télémétrie.

Les superviseurs qui ignorent complètement le jeton s'exécuteront jusqu'à
l'expiration de la fenêtre de 5 secondes, puis seront interrompus de
force. Si votre superviseur détient des ressources qui doivent être vidées
(descripteurs de fichiers ouverts, requêtes HTTP en vol, enregistrements
partiellement écrits), faites toujours un select sur `cancel.cancelled()`
et nettoyez avant de retourner.

### Intégrateurs et tests d'intégration

`Server::run` appelle `SupervisorRegistry::shutdown(...)` pour vous. Le
code qui appelle `SupervisorRegistry::start_all()` en dehors de
`Server::run` (les intégrateurs qui pilotent le framework depuis un
binaire personnalisé, ou les tests d'intégration qui démarrent des
superviseurs directement) doit aussi appeler
`SupervisorRegistry::shutdown(timeout)` au démontage, sinon les tâches de
superviseur fuiteront par-delà la durée de vie du test :

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// Mise en place du test
SupervisorRegistry::start_all().await;

// ... exercer le superviseur ...

// Démontage du test - annule le jeton partagé, vide le JoinSet jusqu'à
// `timeout`, puis `abort_all` pour les traînards.
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

`shutdown` est sans effet si `start_all` n'a jamais été appelé, donc il
est sûr de l'appeler depuis le démontage sans condition.

## Observabilité

Chaque redémarrage sur le chemin d'erreur émet une entrée de journal au
niveau `error!` avec des champs structurés :

- `supervisor` - depuis `Supervisor::name()`.
- `error` - le message d'erreur de la valeur de retour `Err` de `run()`, ou `"panic: <payload>"` pour une panique capturée, ou `"join error: <detail>"` pour un échec de jointure inhabituel.
- `backoff_ms` - le délai de backoff en millisecondes avant le prochain lancement.

Les paniques sont rapportées via le même journal d'erreur - il n'y a pas
de message distinct du type « a paniqué » :

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

`RestartPolicy::Always` qui retourne `Ok(())` émet un `warn!` (pas un
`error!`) avec les mêmes champs `supervisor` / `backoff_ms` et le message
"supervisor returned Ok under Always policy; restarting" - utile pour
repérer les boucles de daemon qui se sont terminées proprement alors
qu'elles n'auraient pas dû.

Les superviseurs n'obtiennent pas de span `tracing` automatique autour de
`run()` - le registre couvre le cycle de vie (démarrage, redémarrage) mais
pas l'intérieur de la tâche. Émettez votre propre `info_span!` ou
instrumentez le corps de votre boucle si vous voulez un contexte de span
sur le travail effectué à l'intérieur du superviseur :

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### Pourquoi Suprnova diverge

Laravel n'a pas d'équivalent direct. Le modèle process-par-requête de PHP
rend impossibles les daemons in-process toujours actifs - le travail de
longue durée doit vivre en dehors du cycle de vie de la requête,
typiquement comme un processus worker géré par `supervisord` qui consomme
une file d'attente ou une commande planifiée par cron. Le worker de file
d'attente de Laravel (`php artisan queue:work`) est l'analogue le plus
proche, mais reste un processus CLI ponctuel qu'un superviseur externe
redémarre.

Suprnova s'exécute sur Tokio à l'intérieur d'un unique processus de longue
durée. Les tâches en arrière-plan toujours actives s'intègrent
naturellement comme des tâches Tokio supervisées à côté du serveur HTTP -
pas de frontière de processus supplémentaire, pas de superviseur externe,
pas de canal IPC séparé pour l'état. Le trait `Supervisor` est
l'équivalent in-process de `supervisord`, cantonné à l'arbre de tâches
propre au framework, avec les mêmes garanties de redémarrage-à-la-sortie +
backoff.

Les workers `Queue` (que Laravel possède aussi) sont toujours livrés -
voir [File d'attente](queues.md) - pour le travail en jobs discrets. Les
superviseurs couvrent le cas « toujours en tick » que Laravel repousse
entièrement hors de la frontière du framework.

## Hors du périmètre v1

Les éléments suivants sont intentionnellement différés :

- **Arbres de supervision (parent/enfant).** Il n'y a pas de hiérarchie - tous les superviseurs sont des pairs sous l'unique `SupervisorRegistry`. La supervision structurée (où un superviseur possède et redémarre des superviseurs enfants) relève du territoire de l'orchestrateur.

- **Limites de ressources (cgroup, mémoire, CPU).** Appliquez les contraintes de ressources via des fichiers unit systemd (`MemoryMax=`, `CPUQuota=`) ou des requests/limits de ressources Kubernetes au niveau du pod. Le framework n'impose pas de limites de ressources internes au processus sur les tâches de superviseur individuelles.

- **Supervision multi-machines.** Les superviseurs s'exécutent à l'intérieur d'un seul processus sur une seule machine. Distribuer les décisions de supervision entre plusieurs machines relève du territoire de l'orchestrateur (Kubernetes, Nomad, systemd sur plusieurs hôtes).

## Référence

Les quatre types principaux - `Supervisor`, `RestartPolicy`,
`SupervisorEntry`, `SupervisorRegistry` - sont réexportés à la racine de
la crate (`suprnova::Supervisor`, etc.) en plus du chemin plus long
`suprnova::supervisor::*`. Les deux accesseurs libres restent sous
`suprnova::supervisor::*`.

| Symbole | Objectif |
|--------|---------|
| `Supervisor` | Trait à implémenter sur votre struct de superviseur. Méthodes requises : `name() -> &'static str`, `async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`. Optionnelle : `restart_policy() -> RestartPolicy` (par défaut `OnError`). Le jeton `cancel` est signalé à l'arrêt du processus ; faites un select sur `cancel.cancelled()` pour sortir proprement avant l'expiration de la fenêtre d'interruption de 5 secondes. |
| `RestartPolicy` | Enum avec les variantes `OnError`, `Always`, `Never`. Contrôle quand le registre lance une tâche de remplacement. |
| `SupervisorEntry` | Élément d'inventory. Déclarez `factory: fn() -> Box<dyn Supervisor>`. Soumettez une entrée par superviseur via `suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })`. |
| `SupervisorRegistry::start_all()` | Fn async. Itère sur toutes les valeurs `SupervisorEntry` soumises, lance chaque superviseur comme une tâche Tokio détachée dans le JoinSet par processus, et commence à surveiller les redémarrages. Idempotent - les statics par processus sont des `OnceLock`. Appelez-la une fois depuis votre `register()` d'amorçage. |
| `SupervisorRegistry::shutdown(timeout)` | Fn async. Annule le jeton d'annulation partagé pour que chaque superviseur qui surveille `cancel.cancelled()` sorte, vide le JoinSet jusqu'à `timeout`, puis `abort_all` pour les traînards. `Server::run` invoque ceci dans le cadre de sa séquence d'arrêt ; les intégrateurs et les tests d'intégration qui appellent `start_all` en dehors de `Server::run` doivent l'appeler eux-mêmes pour éviter de faire fuiter des tâches. Sans effet si `start_all` n'a jamais été appelé. |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | Accesseurs qui retournent `Option<&'static …>` vers le JoinSet et le jeton d'annulation sous-jacents. Utilisés par la séquence d'arrêt de `Server::run` ; exposés en `pub` pour que les intégrateurs qui pilotent le framework depuis un binaire personnalisé puissent s'intégrer. Le code applicatif ne devrait pas en avoir besoin. |

## Suivant

- [File d'attente](queues.md) - décision superviseur-contre-worker-de-file-d'attente et l'alternative en jobs discrets
- [Planification](scheduling.md) - pour le travail périodique qui n'a pas besoin d'une boucle de longue durée
- [Flux de travail](workflows.md) - pour le travail avec état, de longue durée, qui a besoin d'une reprise durable
- [Diffusion](broadcasting.md) - utilise la même séquence d'arrêt (ordre de vidage)
- [Cycle de vie des requêtes](lifecycle.md) - où `Server::run` et le vidage d'arrêt s'intègrent
