# Depuis Laravel

Si vous avez livré des applications Laravel, vous connaissez déjà 80 % de Suprnova.
Ce chapitre mappe vos habitudes à l'équivalent Rust pour que vous deveniez
productif rapidement. Nous vous montrerons les modèles que vous utilisez
quotidiennement, les modèles qui changent de forme, et les quelques choses que
Rust vous offre gratuitement que PHP ne peut pas faire.

## Résumé côte à côte

| Vous écriviez en Laravel | Vous écrivez en Suprnova |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)` (in `routes!`) |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extracts an `#[derive(Data, Validate)]` arg directly |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | (pas de REPL - écrivez un script ou un test `cargo run` ponctuel) |
| `composer require league/csv` | `cargo add csv` |

## Le changement de modèle mental

### Asynchrone, partout

Le plus grand changement : chaque appel de base de données, appel HTTP, E/S de
fichier, appel de cache, envoi en file d'attente - tout ce qui traverse une
limite - est `async` et vous l'appelez avec `.await?`. Une fois que vous l'avez
fait pendant quelques heures, cela disparaît dans le rythme. Jusque-là, le
compilateur pointera du doigt sur chaque endroit où vous l'avez oublié.

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` est le "retour anticipé sur erreur" de Rust. Un handler retourne
`Result<HttpResponse, HttpResponse>` (aliasé comme `Response`), donc un `?`
sur une erreur BD court-circuite votre convertisseur d'erreur et le client
obtient un 500 correct (ou 4xx, selon le type d'erreur). Vous n'avez presque
jamais à écrire un `try/catch` - `?` le fait.

### Modèles au moment de la compilation

Où Eloquent lit votre schéma BD lors de l'exécution, Suprnova le lit au moment
de la compilation :

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

C'est tout - cette struct EST le modèle Eloquent. Vous obtenez
`Post::find`, `Post::query()`, `Post::create`, `post.update(...)`,
`post.delete()`, suppressions logicielles (avec `#[model(soft_deletes)]`),
horodatages, observateurs, tout. La macro génère une SeaORM
`Entity`, `Model`, `ActiveModel`, et énumération `Column`, et implémente le
trait `Model` de Suprnova - mais vous dépendez de `Post`, pas de ceux-ci.

Si vous renommez une colonne dans une migration, la struct ne correspond plus au
schéma BD - et en fonction de votre configuration, soit le compilateur
le détecte lors de la compilation, soit la conversion de type coercé échoue à la
première requête. Quoi qu'il en soit, vous le découvrez avant la préparation,
pas après.

### Binaire unique

Il n'y a pas de PHP-FPM, pas de config nginx lisant `index.php`, pas de `composer
install` lors du déploiement. `cargo build --release` vous donne un binaire
lié statiquement unique. `scp` le sur un serveur, `systemd` le, c'est fait.
Ou construisez un conteneur - `FROM scratch` fonctionne.

Nous avons des [recettes de déploiement](deployment.md) pour Railway, Digital
Ocean, et Hetzner. La forme commune : construire le binaire, livrer le
binaire, définir les variables d'env, lancer.

## Mapping le framework

### Routage

`routes!` joue le rôle de `routes/web.php` et `routes/api.php`
combinés.

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Groupe de routes avec préfixe et middleware partagés
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // Routage de ressource (le Route::resource de Laravel)
    resource!("posts", controllers::post),
}
```

Référence complète : [Routage](routing.md). Différences à connaître :

- Le middleware de groupe est **aplati** dans la liste des middleware de chaque
  route au moment de l'enregistrement (pas exécuté comme une couche de chaîne
  séparée) - cela signifie qu'il n'y a pas de coût d'exécution supplémentaire pour
  le groupement.
- La syntaxe `{id}` de Laravel et la syntaxe style Rails `:id` fonctionnent
  toutes deux ; elles sont normalisées en interne.
- Les routes nommées se résolvent via `route("posts.show", &[("id", "42")])` et
  il existe une variante d'URL signée pour les liens limités dans le temps.

### Contrôleurs

Un contrôleur est simplement une fonction libre retournant `Response` :

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

