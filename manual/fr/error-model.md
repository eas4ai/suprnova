# Modèle d'erreur

Ce chapitre présente le modèle qui sous-tend la gestion des erreurs de Suprnova : les types, le contrat de conversion et les garanties de sécurité que le framework vous offre gratuitement. Pour les motifs quotidiens de handler (`?`, retour d'erreurs, construction d'erreurs de domaine personnalisées), voir [Gestion des erreurs](errors.md) ; ce chapitre explique *pourquoi* ces motifs fonctionnent comme ils le font.

Si vous ne retenez qu'une seule chose de cette page : **les erreurs dans Suprnova sont des valeurs, pas des exceptions**. Chaque erreur devient finalement une `HttpResponse` via une conversion unique et totale. Il n'y a pas de handler d'exceptions global parce qu'il n'y a pas d'exception globale.

## La forme

Le modèle d'erreur de Suprnova comporte cinq éléments :

| Type | Rôle |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | Le contrat que satisfait chaque handler - les deux branches sont déjà des réponses |
| `FrameworkError` | L'enum d'erreur canonique du framework ; chaque chemin d'erreur interne en produit une |
| `AppError` | Erreur de domaine ad hoc pour un usage en ligne sans type dédié |
| `HttpError` (trait) | Ce que vos propres erreurs de domaine typées implémentent pour obtenir un statut + un message |
| `ValidationErrors` | Le sac d'erreurs de forme Laravel/Inertia pour les échecs par champ |

`FrameworkError` et les types d'erreur concrets du framework utilisent des impls `From`. Un `HttpError` écrit à la main doit être mappé avec `FrameworkError::from_http_error` avant `?` ; il n'existe pas d'implémentation globale `From<T: HttpError>`. La chaîne de middleware convertit les erreurs à la limite de la requête, et le handler de panique convertit un unwind. Les erreurs ordinaires partagent alors le même moteur de rendu du corps et la même règle d'assainissement des 5xx.

## `Response` est `Result<HttpResponse, HttpResponse>`

Chaque handler retourne ceci :

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

Les deux branches portent le même type de charge utile, ce qui est tout l'intérêt. Quand la chaîne de middleware termine l'exécution de votre handler, elle réduit le résultat en une seule ligne :

```rust
result.unwrap_or_else(|e| e)
```

Le framework n'a pas besoin de savoir si votre handler a « réussi » ou « échoué » - les deux branches sont déjà des réponses HTTP rendues. La distinction existe uniquement pour que `?` puisse faire son travail :

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` court-circuite sur Err. Chaque conversion ci-dessous produit une
    // HttpResponse via une impl From - la chaîne réduit les deux branches.
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 404 if missing
    Ok(json_response!({ "user": user }))
}
```

Ce contrat unique - chaque chemin d'erreur produit une `HttpResponse` via `From` - est le cœur du modèle. Tout le reste de ce chapitre décrit ce que font réellement les différents impls `From`.

### Pourquoi Suprnova diverge

Laravel lève des exceptions et les achemine à travers une classe `Handler` globale enregistrée dans `app/Exceptions/Handler.php`. Le framework capture tout, demande au handler « que dois-je rendre ? », et émet la réponse. Le modèle d'exceptions à déroulement de PHP rend cela naturel.

Rust n'a pas d'exceptions à déroulement dans le code utilisateur. L'équivalent chez Suprnova est l'impl `From<FrameworkError> for HttpResponse` plus l'événement `ErrorOccurred`. La conversion est le moteur de rendu ; l'événement est l'endroit où vous branchez l'observabilité (Sentry, PagerDuty, expéditeurs structurés). Vous n'enregistrez pas de classe handler - la conversion est une fonction et l'écoute de `ErrorOccurred` est le point d'extension. Même surface, machinerie différente.

## `FrameworkError` - l'enum canonique

Chaque chemin d'erreur à l'intérieur du framework - extracteurs, liaison de route, le conteneur, la validation, la couche base de données, le stockage - produit une `FrameworkError`. C'est un enum à seize variantes, chacune étiquetée avec son statut HTTP :

