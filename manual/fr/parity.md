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
| Cycle de vie des requêtes | Chaîne `Application` → `Server` → `handle_request` | livré | [Cycle de vie](lifecycle.md) |
| Conteneur de service | `Container` + façade `App`, à trois couches (tâche / thread / global) | divergent | Task-local pour le par-requête, thread-local pour les tests - [Conteneur](container.md) |
| Liaison contextuelle (`when()->needs()->give()`) | Pas de liaisons contextuelles - une liaison par trait et par couche de conteneur | non, par conception | Le conteneur est indexé par `TypeId` sans réflexion à l'exécution pour indexer une liaison sur « qui demande ». Composez explicitement : passez la dépendance, ou liez un newtype distinct par consommateur. [Conteneur](container.md) |
| Fournisseurs de services | Fonction `bootstrap()` + `#[service]`, `#[policy]`, `#[command]`, macros d'observateur | divergent | Pas de classe d'enregistrement - bootstrap est une seule fonction ; les macros utilisent `inventory` pour l'enregistrement à la compilation. [Amorçage](bootstrap.md) |
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
| URL signées | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | livré | HMAC-SHA256 avec `APP_KEY` |
| Liaison de modèle de route | `#[handler]` extrait `Post` de `{post}` via une impl `RouteBinding` | livré | Le derive `AutoRouteBinding` implémente automatiquement le trait pour les types `#[suprnova::model]` |
| Limitation de débit | middleware `throttle:60,1` + `RateLimiter::for_signature` | livré | [Limitation de débit](rate-limiting.md) |
| Middleware | trait `impl Middleware` ; enregistrement global ou par route | livré | [Middleware](middleware.md) |
| Groupes de middleware + alias | `register_middleware_group`, `register_middleware_alias` | livré | Résolution par nom de chaîne dans les routes |
| Protection CSRF | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | livré | La validation du token par session est le comportement par défaut. Les politiques optionnelles `SameOriginOnly`, `AllowSameSite` et `OriginOnly` consultent `Sec-Fetch-Site` ; l'application de l'origine n'est pas activée par défaut. [CSRF](csrf.md) |
| Contrôleurs | `#[handler] pub async fn show(req: Request) -> Response` | livré | Les contrôleurs sont des modules de fonctions libres, pas des classes. [Contrôleurs](controllers.md) |
| Contrôleurs à action unique | Un handler est déjà une fonction unique ; regroupez-les en modules | livré | La convention Rust - pas de cérémonie `__invoke` |
| Requêtes | struct `Request` avec `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()`, etc. | livré | [Requêtes](requests.md) |
| Form Requests | `#[derive(Data, Validate, FormRequest)]` | livré | La validation s'exécute au moment de l'extraction |
| Téléversement de fichiers | `req.file("avatar")?` retourne `UploadedFile` ; multipart en streaming avec plafonds de taille et de parties | livré | Bascule automatique vers un fichier temporaire au-delà du seuil |
| Réponses | builders `HttpResponse` + `json_response!()` / `text_response!()` / `Redirect::to` / réponses Inertia | livré | [Réponses](responses.md) |
| Réponses en streaming (`eventStream`, `stream`, `streamJson`) | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | livré | Les mêmes formes réseau que celles qu'attendent les hooks de `@laravel/stream-{react,vue,svelte}`. [Événements serveur](sse.md) |
| `withoutCookie` / `withoutCookies` | `.without_cookie(name)` / `.without_cookies([...])` sur `HttpResponse`, `Response`, `Redirect`, `RedirectRouteBuilder` | livré | `Cookie::forget_with(name, path, domain)` pour un cookie qui n'a pas été posé sur `/` |
| Vues (Blade) | Pages Inertia rendues côté serveur (Svelte/React/Vue) - aucun équivalent de Blade | divergent | Inertia est la couche de vue. Utilisez [Composants de page](frontend-pages.md) plutôt que Blade |
| Bundling d'assets (Vite) | Vite 8 est livré dans chaque scaffold ; `suprnova serve` exécute Vite et le backend ensemble | livré | Lecture du manifeste + HMR câblés automatiquement |
| Assets statiques (`public/`, servis par le serveur web sous Laravel) | Handler de repli en processus `StaticFiles::public()` qui sert `public/` à la racine web | livré | `StaticFiles::from_dir(...)` + `cache_control(...)` ; aucun serveur web séparé n'est nécessaire |
| Génération d'URL | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | livré | [Génération d'URL](urls.md) |
| Session | `session()`, `session_mut()`, flash bag via `req.flash()` | livré | Adossée à la base de données par défaut via `DatabaseSessionDriver` ; le cookie chiffré du navigateur porte l'identifiant de session et les métadonnées de dernière activité, pas le sac de données de session. [Sessions](session.md) |
| File de cookies (`Cookie::queue`) | `Cookie::queue`/`queued`/`unqueue`/`expire` - une réserve task-local que `SessionMiddleware` vide sur la réponse | livré | Exige `SessionMiddleware` dans la chaîne ; mis en file par nom, et non par nom+chemin comme le `CookieJar` de Laravel |
| Validation | `#[derive(Validate)]` + 35 règles intégrées + traits `Rule`/`ValueRule`/`AsyncRule` | livré | `Url` utilise la liste blanche de schémas de Laravel, et `Url::protocols([...])` reflète `url:http,https`. Les règles async (p. ex. `Unique`) atteignent la BD. `ArrayKeys`/`Distinct` sont des `ValueRule` sur `serde_json::Value`, correspondant aux `array:keys` et `distinct` de Laravel. `InArray` prend directement la liste de l'autre champ au lieu de la chaîne de règle `in_array:other.*` de Laravel, et `Contains`/`DoesntContain` font correspondre exactement les éléments JSON de type chaîne. `Gt`/`Gte`/`Lt`/`Lte` prennent un opérande `CompareWith` explicite (nombre littéral, champ voisin numérique, ou champ voisin comparé par nombre de caractères) au lieu d'inférer les quatre mesures de taille de Laravel ; les comparaisons de tableaux et de fichiers n'ont pas de contrepartie. [Validation](validation.md) |
| Règle `Password` (`Password::defaults()`, `uncompromised()`) | `Password::min(n)` + builders de robustesse (`.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) + `.uncompromised()` | livré | Vérification par k-anonymat Have I Been Pwned ; fail-open sur une erreur réseau, comme le `NotPwnedVerifier` de Laravel. [Validation](validation.md#password-strength) |
| Gestion des erreurs | `FrameworkError`, `AppError`, trait `HttpError`, limite de panique dans `execute_chain_safely` | livré | [Gestion des erreurs](errors.md), [Modèle d'erreur](error-model.md) |
| Journalisation | Subscriber `tracing` avec des champs structurés, `LogFormat` (json / pretty / compact) | divergent | Une ligne de log est un document JSON ; `request_id` est toujours présent. [Journalisation](logging.md) |
| Canaux de log / drivers de fichier (`single`, `daily`, `monthly`, `stack`) | `tracing` écrit des lignes structurées sur stdout ; la plateforme les fait tourner et les expédie | non, par conception | Les conteneurs, systemd et tous les expéditeurs de logs assurent déjà la rotation et la rétention. Le réimplémenter dans le processus duplique la plateforme et lui cache les logs. [Journalisation](logging.md) |
| Helpers d'abandon | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | livré | Même forme que la famille `abort_if` de Laravel |

## Approfondir

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Console Artisan | Binaire `console` par application, construit à partir de `#[command]` + `#[derive(Command)]` | livré | [Console](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Pas de REPL | non, par conception | Écrivez un script `cargo run --bin xxx` ponctuel ou un `#[suprnova_test]` |
| Diffusion | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | livré | Fan-out sea-streamer pour le multi-nœud. [Diffusion](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | livré | Opérations atomiques + cache étiqueté + verrous de cache (`LockGuard`). [Cache](cache.md) |
| Collections | `eloquent::Collection<M>` avec des méthodes de forme Laravel | livré | `Deref<Target = Vec<M>>`, si bien que les idiomes Vec existants fonctionnent toujours. [Collections](eloquent-collections.md) |
| Concurrence | Tokio partout - `tokio::spawn`, `tokio::join!`, `tokio::select!` | livré | Tout le framework est async. La façade `Concurrency::run([...])` de Laravel n'est pas livrée ; Tokio est la réponse |
| Contexte | `Context::put` / `Context::get` / `ContextStore` + injection automatique dans la queue / le mail / les événements | livré | [Contexte](context.md) |
| Contrats | Toutes les coutures publiques sont des traits | livré | Voir la ligne « Architecture / Contrats » ci-dessus |
| Événements | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, écouteurs en file d'attente, subscribers | livré | [Événements](events.md) |
| Stockage de fichiers | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` par-dessus OpenDAL | livré | La même surface `put/get/delete/copy/move/exists/url`. Protection intégrée contre la traversée de chemin. Sur un disque local, `put`/`write`, le writer en streaming et `copy` sont tous préparés sous `<root>/.suprnova-atomic/` puis publiés en une seule étape, et une écriture conditionnelle est publiée avec `link(2)` pour rester une vraie création exclusive ; `append` est la seule opération sur place. Le driver local de Laravel écrit directement sur la cible, où une longueur partielle est observable et où un plantage tronque l'objet en service. `Storage::register_read_through` compose deux disques en un disque read-through qui promeut sur le primaire ce qui est trouvé sur le repli, avec `copy: false` pour sauter la promotion et des `copy` / `rename` qui enjambent le repli. [Système de fichiers et stockage](filesystem.md) |
| Helpers | Les équivalents sont dans leurs modules d'origine (pas de `helpers.md` fourre-tout) | divergent | P. ex. les helpers d'URL vivent dans [urls.md](urls.md), les helpers de chaînes dans `std`/`heck`, les helpers de tableaux dans `std::collections` - Rust fait ça avec des crates, pas avec un espace de noms global |
| Client HTTP | Builder `Http::get/post/...` + `Http::fake(...)` pour les tests | livré | Enregistre automatiquement les requêtes ; `assert_sent` / `assert_not_sent` ; `.retry_when(predicate)` restreint la politique de réessai intégrée avec un `RetryContext`. [Client HTTP](http-client.md) |
| Image (`Illuminate\Image`) | `Image::from_bytes/from_path/from_disk/from_upload/from_stream` + la même surface d'opérations et de terminaux | livré | Vit dans `suprnova::media`. Deux drivers comme les `gd`/`imagick` de Laravel : `IMAGE_DRIVER=oxideav` (par défaut, Rust pur) ou `magick`. Lit et écrit PNG, JPEG, WebP, GIF, BMP ; la sortie AVIF est différée en attendant la publication de l'encodeur AV1 maison. Limites de décodage vérifiées sur l'en-tête. [Images](images.md) |
| Décodage HEIC dans le driver par défaut | `IMAGE_DRIVER=magick` sur un hôte doté du délégué libheif | non, par conception | HEVC est grevé de brevets et le seul décodeur crédible en Rust pur est sous double licence AGPL/commerciale : aucun décodeur intégré n'est donc livré. Même forme que chez Laravel, où GD ne lit pas HEIC du tout et où Imagick exige que le délégué soit compilé à la fois dans le binaire et dans l'extension PHP. [Images](images.md#why-suprnova-diverges) |
| Localisation | `Lang::get` / `get_with` / `try_get` / `has` + la macro `__!("key", name: value)` sur des catalogues Fluent `.ftl` dans `lang/<locale>/`, détection par `LocaleMiddleware`, messages de validation traduits, formatage ICU4X | livré | Le même catalogue est servi au navigateur sur `/_suprnova/lang/<locale>.ftl` et typé par `generate-types`. [Localisation](localization.md) |
| Mail | `Mail::to(...).send(MyMail { ... }).await?` + drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | livré | Trait `Mailable` + corps HTML/texte rendus par Tera ; les envois SES portent `TenantName` / `ConfigurationSetName` / `ListManagementOptions` ; le dispatch en file d'attente est routé par `.on_queue(...)` / `.on_connection(...)`, qui l'emportent sur `Queue::route`. [E-mail](mail.md) |
| Notifications | `Notify::send(&user, notif).await?` + canaux `mail/database/broadcast/webpush` | livré | Trait `Notifiable` + `Notification` par canal ; le dispatch en file d'attente (`Notify::queue`) porte les `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` propres à chaque notification jusqu'au job de chaque canal, via la même primitive `EnvelopeOverrides` qu'utilise Mail. [Notifications](notifications.md), [Web Push](web-push.md) |
| Développement de packages | Crates d'adaptateur du workspace (p. ex. `suprnova-payments-stripe`) | livré | Même forme que les packages Laravel : dépendez du framework, liez dans le conteneur, exposez des macros au besoin |
| Processus (exécuter des commandes shell) | `tokio::process::Command` de la bibliothèque standard | non, par conception | Pas de façade - l'API de Tokio a déjà la bonne forme |
| Files d'attente | `Queue::push(job).await?` + drivers `sync/memory/database/redis/null`, batches, chaînes, `JobMiddleware`, `FailedJobStore` | livré | [File d'attente](queues.md) |
| Délai déclaré par le job | `fn delay() -> Option<Duration>` sur `Job`, respecté par `Queue::push` et `Queue::bulk` | livré | Un appel explicite à `Queue::push_later` / `Queue::later(delay, job)` l'emporte toujours sur le défaut du job lui-même. [File d'attente](queues.md) |
| `Queue::forward` | `Queue::forward(from, to)` / `Queue::forward_on(from, to, connection)`, appliqué à l'enveloppe et à la liste `--queue` du worker | livré | File à file uniquement : `connection` conditionne la redirection au lieu de sélectionner un driver, et est comparée au nom de connexion du processus, si bien que le push et la réclamation du worker filtrent sur la même valeur ; `to` est obligatoire là où celui de Laravel est facultatif. [File d'attente](queues.md) |
| Événement de job unique ignoré | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | livré | Émis côté push quand `push_unique` déduplique ; l'appel retourne quand même `Ok(false)` |
| Mise en pause des files (`queue:pause` / `queue:resume`) | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`, adossés au cache, avec les événements `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` | livré | Une pause par file ne prend effet que sur un worker démarré avec une liste `--queue=...` explicite ; `resume_all` n'efface pas une pause par file. Un worker en cours d'exécution émet en plus `WorkerQueuePaused` / `WorkerQueueResumed` une fois par transition, et `queue:work` imprime une ligne pour chacun ; leur champ `queue` est un `Option<String>`, parce qu'un worker démarré sans `--queue` n'a aucun nom de file d'attente à signaler sous une pause globale. [File d'attente](queues.md) |
| Réessai des commandes Redis transitoires | Les commandes Redis de forme lecture réessaient une fois sur un échec au niveau de la connexion, chaque tentative attendant le budget de reconnexion du driver ; `REDIS_COMMAND_RETRIES` en ajoute d'autres | livré | Le `command_retries` de Laravel couvre toutes les commandes via un point de dispatch unique et une liste blanche de 60 entrées ; Suprnova réessaie site d'appel par site d'appel, et aucun réglage ne fait réessayer une écriture ou un dépilement de file d'attente. [Cache](cache.md) |
| Dispatch après commit (`afterCommit()`) | `fn after_commit() -> bool` sur `Job`, `EnvelopeOverrides::after_commit` par push, `Queue::push_after_commit` | livré | Tout le push attend le commit, événements compris, et un rollback le jette ; un `push_unique` différé prend quand même son verrou immédiatement, si bien que la déduplication fonctionne à l'intérieur de la transaction. Un `DB::begin_transaction` manuel ne diffère jamais. [File d'attente](queues.md) |
| Connexion de file en failover | `FailoverQueueDriver` sur une liste ordonnée de connexions, via `QUEUE_DRIVER=failover` + `QUEUE_FAILOVER_CONNECTIONS` | livré | Les écritures parcourent la liste ; `pop`, les compteurs et les listings restent sur la première connexion, si bien que chaque repli a besoin de son propre worker. `QueueFailedOver` est déclenché sur front, et `bulk_push` parcourt la liste enveloppe par enveloppe, chacune gardant ainsi son propre délai. [File d'attente](queues.md) |
| `ShouldBeUniqueUntilProcessing` | `fn unique_until_processing() -> bool` sur `Job`, libéré après la passe de middleware et avant le handler | livré | Libération liée au propriétaire, si bien qu'une tentative redélivrée ne libère jamais le verrou d'un dispatch plus récent. Un job qu'un middleware relâche sur la file garde son verrou. [File d'attente](queues.md) |
| Jobs avec debounce (`#[DebounceFor]`) | `Job::debounce_for` / `max_debounce_wait` / `debounce_id`, plus `Queue::push_debounced(job, DebounceOptions)` | livré | Des méthodes de trait plutôt qu'un attribut de classe, si bien qu'une faute de frappe est une erreur de compilation. Chaque dispatch est mis en file d'attente et la réduction se règle au niveau du worker ; un job qui déclare à la fois `debounce_for` et `unique_id` est refusé avec une `FrameworkError` là où Laravel lève. Les chaînes et les lots refusent d'emblée un job avec debounce. [File d'attente](queues.md) |
| Écouteurs en file d'attente avec debounce | `Job::debounce_for` sur le job de l'écouteur, ou `DebouncedListener::new(window, build).keyed_by(...)` | livré | Laravel pose l'attribut sur la classe de l'écouteur ; le pont écouteur-vers-job de Suprnova passe déjà par `Queue::push`, si bien que le déclarer sur le job couvre le cas courant et que `DebouncedListener` couvre une fenêtre par enregistrement. [Événements](events.md) |
| Inspection de file (`pendingJobs` / `delayedJobs` / `reservedJobs`) | `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs`, un `Option<&str>` réduisant en un seul appel le jumeau `all*Jobs()` de Laravel | livré | DTO `InspectedJob` (`id`/`queue`/`name`/`attempts`/`payload`/`created_at`) ; le défaut du trait est un `Err` honnête plutôt qu'une collection vide ; `sync`/`null` le redéfinissent avec `Ok(vec![])` ; le `reserved_jobs` de Redis est par consommateur. Contrairement à Laravel, ils ne suivent pas un `Queue::forward` : ils rapportent donc la file d'attente littérale que vous nommez - c'est ainsi qu'un backlog laissé en arrière sur une file d'attente redirigée reste visible. [File d'attente](queues.md) |
| Fuseau horaire par tâche planifiée | `.timezone(chrono_tz::Tz)` / `.try_timezone("name")` par tâche, défaut `Schedule::timezone`, `schedule:list --timezone` | livré | Un `chrono_tz::Tz` typé au lieu de la chaîne de Laravel ; le défaut valable pour toute la planification est `Schedule::timezone` dans `schedule::register` plutôt qu'une clé de configuration `app.schedule_timezone`, et une tâche sans zone épinglée garde la zone locale du processus. [Planification](scheduling.md) |
| Limitation de débit | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | livré | Fenêtre glissante via `SlidingWindowConfig`. [Limitation de débit](rate-limiting.md) |
| Recherche (Scout) | Pas d'adaptateur de recherche plein texte de première partie | pas encore | La recherche vectorielle est livrée aujourd'hui via [Vector](vector.md) ; l'équivalent Scout par mot-clé est prévu |
| Chaînes (helpers) | Crate `heck` (conversions de casse), `std::str`, `regex` | divergent | Les mêmes crates qu'utilise le reste de l'écosystème Rust ; pas de `Str::camel($x)` global |
| Planification de tâches | `Schedule::call/command/task` + `#[derive(Task)]` + syntaxe cron + worker `schedule:run` | livré | [Planification](scheduling.md) |
| Clés d'idempotence | `Idempotency::remember(key, ttl, body)` - protection contre le rejeu façon Stripe | livré | L'appelant place la clé dans un espace de noms formé de la route + l'identité utilisateur / métier. [Idempotence](idempotency.md) |
| Délai d'attente de requête | `TimeoutMiddleware` configurable par route | livré | Natif Rust - interrompt la future en cours et libère le worker. [Délais d'attente](timeout.md) |
| Flags de fonctionnalité (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + CRUD d'administration | livré | Propagation en moins d'une seconde via le trait `FeatureSync`. [Flags de fonctionnalité](feature-flags.md) |
| Observabilité (Pulse) | OpenTelemetry via `init_telemetry`, `Metrics`, `tracing` partout | divergent | OTel est la lingua franca de l'observabilité Rust - pointez votre collecteur sur le binaire. [Observabilité](observability.md) |
| Telescope (tableau de bord de débogage) | Pas encore d'équivalent | pas encore | Reporté à la v2+ ; la sortie tracing + OTel du framework couvre l'essentiel des besoins de diagnostic |
| Pulse (tableau de bord de performance) | Pas encore d'équivalent | pas encore | Comme Telescope - exposez les métriques avec votre stack d'observabilité existante jusqu'à ce qu'un tableau de bord soit livré |
| Recherche vectorielle | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | livré | Pas de verrouillage « Postgres pgvector uniquement ». [Vector](vector.md) |

### Exclusivités Suprnova (aucun équivalent Laravel)

| Suprnova | Ce que c'est | Notes / lien |
|---|---|---|
| Macro `ws!()` + handlers WebSocket | Des routes WS typées qui partagent le routeur et la pile de middleware | [WebSockets](websockets.md) |
| Workflows | Travail longue durée avec état, réessais, mise en sommeil et frontières d'étape | [Flux de travail](workflows.md) |
| Superviseurs | Trait `Supervisor` avec redémarrage automatique et capture des paniques pour les tâches tokio longue durée | [Superviseurs](supervisors.md) |
| Web Push (VAPID) | Les notifications push du navigateur comme canal de première classe | [Web Push](web-push.md) |
| Séparation lecture/écriture multi-connexions | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Base de données](database.md) |
| HTTP/2 + WebSocket sur le même socket | `hyper.with_upgrades()` dans `Server::run` | [Cycle de vie](lifecycle.md) |
| Contenu Markdown + pipeline de documentation | `MarkdownRenderer` (comrak assaini → syntect → ammonia) + `build_docs(DocsBuildConfig)` → un `DocsCatalog` de `DocsChapter` interrogeable | Extraction des titres + `slugify_heading` ; alimente la documentation Markdown / le blog sans générateur de site statique séparé |

