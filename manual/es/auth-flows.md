# Flujos de autenticación

`suprnova::auth_flows` es la capa de ciclo de vida sobre
[autenticación](authentication.md). Donde `auth::*` responde «quién es esta
solicitud», `auth_flows::*` cubre prueba de buzón, recuperación de
contraseña, bloqueo de cuenta y desafíos TOTP del framework.

Cinco superficies viven en el namespace:

- `EmailVerification` acuña y consume `auth_flow_tokens`, envía correo
  mediante la fachada [`Mail`](mail.md) y marca como verificado al
  propietario autenticado del token a través del `UserProvider`.
- `PasswordReset` delega emisión, prueba, mutación de contraseña,
  rotación de época y revocación de sesión al motor Magnetar instalado;
  el framework posee el correo y los eventos.
- `BruteForce` y `LoginThrottleMiddleware` delegan el bloqueo de cuenta
  al motor Magnetar instalado.
- `TwoFactor` es la fachada TOTP del framework sobre
  `two_factor_credentials`: inscripción, confirmación, verificación,
  códigos de recuperación, rotación, promoción del desafío y protección
  contra repetición de timestep.
- `remember_me` reexporta el módulo legacy para compatibilidad de
  namespace. Con Magnetar instalado, `Auth` y `SessionMiddleware` usan
  sus credenciales remember.

Dos middleware de compuerta de ruta viven en el mismo namespace:

- `EnsureEmailVerifiedMiddleware` se compone después de `AuthMiddleware`
  y restringe las rutas a `email_verified_at`.
- `TwoFactorChallengeMiddleware` se compone antes de `AuthMiddleware` y
  redirige una sesión con desafío TOTP pendiente al formulario.

Los mensajes transaccionales siempre usan la fachada del framework
[`Mail`](mail.md). Magnetar proporciona motores de seguridad y contratos
de almacenamiento, pero no instala un segundo transporte de correo.

### Dónde vive el estado

Los tokens de verificación viven en `auth_flow_tokens` y la marca de
tiempo verificada se escribe mediante `UserProvider`. La verificación
está ligada al actor: el usuario autenticado debe ser dueño del token.

Tokens de restablecimiento, credenciales de contraseña, filas de bloqueo,
sesiones opacas, credenciales remember, ceremonias passkey y OAuth y
épocas de autenticación pertenecen al motor Magnetar instalado. El
restablecimiento, el enlace mágico y OAuth con email verificado comparten
la frontera atómica de primera prueba para recuperar cuentas no verificadas.

La fachada pública `TwoFactor` de este capítulo conserva su esquema `two_factor_credentials`, propiedad del framework. Magnetar también tiene un motor de factores que usan los flujos integrados de contraseña, enlace mágico, passkey, OAuth y sesión. No asumas que ambos almacenes son intercambiables: usa de forma consistente una sola superficie de inscripción para una aplicación determinada.

Suprnova sigue siendo dueño del middleware HTTP, cookies, correo saliente,
eventos y puente `UserProvider`. El código de aplicación usa fachadas,
no motores de almacenamiento directamente.

## Semántica de fallo entre flujos

Cada fachada sigue una única regla de orden: el cambio de estado durable se
confirma primero, y luego se disparan los efectos secundarios de notificación.
Un pánico de un oyente, un fallo transitorio del transporte de correo, o un
error del despachador después de la mutación no pueden revertir la mutación.

- `EmailVerification::verify` exige al propietario autenticado del token,
  consume el token y marca al usuario como verificado antes de disparar
  `EmailVerified`.
- `PasswordReset::complete` confirma primero la transacción de
  restablecimiento de contraseña de Magnetar. La transacción consume el token,
  aplica la política de primera prueba o de cuenta verificada, avanza la época
  de autenticación y revoca sesiones y credenciales remember. El correo y los
  eventos del framework se ejecutan después.
- `BruteForce::unlock_account` confirma el desbloqueo antes de disparar
  `AccountUnlocked`.
- `TwoFactor::confirm` estampa `confirmed_at` antes de disparar
  `TwoFactorEnrolled`; `TwoFactor::disable` elimina la fila antes de disparar
  `TwoFactorDisabled`; `TwoFactor::complete_challenge` promueve pendiente →
  autenticado antes de despachar el par estándar `auth::Login` +
  `auth::Authenticated`, seguido de `TwoFactorChallenged`.

Un oyente que necesite durabilidad debería almacenar en búfer su trabajo
(encolar un job desde el cuerpo del oyente); la fachada misma nunca reintenta.

## Arranque

Inicializa Magnetar después de `DB::init` y de que `APP_KEY` haya
inicializado `Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` crea el esquema de auth predeterminado, a menos que las
migraciones estén deshabilitadas, y luego instala atómicamente los adaptadores
de contraseña/sesión y passkey. Llamarlo una segunda vez devuelve un error.
Los tests que necesiten una instalación global al proceso deberían usar un
binario dedicado de tests de integración, porque un motor instalado no se
puede reemplazar.

