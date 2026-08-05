# Flujos de autenticación

`suprnova::auth_flows` es la capa de ciclo de vida sobre la
[autenticación de sesión](authentication.md). Donde `auth::*` responde
"quién hace esta solicitud", `auth_flows::*` responde a todo lo que
rodea esa pregunta - probar que la dirección de correo es real,
recuperarla cuando se pierde la contraseña, defenderla contra el
relleno de credenciales, y protegerla con un segundo factor. Cinco
flujos se incluyen bajo un único namespace:

- `EmailVerification` - acuña, comprueba, y consume tokens de
  verificación de un solo uso; `send_link` / `resend` despachan el
  correo de verificación a través de la fachada [`Mail`](mail.md), y
  `verify` marca al usuario como verificado a través del proveedor de
  usuario configurado.
- `PasswordReset` - un `send_link` anti-enumeración, un `check` que no
  consume, y `complete`. `complete` rota la contraseña a través del
  proveedor de usuario configurado, revoca cada sesión y cada fila de
  remember-me del usuario, y envía una notificación de seguridad
  `PasswordChangedMail`.
- `BruteForce` + `LoginThrottleMiddleware` - estado de bloqueo
  respaldado por torii, más un middleware HTTP que hace cortocircuito
  con `429 Too Many Requests` antes de que se invoque el handler de
  login.
- `TwoFactor` - inscripción TOTP, confirmación, verificación, códigos
  de recuperación, rotación de secretos, el flujo de desafío completo
  que pone una compuerta al login por contraseña mediante el segundo
  factor, y protección contra repetición a la granularidad de paso de
  tiempo de 30 segundos.
- `remember_me` - re-export de `crate::auth::remember` (cookies
  persistentes con fila en BD + bcrypt + rotación de un solo uso) para
  cohesión de namespace.

Dos middleware de compuerta de ruta se incluyen en el mismo namespace:

- `EnsureEmailVerifiedMiddleware` - se compone después de
  `AuthMiddleware` para poner una compuerta a las rutas según
  `email_verified_at`.
- `TwoFactorChallengeMiddleware` - se compone delante de
  `AuthMiddleware` para desviar una sesión con un desafío de 2FA
  pendiente hacia el formulario de desafío en lugar de hacia la página
  de login.

Cada mensaje transaccional se entrega a través de la fachada
[`Mail`](mail.md). La feature opcional `mailer` de torii está
deliberadamente desactivada en `framework/Cargo.toml`: correr una
segunda pila de correo dentro de torii dividiría la telemetría,
duplicaría la superficie de configuración de transporte, y obligaría a
las apps a cablear dos direcciones "from".

### Dónde vive el estado

La verificación de correo y el restablecimiento de contraseña son
**agnósticos al proveedor**. Los tokens de verificación y
restablecimiento viven en la tabla propia del framework
`auth_flow_tokens` (de un solo uso, hasheados con SHA-256), y la
búsqueda + mutación de usuario pasan por el
[`UserProvider`](authentication.md) que la app haya registrado - el
mismo proveedor contra el que resuelve `Auth::user`. No hay ninguna
instancia global de autenticación que inicializar para estos dos
flujos: una app recién generada con andamiaje ya tiene
`EloquentUserProvider<User>` vinculado, y eso es todo lo que
`EmailVerification` y `PasswordReset` necesitan.

Torii sigue siendo dueño del estado de seguridad para los flujos que
genuinamente dependen de él - el contador de bloqueo por fuerza bruta
por cuenta, las ceremonias de OAuth / passkey / WebAuthn, y el pool de
sesiones. Suprnova es dueño de las preocupaciones transversales de
cada flujo - el correo saliente, el despacho de eventos, la tabla TOTP
de 2FA, las cookies de remember-me, y el middleware HTTP. El código de
la aplicación solo toca `suprnova::auth_flows::*`. Laravel repliega la
superficie equivalente dentro de Fortify; Suprnova mantiene los traits
de modelo (`MustVerifyEmail` / `CanResetPassword`) y el almacén de
tokens dentro del framework, así que los flujos funcionan contra
cualquier backend de usuario.

## Semántica de fallo entre flujos

Cada fachada sigue una única regla de orden: el cambio de estado
durable se confirma primero, y luego se disparan los efectos
secundarios de notificación. Un pánico de un oyente, un fallo
transitorio del transporte de correo, o un error del despachador
después de la mutación no pueden revertir la mutación.

- `EmailVerification::verify` consume el token y marca al usuario como
  verificado a través del proveedor antes de disparar `EmailVerified`.
- `PasswordReset::complete` primero consume el token y rota la
  contraseña a través del proveedor, luego revoca cada sesión y cada
  fila de remember-me del usuario (se registra en el log ante un
  fallo, no emerge como error), luego despacha `PasswordChangedMail`
  sin esperar el resultado, y luego dispara `PasswordResetCompleted`.
- `BruteForce::unlock_account` confirma el desbloqueo antes de
  disparar `AccountUnlocked`.
- `TwoFactor::confirm` estampa `confirmed_at` antes de disparar
  `TwoFactorEnrolled`; `TwoFactor::disable` elimina la fila antes de
  disparar `TwoFactorDisabled`; `TwoFactor::complete_challenge`
  promueve pendiente → autenticado antes de despachar el par estándar
  `auth::Login` + `auth::Authenticated`, seguido de
  `TwoFactorChallenged`.

Un oyente que necesite durabilidad debería almacenar en búfer su
trabajo (encolar un job desde el cuerpo del oyente); la fachada misma
nunca reintenta.

## Arranque

