# Structure des répertoires

Quand vous exécutez `suprnova new my-app --frontend svelte`, le scaffolder
vous donne ceci :

```
my-app/
├── Cargo.toml                      # manifeste de crate + dépendances, deux cibles [[bin]]
├── .env                            # config locale - URL BD, clé app, ports
├── .env.example                    # modèle pour ops/CI
├── .gitignore                      # exclut target/, .env, node_modules/, public/assets/
├── cmd/
│   └── main.rs                     # entrée binaire ; appelle Application::new().run()
├── src/
│   ├── lib.rs                      # câblage des modules (`pub mod controllers;` etc.)
│   ├── bootstrap.rs                # enregistre services, observateurs, écouteurs -
│   │                               # l'équivalent Suprnova des service providers de Laravel
│   ├── routes.rs                   # l'arbre des macros `routes!` - toutes les URLs servies
│   ├── bin/
│   │   └── console.rs              # entrée `cargo run --bin console <subcommand>` -
│   │                               # l'équivalent Suprnova de `php artisan`
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # contrôleurs invocables à une seule méthode
│   ├── commands/
│   │   └── mod.rs                  # les handlers avec `#[command]` s'enregistrent ici
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # config BD typée (driver, URL, pool)
│   │   └── mail.rs                 # config e-mail typée
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # handler GET /
│   │   ├── auth.rs                 # login / register / logout
│   │   └── dashboard.rs            # requiert auth ; route protégée d'exemple
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # journalisation requête/réponse
│   │   └── authenticate.rs         # guard auth basé session
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # modèle User `#[suprnova::model]`
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # entrée Vite ; monte le SPA
│   └── src/
│       ├── main.{tsx,ts}           # config client Inertia (par-framework)
│       ├── app.css                 # styles globaux + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # généré auto à partir de #[derive(InertiaProps)]
└── public/
    └── assets/                     # sortie build Vite en production