### Verificación de correo

La verificación de correo requiere:

1. Un `UserProvider` registrado que pueda recuperar usuarios por email y
   marcar la marca de tiempo de verificación.
2. `MustVerifyEmail` en el tipo de usuario de la aplicación.
3. Una columna `email_verified_at` anulable.
4. La tabla `auth_flow_tokens` del framework.

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

El handler de verificación debe ejecutarse dentro del alcance de una sesión
autenticada. Un token válido de otro usuario se rechaza sin consumirse.

### Restablecimiento de contraseña y bloqueo

El restablecimiento de contraseña y `BruteForce` requieren el motor de
contraseña Magnetar instalado. `MagnetarConfig::lockout_config` acepta
`magnetar::password::lockout::LockoutConfig`. La política predeterminada
habilita el bloqueo después de cinco intentos fallidos durante 15 minutos,
conserva las filas de auditoría durante siete días y falla cerrado cuando el
backend de bloqueo no está disponible.

El restablecimiento de contraseña aplica antienumeración al enviar. La
finalización usa el almacén atómico de primera prueba de email y devuelve un
`PasswordResetOutcome` a los llamadores que necesitan conocer explícitamente
el estado de revocación de la sesión o de remember.

### Registrar las migraciones de 2FA

El framework incluye el esquema; la aplicación opta por él listando ambas
migraciones en su migrador:

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

Ambas son idempotentes frente a una base de datos donde ya se aplicaron (la
v1 usa `CREATE TABLE IF NOT EXISTS`; la v2 añade una columna). Volver a
ejecutar `suprnova migrate` sobre una base de datos de producción que ya tiene
el esquema no hace nada.


### Entorno

Los mailables transaccionales leen dos variables de entorno al enviar:

| Var | Por defecto | Para qué se usa |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Branding del asunto y etiqueta del emisor `otpauth://` que muestran las aplicaciones autenticadoras. |
| `MAIL_FROM` | ninguno - **da error si no se establece** | `From` del envelope de cada mensaje saliente. Establécelo en un dominio de remitente verificado. |

`MAIL_FROM` deliberadamente no tiene valor por defecto. Usar un placeholder
como `noreply@example.com` rompería silenciosamente DMARC / SPF en producción
y enviaría desde un dominio que el operador no controla, por lo que la fachada
falla cerrado. `EmailVerification::send_link` y
`PasswordReset::send_link` exponen el error como `Err`;
`PasswordReset::complete` registra mediante `tracing::warn!` y continúa
(el cambio de contraseña ya se confirmó, por lo que la ruta de notificación
no puede revertirlo).

Las aplicaciones también establecen `APP_URL` para que los controladores puedan
derivar la URL base usada en las llamadas a `send_link`; la fachada del
framework recibe la URL base como parámetro.

El driver de correo se configura por separado mediante `MAIL_DRIVER`; consulta
la documentación de [Correo](mail.md).

## Verificación de correo

`EmailVerification` acuña, comprueba y consume tokens de verificación
contra la tabla `auth_flow_tokens`, y marca al usuario como verificado a
través del proveedor configurado. Cuatro operaciones cubren el ciclo de vida:

| Método | Firma | Notas |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Acuña + envía correo, dado un usuario que ya tienes en mano. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Normaliza el resultado desconocido del proveedor a `Ok(())`; los fallos de almacenamiento del token y de entrega del correo siguen devolviendo `Err`, y el tiempo de ejecución no se iguala. |
| `check` | `check(token: &str) -> Result<bool>` | No consume - seguro de llamar en una landing page. |
| `verify` | `verify(token: &str) -> Result<String>` | Ligado al actor y de un solo uso: el usuario autenticado debe ser dueño del token; el éxito lo consume, marca al usuario como verificado y devuelve ese id de usuario. |

```rust
use suprnova::auth_flows::EmailVerification;

// Tras un registro reciente, con el usuario recién creado disponible:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Comprobación opcional en la página de destino; no consume el token, por lo que recargar la página no lo invalida.
let valid: bool = EmailVerification::check(&token_str).await?;

// El handler del enlace se ejecuta detrás de la autenticación. `verify` consume el token solo cuando `Auth::id()` coincide con su propietario.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` dispara `EmailVerified` en caso de éxito - los oyentes son el lugar
correcto para desbloquear funcionalidad adicional (correo de bienvenida,
seguimientos por defecto, un CTA de «completa tu perfil») sin acoplarlos al
handler de verificación. El evento lleva el id de usuario del proveedor.

### El endpoint resend (anti-enumeración)

