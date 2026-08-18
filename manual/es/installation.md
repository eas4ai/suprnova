# Instalación

Este capítulo te lleva de "no tener Suprnova en esta máquina" a ejecutar un
proyecto con andamiaje. Si ya estás allí, salta a [Inicio rápido](quickstart.md).

## Requisitos

- **Rust 1.91.1+** (el espacio de trabajo utiliza la edición 2024). Instala a través de
  [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js 20+** y **npm** (o pnpm/yarn/bun) para la cadena de herramientas del
  frontend. Suprnova utiliza Vite 8 y tu iniciador incluye TypeScript +
  Tailwind v4. Instala a través de [nodejs.org](https://nodejs.org/) o tu
  gestor de paquetes.
- **Una biblioteca cliente de base de datos** que coincida con el driver que desees usar:
  - SQLite - no se necesitan extras; sqlite está incluido
  - PostgreSQL - `libpq` en la mayoría de sistemas (a menudo preinstalado)
  - MySQL o MariaDB - `libmariadb` / `libmysqlclient` en la mayoría de sistemas

No tienes que elegir una base de datos ahora. El generador de andamiaje por defecto elige
SQLite para que una aplicación nueva se ejecute sin configuración.

## Instalar la CLI

Suprnova se distribuye como un proyecto Cargo, y el instalador de CLI obtiene
el framework de git (no de crates.io - ve la [nota previa al lanzamiento](#pre-launch-note) abajo):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

Esto compila el binario `suprnova` y lo coloca en `~/.cargo/bin`.
Confirma que funcionó:

```bash
suprnova --version
```

Deberías ver `suprnova 0.x.x`.

Si no se encuentra `suprnova`, tu `~/.cargo/bin` no está en `PATH`. Añade esto
a tu configuración del shell:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Crear un proyecto

`suprnova new` crea un proyecto completo con andamiaje - backend + frontend
elegido + configuración Vite + migraciones de autenticación + rutas de ejemplo.
Es interactivo por defecto:

```bash
suprnova new my-app
```

El asistente pregunta por, en orden:

1. **Nombre del proyecto** - se omite cuando lo pasas como argumento (`my-app`)
2. **Descripción** - se usa en `Cargo.toml`
3. **Autor** - se usa en `Cargo.toml`; por defecto tu `user.name` de git
4. **Framework de frontend** - uno de `svelte` (por defecto), `react`, `vue`

Si deseas omitir las indicaciones (IC, configuración con script), pasa
`--no-interaction` y elige un frontend explícitamente:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

`--no-interaction` acepta los valores por defecto para descripción ("Una
aplicación web construida con Suprnova") y autor (vacío). Para establecerlos,
edita el `Cargo.toml` generado después del andamiaje.

Las tres opciones de frontend incluyen sus propios iniciadores Svelte-5,
React-19 o Vue-3.5. Los tres utilizan Inertia v3 + Vite 8 +
Tailwind v4 e incluyen un flujo preconfigurado de Login/Register/Dashboard con
autenticación basada en sesiones.

Suprnova también incluye un **iniciador de API** más ligero para backends de servicio sin
SPA:

```bash
suprnova new my-api --api
```

El iniciador de API tiene la misma pila de backend pero sin frontend, sin Inertia,
y utiliza autenticación basada en tokens en lugar de cookies de sesión.

## Primera ejecución

```bash
cd my-app

# Ejecuta las migraciones (users, sessions, etc.)
suprnova migrate

# Instala las dependencias del frontend
npm install              # en la raíz del proyecto

# Arranca el backend y Vite juntos
suprnova serve
```

`suprnova serve` ejecuta el backend en `http://127.0.0.1:8765` y Vite
en `http://127.0.0.1:5765`. Accede a la URL del backend - Vite está proxificado por lo que
no necesitas visitarlo directamente.

Deberías ver la página de bienvenida. Luego visita `/register` para crear una
cuenta e `/login` para iniciar sesión.

## Lo que se creó con andamiaje

```
my-app/
├── Cargo.toml          # manifiesto del crate, dos [[bin]] targets
├── .env                # config local (URL de BD, app key, puertos)
├── .env.example        # plantilla para ops/CI
├── .gitignore
├── cmd/
│   └── main.rs         # la entrada binaria; llama a Application::new().run()
├── src/
│   ├── lib.rs          # cableado de módulos
│   ├── bootstrap.rs    # registro de servicios (el análogo de Suprnova de los proveedores)
│   ├── routes.rs       # el árbol de macros routes!
│   ├── bin/
│   │   └── console.rs  # `cargo run --bin console <subcommand>`
│   ├── actions/        # controladores invocables de un solo método
│   ├── commands/       # handlers anotados con `#[command]`
│   ├── config/         # secciones de config tipadas (database, mail)
│   ├── controllers/    # home, auth, dashboard
│   ├── middleware/     # logging, authenticate
│   ├── migrations/     # migradores SeaORM (users, sessions, etc.)
│   └── models/         # structs `#[suprnova::model]` (user)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── main.{tsx,ts}
│       ├── app.css
│       ├── pages/
│       │   ├── Home, Dashboard
│       │   └── auth/{Login,Register}
│       └── types/
│           └── inertia-props.ts
└── public/
    └── assets/         # salida de la compilación de producción de Vite
```

El recorrido completo por los directorios está en [Estructura de directorios](structure.md).

## Actualizar la CLI

La CLI se encuentra en tu `~/.cargo/bin`. Para actualizar a la versión más reciente:

```bash
cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v1.2.4 suprnova-cli
```

`--force` hace que Cargo sobrescriba el binario existente.

## Actualizar la versión del framework de tu aplicación

Una aplicación con andamiaje depende del crate del framework `suprnova` a través de una
dependencia de git en `Cargo.toml`:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Para obtener los cambios del framework más recientes:

```bash
cargo update -p suprnova
```

La dependencia de git rastrea la etiqueta de lanzamiento nombrada. Actualiza la etiqueta en
`Cargo.toml`, luego ejecuta `cargo update -p suprnova`; tu `Cargo.lock` registra el
commit exacto que se resolvió, por lo que las compilaciones permanecen reproducibles entre actualizaciones -
no hay necesidad de fijar manualmente un `rev` en `Cargo.toml`.

## Modelo de distribución

Suprnova se distribuye a través de git, no de crates.io - tanto el framework
como la CLI se instalan desde GitHub. Cada versión se publica como una
Publicación etiquetada de GitHub (p. ej. `v0.7.2`) para el registro de cambios, pero no depende de
la etiqueta: la dependencia de git rastrea la rama por defecto, y `Cargo.lock`
fija el commit exacto que tu aplicación resolvió, por lo que las compilaciones son reproducibles entre
ejecuciones de `cargo update` - no hay necesidad de fijar manualmente una `tag` o `rev`.

## Configuración del editor

Algunas extensiones de VS Code hacen la experiencia más fluida:

- **rust-analyzer** - el servidor de lenguaje de Rust
- **Svelte for VS Code** (o React/Vue si elegiste esos)
- **Tailwind CSS IntelliSense**
- **Even Better TOML**

`rust-analyzer` indexará el proyecto en la primera apertura; espera 1-2
minutos la primera vez, luego incremental.

## Siguiente

- [Inicio rápido](quickstart.md) - construye una pequeña aplicación en 5 minutos
- [Estructura de directorios](structure.md) - qué hay en cada archivo que
  creó el generador de andamiaje
- [Configuración](configuration.md) - la historia de `.env` y configuración tipada
- [Enrutamiento](routing.md) - añade tu primera ruta