Vous pouvez également utiliser la macro `#[handler]` pour extraire les arguments
typés (paramètres de route, requête, corps, la requête elle-même, services du
conteneur) à la signature :

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // La liaison de modèle de route s'est faite automatiquement ; `post` est la ligne chargée.
    json_response!({ "post": post })
}
```

Le type `post::Model` provient du module généré du modèle - c'est le signal que
`#[handler]` utilise pour choisir la liaison de modèle de route plutôt que
l'extraction de requête de formulaire par défaut. Si la ligne n'existe pas, la
liaison retourne un 404 avant que votre code ne s'exécute - même comportement que
la liaison implicite de Laravel.

Les structs d'action (contrôleurs « invokable » à méthode unique, style Laravel)
sont également supportés : voir [Actions](actions.md).

### Eloquent

Le générateur de requêtes avec double API accepte soit les noms de Laravel,
soit les noms idiomatiques de Rust - les deux fonctionnent, choisissez ce qui
se lit clairement au site d'appel.

```rust
// Surface Laravel
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Surface Rust (résultat identique)
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` est le nom du côté Laravel (le `where` nu entre en collision avec le
mot-clé Rust). `filter` est l'alias idiomatique de Rust. Les deux existent ;
les deux font la même chose. Pour les opérateurs non-égalité, utilisez
`db_where_op` (ou son alias `filter_op`) : `.db_where_op("status", "!=", "archived")`.
Voir la [référence Eloquent](eloquent.md) - c'est le chapitre le plus long
pour une raison, la surface est large.

### Authentification

```rust
use suprnova::{Auth, Credentials};

// Dans un handler :
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// Connexion (par exemple dans votre contrôleur de login) :
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// Déconnexion :
Auth::logout().await?;
```

