# Générateurs de code

La famille `suprnova make:*` scaffolde le fichier conventionnel pour
chaque pièce d'un projet - un contrôleur, une action, un middleware,
une commande console, une erreur de domaine, une tâche planifiée,
une page Inertia ou une struct de props, une migration de base de
données - et câble le nouveau module dans son `mod.rs` parent (et là
Faites-y appel quand vous devriez sinon retaper le même boilerplate + la ligne d'import `pub mod x;`, ce qui est le cas la plupart du temps.


## make:controller

Scaffolde un contrôleur - un fichier dans `src/controllers/` avec une
seule fn async `#[handler]` nommée `invoke`.

```bash
suprnova make:controller User
suprnova make:controller order_item
```

Le nom est normalisé en `snake_case` pour le nom de fichier et
utilisé tel quel pour l'écho `controller:` dans la réponse. Seules
les lettres ASCII, les chiffres, et `_` sont acceptés - les chemins
comme `api/User` sont rejetés.

### Fichier généré

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

### Ce qu'elle câble

1. Écrit `src/controllers/<name>.rs` avec la fn `#[handler]`.
2. Ajoute `pub mod <name>;` à `src/controllers/mod.rs` (crée le
   fichier s'il n'existait pas).
3. Affiche une astuce pour ajouter une route dans `src/routes.rs` :
   `.get("/<name>", controllers::<name>::invoke)`.

Voir [Contrôleurs](controllers.md) pour le contrat de handler, les
extracteurs, et la macro `routes!`.

---

## make:action

Scaffolde une action à responsabilité unique - une struct résolvable
par le conteneur avec une méthode async `execute` qui retourne un
`Result<String, FrameworkError>` pour que le squelette compile avant
que vous ne remplissiez le corps.

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

Le nom est mis en PascalCase ; un suffixe `Action` est ajouté s'il
manque, et le fichier est le nom de struct en snake_case.

### Fichier généré

```rust
// src/actions/create_user_action.rs
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct CreateUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl CreateUserAction {
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("CreateUserAction executed".to_string())
    }
}
```

### Ce qu'elle câble

1. Écrit `src/actions/<snake>.rs`.
2. Ajoute `pub mod <snake>;` à `src/actions/mod.rs`.
3. `#[injectable]` enregistre l'action auprès du conteneur au moment
   du link, si bien que n'importe quel contrôleur peut la résoudre
   via `App::get::<CreateUserAction>()` et appeler
   `action.execute().await?`.

Voir [Actions](actions.md) pour le motif résoudre-puis-invoquer et
comment les actions se composent avec le conteneur.

---

## make:middleware

Scaffolde un middleware - une unit struct qui implémente
`suprnova::Middleware`. Le corps par défaut chronomètre le handler
interne et journalise les événements entrant + sortant avec l'id par
requête, si bien qu'il s'exécute bout en bout dès la première fois.

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

Le nom est mis en PascalCase ; un suffixe `Middleware` est ajouté
s'il manque. Le fichier utilise le nom de base en snake_case (sans le
suffixe), par ex. `Auth` → `src/middleware/auth.rs`, struct
`AuthMiddleware`.

### Fichier généré

```rust
// src/middleware/auth.rs
use std::time::Instant;

use suprnova::{async_trait, current_request_id, Middleware, Next, Request, Response};

pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.path().to_string();
        let request_id = current_request_id()
            .map(|id| id.as_str().to_string())
            .unwrap_or_default();
        let started_at = Instant::now();

        println!(
            "[AuthMiddleware] --> {} {} (request_id={})",
            method, path, request_id,
        );

        let response = next(request).await;

        println!(
            "[AuthMiddleware] <-- {} {} ({} ms, request_id={})",
            method, path, started_at.elapsed().as_millis(), request_id,
        );

        response
    }
}
```

### Ce qu'elle câble

1. Écrit `src/middleware/<snake>.rs`.
2. Ajoute `mod <snake>;` + `pub use <snake>::<StructName>;` à
   `src/middleware/mod.rs` (le crée si nécessaire).
3. Affiche à la fois la forme par route
   (`.get("/path", handler).middleware(AuthMiddleware)`) et la forme
   globale (`global_middleware!(middleware::AuthMiddleware)` dans
   `bootstrap.rs`).

Voir [Middleware](middleware.md) pour la sémantique complète de la
chaîne, l'ordre, et la distinction global vs par route.

---

## make:command

