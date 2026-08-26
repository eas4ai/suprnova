# Présentation du déploiement

Une application Suprnova compile en un binaire unique et autonome qui gère
le serveur web, le gestionnaire de migrations, le planificateur et le
worker de file d'attente. Le déploiement se résume à « copier le binaire,
définir quatre variables d'environnement, l'exécuter ». Ce chapitre explique
quelles sont ces quatre variables, ce que font les sous-commandes du binaire
en production et comment le point de terminaison de santé intégré s'intègre
à la sonde de vivacité d'une plateforme. Les procédures pas à pas
spécifiques à chaque plateforme suivent dans [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md) et
[Hetzner](deployment-hetzner.md).

## Le binaire unique

Votre application compile en un binaire unique avec une surface de
sous-commandes clap :

```bash
./app                       # serve (par défaut) - auto-migration, puis HTTP
./app serve                 # serve explicite, avec auto-migration
./app serve --no-migrate    # sert sans exécuter les migrations
./app web:run               # alias de serve

./app migrate               # applique les migrations en attente et quitte
./app migrate:status        # affiche l'état des migrations
./app migrate:rollback [N]  # annule les N dernières migrations (1 par défaut)
./app migrate:fresh         # supprime toutes les tables, puis re-migre - en production
                            # cela exige --force ET une confirmation saisie sur un
                            # terminal interactif ; voir cli-migrations.md

./app schedule:work         # daemon du planificateur - se réveille chaque minute
./app schedule:run          # exécute une fois les tâches dues et quitte
./app schedule:list         # affiche toutes les tâches enregistrées
./app queue:work            # daemon worker de file d'attente
./app workflow:work         # daemon worker de flux de travail

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # quitte le mode maintenance
```

Un binaire signifie une image Docker, un artefact CI, un déploiement à
vérifier. La même image exécute le service web, le planificateur, le
worker de file d'attente et le worker de flux de travail - vous
lancez une sous-commande différente pour chacun.

## Quatre variables d'environnement de production

Suprnova échoue en mode fermé au démarrage si l'environnement de
production est mal configuré. L'ensemble minimum à déployer :

| Variable | Ce qu'elle fait | Mode d'échec |
|---|---|---|
| `APP_ENV` | Sélectionne l'environnement (`production`, `staging`, etc.). | Utilise la valeur par défaut `local` si non défini - votre application s'exécute en mode dev en prod. |
| `APP_KEY` | Clé base64 AES-256 32 bits pour `Crypt`, sessions, cookies et curseurs de pagination. | Le démarrage renvoie une erreur typée et se termine avec un code non-zéro quand `APP_ENV` n'est pas local/dev/test et `APP_KEY` manque ou est malformé. |
| `APP_URL` | URL absolue canonique de votre application (`https://app.example.com`). | Utilise la valeur par défaut `http://localhost:8765`; les URL signées, les redirections, les liens de messages et les URL absolues Inertia utilisent tous cela. |
| `DATABASE_URL` | URL de connexion à votre base de données relationnelle. | Le démarrage refuse de commencer quand `APP_ENV` est `production` ou `staging` et `DATABASE_URL` n'est pas défini - la base SQLite de développement est explicitement rejetée. |

Générez `APP_KEY` une fois avec l'interface de ligne de commande :

```bash
suprnova key:generate           # écrit APP_KEY=… dans ./.env
suprnova key:generate --show    # affiche la clé pour $(…)
```

Pour la rotation des clés, consultez [Chiffrement](encryption.md) -
`APP_KEY_PREVIOUS` (ou `APP_PREVIOUS_KEYS` compatible Laravel)
prend une liste de clés plus anciennes séparées par des virgules pour
le repli de déchiffrement uniquement.

Au-delà des quatre variables requises, les paramètres de production
courants :

