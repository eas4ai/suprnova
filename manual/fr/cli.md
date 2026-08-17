# Présentation CLI

Suprnova livre deux binaires avec des rôles différents. Le `suprnova`
global - installé une fois dans `~/.cargo/bin` - scaffolde de
nouveaux projets, génère du code, démarre des serveurs de dev, et
exécute des migrations. Le `console` par projet, construit depuis le
`src/bin/console.rs` de chaque application, exécute des commandes à
l'exécution qui ont besoin des types compilés de l'application
(seeders, élagueurs, vos propres handlers `#[command]`). Ce chapitre
est la carte ; chaque sous-commande a son propre chapitre dédié dans
les chapitres voisins listés sous [Suivant](#suivant).

## Installation

Le CLI est distribué via `cargo install --git`. Suprnova n'est pas
encore sur crates.io - voir la [note de pré-lancement dans
Installation](installation.md#pre-launch-note) pour savoir pourquoi.

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli
suprnova --version
```

Pour mettre à jour plus tard, passez `--force` :

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli
```

## Les deux binaires

| Binaire | Construit depuis | Utilisé pour |
|---|---|---|
| `suprnova` | `suprnova-cli/` (cette crate) | Scaffolding (`new`), générateurs (`make:*`), lanceur de dev (`serve`), migrations (`migrate*`, `db:sync`), config Docker (`docker:*`), worker SSR (`ssr:*`), génération de clé (`key:generate`), génération de types (`generate-types`) |
| `console` | `src/bin/console.rs` dans votre projet | Commandes à l'exécution qui lient les types de votre application - `db:seed` et `model:prune` intégrées, plus chaque `#[command]` / `#[derive(Command)]` que vous définissez |

Les daemons worker (`schedule:run`, `schedule:work`, `schedule:list`,
`workflow:work`, `queue:work`) reposent sur une troisième surface :
le propre analyseur clap de votre binaire *app*, le même binaire qui
sert HTTP. Le `suprnova` global shelle vers `cargo run --quiet --
<name>` pour ceux-là afin que vous puissiez les lancer depuis le CLI
que vous avez déjà ouvert. Voir [Console](console.md) pour la
répartition complète en trois.

### Pourquoi Suprnova diverge

Laravel résout cela avec un unique script par projet - `php artisan` -
parce que PHP charge le framework et le code utilisateur ensemble à
l'exécution. Rust lie les binaires à la compilation, donc un binaire
`suprnova` global ne peut pas voir statiquement vos seeders,
factories, ou handlers `#[command]`. La scission pragmatique :

- Le travail purement fichier (scaffolding, générateurs, opérations)
  vit sur le binaire `suprnova` global
- Le travail à l'exécution qui a besoin de vos types compilés vit sur
  le binaire `console` par projet
- Les daemons vivent sur votre binaire app/serveur pour qu'ils
  partagent le même chemin de démarrage que `serve`

Vous obtenez l'ergonomie de `php artisan` (`cargo run --bin console
-- db:seed` ou directement `console <name>`) sans le mensonge du lien
statique.

## Commandes en un coup d'œil

La même liste que `suprnova --help` affiche, groupée de la même
façon.

### Créer

| Commande | Description |
|---|---|
| `suprnova new [name]` | Scaffolde un nouveau projet. Voir [`suprnova new`](cli-new.md). |
| `suprnova serve` | Démarre backend + Vite ensemble avec rechargement à chaud. Voir [`suprnova serve`](cli-serve.md). |
| `suprnova dev:tls` | Fait confiance au CA de portless et enregistre une URL de dev `https://<name>.localhost`. Voir [URLs de dev HTTPS](dev-tls.md). |
| `suprnova web:run` | Exécute directement le binaire app (pas de Vite, pas de boucle de recompilation). Exécution locale à la forme de la production. |

### Générer

| Commande | Description |
|---|---|
| `suprnova make:controller <name>` | Scaffolde un contrôleur dans `src/controllers/`. |
| `suprnova make:action <name>` | Scaffolde une action invocable dans `src/actions/`. |
| `suprnova make:middleware <name>` | Scaffolde un middleware dans `src/middleware/`. |
| `suprnova make:migration <name>` | Scaffolde une migration SeaORM dans `src/migrations/`. |
| `suprnova make:inertia <name>` | Scaffolde une page Inertia dans `frontend/src/pages/`. Passez `--data` pour obtenir à la place une struct de props `#[derive(Data, Validate)]` dans `src/props/`. |
| `suprnova make:error <name>` | Scaffolde une erreur de domaine dans `src/errors/`. |
| `suprnova make:task <name>` | Scaffolde une tâche planifiée dans `src/tasks/`. |
| `suprnova make:command <name>` | Scaffolde une commande console `#[derive(Command)]` dans `src/commands/`. |
| `suprnova generate-types` | Émet des types TypeScript depuis chaque struct `#[derive(InertiaProps)]`. `-o <path>` pour redéfinir la sortie, `-w` pour surveiller et régénérer. |

Voir [Générateurs de code](cli-generators.md) pour le détail complet
du scaffold et à quoi ressemble chaque fichier généré.

### Base de données

| Commande | Description |
|---|---|
| `suprnova migrate` | Exécute toutes les migrations en attente. |
| `suprnova migrate:status` | Affiche quelles migrations sont appliquées ou en attente. |
| `suprnova migrate:rollback [--step N]` | Annule les N dernières migrations (1 par défaut). |
| `suprnova migrate:fresh [--force]` | Supprime toutes les tables et relance toutes les migrations. **Destructif.** En production, elle exige `--force` plus une confirmation saisie sur un terminal interactif. |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Exécute les migrations et régénère les entités SeaORM depuis le schéma en vigueur. `--regenerate-models` écrase les fichiers de modèle personnalisés dans `src/models/`. |

`db:seed` n'est **pas** ici - elle vit sur le binaire `console` par
projet parce que le registre de seeders est compilé dans votre
crate. Exécutez-la via `cargo run --bin console -- db:seed` ou
`./target/debug/console db:seed`. Voir [Console](console.md) pour le
motif d'enregistrement.

Voir le [chapitre Migrations](cli-migrations.md) pour le flux de
travail complet des migrations.

### Planification

| Commande | Description |
|---|---|
| `suprnova schedule:run` | Exécute une fois chaque tâche due. La forme adaptée à cron. |
| `suprnova schedule:work` | Daemon en avant-plan qui vérifie chaque minute et exécute les tâches dues. |
| `suprnova schedule:list` | Affiche chaque tâche enregistrée avec son expression cron. |

Chacune d'elles shelle vers `cargo run --quiet -- <name>` contre
votre binaire app/serveur - le même binaire qui sert HTTP - si bien
que les tâches enregistrées et les services amorcés sont visibles.
Voir [Commandes de planification](cli-scheduling.md) et le chapitre
[Planification](scheduling.md).

### Flux de travail

| Commande | Description |
|---|---|
| `suprnova workflow:work` | Démarre le daemon worker de flux de travail. Retire les étapes de flux de travail du registre et les exécute avec la même limite de panique que les handlers HTTP. |
| `suprnova workflow:install` | Dépose les migrations workflow + workflow_steps dans `src/migrations/`. Déjà présentes dans les scaffolds neufs. |

Voir [Flux de travail](workflows.md).

### SSR

| Commande | Description |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | Lance le worker SSR d'Inertia en avant-plan. Retombe sur la variable d'env `SUPRNOVA_SSR_RUNTIME`, puis sur `node` ; le bundle retombe sur `SUPRNOVA_SSR_BUNDLE`, puis sur `frontend/bootstrap/ssr/ssr.js`. |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | Sonde le worker SSR. Retombe sur `SUPRNOVA_SSR_URL`, puis sur `http://127.0.0.1:13714`. Délai d'expiration par défaut 2000 ms. |

Voir [Inertia SSR](frontend.md) pour la configuration de production.

### Déployer

| Commande | Description |
|---|---|
| `suprnova docker:init` | Émet un `Dockerfile` de production multi-étapes + un `.dockerignore`. |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | Émet un `docker-compose.yml` pour le développement local. Postgres + Redis toujours inclus ; Mailpit et MinIO en option. |

Voir [Docker](cli-docker.md) et le chapitre [Déploiement](deployment.md).

### Sécurité

| Commande | Description |
|---|---|
| `suprnova key:generate [--show]` | Génère une clé AES-256 de 32 octets, en base64 URL-safe sans padding (le même format réseau que produit `EncryptionKey::to_base64`). `--show` affiche seulement la clé pour `APP_KEY=$(suprnova key:generate --show)`. |

Voir [Chiffrement](encryption.md) pour ce que `APP_KEY` protège et
comment fonctionne la rotation via `APP_KEY_PREVIOUS`.

## Démarrage rapide

Le chemin le plus courant de « rien n'est installé » à « application
qui tourne » :

```bash
# 1. Installer le CLI
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.3 suprnova-cli

# 2. Scaffolder un projet (interactif - choisit Svelte par défaut)
suprnova new my-app

# 3. Le démarrer
cd my-app
suprnova migrate
npm install
suprnova serve
```

Scaffold non interactif (CI, configuration scriptée) :

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

Scaffold API uniquement (pas d'Inertia, pas de SPA) :

```bash
suprnova new my-api --api
```

Générer du code dans un projet existant :

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # s'enregistre sous le binaire console par projet
suprnova migrate
```

## Obtenir de l'aide

`--help` (ou `-h`) fonctionne sur n'importe quelle sous-commande.
L'aide de premier niveau est formatée à la main (`ui::print_help`) et
groupe les commandes par section ; l'aide par sous-commande vient de
clap et affiche chaque flag avec sa valeur par défaut :

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

Pour le binaire `console` par projet :

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` affiche la version sur sa propre ligne, ce qui est ce que
vous voulez quand vous signalez un bug ou vérifiez si une
installation a pris :

```bash
suprnova --version
# suprnova 1.2.3
```

`-v` et `-V` sont tous les deux acceptés. Le flag généré par clap
n'offre que `-V` ; celui-ci est déclaré à la main pour que
l'orthographe en minuscule - celle que la plupart des gens essaient
en premier - fonctionne aussi. La version apparaît aussi dans la
bannière `--help`, où elle vivait avant que le flag n'existe.

## Suivant

- [`suprnova new`](cli-new.md) - chaque flag que le scaffolder
  accepte et la disposition de répertoires qu'il produit
- [`suprnova serve`](cli-serve.md) - le lanceur de dev : backend +
  Vite + génération de types
- [Générateurs de code](cli-generators.md) - la famille complète
  `make:*` avec les modèles de sortie
- [Migrations CLI](cli-migrations.md) - `migrate`, `migrate:fresh`,
  `db:sync`, et le flux de travail SeaORM
- [Console](console.md) - le binaire `console` par projet,
  `#[command]`, `#[derive(Command)]`, et l'asymétrie à trois binaires
