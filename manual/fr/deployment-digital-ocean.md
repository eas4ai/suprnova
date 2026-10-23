# Déployer sur Digital Ocean

Digital Ocean propose deux cibles de production adaptées à une
application Suprnova : **App Platform** (un PaaS Docker géré -
poussez et oubliez) et un **Droplet** (votre propre VPS, vous gérez
tout). Ce chapitre parcourt les deux. Utilisez App Platform quand
vous voulez des bases de données gérées, des déploiements
automatiques, et le SSL pris en charge pour vous. Utilisez un
Droplet quand vous voulez le contrôle total, faites déjà tourner
d'autres services sur la machine, ou voulez garder une facture plate
quel que soit le trafic.

## Prérequis

- Un [compte Digital Ocean](https://www.digitalocean.com)
- Un projet Suprnova avec un Dockerfile - générez-en un avec :
  ```bash
  suprnova docker:init
  ```
- Un `APP_KEY` pour la production. Générez-en un et gardez-le en
  lieu sûr :
  ```bash
  suprnova key:generate --show
  ```
  Suprnova échoue en mode fermé au démarrage quand `APP_ENV` est
  autre chose que `local` / `development` / `testing` et que
  `APP_KEY` est non défini.
- Un dépôt git (GitHub ou GitLab) - requis pour App Platform ; pour
  les Droplets, vous pouvez aussi pousser une image préconstruite
  vers un registre.

## App Platform

App Platform compile votre Dockerfile, exécute le binaire Suprnova
unique, et vous donne un Postgres géré si vous en voulez un.

### 1. Créer l'application

1. Allez sur [Digital Ocean Apps](https://cloud.digitalocean.com/apps).
2. Cliquez sur **Create App**, connectez GitHub/GitLab, et choisissez
   le dépôt et la branche.
3. App Platform détecte automatiquement le `Dockerfile` à la racine
   du repo.

### 2. Configurer le service web

| Paramètre | Valeur |
|---|---|
| Resource type | Web Service |
| HTTP port | `8765` |
| Run command | laissez vide - le `CMD` du Dockerfile exécute `./app` |
| Health check (HTTP path) | `/_suprnova/health/live` |

Le binaire Suprnova par défaut exécute `serve` avec auto-migrations,
donc le conteneur exécutera les migrations au démarrage puis liera
le listener.

### 3. Ajouter un Postgres géré

1. **Add Resource** -> **Database** -> **PostgreSQL**.
2. Choisissez un plan (Dev Database pour les tests ; un plan
   Production pour du trafic réel).

App Platform injecte `DATABASE_URL` dans chaque composant
automatiquement via le binding `${db.DATABASE_URL}`.

### 4. Variables d'environnement

Dans la section **Environment Variables** de votre composant web,
définissez :

| Variable | Valeur | Notes |
|---|---|---|
| `APP_ENV` | `production` | déclenche la vérification `APP_KEY` en mode fermé |
| `APP_KEY` | sortie de `suprnova key:generate --show` | marquez comme **encrypted** |
| `SERVER_HOST` | `0.0.0.0` | se lie à toutes les interfaces |
| `SERVER_PORT` | `8765` | correspond à `EXPOSE` du Dockerfile |
| `APP_URL` | `https://your-app.ondigitalocean.app` | utilisé par Inertia + les URL signées |

`DATABASE_URL` est fourni automatiquement par le binding de la base
de données gérée ; ne le définissez pas manuellement.

Si vous utilisez Redis pour le cache/les sessions, ajoutez un
cluster Redis géré et définissez `REDIS_URL` sur sa valeur de
binding (`${redis.REDIS_URL}`).

### 5. Déployer

Cliquez sur **Create Resources**. Le premier build prend quelques
minutes (build Rust release + build frontend) ; les builds suivants
utilisent le cache des couches du Dockerfile et s'exécutent bien
plus vite.

### Ajouter un worker de planificateur

Les tâches planifiées (handlers `#[derive(Task)]` enregistrés via
`Schedule::call`) ont besoin d'un processus de longue durée. Ajoutez
un composant Worker qui exécute la même image avec une commande
différente :

1. **Create** -> **Add Resource** -> **Detect from source code**,
   sélectionnez le même dépôt.
2. Définissez le type de ressource sur **Worker**.
3. **Run command** :
   ```bash
   ./app schedule:work
   ```
4. Le worker hérite des variables d'env de l'application, y compris
   `DATABASE_URL` et `APP_KEY`.

Les workers ne reçoivent pas de trafic HTTP. Exécutez exactement
**une** instance de worker - plusieurs planificateurs exécuteraient
chaque tâche plusieurs fois.

Pour les workers de file d'attente (`./app queue:work`), le schéma
est identique ; vous pouvez généralement exécuter plus d'un worker
de file d'attente en toute sécurité, car le driver de file d'attente
coordonne quel worker prend quel job. Consultez [Files
d'attente](queues.md).

### Spec d'application (infrastructure as code)

Pour des déploiements reproductibles, commitez un `.do/app.yaml` :

```yaml
name: my-suprnova-app

services:
  - name: web
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    http_port: 8765
    instance_count: 1
    instance_size_slug: basic-xxs
    health_check:
      # Liveness uniquement - App Platform redémarre le conteneur
      # quand ceci échoue, donc cela ne doit pas dépendre de
      # Postgres. Voir la note sur la vérification de l'intégrité
      # dans Dépannage.
      http_path: /_suprnova/health/live
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: SERVER_HOST
        value: 0.0.0.0
      - key: SERVER_PORT
        value: "8765"
      - key: APP_URL
        value: https://your-app.ondigitalocean.app
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

workers:
  - name: scheduler
    dockerfile_path: Dockerfile
    github:
      repo: your-username/your-repo
      branch: main
      deploy_on_push: true
    instance_count: 1
    instance_size_slug: basic-xxs
    run_command: ./app schedule:work
    envs:
      - key: APP_ENV
        value: production
      - key: APP_KEY
        scope: RUN_TIME
        type: SECRET
        value: ${APP_KEY}
      - key: DATABASE_URL
        scope: RUN_TIME
        value: ${db.DATABASE_URL}

databases:
  - name: db
    engine: PG
    version: "16"
    size: db-s-dev-database
```

Déployez avec le CLI `doctl` :

```bash
doctl apps create --spec .do/app.yaml
```

Définissez le secret `APP_KEY` séparément via l'UI Apps, ou :

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### Domaine personnalisé

Dans **Settings** -> **Domains** -> **Add Domain**, entrez votre
domaine et suivez les instructions DNS. App Platform émet et
renouvelle automatiquement un certificat Let's Encrypt.

Une fois le domaine actif, mettez à jour `APP_URL` en conséquence -
Inertia l'utilise pour l'en-tête X-Inertia-Location et les URL
signées l'utilisent pour l'entrée du hash.

### Mise à l'échelle

- **Horizontale** : augmentez **Instance Count** sur le service web.
  Chaque instance partage le Postgres géré ; plusieurs instances
  exécutant l'auto-migration au démarrage, c'est sûr - Suprnova
  utilise le gestionnaire de migrations à verrou consultatif de
  SeaORM.
- **Verticale** : changez **Instance Size**. Le binaire Rust est à
  l'aise sur le plus petit slug pour les applications à faible
  trafic ; augmentez quand vous commencez à servir des WebSockets ou
  des connexions de longue durée à l'échelle.

Gardez le worker planificateur à un nombre d'instances de **1**.

## Droplet (VPS)

Un Droplet est la voie à suivre quand vous voulez exécuter Suprnova
sur votre propre VPS. La mécanique est identique à tout autre VPS
Linux - service systemd, proxy inverse Caddy, Postgres géré ou
auto-hébergé. Le chapitre [Hetzner VPS](deployment-hetzner.md) est
la procédure canonique pour ce schéma ; tout ce qui s'y trouve
s'applique mot pour mot sur un Droplet. Les seules différences qui
valent la peine d'être signalées :

- **Image** : choisissez **Ubuntu 24.04** ou **Debian 12** dans la
  console du Droplet.
- **Base de données** : vous pouvez utiliser les **Managed
  Databases** de Digital Ocean pour Postgres / MySQL / Redis au lieu
  de les faire tourner sur le Droplet - même histoire `DATABASE_URL`
  / `REDIS_URL`, pointez-les vers l'endpoint géré et Suprnova ne
  voit pas la différence.
- **Sauvegardes** : activez les snapshots de Droplet et les
  sauvegardes quotidiennes des BD gérées dans la console DO.
- **Réseau** : utilisez un **VPC** DO pour garder le Droplet et
  toute base de données gérée sur un réseau privé ; liez le
  listener à `127.0.0.1` et mettez Caddy devant pour le TLS.

Si vous voulez Docker sur un Droplet (plutôt qu'un binaire système),
le schéma docker-compose de [Docker](cli-docker.md) s'y intègre
proprement - remplacez le Postgres auto-hébergé par la base de
données gérée et c'est terminé.

### Pourquoi Suprnova diverge

Le déploiement PHP typique de Laravel a besoin de PHP-FPM + un
opcache + un worker de file d'attente + une entrée cron de
planificateur - au moins trois pièces mobiles, chacune avec sa
propre sémantique de redémarrage. Un déploiement Suprnova est un
binaire unique plus un processus worker optionnel. Le binaire
exécute les migrations, sert le HTTP, gère les WebSockets, et vit
derrière un proxy inverse. Le même binaire, invoqué avec
`./app schedule:work` ou `./app queue:work`, est votre planificateur
ou worker de file d'attente. Le modèle « une image, plusieurs
composants » d'App Platform correspond naturellement à cela - même
Dockerfile pour chaque composant, `run_command` différent par rôle.

## Dépannage

### Le build échoue

La première chose à vérifier est si le Dockerfile compile en local :

```bash
docker build -t myapp .
```

Causes courantes quand le build local fonctionne mais pas celui
d'App Platform :

- **Fichiers de contexte de build manquants** : vérifiez que
  `.dockerignore` n'exclut pas `Cargo.lock` ou le répertoire
  `migrations/`.
- **Mémoire insuffisante pendant le build cargo** : augmentez la
  taille de l'instance de build dans App Settings -> Resources ->
  Build. Les builds Rust release sont gourmands en mémoire.

### L'application démarre, puis plante au démarrage

Vérifiez les logs runtime dans l'onglet **Runtime Logs**. Les deux
échecs de démarrage Suprnova les plus courants sont :

- **`APP_KEY is required when APP_ENV=production`** - générez-en un
  avec `suprnova key:generate --show` et ajoutez-le comme variable
  d'env chiffrée.
- **`SERVER_HOST=…` value invalid** - doit être `0.0.0.0` pour App
  Platform, pas `127.0.0.1` (le loopback n'est pas atteignable
  depuis le load balancer).

### La vérification de l'intégrité échoue

La plateforme ping `/_suprnova/health/live` et attend un 200 dans le
délai configuré. Si ça échoue :

- Confirmez que le chemin est exactement `/_suprnova/health/live`
  (pas `/health`). L'ancien `/_suprnova/health` fonctionne encore si
  c'est ce que votre spec nomme déjà.
- Confirmez que le port est `8765` et correspond à `SERVER_PORT`.
- Pour distinguer « impossible de se lier » de « impossible
  d'atteindre Postgres », sondez la base de données **à la main**
  depuis la console plutôt que depuis la vérification de
  l'intégrité :

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # Sain :    200 {"status":"ok","database":"connected"}
  # Dégradé : 503 {"status":"degraded","database":"error"}
  ```

  Une réponse dégradée signifie que l'application s'est liée mais ne
  peut pas atteindre Postgres - vérifiez le binding `DATABASE_URL`.
  Ne passez pas `-f` : cela fait quitter curl silencieusement sur le
  503, qui est justement le cas que vous essayez de lire.

Ne mettez pas la sonde de base de données dans le `health_check` de
la spec d'application. App Platform redémarre le conteneur quand
cette vérification échoue, donc un incident de base de données
ferait tomber l'application avec elle - le mode d'échec est une
boucle de redémarrage précisément pendant l'incident que vous avez
besoin que l'application survive. Consultez [Utiliser la sonde
appropriée pour la bonne
question](deployment.md#use-the-right-probe-for-the-right-question).

### Les migrations de base de données ne s'exécutent pas

Les migrations s'exécutent automatiquement dans le cadre du
démarrage `./app` par défaut. Si ce n'est pas le cas, vérifiez les
logs runtime pour des erreurs SeaORM. Pour les exécuter manuellement
depuis la console App Platform :

1. Ouvrez l'onglet **Console** sur le composant web.
2. Exécutez `./app migrate`.

Si vous préférez garder les migrations hors du chemin de démarrage,
définissez la run command sur `./app serve --no-migrate` et ajoutez
un **Job** ponctuel à la spec d'application qui exécute
`./app migrate` en pre-deploy.

## Suivant

- [Présentation du déploiement](deployment.md) - l'introduction
  multiplateforme au déploiement (binaire, migrations,
  planificateur, santé)
- [Docker](cli-docker.md) - ce que génèrent `suprnova docker:init`
  et `docker:compose`
- [Configuration](configuration.md) - chaque variable d'env que
  Suprnova lit
- [Variables d'environnement](env-vars.md) - référence complète, y
  compris celles requises en production
- [Déployer sur Hetzner VPS](deployment-hetzner.md) - la procédure
  Droplet s'applique ici mot pour mot
