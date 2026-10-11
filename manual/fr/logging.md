# Journalisation

Suprnova journalise via [`tracing`](https://docs.rs/tracing) - chaque
ligne de journal est un événement structuré doté de champs, pas une
chaîne formatée. Un subscriber est installé à l'amorçage : il lit
`LOG_LEVEL` et `LOG_FORMAT` dans l'environnement, émet une sortie pretty
multiligne en dev et un objet JSON par ligne en production, et propage un
id par requête dans chaque événement qu'émet un handler.

Ce chapitre couvre la surface de journalisation elle-même : le
subscriber, les formats, les niveaux, et la corrélation par id de requête
qui rend un journal de production consultable. Pour la passerelle
OpenTelemetry et la journalisation des requêtes SQL, voir
[Observabilité](observability.md) ; pour le sac `Context` de la requête
que les émetteurs peuvent lire à côté de l'id, voir
[Contexte](context.md).

## Ce qui est journalisé, et où

Deux sorties par défaut :

| Où | Format | Quand |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - multiligne, coloré, lisible par un humain | dev (`APP_ENV` vaut `local`, `dev`, `testing`, …) |
| `stdout` | `LogFormat::Json` - un objet JSON par ligne | production (`APP_ENV=production` / `prod`) |

La valeur par défaut dev/prod est calculée à partir d'`APP_ENV` via
`Environment::detect()`. Remplacez-la par `LOG_FORMAT=pretty` ou
`LOG_FORMAT=json` pour en forcer une explicitement.

```env
# .env (dev)
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # facultatif ; c'est la valeur par défaut en dev

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # facultatif ; c'est la valeur par défaut en prod
```

Le framework n'écrit que sur `stdout`. En production, pointez-y votre
runtime de conteneur, le journal systemd ou votre agrégateur de journaux
(`docker logs`, `kubectl logs`, `journalctl -u my-app`, un agent
Loki/Vector, etc.). Il n'y a pas d'appender de fichier avec rotation -
laissez la plateforme s'approprier la persistance des journaux.

## Émettre des événements

Utilisez les macros de `tracing` dans les handlers, les jobs, le
middleware, n'importe où :

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

Chaque champ devient une clé de premier niveau dans la sortie JSON et une
paire `field=value` colorée dans la sortie pretty. Préférez les champs à
l'interpolation - ils sont consultables dans les journaux JSON et le
formateur gère un rendu conscient des types.

Pour envelopper une fonction dans un span et estampiller de champs
partagés chaque événement qui s'y produit, utilisez `#[instrument]` :

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // porte automatiquement user_id venu du span
    // … requêtes …
}
```

Le même `#[instrument]` devient un span OpenTelemetry quand la feature
`otel` est activée - voir
[Observabilité](observability.md#opentelemetry).

## Niveaux de journalisation

`LOG_LEVEL` est une [directive env-filter de
`tracing-subscriber`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
pas un niveau unique. La grammaire est faite de paires `target=level`
séparées par des virgules, où une valeur nue fixe la valeur par défaut :

```env
LOG_LEVEL=info                                  # tout à partir de info
LOG_LEVEL=debug                                 # tout à partir de debug
LOG_LEVEL=info,sqlx=warn                        # info par défaut, sqlx plus discret
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # warn par défaut, deux cibles verbeuses
```

Les cibles sont généralement la crate émettrice ou le chemin de module
(`suprnova::queue`, `hyper::server`, `my_app::services::checkout`).
Trouvez une cible en lisant la ligne de journal JSON - le champ `target`
présent sur chaque événement est sa clé de filtrage.

Les niveaux, par verbosité croissante : `error` < `warn` < `info` (par
défaut) < `debug` < `trace`. La réponse d'erreur qui part sur le réseau
est toujours assainie en `{"message": "Internal Server Error"}` quel que
soit le niveau - le détail ne va que dans le journal structuré.

### Les directives invalides ne font pas planter l'amorçage

Un `LOG_LEVEL` mal formé (par ex. `LOG_LEVEL=app=notalevel`) se replie
sur `"info"` et écrit un avertissement d'une ligne sur `stderr` :

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

C'est `stderr` plutôt que `tracing::warn!` parce que le subscriber n'a
pas encore été installé - un `warn!` serait silencieusement abandonné.
Corrigez la directive et l'avertissement disparaît.

## Sortie pretty ou JSON

Le même `info!(user_id = 42, "saved")` se rend différemment selon le
format.

**Pretty (dev) :**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON (prod) :**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

La forme JSON est celle que les agrégateurs de production (Datadog, Loki,
Honeycomb, CloudWatch, …) analysent sans configuration.
`span.request_id` est la clé de corrélation - voir ci-dessous.

## Corrélation par id de requête

Chaque requête HTTP reçoit un `RequestId` de `RequestIdMiddleware`, le
middleware le plus externe de chaque chaîne. Cet id est :

- **Réutilisé** depuis un en-tête entrant `X-Request-Id` sûr
  (alphanumériques plus `- _ . :`, jusqu'à 128 octets), ou **frappé à
  neuf** sous forme d'UUID v4 s'il est absent ou non sûr.
- **Renvoyé en écho** sur la réponse sous `X-Request-Id` (aussi bien pour
  les variantes 2xx que 5xx).
- **Cantonné** dans un span `tracing` nommé `request`, si bien que chaque
  événement provenant d'un middleware, d'un handler ou d'une bibliothèque
  en aval porte automatiquement `request_id` dans son tableau `spans`.
- **Déposé** dans le sac `Context` de la requête sous `_request_id`, si
  bien que les émetteurs qui veulent la chaîne nue (jobs, charges utiles
  de diffusion, rapports d'erreur) peuvent le lire par son nom.

Lisez-le dans le code avec `current_request_id()` :

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` retourne `Option<RequestId>` parce que le travail
en arrière-plan (jobs, tâches planifiées, tests qui n'ont pas installé le
middleware) s'exécute en dehors de toute portée de requête.

### Tâches en arrière-plan : faire le spawn avec l'id

`tokio::spawn` démarre une tâche neuve avec des task-locals vides - un
handler qui lance en spawn un travail à effets de bord perd
`current_request_id()`, et ses événements de journal deviennent
orphelins. Utilisez plutôt `spawn_with_request_id` :

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // Cette tâche observe toujours current_request_id(). Ses
        // événements de journal portent le même request_id que le handler.
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

Le helper propage à la fois le task-local `RequestId` et le
`tracing::Span` courant, si bien que les événements de la future lancée
s'imbriquent sous le même span `request` dans le journal. En dehors d'une
portée de requête active, il retombe sur un simple `tokio::spawn` -
utilisable sans condition, en toute sécurité.

Seuls l'id de requête et le span tracing suivent la tâche - le sac
`Context` de la requête, délibérément, non : le travail en arrière-plan ne
sert pas la requête HTTP d'origine.

## Le subscriber

Le framework installe un subscriber `tracing` global à l'amorçage, depuis
`Server::run()`. Vous ne l'appelez presque jamais vous-même ; il est
documenté parce que les tests, les intégrateurs et les points d'entrée
inhabituels en ont parfois besoin.

```rust
use suprnova::{LogConfig, init_subscriber};

// Lire LOG_LEVEL / LOG_FORMAT depuis l'environnement :
init_subscriber(LogConfig::from_env());

// Ou par programme :
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` est **idempotente**. Un second appel laisse en place le
subscriber existant et émet un `tracing::warn!` afin qu'un opérateur
puisse voir que la nouvelle `LogConfig` n'a pas été appliquée. C'est ce
qui évite aux tests qui appellent chacun `init_subscriber` de se
concurrencer - le premier gagne, les autres sont sans effet.

Pour la variante consciente d'OTel (la même `LogConfig`, plus l'export de
tracing distribué), utilisez
[`init_telemetry`](observability.md#opentelemetry).

### Les daemons

`queue:work`, `schedule:work`, `schedule:run` et `workflow:work` sont des
sous-commandes du binaire de votre application et ne s'amorcent pas via
`Server::run()` : elles installent donc leur propre subscriber au
démarrage. Elles lisent les mêmes `LOG_LEVEL` et `LOG_FORMAT` que le
serveur, et vous n'appelez rien vous-même :

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …ou, dans un conteneur, contre le binaire construit :
LOG_LEVEL=info my-app queue:work
```

Avant la 0.9.1, ce chemin n'installait rien du tout. Chaque ligne
`tracing::` qu'émettent les daemons n'allait nulle part et `LOG_LEVEL`
était inerte pour elles, ce qui, dans un conteneur, ne laissait que la
bannière de démarrage en guise de sortie - un worker qui met des jobs en
lettre morte, un planificateur qui saute un tick dont il a perdu
l'élection, et un verrou qu'il n'a pas pu libérer avaient tous exactement
l'air d'un processus inactif. Si vous exécutez une version épinglée
antérieure à la 0.9.1 et que vous vous demandez pourquoi un worker ne dit
rien, c'est pour cela, et le correctif est la mise à niveau plutôt qu'un
changement de configuration.

L'essentiel de ce qu'un worker a à dire, il le dit en `warn!` et en
`error!` - un job qui épuise ses tentatives, une lettre morte qu'il n'a
pas pu persister, un verrou qu'il n'a pas pu libérer - si bien que le
niveau `info` par défaut suffit à voir les ennuis. Descendez à `debug`
quand vous avez aussi besoin des décisions plus discrètes.

## Tests

Les tests n'ont pas besoin d'installer un subscriber - l'attribut
`#[suprnova_test]` et `TestContainer::fake` mettent en place assez de
machinerie pour que les événements des handlers circulent. Si vous voulez
faire des assertions sur la sortie des journaux, capturez-la via le
[`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html)
de `tracing-subscriber` ou via une couche personnalisée ; le framework ne
fournit délibérément pas de faux « capturer tous les journaux de ce
test », parce que les motifs de test standard de `tracing-subscriber`
fonctionnent proprement.

## Pourquoi Suprnova diverge

Laravel utilise [Monolog](https://github.com/Seldaek/monolog) - des
chaînes de message avec des tableaux de contexte facultatifs, des canaux
de journalisation, et des handlers par canal (fichier, syslog, Slack, …).
Le modèle une-requête-par-processus de PHP fait qu'un unique logger
statique global est sûr : chaque requête obtient son propre processus et
son propre contexte.

Le modèle de processus de Rust est l'opposé - un processus sert de
nombreuses requêtes simultanées sur plusieurs threads. Un formateur de
chaînes global créerait une condition de course sur le contexte et
exigerait une plomberie explicite de `request_id` à travers chaque
site d'appel. `tracing` résout les deux avec des champs structurés et des
spans task-local : aucune plomberie, les champs restent typés, et la
corrélation est automatique parce que le span de requête est dans la
portée de chaque événement qu'émet la chaîne.

La sortie exclusivement sur `stdout` est elle aussi intentionnelle. Dans
les déploiements conteneurisés (la seule façon dont Suprnova est livré),
c'est le runtime, et non l'application, qui s'approprie la persistance
des journaux - la rotation des fichiers, la rétention et l'expédition
appartiennent toutes à la plateforme.

## Suivant

- [Observabilité](observability.md) - OpenTelemetry, le journal des
  requêtes SQL, toute la surface offerte aux opérateurs
- [Contexte](context.md) - le sac par requête où vivent `_request_id` et
  les autres champs contextuels
- [Gestion des erreurs](errors.md) - comment la limite de panique du
  framework et son chemin 5xx émettent leurs propres événements
  structurés
- [Variables d'environnement](env-vars.md) - la référence de `LOG_LEVEL`
  et `LOG_FORMAT`