```rust
pub enum FrameworkError {
    ServiceNotFound { type_name: &'static str },        // 500
    ParamError { param_name: String },                   // 400
    ValidationError { field: String, message: String },  // 422
    Database(String),                                    // 500
    Internal { message: String },                        // 500
    Domain { message: String, status_code: u16 },        // *
    Validation(ValidationErrors),                        // 422
    Unauthorized,                                        // 403
    ModelNotFound { model_name: String },                // 404
    ParamParse { param: String, expected_type: &'static str }, // 400
    UnsupportedMediaType,                                // 415
    PrecognitionSuccess,                                 // 204
    PrecognitionFailure(ValidationErrors),               // 422
    AlreadyReported,                                     // CLI uniquement
    RateLimited { retry_after: Option<Duration>, message: String }, // 429
    External { message: String, source: Arc<dyn Error + Send + Sync> }, // 500
}
```

Vous faites rarement un `match` sur la variante. Vous en construisez une via un constructeur de commodité et laissez `?` faire le reste :

```rust
use suprnova::FrameworkError;

// Tous ces constructeurs produisent une FrameworkError avec le bon statut :
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

Il n'existe pas de constructeurs `unauthorized()` ou `forbidden()` sur `FrameworkError` - `Unauthorized` est une variante fixe portant le message Laravel « This action is unauthorized. » au statut 403, et les cas 401 passent par `AppError::unauthorized` (section suivante). Remarque : la variante s'appelle `Unauthorized` mais le statut est 403 car elle modélise le refus d'autorisation de Laravel, pas l'authentification HTTP.

### Conversion automatique

`FrameworkError` implémente `From<sea_orm::DbErr>` et `From<opendal::Error>` afin que les erreurs de base de données et de stockage transitent par `?` sans enveloppe :

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // Les deux appels `?` ici se convertissent automatiquement en FrameworkError :
    // - DB::get retourne Result<_, FrameworkError>
    // - insert retourne Result<_, DbErr>, qui a From<DbErr> pour FrameworkError
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Si votre code retourne `Result<_, FrameworkError>`, chaque erreur courante que produisent vos dépendances parle déjà le bon langage. Le `?` du contrôleur ne fait rien d'autre que convertir un type d'erreur en un autre.

### Ajouter du contexte

Quand vous devez relancer une erreur avec le contexte de l'opération, utilisez `.context()` :

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

Le message devient `"creating new user: <original>"`. La variante est préservée là où cela compte - `Validation`, `ValidationError`, `PrecognitionFailure`, `PrecognitionSuccess`, `Unauthorized`, `ModelNotFound`, `ParamParse`, `UnsupportedMediaType`, `AlreadyReported`, `RateLimited` et `External` conservent leur structure afin que le moteur de rendu de réponse émette toujours la bonne forme (et, pour `External`, pour que la source enveloppée survive). Les variantes qui portent simplement un message (`Internal`, `Database`, `Domain`) s'aplatissent en une `Domain` avec le message préfixé.

### Envelopper une erreur externe

Toute autre variante convertit en chaîne ce qu'elle enveloppe. `from_external_with` conserve l'erreur d'origine accessible, afin que les journaux puissent rendre toute la chaîne et que le code puisse toujours demander ce qui a réellement échoué :

```rust
use suprnova::FrameworkError;

let row = sqlx_like_query()
    .await
    .map_err(|e| FrameworkError::from_external_with("verify query failed", e))?;
```

`from_external(e)` est la même chose en prenant le propre `Display` de l'erreur comme message. Les deux se mappent en HTTP 500.

Pour inspecter l'original, utilisez `external_source()` plutôt que `source()` :

```rust
if let Some(src) = err.external_source() {
    if let Some(db) = src.downcast_ref::<sea_orm::DbErr>() {
        // décider si cela vaut la peine de réessayer
    }
}
```

`std::error::Error::source()` renvoie le handle `Arc` partagé, non l'erreur enveloppée ; un downcast à travers lui retourne donc `None`. `external_source()` déréférence d'abord le handle.

Le framework rend la chaîne complète dans la ligne de journal 5xx et dans le champ `debug_message` qu'il ajoute lorsque `APP_DEBUG=true`, de sorte que le texte d'une erreur enveloppée n'est jamais perdu.

### Préserver les indications de limite de débit

`RateLimited` existe afin qu'une indication `Retry-After` en aval survive son passage dans le système d'erreurs sous forme de `Duration`, plutôt que de s'effondrer en texte de message :

```rust
use std::time::Duration;
use suprnova::FrameworkError;

