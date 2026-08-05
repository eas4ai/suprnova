# Gestion des erreurs

Ce chapitre est le guide des motifs quotidiens pour écrire du code
faillible dans les handlers, les services et le middleware Suprnova.
Pour le modèle sous-jacent - le contrat de conversion, la limite de
panique, la règle d'assainissement des 5xx, les hooks d'observabilité -
lisez [Modèle d'erreur](error-model.md). Ce chapitre-ci montre ce qu'il
faut réellement taper.

La forme à retenir :

- Les handlers retournent `Response = Result<HttpResponse, HttpResponse>`.
- L'opérateur `?` réduit `FrameworkError`, `AppError`, `DbErr`,
  `ParamError`, `ValidationErrors` et tout `HttpError` typé en une
  `HttpResponse`, automatiquement.
- Trois helpers libres (`abort_with`, `abort_if`, `abort_unless`) vous
  laissent court-circuiter à un code de statut sans nommer de type
  d'erreur.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;          // 400 si absent
    let user = find_user(id).await?;    // 500 sur DbErr, 404 sur Option::None
    json_response!({ "user": user })
}
```

Le reste du chapitre est le catalogue des producteurs d'erreurs - quoi
construire, quel statut cela retourne, quelle forme voit le client.

## `?` est la conversion

Chaque `?` dans le corps d'un handler exécute `From<E> for HttpResponse`.
Le framework câble ces impls afin que les choses que vous appelez
réellement retournent des erreurs qui savent déjà se transformer en
réponse. Vous
n'écrivez pas la conversion ; vous écrivez l'échec.

```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```

Trois choses se produisent dans cet extrait - aucune n'est visible :

1. `req.param("id")?` → `ParamError` → `FrameworkError::ParamError` (400).
2. `.await?` sur un appel SeaORM → `DbErr` → `FrameworkError::Database`
   (500, assaini avant le réseau).
3. `.ok_or_else(...)?` construit directement une
   `FrameworkError::ModelNotFound` (404).

Toutes trois passent par le même impl `From<FrameworkError> for
HttpResponse` décrit dans [Modèle d'erreur](error-model.md).

## `AppError` - erreurs de domaine en ligne

Utilisez `AppError` pour les erreurs ponctuelles qui ne méritent pas de
type dédié. Les constructeurs correspondent à la forme
`abort($status, $msg)` de Laravel :

| Constructeur | Statut |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | n'importe lequel |

`AppError` possède un `From` vers `FrameworkError`, si bien que `?`
fonctionne sans cérémonie :

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > balance() {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    json_response!({ "transferred": amount })
}
```

Notez l'asymétrie : `AppError::unauthorized` est **401** (identifiants
d'authentification manquants), tandis que `FrameworkError::Unauthorized`
est **403** (une policy a refusé un utilisateur authentifié). Elles ne
signifient pas la même chose ; choisissez celle qui correspond à l'échec.

## `FrameworkError` - l'enum canonique

