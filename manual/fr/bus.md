# Bus

Le Bus est le dispatcher de commandes **synchrone** de Suprnova. Vous
définissez une `Command` typée (`{ input, Output type }`), enregistrez un
`Handler` pour elle à l'amorçage, et ensuite n'importe quel code du
processus peut appeler `Bus::dispatch(cmd).await` et récupérer un
`Dispatched<T>` portant le résultat typé du handler.

Le Bus va de pair avec [`Queue`](queues.md) - son pendant asynchrone. Ce
sont deux façades intentionnellement séparées, pas un unique dispatcher
qui route :

| Si vous voulez…                                          | Utilisez            |
|-------------------------------------------------------|----------------|
| Exécuter le travail *maintenant*, dans cette tâche, et récupérer le résultat | `Bus`          |
| Pousser le travail vers un worker, réessayer en cas d'échec, de façon durable  | `Queue`        |

L'appelant choisit explicitement. Suprnova ne livre pas de marqueur
`ShouldQueue` - sur Tokio, les deux chemins sont non bloquants, si bien
que le choix explicite est plus clair et plus rapide qu'un routage
implicite.

## Démarrage rapide

Dix lignes entre la commande et le dispatch :

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // l'id de charge que nous avons récupéré
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// À l'amorçage (une fois) :
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// Dans un handler de requête :
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## Définir des commandes

Une `Command` est n'importe quelle struct sérialisable avec un type
`Output` associé et un `command_name()` unique :

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

`Output` est ce que le handler retourne. Il doit seulement être `Send +
'static` - le chemin de dispatch réel conserve les valeurs natives via
`Box<dyn Any>`, sans aller-retour par serde. Cela signifie que des sorties
non-serde comme `Bytes`, des handles opaques, ou un `Arc<Mutex<…>>` font
l'aller-retour vers l'appelant comme des valeurs vivantes. La borne
`Serialize + DeserializeOwned` sur `Command` elle-même est pour le chemin
de capture des fakes : `Bus::fake()` enregistre chaque commande dispatchée
comme une `serde_json::Value`, si bien que les assertions basées sur des
prédicats (`assert_dispatched`, `assert_dispatched_times`) peuvent les
décoder et les inspecter.

`command_name()` devrait être une chaîne stable, unique par impl `Command`
concrète. Elle apparaît dans les messages d'échec de
`assert_dispatched`/`assert_dispatched_times` et dans les erreurs
retournées quand aucun handler n'est enregistré.

## Enregistrer des handlers

Un `Handler<C>` est une fonction async typée qui prend la commande et
retourne `Result<C::Output, FrameworkError>` :

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

Appelez `Bus::register::<C, H>(handler)` une fois par type de commande à
l'amorçage. Le registre est global ; réenregistrer le même `C` écrase le
handler précédent (les tests s'appuient là-dessus pour substituer des
implémentations) et émet un `tracing::warn!` afin qu'une liaison dupliquée
provenant de deux enregistrements de service à l'amorçage soit visible
dans le journal.

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## Dispatch

`Bus::dispatch::<C>(cmd)` exécute le handler enregistré in-process et
retourne un enum `Dispatched<C::Output>` :

```rust
pub enum Dispatched<T> {
    Executed(T),  // le handler s'est exécuté, voici le résultat
    Captured,    // Bus::fake() était actif, le handler ne s'est PAS exécuté
}
```

`Dispatched<T>` possède quatre helpers :

- `.unwrap_executed()` - retourne la valeur, panique sur `Captured`
- `.executed() -> Option<T>` - convertit en `Option`
- `.is_executed()` - prédicat booléen
- `.is_captured()` - prédicat booléen

Pour les sites d'appel en mode réel, `.unwrap_executed()` est la forme
idiomatique.

### `Bus::chain` - séquentiel

`Bus::chain(Vec<C>)` exécute les commandes une à la fois, en s'arrêtant à
la première erreur (incluse). Toutes les commandes doivent être du même
type. Retourne `Vec<Result<Dispatched<C::Output>, FrameworkError>>` - une
entrée par commande tentée.

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// Collecter les ids de charge réussis jusqu'au premier échec :
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` est homogène uniquement, par conception - le dispatcher
retourne `Dispatched<C::Output>`, qui n'est bien typé que lorsque chaque
entrée partage un seul `Output`. Pour des chaînes hétérogènes façon
Laravel (types de jobs mélangés, chaque étape déclenchant la suivante),
utilisez [`Queue::chain`](queues.md) - la file d'attente encapsule chaque
job dans une enveloppe typée et n'a donc pas la même contrainte.

### `Bus::batch` - concurrent

`Bus::batch(Vec<C>)` exécute les commandes de façon concurrente via
`futures::join_all` et collecte les résultats dans l'ordre d'entrée. Même
contrainte de type homogène que `chain`.

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` est homogène uniquement pour la même raison que `chain`. Pour
des batches hétérogènes et persistés, avec des callbacks de progression,
des événements de cycle de vie, et un `BatchRepository`, utilisez
[`Queue::batch`](queues.md).

