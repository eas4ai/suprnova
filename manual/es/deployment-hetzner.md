# Desplegar en Hetzner VPS

Esta guía cubre el despliegue de una aplicación Suprnova en un VPS
usando Hetzner Cloud. Los mismos principios se aplican a cualquier
host de una sola máquina - Linode, Vultr, AWS EC2, o un servidor
dedicado que ya tengas. Elige este camino cuando quieras control total
de la máquina, un coste mensual predecible, y la posibilidad de
colocar Postgres / Redis en la misma máquina.

A lo largo de la guía usamos `myapp` como nombre del proyecto y
`myapp.com` como dominio - sustitúyelos por los tuyos.

## Prerrequisitos

- Un VPS con Ubuntu 22.04 o Debian 12
- Acceso SSH a tu servidor
- Un nombre de dominio apuntando a la IP de tu servidor
- Un proyecto Suprnova - ya sea un árbol de código funcional, o un
  Dockerfile generado con `suprnova docker:init` (ver
  [Docker](cli-docker.md))

## Configuración del servidor

### 1. Crea un VPS

1. Ve a [Hetzner Cloud Console](https://console.hetzner.cloud)
2. Crea un proyecto nuevo y añade un servidor
3. Elige **Ubuntu 22.04** como imagen
4. Selecciona el tamaño de tu servidor (CX11 va bien para apps
   pequeñas)
5. Añade tu clave SSH para acceso seguro

### 2. Configuración inicial del servidor

Conéctate por SSH a tu servidor y ejecuta la configuración inicial:

```bash
# Actualiza los paquetes
apt update && apt upgrade -y

# Crea un usuario no root para tu app
useradd -m -s /bin/bash app
mkdir -p /opt/myapp
chown app:app /opt/myapp

# Instala los paquetes necesarios
apt install -y curl postgresql redis-server
```

### 3. Configura PostgreSQL

```bash
# Crea la base de datos y el usuario
sudo -u postgres psql << EOF
CREATE USER myapp WITH PASSWORD 'your_secure_password';
CREATE DATABASE myapp_production OWNER myapp;
GRANT ALL PRIVILEGES ON DATABASE myapp_production TO myapp;
EOF
```

> **Consejo:**
>
> Para producción, considera usar un servicio de base de datos
> gestionada como el próximo PostgreSQL gestionado de Hetzner, o
> servicios como Neon, Supabase o AWS RDS para mejor fiabilidad y
> copias de seguridad.


## Opciones de despliegue

Elige uno de los siguientes métodos de despliegue. Cada uno termina
con un binario (o contenedor) llamado `app` en `/opt/myapp/app`, que
es lo que la unidad systemd de más abajo sabe ejecutar.

### Opción A: Construir en local

Construye en tu máquina y sube el binario. Sustituye `myapp` por el
nombre real de tu proyecto - `cargo build` nombra el binario según el
`[package].name` de tu `Cargo.toml`:

```bash
# En tu máquina local - compilación cruzada para Linux (si usas macOS)
cargo build --release --target x86_64-unknown-linux-gnu

# O construye con Docker para Linux (el Dockerfile renombra el binario a `app`)
docker build -t myapp .
docker create --name temp myapp
docker cp temp:/app/app ./app-linux
docker rm temp

# Sube al servidor, renombrando a `app` al llegar
scp target/x86_64-unknown-linux-gnu/release/myapp root@your-server:/opt/myapp/app
# o, si fuiste por la vía de Docker:
scp ./app-linux root@your-server:/opt/myapp/app
```

### Opción B: Construir en el servidor

Instala Rust 1.91.1+ (Suprnova usa la edición 2024) y construye
directamente en el servidor:

```bash
# Instala Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Clona, construye y coloca el binario en la ruta estándar
cd /opt/myapp
git clone https://github.com/your-username/your-repo.git .
cargo build --release
cp target/release/myapp ./app   # renómbralo para que ExecStart=/opt/myapp/app de systemd lo encuentre
```

### Opción C: Usar Docker

Ejecuta tu app en un contenedor Docker - el Dockerfile generado por
andamiaje ya nombra el binario de runtime `app` (ver
[Docker](cli-docker.md)):

```bash
# Instala Docker
curl -fsSL https://get.docker.com | sh

# Descarga y ejecuta tu imagen
docker run -d \
  --name myapp \
  --restart unless-stopped \
  -p 8765:8765 \
  --env-file /opt/myapp/.env.production \
  your-registry/myapp:latest
```

Si fuiste por la vía de Docker, sáltate la sección de systemd y ve a
[Proxy inverso con Caddy](#proxy-inverso-con-caddy) - Docker se
encarga de la supervisión de procesos.

## Configuración del entorno

Primero, genera una `APP_KEY` de producción en el servidor (o en
local - lo que importa es el valor). `APP_KEY` es una clave AES-256
de 32 bytes que usa `suprnova::Crypt` para las cookies de sesión y las
URLs firmadas. Suprnova **falla cerrado en el arranque** cuando
`APP_ENV` no es `local`/`dev`/`test` y `APP_KEY` no está establecida -
así que esto no es opcional en producción:

```bash
suprnova key:generate --show
# -> APP_KEY=base64-url-safe-32-bytes
```

Después escribe el archivo de entorno:

```bash
cat > /opt/myapp/.env.production << 'EOF'
APP_NAME="My App"
APP_ENV=production
APP_DEBUG=false
APP_URL=https://myapp.com
APP_KEY=paste-the-generated-key-here

SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# Base de datos - enlaza a localhost cuando la BD está en la misma máquina
DATABASE_URL=postgres://myapp:your_secure_password@localhost:5432/myapp_production
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1

# Sesión
SESSION_SECURE=true
SESSION_SAME_SITE=Lax

# Redis (opcional - usado por los drivers de caché, cola, difusión)
REDIS_URL=redis://127.0.0.1:6379

# Correo
MAIL_DRIVER=smtp
MAIL_HOST=your-smtp-host
MAIL_PORT=587
MAIL_USERNAME=
MAIL_PASSWORD=
MAIL_FROM_ADDRESS=hello@myapp.com
MAIL_FROM_NAME="My App"
EOF

# Protege el archivo - solo el usuario app debe poder leerlo
chmod 600 /opt/myapp/.env.production
chown app:app /opt/myapp/.env.production
```

Ver [Configuración](configuration.md) para la superficie completa de
variables de entorno y cómo se convierte en config tipada.

## Servicios systemd

Un binario de Suprnova admite varios comandos - `./app` (serve, con
auto-migración), `./app schedule:work` (daemon del planificador),
`./app queue:work` (worker de cola), `./app workflow:work` (runner de
flujo de trabajo). Cada proceso de larga duración obtiene su propia
unidad systemd usando el mismo binario y el mismo archivo de entorno.

### Servicio del servidor web

Crea `/etc/systemd/system/myapp.service`:

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

# Entorno
EnvironmentFile=/opt/myapp/.env.production

# Endurecimiento de seguridad
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

El `ExecStart=/opt/myapp/app` predeterminado ejecuta `serve` con
auto-migración. Si prefieres que las migraciones sean un paso de
despliegue separado, usa `ExecStart=/opt/myapp/app serve --no-migrate`
y ejecuta `./app migrate` desde tu script de despliegue antes de
cambiar el binario.

### Servicio del planificador

Si tu app tiene tareas registradas vía `Schedule::call(...)` (ver el
capítulo [Comandos de programación](cli-scheduling.md)), ejecuta
**exactamente un** proceso de planificador para evitar la ejecución
duplicada de tareas. Crea
`/etc/systemd/system/myapp-scheduler.service`:

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

# Entorno
EnvironmentFile=/opt/myapp/.env.production

# Endurecimiento de seguridad
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/myapp

[Install]
WantedBy=multi-user.target
```

### Worker de cola (opcional)

Si despachas jobs a una cola, añade
`/etc/systemd/system/myapp-queue.service`:

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

Puedes escalar los workers de cola horizontalmente - varias instancias
de `myapp-queue.service` en la misma máquina o en máquinas distintas
es seguro.

### Habilita e inicia los servicios

```bash
# Recarga systemd tras escribir los archivos de unidad
systemctl daemon-reload

# Habilita los servicios para que arranquen al iniciar
systemctl enable myapp
systemctl enable myapp-scheduler
systemctl enable myapp-queue        # si añadiste el worker de cola

# Inícialos ahora
systemctl start myapp
systemctl start myapp-scheduler
systemctl start myapp-queue

# Verifica
systemctl status myapp
systemctl status myapp-scheduler
systemctl status myapp-queue
```

## Proxy inverso con Caddy

Caddy gestiona automáticamente los certificados HTTPS con Let's
Encrypt.

### Instala Caddy

```bash
apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
apt update
apt install caddy
```

### Configura Caddy

Edita `/etc/caddy/Caddyfile`:

```
myapp.com {
    reverse_proxy localhost:8765

    # Habilita la compresión
    encode gzip

    # Logs
    log {
        output file /var/log/caddy/myapp.log
    }
}
```

Sustituye `myapp.com` por tu dominio real.

### Inicia Caddy

```bash
systemctl enable caddy
systemctl start caddy
```

Caddy obtendrá y renovará los certificados SSL automáticamente.

## Verificación de salud

Suprnova incluye un endpoint `/_suprnova/health` integrado que se
cortocircuita antes de la cadena de middleware y nunca colisiona con
tus rutas:

```bash
curl https://myapp.com/_suprnova/health
```

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z"
}
```

### Verifica la conectividad de la base de datos

Añade `?db=true` para verificar también la base de datos:

```bash
curl https://myapp.com/_suprnova/health?db=true
```

Respuesta saludable (HTTP 200):

```json
{
  "status": "ok",
  "timestamp": "2026-05-30T10:30:00Z",
  "database": "connected"
}
```

Si la comprobación de la base de datos falla, el endpoint pasa a HTTP
**503** con `"status": "degraded"` y un campo `"database_error"` -
conecta esto a una verificación de salud estilo `livenessProbe` /
`readinessProbe` para que el equilibrador de carga pueda sacar de la
rotación una instancia no saludable.

### Monitoreo externo

Usa el endpoint de salud con servicios de monitoreo:

- **UptimeRobot**: añade un monitor HTTP para
  `https://myapp.com/_suprnova/health`
- **Better Stack** (antes Better Uptime): configura el endpoint de
  verificación de salud con el disparador del 503
- **Prometheus / Grafana**: haz scrape del cuerpo JSON para los
  campos `status` + `database`

## Script de despliegue

Crea un script de despliegue para actualizaciones atómicas. Sustituye
`myapp` por el nombre de tu proyecto (el `[package].name` en tu
`Cargo.toml`) - eso es lo que `cargo build` usa para nombrar el
binario de salida:

```bash
#!/bin/bash
# deploy.sh - ejecútalo en tu máquina local

set -e

PROJECT="myapp"               # el nombre del paquete de Cargo
SERVER="root@your-server"
APP_PATH="/opt/myapp"
BIN="target/x86_64-unknown-linux-gnu/release/$PROJECT"

echo "Construyendo la aplicación..."
cargo build --release --target x86_64-unknown-linux-gnu

echo "Subiendo el binario..."
scp "$BIN" "$SERVER:$APP_PATH/app.new"

echo "Desplegando..."
ssh "$SERVER" << 'EOF'
    set -e
    cd /opt/myapp

    # Detén los servicios de larga duración (ignora fallos en el primer despliegue)
    systemctl stop myapp-queue || true
    systemctl stop myapp-scheduler || true
    systemctl stop myapp

    # Intercambio atómico - renombrar es una sola syscall en el mismo sistema de archivos
    mv app.new app
    chmod +x app

    # Ejecuta las migraciones explícitamente (la unidad también hace
    # auto-migración, pero hacerlo aquí saca a la luz los fallos antes
    # de reiniciar el tráfico)
    sudo -u app ./app migrate

    # Inicia los servicios
    systemctl start myapp
    systemctl start myapp-scheduler || true
    systemctl start myapp-queue || true

    # Verifica la salud (dale al servidor un momento para enlazar)
    sleep 2
    curl -fsS http://localhost:8765/_suprnova/health?db=true > /dev/null || exit 1

    echo "¡Despliegue completo!"
EOF
```

Hazlo ejecutable:

```bash
chmod +x deploy.sh
./deploy.sh
```

## Logs y monitoreo

### Ver los logs

```bash
# Logs del servidor web
journalctl -u myapp -f

# Logs del planificador
journalctl -u myapp-scheduler -f

# Logs de acceso de Caddy
tail -f /var/log/caddy/myapp.log
```

### Rotación de logs

El journald de systemd gestiona la rotación de logs automáticamente.
Para almacenamiento a largo plazo, considera:

- **Loki + Grafana**: agregación de logs autoalojada
- **Papertrail**: servicio de logs en la nube
- **Logtail**: gestión de logs simple

## Configuración del firewall

Protege tu servidor con UFW:

```bash
# Permite SSH
ufw allow 22/tcp

# Permite HTTP/HTTPS (Caddy)
ufw allow 80/tcp
ufw allow 443/tcp

# Activa el firewall
ufw enable
```

> **Advertencia:**
>
> Nunca expongas el puerto 8765 directamente. Usa siempre Caddy como
> proxy inverso para gestionar el SSL y las cabeceras de seguridad.


## Escalado

Un único binario de Suprnova es muy eficiente - un VPS pequeño
soporta una cantidad sorprendente de tráfico antes de que necesites
escalar horizontalmente. Cuando llegue el momento:

### Escalado vertical

Sube el VPS a una instancia más grande para más CPU/memoria. El
binario, el archivo de entorno y las unidades systemd te acompañan
sin cambios.

### Escalado horizontal

Para varias instancias de la aplicación:

1. Configura un equilibrador de carga (Hetzner Load Balancer, HAProxy,
   o Caddy en un nodo dedicado)
2. Mueve Postgres a un servicio gestionado o a un nodo dedicado para
   que las máquinas de la app no tengan estado
3. Mueve las sesiones, la caché y la difusión a Redis para que
   cualquier instancia de la app pueda servir cualquier solicitud
4. Despliega varias instancias de la app; cada una ejecuta su propia
   auto-migración al iniciar con seguridad (el runner de migraciones
   toma un bloqueo para que los arranques concurrentes no colisionen)
5. Mantén **un solo** planificador (`schedule:work`) ejecutándose en
   toda la flota - los workers de cola son seguros de ejecutar en
   paralelo, el planificador no lo es

### Por qué Suprnova diverge

Laravel normalmente ejecuta PHP-FPM detrás de nginx, con cron
disparando `schedule:run` una vez por minuto y Horizon (o supervisord)
gestionando los workers de cola. Suprnova colapsa todo esto en un
único binario con subcomandos. `./app` es un proceso Tokio de larga
duración - no necesita un pool de procesos delante, no necesita un
cron separado, y se mantiene caliente entre solicitudes. systemd es el
supervisor tanto del proceso web como de los workers, y Caddy solo
hace lo que nginx no podía evitar: terminar el TLS y hacer de proxy.

## Dimensionamiento

Elige un VPS según la carga de trabajo, no según el nombre comercial
de un nivel. La gama de Hetzner cambia periódicamente; la lógica de
dimensionamiento no:

| Carga de trabajo | Ajuste aproximado |
|---|---|
| Sitio pequeño, tráfico bajo, SQLite o BD compartida | La instancia vCPU compartida más pequeña (1 vCPU / 2 GB) |
| Tráfico moderado con Postgres + Redis en la misma máquina | 2 vCPU / 4 GB |
| API más pesada + planificador + workers de cola + Postgres | 2–4 vCPU / 8 GB |
| Producción a escala | Instancia de CPU dedicada, o BD separada en su propio nodo |

Consulta los [precios actuales](https://www.hetzner.com/cloud) de
Hetzner para ver el catálogo en vivo. La huella de memoria en reposo
de Suprnova es pequeña (unos pocos MB), así que la RAM es sobre todo
el working set de la base de datos más tu código de dominio.

## Solución de problemas

### El servicio no arranca

Revisa los logs en busca de errores:

```bash
journalctl -u myapp -n 50
```

Problemas comunes:
- Variables de entorno faltantes
- Fallo de conexión a la base de datos
- Puerto ya en uso

### Errores de certificado de Caddy

Asegúrate de que:
- El DNS del dominio apunta a tu servidor
- Los puertos 80 y 443 están abiertos
- Ningún otro servicio está usando el puerto 80

```bash
caddy validate --config /etc/caddy/Caddyfile
```

### Problemas de conexión a la base de datos

Prueba la conexión manualmente:

```bash
sudo -u app psql $DATABASE_URL -c "SELECT 1"
```

### La verificación de salud falla

```bash
# Comprueba si la app está en ejecución
systemctl status myapp

# Prueba el endpoint de salud directamente
curl http://localhost:8765/_suprnova/health

# Comprueba con la base de datos
curl http://localhost:8765/_suprnova/health?db=true
```

Una respuesta `503` con `"status": "degraded"` significa que la app
está levantada pero la verificación de salud de la base de datos
falló - inspecciona `database_error` en el cuerpo y revisa
`DATABASE_URL`, los logs de Postgres y los límites de conexión.

## Siguiente

- [Descripción general de despliegue](deployment.md) - la historia
  agnóstica de plataforma para despliegues de binario único
- [Docker](cli-docker.md) - detalles de `docker:init` y
  `docker:compose`
- [Configuración](configuration.md) - superficie completa de entorno
  y config tipada
- [Desplegar en Railway](deployment-railway.md) - alternativa PaaS
  con construcciones automáticas
- [Desplegar en Digital Ocean](deployment-digital-ocean.md) - App
  Platform con infraestructura gestionada
