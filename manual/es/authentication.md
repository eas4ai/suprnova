# Autenticación

Suprnova ofrece un sistema de autenticación con forma de Laravel: una
fachada estática `Auth`, guards con nombre resueltos a través de un
`AuthManager`, proveedores de usuario conectables, un trait
`Authenticatable` en tu modelo User, y middleware para poner
compuertas a las rutas. Un proyecto con andamiaje arranca con un guard
de sesión (`web`) y un guard de token (`api`) ya cableados
contra tu `User` tipado, así que el login, el registro, y las rutas
protegidas funcionan el mismo día en que ejecutas `suprnova new`.

## Las piezas

| Tipo | Rol |
|---|---|
| `Auth` | Fachada estática - `Auth::user()`, `Auth::attempt()`, `Auth::login()`, `Auth::logout()`, `Auth::guard("name")` |
| `Authenticatable` | Trait que implementa tu modelo User; expone `get_auth_identifier() -> String` y el hash de contraseña |
| `UserProvider` | Trait que obtiene usuarios desde el almacenamiento; `EloquentUserProvider<M>` y `DatabaseUserProvider` vienen incluidos |
| `AuthManager` | Contiene el [`AuthConfig`] + los proveedores registrados; resuelve guards con nombre a demanda |
| `SessionGuard` / `TokenGuard` | Guards respaldados por sesión (con estado) y por bearer token (sin estado) |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | Guards de ruta |
| `Credentials` | Mapa de credenciales con forma JSON, típicamente `{ "email", "password" }` |

El rastro en el código fuente es corto:
`framework/src/auth/{guard,manager,contract,
authenticatable,middleware,session_guard,token_guard,eloquent_provider,
database_provider}.rs`. Los flujos de más alto nivel - verificación de
correo, restablecimiento de contraseña, limitación de fuerza bruta,
2FA TOTP - viven al lado en `framework/src/auth_flows/` y tienen su
propio capítulo: [Flujos de autenticación](auth-flows.md).

## Modelo de identificador

El id del usuario autenticado fluye por Suprnova como un `String` de
punta a punta - el almacenamiento de sesión,
[`UserProvider::retrieve_by_id`], la tabla de "recuérdame", cada
evento de autenticación. La superficie canónica es
`Authenticatable::get_auth_identifier() -> String` (el
`getAuthIdentifier` de Laravel). Las claves primarias numéricas se
convierten a string trivialmente; los UUIDs, ULIDs, y los ids opacos de
proveedor OAuth fluyen sin cambios.

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` es contra lo que los proveedores incluidos
verifican una contraseña en texto plano vía `hashing::verify_async`.
Devuelve `None` para usuarios que se autentican por otros medios
(OAuth, passkey, enlace mágico). El método
`auth_identifier_name() -> &'static str` (por defecto `"id"`) nombra la
columna en la que vive el id. El método de conveniencia
`auth_identifier() -> i64` analiza el id de string por defecto y
recurre a `0` para ids no numéricos - Suprnova mismo nunca lo llama;
sobrescríbelo solo para modelos con clave entera que quieran saltarse
el análisis.

### Por qué Suprnova diverge

El `getAuthIdentifier()` de Laravel devuelve `mixed`. A PHP no le
importa si el id es un int, un string UUID, o una clave primaria
tipada como string de una tabla legada. Rust necesita un único tipo
concreto en el que la sesión, el proveedor, y los eventos estén todos
de acuerdo. `String` es la única opción que acomoda cualquier forma de
id sin forzar al framework a saber cuál usa tu app. La conveniencia
entera `auth_identifier()` existe para el caso común en el que tu
columna es un `BIGINT`, pero el framework nunca depende de ella -
cambia tu `User` a un ULID mañana mismo y nada en la pila de
autenticación lo notará.

## Cablear la autenticación en el arranque

El análogo en Rust de `config/auth.php` es un `AuthConfig` registrado
como singleton `AuthManager` en el contenedor, más un `UserProvider`
registrado bajo un nombre. `bootstrap.rs` normalmente hace ambas cosas
en dos líneas:

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ... DB::init, instalación de SessionMiddleware, etc.

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` lee el guard por defecto desde `AUTH_GUARD`
(por defecto `"web"`) y viene de fábrica con dos guards con nombre: un
guard de sesión `web` y un guard de token `api`, ambos respaldados por
el proveedor `"users"`. Las apps que necesitan más guards (un
proveedor `admins` separado, guards con y sin estado distintos)
construyen la configuración de forma explícita:

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## La fachada `Auth`

La fachada estática `Auth` es la superficie con forma de Laravel que
llamas desde controladores y middleware. Los métodos basados en
credenciales y en usuario delegan al **guard por defecto** (lo que sea
que `AuthConfig::default_guard` señale, por defecto `"web"`); las
lecturas síncronas `check`/`guest`/`id` son la ruta rápida respaldada
por sesión y no necesitan ningún manager.

```rust
use suprnova::{Auth, Credentials};

