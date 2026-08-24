# Docker

Suprnova livre deux commandes CLI qui génèrent des artefacts Docker
que vous pouvez adopter tels quels ou modifier. `docker:init` écrit
+ un `Dockerfile` multi-étapes + un `.dockerignore` pour la production. `docker:compose` écrit un
`docker-compose.yml` pour les services de
développement local (base de données, cache, et optionnellement
Mailpit + MinIO). Les deux commandes écrivent dans la racine du
projet courant ; aucune n'essaie de piloter votre runtime de
conteneurs.



## docker:init

Génère un Dockerfile de production accompagné d'un `.dockerignore`
assorti.

```bash
suprnova docker:init
```

La commande refuse d'écraser un `Dockerfile` existant ; supprimez le
fichier existant d'abord si vous voulez régénérer.

### Ce qui est écrit

| Fichier | Objectif |
|------|---------|
| `Dockerfile` | Build en trois étapes : assets frontend, binaire Rust de release, image runtime |
| `.dockerignore` | Exclut `target/`, `node_modules/`, `.env*`, les artefacts de build existants, et les fichiers Docker eux-mêmes |

### Forme du Dockerfile

Le Dockerfile généré utilise trois étapes pour que l'image runtime
ne porte que le binaire compilé plus ses bibliothèques partagées
requises :

1. **`frontend-builder`** - `node:20-alpine`. Installe les
   dépendances npm et exécute `npm run build`, produisant
   `frontend/dist`.
2. **`backend-builder`** - `rust:1.94.0-slim-bookworm`. Met en cache `Cargo.toml`
   + `Cargo.lock` comme couche de dépendances, puis
   copie vos `cmd/`, `src/`, et le `frontend/dist` construit (en tant
   que `public/assets`) et exécute `cargo build --release`.
3. **`runtime`** - `debian:bookworm-slim` avec `ca-certificates` et
   `libssl3`. S'exécute en tant qu'utilisateur non-root `appuser`.
   Copie le binaire en tant que `./app` et le répertoire `public/` à
   côté. Expose le port 8765.

Le `CMD` par défaut de l'image finale est `["./app"]`, qui exécute la
sous-commande `serve` du binaire unifié (serveur web avec
auto-migrations au démarrage). Pour exécuter une sous-commande
différente, redéfinissez la commande au moment de `docker run` :

```bash
# Serveur web (par défaut)
docker run -p 8765:8765 --env-file .env.production my-app

# Exécuter seulement les migrations et quitter
docker run --env-file .env.production my-app ./app migrate

# Exécuter le daemon du planificateur
docker run --env-file .env.production my-app ./app schedule:work

# Exécuter le worker de file d'attente
docker run --env-file .env.production my-app ./app queue:work
```

Passez la config de production via `--env-file .env.production` ou
des flags `-e` individuels. `.env.production` ne devrait jamais être
commité - il est déjà couvert par le `.dockerignore`.

### Bump de la chaîne d'outils Rust

Le Dockerfile fixe `rust:1.94.0-slim-bookworm` pour l’étape de build afin qu’une image nouvellement générée soit reproductible et corresponde à la branche `main` actuelle. Les Dockerfiles personnalisés doivent utiliser la même chaîne d’outils ou une version plus récente.

```dockerfile
FROM rust:1.94.0-slim-bookworm AS backend-builder
```

Épinglez la version de chaîne d'outils qui correspond à ce que
`rust-toolchain.toml` (si vous en avez un) ou votre `rustc --version`
local rapporte.


La branche `main` actuelle utilise SeaORM 2.0, SeaQuery 1.0 et SQLx 0.9. Les applications qui appellent directement SeaORM doivent importer `ExprTrait` pour les méthodes d’expression SeaQuery et utiliser des méthodes de connexion `*_raw` explicites pour les valeurs `Statement` préconstruites. La mise à niveau des dépendances ne nécessite aucune migration des données de l’application.

### Pourquoi Suprnova diverge

Les déploiements Laravel exécutent typiquement **plusieurs processus
par conteneur ou par hôte** : php-fpm pour le web, un worker de file
d'attente, un planificateur, parfois un dashboard Horizon, parfois un
runner Octane. Chacun est sa propre définition de service.

Suprnova compile en **un seul binaire lié statiquement** qui connaît
chaque sous-commande que le framework livre - `serve`, `migrate`,
`queue:work`, `schedule:work`, `workflow:work`, `ssr:start`. La même
image Docker exécute chaque rôle ; la seule chose qui change est la
commande. Cela fait de « web + worker + scheduler » trois services
dans votre orchestrateur qui pointent tous vers le même tag d'image -
un seul build pour faire avancer toute l'application.

## docker:compose

Génère un `docker-compose.yml` qui démarre les services de
développement local.

```bash
suprnova docker:compose [OPTIONS]
```

Comme `docker:init`, celle-ci refuse d'écraser un
`docker-compose.yml` existant. Elle ajoute aussi
`docker-compose.override.yml` à votre `.gitignore` (si un
`.gitignore` est présent) pour que vous puissiez garder des
redéfinitions par développeur en local sans les commiter.