let err = FrameworkError::rate_limited(
    Some(Duration::from_secs(30)),
    "push provider rejected the batch",
);

assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
assert_eq!(err.status_code(), 429);
```

`retry_after()` retourne `None` pour toute autre variante et pour les limitations arrivées sans indication. La variante se rend en HTTP 429, et `.context(...)` la préserve au lieu de l'aplatir en `Domain`, si bien que la durée n'est jamais supprimée par l'ajout d'un contexte d'opération.

## `AppError` - erreurs de domaine ad hoc

Pour les erreurs ponctuelles où vous ne voulez pas définir de type dédié, utilisez `AppError`. Elle implémente `HttpError` et possède un `From` vers `FrameworkError`, si bien que `?` fonctionne directement :

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > 1_000_000 {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    Ok(json_response!({ "transferred": amount }))
}
```

Les constructeurs correspondent proprement à la forme `abort($status, $msg)` de Laravel :

| `AppError::*` | Statut |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | n'importe lequel |

Notez que `AppError::unauthorized` est **401** (authentification HTTP manquante), tandis que `FrameworkError::Unauthorized` est **403** (autorisation refusée, correspondant au rejet de policy de Laravel). Elles ne signifient pas la même chose ; choisissez celle qui correspond à l'échec.

## `HttpError` - erreurs typées personnalisées

Quand la même erreur de domaine apparaît à de nombreux endroits, modélisez-la comme un type. Implémentez `HttpError` et la conversion vous appartient :

```rust
use suprnova::HttpError;

#[derive(Debug)]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}

impl std::fmt::Display for InsufficientFunds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Insufficient funds: have {}, need {}",
            self.available, self.requested)
    }
}

impl std::error::Error for InsufficientFunds {}

impl HttpError for InsufficientFunds {
    fn status_code(&self) -> u16 { 402 }
    fn error_message(&self) -> String {
        format!("Need {} units, only {} available.",
            self.requested, self.available)
    }
}
```

`HttpError` a deux méthodes, toutes deux avec des valeurs par défaut :

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### Passerelle vers `?`

Un `impl<T: HttpError> From<T> for FrameworkError` naïf entrerait en conflit avec l'impl `From<AppError>` existant (parce que `AppError` implémente elle-même `HttpError`). Suprnova résout le problème de la règle de l'orphelin (orphan rule) avec un constructeur-passerelle explicite à la place :

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

Le code de statut et le message sont pris depuis `HttpError::status_code` et `HttpError::error_message`, puis stockés dans une variante `FrameworkError::Domain`. Le moteur de rendu de réponse suit ensuite le chemin normal de `Domain`.

### `#[domain_error]` pour des types sans code répétitif

Si vous voulez le motif d'erreur typée sans écrire les impls `Display`, `Error`, et `HttpError` à la main, utilisez la macro d'attribut `#[domain_error]` :

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFundsError {
    pub available: i64,
    pub requested: i64,
}
```

`#[domain_error]` génère l'ensemble complet des impls, *y compris* `From<YourError> for FrameworkError`, si bien que `?` fonctionne directement sans appel de passerelle :

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

Les trois paliers des erreurs personnalisées - `AppError` pour l'usage en ligne, `#[domain_error]` pour le typé-avec-macro, `HttpError` écrit à la main pour un contrôle total - vous donnent le bon outil à chaque niveau de formalisme.

## `ValidationErrors` - le sac d'erreurs de forme Laravel

Quand une requête échoue à la validation, Suprnova émet la même forme JSON que celle attendue par les frontends Laravel et Inertia :

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

Vous ne construisez généralement pas cela à la main - `#[derive(Validate)]` sur une requête de formulaire et la crate `validator` derrière elle produisent une `validator::ValidationErrors` que Suprnova convertit via `ValidationErrors::from_validator`. Mais le type est public quand vous en avez besoin :

