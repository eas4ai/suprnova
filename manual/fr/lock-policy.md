# Politique de verrouillage

Suprnova est un unique processus Tokio de longue durée, pas une flotte
de workers PHP éphémères. Chaque registre global au processus, chaque
singleton et chaque cache partagé que vous liez à l'amorçage survit à
chaque requête qui le touche. Cela change une chose, petite mais
lourde de conséquences, dans votre façon de recourir à
`std::sync::Mutex` et `std::sync::RwLock` : une panique alors qu'une
garde est détenue *empoisonne le verrou* pour le reste de la vie du
processus, et le prochain appelant doit décider quoi en faire. Ce
chapitre est la politique valable pour tout le projet concernant cette
décision - deux motifs autorisés, quand choisir lequel, et pourquoi
vous ne devriez jamais recourir à un `.lock().unwrap()` brut dans le
code du framework ou de l'application.

## Pourquoi ce chapitre existe

Sous Laravel, vous ne pensiez jamais aux verrous empoisonnés, parce
qu'il n'y en avait pas. PHP fonctionne en shared-nothing : une erreur
fatale démolit le processus d'une requête, la requête suivante démarre
dans un processus neuf, aucun état en mémoire ne survit pour se
corrompre. Suprnova fonctionne à l'exact opposé. Le processus démarre
une fois, les registres se peuplent, et ils restent vivants pendant
toute la durée de vie du binaire. Un handler qui panique alors qu'il
détient une garde d'écriture sur un `RwLock` global au processus
laisse ce verrou *empoisonné* - chaque `.read()` et `.write()` suivant
retourne `Err(PoisonError)` pour toujours, à moins que quelqu'un ne le
récupère explicitement.

L'idiome Rust par défaut - `.lock().unwrap()` - convertit cet `Err` en
une panique. Qui devient à son tour un autre verrou empoisonné quelque
part plus haut dans la pile. Qui fait alors tomber le prochain
sous-système qui le touche. Une seule mauvaise requête dégénère en
cascade jusqu'à un processus à moitié mort.

La politique ci-dessous empêche cette cascade.

> **Portée.** Ceci s'applique à `std::sync::Mutex` et `std::sync::RwLock`, qui portent un état d'empoisonnement. Les cousins async de `tokio::sync` (`Mutex`, `RwLock`, `Semaphore`) ne s'empoisonnent *pas* - une panique alors qu'une garde `tokio::sync::Mutex` est détenue abandonne la garde proprement, et le `.lock().await` suivant réussit. Si votre hot path est async et que vous n'avez pas besoin d'acquérir la garde depuis un contexte synchrone (une impl `Drop`, un callback du framework, une sous-commande CLI), préférez les variantes Tokio et la question disparaît.

## Les deux motifs autorisés

Chaque endroit du framework qui détient un verrou `std::sync` utilise
l'un de ces deux motifs, et aucun autre. Choisissez de la même façon
dans votre propre code.

### Motif 1 - Convertir l'empoisonnement en une erreur retournée

Quand l'appelant retourne déjà `Result<_, E>` et qu'un `?` de plus ne
change pas sa forme, faites remonter l'empoisonnement comme une erreur
et laissez la requête échouer proprement. Le framework utilise des
helpers internes `pub(crate)` (`lock::read`, `lock::write`,
`lock::lock`) qui convertissent une garde empoisonnée en
`FrameworkError::internal("<context> lock poisoned")`, en intégrant un
label fourni par l'appelant afin que les journaux puissent indiquer
quel sous-système s'est empoisonné sans que chaque site d'appel
n'enveloppe lui-même l'erreur.

Le motif que ces helpers encodent est assez court pour être écrit en
ligne dans le code de votre application :

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use suprnova::FrameworkError;

static FEATURE_FLAGS: RwLock<HashMap<String, bool>> = RwLock::new(HashMap::new());

pub fn enable(flag: &str) -> Result<(), FrameworkError> {
    let mut guard = FEATURE_FLAGS
        .write()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    guard.insert(flag.to_string(), true);
    Ok(())
}