La verificación de correo y el restablecimiento de contraseña están
respaldados por el proveedor y **no necesitan torii**. La protección
contra fuerza bruta y el 2FA sí necesitan torii. Cablea lo que
requieran los flujos que uses - son independientes.

### Verificación de correo + restablecimiento de contraseña

Tres cosas, todas las cuales ya tiene una app con andamiaje:

1. **Un proveedor de usuario que implemente la superficie de
   auth-flow.** Registra `EloquentUserProvider<User>` (el mismo
   proveedor contra el que resuelve `Auth::user`) como la vinculación
   `dyn UserProvider` en `bootstrap.rs::register()`. Ambas fachadas
   resuelven internamente el proveedor activo; no se pasa ninguna
   instancia en el sitio de la llamada.

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **Los dos traits de modelo en tu `User`.**
   `EloquentUserProvider<User>` solo implementa los métodos de
   auth-flow (`retrieve_by_email` / `mark_email_verified` /
   `set_password` / `is_email_verified`) cuando `User` implementa
   tanto `MustVerifyEmail` como `CanResetPassword` - los análogos de
   Suprnova de los contratos `MustVerifyEmail` / `CanResetPassword` de
   Laravel:

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // El valor llega ya hasheado - guárdalo tal cual.
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` tiene un valor por defecto que sigue la marca
   de tiempo (`email_verified_at().is_some()`), y `name()` por
   defecto es `None` - sobrescríbelo para saludar a los usuarios por
   su nombre en el correo.

3. **Dos columnas / tablas en tu migrador.** La tabla `users` necesita
   una marca de tiempo `email_verified_at` que admita NULL (el
   proveedor la lee en `is_email_verified` y la estampa en
   `mark_email_verified`), y la tabla `auth_flow_tokens` de un solo
   uso del framework contiene los tokens de verificación /
   restablecimiento. El framework incluye el `CREATE` de la tabla de
   tokens; lístala en tu migrador:

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   Añade `email_verified_at` a `users` en tu propia migración de
   columna (un `timestamp_with_time_zone` que admita NULL); `NULL`
   significa no verificado, así que las filas existentes se rellenan
   retroactivamente de forma correcta.

Los tokens son de un solo uso y están hasheados con SHA-256 en
reposo - un dump de la base de datos nunca produce un token en texto
plano utilizable. Los TTL por defecto son **24 horas** para la
verificación de correo y **15 minutos** para el restablecimiento de
contraseña.

### Fuerza bruta + 2FA: cablear torii

`BruteForce` / `LoginThrottleMiddleware` y `TwoFactor` están
respaldados por torii - necesitan que la instancia global de torii se
inicialice en `bootstrap.rs::register()`, después de `DB::init`. (Las
ceremonias de OAuth, passkeys, y WebAuthn pasan por la misma
instancia - consulta [Autenticación](authentication.md).)

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` es idempotente. La guarda de `OnceLock` significa que la
segunda llamada es un no-op, así que los harnesses de test que
reentran en `register()` por cada fixture no migran dos veces. Para
tests, cambia a `ToriiConfig::sqlite_in_memory()` - levanta una base de
datos en memoria con caché compartida que sobrevive entre runtimes:

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

### Registrar las migraciones de 2FA

El framework incluye el esquema; tu app se suma listando ambas
migraciones en su propio migrador:

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... tus propias migraciones ...

            // Crea `two_factor_credentials`.
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // Añade `last_used_timestep` para la protección contra repetición de TOTP.
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

Ambas son idempotentes contra una base de datos ya aplicada (la v1 usa
`CREATE TABLE IF NOT EXISTS`; la v2 es una adición de columna). Volver
a ejecutar `suprnova migrate` contra una base de datos de producción
que ya tiene el esquema es un no-op.

### Entorno

Los mailables transaccionales leen dos variables de entorno en el
momento del envío:

| Var | Por defecto | Para qué se usa |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | El branding del subject y la etiqueta de emisor de `otpauth://` que muestran las apps autenticadoras. |
| `MAIL_FROM` | ninguno - **da error si no se establece** | El `From` del envelope en cada mensaje saliente. Ponlo a un dominio de remitente verificado. |

`MAIL_FROM` deliberadamente no tiene ningún valor por defecto. Recurrir
a un marcador de posición como `noreply@example.com` rompería en
silencio DMARC / SPF en producción y enviaría desde un dominio que el
operador no controla, así que la fachada falla en cerrado en su lugar.
`EmailVerification::send_link` y `PasswordReset::send_link` hacen
emerger el error como `Err`; `PasswordReset::complete` lo registra vía
`tracing::warn!` y continúa (el cambio de contraseña ya se confirmó,
así que la ruta de notificación no puede revertirlo).

Las apps además establecen `APP_URL` para que los controladores puedan
derivar la URL base usada en las llamadas a `send_link`; la fachada del
framework en sí toma la URL base como un parámetro.

El driver de correo se configura por separado vía `MAIL_DRIVER` -
consulta la documentación de [Correo](mail.md).

## Verificación de correo

`EmailVerification` acuña, comprueba, y consume tokens de verificación
contra la tabla `auth_flow_tokens`, y marca al usuario como verificado
a través del proveedor configurado. Cuatro operaciones cubren el ciclo
de vida:

| Método | Firma | Notas |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Acuña + envía correo, dado un usuario que ya tienes en mano. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Anti-enumeración: busca al usuario por email; una dirección desconocida es un `Ok(())` silencioso. |
| `check` | `check(token: &str) -> Result<bool>` | No consume - seguro de llamar en una landing page. |
| `verify` | `verify(token: &str) -> Result<String>` | De un solo uso: consume el token, marca al usuario como verificado, devuelve el id del usuario. |