`resend` toma solo el email y busca al usuario mediante el proveedor activo.
Un resultado desconocido del proveedor se normaliza a `Ok(())`. Para una cuenta
conocida, la fachada acuña un token y envía el correo.
`EmailVerification::resend` también normaliza a `Ok(())` el resultado
desconocido del proveedor; no garantiza tiempos idénticos ni un comportamiento
idéntico cuando fallan el almacenamiento del token o la entrega del correo. Un
handler aún puede devolver un mensaje neutral después de cualquiera de los dos
resultados exitosos:

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
    // `resend` realiza internamente la búsqueda y la anti-enumeración.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "Si este correo está registrado, se ha enviado un enlace de verificación."
    ))
}
```

`send_link` y `resend` construyen ambos la URL como
`{base_url}?token={plaintext_token}`. Una barra final en `base_url` se
recorta antes de añadir la query string, así que
`https://app.example.com/verify/` y `https://app.example.com/verify`
producen ambas una URL limpia.

El handler del enlace debe ejecutarse detrás de `AuthMiddleware`. Extrae el
token de la query string y llama a `verify`:

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

`verify` comprueba `Auth::id()` contra el propietario del token antes de
consumirlo. Un token que pertenece a otra cuenta devuelve la misma respuesta
de token inválido y permanece sin usar. En caso de éxito, el proveedor marca
al propietario autenticado como verificado y la fachada dispara
`EmailVerified`.

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

// Superficie de API: 403 con un cuerpo JSON.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Superficie web: 302 (o 409 + X-Inertia-Location para visitas de Inertia).
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
    // ramifica según este valor
}
```

## Restablecimiento de contraseña

`PasswordReset` tiene cuatro operaciones:

| Método | Firma | Notas |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Devuelve `Ok(())` para una dirección desconocida solo después de que tienen éxito las comprobaciones del limitador de abuso, la configuración de correo, el motor y el almacenamiento; otros fallos siguen devolviendo `Err`. |
| `check` | `check(token: &str) -> Result<bool>` | Validación sin consumo mediante el motor Magnetar instalado. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Consume el token de forma atómica, aplica la política de primera prueba, rota las credenciales, revoca las sesiones y el estado de remember, y devuelve el ID de usuario. |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | Ejecuta la misma transacción y devuelve los recuentos de revocación confirmados. |

```rust
use suprnova::auth_flows::PasswordReset;

// Desde el formulario «olvidé mi contraseña». Siempre devuelve Ok(()): la fachada busca al usuario y solo envía cuando existe una cuenta.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Comprobación opcional en la página de destino antes de renderizar el formulario de contraseña nueva.
let valid: bool = PasswordReset::check(&token).await?;

// El handler del enlace, después de que el usuario envía una contraseña nueva: consume el token y rota la contraseña, devolviendo el ID de usuario.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` pasa la contraseña en texto plano mediante `SecretString`; Magnetar
la hashea dentro del motor de credenciales. No la hashees antes. Una contraseña
vacía o formada solo por espacios en blanco devuelve HTTP 400 antes de llamar
al motor.

### Anti-enumeración

`send_link` devuelve `Ok(())` para una dirección desconocida solo después de
que tienen éxito las comprobaciones del limitador de abuso, la configuración
de correo, el motor y el almacenamiento. Los fallos de configuración,
limitador, almacenamiento y correo siguen devolviendo `Err`. El controlador
dogfood da a las solicitudes de cuentas conocidas y desconocidas correctas el
mismo estado y cuerpo HTTP, pero la implementación no iguala su tiempo de
ejecución.

### Efectos secundarios de `complete`

Magnetar confirma el restablecimiento de contraseña en una sola transacción:

1. Consume el token de restablecimiento de un solo uso.
2. Aplica la política de primera prueba de correo cuando la cuenta sigue sin
   verificar.
3. Hashea y reemplaza la contraseña.
4. Avanza la época de autenticación.
5. Revoca las sesiones opacas antiguas y las credenciales de remember.
6. Elimina las credenciales provisionales cuando este restablecimiento es la
   primera prueba de buzón de correo de la cuenta.

Tras la confirmación, el framework envía `PasswordChangedMail` y despacha
`PasswordResetCompleted`. Un fallo del correo o de un oyente no puede
revertir el restablecimiento.

En una cuenta ya verificada, el restablecimiento conserva las passkeys
legítimas, las cuentas vinculadas y la inscripción de dos factores confirmada.
En una cuenta no verificada ocupada de forma abusiva, la primera prueba elimina
las credenciales provisionales para que el registrante anterior no pueda
conservar el acceso.

## Protección contra fuerza bruta

La capa de fuerza bruta tiene dos partes: la fachada `BruteForce`, que registra
y consulta el estado de bloqueo, y `LoginThrottleMiddleware`, que corta en la
capa HTTP antes de que se invoque el handler.

