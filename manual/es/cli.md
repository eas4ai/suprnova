# Descripción general de CLI

Suprnova se distribuye con dos binarios con funciones distintas. El
`suprnova` global - instalado una vez en `~/.cargo/bin` - genera el
andamiaje de proyectos nuevos, genera código, arranca servidores de
desarrollo, y ejecuta migraciones. El `console` por proyecto,
construido a partir del `src/bin/console.rs` de cada app, ejecuta
comandos en tiempo de ejecución que necesitan los tipos compilados de
la app (sembradores, podadores, tus propios handlers `#[command]`).
Este capítulo es el mapa; cada subcomando tiene su propio análisis en
profundidad en los capítulos hermanos listados bajo
[Siguiente](#siguiente).

## Instalación

La CLI se distribuye vía `cargo install --git`. Suprnova todavía no está
en crates.io - consulta la [Nota previa al lanzamiento en
Instalación](installation.md#pre-launch-note) para saber por qué.

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
suprnova --version
```

Para actualizar más adelante, pasa `--force`:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli
```

## Los dos binarios

| Binario | Construido a partir de | Se usa para |
|---|---|---|
| `suprnova` | `suprnova-cli/` (este crate) | Andamiaje (`new`), generadores (`make:*`), ejecutor de dev (`serve`), migraciones (`migrate*`, `db:sync`), configuración de Docker (`docker:*`), worker de SSR (`ssr:*`), acuñado de claves (`key:generate`), generación de tipos (`generate-types`) |
| `console` | `src/bin/console.rs` en tu proyecto | Comandos en tiempo de ejecución que enlazan los tipos de tu app - los `db:seed` y `model:prune` integrados, más cada `#[command]` / `#[derive(Command)]` que definas |

Los demonios worker (`schedule:run`, `schedule:work`,
`schedule:list`, `workflow:work`, `queue:work`) se sitúan en una
tercera superficie: el propio parser de clap del binario de tu *app*,
el mismo binario que sirve HTTP. El `suprnova` global se mete en
`cargo run --quiet -- <name>` para esos, de modo que puedas lanzarlos
desde la CLI que ya tienes abierta. Consulta [Consola](console.md)
para la división completa en tres.

### Por qué Suprnova diverge

Laravel resuelve esto con un único script por proyecto - `php
artisan` - porque PHP carga el framework y el código del usuario
juntos en tiempo de ejecución. Rust enlaza los binarios en tiempo de
compilación, así que un binario `suprnova` global no puede ver de
forma estática tus sembradores, factories, o handlers `#[command]`.
La división pragmática:

- El trabajo que solo toca archivos (andamiaje, generadores,
  operaciones) vive en el binario `suprnova` global
- El trabajo en tiempo de ejecución que necesita tus tipos compilados
  vive en el binario `console` por proyecto
- Los demonios viven en el binario de tu app/servidor, de modo que
  comparten la misma ruta de arranque que `serve`

Obtienes la ergonomía de `php artisan` (`cargo run --bin console --
db:seed` o `console <name>` directamente) sin la mentira del
enlazado estático.

## Comandos de un vistazo

La misma lista que imprime `suprnova --help`, agrupada de la misma
forma.

### Crear

| Comando | Descripción |
|---|---|
| `suprnova new [name]` | Genera el andamiaje de un proyecto nuevo. Consulta [`suprnova new`](cli-new.md). |
| `suprnova serve` | Arranca el backend + Vite juntos con recarga en caliente. Consulta [`suprnova serve`](cli-serve.md). |
| `suprnova dev:tls` | Confía en la CA de portless y registra una URL de dev `https://<name>.localhost`. Consulta [URLs HTTPS de desarrollo](dev-tls.md). |
| `suprnova web:run` | Ejecuta el binario de la app directamente (sin Vite, sin bucle de recompilación). Ejecución local con forma de producción. |

### Generar

| Comando | Descripción |
|---|---|
| `suprnova make:controller <name>` | Genera el andamiaje de un controlador en `src/controllers/`. |
| `suprnova make:action <name>` | Genera el andamiaje de una acción invocable en `src/actions/`. |
| `suprnova make:middleware <name>` | Genera el andamiaje de un middleware en `src/middleware/`. |
| `suprnova make:migration <name>` | Genera el andamiaje de una migración de SeaORM en `src/migrations/`. |
| `suprnova make:inertia <name>` | Genera el andamiaje de una página de Inertia en `frontend/src/pages/`. Pasa `--data` para obtener en su lugar una estructura de props `#[derive(Data, Validate)]` en `src/props/`. |
| `suprnova make:error <name>` | Genera el andamiaje de un error de dominio en `src/errors/`. |
| `suprnova make:task <name>` | Genera el andamiaje de una tarea programada en `src/tasks/`. |
| `suprnova make:command <name>` | Genera el andamiaje de un comando de consola `#[derive(Command)]` en `src/commands/`. |
| `suprnova generate-types` | Emite tipos de TypeScript a partir de cada estructura `#[derive(InertiaProps)]`. `-o <path>` para sobrescribir la salida, `-w` para monitorear y regenerar. |

Consulta [Generadores](cli-generators.md) para el detalle completo del
andamiaje y el aspecto de cada archivo generado.

### Base de datos

| Comando | Descripción |
|---|---|
| `suprnova migrate` | Ejecuta todas las migraciones pendientes. |
| `suprnova migrate:status` | Muestra qué migraciones están aplicadas y cuáles pendientes. |
| `suprnova migrate:rollback [--step N]` | Revierte las últimas N migraciones (por defecto 1). |
| `suprnova migrate:fresh [--force]` | Elimina todas las tablas y vuelve a ejecutar todas las migraciones. **Destructivo.** En producción necesita `--force` más una confirmación escrita en una terminal interactiva. |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Ejecuta las migraciones y regenera las entidades de SeaORM a partir del esquema en vivo. `--regenerate-models` sobrescribe los archivos de modelo personalizados en `src/models/`. |

`db:seed` **no** está aquí - vive en el binario `console` por
proyecto porque el registro de sembradores se compila dentro de tu
crate. Ejecútalo con `cargo run --bin console -- db:seed` o
`./target/debug/console db:seed`. Consulta [Consola](console.md) para
el patrón de registro.

Consulta el [capítulo de migraciones](cli-migrations.md) para el
flujo de trabajo completo de migraciones.

### Programación

| Comando | Descripción |
|---|---|
| `suprnova schedule:run` | Ejecuta una vez cada tarea vencida. La forma amigable con cron. |
| `suprnova schedule:work` | Demonio en primer plano que revisa cada minuto y ejecuta las tareas vencidas. |
| `suprnova schedule:list` | Imprime cada tarea registrada con su expresión cron. |

Cada uno de estos se mete en `cargo run --quiet -- <name>` contra el
binario de tu app/servidor - el mismo binario que sirve HTTP - así que
las tareas registradas y los servicios arrancados son visibles.
Consulta [CLI de programación](cli-scheduling.md) y el capítulo de
[Programación de tareas](scheduling.md).

### Flujo de trabajo

| Comando | Descripción |
|---|---|
| `suprnova workflow:work` | Inicia el demonio worker de flujos de trabajo. Extrae los pasos del flujo de trabajo del registro y los ejecuta con el mismo límite de pánico que los handlers HTTP. |
| `suprnova workflow:install` | Coloca las migraciones de `workflow` + `workflow_steps` en `src/migrations/`. Ya está presente en los proyectos recién generados con andamiaje. |

Consulta [Flujos de trabajo](workflows.md).

### SSR

| Comando | Descripción |
|---|---|
| `suprnova ssr:start [--runtime node\|bun\|deno] [--bundle <path>]` | Lanza el worker de SSR de Inertia en primer plano. Recae en la variable de entorno `SUPRNOVA_SSR_RUNTIME`, y luego en `node`; el bundle recae en `SUPRNOVA_SSR_BUNDLE`, y luego en `frontend/bootstrap/ssr/ssr.js`. |
| `suprnova ssr:check [--url <url>] [--timeout-ms N]` | Verifica que la ruta `GET /health` del worker de SSR responda con 2xx. Recae en `SUPRNOVA_SSR_URL`, y luego en `http://127.0.0.1:13714`. Tiempo de espera por defecto 2000 ms. |

Consulta [SSR de Inertia](frontend.md) para la configuración de
producción.

### Despliegue

| Comando | Descripción |
|---|---|
| `suprnova docker:init` | Emite un `Dockerfile` de producción multi-etapa + un `.dockerignore`. |
| `suprnova docker:compose [--with-mailpit] [--with-minio]` | Emite un `docker-compose.yml` para desarrollo local. Postgres + Redis siempre incluidos; Mailpit y MinIO son opcionales. |

Consulta [Docker](cli-docker.md) y el capítulo de
[Despliegue](deployment.md).

### Seguridad

| Comando | Descripción |
|---|---|
| `suprnova key:generate [--show]` | Acuña una clave AES-256 de 32 bytes, en base64 seguro para URL sin padding (el mismo formato en la red que produce `EncryptionKey::to_base64`). `--show` imprime solo la clave, para `APP_KEY=$(suprnova key:generate --show)`. |

Consulta [Cifrado](encryption.md) para saber qué protege `APP_KEY` y
cómo funciona la rotación vía `APP_KEY_PREVIOUS`.

## Inicio rápido

El camino más común de "nada instalado" a "aplicación en marcha":

```bash
# 1. Instala la CLI
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.0 suprnova-cli

# 2. Genera el andamiaje de un proyecto (interactivo - elige Svelte por defecto)
suprnova new my-app

# 3. Arráncalo
cd my-app
suprnova migrate
npm install
suprnova serve
```

Andamiaje no interactivo (CI, configuración con scripts):

```bash
suprnova new my-app \
  --frontend svelte \
  --no-interaction \
  --no-git
```

Andamiaje solo de API (sin Inertia, sin SPA):

```bash
suprnova new my-api --api
```

Generar código en un proyecto existente:

```bash
suprnova make:controller Posts
suprnova make:migration create_posts_table
suprnova make:command reports:daily   # se registra bajo el binario console por proyecto
suprnova migrate
```

## Obtener ayuda

`--help` (o `-h`) funciona en cualquier subcomando. La ayuda de nivel
superior está formateada a mano (`ui::print_help`) y agrupa los comandos
por sección; la ayuda por subcomando viene de clap y muestra cada flag
con su valor por defecto:

```bash
suprnova --help
suprnova new --help
suprnova serve --help
suprnova make:inertia --help
```

Para el binario `console` por proyecto:

```bash
cargo run --bin console -- --help
cargo run --bin console -- db:seed --help
cargo run --bin console -- <your-command> --help
```

`--version` imprime la versión en su propia línea, que es lo que
interesa al reportar un bug o al comprobar si una instalación surtió
efecto:

```bash
suprnova --version
# suprnova 1.3.0
```

Se aceptan tanto `-v` como `-V`. El flag generado por clap ofrece solo
`-V`; este está declarado a mano para que la grafía en minúscula - la
que la mayoría prueba primero - funcione también. La versión aparece
también en el banner de `--help`, que es donde vivía antes de que
existiera el flag.

## Siguiente

- [`suprnova new`](cli-new.md) - cada flag que acepta el generador de
  andamiaje y el layout de directorios que produce
- [`suprnova serve`](cli-serve.md) - el ejecutor de dev: backend +
  Vite + generación de tipos
- [Generadores](cli-generators.md) - la familia completa `make:*` con
  sus plantillas de salida
- [CLI de migraciones](cli-migrations.md) - `migrate`,
  `migrate:fresh`, `db:sync`, y el flujo de trabajo de SeaORM
- [Consola](console.md) - el binario `console` por proyecto,
  `#[command]`, `#[derive(Command)]`, y la asimetría de los tres
  binarios
