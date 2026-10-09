# Configuración

Suprnova lee la configuración desde variables de entorno (cargadas desde
`.env` en desarrollo, el entorno del proceso en producción) y las expone
a tu código de dos formas:

1. **Acceso directo a env** - `env::env`, `env_required`, `env_optional`
   para búsquedas puntuales
2. **Structs de configuración tipados** - `Config::register` / `Config::get` para
   cualquier cosa que leas más de una vez, con tipado fuerte

El framework lee un puñado de variables de entorno por sí mismo (`APP_KEY`,
`APP_ENV`, `DATABASE_URL`, etc.); el resto son tuyas.

## El archivo `.env`

`suprnova new` escribe un `.env` inicial con los valores que tu aplicación necesita
para arrancar:

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detailed error pages + verbose logs
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Database - SQLite by default; swap to postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # set true in production (HTTPS only)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Mail - defaults to `log` driver (writes outgoing mail to the
# tracing log, good for dev). Set MAIL_DRIVER to one of
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# for production.
MAIL_DRIVER=log
# SMTP credentials (only read when MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Left blank it derives from the credentials
# above - starttls with them, none without. Production refuses to boot
# unencrypted; see the Mail chapter.
MAIL_SMTP_ENCRYPTION=
```

Un `.env.example` acompañante ofrece las mismas claves con valores de marcador de posición -
hazle commit; no hagas commit de `.env`. El `.gitignore` predeterminado ya excluye
`.env`.

## Cómo funciona la carga de `.env`

Al arrancar, el framework:

1. Detecta el entorno desde `APP_ENV` (sin distinción de mayúsculas/minúsculas,
   `prod`/`dev`/`stage`/`stg`/`test` también se reconocen).
2. Carga `.env` desde la raíz del proyecto.
3. Si existe un archivo por entorno (`.env.staging`, `.env.production`),
   lo carga encima - sus valores anulan `.env`.
4. Las variables reales del entorno del proceso anulan ambas (esto es en lo que
   se basa la orquestación de contenedores).

El orden en una línea: **env de proceso > `.env.<environment>` > `.env`**.

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

En una ejecución de CI con `APP_ENV=testing`, el framework carga `.env.testing`
encima de `.env` para que puedas anular las URLs de BD y desactivar drivers de correo
sin tocar el `.env` de desarrollo.

## Acceso directo a env

Para lecturas puntuales de cadenas, números, booleanos - cualquier cosa que implemente
`std::str::FromStr` - usa la familia `env::*`:

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // con valor por defecto
let url: String = env_required("APP_URL");                   // entra en pánico si falta - solo en arranque
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // None si falta
```

- `env(key, default)` - lectura con coerción de tipo y valor alternativo
- `env_required(key)` - entra en pánico si la clave falta o falla en el
  análisis. Solo úsalo en tiempo de arranque (en `bootstrap()` o `config::register()`)
  donde un valor requerido faltante debería fallar el proceso inmediatamente
- `env_optional(key)` - devuelve `Option<T>`; `None` si falta o
  valores no analizables

Cada clave única también se registra una vez en la primera lectura, para que puedas auditar
exactamente qué variables de entorno toca tu aplicación.

## Structs de configuración tipados

Para cualquier cosa que tu aplicación lea más de una vez, define un struct tipado
y regístralo. El patrón es:

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

Entonces léelo en cualquier lugar con una línea:

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

El registro se indexa por `TypeId`, así que cada struct se almacena una vez.
Llamar a `Config::register` nuevamente con el mismo tipo reemplaza la
entrada anterior - conveniente para pruebas.

### Conectar el registro a tu aplicación

El `cmd/main.rs` del andamiaje incluye un paso `.config(…)` en la
canalización de arranque fluida:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← esto llama a tu registro
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` normalmente delega a cada módulo de sección:

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### Deserializar structs completos de env

Para configuraciones más grandes, puedes deserializar directamente desde variables de env mediante
`serde`. Suprnova expone dos ayudantes:

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// Lee SERVER_HOST / SERVER_PORT del entorno
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - deserializa de todas las variables de env del proceso
- `Config::resolve_prefixed::<T>("PREFIX_")` - deserializa solo
  variables con el prefijo dado (el prefijo se elimina antes de
  la deserialización)

Ambos devuelven `Result<T, FrameworkError>` para que un campo requerido faltante
aparezca como `FrameworkError::Internal` llevando el diagnóstico de envy
en lugar de un pánico.

## Configuración específica del entorno

La enumeración `Environment` cubre el conjunto estándar:

| Variant | Valores `APP_ENV` reconocidos |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod` |
| `Testing` | `testing`, `test` |
| `Custom(String)` | cualquier otra cosa (preserva tu capitalización, utilizada para la búsqueda de `.env.<custom>`) |

Ramas comunes:

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // cookies estrictas, driver de correo real, etc.
}