### La fachada `BruteForce`

Llama a `record_failed_attempt` desde la rama de autenticación fallida del
handler de login, y a `reset_attempts` desde la rama de éxito:

```rust
use suprnova::auth_flows::BruteForce;

// En la ruta de autenticación fallida:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Opcionalmente devuelve una respuesta personalizada. El middleware lo hará por ti en la solicitud siguiente; consulta más abajo.
}

// En la ruta de éxito:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` devuelve el `LockoutStatus` actualizado (`is_locked`,
`failed_attempts` y `locked_until` cuando está bloqueada). Pasa el `ip`
opcional para los registros de auditoría; pasa `None` si tu transporte no
expone limpiamente una IP de cliente.

Dos operaciones adicionales:

```rust
// Solo lectura: seguro para correos sin historial.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Desbloqueo administrativo/forzado. Dispara `AccountUnlocked` solo ante una transición real de estado; desbloquear una cuenta ya desbloqueada no dispara el evento.
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` devuelve `true` cuando la cuenta estaba bloqueada en el
momento de la llamada, y `false` en caso contrario. El evento
`AccountUnlocked` se dispara solo con `true`: un retorno `false` es la
operación sin efecto que indica, no un evento de auditoría.

### `LoginThrottleMiddleware`

El middleware lee el estado de bloqueo del correo que una solicitud tenga como
objetivo y corta con `429 Too Many Requests` cuando la cuenta está bloqueada.
El handler de login nunca se invoca, por lo que una cuenta bloqueada ni siquiera
llega a intentar una comprobación de credenciales:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// El extractor de correo es un closure síncrono sobre `&Request`. Leer un cuerpo JSON/de formulario es asíncrono y consume `Request`, por lo que el closure no puede leer el cuerpo: extráelo de un encabezado, la query string o un parámetro de ruta.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Superficies prácticas de extracción:

- Una cabecera (`X-Login-Email`), fijada por un preprocesador anterior: el
  patrón usado en la aplicación dogfood.
- Un parámetro de cadena de consulta (`?email=…`).
- Un parámetro de ruta (`/login/{email}`).

Devolver `None` del extractor es la señal explícita de «no tengo nada que
comprobar»: el middleware deja pasar la solicitud sin cambios. Esto hace que el
middleware se pueda instalar con seguridad en rutas que a veces reciben tráfico
anónimo (por ejemplo, el mismo endpoint `POST /login` que también gestiona una
subacción «solicitar restablecimiento de contraseña» sin correo).

Al bloquear, el middleware devuelve:

- Estado `429 Too Many Requests`.
- Cabecera `Retry-After`: segundos calculados desde el `locked_until` del
  bloqueo mediante `LockoutStatus::retry_after_seconds`. Recurre a `900` (15
  minutos, el período de bloqueo predeterminado de Magnetar) si de algún modo
  falta la marca de tiempo.
- Cuerpo: `"Account locked due to too many failed login attempts. Try
  again later."`

### Errores de backend (falla cerrada de forma predeterminada)

Si `get_lockout_status` devuelve un error, `LoginThrottleMiddleware` registra
el fallo y, de forma predeterminada, devuelve HTTP `503 Service Unavailable`
con `Retry-After: 1` sin invocar al handler de login. Para mantener disponible
el login durante una interrupción del backend de bloqueo, opta explícitamente
por `.on_backend_error(BackendErrorPolicy::FailOpen)`; solo esa política pasa
la solicitud al handler.

### Capas con `RateLimitMiddleware`

`LoginThrottleMiddleware` es por cuenta: bloquea un solo correo cuando se cruza
el umbral. Para cuotas por IP, combínalo con
[`RateLimitMiddleware`](rate-limiting.md). Ambos se componen de forma natural:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Juntos cubren las formas realistas de credential stuffing: la distribuida (un
correo × muchas IP) es tarea del límite de velocidad; la focalizada (muchos
intentos × un correo) es tarea del middleware de limitación.

### Configuración

`MagnetarConfig` acepta un `LockoutConfig`. El valor predeterminado son cinco
intentos fallidos, un período de conteo y bloqueo de 15 minutos, siete días de
retención de intentos y `BackendErrorPolicy::FailClosed`:

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

Usa `LockoutConfig::disabled()` solo cuando otro control de identidad con fallo
cerrado sustituya el bloqueo de cuentas.

## Dos factores (TOTP)

`TwoFactor` cubre el 2FA basado en TOTP: el tipo que se empareja con cualquier
aplicación de autenticación compatible con estándares (Google Authenticator,
1Password, Bitwarden, Authy). El flujo es inscripción → confirmación →
verificación continua, además de códigos de recuperación de un solo uso para
cuando el usuario pierde su dispositivo, y el flujo de desafío que integra todo
en el ciclo de vida del login.