// Valida las credenciales y loguea al usuario. Dispara Attempting →
// (Login + Authenticated), respeta el remember-me. Devuelve el
// usuario resuelto, o None ante credenciales incorrectas.
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// Loguea directamente a un usuario ya conocido.
Auth::login(user, remember).await?;

// Inicia sesión por id sin volver a comprobar las credenciales (por ejemplo, un registro recién terminado).
Auth::login_using_id(&id, remember).await?;

// Valida las credenciales sin persistir una sesión (diálogos de confirmación de contraseña).
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// Autentica solo para esta solicitud - sin escritura de sesión. El `once` de Laravel.
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// Ruta rápida respaldada por sesión (no requiere AuthManager).
if Auth::check()    { /* autenticado */ }
if Auth::guest()    { /* no autenticado */ }
if let Some(id) = Auth::id() { /* id de string */ }

// Si el usuario actual fue autenticado mediante la cookie de
// remember-me en esta solicitud. El `viaRemember()` de Laravel.
if Auth::via_remember() { /* … */ }

// Resuelve el usuario actual (vía el proveedor registrado).
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// Desmonta la autenticación + revoca el remember-me + rota el CSRF + dispara Logout.
Auth::logout().await?;

// Destrucción completa de la sesión (regenera el id + limpia + revoca el remember-me + dispara Logout).
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` devuelve el usuario resuelto en caso de éxito, en
lugar de un `bool` desnudo - más rico que la API de Laravel, y ahorra
la llamada de seguimiento a `Auth::user()`. `Ok(None)` significa que
las credenciales no resolvieron a ningún usuario; `Err` significa un
fallo de base de datos / hashing / configuración que necesita
propagarse.

Si ya verificaste tú mismo la identidad de un usuario y solo quieres
establecer la sesión - por ejemplo, después de que se complete un
callback de OAuth - recurre a la primitiva síncrona:

```rust
// Sync, sin proveedor, sin AuthManager, sin eventos. Devuelve Err si
// se llama fuera de un alcance de solicitud (sin SessionMiddleware
// instalado), para que un login descartado en silencio nunca pueda
// parecer un éxito.
Auth::login_id(user.id.to_string())?;
```

`login_id` regenera el id de sesión (previniendo la fijación de
sesión) y rota el token CSRF, y luego escribe el id en la sesión. Es
deliberadamente estrepitoso al fallar: las versiones anteriores hacían
un no-op silencioso fuera de un alcance de sesión, y la auditoría lo
corrigió - un "login exitoso" que nunca llegó a completarse es
exactamente el tipo de bug que nada más detecta.

## `Auth::user()` y `user_as<T>`

`Auth::user()` devuelve el usuario detrás del trait:

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

Ese trait object cubre a cualquiera que implemente `Authenticatable`.
Para recuperar tu `User` concreto, haz downcast a través de
`user_as::<T>()`:

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // Acceso directo a los campos del modelo.
    println!("Welcome, {}!", user.name);
}
```

`user_as` devuelve `Ok(None)` tanto cuando no hay ningún usuario
autenticado *como* cuando el usuario resuelto no es un `T` (por
ejemplo, un `Auth::set_user(...)` de otro tipo en algún otro punto de
la pila). Dentro de una solicitud, el usuario se cachea por solicitud,
así que llamar a `Auth::user()` repetidamente solo golpea al proveedor
una vez.

## Guards con nombre

Los métodos `Auth::*` desnudos hablan con el guard por defecto. Para
actuar contra un guard específico, resuélvelo por nombre:

```rust
use suprnova::Auth;

// Las operaciones de solo lectura funcionan en todos los drivers.
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attempt necesitan un guard con estado. Los guards de token fallan de forma estrepitosa aquí.
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` devuelve `Arc<dyn Guard>` (el contrato de
lectura) y `Auth::stateful_guard("name")` devuelve
`Arc<dyn StatefulGuard>` (añade `attempt`/`login`/`logout`). Pedir el
contrato con estado sobre un guard de token devuelve un error con un
mensaje de remediación, en lugar de limitar la API en silencio.

