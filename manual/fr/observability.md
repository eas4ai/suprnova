# Observabilité

Trois couches de signal visibles par l'opérateur sont livrées dans le
framework : les journaux structurés (toujours actifs), la corrélation
par id de requête (toujours active, se propage dans les tâches
spawnées), et une passerelle OpenTelemetry en opt-in qui transforme
chaque span `tracing` en un span OTel exporté. Le même
`#[tracing::instrument]` que vous écririez pour des journaux locaux
devient un span de trace distribuée quand la feature OTel est
activée - pas de deuxième API d'instrumentation.

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // ... exécute l'app ...

    // Vide la télémétrie mise en tampon avant de sortir. Les processeurs
    // par lots d'OTel gardent les spans/métriques/journaux en mémoire ;
    // abandonner la garde sans `shutdown` perd tout ce qui n'a pas encore
    // été exporté.
    guard.shutdown().await;
}
```

Le `Server` d'une app scaffoldée appelle déjà `init_telemetry` pour
vous et vide la garde sur le signal d'arrêt - vous ne le câblez à la
main que lorsque vous embarquez Suprnova dans votre propre runtime.

## Les trois couches

| Couche | Toujours active | Ce qu'elle vous apporte |
|---|---|---|
| Journalisation structurée (`tracing`) | Oui | Journaux stdout au format `pretty` (dev) ou `json` (production), conscient de l'environnement |
| Corrélation par id de requête | Oui | Id par requête cantonné via un `tokio::task_local!`, renvoyé en écho sur `X-Request-Id`, propagé dans les tâches `spawn_with_request_id` |
| Export OpenTelemetry | feature `otel` + point de terminaison du collecteur | Export OTLP HTTP/proto des traces, métriques et journaux ; propagation `traceparent` W3C dans les deux sens |

La couche OTel est **en opt-in à la compilation**, si bien que les
builds par défaut ne portent aucune dépendance OpenTelemetry et que la
façade [`Metrics`](#métriques) se compile en no-ops inertes. Avec la
feature désactivée, la « trace » et l'« export de métriques »
deviennent silencieusement des no-ops - vos journaux continuent de
fonctionner.

### Pourquoi Suprnova diverge

L'histoire de l'observabilité de Laravel se scinde entre les
événements internes au framework (`QueryExecuted`, `MessageSent`,
`JobProcessed`) et les préoccupations de runtime déléguées à des
extensions PHP (OpenTelemetry, Sentry, New Relic) branchées au niveau
de FPM. La surface d'événements est riche ; la surface de runtime se
résume à « installez l'extension dont votre fournisseur d'APM a
besoin ».

Suprnova est un unique processus asynchrone, donc il possède les deux
moitiés. La surface d'événements est à parité (même forme
`QueryExecuted`/`NotificationSent`/`ErrorOccurred`), et la surface de
runtime est une passerelle `tracing` → OpenTelemetry à l'intérieur du
framework. Vous n'installez pas d'extension ; vous basculez un flag de
fonctionnalité et les mêmes spans que vous émettez déjà deviennent
exportés vers OTel.

## Journalisation structurée

`LogConfig::from_env()` lit deux variables d'env :

| Var | Défaut | Notes |
|---|---|---|
| `LOG_LEVEL` | `"info"` | Syntaxe env-filter de `tracing-subscriber` (par ex. `"debug,sqlx=warn,hyper=warn"`) |
| `LOG_FORMAT` | conscient de l'environnement | `"json"` en production, `"pretty"` partout ailleurs ; une valeur explicite l'emporte toujours |

La valeur par défaut du format est détectée à partir d'`APP_ENV` via
`Environment::detect()` : un déploiement de production obtient par
défaut une sortie d'un objet JSON par ligne pour les agrégateurs de
journaux, les exécutions locales/dev obtiennent une sortie multiligne
lisible par un humain. Un `LOG_FORMAT=pretty` explicite remplacera la
valeur par défaut de production si vous voulez du stdout brut en
production.

```bash
# Dev local - les redéfinitions explicites l'emportent
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# Production - APP_ENV=production bascule la valeur par défaut du format vers json
APP_ENV=production LOG_LEVEL=info cargo run --release
```

Une directive `LOG_LEVEL` mal formée ne fait pas planter l'amorçage -
elle se replie sur `"info"` et imprime un avertissement d'une ligne
sur stderr afin que la mauvaise configuration soit visible pour
l'opérateur.

### Contexte de span sur chaque ligne

Chaque requête HTTP routée s'exécute à l'intérieur d'un span `request`
créé par le middleware le plus extérieur du framework. Le span porte
trois champs - `request_id`, `method`, `path` - et le formateur JSON
les imbrique sous `span` sur chaque événement émis à l'intérieur de la
requête. Le code de votre application n'a pas besoin de lire ou
d'enregistrer l'id sur chaque ligne ; le span le porte implicitement :

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // La ligne JSON porte span.request_id / span.method / span.path
    // sans que le site d'appel n'ait à faire transiter quoi que ce soit.
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## Corrélation par id de requête

Chaque requête reçoit un id UUID v4 en minuscules de 36 caractères,
cantonné via un `tokio::task_local!`. Le middleware réutilise un
`X-Request-Id` entrant quand la valeur de l'en-tête passe un contrôle
de sûreté strict (alphanumérique ASCII plus `-_.:`, 128 octets max) ;
tout ce qui sort de ce jeu de caractères est rejeté et remplacé par un
UUID neuf, pour qu'un attaquant ne puisse pas injecter de caractères
de contrôle dans la sortie du journal ou faire gonfler les pipelines
en aval.

Le même id est renvoyé en écho sur **chaque** réponse - succès, erreur
et récupération de panique - sous l'en-tête `X-Request-Id`, si bien
qu'un frontend ou un service amont peut l'inclure dans un rapport de
bug et que les opérateurs peuvent le grep dans le journal structuré.

### Lire l'id

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // À l'intérieur d'une requête, l'id est toujours présent.
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // Travail en arrière-plan spawné depuis un handler. `tokio::spawn`
    // démarre une tâche avec des task-locals vides - la future spawnée
    // perdrait l'id de requête sans aide. `spawn_with_request_id` capture
    // l'id de l'appelant et le recantonne pour la future spawnée, et
    // attache le span `tracing` courant afin que les événements de la
    // tâche héritent de `request_id` de la même façon que les événements
    // dans la requête.
    spawn_with_request_id(async move {
        // Cette ligne de journal porte l'id de la requête d'origine.
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` retourne `None` hors d'une requête - les jobs
en arrière-plan, les tâches planifiées et les tests sans le middleware
ne voient aucun id, et le helper n'en invente pas. `spawn_with_request_id`
hors d'une portée de requête est exactement `tokio::spawn` ; rien de
magique ne se produit.