### Options

| Option | Description |
|--------|-------------|
| `--with-mailpit` | Inclut le service de test d'e-mail Mailpit |
| `--with-minio` | Inclut MinIO (stockage d'objets compatible S3) |

Si vous ne passez aucun des deux flags, la commande vous invite
interactivement pour les deux. Passer l'un des deux flags ignore
l'invite et utilise les valeurs de flag que vous avez données.

### Ce que vous obtenez toujours

PostgreSQL et Redis sont écrits dans chaque fichier compose généré :

| Service | Port par défaut | Image |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

Les deux services ont des contrôles de santé, des volumes nommés
persistants, et vivent sur un réseau à portée de projet
(`<project>_network`). L'utilisateur, le mot de passe, et la base de
données Postgres ont pour défaut `suprnova` / `suprnova_secret` /
`suprnova_db`.

### Services optionnels

Quand vous les activez :

| Service | Ports par défaut | Image |
|---------|--------------:|-------|
| Mailpit | 1025 (SMTP), 8025 (UI) | `axllent/mailpit:latest` |
| MinIO | 9000 (S3 API), 9001 (Console) | `minio/minio:latest` |

Mailpit accepte par défaut n'importe quelle authentification SMTP
pour que vous n'ayez pas à configurer d'identifiants pendant le
développement ; l'UI web à `http://localhost:8025` affiche chaque
e-mail que votre application envoie. Les identifiants par défaut de
MinIO sont `minioadmin` / `minioadmin`.

### Démarrer la pile

```bash
# Tout démarrer en arrière-plan
docker compose up -d

# Suivre les logs
docker compose logs -f

# Arrêter et supprimer les conteneurs (les volumes persistent)
docker compose down

# Supprimer aussi les volumes (efface la base de données locale)
docker compose down -v
```

### Câbler `.env` à compose

Le fichier compose utilise la syntaxe `${VAR:-default}` partout, si
bien que vous pouvez redéfinir n'importe quoi en le définissant dans
`.env` ou votre shell. Un `.env` typique pour la pile par défaut :

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit (si activé)
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO (si activé)
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

Pour redéfinir un port (par exemple parce que 5432 est déjà
utilisé), définissez la variable d'env correspondante avant de
démarrer la pile :

```bash
DB_PORT=5433 docker compose up -d
```

L'ensemble complet des ports redéfinissables :

| Variable | Service | Défaut |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | MinIO Console | 9001 |

### Personnaliser le fichier compose

`docker-compose.yml` est à vous pour l'éditer après la génération -
Suprnova ne le régénère ni ne le relit par la suite. Correctifs
courants :

- Échangez `postgres:16-alpine` contre `mysql:8` ou `mariadb:11` si
  vous préférez l'un de ces drivers ; les deux sont de première
  classe dans Suprnova
- Ajoutez une entrée `volumes:` qui monte votre répertoire
  `migrations/` si vous voulez exécuter des migrations dans un
  conteneur ponctuel
- Ajoutez des services supplémentaires (Qdrant, Elasticsearch, Nats)
  de la même façon

## Déploiement en production

Pour un déploiement réel, exécutez `docker:init` et traitez le
`Dockerfile` généré comme votre entrée de build. La plupart des
orchestrateurs (Railway, Fly, Digital Ocean App Platform, Kubernetes)
n'ont besoin que de trois choses :

1. Le tag d'image construit depuis ce `Dockerfile`
2. Un fichier env avec `DATABASE_URL`, `APP_KEY`, et toute clé
   spécifique au driver
3. Une sonde de santé qui pointe vers `GET /_suprnova/health/live`
   (et, si la plateforme distingue les deux, une sonde de préparation
   à `/_suprnova/health/ready`)

La forme mono-binaire signifie que chaque rôle utilise la même
image ; vous déclarez un service « web » exécutant `./app` et un
service « scheduler » ou « worker » exécutant `./app schedule:work`
(ou `./app queue:work`). Les deux lisent le même env, si bien qu'ils
restent synchronisés à chaque déploiement.

Voir [Déploiement](deployment.md) pour la checklist agnostique de
plateforme, et les guides de plateforme pour des exemples entièrement
développés : [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md),
[Hetzner VPS](deployment-hetzner.md).

## Résumé

| Commande | Écrit | Quand l'utiliser |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`, `.dockerignore` | Construction d'images de production |
| `suprnova docker:compose` | `docker-compose.yml` | Démarrage de Postgres/Redis/Mailpit/MinIO en local |

## Suivant

- [Déploiement](deployment.md) - la checklist de déploiement
  agnostique de plateforme
- [Railway](deployment-railway.md) - PaaS géré avec build depuis git
- [Digital Ocean](deployment-digital-ocean.md) - déploiements App
  Platform
- [Hetzner VPS](deployment-hetzner.md) - bare-metal avec systemd +
  Caddy
- [Variables d'environnement](env-vars.md) - chaque variable env que
  le framework lit