## Proveedores de usuario

Un `UserProvider` le dice a la pila de autenticación cómo obtener y
validar usuarios. Dos proveedores vienen incluidos, así que el caso
común no necesita ninguna implementación personalizada:

- **`EloquentUserProvider<M>`** - resuelve a través de un `User`
  tipado con `#[suprnova::model]` que también es `Authenticatable`.
  Busca por clave primaria para los ids, por `email` (por defecto)
  para las credenciales.
- **`DatabaseUserProvider`** - resuelve una tabla cruda por nombre en
  un `GenericUser` (id + mapa de atributos). Úsalo cuando no tengas o
  no quieras un modelo tipado.

Ambos filtran las búsquedas de credenciales contra una lista de
permitidos (por defecto `["email"]`) - un mapa de credenciales hostil
no puede inyectar predicados `WHERE` adicionales. Personaliza la lista
de permitidos con `.credential_columns([...])`, la columna de búsqueda
con `.identifier_column("uuid")`, o la estrategia de vinculación de id
con `.with_id_parser(...)`.

Para conectar una fuente personalizada (LDAP, una API externa),
implementa `UserProvider` directamente. `retrieve_by_id` toma el
identificador como un `&str`:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … obtén de LDAP, devuelve como Arc<dyn Authenticatable>
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials tienen valores
    // por defecto de trait que devuelven None / false. Sobrescríbelos
    // para dar soporte a `Auth::attempt` y `Auth::validate` contra tu
    // fuente.
}
```

Regístralo en el manager:

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## Proteger rutas

### `AuthMiddleware`

Pon una compuerta a las rutas exclusivas para autenticados. Las
solicitudes no autenticadas se redirigen a una página de login o
reciben un `401`:

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` devuelve `401 Unauthorized` en su lugar - mejor
para APIs JSON. `AuthMiddleware::redirect_to("/login")` emite un `302`
para solicitudes normales y un `409 X-Inertia-Location` para
solicitudes de Inertia (que el cliente de Inertia convierte en una
visita de página completa). Para poner la compuerta sobre un guard
específico, encadena `for_guard`:

```rust
// 401 a menos que el guard api esté autenticado.
.middleware(AuthMiddleware::new().for_guard("api"))
```

Un guard de token (`for_guard("api")`) depende de que algún middleware
de bearer token, ejecutado antes en la cadena, rellene el id de
autenticación de la solicitud; sin él, el guard siempre reporta que no
está autenticado.

### `GuestMiddleware`

El inverso - para páginas de login y registro que los usuarios
autenticados no deberían ver:

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` funciona igual que
`AuthMiddleware::for_guard`.

### `BasicAuthMiddleware`

Auth Basic de HTTP desde el encabezado `Authorization: Basic` contra
el proveedor de un guard:

```rust
use suprnova::BasicAuthMiddleware;

// Con estado - loguea al usuario en la sesión si tiene éxito (el `basic` de Laravel).
.middleware(BasicAuthMiddleware::new())

// Sin estado - autentica solo para esta solicitud (el `onceBasic` de Laravel).
.middleware(BasicAuthMiddleware::once())
```

El nombre de usuario decodificado se compara contra la credencial
`field` (por defecto `"email"`); un encabezado ausente, malformado, o
inválido devuelve `401` con un desafío
`WWW-Authenticate: Basic realm="..."`. Configúralo con `.field(...)`,
`.realm(...)`, y `.for_guard(...)`.

## Eventos de ciclo de vida

Los guards despachan cinco eventos de ciclo de vida. Escúchalos vía la
[`EventFacade`](events.md):

| Evento | Cuándo |
|---|---|
| `Attempting` | comienza un intento de credenciales (`attempt`/`once`) |
| `Authenticated` | un usuario queda activamente autenticado en esta solicitud (`login`/`once`/`once_using_id`) |
| `Login` | un usuario se persiste en la sesión (`login`/`attempt` exitoso) |
| `Logout` | un usuario cierra sesión |
| `Failed` | falla un intento de credenciales (contraseña incorrecta o id desconocido) |

Cada evento lleva el nombre del guard y un id de usuario en forma de
string - nunca la contraseña en texto plano ni nunca el mapa de
credenciales crudo. `Authenticated` se dispara solo cuando un usuario
queda activamente establecido, no en una resolución pasiva de
`Auth::user()` sobre una sesión ya existente, así que los oyentes no
reciben un chorro de duplicados en cada solicitud autenticada.

## El flujo de login con andamiaje

`suprnova new` genera un controlador de autenticación que usa
`Auth::attempt` contra el proveedor registrado. Los derives
`FormRequest` y `Validate` del framework manejan la validación por
campo; el cliente de Inertia hace emerger automáticamente un `422` con
`{ message, errors }` en la página de origen:

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

El registro sigue la misma forma: valida el formulario, crea el
usuario, y luego `Auth::login(Arc::new(user), false).await?` loguea al
usuario recién creado en la sesión y dispara el evento `Login`.

## El modelo `User` con andamiaje

El `User` generado es un `#[suprnova::model]` que también implementa
`Authenticatable`. El manejo de contraseñas vive en dos ayudantes
respaldados por el módulo [`hashing`](hashing.md):

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

