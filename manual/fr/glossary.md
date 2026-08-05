# Glossaire

Termes propres à Suprnova, définis une seule fois. Si un chapitre utilise
un mot sans l'expliquer, la définition vit ici. Les entrées sont
alphabétiques ; suivez le lien croisé vers le chapitre qui utilise le
terme en contexte.

Une poignée de conventions à garder en tête en lisant le reste de cette
liste :

- **Trait** désigne un trait Rust - un contrat de comportement que vous
  implémentez sur un type. **Façade** désigne une struct de taille nulle
  dont les méthodes statiques sont le point d'entrée d'un sous-système
  (`Cache`, `Mail`, `Auth`, `Storage`, `Bus`, `Notify`, `Vector`, `DB`,
  `Schedule`, `App`).
- **Driver** désigne un backend interchangeable derrière une façade ou un
  registre - `CacheStore`, `QueueDriver`, `VectorDriver`,
  `RateLimiterDriver`, `MailDriver`. Les drivers sont choisis au
  démarrage via des variables d'environnement et liés à travers le
  conteneur.
- **Registre** désigne une table de correspondance globale au processus,
  peuplée à la compilation via `inventory` ou au démarrage via un
  enregistrement explicite - `ConnectionRegistry`, `MiddlewareRegistry`,
  `InertiaRegistry`, `ChannelRegistry`, `VectorRegistry`,
  `SupervisorRegistry`, `PaymentProviderRegistry`, `ScopeRegistry`.

## A

### Accesseur