### Où l'id est aussi disponible

| Surface | Comment |
|---|---|
| Événements `tracing` | `span.request_id` sur chaque ligne à l'intérieur de la requête |
| En-tête de réponse | `X-Request-Id` sur les réponses de succès, d'erreur et récupérées d'une panique |
| Sac `Context` | `Context::get("_request_id")` - lisible depuis les observateurs, les écouteurs, les jobs qui consultent `Context` |
| Tâches spawnées | `current_request_id()` après `spawn_with_request_id` |

## Événements intégrés pour l'observabilité

Le framework distribue des événements typés aux points où un
opérateur veut habituellement instrumenter. Chacun est un
`suprnova::Event` que vous pouvez écouter (`listen`) via
`EventFacade::listen::<E, _>(...)` et expédier vers Sentry, Datadog,
Slack, ou votre pipeline de métriques. Tous passent par
`dispatch_best_effort`, si bien qu'un écouteur défaillant ne casse pas
la requête qui l'a déclenché.

| Événement | Quand il se déclenche | Porte |
|---|---|---|
| `ErrorOccurred` | Toute conversion `FrameworkError` → 5xx (récupération de panique comprise) | contexte d'erreur + id de requête |
| `QueryExecuted` | Chaque requête routée à travers les helpers d'exécuteur instrumentés | sql, bindings, durée, connexion, classification lecture/écriture, résultat |
| `ConnectionEstablished` | `DbConnection::connect` a réussi | nom de connexion |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | `DB::transaction` en forme closure + handles manuels | nom de connexion |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | Avant/après/erreur par canal de `Notification::send` | notification + canal + destinataire |

`ErrorOccurred` est le point d'accroche pour expédier les exceptions
5xx ; `QueryExecuted` est le point d'accroche pour les alertes de
requête lente ; le trio de notification est le point d'accroche pour
les tableaux de bord de livraison. Voir [Événements](events.md) pour
l'API d'écouteur et [Cycle de vie](lifecycle.md) pour l'endroit du
chemin de requête où chaque événement se déclenche.

### Observation directe des requêtes de base de données