Les extracteurs internes, le conteneur, la liaison de route, la
validation, la couche base de données et le stockage produisent tous une
`FrameworkError`. Vous en construisez généralement une via un
constructeur de commodité et laissez `?` l'acheminer.

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409 (n'importe quel code)
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

L'ensemble complet des variantes, avec ses implications sur la forme de
la réponse, se trouve dans [Modèle d'erreur](error-model.md). Les
constructeurs ci-dessus couvrent tous les cas courants ; vous ne recourez
directement aux variantes que pour faire un `match` sur une erreur que
vous avez reçue.

### Conversions automatiques

`FrameworkError` parle déjà les dialectes qu'émettent vos dépendances.
Ces deux `?` se convertissent automatiquement :

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get retourne Result<_, FrameworkError>.
    // .insert retourne Result<_, DbErr>, avec From<DbErr> for FrameworkError.
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Le framework implémente aussi `From<opendal::Error>` pour les opérations
de stockage et `From<ParamError>` pour l'extraction des paramètres de
chemin.

### Relancer avec du contexte

Quand vous voulez annoter la provenance d'une erreur sans perdre le code
de statut, utilisez `.context()` :

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

Le message devient `"creating new user: <original>"`. Les variantes
structurées (`Validation`, `ValidationError`, `ModelNotFound`,
`ParamParse`, `PrecognitionFailure`, `Unauthorized`) conservent leur
variante afin que le moteur de rendu de réponse émette toujours la bonne
forme ; les variantes plates qui ne portent qu'un message (`Internal`,
`Database`, `Domain`) s'aplatissent en une `Domain` avec le message
préfixé et le statut d'origine préservé.

### Transformer les erreurs de clé dupliquée en 422

La règle de validation `Unique` exécute un `SELECT COUNT(*)` avant
l'écriture : elle est donc indicative - deux requêtes simultanées peuvent
toutes deux passer, puis toutes deux tenter l'insertion. La requête
perdante reçoit une violation de contrainte d'unicité de la base de
données, qui fuirait autrement en 500. `from_unique_violation` la traduit
dans le même 422 qu'aurait produit la règle indicative :

```rust
use suprnova::FrameworkError;

let user = new_user.insert(db).await.map_err(|e| {
    FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    )
})?;
```

Si le `DbErr` sous-jacent n'est pas une violation de contrainte
d'unicité, il passe inchangé en tant qu'erreur `Database` de classe 500.
La couverture des backends est celle que reconnaît le `DbErr::sql_err` de
SeaORM - Postgres, MySQL/MariaDB et SQLite acheminent tous leurs erreurs
de clé dupliquée.

## Erreurs de domaine personnalisées

Trois paliers, selon le degré de réutilisation dont l'erreur a besoin.

### `#[domain_error]` pour le cas typé

La plupart des erreurs réutilisables veulent un nom, un statut fixe et un
gabarit de message fixe - pas de message par appel. La macro d'attribut
`#[domain_error]` génère `Display`, `std::error::Error`, `HttpError` et
`From` pour `FrameworkError` d'un seul coup :

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFound;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}
```

Utilisez-les au site d'appel avec `?` :

```rust
use crate::errors::user_not_found::UserNotFound;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;

    json_response!({ "user": user })
}
```

La macro rejette explicitement les attributs mal formés à la compilation -
codes de statut en dépassement (`status = 70_000`), types de littéraux
erronés (`message = 42`), clés inconnues - si bien qu'une faute de frappe
ne peut pas vous donner silencieusement le mauvais statut.

#### En scaffolder un avec la CLI

```bash
suprnova make:error UserNotFound
```

Écrit `src/errors/user_not_found.rs` avec un `status = 500` par défaut et
un message déduit en casse de phrase, puis met à jour `src/errors/mod.rs`
pour le réexporter. Ajustez `status` et `message` à votre convenance.

### `HttpError` pour le cas écrit à la main

Quand une erreur de domaine a besoin d'un état d'exécution dans son
message (par ex. les ID impliqués dans l'échec), implémentez `HttpError`
directement. Le trait a deux méthodes, avec des valeurs par défaut
raisonnables :

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

Pour faire le pont entre un `HttpError` écrit à la main et `?`, appelez
`FrameworkError::from_http_error`. Un impl générique
`From<T: HttpError> for FrameworkError` entrerait en conflit avec l'impl
`From<AppError>` existant, la passerelle est donc un constructeur
explicite :

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### Des enums d'erreur pour les échecs d'un seul module

Quand un service a plusieurs échecs apparentés, regroupez-les dans un
enum et écrivez un seul `From` pour l'enum tout entier :

```rust
use suprnova::FrameworkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Order {0} not found")]
    NotFound(i64),

    #[error("Insufficient stock for product {product_id}")]
    InsufficientStock { product_id: i64 },

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Order already shipped")]
    AlreadyShipped,
}

impl From<OrderError> for FrameworkError {
    fn from(err: OrderError) -> Self {
        let status = match &err {
            OrderError::NotFound(_) => 404,
            OrderError::InsufficientStock { .. } => 422,
            OrderError::PaymentFailed(_) => 402,
            OrderError::AlreadyShipped => 409,
        };
        FrameworkError::Domain {
            message: err.to_string(),
            status_code: status,
        }
    }
}
```

Une fois que le `From` existe, l'enum se propage à travers `?` exactement
comme n'importe quel autre type d'erreur.

## `abort_with` / `abort_if` / `abort_unless`

Trois helpers court-circuitent un handler à un statut donné. Ils
reflètent les `abort` / `abort_if` / `abort_unless` de Laravel. (La
fonction libre est exportée sous le nom `abort_with` plutôt qu'`abort`,
afin de garder ce dernier disponible comme nom de méthode sur les types
utilisateur.)

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

Chacun retourne `Result<(), FrameworkError>`, si bien que `?` fait le
travail. L'erreur sous-jacente est
`FrameworkError::Domain { message, status_code }`, qui se rend à travers
la même forme de corps que toute autre erreur. Les codes de statut hors
limites sont forcés à 500 par le moteur de rendu de réponse ; vous n'avez
pas besoin de vous prémunir contre une mauvaise entrée au site d'appel.

## `ValidationErrors` - le sac d'erreurs de forme Laravel

Quand la validation échoue - au moment du `#[derive(Validate)]` ou dans
un corps d'`after_validation` - le framework émet la forme JSON
qu'attendent les frontends Laravel et Inertia :

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password field must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

