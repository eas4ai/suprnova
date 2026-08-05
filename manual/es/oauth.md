# OAuth, inicio de sesión con Apple e inicio mágico por enlace

Suprnova ofrece tres métodos de login respaldados por torii detrás de la
fachada `Auth`: **OAuth genérico** (GitHub, Google, o cualquier proveedor
OIDC/OAuth2), **inicio de sesión con Apple**, y **enlaces mágicos sin
contraseña**. Comparten un único prerrequisito (`init_torii` más la
migración de la ceremonia) y la misma forma de fachada -
`Auth::oauth(provider)` / `Auth::magic_link()` - y ninguno de ellos
incluye rutas: añades un controlador delgado (inicio + callback) y el
framework se encarga del `state` CSRF, PKCE, el intercambio de token, la
verificación de identidad, el upsert del usuario, y la acuñación de la
sesión.

Toda la superficie vive en `framework/src/torii_integration/`. **No**
existe ningún contrato de variables de entorno del framework para nada
de esto - cada credencial se pasa de forma programática (obtén las
tuyas desde el entorno); los ejemplos de este capítulo usan
`std::env::var(...)` solo para mostrar dónde van tus secretos.

## Prerrequisitos

1. **Inicializa torii una vez en el arranque** - esto respalda el
   upsert de usuario y la creación de la sesión:

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // en bootstrap::register(), después de DB::init()
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **Ejecuta la migración de la ceremonia.** OAuth y Apple guardan una
   ceremonia de corta duración (10 minutos) de `state` CSRF + PKCE en la
   tabla `auth_ceremony_tokens`. Registra la migración
   `m20251209_000000_create_auth_ceremony_tokens_table` en tu `Migrator`
   (los starter kits ya la incluyen). Opcionalmente, programa
   `suprnova::torii_integration::ceremony::prune_expired()` para
   recolectar las filas caducadas.

3. **`SessionMiddleware` en la ruta de *inicio* de OAuth.** `begin()`
   escribe el `state` en la sesión; una llamada sin sesión falla con un
   500.

Los enlaces mágicos solo necesitan el paso 1.

## OAuth genérico (GitHub, Google, personalizado)

### Configurar un proveedor

Registra cada proveedor una vez al arrancar. El registro es global al
proceso e idempotente, así que volver a registrar el mismo proveedor
simplemente reemplaza la configuración:

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → la tabla well-known incorporada
    apple_key_pair: None,       // Solo Apple; déjalo en None para GitHub/Google
    apple_team_id: None,        // Solo Apple
});
```

Los endpoints well-known de authorize/token/userinfo vienen incorporados
para `github`, `google`, y `apple`. Para cualquier otro proveedor - o un
servidor autohospedado / de pruebas - provéelos tú mismo:

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // alternativa /emails al estilo GitHub para un correo primario privado
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### Iniciar el flujo (URL de autorización)

```rust
// GET /auth/oauth/github/start  (la ruta DEBE llevar SessionMiddleware)
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - redirige el navegador aquí
// kickoff.state - state CSRF, ya guardado en la sesión por ti
```

`begin()` acuña el `state` CSRF (UUID v4) y un verificador/challenge
S256 de PKCE según RFC 7636, registra la ceremonia (TTL de 10 minutos),
y devuelve la URL de autorización del proveedor. Redirige al usuario a
`authorization_url`.

### Completar el flujo - `verify` frente a `complete`

En el callback tienes dos puntos de entrada (separados en la 0.5.4).
Elige según si tu tabla `users` **es** el esquema de torii:

| Método | Devuelve | Efectos secundarios | Úsalo cuando |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **Ninguno** - verifica la ceremonia, intercambia el code, obtiene el userinfo, y extrae un email verificado + un `subject` estable. Sin usuario, sin sesión. | Tu app posee su propia tabla `users` y quieres buscar / crear el usuario tú mismo. |
| `complete(code, state)` | `(User, Session)` | Hace upsert del usuario en torii (`get_or_create_user`) y acuña una sesión. | Tu tabla `users` es el esquema de torii. |

```rust
// Tabla de usuarios personalizada:
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject es el id estable del proveedor; id.email está verificado o es None.
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// …o, respaldado por torii:
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

Un `email` devuelto por `verify` siempre es una dirección *verificada*
(el `email_verified` de OIDC, GitHub tratado como verificado, o la
alternativa `/emails`); un email no verificado o ausente vuelve como
`None`, y los logins repetidos se resuelven por `subject`.

### Rutas que añades

El framework no provee rutas de OAuth - cablea dos handlers delgados
(reflejando la forma de los controladores `auth_verify` / `auth_reset`
ya existentes en el starter kit):

