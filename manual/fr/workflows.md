# Flux de travail

Les flux de travail sont des fonctions async durables et de longue durée
dont l'état intermédiaire survit aux crashes, aux redémarrages et aux
paniques. Tournez-vous vers eux quand une unité de travail s'étend sur
plusieurs étapes - chacune potentiellement lente, faillible, ou à effets
de bord - et que vous ne pouvez pas vous permettre de perdre la
progression à mi-chemin. Le corps d'un flux de travail s'exécute une fois
; la sortie de chaque étape est persistée ; un réessai reprend à partir de
la première étape qui n'est pas encore terminée. Associez-le à
[`Queue`](queues.md) quand le travail est un job ponctuel ; associez-le à
[`Bus`](bus.md) quand le travail s'exécute de façon synchrone dans la
tâche de la requête.

## Démarrage rapide

Un flux de travail est une fonction async qui retourne `Result<T,
FrameworkError>` ; son corps invoque une ou plusieurs fonctions
`#[workflow_step]` ; vous le mettez en file d'attente via la macro
`start_workflow!` et un processus worker le vide.

```rust
use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};

#[workflow_step]
async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
    Ok(format!("user:{}", user_id))
}

#[workflow_step]
async fn send_welcome_email(user: String) -> Result<(), FrameworkError> {
    // … envoie réellement le mail
    Ok(())
}

#[workflow]
async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
    let user = fetch_user(user_id).await?;
    send_welcome_email(user).await?;
    Ok(())
}

// Depuis un handler ou n'importe quel contexte async :
let handle = start_workflow!(welcome_flow, 123).await?;
```