`DB::listen` est un second point d'accroche, synchrone, taillé
spécifiquement pour `QueryExecuted`. Il se déclenche en ligne à
l'intérieur de l'exécuteur, si bien qu'un écouteur lent ralentit la
requête - gardez-le léger. Le chemin du dispatcher
(`EventFacade::listen::<QueryExecuted, _>`) exécute tout le monde en
best-effort et tolère les erreurs ; préférez-le pour tout ce qui peut
échouer.

```rust
use suprnova::DB;

// Dans bootstrap.rs :
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

Un écouteur qui émet lui-même une requête de base de données ne
redéclenchera **pas** `QueryExecuted` pour l'appel imbriqué - un garde-
fou de réentrance task-local empêche la boucle « écouteur log-vers-DB
→ émet un événement → log-vers-DB → ... ».

### Capturer un journal de requêtes pour les tests / le débogage

Pour des assertions de test ou un débogage ponctuel du type
« qu'est-ce qui s'est exécuté pendant ce bloc ? » :

```rust
use suprnova::DB;

DB::enable_query_log()?;
// ... exécutez le code que vous voulez inspecter ...
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

Le tampon est **non borné** - chaque requête capturée le fait
grossir. Utilisez-le pour les tests et l'investigation ponctuelle,
videz-le périodiquement si vous le laissez actif en production.

## Traçage distribué (OTel)

Ajoutez la feature `otel` pour l'opt-in :

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

Configurez via les variables d'environnement OTel standard :

```bash
# Minimum : où réside le collecteur.
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # par défaut "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # par défaut la version de votre crate
```

La télémétrie est **activée** seulement quand
`OTEL_EXPORTER_OTLP_ENDPOINT` est défini **et** que l'interrupteur
d'arrêt `OTEL_SDK_DISABLED` n'est pas actif. Sans point de terminaison,
la couche de journalisation s'exécute seule, et la garde retournée ne
détient aucun fournisseur, si bien que l'abandonner sans `shutdown()`
est silencieux (pas d'avertissement parasite « la télémétrie mise en
tampon pourrait être perdue » à chaque processus de test).

### Le contexte de trace se rejoint automatiquement

**Entrant.** Quand une requête arrive en portant un en-tête W3C
[`traceparent`](https://www.w3.org/TR/trace-context/) - c'est-à-dire
qu'elle a été faite par un autre service tracé - le middleware extrait
ce contexte et rattache le span de la requête comme enfant du span de
l'appelant. Votre span serveur apparaît comme un enfant dans la *même*
trace distribuée, pas comme une nouvelle racine. Une requête sans
`traceparent` (un accès direct depuis un navigateur) reste un span
racine propre.

**Sortant.** Le client HTTP du framework ([`Http`](http-client.md))
injecte le contexte de trace actif comme `traceparent` sur chaque
appel sortant, si bien que le service en aval continue la même trace.

Ensemble : `service amont → votre handler → service en aval` forme une
seule trace connectée, sans plomberie de span manuelle dans vos
handlers.

**Statut d'erreur.** Quand un handler retourne un 5xx, le span de la
requête est marqué en erreur pour que le backend OTel affiche
`Status::Error`. (Une *panique* de handler est capturée et transformée
en un 500 avec un journal de niveau erreur et un événement
`ErrorOccurred`, mais le statut du span OTel n'est pas posé sur ce
chemin - la panique déroule la future du span avant que le marqueur ne
s'exécute.)

### Ajouter vos propres spans

Comme la passerelle transforme chaque span `tracing` en un span OTel,
vous instrumentez avec du `tracing` ordinaire - aucune API spécifique
à OTel dans votre code :

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // Ce span s'imbrique automatiquement sous le span de la requête, et
    // s'exporte vers votre collecteur quand la feature `otel` est activée.
    Ok(())
}
```

### Variables d'environnement que Suprnova lit

| Var | Effet |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | URL de base du collecteur. Non défini → télémétrie désactivée. |
| `OTEL_SERVICE_NAME` | Attribut de ressource `service.name` (défaut `"suprnova"`). |
| `OTEL_SERVICE_VERSION` | Attribut de ressource `service.version` (défaut : version de la crate). |
| `OTEL_SDK_DISABLED` | Interrupteur d'arrêt. `true` ou `1`, insensible à la casse, désactive l'export même avec un point de terminaison défini. |

Le reste des réglages OTLP standard est lu par le SDK lui-même,
configurez-les donc de la façon normale :

| Var | Lu par |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | l'exportateur (auth du collecteur, par ex. `Authorization=Bearer ...`) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | l'exportateur (`http/protobuf`, etc.) |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | l'exportateur |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | l'exportateur |

Les redéfinitions de point de terminaison par signal
(`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `_METRICS_ENDPOINT`,
`_LOGS_ENDPOINT`) sont actuellement masquées par le point de
terminaison de base - les trois signaux partent tous vers
`OTEL_EXPORTER_OTLP_ENDPOINT`. Si vous avez besoin de répartir les
signaux vers des collecteurs différents, exécutez un collecteur local
qui les route.