```rust
use suprnova::auth_flows::EmailVerification;

// Después de un registro reciente, con el usuario recién creado en mano:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Comprobación opcional en la landing page - no consume, así que
// refrescar la página no quema el token.
let valid: bool = EmailVerification::check(&token_str).await?;

// El handler del enlace consume el token y estampa al usuario,
// devolviendo el id del usuario verificado.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` dispara `EmailVerified` en caso de éxito - los oyentes son el
lugar correcto para desbloquear funcionalidad adicional (correo de
bienvenida, seguimientos por defecto, un CTA de "completa tu perfil")
sin acoplarlos al handler de verificación. El evento lleva el id de
usuario del proveedor.

### El endpoint resend (anti-enumeración)

`resend` toma solo el email - la fachada busca al usuario a través del
proveedor activo y, cuando hay una cuenta registrada, acuña un token y
envía el correo; un email desconocido es un no-op silencioso que igual
devuelve `Ok(())`. El handler nunca ramifica según la existencia en sí,
así que quien llame sondeando no puede distinguir "enviado" de "no
existe tal cuenta":

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` realiza la búsqueda + anti-enumeración internamente.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` y `resend` construyen ambos la URL como
`{base_url}?token={plaintext_token}`. Una barra final en `base_url` se
recorta antes de añadir la query string, así que
`https://app.example.com/verify/` y `https://app.example.com/verify`
producen ambas una URL limpia.

El handler del enlace extrae el token de la query string y llama a
`verify`:

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;

    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

El handler no necesita buscar al usuario - `verify` consume el token,
marca al usuario como verificado a través del proveedor, devuelve el
id del usuario, y dispara `EmailVerified`. De un solo uso: un segundo
`verify` sobre el mismo token devuelve un error.

### Rutas solo para verificados: `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` pone una compuerta a las rutas según el
`email_verified_at` del usuario autenticado. Compónlo después de
`AuthMiddleware`, y la cadena bloquea cualquier solicitud cuyo usuario
todavía no haya completado el paso de verificación.

La elección entre **403 JSON** y **redirección HTML 302** se hace en
el momento de registrar la ruta, vía el constructor - no hay ningún
sniffing del contenido de la solicitud, siguiendo el mismo patrón que
fija `AuthMiddleware::new` / `AuthMiddleware::redirect_to`:

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// Superficie de API - 403 con un cuerpo JSON.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Superficie web - 302 (o 409 + X-Inertia-Location para visitas de Inertia).
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

Si no hay ningún usuario autenticado, el middleware cae en la misma
rama de respuesta que "autenticado pero no verificado" - siguiendo la
misma forma que `! $request->user() || ! hasVerifiedEmail()` de
Laravel. Compón `AuthMiddleware` primero cuando quieras un `401`
separado para solicitudes no autenticadas.

Para ramificar dentro del handler (por ejemplo, renderizar
condicionalmente un CTA de "por favor verifica" sin redirigir), carga
el usuario tipado a través del guard de sesión y lee el método del
trait:

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // ramifica según su valor
}
```

## Restablecimiento de contraseña

`PasswordReset` tiene tres operaciones:

| Método | Firma | Notas |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Anti-enumeración: busca al usuario por email; una dirección desconocida es un `Ok(())` silencioso. |
| `check` | `check(token: &str) -> Result<bool>` | No consume - confirma el token antes de renderizar el formulario de nueva contraseña. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | De un solo uso: consume el token, rota la contraseña, revoca sesiones + remember-me, envía la notificación de cambio, devuelve el id del usuario. |

```rust
use suprnova::auth_flows::PasswordReset;

// Desde el formulario de "olvidé mi contraseña". Siempre Ok(()) - la fachada busca
// al usuario y solo envía cuando hay una cuenta registrada.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Comprobación opcional en la landing page antes de renderizar el formulario de nueva contraseña.
let valid: bool = PasswordReset::check(&token).await?;

// El handler del enlace, después de que el usuario envía una contraseña nueva:
// consume el token + rota la contraseña, devolviendo el id del usuario.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` hashea `new_password` antes de entregarla al proveedor -
pasa el texto plano, no un valor ya hasheado. Una contraseña vacía / de
solo espacios se rechaza de entrada con un `400`.

### Anti-enumeración

`send_link` está estructurado para que la forma de la respuesta nunca
filtre si una dirección de correo tiene una cuenta:

- Siempre devuelve `Ok(())`. Cuando el email está ausente no se acuña
  ningún token, no se despacha ningún correo, y no se dispara ningún
  evento `PasswordResetLinkSent` - pero la ausencia tampoco emerge a
  través del tipo de retorno, así que quien llame (y un observador de
  la red) no puede distinguir "no existe tal cuenta" de "enlace
  enviado".
- El controlador dogfood empareja `send_link` con un cuerpo de
  respuesta 200 fijo, así que quien llame sondeando no puede
  distinguir a través del código de estado, el cuerpo de la respuesta,
  o el tiempo de respuesta.

### Efectos secundarios de `complete`

`complete` ejecuta cuatro pasos en orden:

1. Consume el token (de un solo uso) y rota el hash de contraseña a
   través del proveedor configurado (el único paso que puede hacer
   fallar la llamada).
2. Revoca cada fila de sesión del usuario vía
   `crate::session::destroy_all_for_user` (best-effort: los fallos van
   a `tracing::warn!`).
3. Revoca cada fila de remember-me vía
   `crate::auth::remember::revoke_all_for_user` (best-effort).
4. Despacha `PasswordChangedMail` sin esperar el resultado, y luego
   dispara `PasswordResetCompleted`.

