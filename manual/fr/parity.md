# Carte de parité avec Laravel

La cartographie honnête, fonctionnalité par fonctionnalité, entre Laravel
13.x et Suprnova. Utilisez cette page lorsque vous vous demandez « Suprnova
a-t-il X ? » et que vous voulez une réponse oui/non/où en une seule ligne.

Les sections reflètent l'index de la documentation Laravel, pour qu'un
développeur Laravel puisse parcourir la page de haut en bas. Dans chaque
section, les colonnes sont toujours les mêmes :

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|

La colonne **Statut** utilise quatre valeurs :

| Symbole | Signification |
|---|---|
| **livré** | Même surface, même comportement (souvent les mêmes noms de méthode) |
| **divergent** | Même rôle, forme différente parce que Rust permet un meilleur choix |
| **pas encore** | Vraiment prévu, pas encore sur le disque |
| **non, par conception** | Ne sera pas livré - explication dans la colonne Notes |

Le chapitre pertinent (quand il existe) est lié depuis la colonne **Notes**.

C'est une carte vivante. Suprnova livre toute la surface de Laravel 13.x à
travers les 30 domaines documentés ; les lacunes listées ci-dessous sont les
lacunes réelles et actuelles du framework tel qu'il est livré.

## Concepts d'architecture

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Cycle de vie de la requête | chaîne `Application` → `Server` → `handle_request` | livré | [Cycle de vie](lifecycle.md) |
| Conteneur de services | `Container` + façade `App`, à trois couches (tâche / thread / global) | divergent | Local à la tâche pour chaque requête, local au thread pour les tests - [Conteneur](container.md) |
| Fournisseurs de services | fonction `bootstrap()` + macros `#[service]`, `#[policy]`, `#[command]`, macros d'observateur | divergent | Pas de classe d'enregistrement - l'amorçage est une seule fonction ; les macros utilisent `inventory` pour l'enregistrement au moment de la compilation. [Amorçage](bootstrap.md) |
| Façades | `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` statiques | livré | Même forme d'appel ; les façades sont de vrais types, pas des alias |
| Contrats | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider`, etc. | livré | Toutes les coutures publiques vivent sur des traits ; liez par trait, échangez les implémentations librement |

## Commencer

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Installation | `cargo install --git …suprnova-cli` puis `suprnova new <name>` | livré | [Installation](installation.md) |
| Configuration | config typée via `#[derive(Config)]` + `Config::register` | divergent | Typée à la compilation plutôt que des tableaux bruts. [Configuration](configuration.md) |
| Développement agentique (IA) | Pas de SDK IA de premier ordre dans le framework | non, par conception | Utilisez les crates que vous utiliseriez de toute façon (`async-openai`, `anthropic-rs`, `tokenizers`, etc.) sous `App::bind(Arc<dyn YourLlm>)` |
| Structure des répertoires | `src/{actions,bootstrap,controllers,middleware,models,routes}` | livré | Même intention, disposition idiomatique Rust. [Structure](structure.md) |
| Frontend | Inertia v3 sur Svelte 5 / React 19 / Vue 3.5 | livré | [Frontend](frontend.md), [Pages](frontend-pages.md), [Types TS](frontend-typescript-types.md) |
| Kits de démarrage | **Nebula** (auth) et **Pulsar** (site produit complet), plus le scaffold nu `suprnova new` | livré | Deux kits sont livrés aujourd'hui - Nebula est l'équivalent de Breeze ; Pulsar ajoute la doc, le blog, la communauté et le RBAC. [Kits de démarrage](starter-kits.md) |
| Déploiement | Binaire unique ; recettes Docker / Railway / DO / Hetzner | divergent | Un seul artefact, pas un runtime PHP + opcache + FPM. [Déploiement](deployment.md) |