if Config::is_debug() {
    // páginas de error detalladas, registro de consultas
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` devuelve `true` cuando `APP_DEBUG=true` se establece explícitamente,
o - cuando `APP_DEBUG` no se establece - cuando el entorno detectado es
`Local`, `Development`, o `Testing`. Production, staging, y cualquier
entorno personalizado no reconocido por defecto a `false`. Mantenlo desactivado en
producción; controla el detalle de la página de error y algunos valores predeterminados internos.

### `APP_KEY` es requerido en no-desarrollo

En producción (cualquier `APP_ENV` otro que `local`/`development`/
`testing`), Suprnova requiere que `APP_KEY` se establezca en una cadena válida de 32 bytes
base64 segura para URL. Arrancar sin ella falla cerrado con un
mensaje de error descriptivo - no hay alternativa silenciosa.

Si aún no tienes un `APP_KEY`:

```bash
suprnova key:generate          # imprime la clave con una nota que recuerda añadirla a .env
suprnova key:generate --show   # imprime solo la clave, apta para `APP_KEY=$(suprnova key:generate --show)`
```

Ninguna forma edita `.env` por ti - copia la clave impresa en tu
`.env` (o tu gestor de secretos) tú mismo.

Para rotación de claves (donde los datos encriptados antiguos deben desencriptarse durante
la ventana de migración), ver [Cifrado](encryption.md#key-rotation).

## Configuración en pruebas

En pruebas, registra configuración en la configuración de prueba en lugar de depender de
`.env`:

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … tu prueba
}
```

El atributo `#[suprnova_test]` también configura estado de contenedor aislado
para que las pruebas concurrentes no vean las vinculaciones de unos y otros - ver
[Pruebas](testing.md).

## Variables de entorno comunes que Suprnova lee

Una lista no exhaustiva - estas son variables que el framework mismo examina.
Tu aplicación lee más además.

| Var | Default | Qué hace |
|---|---|---|
| `APP_NAME` | `"app"` | Registrado al arranque, utilizado en algunos mensajes de error predeterminados |
| `APP_ENV` | `local` | Impulsa `Environment::detect` y la búsqueda de `.env.<suffix>` |
| `APP_DEBUG` | consciente del entorno (`false` en producción) | Páginas de error detalladas + registro adicional |
| `APP_URL` | `http://localhost:8765` | URL base para generación de URL absoluta, URLs firmadas |
| `APP_KEY` | ninguno (requerido en prod) | Clave AES-256 para `Crypt`, sesiones, cursores |
| `APP_KEY_PREVIOUS` | ninguno | Claves anteriores separadas por comas para rotación (máx. 8) |
| `SERVER_HOST` | `127.0.0.1` | Dirección de enlace |
| `SERVER_PORT` | `8765` | Bind port |
| `DATABASE_URL` | ninguno | Requerido si tu aplicación utiliza la base de datos |
| `DB_MAX_CONNECTIONS` | `10` | máx. del grupo sqlx |
| `DB_MIN_CONNECTIONS` | `1` | mín. del grupo sqlx |
| `DB_CONNECT_TIMEOUT` | `30` (seconds) | tiempo de espera de conexión del grupo sqlx |
| `SESSION_LIFETIME` | `120` (minutes) | Expiración de sesión |
| `SESSION_TOUCH_INTERVAL` | `300` (seconds) | Cadencia de escritura mínima de expiración deslizante |
| `SESSION_GC_INTERVAL` | `3600` (seconds) | Cadencia de limpieza supervisada de sesiones expiradas |
| `SESSION_COOKIE` | `suprnova_session` | Cookie name |
| `SESSION_SECURE` | `true` | Establecer el flag de cookie `Secure`. Anular a `false` para desarrollo HTTP local. |
| `SESSION_SAME_SITE` | `Lax` | `Strict`, `Lax`, or `None` |
| `MAIL_DRIVER` | `log` | Uno de `smtp`, `ses`, `mailgun`, `postmark`, `sendgrid`, `resend`, `log`, `memory` |
| `CACHE_DRIVER` | `memory` | Uno de `memory`, `redis`, `database` |
| `QUEUE_DRIVER` | `memory` | Uno de `memory`, `redis`, `database` (valores desconocidos advierten y vuelven a `memory`) |
| `RATE_LIMIT_DRIVER` | `memory` | One of `memory`, `redis` |
| `LOG_FORMAT` | consciente del entorno (`pretty` en dev/local, `json` en producción) | `pretty` or `json` |
| `LOG_LEVEL` | `info` | Uno de `error`, `warn`, `info`, `debug`, `trace` |

La lista completa auditada vive en [Variables de entorno](env-vars.md).

## Siguiente

- [Arranque de la aplicación](bootstrap.md) - donde se llama a la registración de configuración tipada
- [Contenedor de servicios](container.md) - cómo se lee la configuración registrada
  junto a servicios vinculados
- [Variables de entorno](env-vars.md) - la lista de referencia completa
- [Despliegue](deployment.md) - configuración de env de producción