### El trait `TwoFactorUser`

El framework no puede acceder al almacenamiento de usuarios de tu aplicación,
así que quienes llaman implementan un pequeño trait para tender un puente entre
su modelo de usuario y la fachada de 2FA:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` es una clave de almacenamiento opaca. Puede ser un ID numérico de la
aplicación convertido a texto, un UUID o un `UserId` de Magnetar. La tabla TOTP
del framework no tiene clave foránea hacia la tabla de usuarios de la
aplicación.

`email` se incorpora al segmento `account_name` de la URL `otpauth://`, para
que la aplicación de autenticación muestre una etiqueta de cuenta reconocible.

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### Almacenamiento

El estado de 2FA vive en la tabla `two_factor_credentials`, propiedad del
framework. Los secretos y los códigos de recuperación se cifran en reposo con
`crate::crypto::Crypt::encrypt_string`, lo que requiere una `EncryptionKey`
global para el proceso. Las aplicaciones optan por el esquema al listar ambas
migraciones en su `Migrator::migrations()`: consulta
[Arranque](#arranque).

### Inscribir, confirmar, verificar

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Inscripción: genera un secreto nuevo y 10 códigos de recuperación, los persiste cifrados y devuelve todo lo necesario para renderizar el código QR.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url: enlace profundo `otpauth://totp/...`
// response.qr_code_svg: <svg> que envuelve un PNG en base64; incrústalo en línea
// response.recovery_codes: Vec<String>, 10 códigos en texto plano; muéstralos UNA SOLA VEZ

// 2. Confirmación: el usuario abre la app de autenticación e introduce el código de 6 dígitos. `confirm` lo valida y marca `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// dispara `TwoFactorEnrolled`

// 3. En los inicios de sesión posteriores, protege la sesión con `verify`:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` devuelve códigos de recuperación en texto plano **exactamente una
vez**. No hay API para recuperarlos más tarde: la columna cifrada es
unidireccional desde este punto. Muéstralos en la página de éxito de la
inscripción, anima al usuario a guardarlos y no almacenes el texto plano en
ningún otro lugar.

`enroll` se niega a sobrescribir una inscripción **confirmada**: devuelve un
`409` para dirigir a quien llama hacia `re_enroll`, que requiere prueba de
posesión. Se permite reinscribirse sobre una fila no confirmada (pendiente): la
inscripción anterior nunca llegó a ser autoritativa.

### Protección contra repetición

`verify` escribe el paso de tiempo TOTP actual en `last_used_timestep` cuando
tiene éxito. Las verificaciones posteriores donde `current_timestep <=
last_used_timestep` se rechazan incluso cuando el código es estructuralmente
válido, con lo que se impide la repetición de un código robado dentro de la
ventana de 30 segundos.

La reclamación del paso de tiempo es atómica. La marca se aplica mediante un
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep <
:current` condicional, y la verificación solo tiene éxito cuando la instrucción
afecta exactamente a una fila. Dos verificaciones concurrentes en el mismo paso
de tiempo no pueden ganar ambas: la primera cambia la columna, el predicado de
la segunda deja de coincidir y la segunda se trata como una repetición. Un
simple read-modify-write sería una carrera TOCTOU: ambas verificaciones leerían
la fila antes de marcarla, ambas validarían el mismo código, ambas la marcarían
y ambas tendrían éxito. Los competidores concurrentes también se cuentan como
intentos fallidos, para que el contador de fuerza bruta los registre.

### Códigos de recuperación

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

De un solo uso: un código coincidente se elimina de la fila antes de que la
llamada retorne, por lo que un segundo intento contra el mismo código devuelve
`false`. Los códigos tienen 12 dígitos decimales con el formato
`NNNNNN-NNNNNN` (~40 bits de entropía cada uno, igual que el formato de Laravel
Fortify).

`consume_recovery_code` solo acepta códigos cuando el 2FA está completamente
confirmado: retorna pronto `Ok(false)` mientras `confirmed_at` es NULL. Sin
esta compuerta, un atacante que hubiera iniciado una inscripción en la cuenta de
una víctima (o cualquier flujo que cree la fila sin confirmarla) podría
autenticarse solo con un código de recuperación nuevo y eludir por completo
TOTP. El contrato es simétrico con la protección de «solo inscripción
confirmada» de `verify`.

### Rotación de códigos de recuperación y secretos

Cuando un usuario agota sus códigos de recuperación o quiere rotarlos tras
sospechar un compromiso:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` debe validar como un código TOTP actual o un código de recuperación sin
usar. Sin la comprobación de prueba, un atacante que hubiese secuestrado una
sesión podría eliminar silenciosamente los códigos de recuperación del usuario
legítimo (una denegación de servicio contra la recuperación de la cuenta). Los
códigos nuevos sustituyen el conjunto persistido; el secreto existente y
`confirmed_at` se conservan, de modo que la aplicación de autenticación del
usuario sigue funcionando sin volver a emparejarse. Errores:

- `400`: no existe ninguna inscripción confirmada; llama primero a
  `enroll`/`confirm`.
- `401`: `proof` no valida ni como código TOTP ni como código de recuperación
  sin usar.
- `429`: la cuenta está bloqueada por la limitación de fuerza bruta.

Para rotar el **secreto** (volver a emparejar con un dispositivo nuevo) sin
desactivar primero el 2FA:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

El modelo de prueba es el mismo que el de `regenerate_recovery_codes`. La fila
se reescribe con un secreto nuevo y 10 códigos de recuperación nuevos;
`confirmed_at` se restablece a NULL, por lo que el usuario debe ejecutar
`confirm` con un código de la nueva aplicación de autenticación antes de que el
2FA vuelva a estar activo.

### Desactivar

```rust
TwoFactor::disable(&user_2fa).await?;
// dispara `TwoFactorDisabled` solo si se eliminó una fila
```

Es idempotente: desactivar 2FA para un usuario que nunca se inscribió no es un
error. El evento `TwoFactorDisabled` se dispara solo ante una transición de
estado real, por lo que los oyentes de auditoría ven una entrada por cada
desactivación real en vez de una por cada clic en un botón sin efecto.

### Flujo de desafío (bloquear el login con el segundo factor)

Las primitivas de inscripción / confirmación / verificación son los bloques de
construcción; el **flujo de desafío** las integra en el ciclo de vida del login
para que un usuario con 2FA habilitado no pueda alcanzar páginas protegidas solo
con la contraseña.

El flujo:

1. El login con contraseña resuelve un usuario.
2. Si `TwoFactor::is_enabled_by_id(&user_id)` devuelve `true`, el handler de
   login llama a `TwoFactor::start_challenge(user_id, remember)`: esto guarda el
   ID de usuario como **pendiente** en la sesión, borra el espacio totalmente
   autenticado, revoca cualquier cookie de remember-me emitida por
   `Auth::attempt` y recuerda si el usuario optó por remember-me para que la
   cookie pueda volver a emitirse después de completar el desafío. `Auth::id()`
   devuelve `None` desde este punto hasta que termina el desafío.
3. El handler redirige a una ruta `/two-factor-challenge` que muestra el
   formulario de código.
4. El handler POST del desafío llama a `TwoFactor::complete_challenge(code)`:
   verifica el código (TOTP **o** un código de recuperación sin usar, igual que
   el controlador de desafío de Fortify), promueve pendiente → autenticado,
   rota el ID de sesión (lo que impide la fijación de sesión) y el token CSRF,
   vuelve a emitir la cookie de remember-me cuando el usuario optó por ella y
   despacha los eventos estándar del ciclo de vida `auth::Login` +
   `auth::Authenticated`, además de `TwoFactorChallenged`, específico de 2FA.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Rebaja a «pendiente»: se borra el slot autenticado, se establece el pendiente y se revoca la cookie remember-me. Propaga el flag remember del formulario para que `complete_challenge` pueda volver a emitir la cookie tras el éxito.
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
    // El ID de sesión y CSRF se han rotado; remember-me se ha vuelto a emitir si el formulario de login original lo solicitó. Los listeners de `auth::Login` / `auth::Authenticated` observaron un login normal.
    redirect!("/dashboard").into()
}
```

`complete_challenge` rota el ID de sesión y el token CSRF como parte de la
promoción a autenticado. Con ello se cierra el ataque clásico de fijación de
sesión, en el que un atacante introduce un ID de sesión conocido en una víctima
antes de que esta inicie sesión: tras la rotación, el ID introducido queda
inutilizado y solo el ID recién generado transporta el estado autenticado. El
contrato coincide con `Auth::login_id` / `Auth::login_using_id`, por lo que los
inicios de sesión con 2FA son indistinguibles de los inicios sin 2FA en términos
de estado de sesión y observabilidad de oyentes.

Protege cada grupo de rutas con `TwoFactorChallengeMiddleware` **antes de**
`AuthMiddleware`, para que una sesión pendiente rebote hacia la página del
desafío en vez de hacia la página de login:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

La propia página de desafío (el GET que renderiza el formulario y el POST que
llama a `complete_challenge`) NO debe instalar `TwoFactorChallengeMiddleware`:
es el destino. El handler POST también suele comprobar por adelantado
`TwoFactor::pending_user_id().is_some()` para que un enlace obsoleto no alcance
la lógica de verificación con una sesión vacía.

`TwoFactor::cancel_challenge()` borra ambos espacios pendientes sin autenticar a
nadie: conéctalo a un enlace «volver al login» en la página de desafío.

**Alternativa con código de recuperación.** `complete_challenge(code)` intenta
primero la ruta TOTP y recurre a consumir un código de recuperación, de modo que
un usuario que haya perdido su autenticador todavía pueda entrar. Cada código
de recuperación es de un solo uso.

**Vinculación con fuerza bruta.** Los códigos de desafío fallidos alimentan el
contador de fuerza bruta por cuenta mediante `BruteForce::record_failed_attempt`
del mismo modo que `TwoFactor::verify` sin más. Un atacante que pruebe
repetidamente el formulario de desafío activará `AccountLocked` después del
umbral configurado. Una sola entrega incorrecta cuenta como **un** intento
fallido aunque `complete_challenge` intente internamente las rutas de TOTP y de
código de recuperación: los núcleos de validación silenciosa omiten el contador
de fuerza bruta para que la capa exterior registre el intento canónico
exactamente una vez.

**Compuerta de bloqueo.** `complete_challenge` comprueba por adelantado
`BruteForce::is_locked` y devuelve `429 Too Many Requests` si la cuenta ya está
bloqueada, incluso cuando el código enviado es correcto. Sin esta compuerta
dentro del método, un atacante que activase el bloqueo todavía podría entrar
enviando el código correcto en la siguiente solicitud: el contador de fuerza
bruta usa como clave el correo del usuario, pero `verify` no lo consulta. El
`LoginThrottleMiddleware` de la ruta de contraseña impone la misma restricción
en la capa de ruta; componerlo delante de la ruta POST del desafío es correcto:
ambas compuertas son idempotentes.

**Evento de fallo.** `complete_challenge` despacha
`TwoFactorChallengeFailed { user_id }` ante un código incorrecto (o una cuenta
bloqueada), distinto de `auth::Failed` de la ruta de contraseña. Los oyentes
que vigilan «el usuario intentó 2FA y falló» se suscriben al nuevo evento; los
que vigilan «la contraseña no autenticó» se mantienen en `auth::Failed`. Ambas
superficies se mantienen separadas para que un error al escribir 2FA no parezca
un fallo de contraseña ante las canalizaciones de auditoría.

### Por qué Suprnova diverge

El `user_id` TOTP del framework es un `String`. Un tipo fijo `i64`, UUID o de
identificador de Magnetar vincularía la fachada reutilizable a un esquema de
aplicación. El límite de cadena permite a una aplicación elegir cualquier
identificador estable a cambio de una conversión en el sitio de llamada.

La compuerta de factores integrada de Magnetar está separada de esta fachada
conservada. La separación mantiene la compatibilidad con aplicaciones que usan
`two_factor_credentials`, pero las aplicaciones no deben inscribir la misma
cuenta mediante ambos almacenes.

## Recuérdame

`suprnova::auth_flows::remember_me` reexporta el módulo legado
`suprnova::auth::remember` por compatibilidad.

Cuando Magnetar está instalado, los flujos normales de `Auth::attempt(...,
true)`, `Auth::issue_remember_cookie` y la hidratación de
`SessionMiddleware` usan las credenciales de remember de Magnetar vinculadas a
un propósito. Magnetar almacena resúmenes de verificadores, comprueba la época
de autenticación, rota las credenciales al usarlas correctamente, las revoca
con la sesión del usuario e informa anomalías de repetición o de credenciales
malformadas sin exponer el secreto.

La cookie que mira al navegador sigue siendo propiedad del framework. Está
cifrada con el nombre lógico `remember_me`, sigue `SESSION_COOKIE_PREFIX` y se
borra antes de la revocación en el backend, para que un fallo de almacenamiento
no deje al navegador enviando la credencial antigua.

La implementación de fila de base de datos legada permanece disponible cuando
no hay un motor de Magnetar instalado. Las nuevas aplicaciones deben inicializar
Magnetar y tratar la reexportación legada como una superficie de transición.

## Eventos

Nueve eventos se disparan entre los flujos, uno por cada transición de estado de
seguridad:

| Evento | Disparado por | Contiene |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` en caso de éxito | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` en caso de éxito; silencioso ante correos ausentes por anti-enumeración | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` en caso de éxito | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` en la transición desbloqueada → bloqueada | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` cuando ocurrió un desbloqueo real | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` en caso de éxito | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` promovió pendiente → autenticado | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` rechazó un código incorrecto o rehusó una cuenta bloqueada | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` cuando se eliminó realmente una fila | `user_id: String` |

