# Introduction

Suprnova est un framework web pour Rust qui vous donne l'expérience développeur
de Laravel sur Tokio. Vous écrivez des contrôleurs et des modèles de style
Eloquent ; le framework vous donne la concurrence, la sécurité des types et un
déploiement d'un seul binaire.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// Puis n'importe où :
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

Si vous aviez écrit cela en Laravel la semaine dernière, la version Rust ci-dessus
vous semblera identique - même forme de chaîne, mêmes noms de méthodes, mêmes
valeurs par défaut. La différence se situe en dessous : Tokio au lieu de FPM, un
seul binaire au lieu d'un runtime PHP, vérifications de type à la compilation sur
chaque colonne.

## Pourquoi Suprnova existe

Laravel a résolu le problème de productivité pour le développement web backend.
Les modèles fonctionnent. Après dix ans d'affinage, très peu de choses vous
entravent lors de la construction d'un vrai produit. Mais le modèle une requête
par processus de PHP laisse deux choses hors de portée : les connexions bon marché
longue durée (WebSockets, SSE, notifications poussées par le serveur sans
interrogation) et les I/O concurrentes triviales dans un handler de requête.

Rust vous donne les deux gratuitement avec Tokio. Le problème est que
l'écosystème web Rust vous oblige à construire la couche de productivité
vous-même : choisir une crate HTTP, choisir un ORM, choisir un outil de migration,
choisir une file d'attente, assembler le tout, concevoir vos propres conventions.
Chaque application réinvente ce que Laravel a déjà standardisé.

Suprnova est ce qui se passe quand vous copiez les conventions de Laravel sur
Tokio. Vous obtenez :

- **Même surface** - `routes!`, `Auth::user()`, `Cache::remember`,
  `Mail::send`, `Queue::push`, `Storage::disk("s3")`, `Notify::send`,
  `Schedule::call`, `Gate::allows`, le générateur de requêtes Eloquent, les
  suppressions logicielles, les usines, les observateurs, la diffusion, tout
- **Moteur différent** - async partout, connexions longue durée comme
  citoyens de première classe, binaire lié statiquement, pas de pré-fourche,
  pas d'opcache, pas de FPM
- **Sécurité des types** - vos modèles, routes et charges utiles d'événements
  sont vérifiés à la compilation ; les refactorisations cassées n'atteignent
  pas la préparation
- **Une vraie histoire frontend** - Inertia.js se connecte à Svelte 5, React 19
  ou Vue 3.5 avec démarrage, pas d'API séparée à maintenir

## Principes de conception

Ce sont les principes que les auteurs du framework se fixent. Ils expliquent
pourquoi un chapitre dit ce qu'il dit.

**1. La parité provient du journal des modifications de Laravel.** Quand Laravel
sort une fonctionnalité, Suprnova la suit. La ligne de base actuelle est Laravel
13.x et chaque sous-système livré a été audité par rapport à cela. La
[Carte de parité Laravel](parity.md) est le tableau explicite par fonctionnalité.

**2. Diverger intentionnellement quand Rust améliore les choses.** Quand Laravel
a fait un choix façonné par PHP que nous n'avons pas à faire en Rust, Suprnova
en choisit un façonné par Rust et le dit. Le plus grand exemple est la
concurrence : WebSockets, diffusion, workers d'arrière-plan et HTTP/2
server-push sont de première classe, pas boulonnés. Quand vous verrez cela
appelé dans un chapitre, cherchez les boîtes **« Pourquoi Suprnova diverge »**.

**3. Pas de gardiennage.** Laravel restreint certaines fonctionnalités à un
backend (p. ex. recherche vectorielle via Postgres `pgvector`). Suprnova traite
les backends comme des drivers - `Vector::driver("qdrant")`,
`Vector::driver("pinecone")`, `Vector::driver("mariadb")`,
`Cache::driver("redis")`, `Mail::driver("ses")`. Vous choisissez le bon outil ;
nous ne le choisissons pas pour vous.

**4. Suprnova est la surface de l'API.** En interne, nous utilisons SeaORM,
hyper, Tokio, serde, sqlx, validator, lettre et des dizaines d'autres. Rien de
cela ne devrait apparaître dans votre code. Vous dépendez de `suprnova::*`.
Nous réexportons tout ce que vous allez toucher - y compris les `Entity`,
`Column`, `ActiveModel`, `QueryFilter` de SeaORM, etc. - sous la racine du
framework. L'échappatoire (`use suprnova::sea_orm;`) existe pour le cas rare où
la surface organisée ne le couvre pas, mais vous ne devriez presque jamais en
avoir besoin.

## Ce qui est fourni

Une carte non exhaustive. La liste complète est dans [`documentation.md`](documentation.md).