Scaffolde une commande console - une struct
`#[derive(clap::Parser, Command)]` que le binaire `console` par
projet récupère via `inventory` au moment du link. Le corps par
défaut est un `println!("…: not yet implemented")` pour que la
commande s'exécute immédiatement.

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

Le nommage suit trois règles :

- Les entrées contenant `:` sont utilisées telles quelles comme nom
  de commande enregistré (style d'espace de noms à la Laravel :
  `db:seed`, `mail:send`).
- Sinon, le nom de fn en snake_case est kebabbé pour le nom
  enregistré (`CleanCache` → commande `clean-cache`).
- Le fichier et la struct Rust sont toujours des formes snake_case /
  PascalCase du même identifiant.

### Fichier généré

```rust
// src/commands/clean_cache.rs
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "clean-cache", description = "TODO: describe what clean-cache does")]
pub struct CleanCache {
    // Add clap-derive args here.
}

#[async_trait]
impl TypedCommand for CleanCache {
    async fn run(self) -> Result<(), FrameworkError> {
        println!("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### Ce qu'elle câble

1. Écrit `src/commands/<snake>.rs`.
2. Ajoute `pub mod <snake>;` à `src/commands/mod.rs` (le crée si
   nécessaire).
3. Avertit de manière visible si `src/lib.rs` manque `pub mod
   commands;` - la commande ne se liera pas dans le binaire console
   sans cela.
4. Affiche la commande d'exécution :
   `cargo run --bin console -- clean-cache`.

Voir [Console](console.md) pour la surface complète des commandes
typées, le raccourci `#[command]` pour les handlers argv seul, et le
rôle du binaire console par projet.

---

## live:make

Génère un composant Live : un îlot appartenant au serveur dont les actions typées
arrivent par le protocole Live et dont la vue re-rendue est morphée sur place par
le runtime navigateur livré.

```bash
suprnova live:make Counter
suprnova live:make todo-list
suprnova live:make Counter --dry-run
```

Les noms doivent être de simples identifiants ASCII sous l'une des formes
`Counter`, `TodoList`, `todo-list` ou `todo_list` ; le fichier et le module sont en
snake_case, la struct en PascalCase et le nom de composant enregistré est
`<package>.<kebab>` (pour un paquet nommé `demo-app` : `demo-app.counter`). Les
mots-clés Rust, les séparateurs, les points et les entrées non ASCII sont rejetés
avant toute écriture.

### Fichier généré

```rust
// src/live/counter.rs
use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "demo-app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

```html
<!-- templates/live/counter.html -->
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

### Ce qui est câblé

1. Valide d'abord chaque chemin cible et refuse la traversée et les liens
   symboliques ; si le fichier du composant ou la vue existe déjà, il avertit et
   n'écrit rien du tout.
2. Écrit `src/live/<snake>.rs` et `templates/live/<snake>.html` de façon atomique ;
   si une écriture échoue, chaque fichier créé ou modifié par l'exécution est annulé.
3. Insère `pub mod <snake>;` et `.register::<snake::Pascal>()?` dans le builder
   `registry()` de `src/live/mod.rs`. Tout projet créé par `suprnova new` livre ce
   module avec un registre vide, une fonction `routes()` qui installe les routes Live
   réservées gardées et un bootstrap qui lie le registre ; un projet plus ancien
   obtient le même module au premier usage.
4. Ajoute `pub mod live;` à `src/lib.rs` lorsqu'il manque.
5. Affiche la ligne de bootstrap qui lie le registre, puis la commande de
   vérification : `suprnova live:check`.

Dans un projet antérieur au module Live, liez le registre pendant le bootstrap et
installez les routes depuis `cmd/main.rs` à la main :

```rust
suprnova::App::singleton(crate::live::registry().expect("Live registry"));
```

```rust
.try_routes(|| live::routes(routes::register()))
```

---

## make:error

Scaffolde une erreur de domaine - une unit struct annotée avec
`#[domain_error]` pour qu'elle porte un statut HTTP, un message
`Display`, et un impl `From<…> for FrameworkError` prêt à l'emploi.

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

Le nom est mis en PascalCase pour la struct et en snake_case pour le
fichier. Le statut par défaut est 500 et le message est le nom de
struct en casse de phrase - changez les deux attributs dans le
fichier généré pour correspondre à la situation.

### Fichier généré

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

Changez `status = 500` pour ce qui convient - `404` pour introuvable,
`402` pour paiement requis, `403` pour interdit - et éditez la chaîne
de message. Pour des payloads plus riches, ajoutez des champs nommés
à la struct et référencez-les dans le message via interpolation dans
un impl `Display` écrit à la main (abandonnez la macro
`#[domain_error]` à ce stade).