Todos los eventos son `Debug + Clone + 'static`, no contienen datos sensibles
(ni tokens en texto plano ni IP) y usan identificadores de tipo cadena, para que
los oyentes puedan serializarlos a través de límites de tareas sin filtrar
información de tipo del backend de almacenamiento de usuarios.

### Escuchar

Suscríbete mediante la API de eventos estándar: la misma superficie que para
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
        // ... notificación de Slack, agregar a la tabla de auditoría, etc.
        Ok(())
    }
}

// En bootstrap.rs:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Los oyentes se ejecutan en el runtime de Tokio y se despachan en el orden de
registro. Consulta el capítulo [Eventos](events.md) para ver la superficie
completa.

## Pruebas

Tres fakes cubren la superficie de auth-flows y se componen.

### `Mail::fake()`

Instala un transporte de captura local al proceso. Cada envío durante la vida
del guard acaba en un búfer en memoria en lugar de salir:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... ejecuta el flujo ...
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

`MailFake` expone `assert_sent`, `assert_not_sent`, `assert_sent_count`, además
de los accesores sin procesar `captured()` y `count()`. Cuando el guard se
descarta, se restaura el transporte que estaba vinculado previamente: las
pruebas que intercalan fakes con vinculación explícita de transporte no filtran
estado.