```

Svelte ajoute `frontend/svelte.config.js` et `frontend/src/app.d.ts`.
Vue ajoute `frontend/src/shims-vue.d.ts`.

Le démarrage API (`suprnova new my-api --api`) est plus léger : pas de
`frontend/`, pas de contrôleurs auth, et `cmd/main.rs` est remplacé par
`src/main.rs`.

## À quoi sert chaque répertoire

### `cmd/main.rs`

Le point d'entrée du binaire. Un fichier court - typiquement 10-20 lignes -
qui appelle le pipeline d'amorçage standard :

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

`Application::run()` analyse la CLI du binaire (`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`), charge
`.env`, exécute votre fonction de config, puis envoie la sous-commande. Le
chemin serve exécute aussi votre fonction bootstrap et démarre le serveur
HTTP.

Vous n'éditez presque jamais `cmd/main.rs` après le scaffolding initial.

### `src/lib.rs`

Un fichier de déclaration de modules plat :

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

C'est ce qui rend `crate::controllers::home::index` accessible depuis
`routes.rs`.

### `src/bootstrap.rs`

La fonction unique qui câble votre app. Vous enregistrez les bindings du
conteneur de service, les observateurs, les écouteurs d'événements, les
middlewares personnalisés, et tout autre setup au démarrage. C'est
l'équivalent du `AppServiceProvider`, `EventServiceProvider`,
`BroadcastServiceProvider` de Laravel, tout dans un seul fichier :

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // Lier un service dans le conteneur
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // Enregistrer un observateur Eloquent
    crate::models::user::register_observer();

    // Écouter les événements
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` s'exécute une fois par processus, après le chargeur de config
mais avant que `serve` accepte la première requête. Les workers
(`queue:work`, `schedule:run`, `workflow:work`) réutilisent le même
bootstrap pour voir les mêmes services. Voir [Amorçage de l'application](bootstrap.md).

### `src/routes.rs`

Votre surface d'URLs. La macro `routes!` au niveau du module se développe en
une `pub fn register() -> Router` que `cmd/main.rs` transmet à
`Application::routes(...)` :

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Auth (enregistrement + protégé)
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // Le dashboard requiert le middleware authenticate
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

Voir [Routage](routing.md).

### `src/bin/console.rs`

Votre binaire console par-projet. S'exécute comme `cargo run --bin console
<subcommand>` et envoie le built-in `db:seed` du framework plus chaque
handler avec `#[command]` (ou struct typée `#[derive(Command)]`) dans
`src/commands/` - les deux formes s'enregistrent via inventory à la
compilation :

```bash
cargo run --bin console db:seed           # built-in du framework
cargo run --bin console report:daily      # votre commande personnalisée
```

Les workers de longue durée (`queue:work`, `schedule:run`,
`schedule:work`, `workflow:work`) vivent sur le binaire app principal
parce que `Application::run()` les envoie - appelez-les comme
`cargo run -- queue:work` (ou via `suprnova schedule:run` /
`suprnova workflow:work` si vous préférez la CLI parapluie).

Voir [Console](console.md).

### `src/commands/`

Où vivent vos handlers de console. Deux saveurs : une struct typée avec
les args dérivés de clap et `impl TypedCommand`, ou un `#[command]` brut sur
une `async fn(Vec<String>) -> Result<(), FrameworkError>`. Le scaffolder
génère la forme typée :

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Generate the daily report")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` scaffold le fichier et l'ajoute à
`src/commands/mod.rs`. Voir [Console](console.md).

### `src/config/`

Structs de configuration typées. Le scaffold fournit `database.rs` et
`mail.rs` ; ajoutez les vôtres pour tout sous-système qui intéresse votre
app. Chaque struct config lit ses valeurs de l'environnement, et
`config::register_all()` les enregistre auprès du framework :

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

Câblez-le dans `config/mod.rs` :

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

Voir [Configuration](configuration.md).

### `src/controllers/`

Fonctions handler HTTP. Un module par ressource. Chaque `pub async fn`
qui prend une `Request` et retourne une `Response` est callable depuis une
route.

### `src/middleware/`

Implémentations de middleware. Le scaffold fournit `logging` et
`authenticate` ; vous ajoutez les vôtres ici sous la forme `pub struct Foo`
avec `impl Middleware for Foo`. Enregistrez-les globalement dans
`bootstrap.rs` ou appliquez-les par-route via `.middleware(…)` dans l'arbre
`routes!`. Voir [Middleware](middleware.md).

### `src/migrations/`

Migrateurs SeaORM. Le scaffold fournit une poignée pour les tables auth +
workflow. `suprnova make:migration <name>` en ajoute une nouvelle. `suprnova
migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`,
`db:sync` opèrent tous sur ce répertoire. Voir [Migrations](migrations.md).

### `src/models/`

Vos modèles Eloquent. Un fichier par modèle, chacun une struct
`#[suprnova::model]`. Le scaffold fournit `user.rs` ; ajoutez de nouveaux
modèles en écrivant un nouveau fichier à la main ou en exécutant
`suprnova db:sync --regenerate-models` après une migration de schéma. Voir
[Eloquent](eloquent.md).

### `src/actions/`

Contrôleurs invocables à une seule méthode. Motif optionnel - utilisez-les
quand un contrôleur n'aurait qu'une seule méthode et que vous préféreriez
l'appeler « Action » plutôt que de le wrapper. Le scaffold fournit un
exemple que vous pouvez supprimer ou adapter. Voir [Actions](actions.md).

### `frontend/`

Le SPA Vite + Inertia. C'est un projet frontend normal - `package.json`,
`vite.config.ts`, `tsconfig.json`, une entrée `index.html` Vite, source sous
`src/`. La config du client Inertia vit dans `src/main.{tsx,ts}` et les
composants de page dans `src/pages/`. Les types TypeScript pour vos props Rust
`#[derive(InertiaProps)]` sont régénérés dans `src/types/inertia-props.ts` par
`suprnova generate-types`.

Voir [Présentation du frontend](frontend.md).

### `public/assets/`

Où Vite dépose le build de production (`npm run build`). Le serveur Suprnova
sert ce répertoire sous la forme d'assets statiques à `/assets/*` en
production.

## Répertoires que vous ajouterez au fur et à mesure

Le scaffold vous donne le minimum - assez pour livrer le flux de bienvenue
et un dashboard protégé. Les vraies apps ajoutent plus de sous-systèmes.
Ajouts courants :

| Répertoire | Quand vous l'ajoutez |
|---|---|
| `src/jobs/` | Première fois où vous `Queue::push(SomeJob)`. Voir [Files d'attente](queues.md). |
| `src/listeners/` | Première fois où vous `Event::listen`. Voir [Événements](events.md). |
| `src/observers/` | Première fois où vous implémentez `Observer<MyModel>`. Voir [Eloquent](eloquent.md#observers). |
| `src/notifications/` | Première fois où vous implémentez une `Notification`. Voir [Notifications](notifications.md). |
| `src/mail/` | Première fois où vous implémentez une `Mailable`. Voir [E-mail](mail.md). |
| `src/policies/` | Première fois où vous écrivez un `#[policy]`. Voir [Autorisation](authorization.md). |
| `src/factories/` | Première fois où vous écrivez une `Factory<Model>` pour les tests. Voir [Fabriques Eloquent](eloquent-factories.md). |
| `src/seeders/` | Première fois où vous écrivez un `Seeder` pour `db:seed`. Voir [Ensemencement](seeding.md). |
| `src/events/` | Première fois où vous `impl Event` pour votre propre type d'événement. Voir [Événements](events.md). |
| `src/broadcasting/` | Première fois où vous définissez un `Channel` privé/de présence. Voir [Diffusion](broadcasting.md). |
| `src/ws/` | Première fois où vous écrivez un handler `ws!()`. Voir [WebSockets](websockets.md). |
| `src/supervisors/` | Première fois où vous implémentez un `Supervisor` de longue durée. Voir [Superviseurs](supervisors.md). |
| `src/payments/` | Première fois où vous câblez Stripe/Paddle pour votre app. Voir [Paiements](payments.md). |
| `src/props/` | Quand vous voulez garder les structs `#[derive(InertiaProps)]` séparées des contrôleurs. |
| `resources/views/` | Première fois où vous ajoutez un template Tera pour les corps d'e-mail. |
| `storage/` | Première fois où vous écrivez des fichiers sur le disque du système de fichiers local (voir [Système de fichiers et stockage](filesystem.md)). |
| `tests/` | Première fois où vous écrivez un test d'intégration. |

Vous n'avez pas besoin de demander la permission - `mkdir src/jobs` et ajoutez
`pub mod jobs;` à `src/lib.rs`, et c'est fait. Le framework n'applique pas
les noms de répertoires ; les conventions existent pour que les autres
développeurs Suprnova trouvent les choses rapidement.

## L'app dogfood `app/` dans ce repo

Si vous lisez ceci depuis le repo Suprnova lui-même, vous verrez un
répertoire `app/` à la racine qui utilise chaque fonctionnalité du framework
ensemble. C'est notre banc d'essai interne - il teste les paiements, la
diffusion, les web push, les workflows, les superviseurs, etc. tout à la
fois. Ce n'est PAS une référence propre pour une nouvelle app ; la sortie du
scaffold ci-dessus est délibérément plus petite et plus facile à apprendre.
Lisez `app/` quand vous voulez voir un exemple maximal de comment les pièces
se composent.

## Suivant

- [Configuration](configuration.md) - comment `.env` devient config typée
- [Amorçage de l'application](bootstrap.md) - ce que `bootstrap.rs` fait
  réellement
- [Routage](routing.md) - votre première route
- [Conteneur de service](container.md) - comment `App::bind` et `App::get`
  fonctionnent