| Variable | Défaut | Notes |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | Utilisez `0.0.0.0` dans les conteneurs. |
| `SERVER_PORT` | `8765` | Correspond au port attendu de votre plateforme. |
| `APP_DEBUG` | dérivé de l'env | `false` en production/staging/envs personnalisés. Définissez explicitement si vous voulez des erreurs explicites en staging. |
| `SERVER_MAX_BODY_SIZE` | valeur par défaut par handler | Plafond du corps de requête au niveau du processus. |
| `SERVER_MAX_CONNECTIONS` | non défini (illimité) | Plafond des connexions TCP actives simultanées. Voir ci-dessous. |
| `SERVER_HEALTH_READINESS_TOKEN` | non défini (préparation est publique) | Secret partagé requis pour accéder à la sonde de préparation. Consultez [Vérification de l'intégrité](#vérification-de-l-intégrité). |
| `DB_MAX_CONNECTIONS` | `10` | Taille du pool. |
| `REDIS_URL` | non défini | Requis si vous avez configuré les drivers Redis cache/queue/session. |

Le tableau complet se trouve dans [Variables d'environnement](env-vars.md).

## Base de données recommandée : MariaDB

Suprnova prend en charge SQLite, PostgreSQL, MySQL et MariaDB comme
backends relationnels de première classe. La recommandation dépend de
l'environnement :

- **Développement.** SQLite. Le générateur écrit
  `DATABASE_URL=sqlite://./database.db` afin que `suprnova serve`
  fonctionne sans aucune configuration de base de données.
- **Production.** MariaDB. Il consolide ce qui serait autrement trois
  services séparés (relational + vector + KV cache) en un seul moteur,
  avec des tables versionnées par le système pour l'audit si nécessaire.

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

Utilisez le schéma `mysql://` - le driver MySQL de SeaORM gère MariaDB
nativement, et le `MariaDbVectorDriver` de Suprnova (`VECTOR(N)` + HNSW)
se connecte directement aux charges de travail vectorielles.

Les autres backends relationnels sont aussi de première classe :

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite (pour les tout petits déploiements mono-instance)
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### Pourquoi Suprnova diverge

Les valeurs par défaut de Laravel incitent les nouveaux projets à opter
pour PostgreSQL car PHP + PostgreSQL est la voie bien tracée. Suprnova
choisit la base de données qui offre la posture de production mono-moteur
la plus propre pour une application Rust. Les `VECTOR(N)` (11.7+) de
MariaDB, les colonnes dynamiques et les tables versionnées par le système
signifient qu'un petit à moyen produit peut livrer la recherche, le KV et
l'audit sans ajouter Redis, OpenSearch ou pgvector. PostgreSQL reste
entièrement supporté - la matrice de test du framework s'exécute contre les
trois backends relationnels - mais notre documentation de déploiement conduit
avec le moteur qui minimise les pièces mobiles. Consultez
[Stockage de vecteurs](vector.md) et [Base de données](database.md) pour
les surfaces spécifiques au backend.

## Construire une image de production

Le générateur du scaffolder crée un Dockerfile multi-étapes :

```bash
suprnova docker:init
```

Cela écrit un `Dockerfile` avec trois étapes :

1. **Build du frontend** - `node:20-alpine`, exécute `npm ci && npm run build`
   sur votre application Inertia `frontend/` (Svelte 5, React 19 ou Vue 3.5
   selon votre choix de scaffolder).
2. **Build du backend** - `rust:1.94.0-slim-bookworm`, compile votre crate
   en mode release avec mise en cache des dépendances.
3. **Runtime** - `debian:bookworm-slim`, copie le binaire compilé
   et la sortie Vite, s'exécute comme un utilisateur non-root `appuser`,
   expose le port 8765 et exécute `CMD ["./app"]` (le serveur
   avec auto-migration).

La branche `main` actuelle utilise SeaORM 2.0, SeaQuery 1.0 et SQLx 0.9. Les applications qui appellent directement SeaORM doivent importer `ExprTrait` pour les méthodes d’expression SeaQuery et utiliser des méthodes de connexion `*_raw` explicites pour les valeurs `Statement` préconstruites. La mise à niveau des dépendances ne nécessite aucune migration des données de l’application.

Construisez et exécutez localement pour vérifier avant d'envoyer :

```bash
docker build -t myapp .

# Avec un fichier d'environnement
docker run --rm -p 8765:8765 --env-file .env.production myapp

# Ou avec des variables explicites (les quatre obligatoires)
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

Ne validez jamais `.env.production` (ou tout fichier contenant `APP_KEY` ou
`DATABASE_URL`) dans votre repo. Utilisez le magasin de secrets de votre
plateforme et lisez les valeurs au moment du déploiement.

## Migrations au démarrage

La commande `./app` par défaut (et `./app serve` explicite) applique toute
migration en attente avant de lier le socket. Les deux implications
pratiques :

- **Sûr avec plusieurs instances.** Le gestionnaire de migrations de SeaORM
  prend un verrou consultatif au niveau de la base de données ; le pod le
  plus lent attend, les autres continuent une fois que c'est fait. Vous
  n'avez pas besoin d'une étape « migrer-puis-déployer » séparée pour les
  lancement des versions de routine.
- **Migration échouée = déploiement échoué.** Si une migration génère une
  erreur, le processus quitte avec un code non-zéro avant que le serveur ne
  se lie. La sonde de santé de la plateforme (voir ci-dessous) signale le
  pod comme non sain et le déploiement s'arrête. Corrigez en avant en
  envoyant une migration corrective à la prochaine version.

Pour les pipelines CI qui veulent gater le déploiement sur une migration
réussie avant que un pod accepte du trafic, exécutez les migrations en
une seule fois :

```bash
docker run --rm myapp ./app migrate
# … puis déroulez le déploiement réel
docker run myapp ./app serve --no-migrate
```

`--no-migrate` ignore la phase auto-migration mais démarre toujours le
serveur normalement.

## Les workers comme services séparés

Le planificateur, la file d'attente et les systèmes de flux de travail ont
chacun leur propre sous-commande daemon. En production, exécutez-les comme
des processus séparés sur la même image, partageant le même environnement :

```bash
docker run myapp ./app schedule:work    # une seule instance - voir ci-dessous
docker run myapp ./app queue:work       # met à l'échelle sur N instances
docker run myapp ./app workflow:work    # met à l'échelle sur N instances
```

Deux règles à intérioriser :

- **Exécutez exactement un processus `schedule:work` ou marquez vos tâches
  `.on_one_server()`.** Les répliques du planificateur ne se coordonnent
  pas par défaut : chacune évalue l'horaire indépendamment, donc trois
  répliques exécutent chaque tâche due trois fois. `replicas: 1` est la
  réponse simple ; `.on_one_server()` élit une réplique par tick contre un
  cache partagé et c'est ce que vous voulez si le planificateur doit être
  hautement disponible. Consultez
  [Planification](scheduling.md#running-on-one-server).
- **Les workers de file d'attente et de flux de travail se mettent à
  l'échelle horizontalement.** Les deux tirent le travail d'un magasin
  partagé et utilisent des délais d'expiration de visibilité ou des verrous
  au niveau des lignes pour coordonner ; ajouter des pods augmente le débit.
  `./app queue:work --max-jobs N` fait quitter le worker après N
  emplois afin qu'un superviseur puisse faire tourner le processus - utile
  pour les déploiements release-on-restart.

Consultez [Files d'attente](queues.md), [Planification](scheduling.md) et
[Flux de travail](workflows.md) pour les détails par sous-système.

## Arrêt propre

Chaque processus Suprnova de longue durée - le serveur et les trois
daemons - se vide sur **SIGTERM** ainsi que SIGINT. SIGTERM est ce que
`docker stop`, Coolify, systemd et Kubernetes envoient ; SIGINT est ce que
Ctrl-C envoie. Les deux empruntent le même chemin : arrêter d'accepter du
nouveau travail, terminer ce qui est en vol dans un délai limité, quitter
`0`.

Les fenêtres de grâce sont par sous-système et délimitées à dessein - un
client lent ou une tâche longue ne doit pas pouvoir maintenir un processus
vivant indéfiniment :

| Processus | Attend | Grâce |
|---|---|---|
| `serve` | connexions HTTP en vol | 5s |
| `queue:work` | le travail en vol pour se régler | jusqu'à ce que le travail revienne |
| `schedule:work` | tâches `.run_in_background()` | 30s |
| `workflow:work` | étapes de flux de travail en vol | jusqu'à ce qu'elles reviennent |

**Dimensionnez la grâce de résiliation de votre plateforme au-dessus de
cela.** Docker utilise par défaut 10 secondes, Kubernetes 30. Si la fenêtre
de la plateforme est plus courte que le travail, elle envoie SIGKILL et vous
êtes de retour à perdre les travaux en vol :

```yaml
# docker compose
services:
  worker:
    command: ["app", "queue:work"]
    stop_grace_period: 60s
```

```yaml
# kubernetes
spec:
  terminationGracePeriodSeconds: 60
```

**Un travail tué en vol n'est pas perdu, mais cela coûte une tentative.**
Sa réservation expire et un autre worker le récupère, facturant une
tentative afin qu'un travail qui tue de manière fiable son worker
puisse toujours être lettré mort plutôt que de tourner en boucle
indéfiniment. Consultez [Files d'attente](queues.md#what-counts-as-an-attempt).

**PID 1 est une contrainte réelle.** Un point d'entrée de conteneur
s'exécute en tant que PID 1, et le kernel n'applique pas les dispositions
de signal par défaut à PID 1 - un processus sans handler SIGTERM ne
meurt pas sur SIGTERM, il l'ignore jusqu'à ce que la plateforme abandonne
et envoie SIGKILL. Suprnova installe le handler, donc `CMD ["app",
"queue:work"]` est correct tel qu'écrit et aucun shim `tini` n'est requis.

## Vérification de l'intégrité

Suprnova expose trois chemins de santé intégrés. Le préfixe `_suprnova/`
est réservé afin que vos propres routes ne puissent jamais entrer en
collision avec elles.

| Chemin | Touche | Utiliser pour |
|---|---|---|
| `/_suprnova/health/live` | rien | Vivacité. Répond 200 aussi longtemps que le processus peut servir une requête. |
| `/_suprnova/health/ready` | la base de données | Préparation. 503 quand une dépendance est inaccessible. |
| `/_suprnova/health` | rien, ou la base de données avec `?db=true` | Le point de terminaison d'origine. Se comporte comme l'un ou l'autre ci-dessus. |

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# Sain :    200 {"status":"ok","timestamp":"…","database":"connected"}
# Dégradé : 503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` et `/_suprnova/health?db=true` continuent de
fonctionner exactement comme avant, et rien de ce que vous avez déjà
déployé n'a besoin de changer - le [guide Hetzner](deployment-hetzner.md)
les nomme toujours pour les contrôles ponctuels, et c'est aussi possible
dans vos propres spécifications. Les chemins nommés sont plus clairs, donc
préférez-les dans une nouvelle configuration ; les guides
[Railway](deployment-railway.md), [DigitalOcean](deployment-digital-ocean.md)
et [Docker](cli-docker.md) les utilisent.

### Utiliser la sonde appropriée pour la bonne question

Pointez la vivacité sur `/live` et la préparation sur `/ready`. La
distinction importe plus qu'il n'y paraît : une sonde de **vivacité**
échouée redémarre le pod, tandis qu'une sonde de **préparation** échouée
le retire simplement de l'équilibreur de charge. Câblez une vérification
de base de données dans la vivacité et un accroc de base de données
redémarre chaque réplique que vous avez - au moment précis où la base de
données peut le moins se permettre une ruée de reconnexions.

```yaml
livenessProbe:
  httpGet:
    path: /_suprnova/health/live
    port: 8765
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
```

Le point de terminaison court-circuite avant la chaîne middleware afin
qu'il reste réactif même si un middleware se verrouille ou que le
middleware d'id de requête rejette le trafic.

### Les réponses dégradées ne contiennent pas les détails du driver

Le corps 503 signale `"database":"error"` et rien de plus. Le message
propre du driver - qui nomme les hôtes, les ports, les noms de base de
données et de schéma et les versions du serveur, et pour certaines erreurs
de configuration l'URL de connexion - va au journal au niveau `error!`, où
un opérateur peut le lire et un étranger ne peut pas. Dans les builds de
débogage, il est également inclus dans le corps en tant que
`database_error`, de sorte que le débogage local n'est pas affecté.

### Fermer la préparation

La préparation exécute un aller-retour de base de données pour quiconque
demande. Si le point de terminaison est accessible à partir d'Internet,
définissez un secret partagé :

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

Les sondes doivent alors l'envoyer en tant qu'en-tête :

```bash
curl -H "X-Suprnova-Health-Token: $SERVER_HEALTH_READINESS_TOKEN" \
  http://localhost:8765/_suprnova/health/ready
```

```yaml
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
    httpHeaders:
      - name: X-Suprnova-Health-Token
        value: <the same value>
```

Sans l'en-tête, la préparation répond **404** - la même réponse que
n'importe quel chemin qui n'existe pas, de sorte que le point de terminaison
est invisible plutôt que simplement fermé. La vivacité reste publique de
toute façon, donc vous n'avez pas besoin de mettre le secret dans chaque
manifeste pour garder votre signal restart-on-hang.

Non défini est la valeur par défaut, et la préparation est publique. C'est
délibéré : les configurations que ce manuel et le scaffolder génèrent
appellent tous `?db=true` sans en-tête, et la fermeture par défaut les
briserait.

## Mode de maintenance

Pour dérouler une migration destructive ou mettre le trafic au repos
pendant un incident :

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` écrit un marqueur de maintenance que le middleware lit à chaque
requête. Les requêtes reçoivent un 503 (configurable via `--status`)
avec le message fourni, sauf pour les chemins listés dans `--except`
et pour toute requête qui inclut le secret. `up` retire le marqueur.

Le secret est un identifiant au porteur : quiconque visite `/<secret>`
reçoit un cookie de contournement valable 12 heures. L'échéance est
scellée dans le cookie chiffré et revérifiée à chaque requête, si bien
qu'un cookie capturé cesse de fonctionner à l'heure dite même si le
navigateur ignore son `max-age`. Un cookie dont l'échéance va au-delà
d'un TTL est refusé, avec une petite tolérance pour les écarts
d'horloge entre machines. La correspondance d'URL comme la comparaison
du secret du cookie s'exécutent en temps constant, si bien que le temps
de réponse n'indique pas à celui qui sonde la longueur du préfixe qu'il
a deviné correctement. Préférez `--with-secret`, qui en génère un pour
vous (16 octets aléatoires, 32 caractères hexadécimaux) et affiche
l'URL de contournement, plutôt que de choisir une chaîne mémorisable
pour `--secret` - et traitez-le comme n'importe quel autre identifiant
dans vos notes d'incident.

## Mise à l'échelle

### Web

La mise à l'échelle horizontale est l'histoire par défaut : chaque pod
exécute `./app`, partage `DATABASE_URL` et se connecte au même Redis (si
vous avez configuré les drivers cache/queue/session soutenus par Redis).
L'auto-migration est sûre en raison du verrou consultatif ci-dessus. Les
sessions persistantes ne sont pas requises - l'état de la session vit dans
votre driver de session (base de données ou Redis), pas dans la mémoire du
processus.

### Workers

- **Planificateur.** Exactement une instance, toujours.
- **File d'attente.** Mise à l'échelle horizontale. Si vous avez réparti le
  travail sur plusieurs files d'attente nommées, exécutez un worker par
  file d'attente (ou transmettez des filtres de file d'attente spécifiques au
  driver - consultez [Files d'attente](queues.md)).
- **Flux de travail.** Mise à l'échelle horizontale ; la réclamation et le
  battement de cœur au niveau des lignes coordonnent les workers.

## Plafond de connexion (`SERVER_MAX_CONNECTIONS`)

Par défaut, le serveur accepte un nombre illimité de connexions TCP
simultanées. Dans la plupart des déploiements, un proxy inverse (nginx,
Caddy, Traefik) ou l'équilibreur de charge de la plateforme offre la
première ligne de défense. Si vous voulez un filet de sécurité à l'intérieur
du processus lui-même - pour empêcher un seul pool de clients
misbehaving d'épuiser les descripteurs de fichiers - définissez
`SERVER_MAX_CONNECTIONS` :

```bash
# .env.production - plafonne les connexions simultanées à 1024
SERVER_MAX_CONNECTIONS=1024
```

Quand le plafond est atteint, la **boucle d'acceptation se bloque** (contre-
pression au niveau TCP) jusqu'à ce qu'une connexion existante se ferme ; la
poignée de main en attente reste en attente d'acceptation du noyau. Le
permis est détenu pour la durée de vie complète de chaque connexion et est
libéré au moment où la connexion se termine, de sorte que les emplacements
se remplissent rapidement.

Règles empiriques :

- **Non défini (défaut = illimité).** Correct si vous avez un proxy inverse
  appliquant sa propre limite de connexion, ou si vous exécutez derrière un
  PaaS qui gère la concurrence pour vous.
- **Définir une valeur concrète** si le processus s'exécute directement sur
  Internet ou si vous voulez une défense en profondeur indépendamment de la
  configuration du proxy. Un point de départ typique est 2 × votre nombre
  attendu d'utilisateurs simultanés de pointe, ajusté à la hausse pour les
  connexions de longue durée (WebSocket, SSE).
- **Associez à `LimitNOFILE`** (systemd) ou `ulimit -n` afin que la limite
  de descripteur de fichier du système d'exploitation ne devienne pas le
  plafond surprise. Chaque connexion HTTP coûte un descripteur de fichier ;
  ajoutez la taille de votre pool de base de données et quelques dizaines
  pour l'entretien du système d'exploitation.
- **C'est un filet de sécurité, pas un remplacement pour la limitation de
  débit en amont.** `SERVER_MAX_CONNECTIONS` arrête l'accumulation incontrôlée
  ; votre proxy inverse ou middleware `rate_limit` doit gérer l'accélération
  par client ou par IP.

Les valeurs vides, non analysables ou zéro sont silencieusement traitées
comme non définies afin qu'une erreur de frappe n'empêche pas le serveur de
démarrer.

## Procédures pas à pas par plateforme

La recette ci-dessus s'adapte à chaque PaaS ou VPS moderne. Les trois
chapitres suivants vous présentent les spécificités :

| Plateforme | Style | Procédure pas à pas |
|---|---|---|
| Railway | PaaS avec auto-déploiement depuis git | [Déployer sur Railway](deployment-railway.md) |
| Digital Ocean | App Platform (PaaS) ou Droplets (VPS) | [Déployer sur Digital Ocean](deployment-digital-ocean.md) |
| Hetzner | VPS avec systemd + Caddy | [Déployer sur Hetzner](deployment-hetzner.md) |

## Suivant

- [Variables d'environnement](env-vars.md) - chaque variable env que le framework lit
- [Chiffrement](encryption.md) - `APP_KEY`, rotation des clés, ce qui est chiffré
- [Configuration](configuration.md) - sections de config typées construites sur env
- [Base de données](database.md) - sélection du driver, réglage du pool, division multi-connexion
- [Files d'attente](queues.md) - mise à l'échelle du worker et drivers de file d'attente
