# Contexte

`Context` est le sac clé/valeur par requête de Suprnova. C'est là que
vous rangez les données que vous voulez rendre visibles à tous les
appelants en aval de la même requête - un id de requête, un slug de
tenant, un rôle utilisateur, une piste d'audit - sans faire passer la
valeur par chaque signature de fonction. C'est l'équivalent Suprnova de
la façade `Context` de Laravel.

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

Recourez-y quand :

- Une ligne de journal, un job en file d'attente ou un message de
  diffusion a besoin de métadonnées cantonnées à la requête (id de
  tenant, id de corrélation, rôle utilisateur)
- Un helper profondément imbriqué a besoin d'une valeur que le handler
  possède déjà, mais la chaîne d'appels ne devrait pas transporter un
  paramètre à travers chaque couche
- Vous voulez lire la chaîne de requête de la requête courante
  (`?page=3`, `?cursor=…`) depuis du code qui n'est pas un handler

`Context` n'est **pas** fait pour l'état inter-requêtes. Il est lié à la
tâche Tokio courante et disparaît à la fin de la requête. Pour ce qui
survit à une requête, utilisez le [Conteneur de service](container.md) ou
le [Cache](cache.md).

## Les deux sacs

Chaque portée `Context` active porte deux maps clé/valeur et un
emplacement supplémentaire :