Una sesión robada y una cookie de remember-me capturada no deben
sobrevivir a la credencial de la que dependían. Las revocaciones
ocurren en cada restablecimiento exitoso, no solo en los iniciados por
el usuario, así que un restablecimiento forzado por el equipo de
seguridad también expulsa a un atacante activo.

## Protección contra fuerza bruta

La capa de fuerza bruta tiene dos partes: la fachada `BruteForce` que
registra y consulta el estado de bloqueo, y el `LoginThrottleMiddleware`
que hace cortocircuito en la capa HTTP antes de que se invoque el
handler.

### La fachada `BruteForce`

Llama a `record_failed_attempt` desde la rama de autenticación fallida
de tu handler de login, y a `reset_attempts` desde la rama de éxito:

```rust
use suprnova::auth_flows::BruteForce;

// En la ruta de autenticación fallida:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Opcionalmente, haz emerger una respuesta personalizada. El middleware
    // hará esto por ti en la *siguiente* solicitud - ver más abajo.
}

// En la ruta de éxito:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` devuelve el `LockoutStatus` actualizado
(`is_locked`, `failed_attempts`, y `locked_until` cuando está
bloqueado). Pasa el `ip` opcional para los logs de auditoría; pasa
`None` si tu transporte no expone la IP del cliente de forma limpia.

Dos operaciones adicionales:

```rust
// Solo lectura - segura sobre emails sin historial.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Desbloqueo de admin / forzado. Dispara `AccountUnlocked` solo ante una
// transición de estado real (un desbloqueo no-op sobre una cuenta ya desbloqueada no lo dispara).
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` devuelve `true` cuando la cuenta estaba bloqueada en
el momento de la llamada, `false` en caso contrario. El evento
`AccountUnlocked` se dispara solo con `true` - un retorno `false` es el
no-op que es, no un evento de auditoría.

### `LoginThrottleMiddleware`

El middleware lee el estado de bloqueo del email al que apunta una
solicitud y hace cortocircuito con `429 Too Many Requests` cuando la
cuenta está bloqueada. El handler de login nunca se invoca, así que una
cuenta bloqueada ni siquiera llega a intentar una comprobación de
credenciales:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// El extractor de email es un closure sync sobre `&Request`. Leer
// el cuerpo JSON/form es async y consume `Request`, así que el closure
// no puede leer el cuerpo - obténlo de un encabezado, una query
// string, o un parámetro de ruta en su lugar.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Superficies prácticas de extracción:

- Un encabezado (`X-Login-Email`), fijado por un preprocesador
  anterior - el patrón que usa la app dogfood.
- Un parámetro de query string (`?email=…`).
- Un parámetro de ruta (`/login/{email}`).

Devolver `None` desde el extractor es la señal explícita de "no tengo
nada que comprobar" - el middleware deja pasar la solicitud sin
cambios. Esto hace que sea seguro instalar el middleware en rutas que
ocasionalmente ven tráfico anónimo (por ejemplo, el mismo endpoint
`POST /login` que también maneja una sub-acción sin email de
"solicitar restablecimiento de contraseña").

Al bloquear, el middleware devuelve:

- Status `429 Too Many Requests`.
- Encabezado `Retry-After` - segundos, calculados a partir del
  `locked_until` del bloqueo vía `LockoutStatus::retry_after_seconds`.
  Recurre a `900` (15 minutos - el periodo de bloqueo por defecto de
  torii) si la marca de tiempo está ausente por algún motivo.
- Cuerpo: `"Account locked due to too many failed login attempts. Try
  again later."`

### Fail-open ante errores de backend

Si `get_lockout_status` devuelve un `Err` (un traspié transitorio de la
base de datos), el middleware deja pasar la solicitud. El handler de
login más adelante hará la llamada él mismo y puede decidir si falla en
cerrado o en abierto. El middleware se equivoca del lado de la
disponibilidad: tumbar el endpoint de login cada vez que la base de
datos de autenticación tiene un tropiezo es peor que dejar que el
handler haga la llamada directamente.

### Apilar con `RateLimitMiddleware`

`LoginThrottleMiddleware` es por cuenta - pone una compuerta a un único
email cuando se supera el umbral. Para cuotas por IP, apílalo con
[`RateLimitMiddleware`](rate-limiting.md). Los dos se componen de forma
natural:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Juntos cubren las formas realistas del relleno de credenciales:
distribuido (un email × muchas IPs) es tarea del rate limit;
concentrado (muchos intentos × un email) es tarea del middleware de
throttle.

### Configuración

El `BruteForceProtectionConfig` de torii tiene por defecto **5
intentos fallidos antes del bloqueo** y un **periodo de bloqueo de 15
minutos**. Esto es lo que `init_torii` cablea hoy; configurar valores
por app requiere entrar en la propia superficie de configuración de
torii, y no se expone a través del builder `ToriiConfig` de Suprnova.
Los valores por defecto son deliberadamente conservadores - asume
"cinco erratas me bloquean 15 minutos" antes de decidir relajarlos.

## Dos factores (TOTP)

`TwoFactor` cubre el 2FA basado en TOTP - el tipo que se empareja con
cualquier app autenticadora que cumpla el estándar (Google
Authenticator, 1Password, Bitwarden, Authy). El flujo es inscripción →
confirmación → verificación continua, más códigos de recuperación de
un solo uso para cuando el usuario pierde su dispositivo, más el flujo
de desafío que cose todo dentro del ciclo de vida del login.

### El trait `TwoFactorUser`