pub fn is_enabled(flag: &str) -> Result<bool, FrameworkError> {
    let guard = FEATURE_FLAGS
        .read()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    Ok(guard.get(flag).copied().unwrap_or(false))
}
```

À l'intérieur d'un handler, `is_enabled(...)?` s'effondre à travers le
même chemin `FrameworkError → HttpResponse` que toute autre erreur du
framework utilise : le client reçoit un 500 assaini avec
`{"message": "Internal Server Error"}`, le journal structuré capture
le message d'empoisonnement étiqueté, l'id de requête est préservé de
bout en bout, et le reste du processus continue de servir le trafic.
Voir le chapitre [Gestion des erreurs](errors.md) pour le chemin de
conversion complet.

Utilisez ce motif quand :

- L'appelant retourne déjà `Result` (c'est le cas de la plupart des opérations faillibles).
- Un verrou empoisonné représente un échec réel et irrécupérable du sous-système - il n'y a pas de « vérité partielle » raisonnable sur laquelle se replier.
- Vous voulez que les opérateurs *voient* l'empoisonnement dans les journaux la prochaine fois que le sous-système est sollicité. Le message étiqueté est votre indice judiciaire.

Le dispatcher de notifications du framework, le transport mail, le
registre de mailables, les écouteurs d'événements DB, et le registre
de connexions nommées utilisent tous ce motif. Une panique dans l'un
d'eux se manifeste comme un 500 à la prochaine requête qui sollicite
le registre ; tout le reste continue de fonctionner.

### Motif 2 - Récupérer sur place avec `into_inner()`

Quand la signature de l'appelant n'est *pas* faillible (une recherche
`bool`, une vérification de routage sur le hot path, un chemin dont
dépend le cycle de vie de la requête) ou quand l'état partagé est
structurellement sûr à utiliser après une écriture partielle,
récupérez la garde et continuez :

```rust
use std::collections::HashMap;
use std::sync::RwLock;

static ALLOWED_INCLUDES: RwLock<HashMap<&'static str, Vec<&'static str>>> =
    RwLock::new(HashMap::new());

pub fn allows(dto: &str, field: &str) -> bool {
    ALLOWED_INCLUDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(dto)
        .map(|fields| fields.contains(&field))
        .unwrap_or(false)
}