| Sac | Lu avec | Apparaît dans `Context::all()` |
|---|---|---|
| **Visible** | `Context::get` | Oui |
| **Caché** | `Context::hidden_get` | Non |
| **Requête** | `Context::query_param` | Non (instantané distinct des paires `?key=value` de l'URL) |

La séparation entre visible et caché est toute la raison d'être de ces
deux sacs : les sérialiseurs de journaux qui déversent `Context::all()`
dans une sortie structurée ne feront pas fuiter les données que vous
cachez intentionnellement. Mettez les métadonnées d'audit dans le sac
visible ; mettez dans le sac caché les clés d'API, les jetons bearer
OAuth et les données personnelles que vous ne voulez pas voir dans les
journaux.

Le sac de requête est peuplé automatiquement par le middleware de requête
du framework à partir de la chaîne de requête de l'URL (voir [La
pagination lit les paramètres de
requête](#la-pagination-lit-les-paramètres-de-requête) ci-dessous). Vous ne faites
généralement que le lire, jamais l'écrire.

## La portée active

Une portée `Context` est installée par le framework sur chaque requête
HTTP entrante. À l'intérieur d'un handler, d'un middleware, d'un
observateur de modèle, d'un écouteur d'événement ou de tout autre code
atteignable depuis la tâche de la requête, la portée est vivante et les
lectures et écritures `Context::*` fonctionnent sans cérémonie.

En dehors d'une portée - du code d'amorçage précoce, un `tokio::spawn` nu
qui n'hérite pas du contexte, un test unitaire qui n'en installe pas -
chaque mutation est **silencieusement sans effet** et chaque lecture
retourne `None`. Le contrat est le suivant : jamais de panique, quel que
soit l'endroit d'où vous appelez.

```rust
// Dans un handler - la portée est active, tout fonctionne :
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// En dehors d'une portée - sans effet silencieux + None :
Context::add("user_id", 42i64);            // abandonné
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

Le contrat sans panique est délibéré. Le code de bibliothèque qui touche
à `Context` (un subscriber de journal personnalisé, une extension SDK) ne
devrait pas avoir besoin de savoir s'il s'exécute à l'intérieur d'une
requête ou à l'amorçage - il devrait simplement appeler `Context::get` et
traiter `None` comme « pas disponible pour l'instant ».

### Observabilité des opérations silencieuses

Une absence d'effet vraiment silencieuse masquerait des bugs (middleware
dans le mauvais ordre, contexte non propagé dans une tâche lancée en
spawn, lecture accidentelle au moment de l'amorçage). Les opérations de
mutation du framework restent sans panique, mais émettent un événement
`tracing::trace!` sur la cible `suprnova::context` chaque fois qu'elles
abandonnent quelque chose :

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

Trois classes d'événement :

| Événement | Quand il se déclenche |
|---|---|
| `mutation discarded: no active scope` | `add`, `push`, `hidden_add`, `forget` appelées en dehors de toute portée |
| `mutation discarded: value failed to serialize` | l'impl `Serialize` de la valeur passée à `add`/`push`/`hidden_add` a échoué |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` a trouvé la clé, mais le JSON stocké ne correspond pas au `T` demandé |

L'absence pure et simple - un `get` sur une clé jamais définie - reste
silencieuse, afin que les sondes « est-ce défini ? » n'inondent pas les
journaux. Activez `RUST_LOG=suprnova::context=trace` quand vous
soupçonnez un bug de propagation ; le chemin silencieux sans effet
devient visible sans rien changer au comportement du code de production.

## Ajouter des valeurs

### `Context::add` - remplacer à une clé

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // n'importe quelle valeur Serialize
```

La clé est `Into<String>` ; la valeur est n'importe quel type
`Serialize`. La valeur est convertie une seule fois en
`serde_json::Value` au moment de l'écriture et stockée ainsi. Un `add`
ultérieur sur la même clé remplace.

### `Context::push` - ajouter à une pile

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` initialise un tableau vide au premier appel et ajoute à la fin
lors des appels suivants. Si un scalaire existe déjà à la clé, il est
converti en un tableau `[scalar, new_value]` - `push` est indulgente
vis-à-vis des `add` antérieurs sur la même clé.

### `Context::hidden_add` - écrire dans le sac caché

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// Un déversement du sac visible (par ex. un émetteur de journaux JSON)
// ne les voit pas :
let all = Context::all();
assert!(!all.contains_key("api_key"));

// Mais vous pouvez toujours les lire délibérément :
let key: Option<String> = Context::hidden_get("api_key");
```

Le sac caché est indexé indépendamment du sac visible - un
`hidden_add("user_id", 99)` et un `add("user_id", "alice")` coexistent
sans collision. `Context::forget(key)` retire des deux sacs en un seul
appel.

## Lire des valeurs

### `Context::get` - lecture typée depuis le sac visible

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` est générique sur `T: DeserializeOwned`. La valeur JSON stockée est
désérialisée à chaque lecture. Retourne `None` quand :

- La clé n'est pas définie
- Aucune portée n'est active sur la tâche courante
- La valeur stockée ne se désérialise pas en `T` (par ex. vous avez
  stocké un `i64` et demandé une `String`)

Le dernier cas émet un `tracing::trace!` afin que le bug de mauvais type
soit observable - un `Context::get` qui a l'air de dire « la valeur n'est
pas définie » alors qu'il dit en réalité « la valeur n'a pas la bonne
forme » est le genre de bug qui coûte une heure de recherche sans une
ligne de journal pour le désigner.

### `Context::hidden_get` - lecture typée depuis le sac caché

Même forme que `get`, mais lit le sac caché. Même comportement de tracing
en cas de mauvais type.

### `Context::has` - vérification d'existence sur le sac visible

```rust
if Context::has("user_id") {
    // …
}
```

`has` ne vérifie que le sac visible (utilisez `hidden_get(...).is_some()`
s'il vous faut sonder le sac caché).

### `Context::all` - instantané du sac visible

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

Retourne une `HashMap` vide en dehors d'une portée. C'est ce qu'un
émetteur de journaux JSON devrait appeler pour injecter les champs
cantonnés à la requête dans chaque ligne de journal - et la raison pour
laquelle le sac caché existe séparément.

### `Context::forget` - retirer une clé des deux sacs

```rust
Context::forget("trail");          // retire du visible ET du caché
```

Le retrait dans les deux sacs est intentionnel. Si vous avez stocké des
données liées dans les deux sacs (par ex. `user_id` visible,
`user_email` caché), un seul `forget` nettoie les deux.

## Lire les paramètres de requête

`Context::query_param` lit dans les paires `?key=value` de l'URL,
capturées à l'entrée de la requête. Le middleware de requête analyse la
chaîne de requête une seule fois dans le sac de requête de la portée,
puis chaque appelant en aval peut lire les paramètres un à un, par leur
nom, sans réanalyse :

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

Retourne `None` quand le paramètre est absent ou qu'aucune portée n'est
active. Les clés dupliquées suivent la sémantique « le dernier gagne » de
Laravel - la même valeur que vous donnerait la map de paramètres analysée
de la requête.

### La pagination lit les paramètres de requête

C'est la raison d'être du sac de requête. Les paginateurs d'Eloquent
lisent `?page=` et `?cursor=` directement depuis `Context::query_param`,
si bien qu'un handler qui retourne un paginateur n'a pas besoin de faire
circuler le numéro de page à la main :

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // Lit ?page=N depuis l'URL de la requête via Context::query_param
    // - pas de code répétitif req.query(), pas de paramètre à faire circuler.
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

Trois points d'entrée de paginateur s'en servent :

- `Builder::paginate(per_page)` - lit `?page=`
- `Builder::simple_paginate(per_page)` - lit `?page=`
- `Builder::cursor_paginate(per_page)` - lit `?cursor=`

Voir [Pagination](pagination.md) pour toute la surface.

## Propager dans les tâches lancées en spawn

`tokio::spawn` démarre la tâche enfant avec un environnement task-local
neuf - la portée `Context` du parent n'y entre **pas**. Un
`tokio::spawn` nu à l'intérieur d'une requête voit un `Context` vide, et
chaque lecture retourne `None`.

Pour transporter la portée dans un spawn, prenez-en un instantané avec
`Context::current()` et réentrez dedans, à l'intérieur de l'enfant, avec
`Context::scope` :

```rust
use suprnova::context::Context;

// À l'intérieur d'un handler de requête :
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // Désormais `Context::get`, `Context::query_param`, etc. voient
        // le sac de la requête parente.
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

Le magasin que retourne `Context::current()` partage les maps sous-jacentes
du parent via un `Arc` - les écritures de l'enfant sont visibles du
parent aussi longtemps que l'enfant détient le clone. C'est exactement ce
que veulent les spawns d'audit et de journalisation : l'enfant peut
estampiller des clés supplémentaires
(`Context::add("audit.completed", true)`) et la ligne de journal finale
du parent les voit.

Si vous avez besoin d'un instantané isolé (les écritures de l'enfant ne
doivent pas refluer vers le parent), construisez un `ContextStore` neuf
et n'y copiez que les clés dont vous avez besoin.

### Pourquoi un `spawn` nu ne propage pas

Les task-locals de Tokio (`tokio::task_local!`) sont intentionnellement
cantonnés à une tâche. Un héritage automatique à travers les spawns
signifierait :

- Les tâches d'arrière-plan de longue durée épingleraient pour toujours
  les maps de contexte de leur parent
- Une panique dans une tâche enfant pourrait empoisonner l'état du parent
- Le runtime devrait parcourir une chaîne de pointeurs vers les parents à
  chaque lecture de task-local

La danse explicite `Context::current()` + `Context::scope` fait de la
propagation une décision délibérée plutôt qu'une valeur par défaut
cachée.

## Tests

À l'intérieur de `#[tokio::test]` ou de `#[suprnova_test]`, aucune portée
`Context` n'est installée par défaut. La plus grande partie du code testé
qui touche au contexte gère élégamment le cas « aucune portée » (absence
d'effet silencieuse + lectures à `None`), si bien que les tests unitaires
ordinaires n'ont besoin d'aucune mise en place.

Deux situations où le test a besoin d'un coup de main :

### Quand le code testé appelle `query_param`

Les helpers de pagination lisent `?page=` via `Context::query_param`. Un
test unitaire pour « la page 3 retourne le bon décalage » a besoin que
`query_param` retourne `Some("3")`. Deux façons de faire :

**`test_query_guard` (recommandé) :**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // Le code testé voit maintenant ?page=3
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` est détruit en fin de portée - la substitution thread-local est effacée.
```

`test_query_guard` retourne une garde RAII. Même si le corps du test
panique, `Drop` s'exécute et efface la substitution thread-local avant
que le thread système ne soit recyclé. La garde est `#[must_use]` - la
lier à `_` efface immédiatement, ce qui n'est presque jamais ce que vous
voulez.

**La paire nue `test_set_query` + `test_clear_query` :**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // efface la fuite d'un test voisin
    Context::test_set_query("page", "5");

    // … assertions …

    Context::test_clear_query();
}
```

Utilisez la forme avec garde. La paire manuelle existe pour les cas où
vous devez poser et effacer plusieurs substitutions indépendamment, mais
la garde `#[must_use]` est plus difficile à mal employer.

Les deux API sont conditionnées par
`#[cfg(any(test, feature = "testing"))]` - elles sont compilées dans les
binaires de test et dans les builds release qui activent la feature
`testing` pour les harnais de tests d'intégration. Elles n'existent pas
dans un build release ordinaire.

### Quand le code testé lit ou écrit dans une portée `Context`

Installez-en une explicitement via `Context::scope` :

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

Ou déposez un sac de requête à la création de la portée :

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` est le constructeur qu'utilise le
middleware de requête, si bien qu'un test qui exerce le même chemin de
code qu'en production voit un sac de requête de la même forme.

### Pourquoi la substitution thread-local existe

La substitution des paramètres de requête est un `thread_local!`, pas un
task-local. C'est délibéré : cela permet aux tests d'installer des
paramètres de requête **sans envelopper chaque assertion dans un appel à
`Context::scope`**. La combinaison est la suivante :

1. Les lectures consultent d'abord la substitution thread-local
2. S'il n'y a pas de substitution, elles lisent le sac de requête de la
   portée task-local `CONTEXT`
3. S'il n'y a pas non plus de portée, elles retournent `None`

La recherche thread-local ne coûte pratiquement rien en production (la
substitution est toujours vide en dehors des builds de test) et épargne
aux auteurs de tests les enveloppes répétitives `Context::scope(...)`
autour de chaque assertion liée à la pagination.

## Motifs courants

### Estampiller l'id de requête sur chaque journal

Le framework le fait déjà. Le middleware de requête dépose `_request_id`
dans le sac visible, afin que les jobs en aval, les diffusions et les
déversements de journal via `Context::all()` puissent lire l'id par son
nom. Le même middleware ouvre aussi un span `tracing` portant l'id comme
champ de span, et c'est ce qui le fait apparaître sur chaque ligne de
journal émise à l'intérieur de la requête - voir
[Journalisation](logging.md) pour le côté subscriber. Lire l'id depuis
`Context` est la bonne voie quand vous avez besoin de la valeur sous
forme de chaîne (par exemple pour l'injecter dans une requête HTTP
sortante en tant qu'en-tête de corrélation) :

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### Transporter le contexte de tenant dans un job en file d'attente

`Context` ne se propage pas automatiquement à travers la limite de
sérialisation / désérialisation de la file d'attente - le worker
s'exécute dans un processus différent de celui du dispatcher, souvent sur
une autre machine. Passez tout ce dont vous avez besoin dans la charge
utile du job :

```rust
use suprnova::{Context, FrameworkError, Queue};

// Dans un handler :
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

Quand le worker traite `SendInvoice`, installez une portée `Context`
neuve en tête de `Job::handle` et redéposez les clés dont vous avez besoin
depuis la charge utile du job - un
`Context::scope(ContextStore::default(), async { ... })` qui enveloppe le
corps. Ensuite, toute journalisation et tout helper profondément imbriqué
qu'appelle le job voient le même id de tenant qu'à l'intérieur d'une
requête.

C'est aussi là que `hidden_add` justifie son existence - le job peut
récupérer et ranger une clé d'API une seule fois à l'entrée de la portée,
et chaque appel HTTP en aval à l'intérieur du job la lit via
`Context::hidden_get` sans la récupérer à nouveau. Voir [File
d'attente](queues.md) pour la forme du trait `Job`.

### Piste d'audit à travers une requête

```rust
Context::push("audit.steps", "validated_input");
// … encore du travail …
Context::push("audit.steps", "charged_card");
// … encore du travail …
Context::push("audit.steps", "sent_receipt");

// Dans un middleware au moment de la réponse :
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

Un middleware au moment de la réponse, qui s'exécute après le handler,
peut déverser la piste d'audit en une seule ligne de journal, au lieu
d'éparpiller dans le journal de la requête une ligne de debug par étape.

### Le sac caché pour les identifiants d'une extension SDK

```rust
// À l'entrée de la requête, après l'authentification :
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// Au plus profond d'un appel SDK :
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

Les journaux qui déversent `Context::all()` n'affichent pas la clé. Le
sac caché est le bon endroit pour tout identifiant que le handler doit
faire descendre profondément dans une pile d'appels sans l'exposer aux
surfaces de journalisation.

## Pourquoi Suprnova diverge

La façade `Context` de Laravel (introduite dans Laravel 11) en est
l'inspiration - mêmes noms de méthodes, même séparation visible/caché,
même contrat « silencieux en dehors d'une requête ». Deux différences
viennent du runtime de Rust :

**La propagation asynchrone est explicite, pas magique.** Le `Context` de
Laravel traverse automatiquement les jobs en file d'attente parce que
Laravel sérialise le sac de contexte dans la charge utile du job au
moment du dispatch. Le modèle asynchrone de Rust n'a pas de « requête
courante » unique dans laquelle s'écouleraient les thread-locals -
`tokio::spawn` repart de zéro, et la limite de la file d'attente implique
une sérialisation entre processus. Suprnova expose la primitive de
propagation (`Context::current()` + `Context::scope`) et vous laisse y
souscrire à cette limite, au lieu de prétendre que les tâches héritent
d'un contexte dont elles n'héritent pas.

**Les lectures de mauvais type sont observables.** Un `get::<T>` sur une
valeur stockée avec un autre type retourne silencieusement `None` sous
Laravel (c'est PHP, les types n'étaient de toute façon pas imposés à
l'écriture). Dans Suprnova, la lecture émet un `tracing::trace!` parce
que le cas du mauvais type signale un vrai bug - la valeur a bien été
écrite quelque part, simplement pas avec le type que vous lisez. La trace
vous permet de le trouver dans des exécutions instrumentées sans changer
le contrat sans panique.

La troisième divergence est mécanique : le `Context` de Suprnova est bâti
sur `tokio::task_local!`, si bien que sa durée de vie est liée à la tâche
Tokio, et non à un quelconque état global. Les lectures inter-threads
voient la portée de la **tâche qui s'exécute à cet instant sur ce
thread**, et non la dernière portée installée, quelle qu'elle soit. C'est
ce qui rend la même façade `Context` sûre à appeler depuis un pool de
threads, un acteur ou un corps de `spawn_blocking` - à condition que vous
propagiez la portée dans le spawn.

## Où cela réside

| Sujet | Fichier |
|---|---|
| La façade `Context` + `ContextStore` | `framework/src/context/mod.rs` |
| Installation de la portée sur une requête HTTP | `framework/src/logging/request_id.rs` |
| Les appelants de `Context::query_param` (pagination) | `framework/src/eloquent/builder.rs` |
| Réexports | `framework/src/lib.rs` (`pub use context::{Context, ContextStore}`) |

## Suivant

- [Cycle de vie des requêtes](lifecycle.md) - où la portée `Context` est
  installée sur chaque requête
- [Conteneur de service](container.md) - pour l'état inter-requêtes qui
  survit à une seule tâche
- [Journalisation](logging.md) - comment `Context::all()` finit dans les
  lignes de journal structurées
- [Pagination](pagination.md) - le principal lecteur en aval de
  `Context::query_param`
- [Tests](testing.md) - les motifs `test_query_guard` et `Context::scope`
  pour les tests unitaires