Une transformation en lecture déclarée sur un modèle Eloquent avec la
macro `#[accessor]`. S'exécute chaque fois que la propriété est lue, et
retourne une valeur calculée dérivée d'une ou plusieurs colonnes
sous-jacentes (`full_name` à partir de `first_name + last_name`, par
exemple). Le dual d'un [Mutateur](#mutateur). Voir
[Eloquent - Accesseurs et mutateurs](eloquent.md#accessors-and-mutators).

### Action

Une classe de service injectable qui encapsule un morceau de logique
métier - une seule méthode publique, des dépendances injectées via la
macro `#[injectable]`. L'analogue Suprnova des invocables à action
unique de Laravel. Les actions sont liées comme singletons dans le
conteneur automatiquement et résolues par les handlers, les jobs, et
d'autres actions. Voir [Actions](actions.md).

### Application

Le builder fluent dans `Application::new()` qui enregistre vos
fonctions de config, de bootstrap, de routes, et de migrations, puis
appelle `.run()` pour dispatcher la sous-commande CLI du binaire
(`serve`, `migrate`, `queue:work`, etc.). Un par binaire, vit dans
`src/app.rs`. Voir [Cycle de vie des requêtes](lifecycle.md).

### Compteur atomique

Une opération de cache (`Cache::increment`, `Cache::decrement`) qui
mute une valeur numérique en un seul aller-retour sans course
lecture-modification-écriture. Adossé à `INCR`/`DECR` de Redis sur le
magasin Redis, à une garde tenue sur le magasin en mémoire. Voir
[Cache - Compteurs atomiques](cache.md#atomic-counters).

### Authenticatable

Le trait qu'implémente un type d'utilisateur authentifié
(`get_auth_identifier() -> String`, `get_auth_password()`, etc.) pour
que les guards et le middleware puissent lui parler sans connaître la
struct utilisateur concrète. Voir [Authentification](authentication.md).

### Authorizable

Le trait qui donne à un type d'utilisateur les points d'entrée de
policy (`can`, `can_any`, `cannot`) utilisés par le [Gate](#gate). Voir
[Autorisation](authorization.md).

## B

### Planification de backoff

La séquence de délais qu'un worker de queue attend entre les
réessais d'un job qui échoue. `BackoffSchedule::linear`,
`BackoffSchedule::exponential`, ou un `Vec<Duration>` personnalisé. Voir
[Queues - Backoff schedules](queues.md#backoff-schedules).

### Batch (queue)

Un groupe de jobs dispatchés ensemble et suivis comme une unité -
`PendingBatch::new().add(job).add(other).dispatch()` retourne l'id de
batch persisté. Utile quand vous voulez éparpiller du travail et
exécuter un callback quand tout le batch se termine. Voir
[Queues - Queued batches](queues.md#queued-batches).

### `BelongsTo`

Le type de relation inverse de `HasOne`/`HasMany` - l'enfant porte la
clé étrangère, le parent est de l'autre côté. Un des onze types de
relation Eloquent. Voir
[Eloquent - Relations](eloquent.md#relationships).

### `BelongsToMany`

Un type de relation many-to-many qui passe par un troisième modèle
[Pivot](#pivot) de premier ordre. `BelongsToMany<Local, Related,
Pivot>` - le pivot est nommé dans le type, pas synthétisé par
convention de chaîne. Voir
[Eloquent - Relations](eloquent.md#relationships).

### Amorçage

La `bootstrap_fn` que vous enregistrez sur le builder `Application` et
qui s'exécute une fois au démarrage (après la config, avant de
servir). Là où vous liez des services dans le [Conteneur](#conteneur),
enregistrez des observateurs et des écouteurs d'événements, configurez
des en-têtes par défaut, et ainsi de suite. L'analogue Suprnova des
fournisseurs de service de Laravel, réduit à une seule fonction. Voir
[Amorçage de l'application](bootstrap.md).

### Broadcastable

Le trait qu'implémente un [Événement](#événement) quand il doit être
poussé aux abonnés WebSocket au lieu (ou en plus) des écouteurs
locaux en process. Le pont entre le dispatcher d'événements et le
[Broadcast Hub](#broadcasthub). Voir [Diffusion](broadcasting.md).

### `BroadcastHub`

Le trait qui nomme « la chose qui éparpille un message à tous les
abonnés WebSocket d'un canal » - l'implémentation en mémoire
(`InMemoryBroadcastHub`) est le défaut ; l'implémentation sea-streamer
(`SeaStreamerBroadcastHub`) est le déploiement de production
multi-process. Voir
[Broadcasting - Multi-Process Fanout](broadcasting.md#multi-process-fanout).

### Builder (Eloquent)

L'objet de requête fluent retourné par `Model::query()` - la surface
chaînable où vous construisez `where`, `order_by`, `with`, `limit`,
etc. avant `.get()`, `.first()`, ou `.paginate(...)`. Nommé en double :
chaque méthode de filtre existe à la fois sous son nom Laravel
(`db_where`, `db_or_where`) et son synonyme Rust-natif (`filter`,
`or_filter`). Voir [Eloquent - Query builder](eloquent.md#query-builder--dual-api).

### Commande de bus

Une struct sérialisable dispatchée via `Bus::dispatch(cmd)` qui se
route vers un unique `Handler<C>` enregistré. Les commandes de bus
sont pour du travail en process qui doit faire remonter son résultat
à l'appelant - les [Job](#job)s de queue sont pour du travail qui doit
être persisté et réessayé en arrière-plan. Voir [Command Bus](bus.md).

## C

### Driver de cache

Le backend sélectionné (`memory` ou `redis`) derrière la façade
`Cache`. Choisi au démarrage via `CACHE_DRIVER` et exposé à travers le
trait [CacheStore](#cachestore). Voir [Cache](cache.md).

### `CacheStore`

Le trait qui définit le SPI du driver de cache - `get`, `put`,
`forget`, `increment`, etc. `InMemoryCache` et `RedisCache` sont les
implémentations livrées. Voir [Cache - Configuration](cache.md#configuration).

### Cast (Eloquent)

Une transformation bidirectionnelle déclarée avec `casts!` sur un
modèle Eloquent - type de colonne BD ↔ type Rust. 22 casts intégrés
sont livrés (`AsBool`, `AsDateTime`, `AsJson`, `AsEncrypted`, `AsArray`,
etc.) ; un trait `Cast` implémenté par l'utilisateur couvre tout le
reste. Voir
[Eloquent - Casts](eloquent.md#casts).

### Chaîne (queue)

Une séquence de [Job](#job)s liés de sorte que chacun ne s'exécute que
si le précédent a réussi. Construite avec `PendingChain::dispatch` /
`Queue::chain`. Voir [Queues - Queued chains](queues.md#queued-chains).

### Canal (diffusion)

Le trait vers lequel un événement diffuse - `PublicChannel`,
`PrivateChannel`, ou `PresenceChannel`. La struct de canal se nomme
elle-même (`fn name() -> String`) et autorise la connexion
(`fn authorize(...)`) ; les canaux privés et de présence ajoutent des
bornes de trait plus strictes. Voir [Broadcasting - Channels](broadcasting.md#channels).

### Canal (notification)

Le trait qui route une [Notification](#notification) vers un
mécanisme de livraison - mail, base de données, diffusion, web push.
Une notification nomme ses canaux dans `fn via(...)` ; chaque canal
résout la destination et envoie. Distinct du trait de diffusion qui
porte le même nom. Voir [Notifications - Channels](notifications.md#channels).

### Conteneur

Le registre à trois couches (local à la tâche → local au thread →
global) où les services sont liés et résolus à travers la façade
`App`. L'analogue Suprnova du conteneur de services de Laravel, avec
des couches supplémentaires pour l'isolation par requête et par test.
Voir [Conteneur de service](container.md).

### Contexte (par requête)

Le sac de valeurs typées par requête, atteignable depuis n'importe
quel code dans la même tâche async - `Context::set::<T>(value)`,
`Context::get::<T>()`. Survit aux spawns de tâches quand vous le
propagez explicitement. Distinct du contexte de feature flag qui
partage le nom. Voir [Contexte](context.md).

### CORS

Cross-Origin Resource Sharing. La règle de sécurité du navigateur qui
filtre un fetch JavaScript de l'origine A vers l'origine B ; Suprnova
livre `CorsMiddleware` pour émettre les en-têtes de réponse qui
signalent quelles requêtes cross-origin sont autorisées. Voir [CORS](cors.md).

### CSRF

Cross-Site Request Forgery. L'attaque contre laquelle une session à
état doit se défendre ; Suprnova livre `CsrfMiddleware` pour exiger un
token correspondant sur chaque requête qui change l'état. Voir
[CSRF Protection](csrf.md).

## D

### Façade `DB`

Le point d'entrée sans modèle vers la base de données -
`DB::table(...)`, `DB::transaction(...)`, `DB::raw(...)`. Pour les
requêtes qui ne correspondent pas à la forme Eloquent (colonnes
dynamiques, agrégats joints, SQL brut). Voir
[Eloquent - Façade DB](eloquent.md#db-facade--model-less-queries).

### Disque

Un backend de stockage nommé, enregistré via la façade `Storage` -
`Storage::disk("s3")`, `Storage::disk("local")`. Chaque disque
implémente [DiskExt](#diskext) et est indexé par son nom
d'enregistrement. Voir [File Storage](filesystem.md).

### `DiskExt`

Le trait qu'implémente chaque backend de stockage - `put`, `get`,
`delete`, `list`, `signed_url`, etc. Adossé à `opendal` en interne ;
livre des adaptateurs pour le fs local, la mémoire, S3, Azure Blob, et
GCS. Voir [File Storage](filesystem.md).

## E

### Eloquent

Toute la couche ORM - trait `Model`, `Builder<M>`, relations, casts,
scopes, observateurs, événements, suppressions logicielles, prunable,
fabriques. Le nom Laravel pour ce que d'autres écosystèmes appellent
un ORM ; dans Suprnova elle repose sur SeaORM (que l'utilisateur ne
devrait pas voir). Voir [Eloquent](eloquent.md).

### Enveloppe (queue)

La struct d'enveloppe (`Envelope { payload, attempts, max_attempts,
delay, ... }`) que le driver de queue sérialise et stocke réellement.
Isole le payload du [Job](#job) de la plomberie de la queue. Voir
[Queues](queues.md).

### Événement

Une struct clonable dispatchée via `EventDispatcher::dispatch(evt)`
et livrée à chaque `Listener<E>` enregistré. Suprnova livre le trait,
la façade (`EventFacade`), l'agrégateur `Subscriber`, et des hooks
pour les [Écouteur en file d'attente](#écouteur-en-file-d-attente)s.
Voir [Événements](events.md).

### Écouteur d'événement

Voir [Écouteur](#écouteur).

## F

### Façade

La convention de nommage pour une struct de taille nulle dont le bloc
`impl` porte l'API publique d'un sous-système - `Cache`, `Mail`,
`Auth`, `Storage`, `Bus`, `Notify`, `Vector`, `DB`, `Schedule`, `App`.
Héritée de Laravel ; dans Suprnova l'implémentation sous-jacente est
résolue à travers le [Conteneur](#conteneur) plutôt que par l'appel
magique de PHP. Voir [Conteneur de service](container.md).

### Fabrique (Eloquent)

La macro `#[derive(Factory)]` et le trait `Factory` qui produisent des
lignes de test réalistes avec des défauts pilotés par `fake` -
`UserFactory::times(5).create_many().await?`. La contrepartie Rust des
fabriques de modèle de Laravel. Voir [Macros - Factories](macros.md#factories).

### Échec fermé

Une politique d'échec de driver où une panne de backend fait rejeter
la requête avec un 5xx - utilisée par la limitation de débit, la
session, et l'idempotence quand « mieux vaut refuser que fuiter ».
L'opposé de l'[Échec ouvert](#échec-ouvert). Configurée via
`BackendErrorPolicy::FailClosed`. Voir [Limitation de débit](rate-limiting.md).

### Échec ouvert

Une politique d'échec de driver où une panne de backend laisse passer
la requête (avec un avertissement journalisé) plutôt que de la
rejeter - utilisée quand la disponibilité prime sur la limite.
Configurée via `BackendErrorPolicy::FailOpen`. Voir [Limitation de débit](rate-limiting.md).

### Flag de fonctionnalité

Un booléen (ou une valeur typée) indexé par nom et évalué contre
l'utilisateur/contexte courant - `feature!(MyFeature)`. Adossé au
trait `Evaluator` ; livre un évaluateur base de données et un
évaluateur mis en cache avec TTL par-dessus. Voir [Flags de fonctionnalité](feature-flags.md).

### Fillable

La liste blanche à la compilation qui dit quelles colonnes de modèle
peuvent être assignées en masse depuis une map d'attributs non
fiables - déclarée sur la struct de modèle via l'attribut
`#[fillable]` ou le trait `Fillable`. Le dual de `#[guarded]`. Voir
[Eloquent - Mass assignment](eloquent.md#mass-assignment).

### Système de fichiers

Tout le sous-système de stockage - la façade `Storage`, les
[Disque](#disque)s enregistrés, le trait [DiskExt](#diskext), la
copie en streaming entre disques. Voir [File Storage](filesystem.md).

### Requête de formulaire

Une struct implémentant `FormRequest` (ou dérivée via `#[request]`)
qui extrait et valide un corps de requête avant que le handler ne
s'exécute. L'analogue composable et type-safe des classes de
form-request de Laravel. Voir [Validation](validation.md).

### `FrameworkError`

L'unique enum vers lequel se convertit chaque échec interne au
framework. Porte sa propre projection `HttpResponse`
(`From<FrameworkError> for HttpResponse`) qui assainit les corps 5xx
et estampille un id de requête. Voir [Modèle d'erreur](error-model.md).

## G

### Gate

Le point d'entrée d'autorisation - `Gate::allows("update-post", user,
post)`. Se résout contre les policies enregistrées (déclarées via la
macro `#[policy]`) et court-circuite sur allow/deny. Retourne une
`GateResponse` (ré-exportée comme le `Response` d'autorisation). Voir
[Autorisation](authorization.md).

### Scope global

Une contrainte de requête appliquée à chaque appel `Model::query()`
jusqu'à retrait explicite (`Builder::without_global_scope`).
Implémenté via le trait `GlobalScope` et enregistré au bootstrap. Voir
[Eloquent - Scopes](eloquent.md#scopes).

### Guard (auth)

La stratégie d'authentification nommée attachée à une requête -
`session` (à état, adossée au cookie), `token` (sans état,
bearer-token). Plusieurs guards coexistent ; `Auth::guard("api")` en
choisit un. Voir [Authentification](authentication.md).

### Guarded

La liste noire à la compilation qui dit quelles colonnes de modèle
*ne peuvent pas* être assignées en masse. Le dual de
[Fillable](#fillable). Voir [Eloquent - Mass assignment](eloquent.md#mass-assignment).

## H

### `HasMany`

Un type de relation one-to-many - le parent porte la clé locale, les
enfants portent la clé étrangère. Un des onze types de relation
Eloquent. Voir [Eloquent - Relations](eloquent.md#relationships).

### `HasManyThrough`

Une relation qui atteint le modèle lié en passant par un troisième
modèle intermédiaire - `Country -> User -> Post`. Voir
[Eloquent - Relations](eloquent.md#relationships).

### `HasOne`

Le sibling à une seule ligne de [HasMany](#hasmany) - le parent porte
la clé locale, l'enfant a la clé étrangère, retourne au plus une
ligne. Voir [Eloquent - Relations](eloquent.md#relationships).

### Façade Hash

Le point d'entrée du hachage de mot de passe - `hash(password)`,
`verify(password, hash)`. Choisit bcrypt ou argon2 via `HASH_DRIVER` ;
`needs_rehash` vous permet de migrer les utilisateurs entre
algorithmes à la connexion. Voir [Hachage](hashing.md).

### Handler

La fonction async qui retourne un `Response` pour une route
appariée - transformée en la forme de handler typée du framework par
la macro `#[handler]`. Composée au bord intérieur de la chaîne de
middleware. Voir [Routage](routing.md), [Contrôleurs](controllers.md).

### `HttpError`

Le trait qu'implémente un type d'erreur défini par l'utilisateur pour
spécifier comment il doit se rendre comme réponse HTTP - statut,
corps, en-têtes. Reflète les exceptions `Renderable` de Laravel. Voir
[Gestion des erreurs](errors.md).

### `HttpResponse`

Le type de réponse HTTP concret produit par les handlers et le
middleware. Enveloppe un code de statut, des en-têtes, et un corps -
la chose réellement écrite sur le réseau. Voir [Réponses](responses.md).

## I

### Clé d'idempotence

Un en-tête fourni par le client (`Idempotency-Key`) qui dit « si vous
avez déjà traité une requête avec cette clé, rejouez la même réponse
au lieu de relancer le handler ». Requis pour un POST/PUT/PATCH/DELETE
sûr en cas de réessai ; Suprnova livre `Idempotency`, `Idempotent`, et
`Replay` pour envelopper les handlers. Voir [Idempotence](idempotency.md).

### Réponse Inertia

Une réponse qui retourne un nom de composant typé plus des props
sérialisées au lieu de HTML - le pont entre un handler Rust et une
page Svelte / React / Vue. Construite avec `Inertia::render(...)` ou
la macro `#[derive(InertiaProps)]` plus `inertia_response!`. Voir
[Frontend](frontend.md), [Réponses Inertia](frontend-inertia-responses.md).

### `InertiaProps`

La macro derive qui génère l'impl `Serialize` plus des métadonnées de
type TypeScript pour une struct utilisée comme props d'une page
Inertia. Pilote la commande `suprnova generate-types`. Voir
[Types TypeScript](frontend-typescript-types.md).

## J

### Job

Une struct sérialisable implémentant le trait `Job` - a une méthode
`handle(self)`, mise en queue via `Queue::push(job)` (ou
`Queue::push_later(job, when)` pour un dispatch différé). Persistée
dans le stockage du driver de queue et exécutée par un worker. Voir
[Queues](queues.md).

### Middleware de job

Les wrappers composables (`WithoutOverlapping`, `RateLimited`,
`ThrottlesExceptions`, `Skip`, `FailOnException`,
`SkipIfBatchCancelled`) qui s'exécutent autour de l'appel `handle`
d'un job. L'équivalent en queue du middleware HTTP. Voir
[Queues - Job middleware](queues.md#job-middleware).

### `JobOutcome`

L'enum discriminé que produit la clôture d'un job -
`Completed`, `Failed`, `Released`, `Deleted`, `Skipped` - rapporté à
travers les événements de cycle de vie du job et le compteur de
métriques de la queue. Voir [Queues](queues.md).

## L

### Collection lazy

La contrepartie en streaming de la [Collection](#collection-eloquent) -
`Model::query().lazy().await` retourne une `LazyCollection<M>` qui
tire les lignes de la base de données par lots plutôt que de charger
chaque ligne en mémoire. Voir
[Eloquent - Itération par chunk et en mode lazy](eloquent.md#chunking-and-lazy-iteration).

### Paginateur à total connu

Le paginateur classique à pages numérotées (`Builder::paginate(per_page)`)
qui exécute la requête plus un `COUNT(*)` - connaît le nombre total de
lignes. Voir [Eloquent - Pagination](eloquent.md#pagination).

### Écouteur

Le trait qu'implémente un handler d'événement -
`Listener<E>::handle(evt)`. Enregistré avec
`EventDispatcher::listen::<E, _>(arc_listener)` ou via l'agrégateur
`Subscriber`. Voir [Événements](events.md).

### Garde de verrou (cache)

Le handle retourné par `Cache::lock(key, ttl).acquire()`, représentant
une exclusion mutuelle à travers les process - `LockGuard`. Relâcher
la garde relâche le verrou ; l'abandonner sans la relâcher compte sur
le TTL. Voir [Cache](cache.md).

### Politique de verrouillage

La politique à l'échelle du projet pour gérer l'empoisonnement de
`std::sync::Mutex` / `std::sync::RwLock` dans un process de longue
durée - deux motifs sanctionnés (convertir-en-erreur ou
récupérer-sur-place) ; jamais de `.lock().unwrap()` nu. Voir
[Politique de verrouillage](lock-policy.md).

## M

### `Mailable`

Le trait qu'implémente un message mail - `subject`, `to`, `cc`, `bcc`,
`view`, pièces jointes. Soit écrit à la main, soit dérivé via la macro
`#[derive(NotificationMailable)]` ; envoyé via
`Mail::to(...).send(MyMail).await`. Voir [E-mail](mail.md).

### Mode de maintenance

Un basculement au moment de la requête qui met l'application hors
ligne pour tout le monde sauf une liste blanche -
`maintenance_mode().set(payload)`. Adossé à `FileMaintenanceMode`
(défaut, un fichier sentinelle) ou `CacheMaintenanceMode` (adossé au
cache pour les déploiements multi-instance) ; servi par
`MaintenanceMiddleware`. Ré-exporté à la racine de la crate.

### Middleware

Un wrapper composable autour d'un handler - voit la requête avant, la
réponse après, et peut court-circuiter en retournant `Err(resp)`.
Enregistré globalement, par route, ou par groupe ; s'exécute dans un
ordre fixe outside-in. Voir [Middleware](middleware.md).

### Modèle

Une struct annotée avec `#[suprnova::model]` qui nomme une table de
base de données. La struct *est* le `Model` SeaORM une fois la macro
expansée - Suprnova ne l'enveloppe pas. Porte le CRUD via le trait
`Model`, la construction de requête via `Model::query()`, les
fabriques, les casts, les scopes, les relations, les observateurs.
Voir [Eloquent](eloquent.md).

### Morph

Abréviation de « polymorphique ». Une relation morph laisse une
seule relation pointer vers l'un de plusieurs types de modèle -
`MorphTo` (propriétaire unique de plusieurs types possibles),
`MorphMany`/`MorphOne` (l'inverse, collectant les enfants morphés),
`MorphToMany`/`MorphedByMany` (many-to-many à travers des types
morphés). Le framework garde un [Registre](#registre) à l'exécution
d'associations `MorphTypeEntry` entre chaînes discriminantes et types
Rust. Voir [Eloquent - Relations](eloquent.md#relationships).

### Mutateur

Une transformation en écriture déclarée avec la macro `#[mutator]` -
s'exécute chaque fois que la propriété est définie, avant que la
valeur ne soit stockée sur le modèle. Le dual d'un
[Accesseur](#accesseur). Voir
[Eloquent - Accesseurs et mutateurs](eloquent.md#accessors-and-mutators).

## N

### Notifiable

Le trait qu'implémente un utilisateur (ou tout objet pouvant recevoir
des notifications) - `route_for(channel)` retourne l'adresse pour le
canal nommé (adresse mail, abonnement push, id utilisateur de
diffusion, etc.) ou `None` pour sauter. Voir
[Notifications - The Notifiable Trait](notifications.md#the-notifiable-trait).

### Notification

Le trait qu'implémente un message de notification - `channels()`
retourne la liste des noms de canaux vers lesquels elle doit
s'éparpiller ; chaque canal rappelle la notification (via des traits
par canal comme les méthodes de payload `MailRendering` /
`DatabaseChannel`) pour le payload spécifique au canal. Dispatchée via
`Notify::send(&user, &notif).await`. Voir [Notifications](notifications.md).

## O

### Observateur

Une struct implémentant `Observer<M>` qui écoute les événements de
cycle de vie d'un modèle Eloquent - `creating`, `created`, `updating`,
`updated`, `deleting`, `deleted`, `saving`, `saved`, `retrieved`,
`replicating`, etc. Enregistrée via la macro
`#[suprnova::observer(M)]` ; drainée de l'inventaire au démarrage.
Voir
[Eloquent - Observateurs et événements de cycle de vie](eloquent.md#observers-and-lifecycle-events).

### `OriginPolicy`

Le choix d'application du middleware CSRF pour l'en-tête `Origin` sur
les requêtes qui changent l'état - `Strict` (doit correspondre à
l'hôte), `AllowList`, ou `None`. Voir [CSRF Protection](csrf.md).

## P

### Paginator

Le résultat d'un appel `.paginate(...)` - une de trois saveurs.
`LengthAwarePaginator` (pages numérotées avec un `COUNT(*)`),
`Paginator` (suivant/précédent, pas de total), `CursorPaginator`
(curseur opaque pour une itération stable sur un jeu de résultats qui
bouge). Les trois se sérialisent vers un payload JSON de forme
Laravel. Voir [Eloquent - Pagination](eloquent.md#pagination).

### Limite de panique

Le wrapper `AssertUnwindSafe(...).catch_unwind()` autour de la chaîne
de middleware (et autour de chaque handler de worker en arrière-plan)
qui convertit une panique non gérée en un 500 assaini plus un
événement `ErrorOccurred` journalisé. Un filet de sécurité, pas un
contrat - les API publiques devraient quand même retourner un
`Result`. Voir [Cycle de vie des requêtes - Limite de panique](lifecycle.md#5-panic-boundary--execute_chain_safely).

### Fournisseur de paiement

Un type implémentant le super-trait `PaymentProvider` (= `Checkout` +
`Subscription` + `CustomerStore` + `WebhookHandler`). Adaptateurs de
référence : `suprnova-payments-stripe` (passerelle, impl `Payment`
complète) et `suprnova-payments-paddle` (merchant-of-record, pas de
`Payment`). Voir [Paiements](payments.md), [Provider Guide](payments-provider-guide.md).

### Pivot

Le modèle intermédiaire dans une relation
[BelongsToMany](#belongstomany) - un `#[suprnova::model]` de premier
ordre avec sa propre struct, ses casts, et ses timestamps, nommé
explicitement comme troisième paramètre de type
(`BelongsToMany<L, R, P>`). Suprnova ne synthétise pas de pivot
implicite depuis un nom de table. Voir
[Eloquent - Relations](eloquent.md#relationships).

### Canal de présence

Une variante de [Canal](#canal-diffusion) où le serveur suit qui est
actuellement abonné et émet des événements de connexion/déconnexion
avec les métadonnées de chaque membre. Utile pour les indicateurs
« qui est en ligne ». Voir [Broadcasting - Presence Channels](broadcasting.md#presence-channels).

### Canal privé

Une variante de [Canal](#canal-diffusion) qui exige une autorisation à
l'abonnement - `authorize(...)` doit retourner vrai pour l'utilisateur
qui s'abonne. Utile pour les flux de notification par utilisateur.
Voir [Broadcasting - Channels](broadcasting.md#channels).

### Prunable

Le trait qui marque un modèle à suppression logicielle (ou
interrogeable) comme éligible au nettoyage par `model:prune` -
`Prunable::prunable_query()` retourne le builder pour les lignes qui
doivent partir. `MassPrunable` supprime en un seul `DELETE WHERE` ; le
défaut émet des suppressions ligne par ligne pour que les observateurs
se déclenchent. Marqué pour le registre via la macro `#[prunable]`.
Voir [Eloquent - Prunable](eloquent.md#prunable).

## Q

### File d'attente

Tout le sous-système de travail en arrière-plan - la façade `Queue`,
le trait [Job](#job), l'[Enveloppe](#enveloppe-queue), les drivers
(memory, sync, redis, database, null), le worker, les batches, les
chaînes. Voir [Queues](queues.md).

### Driver de file d'attente

Un type implémentant `QueueDriver` (push, pop, release, etc.) - livre
`MemoryQueueDriver`, `SyncQueueDriver` (exécution en ligne),
`RedisQueueDriver`, `DatabaseQueueDriver`, `NullQueueDriver`. Choisi
au démarrage via `QUEUE_DRIVER`. Voir
[Queues - Drivers](queues.md#drivers).

### Worker de file d'attente

La boucle de longue durée qui tire les enveloppes du driver de queue,
exécute le middleware de job autour du handler, et rapporte le
résultat. Démarre à travers le même cycle de vie que le serveur HTTP,
si bien que les observateurs et les écouteurs se déclenchent
identiquement. Démarré par `cargo run -- queue:work`. Voir
[Queues](queues.md).

### Écouteur en file d'attente

Un `Listener<E>` qui, quand il est invoqué, persiste le payload de
l'événement dans la queue et exécute `handle` dans un worker en
arrière-plan plutôt qu'en process. Utile quand un écouteur d'événement
fait de l'E/S qui ne devrait pas bloquer le chemin de dispatch.
Enveloppé via l'adaptateur `QueuedListener`. Voir [Événements](events.md).

## R

### Limiteur de débit

Tout le sous-système de limitation de débit - `RateLimiter` (la
façade adossée au cache), le builder `Limit`, `SlidingWindowConfig`
(driver à fenêtre glissante), `RateLimitMiddleware` (monté sur route),
`ThrottleRequestsMiddleware` (alias nommé à la Laravel),
`BackendErrorPolicy` (échec ouvert vs échec fermé). Voir
[Limitation de débit](rate-limiting.md).

### Redirection

Un [HttpResponse](#httpresponse) spécialisé enveloppant un en-tête
`Location` - construit via `Redirect::to(...)`,
`Redirect::route(...)`, `Redirect::back()`, avec des chaînes
`.with(...)`/`.with_input(...)` pour les données flash. Voir
[Génération d'URL](urls.md), [Réponses](responses.md).

### Registre

Une table de correspondance globale au processus, peuplée soit à la
compilation par `inventory` (`ModelEntry`, `RelationEntry`,
`MorphTypeEntry`, `ObserverEntry`, `PrunerEntry`, `TaskEntry`,
`PaymentProviderEntry`, `CommandEntry`), soit au démarrage par
enregistrement explicite (`ConnectionRegistry`, `MiddlewareRegistry`,
`InertiaRegistry`, `ChannelRegistry`, `VectorRegistry`,
`SupervisorRegistry`). Tous sont drainés ou interrogés pendant la
séquence de démarrage.

### Relation

Le trait qu'implémente chaque type de relation - `BelongsTo`,
`HasOne`, `HasMany`, `BelongsToMany`, `HasOneThrough`,
`HasManyThrough`, `MorphTo`, `MorphOne`, `MorphMany`, `MorphToMany`,
`MorphedByMany`. Un modèle déclare ses relations comme des méthodes
retournant une struct de relation ; le framework pilote depuis le
trait le chargement hâtif, `with(...)`, les requêtes d'existence de
relation, et les touches en cascade. Voir
[Eloquent - Relations](eloquent.md#relationships).

### Requête

La struct de requête typée du framework - enveloppe la requête hyper
sous-jacente et expose `req.param("id")`, `req.json::<T>()`,
`req.form_data()`, `req.flash()`, etc. Ré-exportée comme
`suprnova::Request`. Voir [Requêtes](requests.md).

### `Response`

Suprnova lie `http::Response` à `Result<HttpResponse,
HttpResponse>` - les deux branches portent un `HttpResponse`. Les
corps de handler retournent `Response`, propagent le travail faillible
avec `?`, et le runtime réduit les deux branches avec
`result.unwrap_or_else(|e| e)`. Le type de décision d'autorisation est
ré-exporté comme `GateResponse` pour éviter la collision. Voir
[Réponses](responses.md),
[Cycle de vie des requêtes](lifecycle.md#the-response-contract).

### Ressource

Deux choses sans rapport partagent le nom ; les deux sont livrées.

1. **Ressource JSON:API** - une struct `#[derive(Resource)]` qui
   sérialise un modèle vers la forme JSON:API avec sparse fieldsets et
   includes. Voir [API Resources](eloquent-resources.md).
2. **Routage de ressource** - un helper de route qui monte un
   ensemble CRUD `index`/`show`/`store`/`update`/`destroy` contre une
   impl `ResourceController`. Voir [Routage](routing.md).

### macro `routes!`

La macro à la compilation qui expanse un DSL de routage
(`get!("/users", users::index)`, `group!`, `middleware!(Auth)`) en
une fonction factory `Router`. La source unique de vérité de route
pour une application. Voir [Routage](routing.md), [Macros](macros.md).

## S

### Scope (local)

Un fragment de requête réutilisable déclaré sur un modèle Eloquent
avec la macro `#[scopes(Model)]` -
`Post::query().published().recent().get()`. Les scopes locaux sont
désactivés par défaut ; ils ne s'exécutent que lorsqu'ils sont
invoqués. La contrepartie du [Scope global](#scope-global). Voir
[Eloquent - Scopes](eloquent.md#scopes).

### Seeder

Un type implémentant le trait `Seeder` qui peuple la base de données
avec des données de départ - enregistré via `suprnova db:seed`.
Souvent adossé à une [Fabrique](#fabrique-eloquent). Voir [Eloquent](eloquent.md).

### URL signée

Une URL dont la query string porte une signature HMAC
(`?signature=...&expires=...`) prouvant qu'elle a été produite par
l'application et n'a pas été altérée. Construite via `sign_url(...)`
/ `sign_route(...)` ; vérifiée par le middleware ou via
`verify_signature(...)`. Voir [URL Generation - Signed URLs](urls.md#signed-urls).

### Suppressions logicielles

Le motif où supprimer une ligne de modèle définit un timestamp
`deleted_at` au lieu d'émettre un `DELETE`. Opt-in par modèle via
`soft_deletes = true` sur l'attribut `#[suprnova::model]` ;
`Model::query()` filtre automatiquement les lignes mises à la
corbeille ; `with_trashed()` et `only_trashed()` les réintègrent. Voir
[Eloquent - Deleting and soft deletes](eloquent.md#deleting-and-soft-deletes).

### Façade `Storage`

Le point d'entrée du sous-système de système de fichiers -
`Storage::disk("s3")`, `Storage::disk("local")` - retournant une
implémentation [DiskExt](#diskext). Voir [File Storage](filesystem.md).

### Subscriber

Un agrégateur qui enregistre plusieurs écouteurs en un seul appel -
implémente `Subscriber::subscribe(dispatcher)` et est enregistré via
`EventDispatcher::subscribe(subscriber)`. Voir [Événements](events.md).

### Superviseur

Le trait qu'implémente un acteur d'arrière-plan de longue durée
(`Supervisor::run`) pour vivre sous le `SupervisorRegistry`. Le
registre attrape les paniques dans la boucle d'exécution, applique
une `RestartPolicy`, et relance. L'équivalent Rust du motif de
supervision `gen_server` d'Erlang. Voir [Superviseurs](supervisors.md).

## T

### Tâche

Une struct implémentant le trait `Task` - déclare une expression cron
ou une fréquence de plus haut niveau (`daily()`, `every_minute()`) et
s'exécute sur le planificateur. Découverte à la compilation via
l'inventaire `TaskEntry`. Voir [Planification de tâches](scheduling.md).

### Middleware terminable

Middleware qui enregistre un hook à exécuter *après* que la réponse a
été écrite au client - implémenté via le trait `Terminable`, capturé
dans un `TerminationSnapshot`, et dispatché par
`dispatch_termination`. Utile pour la journalisation, les vidages de
métriques, l'audit post-vol. Voir [Middleware - Terminable middleware](middleware.md#terminable-middleware-post-response-hooks).

### Through (relation)

Une relation qui passe par un troisième modèle intermédiaire -
[HasManyThrough](#hasmanythrough) et `HasOneThrough`. Voir
[Eloquent - Relations](eloquent.md#relationships).

### Délai d'attente

Le middleware qui borne le temps horloge d'une seule requête et
retourne 504 quand la borne est dépassée - `TimeoutMiddleware`.
Distinct des timeouts de worker de queue (`TimeoutExceeded` côté
queue) et des timeouts de client HTTP. Voir [Timeout](timeout.md).

### `TypedCommand`

Le trait côté console - implémenté par les structs
`#[derive(Command)]` - qui donne à une commande console des arguments
typés (via `clap`) et une méthode async `handle(self)`. Enregistré
dans l'inventaire `CommandEntry` à la compilation. Voir [Console](console.md).

## U

### `UserId`

L'identifiant chaîne opaque retourné par `Auth::id()` - quelle que
soit la clé stable sur laquelle le fournisseur d'utilisateurs
configuré s'indexe, porté comme une `String` de bout en bout. Avec
`EloquentUserProvider<User>` c'est la clé primaire mise en chaîne ;
avec un fournisseur adossé à torii c'est l'id utilisateur émis par
torii. Les sessions stockent l'`UserId` ; les recherches du
fournisseur d'utilisateurs le traduisent vers la struct utilisateur
concrète. L'indirection intentionnelle (une chaîne, pas un type fixe)
vous laisse échanger de backend utilisateur sans réécrire le code de
handler. Voir [Authentification](authentication.md).

## V

### VAPID

Voluntary Application Server Identification - la spec IETF pour
identifier un expéditeur de web-push. Suprnova livre `VapidKey`,
`VapidSigner`, `VapidClaims`, et le `WebPushClient` qui signe chaque
requête push. Voir [Web Push](web-push.md).

### Façade `Vector`

Le point d'entrée du sous-système de recherche vectorielle -
`Vector::driver("qdrant").await?.upsert(...)`. Adossé à des
implémentations `VectorDriver` : en mémoire, Qdrant, Pinecone
(derrière une feature), MariaDB natif. Voir [Vector Search](vector.md).

### `VectorDriver`

Le trait qu'implémente chaque backend vectoriel - `upsert`,
`search`, `delete`, `count`. Permet au framework de supporter
plusieurs bases vectorielles sans en imposer une seule. Voir
[Vector Search](vector.md).

## W

### Web Push

Le protocole de notification push de la plateforme web - des
payloads chiffrés livrés à travers le service push du user agent.
Suprnova livre `WebPushClient` (signataire VAPID, parsing de
retry-after, plafond de rejet à 8 KiB) et `WebPushChannel` pour la
livraison de [Notification](#notification). Voir [Web Push](web-push.md).

### Webhook

Une requête HTTP envoyée par un tiers (fournisseur de paiement,
fournisseur d'identité, …) dans votre application pour rapporter un
événement. Suprnova traite chaque webhook comme idempotent par
défaut - les adaptateurs de fournisseur implémentent
`WebhookHandler::verify(...)` et stockent l'id d'événement du
fournisseur dans une contrainte `UNIQUE` qui rejette les rejeux. Voir
[Payments - Webhook Handling](payments.md#webhook-handling),
[Idempotence](idempotency.md).

### Flux de travail

Un morceau de travail en arrière-plan à état, de longue durée,
composé d'étapes typées - macros `#[workflow]` et `#[workflow_step]`.
La valeur de retour de chaque étape est persistée, si bien qu'un
redémarrage de worker en plein milieu du flux de travail reprend
depuis la dernière étape terminée. La réponse de Suprnova aux
processus d'arrière-plan multi-étapes qui ne tiennent pas dans un
seul [Job](#job). Voir [Flux de travail](workflows.md).

### `WsConfig`

La configuration WebSocket par route - plafonds de taille de payload
(1 MiB texte / 64 KiB binaire par défaut), taille de frame max,
intervalle de ping, timeout d'inactivité, politique d'origine.
Utilisée par les routes `ws!()`. Voir [WebSockets](websockets.md).

### `WsSocket`

Le handle WebSocket typé du framework remis à un handler `ws!()`.
Scindé en une moitié `Sink` (envoi) et une moitié `Stream`
(réception) via `WsSocket::split()` ; les pings/pongs sont gérés par
une tâche de battement de cœur avec un `AbortHandle`, si bien qu'un
handler abandonné se démonte toujours proprement. Voir [WebSockets](websockets.md).

## Suivant

- [Carte de parité avec Laravel](parity.md) - comparaison
  fonctionnalité par fonctionnalité avec Laravel 13
- [Variables d'environnement](env-vars.md) - chaque `env!` que le
  framework lit
- [Index de la documentation](documentation.md) - la carte des
  chapitres