## Les bases

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Définitions de routes | macro `routes!` + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | livré | [Routage](routing.md) |
| Paramètres de route | paramètres de chemin `{id}` + `req.param("id")` | livré | Paramètres optionnels via `{id?}` ; contraintes via `where!()` |
| Noms de route | `.name("posts.show")` sur la route + `url("posts.show", &[("id", "42")])` | livré | [Génération d'URL](urls.md) |
| Groupes de routes | macro `group!` avec `.prefix()` / `.middleware()` / `.name()` / `.controller()` | livré | Le middleware de groupe est aplati sur chaque route au moment de l'enregistrement |
| Routes de ressource | `resource!("posts", PostController)` enregistre les 7 routes standard | livré | `apiResource!`, `only(...)`, `except(...)` sont tous pris en charge |
| URLs signées | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | livré | HMAC-SHA256 avec `APP_KEY` |
| Liaison de modèle de route | `#[handler]` extrait `Post` depuis `{post}` via une impl `RouteBinding` | livré | Le derive `AutoRouteBinding` s'implémente automatiquement pour les types `#[suprnova::model]` |
| Limitation de débit | middleware `throttle:60,1` + `RateLimiter::for_signature` | livré | [Limitation de débit](rate-limiting.md) |
| Middleware | trait `impl Middleware` ; s'enregistre globalement ou par route | livré | [Middleware](middleware.md) |
| Groupes et alias de middleware | `register_middleware_group`, `register_middleware_alias` | livré | Résolus par nom de chaîne dans les routes |
| Protection CSRF | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | livré | La politique d'origine impose un POST same-origin. [CSRF](csrf.md) |
| Contrôleurs | `#[handler] pub async fn show(req: Request) -> Response` | livré | Les contrôleurs sont des modules de fonctions libres, pas des classes. [Contrôleurs](controllers.md) |
| Contrôleurs à action unique | Un handler est déjà une seule fonction ; regroupez-les en modules | livré | La convention Rust - pas de cérémonie `__invoke` |
| Requêtes | struct `Request` avec `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()`, etc. | livré | [Requêtes](requests.md) |
| Requêtes de formulaire | `#[derive(Data, Validate, FormRequest)]` | livré | La validation s'exécute au moment de l'extraction |
| Envoi de fichiers | `req.file("avatar")?` renvoie `UploadedFile` ; multipart en streaming avec plafonds de taille + de parties | livré | Débordement automatique vers un fichier temporaire au-dessus du seuil |
| Réponses | builders `HttpResponse` + `json!()` / `text!()` / `Redirect::to` / `view` | livré | [Réponses](responses.md) |
| Vues (Blade) | Pages Inertia rendues côté serveur (Svelte/React/Vue) - pas d'équivalent Blade | divergent | Inertia est la couche de vue. Utilisez [Pages](frontend-pages.md) au lieu de Blade |
| Empaquetage des assets (Vite) | Vite 8 est livré dans chaque scaffold ; `suprnova serve` lance Vite et le backend ensemble | livré | Lecture du manifeste + HMR câblés automatiquement |
| Assets statiques (`public/`, servis par le serveur web dans Laravel) | handler de repli `StaticFiles::public()` en process, servant `public/` à la racine web | livré | `StaticFiles::from_dir(...)` + `cache_control(...)` ; pas besoin d'un serveur web séparé |
| Génération d'URL | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | livré | [Génération d'URL](urls.md) |
| Session | `session()`, `session_mut()`, flash bag via `req.flash()` | livré | Adossée à la BD via `DatabaseSessionDriver` ; adossée au cookie par défaut. [Session](session.md) |
| Validation | `#[derive(Validate)]` + 17 règles intégrées + traits `Rule`/`AsyncRule` | livré | Les règles async (par ex. `Unique`) sollicitent la BD. [Validation](validation.md) |
| Gestion des erreurs | `FrameworkError`, `AppError`, trait `HttpError`, limite de panique dans `execute_chain_safely` | livré | [Gestion des erreurs](errors.md), [Modèle d'erreur](error-model.md) |
| Journalisation | subscriber `tracing` avec champs structurés, `LogFormat` (json / pretty / compact) | divergent | Une ligne de log est un document JSON ; `request_id` est toujours présent. [Journalisation](logging.md) |
| Aides d'interruption | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | livré | Même forme que la famille `abort_if` de Laravel |

## Approfondir

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Console Artisan | Binaire `console` par app, construit à partir de `#[command]` + `#[derive(Command)]` | livré | [Console](console.md). `cargo run --bin console <sous-commande>` |
| Tinker (REPL) | Pas de REPL | non, par conception | Écrivez un script `cargo run --bin xxx` ponctuel ou un `#[suprnova_test]` |
| Diffusion | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | livré | Fanout sea-streamer pour le multi-nœud. [Diffusion](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | livré | Opérations atomiques + cache taggué + verrous de cache (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` avec des méthodes façon Laravel | livré | `Deref<Target = Vec<M>>`, si bien que les idiomes Vec existants fonctionnent toujours. [Collections](eloquent-collections.md) |
| Concurrence | Tokio partout - `tokio::spawn`, `tokio::join!`, `tokio::select!` | livré | Tout le framework est async. La façade Laravel `Concurrency::run([...])` n'est pas livrée ; Tokio est la réponse |
| Contexte | `Context::put` / `Context::get` / `ContextStore` + auto-injection dans la queue / le mail / les événements | livré | [Contexte](context.md) |
| Contrats | Toutes les coutures publiques sont des traits | livré | Voir la ligne « Architecture / Contrats » ci-dessus |
| Événements | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, écouteurs mis en queue, subscribers | livré | [Événements](events.md) |
| Stockage de fichiers | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` par-dessus OpenDAL | livré | Même surface `put/get/delete/copy/move/exists/url`. Protection contre le path-traversal intégrée. [Filesystem](filesystem.md) |
| Aides | Les équivalents vivent dans leur module d'origine (pas de `helpers.md` fourre-tout) | divergent | Par ex. les aides URL vivent dans [urls.md](urls.md), les aides de chaînes dans `std`/`heck`, les aides de tableaux dans `std::collections` - Rust fait cela avec des crates, pas un espace de noms global |
| Client HTTP | builder `Http::get/post/...` + `Http::fake(...)` pour les tests | livré | Enregistre automatiquement les requêtes ; `assert_sent` / `assert_not_sent`. [Client HTTP](http-client.md) |
| Localisation | `Lang::get` / `get_with` / `try_get` / `has` + la macro `__!("key", name: value)` par-dessus des catalogues Fluent `.ftl` dans `lang/<locale>/`, détection par `LocaleMiddleware`, messages de validation traduits, formatage ICU4X | livré | Le même catalogue est servi au navigateur sur `/_suprnova/lang/<locale>.ftl` et typé par `generate-types`. [Localisation](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` | livré | Trait `Mailable` + corps HTML/texte rendus par Tera. [E-mail](mail.md) |
| Notifications | `Notify::send(&user, notif).await?` + canaux `mail/database/broadcast/webpush` | livré | Trait `Notifiable` + `Notification` par canal. [Notifications](notifications.md), [Web Push](web-push.md) |
| Développement de packages | Crates adaptateurs du workspace (par ex. `suprnova-payments-stripe`) | livré | Même forme que les packages Laravel : dépendre du framework, se lier dans le conteneur, exposer des macros au besoin |
| Processus (exécution de commandes shell) | `tokio::process::Command` de la stdlib | non, par conception | Pas de façade - l'API de Tokio a déjà la bonne forme |
| Files d'attente | `Queue::push(job).await?` + drivers `sync/memory/database/redis/null`, batches, chaînes, `JobMiddleware`, `FailedJobStore` | livré | [File d'attente](queues.md) |
| Limitation de débit | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | livré | Fenêtre glissante via `SlidingWindowConfig`. [Limitation de débit](rate-limiting.md) |
| Recherche (Scout) | Pas d'adaptateur de recherche plein texte de premier ordre | pas encore | La recherche vectorielle est livrée dès aujourd'hui via [Vector](vector.md) ; l'équivalent Scout par mot-clé est prévu |
| Chaînes (aides) | crate `heck` (conversions de casse), `std::str`, `regex` | divergent | Les mêmes crates que le reste de l'écosystème Rust utilise ; pas de `Str::camel($x)` global |
| Planification de tâches | `Schedule::call/command/task` + `#[derive(Task)]` + syntaxe cron + worker `schedule:run` | livré | [Planification](scheduling.md) |
| Clés d'idempotence | `Idempotency::remember(key, ttl, body)` - protection contre le rejeu façon Stripe | livré | L'appelant met la clé sous l'espace de noms de la route + de l'identité utilisateur/métier. [Idempotence](idempotency.md) |
| Timeout de requête | `TimeoutMiddleware` configurable par route | livré | Natif Rust - abandonne le future en vol, libère le worker. [Timeout](timeout.md) |
| Flags de fonctionnalité (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + CRUD admin | livré | Propagation en moins d'une seconde via le trait `FeatureSync`. [Flags de fonctionnalité](feature-flags.md) |
| Observabilité (Pulse) | OpenTelemetry via `init_telemetry`, `Metrics`, `tracing` partout | divergent | OTel est la lingua franca de l'observabilité Rust - pointez votre collecteur sur le binaire. [Observabilité](observability.md) |
| Telescope (tableau de bord de débogage) | Pas d'équivalent pour l'instant | pas encore | Reporté à la v2+ ; la sortie tracing + OTel du framework couvre l'essentiel des besoins de diagnostic |
| Pulse (tableau de bord de perf) | Pas d'équivalent pour l'instant | pas encore | Comme Telescope - exposez les métriques avec votre stack d'observabilité existante jusqu'à ce qu'un tableau de bord soit livré |
| Recherche vectorielle | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | livré | Pas de verrouillage du type « Postgres pgvector uniquement ». [Recherche vectorielle](vector.md) |

### Exclusif à Suprnova (sans équivalent Laravel)

| Suprnova | Ce que c'est | Notes / lien |
|---|---|---|
| macro `ws!()` + handlers WebSocket | Routes WS typées qui partagent le routeur et la pile de middleware | [WebSockets](websockets.md) |
| Événements serveur | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| Workflows | Travail à état, longue durée, avec réessais, sommeil, limites d'étape | [Flux de travail](workflows.md) |
| Superviseurs | Trait `Supervisor` avec redémarrage automatique sur panique pour les tâches tokio de longue durée | [Superviseurs](supervisors.md) |
| Web Push (VAPID) | Notifications push navigateur comme canal de premier ordre | [Web Push](web-push.md) |
| Séparation lecture/écriture multi-connexion | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Base de données](database.md) |
| HTTP/2 + WebSocket sur le même socket | `hyper.with_upgrades()` dans `Server::run` | [Cycle de vie](lifecycle.md) |
| Pipeline de contenu Markdown + docs | `MarkdownRenderer` (comrak assaini → syntect → ammonia) + `build_docs(DocsBuildConfig)` → `DocsCatalog` interrogeable de `DocsChapter` | Extraction des titres + `slugify_heading` ; fait tourner la doc/le blog Markdown sans générateur de site statique séparé |

## Sécurité

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Authentification | `Auth::user/check/login/logout/attempt`, trait `Authenticatable`, `Guard` par nom | livré | [Authentification](authentication.md) |
| Guards multiples | `Guard` enregistré par nom (`web`, `api`, …) via `AuthManager` | livré | `SessionGuard`, `TokenGuard`, impls personnalisées |
| Fournisseurs d'utilisateurs | `EloquentUserProvider<U>`, `DatabaseUserProvider`, personnalisés via le trait `UserProvider` | livré | [Flux d'authentification](auth-flows.md) |
| Vérification d'e-mail | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail` ; contrat `MustVerifyEmail` sur le modèle utilisateur | livré | Adossé à un fournisseur (pas de torii) - [Flux d'authentification](auth-flows.md) |
| Réinitialisation de mot de passe | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail` ; contrat `CanResetPassword` sur le modèle utilisateur | livré | Adossé à un fournisseur (pas de torii) - [Flux d'authentification](auth-flows.md) |
| Throttling anti-force-brute | `BruteForce` + `LoginThrottleMiddleware` | livré | Comptage par IP + par utilisateur |
| Deux facteurs (TOTP) | `TwoFactor` + `TwoFactorChallengeMiddleware` + trait `TwoFactorUser` | livré | Codes de récupération + protection contre le rejeu |
| Se souvenir de moi | Cookie signé longue durée via `SessionGuard` | livré | `auth::remember` propriété du framework : ligne BD + bcrypt + rotation à usage unique |
| OAuth (Socialite) | Via le fork vendorisé `torii_integration` (Google / GitHub / Apple etc.) | livré | [Authentification](authentication.md) |
| Sanctum (jetons API) | `TokenGuard` + jetons adossés à la BD via torii | divergent | Le modèle de jeton + le middleware bearer sont livrés ; pas de surface API Sanctum séparée |
| Passport (serveur OAuth) | Pas encore | pas encore | Si vous avez besoin d'un fournisseur OAuth, faites tourner un service d'identité dédié (Keycloak, Hydra) derrière Suprnova |
| Fortify (backend d'auth) | Remplacé par le module `auth_flows` + les types `auth_flows::*` | livré | Même rôle ; pas besoin de séparation headless/headed puisque le frontend est Inertia |
| Autorisation (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + trait `Authorizable` + enregistrement par macro | livré | [Autorisation](authorization.md) |
| Rôles et permissions (spatie/laravel-permission) | Trait `HasRoles` + tables `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | livré | Propriété du framework, pas un package communautaire. Aides `create_role` / `give_permission_to_role` / `assign_role_to_model` ; se superpose à Gate/Policy. [Autorisation](authorization.md) |
| Chiffrement | `Crypt::encrypt/decrypt` + liaison AAD `CryptPurpose` | livré | AES-256-GCM, rotation de clé via `APP_KEY_PREVIOUS`. [Chiffrement](encryption.md) |
| Hachage | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | livré | Bcrypt par défaut ; argon2id disponible. [Hachage](hashing.md) |

## Base de données

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | livré | [Base de données](database.md), [Générateur de requêtes](queries.md) |
| Connexions multiples | `DB::on("read")` + `ConnectionRegistry` | livré | Séparation lecture/écriture de premier ordre |
| Transactions | `DB::transaction(\|tx\| async move { ... }).await?` | livré | Savepoints + réessai sur deadlock |
| Événements de requête | `QueryListener` + événement `QueryExecuted` | livré | `DB::listen(\|q\| { ... })` |
| Expressions brutes | `DB::raw("...")`, `DB::select("...", &[...])` | livré | Liaison de paramètres requise (pas d'interpolation de chaîne) |
| Postgres / MySQL / SQLite | Les trois de premier ordre via SeaORM | livré | Détection d'URL dans `database::config::database_type()` |
| MariaDB | Option de premier ordre à part entière (vecteur + JSON + temporel) | divergent | Traité séparément à cause des fonctionnalités multi-paradigmes que Laravel ne livre que pour Postgres |
| Redis | Utilisé par les drivers (cache/queue/rate-limit) - pas de façade `Redis::*` séparée | divergent | Utilisez directement la crate `redis` quand vous avez besoin de commandes ad hoc ; cache/queue/rate-limit couvrent 95 % des usages courants |
| MongoDB | Pas d'adaptateur de premier ordre pour l'instant | pas encore | Utilisez directement la crate `mongodb` via `App::bind` |
| Query Builder | `Builder<M>` avec `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` / etc. | livré | [Générateur de requêtes](queries.md) |
| Pagination | `LengthAwarePaginator`, `Paginator` (simple), `CursorPaginator` | livré | Les trois se sérialisent en JSON façon Laravel. [Pagination](pagination.md) |
| Migrations | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | livré | Exécutées via `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migrations](migrations.md), [Migrations CLI](cli-migrations.md) |
| Seeders | Trait `Seeder` + sous-commande `db:seed` | livré | Fabriques par modèle. [Ensemencement](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | livré | La struct EST le `Model` SeaORM. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | livré | Tout est async |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | livré | macro `attrs! { name: "...", email: "..." }` pour des attributs partiels |
| Garde-fous d'assignation de masse | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + scope `unguarded \|\| { ... }` | livré | `prevent_silently_discarding_attributes()` pour le mode strict |
| Suppressions logicielles | `#[model(soft_deletes)]` injecte automatiquement `deleted_at` + trait `SoftDeletes` | livré | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + worker `model:prune` | livré | Épinglé en cascade sur les relations |
| Timestamps | `created_at`/`updated_at` automatiques si les colonnes sont présentes | livré | Désactivable via `#[model(timestamps = false)]` |
| Types de clé primaire | i64 par défaut ; UUID / ULID via `#[model(unique_id = "uuid")]` ou `unique_id = "ulid"` | livré | Génère automatiquement l'id à l'insertion |
| Scopes locaux | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | livré | Dispatch de méthode sur `Builder<M>` |
| Scopes globaux | `impl GlobalScope for ActiveOnly { ... }` + enregistrement | livré | Retirés via `Builder::without_global_scope` |
| Relations (11 sortes) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | livré | Enum de morph par famille. [Relations](eloquent-relationships.md) |
| Chargement hâtif | `User::query().with(&["posts", "posts.comments"]).get()` | livré | `EagerLoadDispatch` est scellé ; seules les relations générées par macro peuvent l'implémenter |
| Prévention du lazy loading | `prevent_silently_discarding_attributes(true)` | livré | Même forme que le `preventLazyLoading` de Laravel |
| Agrégats sur les relations | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | livré | Une seule sous-requête par agrégat |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | livré | Moteur EXISTS corrélé |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | livré | Opère sur toute la collection |
| Cloner un enregistrement | `user.replicate()` / `user.replicate_into::<OtherType>()` | livré | Déclenche l'événement `Replicating` |
| Toucher les timestamps du parent | `#[model(touches = ["post"])]` | livré | `without_touching \|\| { ... }` pour l'ignorer |
| Observateurs | `impl Observer<User>` + `#[suprnova::observer(User)]` | livré | 16 événements de cycle de vie |
| 16 événements de cycle de vie | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | livré | Sous-module `events::*` par modèle. `EventResult::cancel(_)` court-circuite avec un 400 |
| Mutateurs / Accesseurs | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | livré | [Mutateurs](eloquent-mutators.md) |
| Casts (22 intégrés) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | livré | Implémentez `Cast` pour un cast personnalisé |
| Collections | `Collection<M>` avec `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` et leurs amis façon Laravel ; `Deref<Target = Vec<M>>`, si bien que tous les idiomes `Vec` continuent de fonctionner | livré | [Collections](eloquent-collections.md) |
| API Resources | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + fieldsets + includes | livré | La forme JSON:API et la forme resource façon Laravel sont toutes deux disponibles. [Ressources API](eloquent-resources.md) |
| Sérialisation | `#[model(hidden = [...], visible = [...], appends = [...])]` | livré | Même contrôle sur les attributs qui se sérialisent. [Sérialisation](eloquent-serialization.md) |
| Fabriques | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (ou `UserFactory::times(5).create_many().await?`) | livré | `Sequence` pour faire cycler des valeurs. [Fabriques](eloquent-factories.md) |
| Cycle de vie : chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | livré | Itération à mémoire bornée sur de grandes tables |
| Verrouillage pessimiste | `Builder::lock_for_update()`, `shared_lock()` | livré | À l'intérieur d'une transaction |
| Famille `whereJsonContains` | Disponible via les expressions de colonne de SeaORM (selon le backend) | livré | L'orthographe exacte diffère selon le backend ; des aides sont livrées pour les cas courants |

## Pagination

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator` (page + total + per_page + last_page) | livré | `Builder::paginate(n).await?` |
| `Paginator` (simple) | `Paginator` (page + per_page + has_more, pas de count) | livré | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator` (jeton de curseur opaque + direction) | livré | `Builder::cursor_paginate(n).await?` ; déterministe pour le défilement infini |
| Intégration Inertia | Trait `IntoInertiaScroll` + `ScrollMetadata` | livré | Se câble directement sur `WhenVisible` / `merge` d'Inertia |

## IA (Laravel l'expédie nativement aujourd'hui ; nous ne verrouillons rien)

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| SDK IA | Pas de SDK IA propriétaire | non, par conception | Apportez la crate que vous utilisez déjà (`async-openai`, `anthropic-sdk`, `ollama-rs`, `tokenizers`, etc.) et liez-la sous `App` |
| MCP (Model Context Protocol) | Pas d'adaptateur serveur MCP propriétaire | non, par conception | Les crates MCP Rust (`mcp-rs`, `mcp-sdk-rust`) se posent proprement sous la surface de routage / superviseur existante |
| Boost (agent de code Laravel) | s/o | non, par conception | Hors du périmètre du framework |

## Tests

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `php artisan test` | `cargo test` | livré | [Tests](testing.md) |
| Style Pest / PHPUnit | `#[suprnova_test]` (async-aware) + assertions façon Jest `expect!()` + macros BDD `describe!()` / `test!()` | livré | Les trois s'utilisent indifféremment |
| Tests fonctionnels (HTTP) | Fait tourner `handle_request(router, registry, req)` en process - pas de socket ouvert | livré | [HTTP Tests](http-tests.md) |
| Tests de console | Exécutez `dispatch_argv(["console", "..."])` et affirmez | livré | Même forme que les tests HTTP pour le binaire console |
| Tests navigateur (Dusk) | s/o dans le framework - utilisez Playwright / WebdriverIO / l'agent navigateur `gstack` | non, par conception | L'outillage cross-langage existe déjà ; nous ne le réinventons pas |
| Tests de base de données | `TestDatabase::fresh::<Migrator>()` + rollback par test | livré | [Tests de base de données](database-testing.md) |
| Mocking et fakes | Fakes par façade : `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | livré | Appels enregistrés + aides d'assertion. [Mocking](mocking.md) |
| Voyage dans le temps | `tokio::time::{pause, advance, resume}` du runtime stdlib | livré | Nous ne livrons pas le nôtre - l'API de Tokio le fait déjà |
| Isolation du conteneur | `TestContainer::fake(\|tc\| tc.bind(...))` - local au thread | divergent | Sûr en parallèle par construction. [Container](container.md) |

## Paiements (Cashier chez Laravel ; le nôtre est générique par fournisseur)

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Cashier (Stripe) | crate adaptateur `suprnova-payments-stripe` derrière les traits génériques `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` | divergent | Surface générique, adaptateur concret. [Paiements](payments.md), [Adaptateur Stripe](payments-stripe.md) |
| Cashier (Paddle) | adaptateur `suprnova-payments-paddle` | divergent | Flux Merchant-of-Record + pas d'impl `Payment` directe (Paddle possède la passerelle). [Adaptateur Paddle](payments-paddle.md) |
| Fournisseur personnalisé | Implémentez `PaymentProvider` + `SessionPayload` + `WebhookHandler` | livré | [Guide du fournisseur](payments-provider-guide.md) |
| Composants de checkout Inertia | Boucles de dispatch documentées pour Svelte / React / Vue face à `SessionPayload.flow` | livré | [Paiements Frontend](payments-frontend.md). Des pages de facturation prêtes à l'emploi sont un ajout de starter-kit prévu ([Kits de démarrage](starter-kits.md)) |
| Cycles de vie d'abonnement | `Subscription::subscribe / update / cancel / get` (là où le fournisseur les prend en charge) | livré | `NotSupported` renvoyé là où le fournisseur ne les prend pas en charge (par ex. `subscribe` chez Paddle et le remplacement d'ensemble de prix) |
| Idempotence des webhooks | table miroir `payments_webhook_events` avec `UNIQUE(provider, provider_event_id)` | livré | Protection contre le rejeu façon Stripe |
| Tables miroir | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | livré | Colonne JSONB `provider_metadata` sur chacune pour les champs spécifiques à l'adaptateur |

## Frontend (Laravel a Blade et des kits de démarrage ; nous avons Inertia)

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Blade | s/o - Inertia est la couche de vue | divergent | [Frontend](frontend.md) |
| Inertia.js | De premier ordre : v3 sur Svelte 5 / React 19 / Vue 3.5 | livré | [Réponses Inertia](frontend-inertia-responses.md), [Pages](frontend-pages.md) |
| Rechargements partiels | `#[derive(Data)]` + `req.includes("subset")` + protocole de rechargement partiel d'Inertia | livré | Ensembles d'inclusion typés |
| Props différées | `Prop::deferred(...)` + `DeferConfig` | livré | Protocole de props différées d'Inertia v3 |
| Props fusionnées | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | livré | Protocole de fusion d'Inertia v3 |
| Historique chiffré | `EncryptHistoryMiddleware` | livré | Historique chiffré au repos côté client |
| Position de défilement | `ScrollConfig` + `ScrollMetadata` | livré | Restauration automatique à la navigation |
| Types TypeScript | `suprnova generate-types` lit `#[derive(InertiaProps)]` et émet des `.d.ts` | livré | [Types TS](frontend-typescript-types.md) |
| Lecture du manifeste Vite | Câblée automatiquement via `Inertia::root_view` | livré | HMR en dev, assets hashés en prod |

## CLI

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `php artisan` | Binaire `console` par app, construit à partir de macros `#[command]` | livré | [Console](console.md), [Présentation CLI](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | livré | [Générateurs](cli-generators.md) |
| `serve` | `suprnova serve` (backend + serveur de dev Vite ensemble) | livré | [Serve](cli-serve.md) |
| Famille `migrate` | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | livré | [Migrations CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (via le console par app) | livré | Seeders enregistrés via le trait `Seeder` |
| `schedule:run` / `schedule:work` / `schedule:list` | Mêmes noms via le binaire console par app | livré | [Commandes de planification](cli-scheduling.md) |
| `queue:work` | Même nom via le binaire console par app | livré | Arrêt propre sur SIGTERM/SIGINT |
| `tinker` | Pas de REPL | non, par conception | Voir la ligne dans « Approfondir » |

## Déploiement

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | divergent | Un seul binaire, pas d'étape opcache |
| `php artisan config:cache` | La config typée est déjà vérifiée à la compilation | divergent | Pas de cache runtime à invalider |
| `php artisan route:cache` | Les routes sont expansées par macro à la compilation | divergent | Le routeur est construit au démarrage à partir de routes déjà typées |
| Envoy (déploiements SSH) | Utilisez n'importe quel orchestrateur - Docker, systemd, Kubernetes, fly.io, Railway | non, par conception | Le binaire est l'artefact de déploiement |
| Forge / Vapor | Pas à nous de les livrer - mais les recettes pour Railway, DO et Hetzner couvrent le même rôle | divergent | [Déploiement](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Horizon (tableau de bord de queue) | Pas de tableau de bord pour l'instant | pas encore | Inspection des jobs échoués via `cargo run --bin console queue:failed` en attendant |

## Packages (les packages officiels de Laravel - les nôtres sont soit livrés dans le cœur, soit livrés comme adaptateurs, soit des lacunes délibérées)

| Package Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` | livré | Générique + adaptateur. [Paiements](payments.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` | livré | Flux MoR. [Paiements](payments.md) |
| Dusk | s/o | non, par conception | L'outillage navigateur cross-langage existe déjà (Playwright, etc.) |
| Envoy | s/o | non, par conception | Les containers / systemd / orchestrateurs font le travail |
| Fortify | Remplacé par `auth_flows` | livré | Même rôle, intégré. [Flux d'authentification](auth-flows.md) |
| Folio | s/o - le routage par page n'est pas idiomatique en Rust | non, par conception | Utilisez `routes!` pour un routage explicite |
| Homestead | s/o - utilisez Docker / DevContainers | non, par conception | [Recette Docker](cli-docker.md) |
| Horizon | pas encore | pas encore | Les jobs échoués remontent via le console par app |
| Mix | Remplacé par Vite | divergent | Vite est livré dans chaque scaffold |
| Octane | s/o - nous sommes déjà du Tokio longue durée | non, par conception | Binaire unique, toujours chaud, pas de FPM à sortir |
| Passport | pas encore | pas encore | Faites tourner un IdP dédié derrière Suprnova jusqu'à ce que ce soit livré |
| Pennant (flags de fonctionnalité) | Réimplémenté sous `features::*` | livré | [Flags de fonctionnalité](feature-flags.md) |
| Pint (style de code PHP) | `cargo fmt` + `cargo clippy` | divergent | Toolchain Rust standard |
| Precognition | Requêtes précognitives Inertia via les rechargements partiels + les mêmes types `#[derive(Data, Validate, FormRequest)]` | livré | Les deux moitiés de Precog (validation précoce + rechargement léger) découlent toutes deux d'Inertia v3 + des form requests |
| Prompts (UI CLI) | Utilisez la crate `dialoguer` / `inquire` au besoin | non, par conception | L'écosystème Rust couvre déjà ça |
| Pulse | pas encore | pas encore | OTel aujourd'hui, tableau de bord plus tard |
| Reverb (serveur WebSocket) | Intégré à Suprnova (`ws!()` + `BroadcastHub`) | divergent | Pas besoin de serveur séparé - c'est le même process |
| Sail (dev Docker) | `suprnova-cli` livre des recettes Docker intégrées | livré | [CLI Docker](cli-docker.md) |
| Sanctum | `TokenGuard` + middleware bearer | divergent | Le modèle de jeton est livré ; pas de surface de package séparée |
| Scout (recherche plein texte) | pas encore | pas encore | La recherche vectorielle est livrée ([Vector](vector.md)) ; l'équivalent Scout par mot-clé viendra plus tard |
| Socialite | Via le fork torii vendorisé | livré | [Authentification](authentication.md) |
| Telescope | pas encore | pas encore | Tracing + OTel couvrent le manque de diagnostic jusqu'à ce qu'un tableau de bord soit livré |
| Valet | s/o - les apps Rust tournent directement | non, par conception | `suprnova serve` est le lanceur de dev |

## Macros (surface spécifique à Rust ; analogues Laravel les plus proches, à titre de contexte)

Suprnova livre un large ensemble de proc-macros qui n'ont pas d'analogue
Laravel parce que Laravel n'a pas de macros - il a la réflexion à
l'exécution. Elles sont incluses ici pour que vous ne les manquiez pas.

| Macro | Idée Laravel la plus proche | Ce que ça fait |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | Génère une entité SeaORM + implémente le trait `Model` |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | Enregistre une impl `Observer<M>` via `inventory` |
| `#[scopes(M)]` | Scopes locaux sur un modèle | Ajoute des méthodes à `Builder<M>` |
| `#[accessor]` / `#[mutator]` | Accesseurs / mutateurs Eloquent | Hooks get/set au niveau du champ |
| `#[handler]` | `__invoke` du contrôleur | Extrait automatiquement les paramètres typés depuis `Request` |
| `#[command]` / `#[derive(Command)]` | Classe de commande Artisan | Enregistre une sous-commande console |
| `#[policy]` | Classe policy | Enregistre une impl `Policy` via `inventory` |
| `#[service(T)]` | `register` du fournisseur de service | Lie `T` dans le conteneur |
| `#[injectable]` | Injection par constructeur | Génère un constructeur adossé à `App::make` |
| `#[derive(InertiaProps)]` | Props Inertia | Génération de code TypeScript + sérialisation Inertia |
| `#[derive(Data)]` | DTO de requête | Extractible depuis `Request` avec prise en charge des ensembles d'inclusion |
| `#[derive(FormRequest)]` | Classe `FormRequest` | Validation + gate d'auth + transformation |
| `#[derive(Factory)]` | Fabrique de modèle | Génération de données de test adossée à Faker |
| `#[derive(Resource)]` | API Resource | Sérialisation JSON:API + façon Laravel |
| `#[workflow]` / `#[workflow_step]` | s/o chez Laravel | Travail à état, longue durée |
| `routes!` + `get!` / `post!` / `ws!` etc. | `Route::get` / `Route::post` | Enregistrement de route à la compilation |
| `casts!` | `protected $casts = [...]` | Déclaration de cast par modèle |
| `attrs!` | Tableau d'assignation de masse | Builder d'attributs partiels |
| `json_response!` / `text_response!` | `response()->json(...)` | `Ok(HttpResponse::...)` rapide |

Voir [Macros](macros.md) pour la référence complète.

## Fonctions utilitaires (les aides globales de Laravel ; les nôtres sont typées)

Laravel livre des centaines de petites globales (`str_replace_first`,
`array_flatten`, `now()`, `tap()`, `optional()` …). La plupart ont un
équivalent Rust direct dans `std` ou une petite crate standard, si bien que
Suprnova ne les réintroduit pas sous un espace de noms unique. Celles qui
*sont* utiles à avoir en alias sont livrées sous leur module d'origine.

| Aide Laravel | Équivalent Suprnova / Rust | Où |
|---|---|---|
| `auth()` | `Auth::user().await?` | [Authentification](authentication.md) |
| `cache()` | `Cache::get/put/...` | [Cache](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [Configuration](configuration.md) |
| `csrf_token()` | `csrf_token()` (même nom) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()` (dump-and-die de requête Eloquent) / `dbg!()` de la stdlib | `Builder::dump()` / `Builder::dd()` existent pour l'inspection de requête ; utilisez `dbg!()` pour les valeurs générales |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [Configuration](configuration.md), [Env Vars](env-vars.md) |
| `now()` | `chrono::Utc::now()` (ré-exporté sous `suprnova::chrono`) | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust gère ça directement avec `Option<T>` |
| `redirect('/')` | `redirect("/")` (même nom) | [Routage](routing.md) |
| `request()` | `Request` est passé dans votre handler | [Requêtes](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [Réponses](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [Génération d'URL](urls.md) |
| `session('key')` | `session().get("key")` | [Session](session.md) |
| `str()` / `Str::camel($x)` | méthodes de la crate `heck` (`ToUpperCamelCase`, etc.) | - |
| `tap($x, fn) → $x` | `tap` de la crate `tap`, ou `dbg!` pour une inspection rapide | Utilisez la crate `tap` de façon idiomatique |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | Appelez simplement la closure : `x()` | s/o - les closures Rust n'ont besoin d'aucune aide |
| `view('home', $data)` | Réponse Inertia : `Inertia::render("Home", data)` | [Réponses Inertia](frontend-inertia-responses.md) |

## Ce qui nous manque réellement pour l'instant

Une liste consolidée de chaque **pas encore** ci-dessus, pour que vous voyiez
la forme du manque en un seul endroit :

| Domaine | Ce qui manque | Solution de contournement en attendant |
|---|---|---|
| Recherche (Scout - mot-clé) | Adaptateur Algolia / Meilisearch / Elastic | Roulez le vôtre avec `meilisearch-sdk` / `elasticsearch` en attendant ; [Vector](vector.md) gère la recherche sémantique dès aujourd'hui |
| Passport (serveur OAuth) | Fournisseur d'identité OAuth de premier ordre | Faites tourner Hydra / Keycloak derrière Suprnova |
| Telescope (tableau de bord de débogage) | UI web pour les requêtes / queries / événements / hits de cache | Utilisez la sortie OTel + tracing ([Observabilité](observability.md)) |
| Pulse (tableau de bord de perf) | UI web pour les requêtes lentes / erreurs / routes chaudes | Pareil : surface OTel aujourd'hui, tableau de bord plus tard |
| Horizon (tableau de bord de queue) | UI web pour la profondeur de queue / jobs échoués / débit | `cargo run --bin console queue:failed` et les métriques OTel |

## Ce que nous ne livrerons pas (et pourquoi)

| Fonctionnalité Laravel | Pourquoi Suprnova ne l'a pas |
|---|---|
| Tinker (REPL) | Rust n'a pas d'histoire de REPL productive pour des binaires compilés. Un court `#[suprnova_test]` ou un script `cargo run --bin <thing>` ponctuel fait le travail |
| Templates Blade | Inertia est la couche de vue ; nous ne livrons pas de moteur de template rendu côté serveur en parallèle |
| Fourre-tout `helpers.md` | Rust livre `std` + de petites crates focalisées (`heck`, `chrono`, `regex`) ; nous ne réintroduisons pas un espace de noms global unique |
| Mix | Vite couvre ça et est livré dans chaque scaffold |
| Octane | Suprnova est déjà du Tokio longue durée ; il n'y a pas de mode FPM dont il faille sortir |
| Dusk (tests navigateur) | L'outillage cross-langage (Playwright, WebdriverIO, l'agent navigateur `gstack`) résout déjà ça |
| Sail (dev Docker) | Les recettes Docker sont livrées intégrées ([CLI Docker](cli-docker.md)) ; pas besoin de package séparé |
| Valet | `suprnova serve` est le serveur de dev |
| Envoy (déploiements SSH) | Les containers / systemd / orchestrateurs font le travail ; nous n'avons pas besoin d'un DSL SSH sur mesure |
| Façade de concurrence (`Concurrency::run`) | Tokio (`tokio::join!` / `tokio::spawn` / `tokio::select!`) est la réponse ; pas besoin de façade |
| Façade de processus | `tokio::process::Command` a déjà la bonne forme |
| SDK IA / MCP / Boost propriétaires | Choisissez les crates Rust que vous utilisez déjà ; nous ne verrouillons rien |
| Façade Redis dédiée | Cache/queue/rate-limit couvrent 95 % des usages courants ; utilisez la crate `redis` quand vous avez besoin de commandes ad hoc |
| Façade de chaînes | `heck`, `regex`, `std::str` couvrent ça ; pas de `Str::camel($x)` global |
| Bibliothèque de prompts (UI CLI) | `dialoguer` / `inquire` existent déjà ; nous ne réinventons pas |
| Fichiers de traduction PHP/JSON façon Laravel | La localisation est livrée, mais le format de catalogue est Fluent `.ftl` - un seul format que le serveur et le navigateur analysent tous les deux. `trans_choice` n'a pas non plus d'équivalent : Fluent sélectionne les catégories de pluriel CLDR à l'intérieur du message. [Localisation](localization.md) |

## Comment cette liste reste honnête

Chaque ligne de la colonne **livré** est vérifiable par :

1. Grepper `framework/src/lib.rs` pour l'export nommé
2. Faire tourner la suite de tests du framework (`cargo test --workspace`)
3. Lire le chapitre lié

Chaque ligne de la colonne **pas encore** est un travail prévu, pas un
refus. Chaque ligne de la colonne **non, par conception** a une raison en
une phrase dans la colonne Notes ; ces raisons sont les principes de
conception d'[Introduction](introduction.md) appliqués à une fonctionnalité
précise.

Si vous trouvez une fonctionnalité Laravel que vous cherchez et qui n'est
pas sur cette carte, ouvrez une issue - soit elle a une réponse Suprnova à
qui il manque une ligne, soit c'est un vrai manque et nous voulons le
savoir.

## Suivant

- [Depuis Laravel](from-laravel.md) - la même carte, racontée côte à côte
- [Introduction](introduction.md) - les principes de conception que suit ce
  travail de parité
- [`documentation.md`](documentation.md) - la table des matières maîtresse à
  travers tous les chapitres
