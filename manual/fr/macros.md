# Macros

Suprnova fournit environ trois douzaines de macros, toutes réexportées
depuis `suprnova::*`. Ce sont les points de jonction où le framework
rencontre votre code - `routes!` construit le routeur, `#[handler]`
adapte une fonction pour en faire un handler, `#[suprnova::model]`
transforme une struct en modèle Eloquent, `#[derive(Data)]` produit
une charge utile Inertia typée. Ce chapitre est l'index. Chaque macro
reçoit une description d'un paragraphe, un exemple minimal, et un
renvoi vers le chapitre qui l'utilise pour un vrai travail.

Quelques principes qui tiennent sur toute la surface :

- **Les macros émettent des chemins entièrement qualifiés.** Le code généré
  écrit `::suprnova::…` afin que les macros fonctionnent que vous ayez
  importé les types sous-jacents ou non.
- **Usage intensif de `inventory::submit!`.** Les modèles, commandes, policies,
observateurs, fournisseurs de paiement, et plus encore s'enregistrent
eux-mêmes à la compilation et le framework vide le registre à
l'amorçage.
  Vous ne câblez presque jamais l'enregistrement à la main.
- **Validation à la compilation là où cela rapporte.** `inertia_response!`
vérifie que le fichier de composant nommé existe. `redirect!` vérifie
  que la route nommée existe. `routes!` rejette les chemins qui ne
  commencent pas par `/`. Les erreurs qui peuvent être détectées à la
  compilation le sont.

## Routage

| Macro | Retourne | Ce qu'elle fait |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | Liste de routes de premier niveau - exporte un `register()` que votre `app.rs` appelle |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | Une route HTTP - chaînable avec `.name(...)` / `.middleware(...)` |
| `group!` | `GroupDef` | Préfixe + middleware appliqués à une liste enfant de routes |
| `fallback!` | `FallbackDefBuilder<H>` | Handler 404 personnalisé quand aucune route ne correspond |
| `ws!` | `WsRouteDef` | Une route WebSocket - chaînable avec `.middleware(...)` / `.config(...)` |

```rust
use suprnova::{routes, get, post, ws, group};
use crate::{controllers, middleware::AuthMiddleware, ws::ChatHandler};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::user::show).name("users.show"),
    post!("/users", controllers::user::store).name("users.store"),

    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard),
    }).middleware(AuthMiddleware),

    ws!("/ws/chat", ChatHandler),
}
```

La chaîne de chemin de route est vérifiée à la compilation -
`validate_route_path` rejette tout ce qui ne commence pas par `/`. Les
noms de route enregistrés via `.name("…")` sont aussi vérifiés pour
leur unicité à l'amorçage via `register_route_name`. Voir
[Routage](routing.md) pour l'expansion complète et
[WebSockets](websockets.md) pour `ws!`.

## Handlers et requêtes

### `#[handler]`

Réécrit une fonction de contrôleur pour qu'elle puisse extraire des
paramètres typés (via `FromRequest`) directement depuis la requête
entrante - au lieu de tirer les champs de `Request` à la main, vous
déclarez ce dont le handler a besoin et la macro fait le câblage.

```rust
use suprnova::{handler, Response, json_response, request};

#[request]
pub struct CreateUserRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` est déjà validé - un 422 est retourné automatiquement en cas d'échec
    json_response!({ "email": form.email })
}
```

Un premier paramètre de forme `Request` reste accepté comme cas
d'identité. Voir [Contrôleurs](controllers.md).

### `#[request]` et `#[derive(FormRequest)]`

`#[request]` est la façon recommandée de déclarer un type de requête
validée. Il dérive automatiquement `Deserialize`, `Validate` et
`FormRequest`, si bien que la struct fonctionne aussi bien avec des corps
`application/json` qu'`application/x-www-form-urlencoded`.

`#[derive(FormRequestDerive)]` est le derive sous-jacent, si vous voulez
vous passer de l'attribut (il vous faudra alors dériver `Deserialize` et
`Validate` vous-même). L'attribut est ce que nous recommandons ; le derive
existe pour le cas limite. Voir [Requêtes](requests.md) et
[Validation](validation.md).

### `#[derive(MultipartRequest)]`