pub fn register(dto: &'static str, fields: &'static [&'static str]) {
    let mut guard = ALLOWED_INCLUDES
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(dto, fields.to_vec());
}
```

`PoisonError::into_inner()` retourne la garde malgré l'empoisonnement.
Les lectures et écritures suivantes se déroulent normalement - le
verrou reste empoisonné pour les requêtes `is_poisoned()`, mais le
flux de données est rétabli.

Le framework utilise ce motif dans `data::registry` (l'allowlist des
include lue à chaque réponse JSON:API), `auth::manager` (la map des
fournisseurs d'authentification nommés), `app::paths` (le cache des
chemins résolus), les fakes de test pour le mail et les événements, et
la map des clés d'environnement chargées dans la config. Chacun de ces
cas est soit un endroit où aucun appelant n'a de `Result` à retourner,
soit un état à ajout seul, structurellement sûr à continuer
d'utiliser.

Utilisez ce motif quand :

- La signature de l'appelant est simple (`bool`, `&str`, un clone d'une valeur stockée) et la faire passer à `Result` forcerait chaque appelant - parfois chaque sous-système du framework - à propager l'erreur.
- L'état partagé peut tolérer une écriture partielle. Les maps et caches à ajout seul en sont la forme typique : le pire cas est une entrée manquante ou périmée, que l'appelant gère déjà (refus par défaut, repli sur le primaire, recalcul).
- Le hot path s'exécute assez souvent pour que retourner une erreur à chaque requête suivante soit, sur le plan opérationnel, pire que de dégrader le service.

## Comment choisir entre les deux

La règle de décision, en une phrase : **si le pire cas d'utilisation
d'un état post-empoisonnement est une mauvaise réponse aux
conséquences réelles, convertissez en erreur ; si c'est une entrée
manquante ou périmée que l'appelant gère déjà, récupérez sur place.**

Déroulons-la :

1. **La signature de l'appelant est-elle `Result<_, E>` ?** Si non, vous devez récupérer sur place - ajouter `Result` à un `bool` est en général un refactor à l'échelle du projet, ce qui n'en vaut pas la peine pour un cas limite d'empoisonnement.
2. **Si une valeur écrite à moitié était observée, l'application prendrait-elle une mauvaise décision aux conséquences réelles ?** Facturer le mauvais client, autoriser un include non autorisé, accorder l'accès au mauvais tenant - c'est « oui, convertir en erreur ». Retourner `false` à « ce nom est-il enregistré ? » et se replier sur le pool primaire - c'est « non, récupérer sur place ».
3. **L'état est-il à ajout seul, ou naturellement idempotent en cas de réenregistrement ?** Si oui, récupérer sur place est sûr. Si une écriture est une transition de machine à états qui dépend de la valeur précédente, préférez convertir en erreur pour ne pas cumuler une corruption.

Dans le doute, convertissez en erreur. Une requête qui retourne 500
est un signal bien visible que vous pouvez corriger ; de mauvaises
réponses silencieuses ne le sont pas.

## Ne recourez jamais à `.lock().unwrap()`

La forme interdite :

```rust
// JAMAIS - une seule panique n'importe où dans le graphe d'appels sous
// cette ligne empoisonne le verrou, et chaque appelant suivant
// transforme l'empoisonnement en une nouvelle panique.
let mut guard = SOMETHING.lock().unwrap();
```

`.expect("…")` fait la même chose avec un message plus agréable. Les
deux convertissent un `Err` de verrou empoisonné en une panique que le
filet `AssertUnwindSafe(...).catch_unwind()` du cycle de vie de la
requête capture et convertit en un 500 - ce filet est une *dernière
ligne de défense*, pas une licence pour esquiver la décision
ci-dessus. Les API publiques du framework et le code applicatif
doivent choisir l'un des deux motifs autorisés.

Les deux exceptions où `.unwrap()` est acceptable sur un verrou
`std::sync` :

- **Une configuration de test qui *veut* vérifier que l'empoisonnement a bien été atteint** - le propre helper d'induction d'empoisonnement de `framework/src/lock.rs` utilise `.unwrap()` à l'intérieur du thread qui panique, délibérément.
- **Le chemin d'erreur d'une opération d'empoisonnement qui a déjà échoué** - une fois à l'intérieur du thread de `poison_rw(...)`, la panique *est* le but recherché.

Si vous n'êtes dans aucun de ces cas, choisissez un motif dans la
section ci-dessus.

## Et si ma fonction retourne `bool` ?

C'est la situation dans laquelle vit `ConnectionRegistry::has`. C'est
une recherche `bool` sur le hot path du routage read-replica de
l'exécuteur, appelée en ligne comme
`if ConnectionRegistry::has("read_replica").await { … }`. L'élargir en
`Result<bool, FrameworkError>` forcerait chaque appelant de
l'exécuteur à propager l'erreur avec `?`, injectant un chemin de code
d'erreur interne dans des décisions de routage qui veulent simplement
un oui/non.

Le motif de récupération sur place gère cela - retournez `false` et
laissez la logique de repli de l'appelant prendre le relais (ici,
l'exécuteur retombe sur le pool primaire, ce qui est de toute façon le
comportement sûr). Pour que les opérateurs voient quand même la
condition, émettez un `tracing::warn!` unique la première fois que
l'empoisonnement est observé :

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;

static REGISTRY: RwLock<HashMap<String, ()>> = RwLock::new(HashMap::new());
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub fn has(name: &str) -> bool {
    match REGISTRY.read() {
        Ok(g) => g.contains_key(name),
        Err(_) => {
            // Sûr en cas de concurrence : seul le premier à observer journalise.
            if !POISON_WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(
                    target: "myapp::registry",
                    "registry lock poisoned - `has({name})` degrading to false",
                );
            }
            false
        }
    }
}
```

La barrière basée sur `swap` compte : l'empoisonnement d'un `RwLock`
est persistant, donc sans cette barrière, chaque appel suivant
redéclencherait l'avertissement et inonderait vos journaux. Avec la
barrière, vous obtenez exactement un avertissement par processus et
par registre, et un getter correspondant retournant `Result` (`get`,
`register`) sur le même registre fera remonter l'empoisonnement la
prochaine fois que quelque chose a *réellement besoin* que la
recherche réussisse. Cela donne aux opérateurs les deux signaux : un
avertissement précoce « quelque chose ne va pas », et un 500 dur au
moment où une requête a vraiment dépendu du registre.

## Ce que le framework protège déjà

Vous n'avez pas à appliquer cette politique à un état quelconque que
le framework possède - c'est déjà en place. Concrètement :

- Le registre de connexions nommées (`ConnectionRegistry::register`, `get`, `has`) convertit l'empoisonnement en `FrameworkError::internal` sur les écritures et les lectures retournant `Result` ; `has` se dégrade en `false` avec la barrière d'avertissement unique.
- Le dispatcher de notifications et son registre de fabriques, le registre de mailables, le transport mail, la capture mémoire du mail, et les écouteurs d'événements DB retournent tous `FrameworkError::internal` en cas d'empoisonnement.
- L'allowlist des include de `data::registry`, la map des fournisseurs de `auth::manager`, `app::paths`, le cache des clés d'environnement chargées, et les fakes de test en mémoire récupèrent tous sur place.

Là où vous croisez ces sous-systèmes via leur API publique
(`Notification::send`, `Mail::send`, `Auth::user`, `DB::connection`,
le chemin de réponse JSON:API), un verrou empoisonné du framework se
manifeste comme un 500 propre - jamais une panique à votre site
d'appel.

## Pourquoi Suprnova diverge

Laravel n'a pas de politique de verrouillage parce qu'il n'a pas
d'état partagé de longue durée. Chaque requête PHP obtient son propre
processus, sa propre mémoire, ses propres copies de chaque singleton.
Il n'y a pas de registre en mémoire à empoisonner, ni de notion de «
prochaine requête » héritant des dégâts de la précédente - le runtime
garantit une ardoise vierge à chaque fois.

Suprnova est construit sur Tokio, qui vous donne exactement l'état
partagé de longue durée que PHP exclut. Des WebSockets peu coûteux,
des caches en mémoire, des pools de connexion que vous n'avez pas à
payer pour reconstruire - tout cela a besoin de registres globaux au
processus qui survivent à n'importe quelle requête isolée. Cette
capacité est tout l'intérêt de passer à Rust pour ce genre
d'application (voir l'[introduction](introduction.md) pour la
motivation complète du framework). Le prix à payer pour l'avoir, c'est
que vous devez désormais réfléchir à ce qui se passe quand un thread
qui panique laisse un état partagé dans une condition gardée - parce
qu'il y a *bel et bien* un état partagé à laisser derrière soi.