### Ce qu'elle câble

1. Écrit `src/errors/<snake>.rs`.
2. Ajoute `pub mod <snake>;` à `src/errors/mod.rs` (le crée si
   nécessaire).
3. Avertit à propos de déclarer `mod errors;` dans `src/lib.rs` si le
   répertoire `errors/` a été créé à neuf.

### L'utiliser

À l'intérieur d'un handler qui retourne `Response`, élevez le type de
domaine en `FrameworkError` pour que `?` court-circuite proprement :

```rust
use crate::errors::user_not_found::UserNotFound;
use suprnova::FrameworkError;

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;
    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;
    json_response!({ "user": user })
}
```

Le chapitre [Gestion des erreurs](errors.md) couvre l'histoire
complète des erreurs personnalisées, y compris quand utiliser
`#[domain_error]` vs `AppError::bad_request(…)` vs un impl `HttpError`
écrit à la main.

---

## make:task

Scaffolde une tâche planifiée - une unit struct qui implémente
`suprnova::Task` et affiche des lignes structurées de début/fin, si
bien que le scaffold journalise la progression avant que vous ne
remplissiez le vrai corps.

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

Le nom est mis en PascalCase ; un suffixe `Task` est ajouté s'il
manque. Le fichier est le nom de struct en snake_case, par ex.
`CleanupLogs` → `src/tasks/cleanup_logs_task.rs`.

### Fichier généré

```rust
// src/tasks/cleanup_logs_task.rs
use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

### Ce qu'elle câble

La première invocation de `make:task` fait un câblage plus lourd que
les autres générateurs - elle crée la surface du planificateur dans
le projet depuis zéro :

1. Crée `src/tasks/` et `src/tasks/mod.rs` s'ils manquent.
2. Crée `src/schedule.rs` (le point d'entrée
   `register(schedule: &mut Schedule)`) s'il manque.
3. Déclare `pub mod schedule;` et `pub mod tasks;` dans `src/lib.rs`.
4. Insère `.schedule(<crate>::schedule::register)` dans la chaîne
   `Application::new()` de `cmd/main.rs` ou `src/main.rs`,
   immédiatement avant `.run()`.
5. Écrit `src/tasks/<snake>.rs` et l'ajoute à `src/tasks/mod.rs`.

Les invocations suivantes ignorent les étapes déjà exécutées.

### Enregistrer la tâche

Ouvrez `src/schedule.rs` et ajoutez un appel d'enregistrement avec
l'API de planification fluide :

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes old log files daily"),
    );
}
```

Puis exécutez le planificateur :

```bash
suprnova schedule:work   # daemon - vérifie chaque minute
suprnova schedule:run    # ponctuel - typiquement appelé par cron
suprnova schedule:list   # affiche chaque tâche enregistrée
```

Voir [Planification](scheduling.md) pour la surface complète de
tâche (`hourly`, `weekly`, `cron(...)`, `between`, `when`,
`without_overlapping`, la gestion des fuseaux horaires) et [Commandes
de planification](cli-scheduling.md) pour l'arbitrage entre
l'exécution en cron et l'exécution en daemon.

---

## make:inertia

Scaffolde soit un composant de page Inertia (par défaut), soit une
struct Data typée (`--data`), selon le flag. Le générateur de page
détecte le framework frontend (Svelte 5, React 19, Vue 3.5) depuis
`.env` et émet l'extension de fichier correspondante.

### Mode page (par défaut)

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

Le nom est mis en PascalCase et le suffixe `Page` est ajouté s'il
manque, donc `About` → `AboutPage`. Le fichier atterrit dans
`frontend/src/pages/` avec l'extension par frontend : `AboutPage.svelte`
pour Svelte, `AboutPage.tsx` pour React, `AboutPage.vue` pour Vue.

Exemple (Svelte) :

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

Rendez-le depuis un contrôleur :

```rust
inertia_response!(&req, "AboutPage", props)
```

Voir [Composants de page](frontend-pages.md) et [Réponses
Inertia](frontend-inertia-responses.md) pour le pont entre les
contrôleurs et les pages, les rechargements partiels, et les props
partagées.

### Mode struct de données (`--data`)

```bash
suprnova make:inertia UserProps --data
```

Émet une struct `#[derive(Data, Validate)]` dans `app/src/props/`
(pas `src/props/` - le préfixe `app/` est codé en dur pour que le
fichier atterrisse dans l'app hôte/exemple de l'espace de travail) :