### `EventFacade::fake()`

La misma forma, pero para los eventos:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... ejecuta el flujo ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

El fake registra los eventos despachados sin invocar oyentes, por lo que un
oyente que se comunique con un servicio externo no se disparará durante la
prueba. El `assert_not_dispatched::<E>(pred)` complementario afirma el
negativo; `dispatched_count::<E>(pred)` devuelve el recuento sin procesar para
aserciones más granulares.

### Pruebas de integración para verificación de correo y restablecimiento de contraseña

Las pruebas de verificación de correo crean `auth_flow_tokens`, registran un
`UserProvider`, establecen el propietario de token autenticado, definen
`MAIL_FROM` y conducen la fachada bajo `Mail::fake()`.

Las pruebas de restablecimiento de contraseña instalan un adaptador de prueba
`MagnetarPasswordAuthEngine` y verifican emisión, comprobación sin consumo,
finalización atómica, revocación de sesión y comportamiento de un solo uso.

Los ejemplos fuente canónicos son:

- `framework/tests/email_verify.rs` para la verificación vinculada al actor y
  los tokens de un solo uso.
- `framework/tests/password_reset.rs` para la delegación de Magnetar y los
  resultados de finalización.
- `framework/tests/magnetar_default_engine.rs` para la configuración de un
  motor predeterminado real.
