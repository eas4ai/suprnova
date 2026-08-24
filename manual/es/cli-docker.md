# Docker

Suprnova se distribuye con dos comandos de CLI que generan artefactos
de Docker que puedes adoptar textualmente o modificar. `docker:init`
escribe un `Dockerfile` multietapa + `.dockerignore` para producción. `docker:compose` escribe un
`docker-compose.yml` para los
servicios de desarrollo local (base de datos, caché, y opcionalmente
Mailpit + MinIO). Ambos comandos escriben en la raíz del proyecto
actual; ninguno intenta controlar tu tiempo de ejecución de
contenedores.

## docker:init

Genera un Dockerfile de producción junto con un `.dockerignore`
correspondiente.

```bash
suprnova docker:init
```

El comando se niega a sobrescribir un `Dockerfile` existente; elimina
primero el archivo existente si quieres regenerarlo.

### Qué se escribe

| Archivo | Propósito |
|------|---------|
| `Dockerfile` | Build de tres etapas: activos de frontend, binario de Rust en modo release, imagen de runtime |
| `.dockerignore` | Excluye `target/`, `node_modules/`, `.env*`, los artefactos de build existentes, y los propios archivos de Docker |

### Forma del Dockerfile

El Dockerfile generado usa tres etapas para que la imagen de runtime
cargue solo el binario compilado más las bibliotecas compartidas que
necesita:

1. **`frontend-builder`** - `node:20-alpine`. Instala las
   dependencias de npm y ejecuta `npm run build`, produciendo
   `frontend/dist`.
2. **`backend-builder`** - `rust:1.94.0-slim-bookworm`. Cachea
   `Cargo.toml` + `Cargo.lock` como una capa de dependencias, luego copia tu `cmd/`,
   `src/`, y el `frontend/dist` ya construido (como `public/assets`) y
   ejecuta `cargo build --release`.
3. **`runtime`** - `debian:bookworm-slim` con `ca-certificates` y
   `libssl3`. Se ejecuta como `appuser` sin raíz. Copia el binario
   dentro como `./app` y el directorio `public/` junto a él. Expone
   el puerto 8765.

El `CMD` por defecto de la imagen final es `["./app"]`, que ejecuta
el subcomando `serve` del binario unificado (servidor web con
auto-migraciones al iniciar). Para ejecutar un subcomando distinto,
sobrescribe el comando en el momento de `docker run`:

```bash
# Servidor web (por defecto)
docker run -p 8765:8765 --env-file .env.production my-app

# Ejecuta solo las migraciones, y sale
docker run --env-file .env.production my-app ./app migrate

# Ejecuta el demonio del planificador
docker run --env-file .env.production my-app ./app schedule:work

# Ejecuta el worker de colas
docker run --env-file .env.production my-app ./app queue:work
```

Pasa la configuración de producción mediante `--env-file
.env.production` o flags `-e` individuales. Nunca se debería hacer
commit de `.env.production` - ya está cubierto por el
`.dockerignore`.

### Actualizar la cadena de herramientas de Rust

El Dockerfile fija `rust:1.94.0-slim-bookworm` para la fase de compilación, de modo que una imagen recién generada sea reproducible y coincida con la rama `main` actual. Los Dockerfiles personalizados deben usar la misma cadena de herramientas o una más reciente.

```dockerfile
FROM rust:1.94.0-slim-bookworm AS backend-builder
```

Fija la versión de la cadena de herramientas que coincida con lo que
reporte tu `rust-toolchain.toml` (si tienes uno) o tu `rustc
--version` local.


La rama `main` actual usa SeaORM 2.0, SeaQuery 1.0 y SQLx 0.9. Las aplicaciones que llaman directamente a SeaORM deben importar `ExprTrait` para los métodos de expresión de SeaQuery y usar métodos de conexión `*_raw` explícitos para valores `Statement` preconstruidos. La actualización de dependencias no requiere ninguna migración de datos de la aplicación.

### Por qué Suprnova diverge

Los despliegues de Laravel normalmente ejecutan **varios procesos por
contenedor o host**: php-fpm para la web, un worker de colas, un
planificador, a veces un panel de Horizon, a veces un runner de
Octane. Cada uno es su propia definición de servicio.

Suprnova compila a **un único binario estáticamente enlazado** que
conoce cada subcomando que distribuye el framework - `serve`,
`migrate`, `queue:work`, `schedule:work`, `workflow:work`,
`ssr:start`. La misma imagen Docker ejecuta cada rol; lo único que
cambia es el comando. Eso convierte "web + worker + planificador" en
tres servicios en tu orquestador que apuntan todos a la misma
etiqueta de imagen - una sola build para hacer avanzar toda la app.

## docker:compose

Genera un `docker-compose.yml` que levanta los servicios de
desarrollo local.

```bash
suprnova docker:compose [OPTIONS]
```

Igual que `docker:init`, este comando se niega a sobrescribir un
`docker-compose.yml` existente. También añade
`docker-compose.override.yml` a tu `.gitignore` (si hay un
`.gitignore` presente), para que puedas mantener sobrescrituras por
desarrollador de forma local sin hacer commit de ellas.

### Opciones

| Opción | Descripción |
|--------|-------------|
| `--with-mailpit` | Incluye el servicio de pruebas de correo Mailpit |
| `--with-minio` | Incluye MinIO (almacenamiento de objetos compatible con S3) |

Si no pasas ninguno de los dos flags, el comando pregunta de forma
interactiva por ambos. Pasar cualquiera de los dos flags omite la
pregunta y usa los valores de flag que diste.