```rust
use suprnova::{FrameworkError, ValidationErrors};

pub async fn after_validation(payload: &Signup) -> Result<(), FrameworkError> {
    let mut errs = ValidationErrors::new();

    if payload.email.ends_with("@example.com") {
        errs.add("email", "example.com addresses are not allowed");
    }
    if payload.password == payload.email {
        errs.add("password", "password must not match email");
    }

    errs.into_result().map_err(FrameworkError::Validation)
}
```

`add_to_bag` cantonne les erreurs sous un sac nommé (la forme `withErrors($errors, 'profile')` de Laravel) en préfixant le nom du sac avec un séparateur `.` :

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors map: { "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` ne garde que les entrées listées - utilisé en interne par l'en-tête `Precognition-Validate-Only` de Precognition afin que le serveur exécute la validation complète mais ne rapporte les erreurs que pour les champs que le client a demandés.

## Le contrat de conversion

Quand une `FrameworkError` atteint une limite HTTP, elle passe par `From<FrameworkError> for HttpResponse`. Trois choses se produisent, dans l'ordre :

1. **Routage du statut**. Le `status_code()` de la variante est lu une fois.
2. **Journalisation + observabilité**. Les 5xx déclenchent `tracing::error!` et dispatchent `ErrorOccurred` ; les 4xx déclenchent `tracing::warn!`. Les deux portent l'id de requête quand un est dans la portée.
3. **Rendu du corps**. Un corps JSON dans la forme Laravel, assaini pour les 5xx.

### La forme ordinaire du corps

Les réponses d'erreur ordinaires qui atteignent le moteur de rendu commun suivent ce squelette JSON :

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` est toujours présent dans ces réponses ordinaires.
- `errors` n'apparaît que pour les erreurs de type validation (`Validation`, `ValidationError`) - les deux rendent la même forme afin que les consommateurs n'aient qu'un seul chemin à analyser.
- `request_id` apparaît dans ces réponses ordinaires (`null` en dehors d'une portée de requête, par exemple pendant l'amorçage précoce ou dans des tests sans contexte de requête).
- `debug_message` n'apparaît que pour les 5xx quand `APP_DEBUG=true`. C'est strictement additif - les clients de production ne doivent pas s'appuyer dessus.

Trois variantes spéciales retournent avant l'injection de l'id de requête :

- `PrecognitionSuccess` est une réponse 204 sans corps.
- `PrecognitionFailure` contient le corps de validation plus les en-têtes Precognition.
- Une sentinelle `AlreadyReported` rendue par erreur via HTTP est une réponse 500 générique ne contenant que `message`.

### La règle d'assainissement des 5xx

C'est la garantie de sécurité qui mérite d'être retenue par cœur. Pour toute erreur dont le statut est ≥ 500 et qui atteint le moteur de rendu commun, le `message` du corps JSON est remplacé par la chaîne littérale :

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

Le détail brut de l'erreur ne fuit **pas** vers le corps de la réponse. Il va vers :

- l'entrée de journal `tracing::error!`, avec l'id de requête et le statut
- l'événement `ErrorOccurred`, que n'importe quel écouteur peut récupérer

Quand `APP_DEBUG=true` (faux par défaut en dehors de `local`/`dev`/`test`), la réponse porte aussi un champ `debug_message` avec le détail brut - mais `message` reste générique dans les deux modes, afin que les frontends et clients ne puissent pas se coupler accidentellement à des données réservées au développement.

C'est le contrat qui vous permet d'appeler `FrameworkError::internal("db connection refused: password mismatch on user 'app_rw'")` sans faire fuiter le mot de passe vers le réseau. Le `message` que vous passez est destiné aux opérateurs qui lisent les journaux ; le `message` que voit le client est `"Internal Server Error"`.

Pour les erreurs 4xx, le message destiné à l'appelant est préservé - `404 User not found`, `400 Missing required parameter: user_id`. Ce sont des erreurs de domaine sur lesquelles le client doit agir, pas des défaillances internes.

### Où réside le contrat

Toute la conversion tient en une seule fonction - `impl From<FrameworkError> for HttpResponse` dans `framework/src/http/response.rs`. Lisez-la une fois et vous aurez lu toute la surface de rendu des erreurs de Suprnova. Il n'y a pas d'autre chemin.

## La limite de panique

Une panique dans un middleware ou un handler se propagerait autrement le long de la tâche par connexion et démolirait le service hyper en pleine réponse, laissant le client avec une réinitialisation TCP et aucune réponse HTTP. Suprnova la capture.

`execute_chain_safely` dans `framework/src/server.rs` enveloppe la chaîne de middleware dans `AssertUnwindSafe(...).catch_unwind().await`. Lors d'une panique, elle :

1. Extrait la charge utile de la panique (gère les charges `&'static str` et `String` ; tout le reste apparaît comme `"panic with non-string payload"`).
2. Journalise avec `tracing::error!`, avec la méthode, le chemin et l'id de la requête.
3. Construit `FrameworkError::internal(format!("request handler panicked: {msg}"))` et l'achemine à travers la *même* conversion `From<FrameworkError> for HttpResponse` que tout autre 5xx utilise.
4. Renvoie l'id de requête en écho sous `X-Request-Id`.

La charge utile de la panique reste dans l'entrée de journal ; le client reçoit le corps assaini `{"message": "Internal Server Error"}`. Les écouteurs d'observabilité qui se déclenchent sur `ErrorOccurred` pour les erreurs 5xx retournées se déclenchent aussi sur les paniques - il n'y a pas de surface d'événement de panique séparée à câbler.

Le même motif de récupération de panique est utilisé par :

- les handlers WebSocket (`framework/src/server.rs`)
- les tâches planifiées (`framework/src/schedule/mod.rs`)
- les workflows (`framework/src/workflow/mod.rs`)
- le trait `Supervisor` (broadcasting)

Une panique dans l'un de ces sous-systèmes est journalisée et soit traduite en état d'erreur, soit redémarrée automatiquement ; elle ne fait pas tomber la tâche worker.

## Brancher l'observabilité avec `ErrorOccurred`

`ErrorOccurred` est un événement intégré que le framework distribue sur chaque réponse 5xx (y compris celles synthétisées à partir de paniques) :

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

Écoutez-le de la même manière que n'importe quel événement :

```rust
use std::sync::Arc;
use suprnova::{ErrorOccurred, EventFacade, FrameworkError, Listener};

pub struct SentryReporter;

#[suprnova::async_trait]
impl Listener<ErrorOccurred> for SentryReporter {
    async fn handle(&self, evt: &ErrorOccurred) -> Result<(), FrameworkError> {
        sentry::capture_message(&evt.error_message, sentry::Level::Error);
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

C'est l'équivalent Suprnova du callback `report()` de Laravel sur le handler d'exceptions global. L'événement arrive avec l'`error_message` original non assaini (le corps que voit le client reste assaini), le code de statut et l'id de requête corrélable.

### Rendre toute la chaîne : `render_error_chain`

Le `Display` généré par `thiserror` n'imprime que le message propre d'une erreur ; la `source` enveloppée d'un `FrameworkError::External` est donc invisible tant que quelque chose ne parcourt pas la chaîne. `render_error_chain` effectue ce parcours et joint le résultat avec `": "`, le même séparateur que `.context()` - le framework l'appelle avant de construire `error_message` ci-dessus et avant la ligne de journal 5xx correspondante, raison pour laquelle une erreur enveloppée ne perd pas sa cause dans l'un ou l'autre cas.

Utilisez-le vous-même lorsqu'un écouteur ou un collecteur de journaux a besoin du même rendu de chaîne complet, par exemple pour réenvelopper `error_message` avant de le transmettre à un collecteur qui n'accepte qu'une chaîne plate :

```rust
use suprnova::render_error_chain;

let chain = render_error_chain(&err);
// "loading users: connection refused (os error 111)"
```

## Fonctions d'interruption

Trois fonctions libres court-circuitent un handler à un statut donné. Elles reflètent les `abort` / `abort_if` / `abort_unless` de Laravel :

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

Chacune retourne `Result<(), FrameworkError>`. Utilisez-les avec `?`. L'erreur sous-jacente est `FrameworkError::Domain { message, status_code }`, si bien qu'elle se rend à travers la même forme de corps et les mêmes règles d'assainissement que toute autre erreur. Les codes de statut hors limites sont forcés à 500 par la validation de statut du moteur de rendu de réponse ; vous n'avez pas besoin de vous prémunir contre une mauvaise entrée au site d'appel.

## La sentinelle CLI : `AlreadyReported`

Une variante de `FrameworkError` n'a aucune signification HTTP. `AlreadyReported` est construite via `FrameworkError::silent()` et utilisée par le dispatcher de console quand clap a déjà formaté et affiché sa propre erreur d'analyse d'arguments. Le `main` du binaire traduit la sentinelle en un code de sortie non nul sans `eprintln`, si bien que les utilisateurs ne voient jamais deux messages d'erreur pour le même échec.

Si `AlreadyReported` atteint un jour un convertisseur de réponse HTTP, cela indique qu'un handler de requête a accidentellement retourné `silent()`. Le convertisseur journalise un `tracing::error!` bien visible identifiant la fuite et retourne un 500 générique contenant uniquement `{"message": "Internal Server Error"}`. La variante n'a rien à faire dans le chemin de requête, et le journal bien visible rend le bug observable plutôt que silencieux.

Vous ne voyez normalement pas cette variante ; elle est documentée ici parce que l'enum est `HTTP-flavoured` et que cette variante autrement inexpliquée intriguerait quiconque lit le code source.

## Garanties de sécurité, en résumé

Le contrat que Suprnova vous donne :

- **Conversion totale**. Chaque `FrameworkError` produit une `HttpResponse`. Il n'y a aucun chemin d'erreur qui fait planter le serveur ou qui abandonne la connexion silencieusement.
- **5xx assainis**. Le moteur de rendu commun remplace le `message` transmis pour tout 5xx par `Internal Server Error` ; le détail brut s'écoule vers les journaux et `ErrorOccurred`. Une sentinelle `AlreadyReported` rendue par erreur via HTTP retourne le même message générique sans `request_id`.
- **Visibilité de débogage optionnelle**. `APP_DEBUG=true` ajoute un champ `debug_message` pour les réponses ordinaires 5xx, jamais `message`. Les clients de production ne peuvent pas se coupler accidentellement à des données réservées au développement.
- **Ids de requête corrélables**. Chaque corps d'erreur ordinaire qui atteint le moteur de rendu commun porte l'id de requête (ou `null` quand aucune portée de requête n'existe) ; le même id apparaît dans la ligne de journal et dans l'événement `ErrorOccurred`. Les trois variantes à retour anticipé décrites ci-dessus contournent ce champ.
- **Récupération de panique**. Les paniques dans les handlers et le middleware sont capturées, journalisées et acheminées à travers le même impl `From` que les erreurs retournées. Aucune perte de connexion, aucune lacune d'observabilité.
- **Une seule forme commune pour les erreurs ordinaires**. Les erreurs de validation, les erreurs de paramètres, les paniques, les erreurs de domaine personnalisées et les défaillances de stockage qui atteignent le moteur de rendu commun utilisent le même squelette JSON. Les trois variantes spéciales documentées ci-dessus ont des formes réseau distinctes.

## Où réside chaque élément

| Élément | Fichier |
|---|---|
| `FrameworkError`, `AppError`, `HttpError`, `ValidationErrors` | `framework/src/error.rs` |
| `render_error_chain` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse` (conversion + assainissement) | `framework/src/http/response.rs` |
| `abort`, `abort_if`, `abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely` (limite de panique) | `framework/src/server.rs` |
| Événement `ErrorOccurred` | `framework/src/events/builtins.rs` |
| Macro `#[domain_error]` | `suprnova-macros/src/domain_error.rs` |

## Suivant

- [Gestion des erreurs](errors.md) - les motifs pratiques de handler qui utilisent ce modèle
- [Cycle de vie des requêtes](lifecycle.md) - à quel moment du flux de requête s'exécute la conversion d'erreur
- [Validation](validation.md) - `#[derive(Validate)]`, les requêtes de formulaire et comment `ValidationErrors` se peuple
- [Réponses](responses.md) - les builders `HttpResponse`, les en-têtes, les cookies, le streaming
- [Événements](events.md) - écouter `ErrorOccurred` et les autres événements intégrés