La politique à deux motifs est la plus petite réponse qui conserve la
capacité tout en supprimant le coût. Récupérez sur place là où l'état
est sûr à continuer d'utiliser ; convertissez en erreur là où vous
préférez un 500 propre à une mauvaise réponse. Les deux options
laissent le reste du processus continuer à servir le trafic. Aucune
des deux ne laisse un `unwrap` paniqué en attente de faire tomber le
sous-système au-dessus.

C'est la même forme que la [décision fail-open contre
fail-closed](rate-limiting.md) que le framework applique aux backends
de cache et de limitation de débit injoignables : un choix de
politique explicite au site d'appel, pas une valeur par défaut.
L'async partout vous donne un état de longue durée ; le framework vous
donne le plan de jeu pour le garder honnête.

## Suivant

- [Gestion des erreurs](errors.md) - comment `FrameworkError::internal` devient le 500 assaini que le client reçoit, avec le message d'empoisonnement étiqueté préservé dans votre journal structuré.
- [Conteneur de service](container.md) - où vivent réellement les registres globaux au processus que cette politique protège, et pourquoi le cloisonnement task-local/thread-local empêche les tests d'hériter des liaisons les uns des autres.
- [Cycle de vie des requêtes](lifecycle.md) - la limite de panique (`execute_chain_safely`) qui capture l'`unwrap` de *dernier recours* et le convertit en un 500, pour que vous compreniez exactement ce que fait le filet de sécurité et pourquoi ce n'est pas une excuse pour esquiver la politique ci-dessus.
- [Limitation de débit](rate-limiting.md) - l'histoire parallèle de `BackendErrorPolicy` pour les backends qui peuvent être *injoignables* plutôt qu'empoisonnés ; même principe de choix explicite, mode de défaillance différent.
- [Tests](testing.md) - comment `TestContainer::fake` et la couche de conteneur thread-local empêchent les tests parallèles de polluer les registres les uns des autres, ce qui est le complément, au moment des tests, de l'histoire de la gestion de l'empoisonnement.
