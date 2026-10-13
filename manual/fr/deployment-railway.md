# Déployer sur Railway

[Railway](https://railway.app) est un PaaS piloté par Git qui compile
votre Dockerfile et l'exécute sur une infrastructure gérée.
Associez-le au Postgres et au Redis gérés de Railway et vous obtenez
une stack de production Suprnova complète, sans serveur à surveiller.
Cette recette part d'une application fraîchement scaffoldée avec
`suprnova new` jusqu'à une URL en ligne.

## Prérequis

- Un [compte Railway](https://railway.app)
- Un projet Suprnova poussé sur GitHub, GitLab, ou Bitbucket
- Un `Dockerfile` et un `.dockerignore` à la racine du repo, générés
  par :
  ```bash
  suprnova docker:init
  ```
- Un `APP_KEY` généré que vous pouvez coller dans les variables de
  Railway :
  ```bash
  suprnova key:generate --show
  ```

`suprnova` n'est nécessaire qu'en local - Railway compile lui-même le
Dockerfile. La crate du framework est récupérée depuis git comme une
dépendance cargo normale pendant le build.

## Provisionner le projet

1. Ouvrez le [tableau de bord Railway](https://railway.app/dashboard),
   cliquez sur **New Project**, et choisissez **Deploy from GitHub
   repo**.
2. Choisissez le dépôt. Railway détecte le `Dockerfile` et lance
   automatiquement le premier build.
3. Pendant la compilation, ajoutez une base de données : **New** →
   **Database** → **Add PostgreSQL**. Railway expose `DATABASE_URL`
   comme variable de référence sur le projet.
4. Ajoutez éventuellement Redis de la même façon (**New** →
   **Database** → **Redis**) si votre application utilise le driver
   Redis cache, session, queue, ou rate-limit. Railway expose l'URL
   de connexion en tant que `REDIS_URL`.

## Câbler les variables

Ouvrez le service web, allez dans **Variables**, et ajoutez la
configuration de production. Utilisez la syntaxe de référence
`${{ }}` de Railway pour récupérer les URL depuis les services de
base de données, afin que les rotations n'exigent pas un nouveau
collage.

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

Quelques points à connaître :

- **`APP_KEY` est obligatoire dans les environnements non-dev.**
  Suprnova échoue en mode fermé au démarrage quand
  `APP_ENV != local|dev|test` et que `APP_KEY` manque ou est
  malformé. Le serveur journalise un message de remédiation et
  quitte avec un code non-zéro - Railway marquera le déploiement en
  échec. Générez la clé avec `suprnova key:generate --show`.
- **`SERVER_HOST=0.0.0.0` est requis.** Railway route le trafic via
  l'interface réseau du conteneur ; se lier à `127.0.0.1` (la valeur
  par défaut locale) ressemblera à une connexion refusée.
- **`SERVER_PORT` correspond à `EXPOSE` dans le Dockerfile.** Le
  Dockerfile généré expose le port 8765. Railway le mappe
  automatiquement vers une URL publique.

## Build et déploiement

Railway compile à chaque push sur la branche connectée. Le Dockerfile
généré par `docker:init` fait :

1. **Étape 1 - Frontend.** Exécute `npm ci` et `npm run build` dans
   `frontend/`. La sortie de Vite atterrit dans `frontend/dist/`.
2. **Étape 2 - Backend.** Exécute `cargo build --release` contre
   votre workspace ; les couches de dépendances mises en cache
   gardent les builds itératifs rapides.
3. **Étape 3 - Runtime.** Une image `debian:bookworm-slim` avec
   `ca-certificates` + `libssl3`, un `appuser` non-root, et le
   binaire `./app` compilé. Le `CMD` par défaut est `./app`, qui
   exécute `serve` avec auto-migration.

Le premier build prend généralement plusieurs minutes (cache Rust
froid) ; les builds suivants sont bien plus rapides grâce à la mise
en cache des couches Docker.

## Ajouter un service de planificateur

Si votre application utilise des planifications `#[derive(Task)]`,
le planificateur a besoin de son propre processus de longue durée.
Ajoutez un second service depuis le même repo :

1. **New** → **GitHub Repo** → choisissez le même dépôt.
2. Nommez-le `scheduler` pour le repérer facilement dans le tableau
   de bord.
3. Sous **Settings** → **Deploy**, définissez **Custom Start
   Command** sur :
   ```bash
   ./app schedule:work
   ```
4. Copiez les mêmes variables (en particulier `APP_KEY` et les
   références de base de données) pour que le worker lise la même
   configuration que le service web.

`schedule:work` est une boucle daemon - elle se réveille une fois par
minute, interroge la planification pour les tâches dues, et les
exécute via le même amorçage que le serveur HTTP. Consultez
[Console](console.md) et le chapitre du planificateur pour le
contrat.

Exécutez exactement une instance du planificateur. Plusieurs
processus `schedule:work` se coordonnent via des verrous adossés au
cache, mais l'attente par défaut est un seul worker.

### Pourquoi Suprnova diverge

Un déploiement Laravel sur Forge ou Vapor câble typiquement un
serveur web (php-fpm + nginx), un worker de file d'attente
(`php artisan queue:work`), et une entrée cron qui invoque
`schedule:run` chaque minute. Trois composants, trois surfaces de
déploiement.

Suprnova compile chaque rôle dans le même binaire. La spec de
service Railway est `./app` pour le rôle web et
`./app schedule:work` pour le planificateur - même image, même
amorçage, argv différent. Il n'y a pas de conteneur php-fpm séparé,
pas d'image de worker séparée, pas de cron hôte. Ajoutez
`./app queue:work` comme troisième service si vous avez des jobs en
file d'attente, et vous obtenez la topologie Laravel complète en
trois services Railway à partir d'un seul Dockerfile.

## Vérifications de l'intégrité et `railway.json`

Pour plus de contrôle sur le déploiement, commitez un `railway.json`
à la racine du repo. Railway le détecte automatiquement.

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "./app",
    "healthcheckPath": "/_suprnova/health/live",
    "healthcheckTimeout": 300,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10
  }
}
```

Suprnova fournit des points de terminaison de santé intégrés qui
court-circuitent avant la chaîne de middleware - ils renvoient un
statut JSON 200 sans passer par l'auth, le CSRF, ou le rate-limiting.
Le préfixe `/_suprnova/` est réservé afin qu'ils n'entrent jamais en
collision avec vos routes.

`healthcheckPath` ci-dessus pointe vers `/_suprnova/health/live`, qui
ne touche à rien. Cet appariement est délibéré : ce service est
configuré avec `"restartPolicyType": "ON_FAILURE"`, donc tout ce que
la vérification de l'intégrité sonde devient un déclencheur de
redémarrage. Le pointer vers la base de données - via
`/_suprnova/health/ready` ou l'ancien `/_suprnova/health?db=true` -
signifie qu'un incident de base de données redémarre chaque réplique
au moment précis où la base de données peut le moins se permettre
une ruée de reconnexions. Sondez la base de données depuis une
vérification de préparation séparée ou votre monitoring, pas depuis
le chemin qui redémarre le processus. Consultez [Utiliser la sonde
appropriée pour la bonne
question](deployment.md#use-the-right-probe-for-the-right-question).

Les deux anciens chemins continuent de fonctionner, donc un service
Railway existant n'a besoin d'aucun changement ; les chemins nommés
sont simplement plus clairs.

## Domaines personnalisés et TLS

1. Dans le service web, ouvrez **Settings** → **Networking**.
2. Cliquez sur **Generate Domain** pour un sous-domaine
   `*.up.railway.app`, ou sur **Custom Domain** pour pointer votre
   propre nom d'hôte vers le service.
3. Mettez à jour le DNS comme Railway l'indique (un `CNAME` pour les
   sous-domaines, un ANAME/ALIAS pour les domaines apex).

Railway provisionne et renouvelle les certificats Let's Encrypt pour
les domaines générés comme personnalisés.

## Migrations en CI/CD

Le `CMD ["./app"]` par défaut exécute les migrations au démarrage, ce
qui convient pour les déploiements à instance unique. Pour les
configurations multi-répliques, découplez l'étape de migration :

1. Ajoutez un **hook de pré-déploiement** ponctuel qui exécute
   `./app migrate` contre la base de données de production avant que
   les nouvelles répliques ne démarrent.
2. Changez la commande de démarrage du runtime en
   `./app serve --no-migrate` pour que les répliques ne se
   court-circuitent pas entre elles.

Le gestionnaire de migrations est idempotent - même si vous ne
séparez pas les étapes, exécuter les migrations à chaque démarrage
est sûr entre répliques. La séparation existe pour que vous puissiez
faire échouer le déploiement tôt sur une mauvaise migration sans
maintenir le rollout ouvert.

## Logs, métriques, rollbacks

L'onglet du service web expose :

- **Deployments** - chaque build par ordre chronologique ; le menu à
  trois points sur un déploiement précédent réussi est le chemin de
  rollback en un clic
- **Logs** - sortie `tracing` du conteneur, avec des champs de log
  structuré (`request_id`, `route`, `status`) prêts pour les filtres
  du visualiseur de logs
- **Metrics** - CPU, mémoire, IO réseau ; utile pour dimensionner
  l'instance à la hausse ou à la baisse

## Dépannage

**Le build échoue sur `cargo build --release`.** Reproduisez en
local avec `docker build -t myapp .`. La cause la plus courante est
un membre du workspace qui compile sur votre machine mais manque
dans le repo - le Dockerfile copie d'abord `Cargo.toml` et
`Cargo.lock`, donc les crates manquantes échouent explicitement.

**L'application renvoie « connection refused ».** Vérifiez que
`SERVER_HOST=0.0.0.0` est défini sur le service. La valeur par
défaut est `127.0.0.1`, vers laquelle Railway ne peut pas router.

**L'application démarre puis quitte avec une erreur de clé.**
`APP_KEY` est non défini ou malformé. Le framework refuse de
démarrer en production sans elle ; recollez la sortie de
`suprnova key:generate --show` dans les variables du service.

**Les migrations échouent au démarrage.** Vérifiez les logs pour
l'erreur SQL sous-jacente. Les causes courantes sont un
`DATABASE_URL` non défini (vérifiez que la référence
`${{ Postgres.DATABASE_URL }}` s'est résolue) ou une migration
exécutée contre une base obsolète (`./app migrate:status` indique ce
qui est appliqué où).

**Le planificateur ne se déclenche jamais.** Vérifiez que la
commande de démarrage est exactement `./app schedule:work` (pas
`schedule:run`, qui exécute les tâches dues une fois puis quitte).
`schedule:list` depuis un déploiement ponctuel confirme que vos
tâches sont enregistrées.

## Suivant

- [Présentation du déploiement](deployment.md) - le modèle de
  binaire unifié que vos services Railway exécutent
- [Docker CLI](cli-docker.md) - ce que `docker:init` et
  `docker:compose` génèrent réellement
- [Configuration](configuration.md) - chargement de `.env`, config
  typée, clés requises
- [Console](console.md) - `schedule:work`, `queue:work`,
  `workflow:work`, et le reste du CLI unifié
- [Déployer sur Digital Ocean](deployment-digital-ocean.md) - la
  même recette sur un PaaS différent