- `framework/tests/brute_force.rs` para el ciclo de vida de bloqueo.
- `framework/tests/two_factor_challenge_flow.rs` para el flujo de desafío TOTP
  del framework conservado.
- `framework/tests/magnetar_remember_middleware.rs` para la rotación de
  remember y la vinculación de sesión dual.

La instalación de Magnetar global al proceso es intencionadamente de una sola
vez. Coloca las pruebas que necesiten motores distintos en binarios de pruebas
de integración separados, o instala un adaptador de prueba una vez para todo el
binario.


## Referencia

| Símbolo | Propósito |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check` y `verify` vinculado al actor; `verify` devuelve el ID de usuario. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` para JSON 403 y `redirect_to(path)` para redirecciones de navegador o Inertia. |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` y `complete_with_outcome` respaldados por Magnetar. |
| `suprnova::MustVerifyEmail` | Contrato de usuario de aplicación para la fachada de verificación del framework. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | Definición de tabla SeaORM para tokens de verificación del framework. |
| `suprnova::auth_flows::BruteForce` | Fachada de bloqueo de cuentas respaldada por Magnetar. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP que devuelve 429 antes del handler de login cuando la cuenta está bloqueada. |
| `suprnova::auth_flows::TwoFactor` | Fachada TOTP conservada del framework para inscripción, verificación, recuperación y desafío. |
| `suprnova::auth_flows::TwoFactorUser` | Puente de usuario de aplicación para la fachada TOTP del framework. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | Compuerta para sesiones que esperan el desafío TOTP del framework. |
| `suprnova::auth_flows::remember_me` | Reexportación de compatibilidad del módulo remember legado del framework. |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | Configuración predeterminada del motor Magnetar e instalación de una sola vez. |
| `suprnova::auth_flows::events::*` | Eventos del ciclo de vida de autenticación. |

## Siguiente

- [Autenticación](authentication.md): guards, proveedores, la fachada `Auth` y
  `AuthMiddleware`.
- [Correo](mail.md): la capa de transporte mediante la que se despachan las
  llamadas a `send_link`.
- [Eventos](events.md): registrar oyentes para los nueve eventos de flujo de
  autenticación.
- [Limitación de velocidad](rate-limiting.md): combina
  `RateLimitMiddleware::ip_based` con `LoginThrottleMiddleware` para una
  defensa en capas.
- [Sesiones](session.md): lo que `start_challenge` / `complete_challenge` tocan
  cuando rotan el ID de sesión.