Extracteur fortement typé pour `multipart/form-data` - lie les champs
texte et les fichiers téléversés dans une seule struct, avec des validateurs
au niveau du type pour chaque champ.

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{ImageFile, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(ImageFile, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

Les validateurs intégrés (`ImageFile`, `MimeAllowlist<…>`, `MaxSize<…>`,
`MimeType<…>`) se composent via des tuples. Voir [Requêtes](requests.md).

## Réponses

### `json_response!` et `text_response!`

Les deux macros de réponse en forme courte. Toutes deux enveloppent
`HttpResponse::*` dans `Ok(...)` afin de s'insérer directement dans la
position de retour d'un handler :

```rust
use suprnova::{handler, json_response, text_response, Response};

#[handler]
pub async fn health() -> Response {
    json_response!({ "status": "ok" })
}

#[handler]
pub async fn robots() -> Response {
    text_response!("User-agent: *\nDisallow:")
}
```

Voir [Réponses](responses.md).

### `inertia_response!`

Construit une réponse de page Inertia, en vérifiant à la compilation
que le fichier de composant nommé (`.svelte` / `.tsx` / `.jsx` /
`.vue`) existe dans `frontend/src/pages/`. Si vous faites une faute de
frappe dans le nom du composant, la compilation échoue avec des
suggestions :

```rust
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
struct HomeProps {
    title: String,
    user_count: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        user_count: 42,
    })
}
```

`#[derive(InertiaProps)]` génère l'impl `Serialize` dont la forme de
réponse a besoin. Voir [Réponses
Inertia](frontend-inertia-responses.md).

### `redirect!`

Redirection sûre au niveau des types vers une route nommée - le nom de
route est vérifié à la compilation par rapport aux noms enregistrés
via `routes!` :

```rust
use suprnova::redirect;

// Ne compile que si "users.show" est un nom de route enregistré
let resp = redirect!("users.show").with("id", "42").into();
```

Voir [Génération d'URL](urls.md).

## Eloquent

### `#[suprnova::model]`

Transforme une struct simple en un modèle Eloquent complet : génère
les stubs SeaORM `Entity`, `Model`, `ActiveModel`, `Column`,
`Relation`, plus tous les impls de trait dont Eloquent a besoin. Fait
aussi un `inventory::submit!` d'une `ModelEntry` afin que le framework
puisse énumérer chaque modèle à l'amorçage.

```rust
use suprnova::model;

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Les clés d'attribut incluent `table`, `primary_key`, `key_type`,
`auto_increment`, `connection`, `fillable`, `guarded`, `casts`,
`timestamps`, `soft_deletes`, `appends`, `hidden`, `visible`,
`mutators`, `touches`, et `unique_id` (pour les PK UUID/ULID). Voir
[Eloquent](eloquent.md).

### `#[suprnova::scopes(Model)]`

Parcourt un bloc `impl Model { … }` et transforme chaque méthode dont
la signature correspond à
`fn name(query: Builder<Self>[, args…]) -> Builder<Self>` en un scope -
en générant à la fois `Model::scope_name(args)` et un
`.scope_name(args)` chaînable sur `Builder<Model>`.

```rust
use suprnova::{scopes, Builder};

#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }

    // Pas un scope - passe inchangée
    pub fn display_name(&self) -> String { self.name.clone() }
}

// Les deux sites d'appel compilent :
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

La forme chaînable requiert que le trait généré
`HasScope_<scope>_<Model>` soit en portée quand elle est appelée
depuis un autre module. Voir [Eloquent](eloquent.md).

### `#[suprnova::observer(Model)]`

Câble un bloc `impl Observer<M>` dans le système d'événements de cycle
de vie - chacune des 16 méthodes redéfinies devient un écouteur
enregistré, soumis à l'inventory et vidé à l'amorçage.

```rust
use async_trait::async_trait;
use suprnova::eloquent::observers::Observer;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::attrs::Attrs;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

**Ordre d'attribut requis : `#[suprnova::observer(M)]` doit venir
avant `#[async_trait]`.** Les macros d'attribut se développent de
l'extérieur vers l'intérieur - si `async_trait` s'exécute en premier,
elle réécrit chaque `async fn` sous une forme sans sucre syntaxique et
la correspondance de nom de la macro observer contre les 16 noms de
méthode du trait ne trouve silencieusement rien. Voir
[Événements](events.md).

### `#[suprnova::accessor]` et `#[suprnova::mutator]`

Marqueurs au niveau de la fonction sur les méthodes de
`impl Model { … }` qui se branchent sur les chemins `to_json()` /
`fill()` du modèle. Référencez le nom du champ dans
`#[model(appends = […])]` (accessor) ou `#[model(mutators = […])]`
(mutator) pour que la macro les câble.

```rust
#[suprnova::model(appends = ["full_name"], mutators = ["password"])]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

impl User {
    #[suprnova::accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        self.password = bcrypt(raw);
        Ok(())
    }
}
```

Voir [Mutateurs et casts](eloquent-mutators.md).

### `#[suprnova::prunable]`

Enveloppe un impl `Prunable` (ou `MassPrunable`) et soumet une
`PrunerEntry` dans le registre que `model:prune` parcourt à
l'exécution :

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for Session {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

Voir [Eloquent](eloquent.md).

### `attrs!`

Construit une map `Attrs` ordonnée
(`IndexMap<&'static str, serde_json::Value>`) pour `Model::create` /
`Model::update` / `Model::fill` :

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

Voir [Eloquent](eloquent.md).

### `casts!`

Construit une map de casts par requête que vous pouvez passer à
`Builder::with_casts` :

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

Voir [Mutateurs et casts](eloquent-mutators.md).

### `route_binding!`

Implémente `RouteBinding` pour une entité SeaORM écrite à la main afin
qu'elle se résolve automatiquement depuis un paramètre de route. Les
modèles définis avec `#[suprnova::model]` s'enregistrent
automatiquement et n'en ont pas besoin ; tournez-vous vers
`route_binding!` quand vous avez écrit l'entité à la main :

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

Après cela, `get!("/users/{user}", controllers::user::show)` passe un
`User` entièrement chargé à votre handler. Voir [Routage](routing.md).

## Données et Inertia

### `#[derive(Data)]`

Le derive composite pour les charges utiles typées. Produit un impl
`Serialize` qui respecte les champs `#[data(input_only)]`, plus un
impl `Deserialize` qui rejette les charges utiles qui tentent de
définir des champs `#[data(output_only)]`. Associez-le à
`#[json_resource("type")]` pour une sortie JSON:API via le chapitre
`Resource`.

```rust
use suprnova::{Data, Validate};

#[derive(Data, Validate)]
struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    #[data(allow_include)]
    pub posts: Vec<PostDto>,
}
```

`#[data(allow_include)]` enregistre le champ dans l'allowlist
d'include du rechargement partiel via `inventory::submit!`. Voir
[Objets de données](data.md) et [Ressources
API](eloquent-resources.md).

### `#[derive(InertiaProps)]`

Génère l'impl `Serialize` dont `inertia_response!` a besoin. Un simple
derive marqueur - la plupart des applications se tournent plutôt vers
`#[derive(Data)]` parce qu'il vous donne les includes de rechargement
partiel gratuitement.

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

Voir [Réponses Inertia](frontend-inertia-responses.md).

### `when_loaded!`

Émet un `Prop::lazy(…)` seulement quand une relation nommée a été
chargée en eager (eager-loaded) sur l'entité ; sinon émet
`Prop::absent()` afin que la prop soit entièrement omise de la
réponse :

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

Voir [Objets de données](data.md).

## Injection de dépendances

### `#[service]`

Ajoute `Send + Sync + 'static` à un trait afin qu'il s'insère dans le
conteneur :

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

Voir [Conteneur de service](container.md).

### `#[injectable]`

Enregistre automatiquement un type concret comme singleton. Dérive
`Default` + `Clone` et soumet un enregistrement qui s'exécute à
l'amorçage.

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

Voir [Conteneur de service](container.md).

## Erreurs

### `#[domain_error]`

Définit une erreur de domaine qui implémente `Display`, `Error`,
`HttpError`, et `From<T> for FrameworkError` - si bien qu'elle
court-circuite un handler via `?` :

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError {
    pub user_id: i32,
}

pub async fn get_user(id: i32) -> Result<User, FrameworkError> {
    let user = User::find(id).await?
        .ok_or_else(|| UserNotFoundError { user_id: id })?;
    Ok(user)
}
```

Voir [Gestion des erreurs](errors.md).

## Console et travail en arrière-plan

### `#[command]`

Marque un `async fn(Vec<String>) -> Result<(), FrameworkError>` comme
commande de console. Soumet une `CommandEntry` afin que
`dispatch_argv` la trouve quand le binaire console propre au projet
s'exécute :

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

Voir [Console](console.md).

### `#[derive(Command)]`

L'alternative à arguments typés. Se place au-dessus de
`#[derive(clap::Parser)]`, lit `#[console(...)]` pour les métadonnées,
et émet le runner qui appelle votre `TypedCommand::run` :

```rust
use async_trait::async_trait;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(clap::Parser, Command)]
#[console(name = "greet", description = "Greet someone")]
pub struct Greet {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(long)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let target = self.name.unwrap_or_else(|| "world".into());
        println!("{}", if self.loud { format!("HELLO {target}!") } else { format!("Hello {target}") });
        Ok(())
    }
}
```

Voir [Console](console.md).

### `#[workflow]` et `#[workflow_step]`

`#[workflow]` enregistre une fonction async comme un workflow durable -
état exécutable, étapes réessayables, historique persisté. Chaque
`#[workflow_step]` à l'intérieur du corps est un point de contrôle
depuis lequel le runtime peut reprendre après un crash ou un
redémarrage.

```rust
use suprnova::{workflow, workflow_step, FrameworkError};

#[workflow]
async fn onboard_user(user_id: i64) -> Result<(), FrameworkError> {
    send_welcome_email(user_id).await?;
    enable_default_features(user_id).await?;
    Ok(())
}

#[workflow_step]
async fn send_welcome_email(user_id: i64) -> Result<(), FrameworkError> {
    // …
    Ok(())
}
```

### `start_workflow!`

Démarre un workflow par son chemin, en sérialisant les arguments dans
la forme d'enveloppe du runtime de workflow :

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

Voir [Flux de travail](workflows.md).

### `schedule_task!`

Sucre syntaxique autour de `TaskBuilder::from_async` afin qu'une
closure se planifie proprement aux côtés des impls `Task` basés sur
trait :

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

Voir [Planification de tâches](scheduling.md).

## Autorisation

### `#[policy(UserType, ResourceType)]`

Enveloppe un bloc `impl Policy` et enregistre chaque méthode comme une
action de gate nommée. Le nom de la gate combine le nom de la méthode
avec le type de ressource en minuscules - `fn view(...)` sur `Comment`
devient `"view-comment"` :

```rust
use suprnova::policy;

struct CommentPolicy;

#[policy(User, Comment)]
impl CommentPolicy {
    fn view(_user: &User, _comment: &Comment) -> bool { true }
    fn update(user: &User, comment: &Comment) -> bool {
        comment.author_id == user.id
    }
}
```

`Server::run` appelle `authorization::init_policies()`
automatiquement. Voir [Autorisation](authorization.md).

## Notifications et e-mail

### `#[derive(NotificationMailable)]`

Génère automatiquement `to_mail` à partir d'un attribut `#[mail(...)]` -
des templates Tera en ligne ou adossés à un fichier pour le sujet, le
corps HTML, et le corps texte. Vérifications à la compilation : sujet
requis, au moins un corps présent, html/html_template exclusifs,
`from_name` requiert `from` :

```rust
use serde::{Serialize, Deserialize};
use suprnova::NotificationMailable;

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Your order shipped - tracking {{ tracking }}",
    html    = "<p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@suprnova.dev",
)]
pub struct OrderShipped { pub tracking: String }
```

Le trait notification lui-même est implémenté à la main - il n'y a pas
de `#[derive(Notification)]`. Voir [Notifications](notifications.md)
et [E-mail](mail.md).

## Validation

### `validate!`

Point d'entrée de validation synchrone et déclaratif. Chaque ligne
associe un nom de champ à une ou plusieurs valeurs `Rule` (ou
`ContextualRule`), avec `?:` pour « valider seulement si présent » et
`?=>` pour les champs optionnels conditionnellement requis :

```rust
use suprnova::{validate, ValidationErrors};
use suprnova::validation::rules::*;

fn validate_form(self_ref: &SignupForm) -> Result<(), ValidationErrors> {
    validate! { self_ref =>
        email   => Required, Email;
        password => Required, Min(8);
        bio     ?: Max(500);
        card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
    }
}
```

`Validate` est réexporté depuis la crate `validator` - les attributs
`#[validate(...)]` (par ex. `#[validate(email)]`) viennent de
`validator` et s'exécutent via le chemin synchrone de `FormRequest`.
Utilisez `validate!` quand vous avez besoin de règles contextuelles /
inter-champs, de règles async, ou de règles de la palette
`suprnova::validation::rules`. Voir [Validation](validation.md).

## Fabriques

### `#[derive(Factory)]`

Génère un marqueur `<Model>Factory` frère et un impl `Factory` qui
produit des modèles via `fake::Faker`. Le modèle doit implémenter
`fake::Dummy<fake::Faker>` - typiquement via `#[derive(Dummy)]` :

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory existe :
let users = UserFactory::new().count(10).make_many();
```

Voir [Fabriques](eloquent-factories.md).

## Tests

### `#[suprnova_test]`

Enveloppe un test `async fn` avec une base de données SQLite en
mémoire (exécutant `crate::migrations::Migrator` par défaut), invoque
`App::init()` et `App::boot_services()`, et exécute le corps sous
`#[tokio::test]`. Les tests parallèles restent hermétiques grâce à la
couche par thread du conteneur - liez les services spécifiques au test
via `TestContainer::fake` (et non `App::bind`) afin que chaque thread
voie ses propres fakes :

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

Un migrateur personnalisé passe par
`#[suprnova_test(migrator = MyMigrator)]`. Voir [Tests](testing.md).

### `test_database!`

Le constructeur `TestDatabase` en une ligne pour les tests qui ne
prennent pas le paramètre `db` via `#[suprnova_test]` :

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`, `test!`, `expect!`

Regroupement façon Jest + assertions fluides. `describe!` est un
module, `test!` produit un `#[test]` (sync ou async, avec ou sans
paramètre `TestDatabase`), et `expect!` enveloppe une valeur pour des
assertions chaînées avec le contexte fichier/ligne en cas d'échec :

```rust
use suprnova::{describe, test, expect};

describe!("CreateUserAction", {
    test!("creates a user", async fn(db: TestDatabase) {
        let user = CreateUserAction::new()
            .execute("test@example.com").await.unwrap();
        expect!(user.email).to_equal("test@example.com".to_string());
    });
});
```

Voir [Tests](testing.md).

## Middleware

### `global_middleware!`

Enregistre un middleware qui s'exécute sur chaque requête, dans
l'ordre d'enregistrement, avant tout middleware spécifique à une
route. Idempotent par type :

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

Doit s'exécuter avant `Server::from_config` / `Server::new` - le
serveur prend un instantané du registre global au moment de sa
construction. Voir [Middleware](middleware.md).

## Pièges

Une courte liste de modes de défaillance faciles à rencontrer et
faciles à corriger.

### Ordre des attributs - `#[observer]` doit venir avant `#[async_trait]`

```rust
// CORRECT
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// FAUX - émet silencieusement zéro écouteur
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

Les macros d'attribut se développent de l'extérieur vers l'intérieur.
`async_trait` réécrit chaque `async fn` sous une forme
`Pin<Box<dyn Future>>` sans sucre syntaxique. Si elle s'exécute en
premier, la macro observer ne peut plus faire correspondre par nom de
méthode et n'émet rien. La même règle de l'extérieur-vers-l'intérieur
s'applique chaque fois que vous empilez plusieurs macros - placez
l'attribut Suprnova le plus à l'extérieur en cas de doute.

### Le piège de l'impl inhérent

Une méthode `impl` inhérente **ne peut pas** masquer la méthode par
défaut d'un trait via le dispatch de trait. Si vous écrivez une macro
(ou du code à la main) qui définit `fn save(&self)` sur un modèle
comme méthode inhérente, les appels qui passent par le trait `Model`
(`some_model.save()` où le site d'appel ne le connaît que comme
`&dyn Model`) choisiront la valeur par défaut du trait - pas votre
substitution inhérente.

Correction : émettez une substitution au niveau de la méthode du
trait, jamais une méthode inhérente, quand le comportement généré doit
participer au dispatch de trait. C'est pourquoi les macros du
framework (notamment `#[suprnova::model]`) écrivent dans l'impl du
trait. Si vous écrivez vos propres extensions Eloquent à la main,
faites de même.

### `global_middleware!` ne prend effet qu'avant `Server::from_config`

Le serveur prend un instantané du registre global au moment de sa
construction. Appeler `global_middleware!(M)` après
`Server::from_config(...)` ne s'applique pas rétroactivement à ce
serveur. Enregistrez chaque middleware global dans `bootstrap()`,
avant que `Application::run()` n'atteigne l'étape de service.

### `redirect!` et `inertia_response!` sont des vérifications à la compilation

Les deux macros refusent de compiler si la cible nommée n'existe pas -
c'est le but. Si un refactor supprime un nom de route ou de composant,
chaque site d'appel qui le mentionne casse la compilation, ce qui est
exactement ce que vous voulez. Si l'erreur de compilation vous
surprend, cherchez le littéral de chaîne dans votre bloc `routes!` /
répertoire de pages avant de « corriger » l'appel de macro.

### `?:` saute sur `None` ; `?=>` s'exécute même sur `None`

Dans les lignes de `validate!`, `?:` n'exécute les règles que quand le
champ est `Some`. Une règle conditionnelle à la présence comme
`RequiredIf` sur une ligne `?:` ne peut donc jamais échouer sur un
champ absent. Utilisez `?=>` (qui traite l'absence comme `""`) pour le
cas « requis quand X ».

### `#[derive(Validate)]` vient de la crate `validator`, pas de Suprnova

Suprnova réexporte `validator::Validate` afin que vous n'ayez pas de
dépendance directe sur `validator`. Les attributs `#[validate(...)]`
viennent de `validator`. La propre macro `validate!` de Suprnova est
le point d'entrée d'exécution pour les règles inter-champs /
contextuelles ; les deux se complètent mais vivent dans des espaces de
noms différents.

## Pourquoi Suprnova diverge

Laravel découvre les routes, commandes, templates de mail, classes de
modèle, factories, observateurs, et policies à l'exécution - via la
réflexion, l'analyse du système de fichiers, et le dispatch basé sur
des chaînes. PHP rend cela peu coûteux (l'autoloading + l'opcache
amortissent le coût), et l'expérience développeur est excellente :
déposez un fichier dans le bon répertoire et il apparaît.

Ce modèle ne convient pas à Rust. Nous n'avons pas de réflexion à
l'exécution sur les impls de trait, le runtime est un unique binaire
lié statiquement, et les analyses du système de fichiers à l'amorçage
conviennent moins bien à un modèle de processus où chaque binaire sert
des millions de requêtes.

Suprnova fait donc le même travail à la compilation. Les routes sont
vérifiées, les noms de composants sont vérifiés par rapport au
répertoire des pages, les templates de mail sont embarqués via
`include_str!`, les noms de route sont vérifiés pour leur unicité via
l'inventory, les modèles s'enregistrent eux-mêmes dans un inventory
que le framework vide à l'amorçage, de même pour les commandes.
L'expérience développeur est similaire - déposez un fichier, ajoutez
un `#[command]` ou `#[suprnova::model]`, exécutez le binaire - mais le
câblage se produit avant `main` plutôt qu'à la première requête.

Le compromis est que les fautes de frappe, les composants manquants,
et les références cassées sont des erreurs de compilation plutôt que
des erreurs à l'exécution, et qu'il n'y a aucun coût de réflexion par
requête.

## Suivant

- [Routage](routing.md) - expansion complète de `routes!`, nommage, liaison de modèle
- [Contrôleurs](controllers.md) - `#[handler]` et `#[request]` ensemble
- [Eloquent](eloquent.md) - `#[suprnova::model]` et ses compagnons en contexte
- [Validation](validation.md) - `validate!`, règles contextuelles, règles async
- [Console](console.md) - `#[command]` et `#[derive(Command)]` de bout en bout
- [Tests](testing.md) - `#[suprnova_test]`, `expect!`, fakes