## Sécurité

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Authentification | `Auth::user/check/login/logout/attempt`, trait `Authenticatable`, `Guard` par nom | livré | [Authentification](authentication.md) |
| Guards multiples | `Guard` enregistré par nom (`web`, `api`, …) via `AuthManager` | livré | `SessionGuard`, `TokenGuard`, impls personnalisées |
| Fournisseurs d'utilisateurs | `EloquentUserProvider<U>`, `DatabaseUserProvider`, personnalisé via le trait `UserProvider` | livré | [Flux d'authentification](auth-flows.md) |
| Vérification de l'e-mail | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail` ; contrat `MustVerifyEmail` | livré | Adossée au fournisseur et liée à l'acteur - [Flux d'authentification](auth-flows.md) |
| Réinitialisation de mot de passe | `PasswordReset` + transaction Magnetar de preuve par premier e-mail, ou repli sur un `UserProvider` vérifié, + e-mails de réinitialisation et de changement | livré | Magnetar gère la première preuve de façon atomique ; les applications adossées à un fournisseur peuvent réinitialiser des utilisateurs déjà vérifiés - [Flux d'authentification](auth-flows.md) |
| Limitation des attaques par force brute | Moteur de verrouillage Magnetar + `BruteForce` + `LoginThrottleMiddleware` | livré | Verrouillage de compte, plus limitation par IP et par route au niveau du framework |
| Double facteur (TOTP) | Façade de compatibilité `TwoFactor` du framework, plus le moteur de facteurs Magnetar | livré | Codes de récupération, protection contre le rejeu et connexion intégrée conditionnée à un facteur |
| Se souvenir de moi | Identifiant rotatif lié à un usage, fourni par Magnetar derrière le cookie du framework | livré | Vérifications d'époque d'authentification, rotation, traitement des anomalies et repli hérité |
| OAuth (Socialite) | Registre de fournisseurs Magnetar et façade `Auth::oauth(provider)` | livré | OAuth, `form_post` Apple, liaison PKCE/state, politique d'identité vérifiée - [OAuth](oauth.md) |
| Sanctum (jetons d'API) | `BearerTokenMiddleware` sur les sessions bearer Magnetar | divergent | Authentifie des sessions bearer ; pas d'API distincte de gestion de jetons façon Sanctum |
| Passport (serveur OAuth) | Moteurs de protocole et de plugins Magnetar | divergent | Les primitives du moteur sont livrées ; pas de façade applicative compatible Laravel Passport |
| Fortify (backend d'authentification) | Façades `Auth`/`auth_flows` du framework par-dessus les moteurs Magnetar | livré | Le framework possède le HTTP, le mail, les événements, les cookies et la liaison applicative |
| Autorisation (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + trait `Authorizable` + enregistrement par macro + `Gate::default_denial_response` | livré | [Autorisation](authorization.md) |
| Rôles et permissions (spatie/laravel-permission) | Trait `HasRoles` + tables `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | livré | De première partie, et non un package communautaire. Helpers `create_role` / `give_permission_to_role` / `assign_role_to_model` ; se superpose à Gate/Policy. [Autorisation](authorization.md) |
| Chiffrement | `Crypt::encrypt/decrypt` + liaison AAD `CryptPurpose` | livré | AES-256-GCM, rotation de clés via `APP_KEY_PREVIOUS`. [Chiffrement](encryption.md) |
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
| Options DSN `keepalives_*` de Postgres | Vivacité du pool via `DB_IDLE_TIMEOUT` / `DB_MAX_LIFETIME` / `DB_ACQUIRE_TIMEOUT` / `DB_TEST_BEFORE_ACQUIRE` / `DB_PING_AFTER_IDLE` | divergent | sqlx n'expose aucun setter de keepalive TCP : Suprnova recycle et pingue donc les connexions du pool à la place. [Base de données](database.md#pool-liveness) |
| MariaDB | Option de premier ordre à part entière (vecteur + JSON + temporel) | divergent | Traité séparément à cause des fonctionnalités multi-paradigmes que Laravel ne livre que pour Postgres |
| Redis | Utilisé par les drivers (cache/queue/rate-limit) - pas de façade `Redis::*` séparée | divergent | Utilisez directement la crate `redis` quand vous avez besoin de commandes ad hoc ; cache/queue/rate-limit couvrent 95 % des usages courants |
| MongoDB | Pas d'adaptateur de premier ordre pour l'instant | pas encore | Utilisez directement la crate `mongodb` via `App::bind` |
| Query Builder | `Builder<M>` avec `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` / etc. | livré | [Générateur de requêtes](queries.md) |
| Famille `whereBinary()` | `Builder::where_binary` / `or_where_binary` / `where_not_binary` / `or_where_not_binary`, et `DB::table(...).where_binary(...)` | livré | MySQL et MariaDB émettent `= binary` ; Postgres et SQLite retournent une erreur au lieu d'une correspondance dépendante de la collation. [Générateur de requêtes](queries.md) |
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
| Famille `wherePivot` (forme en closure incluse) | `where_pivot` / `where_pivot_op` / `where_pivot_in` / `where_pivot_not_in` / `where_pivot_null` / `where_pivot_not_null` / `where_pivot_between` / `where_pivot_not_between` / `where_pivot_group` plus les jumelles `or_` | divergent | En lecture seulement - un filtre de pivot ne restreint jamais `attach` / `detach` / `sync`, et les chargements hâtifs ne le transportent pas. [Relations](eloquent-relationships.md) |
| Chargement hâtif | `User::query().with(&["posts", "posts.comments"]).get()` | livré | `EagerLoadDispatch` est scellé ; seules les relations générées par macro peuvent l'implémenter |
| Prévention du lazy loading | `prevent_silently_discarding_attributes(true)` | livré | Même forme que le `preventLazyLoading` de Laravel |
| Agrégats sur les relations | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | livré | Une seule sous-requête par agrégat |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | livré | Moteur EXISTS corrélé |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | livré | Opère sur toute la collection |
| Cloner un enregistrement | `user.replicate()` / `user.replicate_into::<OtherType>()` | livré | Déclenche l'événement `Replicating` |
| Toucher les timestamps du parent | `#[model(touches = ["post"])]` | livré | Une mise à jour `UPDATE` par propriétaire `BelongsTo`, sur un seul niveau et sans événement (pas de récursion vers le grand-parent, pas d'événement `saved` du parent). `without_touching` / `without_touching_on::<M, _, _>()` pour l'ignorer. [Toucher les timestamps du parent](eloquent.md#parent-touching) |
| Observateurs | `impl Observer<User>` + `#[suprnova::observer(User)]` | livré | 16 événements de cycle de vie |
| 16 événements de cycle de vie | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | livré | Sous-module `events::*` par modèle. `EventResult::cancel(_)` court-circuite avec un 400 |
| Mutateurs / Accesseurs | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | livré | [Mutateurs](eloquent-mutators.md) |
| Casts (22 intégrés) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | livré | Implémentez `Cast` pour un cast personnalisé |
| Collections | `Collection<M>` avec `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` et leurs amis façon Laravel ; `Deref<Target = Vec<M>>`, si bien que tous les idiomes `Vec` continuent de fonctionner | livré | [Collections](eloquent-collections.md) |
| `modelKeys()` | `Builder::model_keys().await?` (sans hydratation, clé qualifiée) et `Collection::model_keys()` | livré | Les deux renvoient `Vec<M::Key>` ; le terminal du builder projette `users.id` pour survivre aux jointures |
| API Resources | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + fieldsets + includes | livré | La forme JSON:API et la forme resource façon Laravel sont toutes deux disponibles. Les chemins `?include=` sont plafonnés à `max_relationship_depth` (par défaut 5), à l'image de `JsonApiResource::$maxRelationshipDepth`. [Ressources JSON:API](eloquent-resources.md) |
| Sérialisation | `#[model(hidden = [...], visible = [...], appends = [...])]` | livré | Même contrôle sur les attributs qui se sérialisent. [Sérialisation](eloquent-serialization.md) |
| Fabriques | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (ou `UserFactory::times(5).create_many().await?`) | livré | `Sequence` pour faire cycler des valeurs. [Fabriques](eloquent-factories.md) |
| Cycle de vie : chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | livré | Itération à mémoire bornée sur de grandes tables |
| Verrouillage pessimiste | `Builder::lock_for_update()`, `shared_lock()` | livré | À l'intérieur d'une transaction |
| `refreshForUpdate()` | `model.refresh_for_update().await?` | livré | Rechargement `SELECT ... FOR UPDATE` ; verrou sans effet sur SQLite. [Verrouillage des lignes](eloquent.md#row-locking) |
| `inOrderOf(col, values)` | `Builder::in_order_of(col, values)` | livré | Tri par `CASE WHEN` lié ; les valeurs non listées sont triées en dernier. Builder typé seulement. [Tri](eloquent.md#ordering) |
| `orWhereKey` / `orWhereKeyNot` | `Builder::or_where_key(id)` / `Builder::or_where_key_not(id)` | livré | Se fondent dans la clause précédente comme une disjonction ; alias `or_filter_key` / `or_filter_key_not` |
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
| Style Pest / PHPUnit | `#[suprnova_test]` (conscient de l'async) + assertions `expect!()` façon Jest + macros BDD `describe!()` / `test!()` | livré | Les trois s'utilisent indifféremment |
| Tests de fonctionnalité (HTTP) | Piloter `handle_request(router, registry, req)` dans le même processus - normalement via une connexion hyper loopback afin que le serveur reçoive un vrai corps `Incoming` | livré | [Tests HTTP](http-tests.md) |
| Wrapper `TestResponse` | Assertions directes sur `HttpResponse` (`status_code()`, `body()`, `header_value()`) | pas encore | Un wrapper fluide `assert_status` / `assert_json_path` / `assert_cookie` est prévu ; aujourd'hui les tests décodent la réponse une fois et font leurs assertions sur la valeur [Tests HTTP](http-tests.md#fluent-response-assertions-with-testresponse) |
| Aides de test Inertia | `suprnova::testing::AssertableInertia` - `component`/`url`/`version`/`prop`/`has`/`missing`/`where_`/`count`/`has_flash`, plus `reload_only`/`reload_except`/`load_deferred_props` via une fermeture `with_reload` fournie par l'appelant | livré | [Tests HTTP](http-tests.md#testing-inertia-responses) |
| Tests de console | Exécuter `dispatch_argv(["console", "..."])` et faire les assertions | livré | Même forme que les tests HTTP, pour le binaire console |
| Tests navigateur (Dusk) | s.o. dans le framework - utilisez Playwright / WebdriverIO / le navigateur agent `gstack` | non, par conception | L'outillage inter-langages existe déjà ; nous ne le réinventons pas |
| Tests de base de données | `TestDatabase::fresh::<Migrator>()` | livré | Crée une base SQLite en mémoire fraîche par test, applique les migrations, l'enregistre dans le conteneur de test, puis jette cet état base de données/conteneur isolé à la destruction ; il n'enveloppe pas chaque test dans une transaction de rollback. [Tests de base de données](database-testing.md) |
| Mocking et fakes | Fakes par façade : `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | livré | Appels enregistrés + helpers d'assertion. [Mocking](mocking.md) |
| UUID de job `QueueFake` | `queue::testing::pushed_with_id::<J>()` | livré | La fake ajoute un identifiant d'enveloppe à chaque push et émet le même `JobQueued` qu'un vrai push |
| Voyage dans le temps | `tokio::time::{pause, advance, resume}` depuis le runtime de la bibliothèque standard | livré | Nous ne livrons pas le nôtre - l'API de Tokio le fait déjà |
| Isolation du conteneur | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-local | divergent | Sûr en parallèle par construction. [Conteneur](container.md) |

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

## Frontend (Laravel a Blade + des kits de démarrage ; nous avons Inertia)

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| Blade | s.o. - Inertia est la couche de vue | divergent | [Frontend](frontend.md) |
| Inertia.js | De première classe : v3 sur Svelte 5 / React 19 / Vue 3.5 | livré | [Réponses Inertia](frontend-inertia-responses.md), [Pages](frontend-pages.md) |
| `Route::inertia($uri, $component, $props)` | `Router::inertia(path, component, props)` | livré | Renvoie un `RouteBuilder`, donc la chaîne `.name(...)` / `.middleware(...)`; `Router::view` est l'ancien alias |
| Résolution de l'URL de page (`Inertia::resolveUrlUsing`) | `page.url` est le chemin + la chaîne de requête ; redéfinissez avec `InertiaConfig::url_resolver` | livré | La dérivation par défaut correspond octet pour octet au `X-Inertia-Location` du middleware de version ; un `url_resolver` ne change que `page.url` |
| Middleware du protocole Inertia (`Vary`, réponse vide, rebond de version) | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware` - trois des middlewares câblés par `Inertia::install` (la redirection des erreurs de validation et les pages d'erreur sont les deux lignes suivantes) | livré | `Vary: X-Inertia` sur chaque réponse ; un `200` vide sur une visite Inertia devient un `303` de retour ; le rebond 409 re-flashe la session |
| Redirection des erreurs de validation (`Middleware::resolveValidationErrors`, `$withAllErrors`) | `InertiaValidationRedirectMiddleware`, câblé par `Inertia::install`; `InertiaConfig::with_all_errors(bool)` | livré | Un `422` sur une visite Inertia devient un `303` de retour avec les erreurs flashées ; la valeur d'un champ se replie sur son premier message sauf si `with_all_errors(true)`. [Réponses Inertia](frontend-inertia-responses.md#validation-failures) |
| Pages d'erreur (`Inertia::render('Error', ['status' => …])` depuis le handler d'exceptions) | `InertiaConfig::error_page("Error")` ; `InertiaErrorPageMiddleware`, câblé par `Inertia::install` lorsqu'un composant est nommé | livré | Une ligne de configuration au lieu d'une modification du handler d'exceptions. Un `403` / `404` / `429` / `500` sur une visite Inertia ou une navigation de navigateur rend la page nommée au statut d'origine, avec les props `status` / `message` / `request_id` ; les clients d'API, les `422`, les rebonds `X-Inertia-Location` et tout corps qui n'est pas une enveloppe d'erreur restent intacts. Les trois starters livrent la page et l'activent. [Réponses Inertia](frontend-inertia-responses.md#error-pages) |
| Redirection externe + effacement de l'historique | `InertiaResponse::location_for(&req, url)`, `App::clear_history()` | livré | `location_for` vaut `409` pour un XHR et `302` pour une navigation dure ; `App::clear_history()` survit à la redirection de déconnexion |
| `Inertia::share` / `getShared` / `flushShared` | `App::inertia_share` / `_lazy` / `_once`, `App::inertia_shared(key)`, `App::flush_inertia_shared()` | livré | Imbrication par clés pointées via la sémantique `Arr::set` ; `InertiaSharedData::share(&req, component)` peut varier selon la page. Un partage pointé reste plat jusqu'au passage de dépliage de la réponse, donc `only`/`except` correspondent à une entrée ancêtre (`only: ['auth']` atteint `auth.user`) comme Laravel obtient le même résultat via `Arr::set` au moment du partage |
| Rechargements partiels | `#[derive(Data)]` + `req.includes("subset")` + le protocole de rechargement partiel d'Inertia | livré | Ensembles d'includes typés. `?include=` filtre chaque variante lazy, y compris `lazy(deferred)`, et s'exécute avant `X-Inertia-Partial-Data`, si bien qu'un include interdit retourne quand même 400. `errors` est exempté d'`only`/`except`, à l'image du partage `Inertia::always` de Laravel |
| Props deferred | `.defer(…)` / `.defer_with(…, DeferOptions)`, ou `Prop::…defer()` | livré | Protocole de props deferred d'Inertia v3 ; `DeferOptions` porte le groupe et le flag rescue. `deferredProps` n'est livré que lors de la visite initiale - `resolveDeferredProps` retourne `[]` sur tout partiel correspondant |
| Props merge | `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with(MergeStrategy)` / `.merge_lazy` / `.merge_lazy_with`, ou `Prop::…merge().merge_with_path(...)` | livré | Protocole de fusion d'Inertia v3 ; `match_on` prend un champ ou plusieurs ; `merge_with_path` fusionne un champ imbriqué au lieu de la racine de la prop |
| Composition des props (`defer()->merge()`, `merge()->once()`, `optional()->once()`) | `Prop` flag builder + `InertiaResponse::prop(key, prop)` | livré | `Prop` est une struct de flags orthogonaux, à l'image des interfaces `Deferrable` / `Mergeable` / `Onceable` de l'adaptateur PHP |
| Chiffrer l'historique | `EncryptHistoryMiddleware` | livré | Historique chiffré au repos dans le client |
| Position de défilement | `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.paginate` + `ScrollMetadata` / `ProvidesScrollMetadata` | livré | Restauration automatique à la navigation ; `reset` lit `X-Inertia-Reset`, à l'image de `resolveScrollProps` |
| Types TypeScript | `suprnova generate-types` lit `#[derive(InertiaProps)]` et émet des `.d.ts` | livré | [Types TypeScript](frontend-typescript-types.md) |
| Lecture du manifeste Vite | Câblée automatiquement via `InertiaConfig::manifest_path` | livré | HMR en dev, assets hachés en prod. `Inertia::install` fait un échec fermé en production quand le manifeste est absent |
| Version des actifs depuis le manifeste de build | `InertiaConfig` par défaut : `VersionResolver::from_manifest(manifest_path)` | livré | Hachage des octets du manifeste ; repli statique `"1.0"` quand il n'y a rien à hacher |
| SSR Inertia (`inertia:start-ssr`) | `InertiaConfig::ssr(...)` sur la config passée à `Inertia::install`, worker lancé par `suprnova ssr:start` | livré | Worker hors processus via un loopback HTTP ; retombe sur le CSR en cas d'erreur ou d'expiration, sauf si `ssr_throw_on_error(true)`. `InertiaConfig::ssr_bundle_path(...)` conditionne l'envoi à la présence du bundle construit sur le disque (à l'image d'`ensure_bundle_exists`), à basculer avec `.ssr_ensure_bundle_exists(bool)` (actif par défaut dès qu'un chemin de bundle est défini) ; `suprnova new` scaffolde `frontend/src/ssr.{ts,tsx}` et un script `build:ssr` pour chaque starter ; `suprnova ssr:check` vérifie la route `GET /health` du worker. [Réponses Inertia](frontend-inertia-responses.md) |

## CLI

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `php artisan` | Binaire `console` par app, construit à partir de macros `#[command]` | livré | [Console](console.md), [Présentation CLI](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | livré | [Générateurs](cli-generators.md) |
| `serve` | `suprnova serve` (backend + serveur de dev Vite ensemble) | livré | [Serve](cli-serve.md). Saute le panneau Vite sur un projet `--api` au lieu de refuser de démarrer. |
| Famille `migrate` | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | livré | [Migrations CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (via le console par app) | livré | Seeders enregistrés via le trait `Seeder` ; une exécution ciblée imprime RUNNING / DONE avec les millisecondes écoulées |
| `schedule:run` / `schedule:work` / `schedule:list` | Mêmes noms via le binaire console par app | livré | [Commandes de planification](cli-scheduling.md) |
| `queue:work` | Même nom via le binaire console par app | livré | Arrêt propre sur SIGTERM/SIGINT |
| `tinker` | Pas de REPL | non, par conception | Voir la ligne dans « Approfondir » |

## Déploiement

| Laravel | Suprnova | Statut | Notes / lien |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | divergent | Un seul binaire, pas d'étape opcache |
| `php artisan config:cache` | La config typée est déjà vérifiée à la compilation | divergent | Aucun cache à l'exécution à invalider |
| `php artisan route:cache` | Les routes sont expansées par macro à la compilation | divergent | Le routeur est construit au démarrage à partir de routes déjà typées |
| Envoy (déploiements SSH) | Utilisez n'importe quel orchestrateur - Docker, systemd, Kubernetes, fly.io, Railway | non, par conception | Le binaire est l'artefact de déploiement |
| Forge / Vapor | Ce n'est pas à nous de les livrer - mais les recettes pour Railway, DO et Hetzner couvrent le même besoin | divergent | [Déploiement](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Mode de maintenance (`php artisan down` / `up`) | `./app down` / `./app up` - secret de contournement avec une expiration de 12 heures vérifiée côté serveur, retry/message/chemins exclus personnalisés, driver `file` ou `cache` | livré | [Déploiement](deployment.md) |
| Horizon (tableau de bord de file) | Pas encore de tableau de bord | pas encore | Inspection des jobs en échec via `cargo run --bin console queue:failed` en attendant |

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
| Sanctum | `BearerTokenMiddleware` sur les sessions bearer Magnetar | divergent | Aucun package distinct ni surface de gestion de jetons d'accès personnels |
| Scout (recherche plein texte) | pas encore | pas encore | La recherche vectorielle est livrée ([Vector](vector.md)) ; l'équivalent Scout par mot-clé viendra plus tard |
| Socialite | Registre de fournisseurs Magnetar et `Auth::oauth(provider)` | livré | [OAuth](oauth.md) |
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

## Ce qui nous manque vraiment encore

Une liste consolidée de chaque **pas encore** ci-dessus, pour que vous
voyiez la forme de la lacune en un seul endroit :

| Domaine | Ce qui manque | Contournement en attendant la livraison |
|---|---|---|
| Recherche (Scout - par mot-clé) | Adaptateur Algolia / Meilisearch / Elastic | Faites le vôtre avec `meilisearch-sdk` / `elasticsearch` en attendant ; [Vector](vector.md) gère la recherche sémantique aujourd'hui |
| Passport (serveur OAuth) | Fournisseur d'identité OAuth de première partie | Faites tourner Hydra / Keycloak derrière Suprnova |
| Telescope (tableau de bord de débogage) | Interface web pour les requêtes / les requêtes SQL / les événements / les succès de cache | Utilisez la sortie OTel + tracing ([Observabilité](observability.md)) |
| Pulse (tableau de bord de performance) | Interface web pour les requêtes lentes / les erreurs / les routes les plus sollicitées | Pareil : la surface OTel aujourd'hui, un tableau de bord plus tard |
| Horizon (tableau de bord de file d'attente) | Interface web pour la profondeur de file / les jobs en échec / le débit | `cargo run --bin console queue:failed` et les métriques OTel |

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
| `php artisan dev --tabs` (mode de développement TUI multi-panneaux) | Le flux mono-terminal préfixé par `[name]` est la norme des outils de dev Rust (`cargo watch`, `bacon`, `just`) - `suprnova serve` donne déjà à chaque processus (backend, frontend et toute entrée `Suprnova.toml`) son propre préfixe coloré et son redémarrage auto. Un TUI à onglets serait un second modèle d'interaction pour un signal que cela fournit déjà ; la mission de `--stream` - un flux scriptable et temps réel - est livrée sous `suprnova serve --json` (NDJSON, un événement par ligne). [Serve](cli-serve.md#extra-dev-processes) |

## Comment cette liste reste honnête

Chaque ligne de la colonne **livré** est vérifiable en :

1. Cherchant l'export nommé dans `framework/src/lib.rs`
2. Exécutant la suite de tests du framework (`cargo test --workspace`)
3. Lisant le chapitre lié

Chaque ligne de la colonne **pas encore** est du travail prévu, pas un
refus. Chaque ligne de la colonne **non, par conception** a une raison en
une phrase dans la colonne Notes ; ces raisons sont les principes de
conception de l'[Introduction](introduction.md) appliqués à une
fonctionnalité précise.

Dernière revue face à Laravel 13.25.0.

Si vous trouvez une fonctionnalité Laravel que vous utilisez et qui n'est
pas sur cette carte, ouvrez une issue - soit elle a une réponse Suprnova
à qui il manque une ligne, soit c'est une vraie lacune et nous voulons le
savoir.

## Suivant

- [Depuis Laravel](from-laravel.md) - la même carte, racontée côte à côte
- [Introduction](introduction.md) - les principes de conception que suit ce
  travail de parité
- [`documentation.md`](documentation.md) - la table des matières maîtresse à
  travers tous les chapitres