```rust
// app/src/props/user_props.rs
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserProps {
    pub id: i64,
    // Add fields here.
    //
    // Available field attributes:
    //   #[data(input_only)] - accepted on Deserialize, omitted from Serialize
    //   #[data(output_only)] - rejected on Deserialize, included in Serialize
    //   #[data(allow_include)] - registers as ?include=-eligible (default-deny)
    //
    // For PATCH endpoints, use suprnova::data::Field<T> to distinguish
    // absent from null. For lazy outbound fields, use suprnova::inertia::Prop<T>.
}
```

Utilisez-la dans un contrôleur pour valider les corps de requête :

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

Scaffolde un fichier de migration SeaORM horodaté. Couvert en détail
dans [Migrations CLI](cli-migrations.md), qui parcourt aussi les
commandes `migrate` / `migrate:rollback` / `migrate:status` /
`migrate:fresh` / `db:sync`. La forme courte :

```bash
suprnova make:migration create_users_table
```

Le nom de la migration est préservé tel quel et préfixé avec un
timbre `YYYYMMDDHHMMSS_` pour que les fichiers se trient
chronologiquement. Le fichier généré atterrit dans `migrations/`.

Voir [Migrations](migrations.md) pour la surface du schema-builder et
[Tests de base de données](database-testing.md) pour le motif
`TestDatabase::fresh` qui exécute des migrations contre une base de
données isolée par test.

---

## generate-types

Émet des interfaces TypeScript depuis chaque struct Rust annotée avec
`#[derive(InertiaProps)]`. Le serveur de dev exécute ceci
automatiquement ; la commande autonome est pour les checks CI et les
régénérations ponctuelles.

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| Option | Par défaut | Description |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | Chemin du fichier de sortie |
| `-w, --watch` | désactivé | Surveille les fichiers source et régénère au changement |

```bash
# Ponctuel
suprnova generate-types

# Mode surveillance (utile quand vous ne voulez pas exécuter le serveur de dev complet)
suprnova generate-types --watch

# Chemin de sortie personnalisé
suprnova generate-types --output frontend/src/types/props.ts
```

Une forme Rust à gauche produit une interface TypeScript à droite :

```rust
#[derive(InertiaProps)]
pub struct UserPageProps {
    pub user: User,
    pub posts: Vec<Post>,
}
```

```typescript
export interface UserPageProps {
    user: User;
    posts: Post[];
}
```

Voir [Types TypeScript](frontend-typescript-types.md) pour la table
de correspondance complète (enums, options, dates, structs
imbriquées) et les hooks de redéfinition.

---

### Pourquoi Suprnova diverge

Le `php artisan make:*` de Laravel dépose un fichier dans le bon
répertoire et c'est tout - l'autoloading PSR-4 récupère la nouvelle
classe la prochaine fois que le framework démarre. Rust n'a pas
d'équivalent. Un fichier à `src/foo/bar.rs` n'est pas compilé dans la
crate jusqu'à ce que `src/foo/mod.rs` déclare `pub mod bar;`, et le
répertoire parent doit être câblé de la même façon dans `src/lib.rs`.

Donc chaque générateur `suprnova make:*` fait deux choses au lieu
d'une : il écrit le nouveau fichier *et* édite le `mod.rs` le plus
proche (et, pour `make:task` et `make:command`, `src/lib.rs` et
`cmd/main.rs` aussi). C'est pourquoi chaque générateur affiche une
ligne `Created src/.../mod.rs` ou `Updated src/.../mod.rs` - le
câblage fait partie du travail, pas une étape de suivi que vous
retenez vous-même.

---

## Résumé

| Commande | Crée | Câble dans |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs` (+ avertit à propos de `lib.rs`) |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`, `schedule.rs`, `lib.rs`, `main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | (pas de câblage de module) |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | (pas de câblage de module) |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | (pas de câblage de module) |
| `generate-types` | `frontend/src/types/inertia-props.ts` | n/a |

## Suivant

- [Présentation CLI](cli.md) - le tableau complet des sous-commandes
- [Console](console.md) - le binaire console par projet dans lequel
  `make:command` s'insère
- [Contrôleurs](controllers.md) - le contrat de handler que
  `make:controller` scaffolde
- [Planification](scheduling.md) - l'API de planification fluide
  utilisée pour enregistrer les tâches générées par `make:task`
- [Migrations CLI](cli-migrations.md) - les commandes migrate /
  db:sync qui vont de pair avec `make:migration`