La plupart du temps, vous ne construisez pas cela directement -
`#[derive(Validate)]` s'exécute et le framework convertit
`validator::ValidationErrors` pour vous. Quand vous avez besoin d'ajouter
des erreurs impérativement (règles inter-champs, vérifications d'unicité
asynchrones qui complètent `Unique`), construisez une `ValidationErrors`
et retournez-la :

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

`add_to_bag` cantonne un champ sous un sac nommé (la forme
`withErrors($errors, 'profile')` de Laravel) en préposant le nom du sac
au champ, séparés par un point. Utile quand une même réponse porte les
erreurs de plusieurs sous-formulaires qui ne peuvent pas partager un
espace de noms plat :

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// map des erreurs : { "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` convertit une `validator::ValidationErrors` ;
`retain_fields(&keep)` retourne une copie ne contenant que les entrées
listées (utilisée en interne par l'en-tête `Precognition-Validate-Only`
de Precognition).

## Brancher l'observabilité avec `ErrorOccurred`

Chaque réponse 5xx déclenche un événement `ErrorOccurred` - y compris
celles synthétisées à partir de paniques. Écoutez-le de la même façon que
vous écoutez n'importe quel événement :

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

// Dans bootstrap.rs :
// `listen` déduit les deux génériques du type de l'écouteur. Elle retourne
// `()` (l'enregistrement ne peut pas échouer), donc pas de `?` ni de Result.
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

L'événement porte le message d'erreur brut (le corps qui part sur le
réseau reste assaini - voir [Modèle d'erreur](error-model.md)), le
statut, et l'id de requête corrélable. C'est l'équivalent Suprnova du
callback `report()` de Laravel sur le handler d'exceptions.

## Motifs que vous écrirez souvent

### Analyser un paramètre de chemin en valeur typée

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` se convertit déjà en 400 ; `param_parse` en est l'équivalent
pour l'échec d'analyse et rend la même forme.

### Rechercher par ID, 404 si absent

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` fait passer le `DbErr` de SeaORM par
`From<DbErr> for FrameworkError`, puis par
`From<FrameworkError> for HttpResponse`. Rust n'enchaîne pas
automatiquement les impls `From` sur deux sauts, le `.map_err` explicite
est donc obligatoire.

Ou, avec la couche Eloquent (qui enveloppe déjà SeaORM et retourne
directement `Result<_, FrameworkError>`) :

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail`, c'est `find(id).ok_or(ModelNotFound)` empaqueté.

### Autoriser une action

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` retourne `Result<(), FrameworkError>` ; le `?` la réduit
dans la branche d'erreur de votre handler.

### Un service qui retourne des erreurs typées

```rust
use suprnova::{App, FrameworkError, injectable};

#[injectable]
pub struct UserService;

impl UserService {
    pub async fn find_by_email(&self, email: &str)
        -> Result<users::Model, FrameworkError>
    {
        users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&*DB::get()?)
            .await?
            .ok_or_else(|| FrameworkError::not_found("User"))
    }
}

// Site d'appel :
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` retourne `Result<Arc<UserService>,
FrameworkError>`. Les `?` chaînés réduisent à une réponse aussi bien
l'échec de résolution que l'échec de recherche.

## Aide-mémoire

| Vous voulez… | Recourez à |
|---|---|
| Une erreur en ligne avec un statut | `AppError::bad_request("…")` et consorts |
| Une erreur typée réutilisable | `#[domain_error(status = …, message = "…")]` |
| Un scaffold généré | `suprnova make:error UserNotFound` |
| Une erreur écrite à la main avec un état d'exécution | `impl HttpError for MyError` |
| Faire le pont vers `?` depuis une erreur écrite à la main | `FrameworkError::from_http_error(e)` |
| Court-circuiter à un statut | `abort_with` / `abort_if` / `abort_unless` |
| Un 404 sur un modèle absent | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| Un échec d'analyse sur un paramètre de chemin | `FrameworkError::param_parse("id", "i64")` |
| Une erreur de validation au niveau du champ | `FrameworkError::validation("email", "…")` |
| Un sac d'erreurs multi-champs | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| Une violation de clé dupliquée → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| Annoter une erreur existante | `err.context("creating user")` |
| Observer chaque 5xx | Écouter `ErrorOccurred` |

## Suivant

- [Modèle d'erreur](error-model.md) - les variantes, le contrat de
  conversion, l'assainissement des 5xx, la limite de panique
- [Validation](validation.md) - `#[derive(Validate)]`, les requêtes de
  formulaire, et `after_validation`
- [Réponses](responses.md) - les builders `HttpResponse`, le statut, les
  en-têtes
- [Événements](events.md) - écouter `ErrorOccurred` et les autres
  événements intégrés
- [Cycle de vie des requêtes](lifecycle.md) - à quel moment du flux de
  requête s'exécute la conversion d'erreur
