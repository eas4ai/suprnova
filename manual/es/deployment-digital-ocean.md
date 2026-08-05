# Desplegar en Digital Ocean

Digital Ocean tiene dos objetivos de producción que le sientan bien a
una app Suprnova: **App Platform** (un PaaS de Docker gestionado -
publica y olvídate) y un **Droplet** (tu propio VPS, tú lo gestionas
todo). Este capítulo recorre ambos. Usa App Platform cuando quieras
bases de datos gestionadas, despliegues automáticos y el SSL resuelto
por ti. Usa un Droplet cuando quieras control total, ya ejecutes otros
servicios en la máquina, o quieras mantener la factura plana sin
importar el tráfico.

## Prerrequisitos

- Una [cuenta de Digital Ocean](https://www.digitalocean.com)
- Un proyecto Suprnova con un Dockerfile - genera uno con:
  ```bash
  suprnova docker:init
  ```
- Una `APP_KEY` de producción. Genérala y guárdala en un lugar seguro:
  ```bash
  suprnova key:generate --show
  ```
  Suprnova falla cerrado en el arranque cuando `APP_ENV` es cualquier
  cosa distinta de `local` / `development` / `testing` y `APP_KEY` no
  está establecida.
- Un repositorio git (GitHub o GitLab) - obligatorio para App
  Platform; para Droplets también puedes subir una imagen ya
  construida a un registro.

## App Platform

App Platform construye tu Dockerfile, ejecuta el binario único de
Suprnova y te da un Postgres gestionado si lo quieres.

### 1. Crea la app

1. Ve a [Digital Ocean Apps](https://cloud.digitalocean.com/apps).
2. Haz clic en **Create App**, conecta GitHub/GitLab y elige el repo y
   la rama.
3. App Platform detecta automáticamente el `Dockerfile` en la raíz del
   repo.

### 2. Configura el servicio web

| Ajuste | Valor |
|---|---|
| Resource type | Web Service |
| HTTP port | `8765` |
| Run command | déjalo vacío - el `CMD` del Dockerfile ejecuta `./app` |
| Health check (HTTP path) | `/_suprnova/health/live` |

El binario de Suprnova por defecto ejecuta `serve` con
auto-migraciones, así que el contenedor ejecutará las migraciones al
iniciar y luego se pondrá a escuchar.

### 3. Añade un Postgres gestionado

1. **Add Resource** -> **Database** -> **PostgreSQL**.
2. Elige un plan (Dev Database para pruebas; un plan Production para
   tráfico real).

App Platform inyecta `DATABASE_URL` en cada componente automáticamente
mediante la vinculación `${db.DATABASE_URL}`.

### 4. Variables de entorno

En la sección **Environment Variables** de tu componente web,
establece:

| Variable | Valor | Notas |
|---|---|---|
| `APP_ENV` | `production` | activa la comprobación de `APP_KEY` que falla cerrado |
| `APP_KEY` | salida de `suprnova key:generate --show` | márcala como **encrypted** |
| `SERVER_HOST` | `0.0.0.0` | enlaza a todas las interfaces |
| `SERVER_PORT` | `8765` | coincide con el `EXPOSE` del Dockerfile |
| `APP_URL` | `https://your-app.ondigitalocean.app` | usada por Inertia + URLs firmadas |

`DATABASE_URL` se proporciona automáticamente mediante la vinculación
de la base de datos gestionada; no la establezcas manualmente.

Si usas Redis para caché/sesiones, añade un clúster de Redis
gestionado y establece `REDIS_URL` a su valor de vinculación
(`${redis.REDIS_URL}`).

### 5. Despliega

Haz clic en **Create Resources**. La primera construcción tarda unos
minutos (build de release de Rust + build de frontend); las
construcciones siguientes usan la caché de capas del Dockerfile y son
mucho más rápidas.

### Añade un worker de planificador

Las tareas programadas (handlers `#[derive(Task)]` registrados vía
`Schedule::call`) necesitan un proceso de larga duración. Añade un
componente Worker que ejecute la misma imagen con un comando distinto:

1. **Create** -> **Add Resource** -> **Detect from source code**,
   selecciona el mismo repositorio.
2. Establece el tipo de recurso en **Worker**.
3. **Run command**:
   ```bash
   ./app schedule:work
   ```
4. El worker hereda las variables de entorno de la app, incluidas
   `DATABASE_URL` y `APP_KEY`.

Los workers no reciben tráfico HTTP. Ejecuta exactamente **una**
instancia de worker - varios planificadores ejecutarían cada tarea
varias veces.

Para los workers de cola (`./app queue:work`) el patrón es idéntico;
normalmente puedes ejecutar más de un worker de cola con seguridad
porque el driver de cola coordina qué worker toma cada job. Ver
[Cola](queues.md).

### Especificación de la app (infraestructura como código)

Para despliegues repetibles, haz commit de un `.do/app.yaml`:

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
      # Solo actividad - App Platform reinicia el contenedor cuando
      # esto falla, así que no debe depender de Postgres. Ver la nota
      # de verificación de salud en Solución de problemas.
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

Despliega con la CLI `doctl`:

```bash
doctl apps create --spec .do/app.yaml
```

Establece el secreto `APP_KEY` por separado mediante la interfaz de
Apps, o:

```bash
doctl apps update <app-id> --spec .do/app.yaml \
  --set-env "APP_KEY=$(suprnova key:generate --show)"
```

### Dominio personalizado

En **Settings** -> **Domains** -> **Add Domain**, introduce tu dominio
y sigue las instrucciones de DNS. App Platform emite y renueva un
certificado de Let's Encrypt automáticamente.

Cuando el dominio esté en producción, actualiza `APP_URL` para que
coincida - Inertia lo usa para la cabecera X-Inertia-Location y las
URLs firmadas lo usan como entrada del hash.

### Escalado

- **Horizontal**: sube **Instance Count** en el servicio web. Cada
  instancia comparte el Postgres gestionado; que varias instancias
  ejecuten auto-migraciones al iniciar es seguro - Suprnova usa el
  migrador con bloqueo asesor de SeaORM.
- **Vertical**: cambia **Instance Size**. El binario de Rust va bien
  en el slug más pequeño para apps de tráfico bajo; súbelo cuando
  empieces a servir WebSockets o conexiones de larga duración a
  escala.

Mantén el worker de planificador en instance count **1**.

## Droplet (VPS)

Un Droplet es el camino cuando quieres ejecutar Suprnova en tu propio
VPS. La mecánica es idéntica a la de cualquier otro VPS Linux -
servicio systemd, proxy inverso Caddy, Postgres gestionado o
autoalojado. El capítulo [VPS de Hetzner](deployment-hetzner.md) es el
recorrido canónico de ese patrón; todo lo que hay allí aplica
literalmente en un Droplet. Las únicas diferencias que vale la pena
señalar:

- **Imagen**: elige **Ubuntu 24.04** o **Debian 12** en la consola del
  Droplet.
- **Base de datos**: puedes usar las **Managed Databases** de Digital
  Ocean para Postgres / MySQL / Redis en lugar de ejecutarlas en el
  Droplet - la misma historia de `DATABASE_URL` / `REDIS_URL`,
  apúntalas al endpoint gestionado y Suprnova no nota la diferencia.
- **Copias de seguridad**: activa los snapshots del Droplet y las
  copias de seguridad diarias de la BD gestionada en la consola de DO.
- **Red**: usa una **VPC** de DO para mantener el Droplet y cualquier
  base de datos gestionada en una red privada; haz que la app escuche
  solo en `127.0.0.1` y pon Caddy delante para el TLS.

Si quieres Docker en un Droplet (en lugar de un binario de sistema),
el patrón docker-compose de [Docker](cli-docker.md) encaja sin
problemas - cambia el Postgres autoalojado por la base de datos
gestionada y ya está.

### Por qué Suprnova diverge

El despliegue típico de PHP en Laravel necesita PHP-FPM + un opcache +
un runner de cola + una entrada de cron para el planificador - al
menos tres piezas móviles, cada una con su propia semántica de
reinicio. Un despliegue de Suprnova es un único binario más un proceso
worker opcional. El binario ejecuta migraciones, sirve HTTP, gestiona
WebSockets y vive detrás de un proxy inverso. El mismo binario,
invocado con `./app schedule:work` o `./app queue:work`, es tu
planificador o tu worker de cola. El modelo de App Platform de "una
imagen, varios componentes" encaja con esto de forma natural - el
mismo Dockerfile para cada componente, distinto `run_command` por rol.

## Solución de problemas

### Falla la compilación

Lo primero que hay que comprobar es si el Dockerfile construye en
local:

```bash
docker build -t myapp .
```

Causas comunes cuando la construcción local funciona pero la de App
Platform no:

- **Faltan archivos de contexto de construcción**: comprueba que el
  `.dockerignore` no esté excluyendo `Cargo.lock` ni el directorio
  `migrations/`.
- **Falta de memoria durante `cargo build`**: sube el tamaño de la
  instancia de construcción en App Settings -> Resources -> Build.
  Las construcciones de release de Rust consumen mucha memoria.

### La app arranca y luego se cae

Revisa los logs de runtime en la pestaña **Runtime Logs**. Los dos
fallos de arranque de Suprnova más comunes son:

- **`APP_KEY is required when APP_ENV=production`** - genera una con
  `suprnova key:generate --show` y añádela como variable de entorno
  cifrada.
- **valor de `SERVER_HOST=…` inválido** - debe ser `0.0.0.0` para App
  Platform, no `127.0.0.1` (el loopback no es alcanzable desde el
  equilibrador de carga).

### La verificación de salud falla

La plataforma hace ping a `/_suprnova/health/live` y espera un 200
dentro del tiempo límite configurado. Si está fallando:

- Confirma que la ruta sea exactamente `/_suprnova/health/live` (no
  `/health`). La ruta antigua `/_suprnova/health` sigue funcionando si
  es lo que ya nombra tu spec.
- Confirma que el puerto sea `8765` y coincida con `SERVER_PORT`.
- Para distinguir "no puede enlazar" de "no puede alcanzar Postgres",
  comprueba la base de datos **a mano** desde la consola en lugar de
  hacerlo desde la verificación de salud:

  ```bash
  curl http://localhost:8765/_suprnova/health/ready
  # Saludable: 200 {"status":"ok","database":"connected"}
  # Degradado: 503 {"status":"degraded","database":"error"}
  ```

  Una respuesta degradada significa que la app se enlazó pero no puede
  alcanzar Postgres - revisa la vinculación de `DATABASE_URL`. No
  pases `-f`: hace que curl salga en silencio ante el 503, que es
  justo el caso que estás intentando leer.

No pongas la verificación de la base de datos en el `health_check` del
app spec. App Platform reinicia el contenedor cuando esa comprobación
falla, así que un fallo puntual de la base de datos se llevaría la app
por delante con él - el modo de fallo es un bucle de reinicio justo
durante el incidente que necesitas que la app sobreviva. Ver [Usa la
sonda correcta para la pregunta
correcta](deployment.md#use-the-right-probe-for-the-right-question).

### Las migraciones de base de datos no se ejecutan

Las migraciones se ejecutan automáticamente como parte del arranque
predeterminado de `./app`. Si no lo hacen, revisa los logs de runtime
en busca de errores de SeaORM. Para ejecutarlas manualmente desde la
consola de App Platform:

1. Abre la pestaña **Console** del componente web.
2. Ejecuta `./app migrate`.

Si prefieres mantener las migraciones fuera de la ruta de arranque,
establece el comando de ejecución en `./app serve --no-migrate` y
añade un **Job** de una sola vez al app spec que ejecute
`./app migrate` antes del despliegue.

## Siguiente

- [Descripción general de despliegue](deployment.md) - la
  introducción de despliegue multiplataforma (binario, migraciones,
  planificador, salud)
- [Docker](cli-docker.md) - lo que generan `suprnova docker:init` y
  `docker:compose`
- [Configuración](configuration.md) - cada variable de entorno que lee
  Suprnova
- [Variables de entorno](env-vars.md) - referencia completa, incluidas
  las requeridas en producción
- [Desplegar en Hetzner VPS](deployment-hetzner.md) - el recorrido de
  Droplet aplica aquí literalmente