Les protections, les fournisseurs, les sessions, la mémorisation, la
vérification par courrier électronique, la réinitialisation du mot de passe,
l'étranglement par force brute, l'authentification multifacteur TOTP, et OAuth
sont tous ici. La surface des flux d'authentification reflète Laravel Fortify.
La vérification par courrier électronique et la réinitialisation du mot de
passe sont adossées au fournisseur (aucun torii requis) : votre modèle
d'utilisateur implémente `MustVerifyEmail` / `CanResetPassword` - les analogues
de Suprnova des contrats de Laravel de mêmes noms - et le `UserProvider`
configuré pilote les flux. Voir [Authentification](authentication.md)
et [Flux d'authentification](auth-flows.md).

### Migrations

Vous écrivez des migrateurs SeaORM. La forme semblera familière même si la
syntaxe est nouvelle :

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` scaffolde le fichier.
`suprnova migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`
font tous ce que vous attendez. `suprnova db:sync` exécute les migrations et
régénère les entités SeaORM contre lesquelles la couche macro compile.
Voir [Migrations](migrations.md).

### Files d'attente et planification

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// Définit un job - les données vivent sur la struct, le contrat vit sur
// `impl Job`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// Poussez-le dans la file d'attente :
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// Ou avec un délai :
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

Les workers s'exécutent avec `cargo run -- queue:work`. Les drivers
incluent la mémoire et la synchronisation (en processus, pour les tests), la
base de données, redis, et null. Les lots, les chaînes, les tâches uniques,
les tentatives, la sauvegarde progressive, le middleware, l'ensemble des
tâches échouées - tout est là. Voir [Files d'attente](queues.md).

La planification utilise le trait `Task` et le binaire de planificateur par
projet :

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// Enregistrez-la dans bootstrap (par exemple via Schedule::call / .task / .add) :
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

Voir [Planification de tâches](scheduling.md).

### E-mail, notifications, diffusion

Ceux-ci suivent Laravel un par un. `Mailable` est une macro de dérivation ;
`Notifiable` est un trait sur votre modèle User ; les canaux sont
`mail`/`database`/`broadcast`/`webpush` ; la diffusion supporte
les canaux publics, privés et de présence. Voir [E-mail](mail.md),
[Notifications](notifications.md), [Diffusion](broadcasting.md).

### Frontend

Il n'y a pas de Blade. Au lieu de cela, le frontend est une vraie SPA via
Inertia.js, et vous passez des props typés de Rust :

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` est un composant Svelte (ou React, ou Vue - votre démarrage
choisit). Les types TypeScript pour les props sont générés automatiquement à
partir de la dérivation `InertiaProps` - lancez `suprnova generate-types`
après l'ajout d'une nouvelle struct prop et le frontend obtient les liaisons
typées.

Si vous avez utilisé Inertia dans Laravel via `inertia()`, c'est la même
chose - juste typée bout en bout. Voir la [Présentation du frontend](frontend.md).

## Les choses qui changent de forme

Quelques choses se déplacent différemment dans Suprnova. Aucune d'elles n'est
un bloqueur, mais elles méritent d'être connues d'avance.

### Aucun fournisseur de service

Laravel a des douzaines de fournisseurs de service enregistrant des liaisons,
des observateurs, des compositeurs de vue, etc. Suprnova a **une** fonction
bootstrap unique dans le `bootstrap.rs` de votre application. Vous enregistrez
tout là, dans l'ordre. Ce n'est pas élégant mais c'est transparent - vous pouvez
voir en 30 lignes exactement ce que votre application amorce.

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

Les chapitres [Conteneur](container.md) et [Amorçage](bootstrap.md)
ont les détails.

### La configuration est typée

Où Laravel utilise `config('app.timezone')` retournant ce que le tableau dit,
Suprnova a des structs de configuration typées :

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // &str, pas mixed
```

Vous pouvez enregistrer vos propres sections de configuration typées. Voir
[Configuration](configuration.md).

### Aucune façade-comme-alias

Les façades Laravel comme `DB::` sont des alias de classe configurés dans
`config/app.php`. Les façades de Suprnova sont de vrais modules à la racine
de la crate :

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

Même surface, aucun alias global nécessaire.

### Les temps de compilation sont réels

Les temps de compilation de Rust ne sont pas PHP. Une construction propre d'une
application Suprnova nouvelle prend 1–2 minutes ; les constructions
incrémentielles pendant le développement durent quelques secondes. Le flux de
travail de dev est le même - `suprnova serve` surveille les changements et
reconstruit - mais vous le ressentirez la première fois que vous changerez une
macro et recompilerez une crate en aval. La mise en cache se rembourse rapidement.

### L'emprunteur vérificateur existe

La plupart des contrôleurs et des handlers ne touchent jamais une annotation de
durée de vie - les signatures du framework les cachent. Quand le vérificateur
d'emprunt vous crie dessus, c'est généralement parce que vous avez essayé de
tenir une référence à travers un `.await` qui franchissait un mutex ou teniez
une transaction BD à travers un appel attendu qui avait besoin d'accès exclusif.
Les erreurs sont claires et les correctifs sont généralement `.clone()` ou
restructurer-en-portées-plus-petites.

### Pas de REPL `tinker`

Il n'y a pas de REPL. L'équivalent le plus proche est un script `cargo run`
unique dans `examples/`, ou un test `#[suprnova_test]` qui exerce la chose que
vous déboguez. La plupart de ce que vous feriez dans tinker (examiner un modèle,
déclencher une notification, envoyer une tâche) est un test de 5 lignes.

## Où les chapitres de Laravel atterrissent

Recherche rapide si vous savez ce que vous cherchez mais pas où ça se trouve :

| Sujet Laravel | Chapitre Suprnova |
|---|---|
| Cycle de vie | [Cycle de vie des requêtes](lifecycle.md) |
| Conteneur de service | [Conteneur de service](container.md) |
| Fournisseurs de service | [Amorçage de l'application](bootstrap.md) |
| Façades | [Conteneur de service](container.md) |
| Routage | [Routage](routing.md) |
| Middleware | [Middleware](middleware.md) |
| Protection CSRF | [CSRF](csrf.md) |
| Contrôleurs | [Contrôleurs](controllers.md) |
| Requêtes | [Requêtes](requests.md) |
| Réponses | [Réponses](responses.md) |
| Génération d'URL | [Génération d'URL](urls.md) |
| Session | [Sessions](session.md) |
| Validation | [Validation](validation.md) |
| Gestion des erreurs | [Gestion des erreurs](errors.md) |
| Journalisation | [Journalisation](logging.md) |
| Console Artisan | [Console](console.md) + [Référence CLI](cli.md) |
| Diffusion | [Diffusion](broadcasting.md) |
| Cache | [Cache](cache.md) |
| Événements | [Événements](events.md) |
| Stockage de fichiers | [Système de fichiers et stockage](filesystem.md) |
| Client HTTP | [Client HTTP](http-client.md) |
| Localisation | [Localisation](localization.md) - Catalogues Fluent `.ftl`, pas des tableaux PHP |
| E-mail | [E-mail](mail.md) |
| Notifications | [Notifications](notifications.md) |
| Files d'attente | [File d'attente](queues.md) |
| Limitation de débit | [Limitation de débit](rate-limiting.md) |
| Planification de tâches | [Planification de tâches](scheduling.md) |
| Authentification | [Authentification](authentication.md) |
| Autorisation | [Autorisation](authorization.md) |
| Vérification du courrier | [Flux d'authentification](auth-flows.md) |
| Réinitialisation du mot de passe | [Flux d'authentification](auth-flows.md) |
| Chiffrement | [Chiffrement](encryption.md) |
| Hachage | [Hachage](hashing.md) |
| Base de données | [Base de données](database.md) |
| Générateur de requêtes | [Générateur de requêtes](queries.md) |
| Pagination | [Pagination](pagination.md) |
| Migrations | [Migrations](migrations.md) |
| Ensemencement | [Ensemencement](seeding.md) |
| Eloquent | [Eloquent API](eloquent.md) |
| Eloquent : Relations | [Relations Eloquent](eloquent-relationships.md) |
| Eloquent : Collections | [Collections Eloquent](eloquent-collections.md) |
| Eloquent : Mutateurs / Casts | [Eloquent - Casts, accesseurs et mutateurs](eloquent-mutators.md) |
| Eloquent : Ressources API | [Ressources JSON:API](eloquent-resources.md) |
| Eloquent : Sérialisation | [Sérialisation Eloquent](eloquent-serialization.md) |
| Eloquent : Fabriques | [Fabriques Eloquent](eloquent-factories.md) |
| Tests | [Tests](testing.md) |
| Tests HTTP | [Tests HTTP](http-tests.md) |
| Tests de base de données | [Tests de base de données](database-testing.md) |
| Mocking | [Mocking et doublures](mocking.md) |
| Cashier (Stripe) | [Paiements - Adaptateur Stripe](payments-stripe.md) |
| Cashier (Paddle) | [Paiements - Adaptateur Paddle](payments-paddle.md) |
| Sanctum / Passport | (pas encore - authentification par token via intégration torii) |
| Horizon | (pas encore - introspection de file d'attente intégrée) |
| Telescope / Pulse | (reporté à v2+) |

Choses que Laravel a que Suprnova n'a pas (encore) :

- Telescope / Pulse (surface d'observabilité) - l'[observabilité](observability.md) de base est livrée, pas les tableaux de bord
- Authentification par token Sanctum / Passport - l'intégration torii couvre OAuth et l'authentification de session ; l'authentification par token dédiée est prévue, non livrée
- Horizon - l'introspection de file d'attente est intégrée au framework, pas de tableau de bord séparé
- Blade - de par la conception ; Inertia est l'histoire du frontend
- `trans_choice` - [Localisation](localization.md) est livrée, mais les pluriels sont
  sélectionnés à l'intérieur du message par catégorie CLDR plutôt que par les
  plages d'entiers de style `[1,19]` que `trans_choice` accepte

## Suivant

- [Installation](installation.md) - mettre en place un projet
- [Démarrage rapide](quickstart.md) - construire une petite application en 5 minutes
- [Routage](routing.md) - le chapitre naturellement suivant à partir d'ici

Ou sautez n'importe où via [`documentation.md`](documentation.md).