### Lo que siempre obtienes

PostgreSQL y Redis se escriben en cada archivo compose generado:

| Servicio | Puerto por defecto | Imagen |
|---------|-------------:|-------|
| PostgreSQL | 5432 | `postgres:16-alpine` |
| Redis | 6379 | `redis:7-alpine` |

Ambos servicios tienen verificaciones de salud, volúmenes nombrados
persistentes, y viven en una red delimitada al proyecto
(`<project>_network`). El usuario, la contraseña y la base de datos
de Postgres son por defecto `suprnova` / `suprnova_secret` /
`suprnova_db`.

### Servicios opcionales

Si los activas:

| Servicio | Puertos por defecto | Imagen |
|---------|--------------:|-------|
| Mailpit | 1025 (SMTP), 8025 (UI) | `axllent/mailpit:latest` |
| MinIO | 9000 (API S3), 9001 (consola) | `minio/minio:latest` |

Mailpit acepta cualquier autenticación SMTP por defecto, para que no
tengas que configurar credenciales durante el desarrollo; la UI web
en `http://localhost:8025` muestra cada correo que envía tu app. Las
credenciales por defecto de MinIO son `minioadmin` / `minioadmin`.

### Ejecutar la pila

```bash
# Levanta todo en segundo plano
docker compose up -d

# Sigue los logs
docker compose logs -f

# Detén y elimina los contenedores (los volúmenes persisten)
docker compose down

# Elimina también los volúmenes (borra la base de datos local)
docker compose down -v
```

### Conectar `.env` con compose

El archivo compose usa la sintaxis `${VAR:-default}` en todas partes,
así que puedes sobrescribir cualquier cosa configurándola en `.env` o
en tu shell. Un `.env` típico para la pila por defecto:

```env
DATABASE_URL=postgres://suprnova:suprnova_secret@localhost:5432/suprnova_db
REDIS_URL=redis://localhost:6379

# Mailpit (si está habilitado)
MAIL_DRIVER=smtp
MAIL_HOST=localhost
MAIL_PORT=1025

# MinIO (si está habilitado)
FILESYSTEM_DISK=s3
S3_ENDPOINT=http://localhost:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=local
S3_REGION=us-east-1
```

Para sobrescribir un puerto (por ejemplo, porque el 5432 ya está en
uso), establece la variable de entorno correspondiente antes de
levantar la pila:

```bash
DB_PORT=5433 docker compose up -d
```

El conjunto completo de puertos que se pueden sobrescribir:

| Variable | Servicio | Por defecto |
|----------|---------|--------:|
| `DB_PORT` | PostgreSQL | 5432 |
| `REDIS_PORT` | Redis | 6379 |
| `MAILPIT_SMTP_PORT` | Mailpit SMTP | 1025 |
| `MAILPIT_UI_PORT` | Mailpit UI | 8025 |
| `MINIO_API_PORT` | MinIO S3 | 9000 |
| `MINIO_CONSOLE_PORT` | Consola de MinIO | 9001 |

### Personalizar el archivo compose

`docker-compose.yml` es tuyo para editar después de generarlo -
Suprnova no lo regenera ni lo vuelve a leer después. Cambios
habituales:

- Cambia `postgres:16-alpine` por `mysql:8` o `mariadb:11` si
  prefieres uno de esos drivers; ambos son de primera clase en
  Suprnova
- Añade una entrada `volumes:` que monte tu directorio `migrations/`
  si quieres ejecutar migraciones dentro de un contenedor de una sola
  vez
- Añade servicios adicionales (Qdrant, Elasticsearch, Nats) de la
  misma forma

## Despliegue en producción

Para un despliegue real, ejecuta `docker:init` y trata el
`Dockerfile` generado como tu entrada de build. La mayoría de los
orquestadores (Railway, Fly, Digital Ocean App Platform, Kubernetes)
solo necesitan tres cosas:

1. La etiqueta de imagen construida a partir de este `Dockerfile`
2. Un archivo de entorno con `DATABASE_URL`, `APP_KEY`, y cualquier
   clave específica del driver
3. Una verificación de salud que apunte a `GET
   /_suprnova/health/live` (y, si la plataforma distingue entre las
   dos, una verificación de preparación en
   `/_suprnova/health/ready`)

La forma de binario único significa que cada rol usa la misma imagen;
declaras un servicio "web" que ejecuta `./app` y un servicio
"planificador" o "worker" que ejecuta `./app schedule:work` (o `./app
queue:work`). Ambos leen el mismo entorno, así que se mantienen
sincronizados en cada despliegue.

Consulta [Despliegue](deployment.md) para la lista de verificación
independiente de la plataforma, y las guías de plataforma para
ejemplos completamente resueltos: [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md), [VPS de
Hetzner](deployment-hetzner.md).

## Resumen

| Comando | Escribe | Cuándo usarlo |
|---------|--------|-------------|
| `suprnova docker:init` | `Dockerfile`, `.dockerignore` | Construir imágenes de producción |
| `suprnova docker:compose` | `docker-compose.yml` | Levantar Postgres/Redis/Mailpit/MinIO local |

## Siguiente

- [Despliegue](deployment.md) - la lista de verificación de
  despliegue independiente de la plataforma
- [Railway](deployment-railway.md) - PaaS gestionado con build desde
  git
- [Digital Ocean](deployment-digital-ocean.md) - despliegues en App
  Platform
- [VPS de Hetzner](deployment-hetzner.md) - bare-metal con systemd +
  Caddy
- [Variables de entorno](env-vars.md) - cada clave que lee el
  framework