```rust
// inicio - redirige al proveedor
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// callback - GitHub/Google usan GET ?code&state
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

Pon la ruta `/start` (al menos) detrás de `SessionMiddleware`.

## Inicio de sesión con Apple

Apple usa la misma fachada - `Auth::oauth("apple")` - con unas cuantas
reglas específicas de Apple ya incorporadas:

- **El callback es un `POST`.** Apple usa `response_mode=form_post`,
  así que la redirección entrega `code` + `state` en un cuerpo de
  formulario, no en parámetros de query. Registra el callback de Apple
  como una ruta `post!` y lee los campos desde el formulario.
- **Sin PKCE.** Apple rechaza `code_challenge`, así que la URL de
  autorización lo omite (en su lugar, el client secret es un JWT
  firmado).
- **`client_secret` no se usa** - déjalo como `String::new()`.
  Suprnova acuña el client secret JWT de corta duración a partir de tu
  clave `.p8` en cada intercambio de token.
- **Los ID tokens se verifican contra el JWKS de Apple (RS256)** desde
  la 0.5.6, en lugar de confiarse de forma estructural.

### Provee tu clave de Apple - `AppleKeyPair`

`AppleKeyPair` es el único tipo de Apple re-exportado para las apps (así
que no necesitas una dependencia directa de `apple`). Constrúyelo a
partir de tu clave de firma `.p8`:

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // *Key ID* de Apple (no el Team ID)
    &std::env::var("APPLE_P8_PATH")?,  // ruta a AuthKey_XXXXXX.p8
)?;
// o: AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### Configurar Apple

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // tu Services ID
    client_secret: String::new(),                  // no se usa - se acuña a partir de la clave
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // Team ID de 10 caracteres
});
```

### Completar el flujo de Apple

Misma división que el OAuth genérico. `complete` hace upsert + sesiones;
la ruta de verify devuelve un `AppleIdentity` para una tabla de usuarios
personalizada:

```rust
// POST /auth/apple/callback - lee code + state desde el cuerpo del FORM
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// …o tabla de usuarios personalizada:
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id: AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` es `Some(_)` solo cuando Apple afirma que está
verificado; un email no verificado se rechaza (401) antes de construir
la identidad. `is_private_email` se activa cuando el usuario elige la
dirección de relay privado de Apple - persiste el `subject` como la
clave estable, ya que la dirección de relay es el único email que
obtendrás.

## Inicio mágico por enlace

Login por correo sin contraseña, respaldado por torii, vía
`Auth::magic_link()`. El framework emite y verifica el token; **tú**
envías el correo con el enlace (nunca envía correo por sí mismo), lo
cual se compone limpiamente con el capítulo de [Correo](mail.md).

```rust
use suprnova::Auth;

// POST /auth/magic - solicita un enlace
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// Construye el enlace y envíalo tú mismo por correo:
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - consúmelo (de un solo uso; una segunda llamada falla)
let (user, session) = Auth::magic_link().consume(&token).await?;
```

El usuario se autocrea en el primer uso. `send` devuelve el token en
**texto plano** para que tú controles la forma de la URL y la entrega.

> **Nota - `TokenPurpose::MagicLink`.** El enum `TokenPurpose` de
> `auth_flows` tiene una variante `MagicLink` (añadida en la 0.5.5), pero
> es un *discriminador reservado* para el `TokenStore` genérico - ningún
> flujo incorporado lo consume. La ruta de enlace mágico funcional y
> soportada es `Auth::magic_link()` de arriba. Solo recurre a
> `TokenPurpose::MagicLink` si estás construyendo a mano tu propio flujo
> sobre la tabla `auth_flow_tokens`.

## Una nota sobre la configuración

Ninguno de estos métodos lee variables de entorno del framework - los
IDs de proveedor, los secretos, las URLs de redirección y las claves de
Apple se pasan todos a `configure(...)` de forma programática. Cárgalos
como prefieras (`std::env::var`, un struct de configuración tipado, un
gestor de secretos) y registra los proveedores una vez durante el
`bootstrap`. Esto mantiene como ciudadanos de primera clase las
configuraciones de proveedor multi-tenant / por despliegue, en lugar de
forzar un esquema fijo de nombres de variables de entorno.

## Referencia

- Puntos de entrada de la fachada: `Auth::oauth(provider)`,
  `Auth::magic_link()` (`suprnova::Auth`)
- Configuración: `suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- Resultados de OAuth: `OAuthKickoff { authorization_url, state }`,
  `OAuthIdentity { provider, subject, email, name }`,
  `AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- Bootstrap: `suprnova::{init_torii, ToriiConfig}`
- Almacén de la ceremonia: tabla `auth_ceremony_tokens` +
  `suprnova::torii_integration::ceremony::prune_expired()`

## Siguiente

- [Autenticación](authentication.md) - guards, proveedores, y el modelo
  de usuario `Authenticatable` para el que estos flujos crean sesiones
- [Flujos de autenticación](auth-flows.md) - verificación de correo,
  restablecimiento de contraseña, y 2FA
- [Correo](mail.md) - el envío del correo de enlace mágico (y la
  configuración de remitente `MAIL_FROM` / `MAIL_FROM_NAME`)
- [Sesiones](session.md) - qué es el `Session` devuelto y cómo se
  persiste