El framework no puede entrar en el almacenamiento de usuarios de tu
aplicación, así que quien llama implementa un pequeño trait para hacer
de puente entre su modelo de usuario y la fachada de 2FA:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` es la clave de almacenamiento opaca - típicamente
`torii::UserId.as_str()`, pero funciona cualquier identificador estable
por usuario. La tabla de 2FA indexa por ella; no hay ninguna FK hacia
tu tabla de usuarios.

`email` se incorpora al segmento `account_name` de la URL
`otpauth://`, así que la app autenticadora renderiza la fila con una
etiqueta legible para humanos (por ejemplo, "MyCorp
(alice@example.com)").

Un patrón común es un newtype pequeño que envuelve tu modelo de
usuario:

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
}
```

### Almacenamiento

El estado de 2FA vive en la tabla `two_factor_credentials`, propiedad
del framework. Los secretos y los códigos de recuperación se cifran en
reposo con `crate::crypto::Crypt::encrypt_string`, que requiere una
`EncryptionKey` global al proceso. Las apps se suman al esquema
listando ambas migraciones en su `Migrator::migrations()` - consulta
[Arranque](#arranque).

### Inscribir, confirmar, verificar

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Inscripción: genera un secreto nuevo + 10 códigos de recuperación, persístelos
//    cifrados, y devuelve todo lo necesario para renderizar el código QR.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - deep link `otpauth://totp/...`
// response.qr_code_svg - <svg> que envuelve un PNG en base64, embébelo inline
// response.recovery_codes - Vec<String>, 10 códigos en texto plano - muéstralos UNA sola vez

// 2. Confirma: el usuario abre la app autenticadora y escribe el
//    código de 6 dígitos. `confirm` lo valida y estampa `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// dispara `TwoFactorEnrolled`

// 3. En logins posteriores, pon una compuerta a la sesión mediante `verify`:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` devuelve los códigos de recuperación en texto plano
**exactamente una vez**. No hay ninguna API para recuperarlos
después - la columna cifrada es unidireccional desde este punto en
adelante. Muéstralos en la página de éxito de la inscripción, anima al
usuario a guardarlos, y no almacenes el texto plano en ningún otro
sitio.

`enroll` se niega a sobrescribir una inscripción **confirmada** -
devuelve un `409` para empujar a quien llama hacia `re_enroll`, que
requiere prueba de posesión. Reinscribirse sobre una fila no confirmada
(pendiente) sí está permitido: la inscripción previa nunca llegó a ser
autoritativa.

### Protección contra repetición

`verify` escribe el paso de tiempo TOTP actual en `last_used_timestep`
en caso de éxito. Los verifies posteriores donde `current_timestep <=
last_used_timestep` se rechazan incluso cuando el código en sí es
estructuralmente válido, derrotando una repetición de código robado
dentro de la ventana de 30 segundos.

La reclamación del paso de tiempo es atómica. El estampado llega vía un
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep <
:current` condicional, y el verify solo tiene éxito cuando la
instrucción afecta exactamente a una fila. Dos verifies concurrentes en
el mismo paso de tiempo no pueden ganar ambos: el primero invierte la
columna, el predicado del segundo ya no coincide, y el segundo se trata
como una repetición. Un read-modify-write sencillo sería una carrera
TOCTOU - ambos verifies leerían la fila antes del estampado, ambos
validarían el mismo código, ambos estamparían, ambos tendrían éxito.
Los competidores concurrentes también se cuentan como intentos
fallidos, así que el contador de fuerza bruta los registra.

### Códigos de recuperación

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

De un solo uso: un código coincidente se elimina de la fila antes de
que la llamada retorne, así que un segundo intento contra el mismo
código devuelve `false`. Los códigos son 12 dígitos decimales con forma
`NNNNNN-NNNNNN` (~40 bits de entropía cada uno, igualando el formato de
Fortify de Laravel).

`consume_recovery_code` solo acepta códigos cuando el 2FA está
completamente confirmado - hace cortocircuito a `Ok(false)` mientras
`confirmed_at` sea NULL. Sin esta compuerta, un atacante que haya
disparado la inscripción sobre la cuenta de una víctima (o cualquier
flujo que cree la fila sin confirmar) podría autenticarse usando solo
un código de recuperación fresco, evitando TOTP por completo. El
contrato es simétrico con la salvaguarda de `verify` de "solo
inscripción confirmada".

### Rotar códigos de recuperación y secretos

Cuando un usuario agota sus códigos de recuperación, o quiere rotarlos
después de un compromiso sospechado:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` debe validar como un código TOTP actual o como un código de
recuperación no usado. Sin la comprobación de `proof`, un atacante que
haya secuestrado la sesión podría destruir en silencio los códigos de
recuperación del usuario legítimo (denegación de servicio contra la
recuperación de la cuenta). Los códigos nuevos reemplazan el conjunto
persistido; el secreto existente y `confirmed_at` se preservan, así que
la app autenticadora del usuario sigue funcionando sin volver a
emparejarse. Errores:

- `400` - no existe ninguna inscripción confirmada; llama primero a
  `enroll`/`confirm`.
- `401` - `proof` no valida ni como código TOTP ni como código de
  recuperación no usado.
- `429` - la cuenta está bloqueada por la limitación de fuerza bruta.

Para rotar el **secreto** (volver a emparejar con un dispositivo nuevo)
sin desactivar el 2FA primero:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

Mismo modelo de `proof` que `regenerate_recovery_codes`. La fila se
reescribe con un secreto nuevo + 10 códigos de recuperación nuevos;
`confirmed_at` vuelve a NULL, así que el usuario debe `confirm` con un
código de la nueva app autenticadora antes de que el 2FA vuelva a
estar activo.

### Desactivar

```rust
TwoFactor::disable(&user_2fa).await?;
// dispara `TwoFactorDisabled` solo si se eliminó una fila
```

