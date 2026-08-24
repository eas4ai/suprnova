# Depuis le web en Rust

Vous avez livré des services Rust sur Axum, Actix, Rocket ou hyper
artisanal. Vous connaissez le langage et le runtime. Qu'est-ce que
Suprnova vous apporte vraiment ?

**La couche de productivité.** Routage, contrôleurs, un ORM, migrations,
files d'attente, planification de tâches, authentification, e-mail,
notifications, diffusion, cache, stockage, validation et un pont frontend
typé - tout interconnecté, utilisant les mêmes conventions, prêt pour la
production. Vous écrivez les contrôleurs et les modèles ; vous ne choisissez
pas la disposition.

Si vous avez déjà créé une ou deux vraies applications sur Axum, vous savez
combien d'effort était de l'interconnexion plutôt que des fonctionnalités.
Suprnova est l'interconnexion, faite une seule fois, avec des avis là où ils
comptent, modulable où ils ne comptent pas.

## Résumé en 30 secondes

```bash
suprnova new myapp --frontend svelte    # scaffolde le backend + SPA + Vite
cd myapp
suprnova db:sync                        # exécute les migrations, régénère les entités
suprnova serve                          # backend + serveur de dev Vite
```

Vous disposez maintenant de :

- Un serveur hyper avec HTTP/1.1 et HTTP/2, mise à niveau WebSocket, arrêt gracieux
- Une couche Eloquent alimentée par SeaORM avec relations, chargement hâtif, suppressions logicielles
- Inertia.js reliant Rust → Svelte 5 avec `#[derive(InertiaProps)]` typé
- Une authentification avec guards et middleware du framework, plus des moteurs
  Magnetar pour le mot de passe, les passkeys, les magic links, OAuth, les
  sessions bearer, le verrouillage et la mémorisation
- Une file d'attente avec drivers mémoire/sync/redis/base de données/null
- Un planificateur cron piloté par le trait `Task`
- Un binaire de console par projet pour `cargo run --bin console <cmd>`
- Cache, stockage (fs/s3/azblob/gcs), e-mail (SMTP + 5 fournisseurs : SES, Mailgun, Postmark, SendGrid, Resend), web push
- Diffusion sur un hub pluggable (sea-streamer par défaut)
- Validation, CSRF, CORS, limitation de débit, idempotence, délais d'attente des requêtes, erreurs structurées

Et un binaire lié statiquement au bout de `cargo build --release`.

## Ce qui se cache dessous