El atributo `hidden = ["password", "remember_token"]` hace que el
modelo se salte esas columnas al serializar a JSON para la red - existen
en el struct pero nunca se filtran a través de una respuesta de
Inertia.

## Recuérdame

`Auth::attempt(credentials, remember)` con `remember = true` emite un
token de "recuérdame" junto con el login de sesión. El token vive en la
tabla `remember_tokens` (hasheado con bcrypt, rotativo de un solo uso)
y una cookie cifrada correspondiente. En una solicitud futura donde la
sesión ya no existe, `SessionMiddleware` verifica la cookie contra la
fila hasheada, rota el token, e hidrata la sesión - el usuario vuelve a
quedar logueado de forma transparente.

Las apps que ya establecieron una sesión y quieren emitir la mitad de
"recuérdame" por separado (el flujo de desafío de 2FA hace esto)
recurren a `Auth::issue_remember_cookie(&user_id, ttl_minutes).await?`.
`Auth::revoke_remember_tokens()` invalida cada token de "recuérdame"
del usuario actual - el gancho correcto para un botón de seguridad de
cuenta tipo "cerrar sesión en todas partes".

## Garantías de seguridad

Una lista corta de invariantes que establece la pila de autenticación:

- **`Auth::login_id` falla de forma estrepitosa fuera de un alcance de
  solicitud.** Las versiones anteriores descartaban en silencio la
  escritura de sesión; un "login exitoso" que nunca llegó a
  completarse es exactamente el tipo de bug que nada más detecta.
- **El id de sesión y el token CSRF se regeneran en cada login.**
  Tanto `login_id` como el `login`/`attempt` respaldado por el guard
  los rotan para prevenir la fijación de sesión.
- **El logout limpia el estado de autenticación antes de revocar el
  remember-me.** Si la revocación en la BD falla, la sesión ya está en
  un estado de sesión cerrada, así que una ranura de autenticación
  obsoleta no puede sobrevivir a un logout parcial. La cookie de
  borrado del remember-me se encola *antes* del delete en la BD, así
  que el navegador descarta la cookie incluso cuando el delete de la
  fila falla (la pasada de poda lo limpia después).
- **Las listas de permitidos de credenciales bloquean la inyección.**
  Ambos proveedores incluidos filtran `retrieve_by_credentials` contra
  `credential_columns`, así que las claves extra en un mapa de
  credenciales influenciado por un atacante no pueden convertirse en
  predicados `WHERE` adicionales.
- **Los eventos de autenticación nunca llevan texto plano.** Nombre
  del guard + id de usuario en forma de string, nada más. El
  seguimiento de intentos fallidos (bloqueos indexados por email)
  pertenece a `BruteForce` en [Flujos de autenticación](auth-flows.md),
  no a los eventos de ciclo de vida.

El capítulo de [Sesiones](session.md) cubre la configuración de
cookies (`SESSION_LIFETIME`, `SESSION_COOKIE`, `SESSION_SECURE`,
`SESSION_SAME_SITE`) que heredan los guards respaldados por sesión.

## Siguiente

- [Flujos de autenticación](auth-flows.md) - verificación de correo,
  restablecimiento de contraseña, limitación de fuerza bruta con
  `LoginThrottleMiddleware`, 2FA TOTP, la suite de eventos de
  `auth_flows`
- [Autorización](authorization.md) - `Gate`, políticas,
  `Authorizable` para "qué puede hacer este usuario"
- [Sesiones](session.md) - la cookie + el almacenamiento que respalda
  los guards de estilo `web`
- [CSRF](csrf.md) - cómo se les pone una compuerta a las solicitudes
  que cambian estado
- [Hashing](hashing.md) - los ayudantes de bcrypt + argon2 detrás de
  `verify_password`