Idempotente: desactivar sobre un usuario que nunca se inscribió no es
un error. El evento `TwoFactorDisabled` se dispara solo ante una
transición de estado real, así que los oyentes de auditoría ven una
entrada por cada desactivación real, en lugar de una por cada clic en
un botón que no hace nada.

### Flujo de desafío (poner una compuerta al login mediante el segundo factor)

Las primitivas enroll / confirm / verify son los bloques de
construcción; el **flujo de desafío** las cose dentro del ciclo de vida
del login, de modo que un usuario con el 2FA activado no pueda llegar a
páginas protegidas solo con la contraseña.

El flujo:

1. El login por contraseña resuelve un usuario.
2. Si `TwoFactor::is_enabled_by_id(&user_id)` devuelve `true`, el
   handler de login llama a `TwoFactor::start_challenge(user_id,
   remember)` - eso guarda el user-id como **pendiente** en la sesión,
   limpia la ranura de totalmente autenticado, revoca cualquier cookie
   de remember-me emitida por `Auth::attempt`, y recuerda si el
   usuario optó por remember-me para que la cookie pueda reemitirse
   después de que se complete el desafío. `Auth::id()` devuelve `None`
   desde este punto hasta que se completa el desafío.
3. El handler redirige a una ruta `/two-factor-challenge` que muestra
   el formulario de código.
4. El handler POST del desafío llama a
   `TwoFactor::complete_challenge(code)` - verifica el código (TOTP
   **o** un código de recuperación no usado, igualando el controlador
   de desafío de Fortify), promueve pendiente → autenticado, rota el
   id de sesión (derrotando la fijación de sesión) y el token CSRF,
   reemite la cookie de remember-me cuando el usuario optó por ella, y
   despacha los eventos de ciclo de vida estándar `auth::Login` +
   `auth::Authenticated`, más el evento específico de 2FA
   `TwoFactorChallenged`.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Degrada a "pendiente": la ranura de autenticación se limpia,
                // se fija pendiente, se revoca la cookie de remember-me. Pasa el
                // flag remember del formulario para que `complete_challenge`
                // pueda reemitir la cookie si tiene éxito.
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // El id de sesión + el CSRF han rotado; el remember-me se ha reemitido
    // si el formulario de login original lo activó. Los oyentes que se
    // engancharon a `auth::Login` / `auth::Authenticated` vieron un login normal.
    redirect!("/dashboard").into()
}
```

`complete_challenge` rota el id de sesión y el token CSRF como parte
de la promoción a autenticado. Eso cierra el ataque clásico de
fijación de sesión, en el que un atacante planta un id de sesión
conocido sobre una víctima antes de que inicie sesión - después de la
rotación, el id plantado está muerto y solo el id recién generado
lleva el estado autenticado. El contrato coincide con
`Auth::login_id` / `Auth::login_using_id`, así que los logins con 2FA
son indistinguibles de los logins sin 2FA en cuanto al estado de
sesión y a la observabilidad para los oyentes.

Ponle una compuerta a cada grupo de rutas protegido con
`TwoFactorChallengeMiddleware` **antes** de `AuthMiddleware`, para que
una sesión pendiente se desvíe hacia la página de desafío en lugar de
hacia la página de login:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

La página de desafío en sí (el GET que renderiza el formulario, el
POST que llama a `complete_challenge`) NO debe instalar
`TwoFactorChallengeMiddleware` - ella es el destino. El handler POST
típicamente también comprueba `TwoFactor::pending_user_id().is_some()`
de entrada, así que un enlace obsoleto no llega a la lógica de verify
con una sesión vacía.

`TwoFactor::cancel_challenge()` limpia ambas ranuras pendientes sin
autenticar a nadie - cablealo a un enlace de "volver al login" en la
página de desafío.

**Alternativa de código de recuperación.** `complete_challenge(code)`
prueba primero la ruta TOTP y recurre a consumir un código de
recuperación como alternativa, así que un usuario que perdió su app
autenticadora todavía puede entrar. Cada código de recuperación es de
un solo uso.

**Vínculo con la fuerza bruta.** Los códigos de desafío fallidos
alimentan el contador de fuerza bruta por cuenta a través de
`BruteForce::record_failed_attempt`, de la misma forma que hace el
`TwoFactor::verify` puro. Un atacante que fuerce el formulario de
desafío disparará `AccountLocked` al superar el umbral configurado. Un
único envío incorrecto cuenta como **un** intento fallido, aunque
`complete_challenge` prueba internamente tanto la ruta TOTP como la de
código de recuperación - los núcleos de validación silenciosa se
saltan el contador de fuerza bruta, así que la capa externa registra
el intento canónico exactamente una vez.

**Compuerta de bloqueo.** `complete_challenge` comprueba
`BruteForce::is_locked` de entrada y devuelve `429 Too Many Requests`
si la cuenta ya está bloqueada - incluso cuando el código enviado es
correcto. Sin esta compuerta dentro del método, un atacante que
disparó el bloqueo todavía podría entrar enviando el código correcto
en la siguiente solicitud: el contador de fuerza bruta se indexa por
el email del usuario, pero `verify` en sí no lo consulta. El
`LoginThrottleMiddleware` de la ruta de contraseña impone la misma
restricción en la capa de ruta; componerlo delante de la ruta POST del
desafío está bien - ambas compuertas son idempotentes.

**Evento de fallo.** `complete_challenge` despacha
`TwoFactorChallengeFailed { user_id }` ante un código incorrecto (o
una cuenta bloqueada), distinto del `auth::Failed` de la ruta de
contraseña. Los oyentes que vigilan "el usuario probó el 2FA y falló"
se suscriben al evento nuevo; los oyentes que vigilan "la contraseña
no autenticó" se quedan en `auth::Failed`. Las dos superficies se
mantienen separadas para que una errata de 2FA no parezca un fallo de
contraseña a ojos de los pipelines de auditoría.

### Por qué Suprnova diverge

El `user_id` de 2FA es deliberadamente un `String`. Si estuviera
tipado como `i64`, `Uuid`, o `torii::UserId`, la tabla de 2FA quedaría
permanentemente atada a la forma que el framework elija primero - las
apps que almacenan usuarios con una forma distinta (UUIDs frente a
enteros autoincrementales, o apps que no usan torii en absoluto pero
quieren el módulo de 2FA) quedarían excluidas. Un `user_id` en forma de
string permite que cada app elija el identificador estable por usuario
que prefiera; la contrapartida es un `.to_string()` en el sitio de la
llamada. El Fortify de Laravel ata la columna equivalente al `User::id`
de Eloquent - Suprnova la desacopla, así que `TwoFactor` es una
primitiva de ciclo de vida reutilizable, no un accesorio con forma de
User.

## Recuérdame

`suprnova::auth_flows::remember_me` re-exporta
`suprnova::auth::remember` - el módulo de cookies persistentes que ya
se incluía junto a la autenticación de sesión. El re-export es
puramente organizativo: todo lo que tiene forma de auth-flow vive bajo
`auth_flows::*`, incluso cuando la implementación es anterior a este
namespace.

El diseño que se incluye:

- **Fila en BD + hash bcrypt** - cada token emitido tiene una fila en
  la tabla `remember_tokens` que almacena solo el hash bcrypt, nunca el
  texto plano. Un dump de la base de datos no puede producir
  credenciales que permitan volver a autenticarse.
- **Rotación de un solo uso** - una verificación exitosa hace DELETE
  de la fila coincidente y emite una nueva. Una cookie capturada no
  puede reutilizarse; si el atacante y la víctima compiten por usarla,
  quien pierda la carrera ve la fila desaparecida y no logra
  autenticarse.
- **Revocación** - `revoke_all_for_user` borra cada fila de un usuario
  en un solo DELETE. `Auth::logout` encadena esto para que un logout
  real limpie de verdad el estado persistente, y
  `PasswordReset::complete` hace lo mismo para que un
  restablecimiento de contraseña invalide cada cookie persistente
  existente.
- **Poda** - `prune_expired` limpia las filas caducadas según una
  programación.

En la práctica, el middleware de sesión del framework hace el trabajo
pesado; la app típica no llama directamente al módulo `remember_me`.
El documento de [Autenticación](authentication.md) cubre la superficie
de cara al usuario - el flag `remember` en `Auth::login`, el nombre de
la cookie, y los mandos de duración.

## Eventos

Nueve eventos se disparan a través de los flujos, uno por cada
transición de estado de seguridad:

| Evento | Disparado por | Lleva |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` en caso de éxito | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` en caso de éxito - silencioso por anti-enumeración para emails ausentes | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` en caso de éxito | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` en la transición desbloqueado → bloqueado | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` cuando ocurre un desbloqueo real | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` en caso de éxito | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` promovió pendiente → autenticado | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` rechazó un código incorrecto o rehusó una cuenta bloqueada | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` cuando de verdad se eliminó una fila | `user_id: String` |