## Tests

Installez le fake au début du test. `install_fake()` acquiert un mutex
`FAKE_SERIAL` à l'échelle du processus pour la durée de vie de la garde,
si bien que deux tests `Bus::fake()` parallèles ne peuvent pas écraser
mutuellement leur magasin de capture - le second bloque jusqu'à ce que la
première garde soit détruite. Vous devez quand même marquer le test
`#[serial]` si un test voisin dans le même binaire appelle un vrai
`Bus::dispatch` : un appelant en dispatch réel n'acquiert pas
`FAKE_SERIAL`, donc sans `#[serial]` il peut entrer en course avec un test
fake parallèle et observer `is_active() == true`. `FAKE_SERIAL` supprime
le risque fake-contre-fake, `#[serial]` supprime celui réel-contre-fake.

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

Le fake capture les commandes dispatchées sans exécuter leurs handlers. Un
appel `Bus::dispatch` retourne `Ok(Dispatched::Captured)` (pas de sortie
de handler) au lieu de `Executed`. Les vraies erreurs - échecs
d'encodage/décodage, un handler enregistré manquant avant que le fake ne
soit installé - remontent toujours sous forme d'`Err(_)`.

`install_fake()` retourne un `BusFakeGuard`. Détruisez-le (il est RAII) et
le fake est effacé et le mutex `FAKE_SERIAL` est libéré. L'idiome typique
est `let _guard = install_fake();` au début du test.

### Surface d'assertion

| Assertion                                            | Vérifie…                                                   |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)`                       | au moins une commande de type `C` correspondant à `pred`           |
| `assert_not_dispatched::<C>(pred)`                   | zéro commande de type `C` correspondant à `pred`                  |
| `assert_dispatched_times::<C>(pred, count)`          | exactement `count` commandes de type `C` correspondant à `pred`       |
| `assert_nothing_dispatched()`                        | zéro commande de quelque type que ce soit dispatchée sous le fake actif |

Les quatre paniquent avec `Bus::fake() must be active` si aucun fake n'est
installé. Celles qui sont cantonnées à un type paniquent avec `expected …
dispatched <command_name> …` quand le compte ne correspond pas.
`assert_nothing_dispatched` panique avec `expected no dispatched commands
but found <n>`.

## Quand utiliser `Queue` à la place

Tournez-vous vers [`Queue`](queues.md) quand vous voulez l'un des éléments
suivants :

- **Durabilité à travers les redémarrages.** Un job mis en file d'attente survit à un crash de processus si le driver est `database` ou `redis`.
- **Réessais avec backoff.** Le worker de la file d'attente applique `Job::max_tries` + `Job::backoff` (exponentiel / fixe / séquence) à chaque échec.
- **Timeout par job.** `Job::timeout` + `Job::fail_on_timeout` sont honorés par la boucle du worker.
- **Exécution différée.** `Queue::later(duration, job)` ou `Queue::push_later(job, at)`.
- **Dédoublonnage / idempotence.** `Job::unique_id` + `Queue::push_unique` bloque les resoumissions pendant un TTL configurable.
- **Découpler l'appelant du worker.** Exécutez les jobs sur une flotte séparée de workers `cargo run --bin app -- queue:work`.

Tournez-vous vers `Bus` quand vous voulez l'un des éléments suivants :

- **In-process, exécution immédiate.** Aucune sérialisation entre processus.
- **Résultat typé restitué à l'appelant.** `Dispatched<C::Output>` porte la valeur de retour typée du handler jusqu'au site d'appel.
- **Composition synchrone.** Un handler de requête qui décompose le travail en appels `Command` plus petits et lit chaque résultat en séquence.

Une application typique utilise les deux : les chemins de requête
synchrones dispatchent des opérations qui retournent un résultat via
`Bus`, et le travail « fire-and-forget » / durable passe par `Queue`.

## Suivant

- [File d'attente](queues.md) - pendant asynchrone, drivers, worker, politique de réessai, chaînes et batches hétérogènes
- [Événements](events.md) - dispatcher pub/sub (un événement → plusieurs écouteurs)
- [Flux de travail](workflows.md) - travail avec état de longue durée qui survit aux redémarrages, quand une chaîne ne suffit pas
- [Tests](testing.md) - `#[suprnova_test]`, fakes du conteneur, et le motif de sérialiseur à l'échelle du processus utilisé par `Bus::fake()`