| Domaine | Crate |
|---|---|
| Serveur HTTP | `hyper` + middleware style tower (implémentation propre) |
| Runtime asynchrone | `tokio` |
| Routeur | `matchit` |
| ORM | `sea-orm` (réexporté en tant que `suprnova::sea_orm`) |
| Migrations | `sea-orm-migration` |
| Drivers de base de données | `sqlx` (postgres / mysql / mariadb / sqlite) |
| Sérialisation | `serde` / `serde_json` |
| Validation | `validator` |
| Sessions de navigateur | `SessionMiddleware` du framework et magasins de session pluggables |
| Moteurs d'authentification | `suprnova-magnetar` derrière des façades possédées par le framework |
| Modèles de page | `tera` (pour les corps d'e-mail ; le frontend est Inertia) |
| Crypto | `aes-gcm`, `argon2`, `bcrypt` |
| WebSockets | `hyper-tungstenite` |
| Streaming | `sea-streamer` (backend de diffusion en éventail) |
| OAuth | Registre de fournisseurs et moteur de cérémonie Magnetar |
| Tracing | `tracing` + `tracing-subscriber` |

Vous n'allez généralement pas accéder à ces éléments directement - Suprnova
réexporte ce dont vous avez besoin. SeaORM est le passage le plus profond :
`Entity`, `Column`, `ActiveModel`, `ConnectionTrait`, le générateur de
requêtes, le prélude de migration. L'échappatoire est `use suprnova::sea_orm;`
si vous avez besoin de quelque chose que la surface sélectionnée ne couvre pas.

## Ce que Suprnova ajoute par rapport à Axum pur

Axum est excellent. Il en va de même pour Actix. Et pour Rocket. La raison
pour laquelle Suprnova existe n'est pas que ces frameworks sont mauvais - c'est
que chaque équipe construisant un vrai produit sur eux finit par
réimplémenter la même couche de productivité. Suprnova livre cette couche :

| Capacité | À la main sur Axum | Dans Suprnova |
|---|---|---|
| Macros de routage qui passent à l'échelle de centaines de routes | Builder API, peut devenir bruyant | Macro `routes!` avec groupage, préfixes, middleware, nommage |
| Liaison de modèle d'itinéraire (id du chemin → modèle chargé) | Extracteur personnalisé par type | `#[handler]` résout `post::Model` de `{id}` automatiquement |
| Générateur de requêtes enchaînable style Eloquent | Utilisez SeaORM directement | `Post::query().db_where(...).order_by(...).get().await?` |
| Suppressions logicielles, observateurs, événements de cycle de vie | Construire par modèle | `#[model(soft_deletes)] + impl Observer<Post>` |
| Migrations + génération d'entité | Connecter sea-orm-cli + scripts | `suprnova db:sync` exécute les migrations et régénère les entités |
| Authentification (sessions, fournisseurs, guards) | Assembler tower-sessions + logique propre | `Auth::attempt`, `Auth::user`, `.middleware(AuthMiddleware)` par route |
| Vérification d'e-mail, réinitialisation de mot de passe, 2FA, force brute | Construire les quatre | Tous intégrés, configurables, idempotents |
| File d'attente en arrière-plan | Choisir un driver, écrire des workers | `Queue::push` + `cargo run -- queue:work` |
| Planification cron | Écrire une tâche tokio avec `tokio_cron_scheduler` | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Pont Inertia | Construire les extracteurs + un adaptateur JS | `inertia_response!(&req, "Page", props)` |
| Props frontend typées (Rust → TS) | Écrire un générateur | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| Diffusion (canaux publics / privés / présence) | Connecter un backend de streaming + authentification | Traits `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel` |
| E-mail avec plusieurs fournisseurs | En choisir un, écrire votre propre abstraction | `Mail::driver("ses")` etc., API `Mailable` uniforme |
| WebPush | Lire la spécification, construire un notificateur | `WebPushChannel` fourni, VAPID intégré |
| Validation + demandes de formulaire | Utiliser `validator` + extracteur personnalisé | `#[derive(Data, Validate)]` demandes de formulaire, validation asynchrone |
| Ressources JSON:API | Formater les réponses à la main | `#[derive(Resource)]` |
| Limitation de débit avec politique échouer-ouvert/fermé | Construire | `RateLimiter` + `BackendErrorPolicy` |
| Clés d'idempotence | Construire | `Idempotency::remember(key, ttl, body)` avec rejeu style Stripe |
| CSRF (avec exclusions glob style Laravel) | Construire | `CsrfMiddleware` avec `except` + `except_method` |
| Erreurs structurées avec 5xx aseptisés | Construire | `FrameworkError` / trait `HttpError`, récupération de panique |
| Conteneur avec portées tâche-locale → thread-locale → globales | Écrire votre propre | `App::bind` / `singleton` / `factory` avec isolation appropriée |
| Point de terminaison de santé, id de requête, journalisation structurée | Assembler | Tous activés par défaut |

Le compromis est constitué d'opinions : Suprnova choisit une disposition,
choisit un driver par défaut, choisit une convention de nommage. Vous pouvez
vous en écarter (les drivers sont pluggables, la config est surchargeable,
le conteneur vous permet d'échanger les services), mais les valeurs par défaut
sont conçues pour être le bon choix pour « construire un produit rapidement ».

## Modèles Rust familiers

Vous reconnaîtrez les formes :

```rust
// Un handler retourne `Result<HttpResponse, HttpResponse>` (aliasé Response).
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// Middleware est un trait, pas une fermeture :
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// Le travail en arrière-plan est le trait `Job` - `handle(self)` exécute le travail :
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

Si vous êtes habitué au middleware Tower : le middleware Suprnova est
conceptuellement le même (un wrapper autour de `next`), mais utilise son propre
trait (pas le `Service` de Tower) car les types combinateurs de tower deviennent
désagréables quand vous commencez à imbriquer des extracteurs spécifiques à
l'application. La forme est plus simple ; le modèle mental est le même.

Si vous avez utilisé le modèle d'extracteur d'Axum : la macro `#[handler]` de
Suprnova joue le même rôle, mais se résout via le conteneur de service plutôt que
via des traits, ce qui vous permet d'injecter les services d'application ainsi que
les données de requête. La liaison de modèle d'itinéraire (`Post` de `{id}`) est
intégrée.

Si vous avez utilisé `sqlx` directement : l'ORM de Suprnova repose sur SeaORM,
qui repose sur sqlx. Vous pouvez passer au SQL brut via `DB::select(...)` /
`DB::select_one(...)` ou utiliser `DB::table("name")` pour des requêtes dynamiques
enchaînables ; vous pouvez passer directement à SeaORM pour les choses que la
surface Eloquent ne couvre pas (par exemple, les requêtes `Statement` brutes avec
mappage de résultat personnalisé). Le [chapitre Eloquent](eloquent.md) couvre les
échappatoires.

## Quel est le delta de productivité ?

Choisissez une fonctionnalité que vous avez créée auparavant sur Axum pur.
Suprnova la livre en tant que chapitre :

- **« J'ai construit un système d'authentification une fois et cela a pris deux
  semaines. »** → [Authentification](authentication.md) +
  [Flux d'authentification](auth-flows.md). Définissez la migration, configurez
  le guard, c'est fait.
- **« J'ai écrit mon propre worker de file d'attente avec retry/backoff. »**
  → [Files d'attente](queues.md). `Queue::push` + `cargo run -- queue:work`.
- **« J'ai câblé WebSockets avec hyper-tungstenite une fois. »** →
  [WebSockets](websockets.md). La macro `ws!()` tape le handler ; la mise à
  niveau, le battement de cœur ping/pong, la poignée de main de fermeture et la
  contre-pression sont pris en charge.
- **« J'ai construit un adaptateur Inertia à partir de zéro. »** →
  [Inertia](frontend.md). `inertia_response!(&req, "Page", props)`, avec
  `InertiaProps` générant les types TS.
- **« J'ai construit un limiteur de débit par client. »** →
  [Limitation de débit](rate-limiting.md). Clé configurable, politique
  échouer-ouvert vs échouer-fermé configurable, échouer-fermé retourne 503.
- **« J'ai implémenté la vérification de signature de webhook Stripe + la
  protection contre le rejeu. »** →
  [Paiements : Stripe](payments-stripe.md). Intégré à l'adaptateur, les webhooks
  vont dans une table miroir avec idempotence UNIQUE.

Ce que vous construiriez à la main en deux semaines, vous l'importez en une
seule ligne.

## Ce que vous reconnaîtrez toujours comme « le vôtre »

Quelques choses restent proches du Rust pur car le langage vous donne
quelque chose de mieux qu'une abstraction de framework :

- **Primitives de concurrence.** `tokio::spawn`, `Arc`, `Mutex`, canaux -
  utilisez-les. Le framework ne les enveloppe pas.
- **Types d'erreur.** Vous définissez vos erreurs de domaine. Implémentez le
  trait `HttpError` sur eux pour obtenir un code d'état et un message appropriés
  dans la réponse câblée. Le `FrameworkError` et `AppError` du framework sont des
  échappatoires pour les erreurs transversales et ad-hoc respectivement.
- **Drivers personnalisés.** Cache, file d'attente, e-mail, diffusion, vecteur,
  paiements - chaque sous-système « registre de drivers » accepte des drivers
  personnalisés. Implémentez le trait, enregistrez-le dans `bootstrap.rs`,
  c'est fait.
- **SQL brut quand vous le souhaitez.** `DB::select(...)`, `DB::table(...).get()`
  pour les lignes dynamiques, ou allez complètement à SeaORM. L'ORM s'écarte du
  chemin.
- **Votre propre middleware tower ?** Suprnova ne livre pas d'adaptateur Tower -
  le middleware ici est `impl Middleware`, pas `tower::Service`. Si vous avez
  besoin d'apporter une crate Tower uniquement, vous l'adapteriez à la main. En
  pratique, le système de middleware intégré couvre presque tout ce que vous
  chercheriez. Voir [Middleware](middleware.md).

## Ce à quoi vous renoncez

L'honnêteté compte plus que le marketing :

- **Conventions.** Les modèles vivent ici, les contrôleurs là, les migrations
  là-bas, les observateurs là. L'échaffaudeur choisit. Vous pouvez le combattre ;
  vous ne devriez probablement pas. Les conventions sont celles de Laravel,
  auditées et éprouvées.
- **Une certaine flexibilité dans la façon dont la requête s'écoule.** La chaîne de
  middleware a un ordre extérieur fixe (request-id → globals → middleware
  d'itinéraire → handler). Vous pouvez insérer du middleware n'importe où, mais
  vous ne pouvez pas déplacer les couches request-id ou panic-recovery - ce sont
  des invariants.
- **Les coins façonnés par PHP.** Là où Laravel fait quelque chose parce que PHP,
  Suprnova fait la chose façonnée par Rust à la place - mais nous vous le disons.
  Cherchez les légendes **« Pourquoi Suprnova diverge »** dans les chapitres.

## Pourquoi « inspiré par Laravel » devrait compter pour vous même si vous n'avez jamais écrit de PHP

L'écosystème web Rust est à peu près où celui de PHP était autour de 2009. Les
crates existent ; les modèles n'existent pas. Suprnova porte un ensemble
extrêmement raffiné de modèles d'un framework qui a eu 10+ ans de pression de
production le façonnant. Vous obtenez des modèles qui ont déjà survécu au contact
avec la réalité.

Le coût est que Suprnova *est une opinion*. Si vous voulez un framework minimal
« choisissez-tout vous-même », Axum est bien là et c'est excellent. Si vous
voulez un « framework qui décide les choses pour que vous puissiez vous
concentrer sur le produit », c'est Suprnova.

## Prochaines étapes

- [Installation](installation.md) - `suprnova new`, ce qui est échaffaudé
- [Démarrage rapide](quickstart.md) - créer une petite application en 5 minutes
- [Cycle de vie des requêtes](lifecycle.md) - comment une requête s'écoule, ce qui s'exécute où
- [Conteneur de service](container.md) - comment les services sont liés et résolus
- [Eloquent](eloquent.md) - le chapitre le plus long ; la surface est large

Ou allez n'importe où via [`documentation.md`](documentation.md).