Cada evento es `Debug + Clone + 'static`, no lleva ningún dato
sensible (sin tokens en texto plano, sin IPs), y usa identificadores en
forma de string, así que los oyentes pueden serializarlos a través de
fronteras de tarea sin filtrar información de tipo del backend de
almacenamiento de usuarios.

### Escuchar

Suscríbete vía la API de eventos estándar - la misma superficie que
cualquier otro evento dentro del proceso:

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ... notificación a Slack, append a la tabla de auditoría, etc.
        Ok(())
    }
}

// En bootstrap.rs:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Los oyentes corren en el runtime de Tokio y se despachan en el orden
de registro. Consulta el capítulo de [Eventos](events.md) para la
superficie completa.

## Pruebas

Tres fakes cubren la superficie de auth-flows, y se componen entre sí.

### `Mail::fake()`

Instala un transporte de captura local al proceso. Cada envío durante
la vida de la guarda cae en un búfer en memoria en lugar de salir de
verdad:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... conduce el flujo ...
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` expone `assert_sent`, `assert_not_sent`,
`assert_sent_count`, además de los accessors crudos `captured()` y
`count()`. Cuando la guarda se descarta, se restaura el transporte
previamente vinculado - los tests que entrelazan fakes con
vinculación de transporte explícita no filtran estado.

### `EventFacade::fake()`

La misma forma, pero para eventos:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... conduce el flujo ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

El fake registra los eventos despachados sin invocar a los oyentes,
así que un oyente que hable con un servicio externo no se disparará
durante el test. El `assert_not_dispatched::<E>(pred)` que lo acompaña
afirma el negativo; `dispatched_count::<E>(pred)` devuelve el conteo
crudo para afirmaciones más finas.

### Tests de integración para verificación de correo + restablecimiento de contraseña

Los tests de verify / reset no necesitan torii - provisiona la tabla
`auth_flow_tokens` en una base de datos en memoria, registra un
proveedor, establece `MAIL_FROM`, y conduce la fachada bajo
`Mail::fake()`. Los propios tests del framework acuñan la tabla
directamente desde `create_auth_flow_tokens_table()`:

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // Las fachadas leen MAIL_FROM (fail-closed); establécela para el test.
    // SAFETY: serializado por `#[serial]` - sin observador en paralelo.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // ... conduce EmailVerification::send_link(&user, base) ...
    fake.assert_sent_to("ada@example.com");
}
```

Las rutas respaldadas por el proveedor (`resend` / `verify` /
`complete`) además registran una vinculación `dyn UserProvider` para
que la búsqueda + mutación resuelvan - consulta
`framework/tests/email_verify.rs` y
`framework/tests/password_reset.rs`.

### `ToriiConfig::sqlite_in_memory()` para tests de fuerza bruta + 2FA

Los tests de fuerza bruta y 2FA levantan un torii nuevo sobre una base
de datos SQLite en memoria. Los archivos de test de ejemplo en
`framework/tests/` usan un patrón de runtime compartido +
`once_cell::sync::Lazy<()>` para amortizar el costo entre tests, más
`#[serial]` para mantener estable el transporte de correo global al
proceso entre tests que entrelazan `Mail::fake()`:

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // ... usa Mail::fake() / EventFacade::fake() aquí ...
    });
}
```

Ejemplos canónicos - copia de estos al escribir los tuyos:

- `framework/tests/email_verify.rs` - round-trip del token de verify,
  recorte de la barra final en `send_link`, afirmaciones de
  `Mail::fake()` sobre subject/HTML.
- `framework/tests/password_reset.rs` - round-trip de reset con
  autenticación de la nueva contraseña, anti-enumeración sobre emails
  desconocidos, `complete` rechaza tokens reutilizados.
- `framework/tests/brute_force.rs` - ciclo de vida completo del
  bloqueo, `AccountLocked` se dispara una vez por transición,
  `unlock_account` devuelve `was_locked`.
- `framework/tests/two_factor.rs` - enroll → confirm → verify completo
  con un código TOTP real calculado a partir de la URL otpauth, un
  solo uso del código de recuperación, la reinscripción sobrescribe el
  secreto, rechazo de repetición entre dos verifies concurrentes.
- `framework/tests/two_factor_challenge_flow.rs` - el flujo de desafío
  de punta a punta con rotación de sesión, reemisión del remember-me,
  y despacho de eventos.
- `framework/tests/email_verified_middleware.rs` y
  `two_factor_challenge_middleware.rs` - formas de respuesta del
  middleware (403 JSON frente a 302 frente a 409 + X-Inertia-Location).

## Referencia

| Símbolo | Propósito |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, `verify` - respaldado por el proveedor; `verify` devuelve el id del usuario. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` para 403 JSON, `redirect_to(path)` para 302 / 409 + X-Inertia-Location. Comprueba el `is_email_verified` del proveedor configurado (fail-closed). |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` - respaldado por el proveedor; `complete` devuelve el id del usuario. |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | Traits de modelo que un usuario detrás de `EloquentUserProvider` implementa, para que las fachadas de verify / reset puedan leer su email + escribir su marca de tiempo de verificación / hash de contraseña. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | `CREATE TABLE` de SeaORM para `auth_flow_tokens` - lístala en tu migrador. |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`, `reset_attempts`, `get_lockout_status`, `is_locked`, `unlock_account`. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP que responde 429 antes del handler cuando la cuenta objetivo está bloqueada. |
| `suprnova::auth_flows::TwoFactor` | `enroll`, `re_enroll`, `confirm`, `verify`, `consume_recovery_code`, `regenerate_recovery_codes`, `is_enabled`, `is_enabled_by_id`, `start_challenge`, `pending_user_id`, `cancel_challenge`, `complete_challenge`, `disable`. |
| `suprnova::auth_flows::TwoFactorUser` | Trait que hace de puente entre el modelo de usuario de la app y la fachada de 2FA. |
| `suprnova::auth_flows::EnrollmentResponse` | Valor de retorno de `TwoFactor::enroll` - `otpauth_url`, `qr_code_svg`, `recovery_codes`. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | `new()` para 403 JSON, `redirect_to(path)` para 302 / 409 + X-Inertia-Location. Compónlo delante de `AuthMiddleware`. |
| `suprnova::auth_flows::two_factor::migration::Migration` | Migración de SeaORM para `two_factor_credentials`. Lístala en tu `Migrator::migrations()`. |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | Adición de columna para `last_used_timestep` (protección contra repetición de TOTP). Lístala después de la migración de creación de tabla. |
| `suprnova::auth_flows::remember_me` | Re-export de `suprnova::auth::remember`. |
| `suprnova::auth_flows::events::*` | Nueve eventos - consulta [Eventos](#eventos). |
| `suprnova::auth_flows::EmailVerificationMail` | Mailable transaccional. Subject `"Verify your email for {APP_NAME}"`. |
| `suprnova::auth_flows::PasswordResetMail` | Mailable transaccional. Subject `"Reset your {APP_NAME} password"`. |
| `suprnova::auth_flows::PasswordChangedMail` | Mailable de notificación de seguridad. Subject `"Your {APP_NAME} password was changed"`. |
| `suprnova::torii_integration::ToriiConfig` | Configuración de arranque de torii. `from_sea_orm(conn)` para producción, `sqlite_in_memory()` para tests. |
| `suprnova::torii_integration::init_torii` | Init global idempotente. Llámalo una vez desde `bootstrap.rs::register()`. |

## Siguiente

- [Autenticación](authentication.md) - guards, proveedores, la fachada
  `Auth`, `AuthMiddleware`.
- [Correo](mail.md) - la capa de transporte a través de la cual
  despachan las llamadas a `send_link`.
- [Eventos](events.md) - registrar oyentes para los nueve eventos de
  auth-flow.
- [Limitación de velocidad](rate-limiting.md) - empareja
  `RateLimitMiddleware::ip_based` con `LoginThrottleMiddleware` para
  una defensa en capas.
- [Sesiones](session.md) - qué tocan `start_challenge` /
  `complete_challenge` cuando rotan el id de sesión.
