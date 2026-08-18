# suprnova new

`suprnova new` genera el andamiaje de un proyecto de Suprnova - un
crate de Cargo nuevo con controladores, rutas, migraciones, una SPA
de Inertia, y un flujo de autenticación funcional ya conectados entre
sí. Ejecútalo una vez por app, y luego vive en `suprnova serve` a
partir de ahí.

## Uso

```bash
suprnova new [name] [options]
```

Si se omite `name`, el asistente interactivo lo solicita. El nombre
se convierte en el directorio del proyecto, el nombre del paquete de
Cargo (tras convertirlo a snake_case), y el `APP_NAME` por defecto en
`.env`. Los nombres deben ser letras/dígitos ASCII/`-`/`_`, empezar
por una letra, no contener separadores de ruta ni `..`, y tener 64
caracteres o menos.

## Opciones

| Opción | Descripción |
|---|---|
| `--frontend <svelte\|react\|vue>` | Elige el framework de la SPA sin interacción. Entra en conflicto con `--api`. |
| `--api` | Genera el andamiaje de un proyecto solo JSON:API (sin Inertia, sin SPA, con autenticación por token en lugar de sesiones). |
| `--no-interaction` | Omite todas las preguntas y usa los valores por defecto (nombre `my-suprnova-app`, frontend `svelte`, autor/descripción vacíos). |
| `--no-git` | Omite `git init` en el proyecto nuevo. |
| `--with-portless` | Emite un `portless.json` para que [`suprnova dev:tls`](dev-tls.md) pueda servir la app en `https://<name>.localhost`. Opcional; no cambia nada más. |

## Modo interactivo

```bash
suprnova new my-app
```

El asistente hace cuatro preguntas, en este orden:

1. **Nombre del proyecto** - por defecto usa el argumento del
   directorio (`my-app`)
2. **Descripción** - se usa como la descripción del paquete de Cargo
3. **Autor** - se usa como el autor del paquete de Cargo; por
   defecto toma tu `git config user.name <name@email>` si está
   establecido
4. **Framework de frontend** - `Svelte (recomendado)`, `React`, o
   `Vue`

Tras confirmar, el generador de andamiaje escribe el proyecto,
ejecuta `git init` (a menos que se use `--no-git`), e imprime los
siguientes pasos:

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

## Modo no interactivo

Para CI, dotfiles, o configuración por script, pasa
`--no-interaction` más los flags que quieras sobrescribir:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

Valores por defecto bajo `--no-interaction`:

- Frontend: `svelte`
- Descripción: `"A web application built with Suprnova"`
- Autor: vacío
- Git: inicializado

No existen flags `--description` ni `--author`; esos valores solo se
establecen a través de las preguntas interactivas, o toman sus
valores por defecto.

## Proyecto solo API

Para backends de servicio sin SPA, usa `--api`:

```bash
suprnova new my-api --api
```

El iniciador de API es considerablemente más pequeño: sin directorio
`frontend/`, sin Inertia, sin vistas de autenticación, con un layout
de crate único en `src/main.rs` (en lugar del workspace
`cmd/main.rs` del iniciador de SPA), autenticación basada en token, y
un controlador de ejemplo `users` más un serializador JSON
`UserResource`. El iniciador de API se vincula al puerto 8765 en su
`.env`.

`--api` es mutuamente excluyente con `--frontend`; pasar ambos
produce un error. Bajo `--api`, solo se pregunta el nombre del
proyecto - las preguntas de descripción/autor/frontend se omiten.

## Qué genera el andamiaje

Un recorrido completo de directorios vive en [Estructura de
directorios](structure.md); la versión breve es:

- `cmd/main.rs` - entrada del binario; llama a
  `Application::new()…run()`
- `src/` - controladores, acciones, comandos, configuración,
  middleware, modelos, migraciones, además de `bootstrap.rs` y
  `routes.rs`
- `src/bin/console.rs` - el análogo por proyecto de `php artisan`
- `frontend/` - Vite 8 + Tailwind v4 + el framework que elegiste, con
  las páginas Home / Dashboard / Login / Register ya conectadas a
  través de Inertia
- `src/migrations/` - las tablas `users`, `sessions`, y
  `remember_tokens` listas para usar
- `.env` - base de datos SQLite por defecto, con un `APP_KEY` recién
  generado para que la app arranque sin intervención del operador
- `.gitignore`, `Cargo.toml`

### Por qué Suprnova diverge

Laravel se distribuye con Blade y añade un frontend a través de
Breeze/Jetstream después de los hechos. Suprnova va en la otra
dirección: `suprnova new` siempre genera el andamiaje de una SPA real
(Svelte/React/Vue sobre Inertia) o de un proyecto JSON:API real. No
hay un iniciador basado primero en un motor de plantillas - si
quieres HTML renderizado en el servidor, Tera está disponible, pero
no es la forma por defecto y no hay ningún camino del generador de
andamiaje que coloque vistas al frente de tu app.

El frontend por defecto es **Svelte 5** (con runes activadas), no
React. Lo elegimos porque es el más ligero de los tres en tiempo de
ejecución y el más cercano a la filosofía del framework: "el tiempo
de compilación vence al ingenio en tiempo de ejecución". React y Vue
son igualmente de primera clase - elige lo que tu equipo conozca.

## Distribución

La CLI en sí se distribuye vía git, no vía crates.io (previo al
lanzamiento):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

`--force` en el mismo comando actualiza una instalación existente.
Los proyectos con andamiaje dependen del crate del framework de la
misma forma - una dependencia de git en su `Cargo.toml`, fijada a la
etiqueta de la versión actual. Consulta [Instalación](installation.md)
para los requisitos previos completos de la cadena de herramientas.

## Siguiente

- [Instalación](installation.md) - requisitos previos de Rust/Node/BD
  y configuración de la cadena de herramientas
- [Estructura de directorios](structure.md) - qué hace cada archivo
  generado con el andamiaje
- [Inicio rápido](quickstart.md) - los primeros 5 minutos después de
  `suprnova new`
- [suprnova serve](cli-serve.md) - el ejecutor de dev que usarás a
  continuación
- [Consola](console.md) - `cargo run --bin console` y el sistema
  `#[command]`