## Métriques

`Metrics` est la façade pour les compteurs, les histogrammes et les
jauges. Les handles sont bon marché à cloner et résolvent le meter
global à chaque construction :

```rust
use suprnova::telemetry::Metrics;

// Compteur - monotone.
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // +1 avec un label

// Histogramme - distributions (latence, tailles).
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// Jauge - valeur à un instant donné.
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

Sans la feature `otel`, chaque appel ci-dessus est un no-op à
allocation nulle - laissez l'instrumentation dans les hot paths et ne
payez rien dans les builds par défaut.

Les handles de métrique se lient à quel que soit le fournisseur de
meter actif quand l'instrument sous-jacent est résolu pour la première
fois. Créez les handles **après** que `init_telemetry` s'est exécuté
(ou paresseusement au premier usage) - un handle construit avant
l'initialisation se résout contre le fournisseur no-op et reste
inerte. Le motif idiomatique est un handle `once_cell` / `LazyLock`
résolu au premier envoi, bien après l'amorçage.

Les valeurs d'attribut sont typées comme chaînes
(`&[(&'static str, &str)]`). Les attributs numériques et booléens sont
une amélioration prévue ; formatez-les en chaînes au site d'appel pour
l'instant.

Nommage : stable, ASCII, délimité par des points (par ex.
`"http.requests.total"`, `"http.request.duration"`). Les conventions
sémantiques OTel standard vivent dans
`opentelemetry-semantic-conventions::metric::*`.

## Le contrat d'arrêt

`init_telemetry` retourne un `TelemetryGuard` qui détient les handles
de fournisseur du SDK. Les processeurs par lots d'OTel mettent en
tampon les spans / métriques / journaux en mémoire et les vident de
façon asynchrone, donc vous devez faire `guard.shutdown().await` avant
que le processus ne sorte, sinon vous perdez tout ce qui est encore en
tampon.

- Appeler `shutdown()` vide et est sûr à appeler une fois (il prend
  `self`).
- Abandonner la garde **sans** `shutdown()` journalise un
  avertissement - mais seulement quand la garde détient réellement des
  fournisseurs. Une exécution avec télémétrie désactivée (pas de point
  de terminaison, ou `OTEL_SDK_DISABLED`, ou un build sans `otel`)
  renvoie une garde sans fournisseur dont l'abandon est silencieux,
  pour que les exécutions de dev et de test sans collecteur ne soient
  pas spammées.

## Résumé

| Tâche | API |
|---|---|
| Activer OTel | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| Initialiser | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| Vider à la sortie | `guard.shutdown().await` |
| Désactiver à l'exécution | `OTEL_SDK_DISABLED=true` |
| Span personnalisé | `#[tracing::instrument]` (auto-relié à OTel) |
| Compteur / histogramme / jauge | `Metrics::counter/histogram/gauge(name)` |
| Jonction de trace distribuée | Automatique - `traceparent` entrant extrait, sortant injecté |
| Lire l'id de requête courant | `current_request_id()` |
| Propager l'id dans un spawn | `spawn_with_request_id(future)` |
| Observateur de requête synchrone | `DB::listen(|q| { ... })` |
| Observateur de requête best-effort | `EventFacade::listen::<QueryExecuted, _>(...)` |
| Capturer les requêtes pour les tests | `DB::enable_query_log()` → `DB::get_query_log()` |

## Suivant

- [Événements](events.md) - API d'écouteur, modes de distribution, `EventFacade::fake()` pour les tests
- [Cycle de vie](lifecycle.md) - où dans le chemin de requête chaque événement se déclenche et où le span de requête est construit
- [Gestion des erreurs](errors.md) - `ErrorOccurred`, `HttpError`, corps 5xx assainis
- [Base de données](database.md) - `QueryExecuted`, `DB::transaction`, les helpers d'exécuteur qui déclenchent les événements
- [Client HTTP](http-client.md) - injection sortante de `traceparent` qui referme la boucle de trace distribuée