La macro sérialise les arguments en JSON, insère une ligne dans la table
`workflows`, et retourne un [`WorkflowHandle`](#attendre-les-résultats)
identifiant l'instance mise en file d'attente. Un processus worker séparé
récupère la ligne, exécute le corps, et persiste la sortie de chaque étape
au fur et à mesure.

`#[workflow]` collecte la fonction dans l'inventory des flux de travail
sous son chemin pleinement qualifié (`module_path::fn_name`). Des
enregistrements en double sous le même nom interrompent le démarrage du
worker via `registry::assert_no_duplicates` - un masquage silencieux
serait indéboguable, donc le framework échoue explicitement.

## Schéma

Les flux de travail persistent dans deux tables : `workflows` (une ligne
par instance) et `workflow_steps` (une ligne par invocation d'étape,
indexée par `(workflow_id, step_index)`). Le framework possède le schéma ;
vous choisissez quand l'appliquer.

Deux façons de câbler les migrations.

### Fichiers de migration générés

Le CLI scaffolde des copies des migrations du framework dans votre
application :

```bash
suprnova workflow:install
suprnova migrate
```

`workflow:install` écrit `m_create_workflows_table.rs` et
`m_create_workflow_steps_table.rs` sous `src/migrations/`, puis les
enregistre dans votre `Migrator`. Utilisez ceci quand vous voulez que le
schéma soit versionné avec le reste des migrations de votre application.

### Enregistrement programmatique

Autrement, enregistrez directement les structs de migration possédées par
le framework :

```rust
use sea_orm_migration::MigratorTrait;
use suprnova::workflow::migrations::{
    CreateWorkflowsTable, CreateWorkflowStepsTable,
};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(CreateWorkflowsTable),
            Box::new(CreateWorkflowStepsTable),
        ]
    }
}
```

Les deux voies produisent un SQL identique. La même convention est
utilisée par [`features::migrations`](feature-flags.md) et
[`payments::migrations`](payments.md).

## Exécuter le worker

Dans une application scaffoldée, le worker est démarré par la
sous-commande `workflow:work` du binaire :

```bash
suprnova workflow:work
```

Le worker exécute le même amorçage que votre serveur HTTP, si bien que les
observateurs, les écouteurs, et les liaisons du conteneur enregistrés dans
`bootstrap()` sont visibles pour les étapes de flux de travail. Sur
`SIGINT` / `SIGTERM`, le worker arrête de tirer de nouvelles réclamations
et attend chaque flux de travail en vol avant de sortir - aucun flux de
travail ne se retrouve orphelin en plein milieu d'une étape lors d'un
arrêt propre.

Le chemin de réclamation (`claim_next_workflow`) utilise `FOR UPDATE SKIP
LOCKED` sur la table `workflows`, donc le processus worker **nécessite
Postgres**. SQLite et MySQL fonctionnent pour les tests et pour le chemin
de mise en file d'attente/persistance, mais le daemon worker se terminera
avec une erreur à la première réclamation si la connexion n'est pas
Postgres.

## Configuration

Cinq variables d'environnement règlent le worker. Les valeurs hors limites
sont ramenées à des minimums sûrs avec un `tracing::warn!`, si bien
qu'une faute de frappe dans `.env` ne peut pas rendre le daemon
inutilisable.

| Variable | Défaut | Notes |
|---|---|---|
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` | Pause entre les cycles de réclamation vides |
| `WORKFLOW_CONCURRENCY` | `4` | Nombre max de flux de travail actifs par worker (min 1) |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` | Durée du bail avant qu'un autre worker ne puisse le reprendre |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | Budget de tentatives par flux de travail (min 1) |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | Backoff linéaire : `attempts * value` (min 0) |

Pour les configs programmatiques (construites dans le code plutôt
qu'analysées depuis l'env), appelez `WorkflowConfig::validate()` pour
échouer rapidement sur les mêmes invariants avant de construire un
`WorkflowWorker`.

## Récupération après crash

Trois couches de protection empêchent les flux de travail de rester
bloqués à cause d'échecs de worker.

**Limite de panique.** Le corps du flux de travail s'exécute à l'intérieur
de `AssertUnwindSafe(...).catch_unwind()`. Une panique dans n'importe
quelle étape est capturée, le payload est capturé dans la colonne
d'erreur, et la ligne passe par la même comptabilité de réessai/échec
qu'un `Err` retourné. Sans cette limite, une panique sauterait le chemin
de clôture et laisserait la ligne à `status='running'` pour toujours.

**Battement de cœur du bail.** Une étape de longue durée qui survit plus
longtemps que `WORKFLOW_LOCK_TIMEOUT_SECS` pourrait sinon voir son bail
expirer alors qu'elle est encore en cours d'exécution. Le worker lance
une tâche de battement de cœur qui rafraîchit `locked_until` à la moitié
de l'intervalle de lock-timeout jusqu'à ce que le corps se résolve. Le
battement de cœur s'interrompt au drop, si bien qu'un `?` retourné ne
peut pas faire fuiter une tâche de renouvellement et geler le bail pour
un flux de travail que personne n'exécute.

**Reprise de bail expiré.** Quand un worker meurt sans jamais libérer son
verrou (kill forcé, crash de l'hôte, OOM du kernel), la ligne reste à
`status='running'` jusqu'à ce que `locked_until` soit dépassé. La requête
de réclamation récupère explicitement ces lignes : tout flux de travail
`running` dont le bail a expiré devient réclamable par un autre worker au
tour suivant, avec `attempts` incrémenté. La récupération après crash est
automatique - il n'y a rien à scripter et aucune commande admin à retenir.

## Sémantique de livraison - au moins une fois

Les corps d'étape s'exécutent avec une sémantique **au moins une fois**.
Une étape peut s'exécuter plus d'une fois dans deux situations :

1. **`Err` retourné** - le flux de travail est remis en file d'attente ; au réessai, l'étape en échec s'exécute à nouveau, et toute étape antérieure est rejouée depuis le cache.
2. **Crash après l'effet de bord, avant que `mark_step_succeeded` ne commite** - le bail expire, un autre worker le reprend, ne voit aucune sortie en cache à cet index d'étape, et exécute le corps à nouveau.

Le framework persiste les **sorties** d'étape de façon durable, mais il ne
peut pas observer l'effet de bord lui-même. Rendre les corps d'étape
idempotents est votre responsabilité. Deux motifs fonctionnent pour
presque tous les cas.

**Écritures conditionnelles.** Utilisez `INSERT ... ON CONFLICT DO
NOTHING`, des colonnes de clé d'idempotence, ou des marqueurs
`seen_event_id`. Dérivez une clé stable par étape à partir de données déjà
dans la portée : les arguments d'entrée du flux de travail plus une
étiquette d'étape littérale (`("wf-charge", customer_id)`) suffisent, car
les mêmes arguments correspondent à la même ligne de flux de travail à
travers les réessais.

**Clés d'idempotence externes.** La plupart des API tierces (Stripe, SES,
SQS) acceptent un en-tête `Idempotency-Key`. Passez une clé dérivée de
l'entrée du flux de travail plus une étiquette locale à l'étape
(`format!("wf-charge-{}", customer_id)`) afin que les requêtes réessayées
se dédupliquent chez le fournisseur.

Ne présumez **pas** qu'une étape ayant retourné `Ok` ne peut pas
s'exécuter une seconde fois - un crash peut faire atterrir cette seconde
exécution sur n'importe quel worker suivant, y compris après un
redémarrage sur un hôte différent. Voir le chapitre
[Idempotence](idempotency.md) pour `Idempotency::once`,
`Idempotency::commit_on_success`, et `Idempotency::remember` - tous des
enveloppes valides autour d'un corps d'étape.

## Contrat de déterminisme

Les flux de travail doivent être déterministes à travers les rejeux.
Chaque étape est indexée par `(step_name, step_index)`, et le framework
met en cache son entrée sérialisée à côté de la sortie. Quand une étape au
même index est rejouée avec une entrée sérialisée différente, le framework
retourne une erreur plutôt que de masquer la corruption en retournant la
sortie en cache de l'entrée précédente.

En pratique, cela signifie :

- Ne faites pas de branchement conditionnel sur `Utc::now()`, `rand::random()`, ou d'autres sources non déterministes en dehors d'un `#[workflow_step]`. Les corps d'étape peuvent les appeler librement - leur résultat est capturé dans le cache de sortie de l'étape.
- N'insérez pas d'étapes de façon conditionnelle. Si un réessai rencontre un nombre d'étapes différent avant un index donné, vous obtenez une erreur de non-correspondance de nom d'étape. Placez la logique de branchement à l'intérieur d'une étape.
- Ne changez pas la forme des arguments d'une étape entre deux déploiements sans renommer l'étape. Renommer change `step_name`, ce qui redémarre la mise en cache depuis le début pour cette étape.

## Attendre les résultats

`WorkflowHandle` permet à l'appelant d'interroger la ligne, d'attendre
qu'elle se termine, ou de récupérer la sortie sérialisée.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, WorkflowStatus};

let handle = start_workflow!(welcome_flow, 123).await?;

match handle.wait_with_timeout(Duration::from_secs(30)).await {
    Ok(WorkflowStatus::Succeeded) => { /* terminé */ }
    Ok(WorkflowStatus::Failed) => { /* colonne d'erreur persistée */ }
    Ok(_) => unreachable!("wait_* only returns terminal status"),
    Err(FrameworkError::Internal { message }) if message.contains("Timed out") => {
        // Le flux de travail est toujours en cours ; on retombe sur l'UX asynchrone.
    }
    Err(other) => return Err(other),
}
```

`wait()` interroge indéfiniment - à utiliser seulement dans des tests ou
des scripts de courte durée où bloquer pour toujours est acceptable. Pour
les chemins de requête HTTP, `wait_with_timeout(Duration)` gagne toujours
contre la boucle d'interrogation interne, même si la requête de statut
sous-jacente se bloque. Une erreur de timeout n'annule **pas** le flux de
travail - le worker continue, et `handle.status().await` retourne l'état
actuel plus tard.

`wait_with_options(Some(poll), Some(deadline))` expose les deux réglages
quand les défauts ne conviennent pas.

Pour des sorties typées, définissez un type de retour `T: Serialize +
DeserializeOwned` sur le flux de travail et appelez
`handle.output::<T>().await?`. Le JSON brut est disponible via
`output_raw()`.

## Mise en cache des étapes, en détail

La mise en cache des étapes est indexée par **nom d'étape + index
d'étape**. La première invocation d'une étape persiste son JSON d'entrée,
exécute le corps, et en cas de succès persiste le JSON de sortie. Un rejeu
au même index :

- Retourne la sortie en cache si l'étape est `succeeded` et que l'entrée rejouée correspond à l'entrée en cache.
- Retourne une erreur si l'entrée diffère (le garde-fou de déterminisme).
- Réexécute le corps si l'étape est `running` ou `failed` (aucune sortie en cache à retourner).

Les index d'étape sont assignés par un `AtomicI32` par contexte de flux de
travail, si bien que l'ordre est déterminé par les appels que fait le
corps de votre flux de travail. Un branchement qui produit une étape
différente au même index lors d'un réessai remonte comme une erreur de
non-correspondance de nom d'étape plutôt que de corrompre silencieusement
les étapes en aval.

Les sorties et les entrées sont stockées en JSON TEXT, donc tous les types
de retour et arguments d'étape doivent être `Serialize +
DeserializeOwned`.

## Détecter le contexte de flux de travail depuis un helper

`WorkflowContext::is_active()` retourne si la tâche courante s'exécute
sous un flux de travail. Utilisez-la depuis des helpers qui doivent se
comporter différemment à l'intérieur qu'à l'extérieur du worker - par
exemple, un logger qui n'attache l'étiquette de flux de travail que
lorsqu'il en existe une :

```rust
use suprnova::workflow::WorkflowContext;

fn maybe_workflow_tagged(message: &str) -> String {
    if WorkflowContext::is_active() {
        format!("[workflow] {message}")
    } else {
        message.to_string()
    }
}
```

En dehors d'un flux de travail (appelée directement depuis un test ou un
handler), une fonction `#[workflow_step]` s'exécute quand même -
`WorkflowContext::current()` retourne simplement `None`, le corps
s'exécute sans persistance, et l'étape contourne entièrement le cache.
C'est intentionnel : cela rend les fonctions d'étape individuellement
testables sans avoir à faire tourner un worker.

### Pourquoi Suprnova diverge

Laravel n'a pas de primitive de flux de travail de premier ordre - les
jobs sont le voisin le plus proche, mais ils réessaient en réexécutant
tout le corps du job, pas en reprenant depuis la dernière étape réussie.
Suprnova livre les flux de travail comme une construction séparée parce
que Tokio rend bon marché le motif « rester connecté à une fonction async
lente pendant une heure », et parce que la persistance au niveau de
l'étape est la bonne abstraction pour toute interaction externe en
plusieurs étapes (provisionner un client, exécuter une saga à travers deux
fournisseurs de paiement, générer un rapport qui implique plusieurs API
amont).

La conception est plus proche de [DBOS](https://www.dbos.dev/) et de
Cadence/Temporal que d'une file d'attente : état durable, rejeu
déterministe, limites d'étape explicites. La différence avec Temporal est
le poids opérationnel - il n'y a pas de service de flux de travail séparé
à faire tourner ; le worker n'est que `suprnova workflow:work` contre
votre base de données applicative.

## Remarques

- Les corps d'étape peuvent retourner n'importe quel type `Serialize + DeserializeOwned`. Le type unité `()` fonctionne pour les étapes qui n'existent que pour leur effet de bord.
- Une fonction `#[workflow_step]` appelée en dehors d'un contexte de flux de travail s'exécute en ligne - pas de mise en cache, pas de rejeu. C'est ainsi que les tests exercent directement les corps d'étape.
- La mise en cache des étapes est indexée par `(step_name, step_index)` ; renommez une étape (ou réordonnez les appels) et la mise en cache se réinitialise pour cette étape au prochain rejeu.
- `start_workflow!` accepte n'importe quel tuple d'arguments sérialisables. Les tuples préservent l'ordre des arguments, donc renommer des paramètres positionnels est sûr ; changer les types d'arguments casse le schéma pour tout flux de travail en vol.
- La couche d'[observabilité](observability.md) du framework capture les journaux structurés du worker (`worker_id`, `workflow_id`, `attempts`, `max_attempts`) sur chaque chemin de clôture, si bien que vous pouvez auditer les budgets de réessai en production sans instrumenter vos étapes.

## Suivant

- [File d'attente](queues.md) - jobs ponctuels en arrière-plan avec drivers sync/redis/database
- [Idempotence](idempotency.md) - enveloppes pour la livraison au moins une fois
- [Bus](bus.md) - dispatch de commande synchrone avec résultats typés
- [Superviseurs](supervisors.md) - supervision de tâches de longue durée avec redémarrage automatique par capture de panique
- [Modèle d'erreur](error-model.md) - `FrameworkError`, la limite de panique, et pourquoi la clôture passe par `?`
