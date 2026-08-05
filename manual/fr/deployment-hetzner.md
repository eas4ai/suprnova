# Déployer sur Hetzner VPS

Ce guide couvre le déploiement d'une application Suprnova sur un VPS avec Hetzner Cloud. Les mêmes principes s'appliquent à tout hôte mono-machine - Linode, Vultr, AWS EC2, ou un serveur dédié que vous possédez déjà. Choisissez cette voie quand vous voulez le contrôle total de la machine, un coût mensuel prévisible, et la possibilité de colocaliser Postgres / Redis sur la même machine.

Tout au long du guide, nous utilisons `myapp` comme nom de projet et `myapp.com` comme domaine - substituez les vôtres.

## Prérequis

- Un VPS exécutant Ubuntu 22.04 ou Debian 12
- Un accès SSH à votre serveur
- Un nom de domaine pointé vers l'adresse IP de votre serveur
- Un projet Suprnova - soit un arbre source fonctionnel, soit un Dockerfile généré avec `suprnova docker:init` (voir [Docker](cli-docker.md))

## Configuration du serveur

### 1. Créer un VPS

1. Allez sur [Hetzner Cloud Console](https://console.hetzner.cloud)
2. Créez un nouveau projet et ajoutez un serveur
3. Choisissez **Ubuntu 22.04** comme image
4. Sélectionnez la taille de votre serveur (CX11 convient pour les petites applications)
5. Ajoutez votre clé SSH pour un accès sécurisé

### 2. Configuration initiale du serveur

Connectez-vous en SSH à votre serveur et exécutez la configuration initiale :

```bash
# Mettre à jour les paquets
apt update && apt upgrade -y

# Créer un utilisateur non-root pour votre application
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# Installer les paquets requis
apt install -y curl postgresql redis-server
```

### 3. Configurer PostgreSQL

```bash
# Créer la base de données et l'utilisateur
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **Astuce :**
>
> Pour la production, envisagez d'utiliser un service de base de données géré comme le futur PostgreSQL géré de Hetzner, ou des services comme Neon, Supabase, ou AWS RDS, pour une meilleure fiabilité et de meilleures sauvegardes.


## Options de déploiement

Choisissez l'une des méthodes de déploiement suivantes. Chacune se termine par un binaire (ou conteneur) nommé `app` situé à `/opt/myapp/app`, que l'unité systemd ci-dessous sait exécuter.

### Option A : compiler en local

Compilez sur votre machine et uploadez le binaire. Remplacez `myapp` par le nom réel de votre projet - `cargo build` nomme le binaire d'après `[package].name` dans `Cargo.toml` :

```bash
# Sur votre machine locale - cross-compilez pour Linux (si sur macOS)
cargo build --release --target x86_64-unknown-linux-gnu

# Ou compilez avec Docker pour Linux (le Dockerfile renomme le binaire en `app`)
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# Uploader vers le serveur, en renommant en `app` à l'arrivée
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# ou, si vous avez pris la voie Docker :
scp ./app-linux root@your-server:/opt/myapp/app
```

### Option B : compiler sur le serveur

Installez Rust 1.91.1+ (Suprnova utilise l'édition 2024) et compilez directement sur le serveur :

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Cloner, compiler, et placer le binaire au chemin standard
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # renommer pour que l'ExecStart=/opt/myapp/app de systemd le trouve
```

### Option C : utiliser Docker

Exécutez votre application dans un conteneur Docker - le Dockerfile scaffoldé nomme déjà le binaire runtime `app` (voir [Docker](cli-docker.md)) :

```bash
# Installer Docker
curl -fsSL https://get.docker.com | sh

# Récupérer et exécuter votre image
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

Si vous avez pris la voie Docker, sautez la section systemd et passez à [Proxy inverse Caddy](#proxy-inverse-caddy) - Docker gère la supervision de processus.

## Configuration de l'environnement

D'abord, générez un `APP_KEY` de production sur le serveur (ou en local - c'est la valeur qui compte). `APP_KEY` est une clé AES-256 de 32 octets utilisée par `suprnova::Crypt` pour les cookies de session et les URL signées. Suprnova **échoue en mode fermé au démarrage** quand `APP_ENV` n'est pas `local`/`dev`/`test` et que `APP_KEY` est non défini - ce n'est donc pas optionnel en production :

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

Puis écrivez le fichier env :

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# Base de données - se lier à localhost quand la BD est sur la même machine
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# Session
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis (optionnel - utilisé par les drivers cache, queue, broadcasting)
REDIS_URL=redis://127.0.0.1:6379

# E-mail
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# Sécuriser le fichier - seul l'utilisateur app devrait pouvoir le lire
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

Consultez [Configuration](configuration.md) pour la surface env complète et comment elle devient une config typée.

## Services systemd

Un binaire Suprnova prend en charge plusieurs commandes - `./app` (serve, avec auto-migration), `./app schedule:work` (daemon planificateur), `./app queue:work` (worker de file d'attente), `./app workflow:work` (worker de flux de travail). Chaque processus de longue durée obtient sa propre unité systemd utilisant le même binaire et le même fichier env.

### Service serveur web

Créez `/etc/systemd/system/myapp.service` :

```ini
[Unit]
Description=Suprnova Application
After=network.target postgresql.service redis.service
Requires=postgresql.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app
Restart=always
RestartSec=5

# Environnement
EnvironmentFile=/opt/myapp/.env.production

# Durcissement de sécurité
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

Le `ExecStart=/opt/myapp/app` par défaut exécute `serve` avec auto-migration. Si vous préférez que les migrations soient une étape de déploiement séparée, utilisez `ExecStart=/opt/myapp/app serve --no-migrate` et exécutez `./app migrate` depuis votre script de déploiement avant de basculer le binaire.

### Service planificateur

Si votre application a des tâches enregistrées via `Schedule::call(...)` (voir le chapitre [Planification](cli-scheduling.md)), exécutez **exactement un** processus planificateur pour éviter une exécution en double des tâches. Créez `/etc/systemd/system/myapp-scheduler.service` :

```ini
[Unit]
Description=Suprnova Scheduler
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app schedule:work
Restart=always
RestartSec=5

# Environnement
EnvironmentFile=/opt/myapp/.env.production

# Durcissement de sécurité
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### Worker de file d'attente (optionnel)

Si vous dispatchez des jobs vers une file d'attente, ajoutez `/etc/systemd/system/myapp-queue.service` :

```ini
[Unit]
Description=Suprnova Queue Worker
After=network.target myapp.service
Requires=myapp.service

[Service]
Type=simple
User=app
Group=app
WorkingDirectory=/opt/myapp
ExecStart=/opt/myapp/app queue:work
Restart=always
RestartSec=5

EnvironmentFile=/opt/myapp/.env.production

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

Vous pouvez mettre à l'échelle les workers de file d'attente horizontalement - plusieurs instances de `myapp-queue.service` sur la même machine ou des machines différentes, c'est sûr.

### Activer et démarrer les services

```bash
# Recharger systemd après avoir écrit les fichiers d'unité
systemctl daemon-reload

# Activer les services pour qu'ils démarrent au boot
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # si vous avez ajouté le worker de file d'attente

# Les démarrer maintenant
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# Vérifier
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Proxy inverse Caddy

Caddy gère automatiquement les certificats HTTPS avec Let's Encrypt.

### Installer Caddy

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### Configurer Caddy

Éditez `/etc/caddy/Caddyfile` :

```
myapp.com {
    reverse_proxy localhost:8765

    # Activer la compression
    encode gzip

    # Journalisation
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

Remplacez `myapp.com` par votre domaine réel.

### Démarrer Caddy

```bash
systemctl enable caddy
systemctl start caddy
```

Caddy obtiendra et renouvellera automatiquement les certificats SSL.

## Vérifications de l'intégrité

Suprnova fournit un point de terminaison `/_suprnova/health` intégré qui court-circuite avant la chaîne de middleware et n'entre jamais en collision avec vos routes :

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### Vérifier la connectivité à la base de données

Ajoutez `?db=true` pour aussi vérifier la base de données :

```bash
curl https://myapp.com/_suprnova/health?db=true
```

Réponse saine (HTTP 200) :

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

Si la vérification de la base de données échoue, le point de terminaison bascule en HTTP **503** avec `"status": "degraded"` et un champ `"database_error"` - câblez ceci dans une vérification de l'intégrité de style `livenessProbe` / `readinessProbe` pour que le load balancer puisse retirer une instance non saine de la rotation.

### Monitoring externe

Utilisez le point de terminaison de santé avec des services de monitoring :

- **UptimeRobot** : ajoutez un moniteur HTTP pour `https://myapp.com/_suprnova/health`
- **Better Stack** (anciennement Better Uptime) : configurez le point de terminaison de vérification de l'intégrité avec le déclencheur 503
- **Prometheus / Grafana** : scrapez le corps JSON pour les champs `status` + `database`

## Script de déploiement

Créez un script de déploiement pour des mises à jour atomiques. Remplacez `myapp` par le nom de votre projet (le `[package].name` dans `Cargo.toml`) - c'est ce nom que `cargo build` donne au binaire de sortie :

```bash
#!/bin/bash
# deploy.sh - À exécuter sur votre machine locale

set -e

PROJECT="myapp"               # le nom du package Cargo
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "Compilation de l'application..."
cargo build --release --target x86_64-unknown-linux-gnu

echo "Upload du binaire..."
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "Déploiement..."
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # Arrêter les services de longue durée (ignorer les échecs au premier déploiement)
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # Échange atomique - le renommage est un seul appel système sur le même système de fichiers
    mv app.new app
    chmod +x app

    # Exécuter les migrations explicitement (l'unité fait aussi de
    # l'auto-migration, mais le faire ici révèle les échecs avant que
    # nous relancions le trafic)
    sudo -u app ./app migrate

    # Démarrer les services
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # Vérifier la santé (laisser un instant au serveur pour se lier)
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "Déploiement terminé !"
EOF
```

Rendez-le exécutable :

```bash
chmod +x deploy.sh
./deploy.sh
```

## Logs et monitoring

### Consulter les logs

```bash
# Logs du serveur web
journalctl -u myapp -f

# Logs du planificateur
journalctl -u myapp-scheduler -f

# Logs d'accès Caddy
tail -f /var/log/caddy/myapp.log
```

### Rotation des logs

Le journald de systemd gère la rotation des logs automatiquement. Pour un stockage à long terme, envisagez :

- **Loki + Grafana** : agrégation de logs auto-hébergée
- **Papertrail** : service de journalisation dans le cloud
- **Logtail** : gestion de logs simple

## Configuration du pare-feu

Sécurisez votre serveur avec UFW :

```bash
# Autoriser SSH
ufw allow 22/tcp

# Autoriser HTTP/HTTPS (Caddy)
ufw allow 80/tcp
ufw allow 443/tcp

# Activer le pare-feu
ufw enable
```

> **Avertissement :**
>
> N'exposez jamais le port 8765 directement. Utilisez toujours Caddy comme proxy inverse pour gérer le SSL et les en-têtes de sécurité.


## Mise à l'échelle

Un seul binaire Suprnova est très efficace - un petit VPS gère une quantité surprenante de trafic avant que vous n'ayez besoin de scaler horizontalement. Quand ce sera le cas :

### Mise à l'échelle verticale

Passez le VPS à une instance plus grande pour plus de CPU/RAM. Le binaire, le fichier env, et les unités systemd vous suivent sans changement.

### Mise à l'échelle horizontale

Pour plusieurs instances d'application :

1. Mettez en place un load balancer (Hetzner Load Balancer, HAProxy, ou Caddy sur un nœud dédié)
2. Déplacez Postgres vers un service géré ou un nœud dédié pour que les machines applicatives soient sans état
3. Déplacez les sessions, le cache, et la diffusion vers Redis pour que n'importe quelle instance de l'application puisse servir n'importe quelle requête
4. Déployez plusieurs instances de l'application ; chacune exécute en toute sécurité sa propre auto-migration au démarrage (le gestionnaire de migrations prend un verrou pour que les démarrages concurrents n'entrent pas en collision)
5. Gardez **un seul** planificateur (`schedule:work`) qui tourne sur toute la flotte - les workers de file d'attente peuvent tourner en parallèle sans risque, le planificateur non

### Pourquoi Suprnova diverge

Laravel exécute typiquement PHP-FPM derrière nginx, avec cron déclenchant `schedule:run` une fois par minute et Horizon (ou supervisord) gérant les workers de file d'attente. Suprnova réduit tout cela à un seul binaire avec des sous-commandes. `./app` est un processus Tokio de longue durée - il n'a pas besoin d'un pool de processus devant lui, n'a pas besoin d'un cron séparé, et reste chaud entre les requêtes. systemd est le superviseur à la fois pour le processus web et pour les workers, et Caddy ne fait que ce que nginx ne pouvait pas éviter : terminer le TLS et faire le proxy.

## Dimensionnement

Choisissez un VPS en fonction de la charge de travail, pas d'un nom de palier marketing. La gamme de Hetzner change périodiquement ; la logique de dimensionnement, non :

| Charge de travail | Ordre de grandeur |
|---|---|
| Petit site, faible trafic, SQLite ou BD partagée | Plus petite instance vCPU partagé (1 vCPU / 2 Go) |
| Trafic modéré avec Postgres + Redis sur la même machine | 2 vCPU / 4 Go |
| API plus lourde + planificateur + workers de file d'attente + Postgres | 2–4 vCPU / 8 Go |
| Production à l'échelle | Instance CPU dédié, ou BD séparée sur son propre nœud |

Consultez la [tarification actuelle](https://www.hetzner.com/cloud) de Hetzner pour le catalogue à jour. L'empreinte mémoire au repos de Suprnova est petite (quelques Mo), donc la RAM sert surtout à l'ensemble de travail de la base de données et à votre code métier.

## Dépannage

### Le service ne démarre pas

Vérifiez les logs pour des erreurs :

```bash
journalctl -u myapp -n 50
```

Problèmes courants :
- Variables d'environnement manquantes
- Échec de connexion à la base de données
- Port déjà utilisé

### Erreurs de certificat Caddy

Assurez-vous que :
- Le DNS du domaine pointe vers votre serveur
- Les ports 80 et 443 sont ouverts
- Aucun autre service n'utilise le port 80

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### Problèmes de connexion à la base de données

Testez la connexion manuellement :

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### La vérification de l'intégrité échoue

```bash
# Vérifier si l'application tourne
systemctl status myapp

# Tester le point de terminaison de santé directement
curl http://localhost:8765/_suprnova/health

# Vérifier avec la base de données
curl http://localhost:8765/_suprnova/health?db=true
```

Une réponse `503` avec `"status": "degraded"` signifie que l'application est up mais que la vérification de l'intégrité de la base de données a échoué - inspectez `database_error` dans le corps et vérifiez `DATABASE_URL`, les logs Postgres, et les limites de connexion.

## Suivant

- [Présentation du déploiement](deployment.md) - l'histoire agnostique de la plateforme pour les déploiements en binaire unique
- [Docker](cli-docker.md) - détails de `docker:init` et `docker:compose`
- [Configuration](configuration.md) - surface env complète et config typée
- [Déployer sur Railway](deployment-railway.md) - alternative PaaS avec des builds automatiques
- [Déployer sur Digital Ocean](deployment-digital-ocean.md) - App Platform avec infrastructure gérée