| Domaine | Ce qui est fourni |
|---|---|
| **HTTP** | Macro `routes!`, contrôleurs, middleware, requêtes, réponses, liaison de modèle de route, URL signées, routage des ressources, assistants de redirection, CORS, CSRF, clés d'idempotence, délai d'attente, limitation du débit, erreurs structurées avec récupération de panique |
| **Base de données** | SeaORM sous le capot, multi-driver (Postgres, MySQL, MariaDB, SQLite), migrations, semeurs, constructeur de requêtes, transactions avec points de sauvegarde, fractionnement lecture-écriture multi-connexions |
| **Eloquent** | Macro `#[suprnova::model]`, les 11 types de relations, chargement hâtif, suppressions logicielles, élagable, portées (locales + globales), 16 événements du cycle de vie, observateurs, 22 conversions intégrées, accesseurs/mutateurs, trois paginateurs, itération chunk/lazy/cursor, collections, réplication |
| **Authentification** | Guards du framework, middleware, fournisseurs et sessions navigateur ; Magnetar avec moteurs de mot de passe, passkey, magic-link, OAuth, bearer-session, lockout, remember, auth-epoch, et migration ; vérification d'e-mail soutenue par le fournisseur ; façade de compatibilité TOTP du framework ; macros de politique et gates |
| **Frontend** | Pont Inertia v3, modèles de démarrage Svelte 5 / React 19 / Vue 3.5, `#[derive(InertiaProps)]` typé, rechargements partiels, génération automatique de types TypeScript |
| **Arrière-plan** | File d'attente avec drivers mémoire/sync/redis/base de données/null, lots, chaînes, middleware de travail, stockage des travaux échoués, binaire console `#[command]`/`#[derive(Command)]`, ordonnanceur trait `Task`, travaux avec état de longue durée `#[workflow]`, trait `Supervisor` avec redémarrage automatique de capture de panique, bus de commande, dispatcheur d'événements |
| **Temps réel** | Macro `ws!()` pour les handlers WebSocket typés, canaux de diffusion (publics, privés, présence), diffusion sea-streamer, événements envoyés par le serveur, notifications web (VAPID) |
| **Cache et stockage** | Drivers de cache mémoire, Redis, base de données ; opérations atomiques ; cache étiqueté ; verrous de cache ; système de fichiers avec drivers fs/memory/s3/azblob/gcs ; protection contre la traversée de répertoires ; stockage vectoriel avec plusieurs backends |
| **E-mail et notifications** | Trait `Mailable`, drivers SMTP/SES/Mailgun/Postmark/SendGrid/Resend, aperçus de fichiers RFC 5322, transports mémoire/log, et `Notifiable` avec canaux mail/base de données/diffusion/webpush |
| **Validation et données** | `#[derive(Validate)]`, requêtes de formulaires, validation asynchrone, `#[derive(Data)]` pour les ensembles d'inclusion de rechargement partiel, `#[derive(Resource)]` pour JSON:API |
| **Paiements** | Surface de fournisseur générique (passerelle/MoR/flux de redirection), adaptateurs de référence pour Stripe et Paddle, tables miroir avec idempotence des webhooks, composants de paiement Inertia |
| **Flags de fonctionnalité** | Évaluateur de base de données, évaluateur mis en cache avec TTL, middleware d'indicateur de fonctionnalité, propagation sub-seconde via trait sync |
| **Tests** | `#[suprnova_test]`, `expect!`, `TestDatabase`, contrefaçons pour chaque surface externe (Mail, Notify, Queue, Bus, Events, Storage, Http) |
| **CLI** | Scaffolder `suprnova new` (Svelte/React/Vue), exécuteur dev `serve`, `migrate*`, `db:sync`, `db:seed`, générateurs `make:*`, `model:prune`, binaire console par projet |

## Préparation pour la production

Le framework est de qualité production en étendue et en test. À partir du HEAD
actuel :

- Chaque surface de Laravel 13.x dans les 30 domaines documentés est livrée
- Chaque problème soulevé par un examen de code indépendant a été résolu
- La suite de tests de l'espace de travail réussit à chaque modification
- Chaque API publique dans `framework/src/lib.rs` est documentée - un élément
  public non documenté fait échouer la construction

À partir de **v1.0.0**, l'API publique est stable : les applications épinglent une
étiquette de version (`tag = "v<version>"` - l'étiquette est la version ; il n'y a
pas de publication sur crates.io), et une modification décisive ne se fait que
derrière un bump de version dont la section [CHANGELOG](changelog.md) le dit.

## Choisir un chemin de lecture

| Vous êtes... | Commencer par |
|---|---|
| Un développeur Laravel | [Depuis Laravel](from-laravel.md) |
| Un développeur Rust qui a utilisé Axum/Actix/Rocket | [Depuis le web en Rust](from-rust-web.md) |
| Les deux, ou ni l'un ni l'autre, et vous voulez juste construire | [Installation](installation.md) → [Démarrage rapide](quickstart.md) |
| À la recherche d'une fonctionnalité spécifique | [`documentation.md`](documentation.md) (la table des matières maître) |
| Vous vous demandez « Suprnova a-t-il X ? » | [Carte de parité Laravel](parity.md) |
