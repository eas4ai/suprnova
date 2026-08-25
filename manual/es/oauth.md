# OAuth, inicio de sesión con Apple e inicio mágico por enlace

Suprnova expone OAuth, Sign in with Apple y enlaces mágicos sin contraseña a través de la fachada `Auth` propiedad del framework. Magnetar proporciona los motores de credenciales, ceremonias, identidad, compuerta de factores y sesiones que hay detrás de esa fachada.

Los puntos de entrada públicos son:

- `Auth::oauth(provider)` para OAuth y Apple.
- `Auth::magic_link()` para el inicio de sesión por email sin contraseña.

Suprnova no instala rutas para estos flujos. La aplicación proporciona
handlers pequeños de inicio y callback y decide cómo entregar el correo
del enlace mágico.

## Inicializar Magnetar con OAuth

Configure OAuth en la misma `MagnetarConfig` que inicializa los servicios de contraseña, clave de acceso, sesión, bloqueo y autenticación de dos factores. El registro de proveedores se publica de forma atómica con esos servicios: si algún servicio no puede construirse, ninguno queda visible.

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

El framework reexporta el contrato `OAuthProvider`, los cinco proveedores propios y los tipos de configuración, y todos los tipos necesarios para implementar un proveedor personalizado. `ReqwestOAuthTransport` proporciona E/S de producción para tokens, userinfo y revocación. `FrameworkAbuseLimiter` usa el `RateLimiterDriver` configurado por la aplicación. Las aplicaciones no necesitan ni una dependencia directa de `suprnova-magnetar` ni adaptadores de transporte y limitador escritos a mano.

`MagnetarConfig` crea su esquema cuando `apply_migrations` está habilitado, que es el valor predeterminado. Use `.apply_migrations(false)` solo cuando el despliegue prepare el mismo esquema por separado. Una segunda inicialización devuelve un error en lugar de reemplazar cualquier motor instalado.

### Requisitos del proveedor de GitHub

El endpoint REST de usuario de GitHub requiere un `User-Agent`; un proveedor de la comunidad lo añade, junto con cualquier valor de `Accept` de tipo de medio que necesite, mediante `OAuthProvider::userinfo_headers`. Suprnova añade por separado la cabecera bearer `Authorization` y rechaza los intentos del proveedor de sobrescribirla.

La respuesta `/user` de GitHub incluye un correo electrónico solo cuando el usuario lo hizo público. La dirección primaria verificada requiere una segunda solicitud a `/user/emails`, mientras que `resolve_identity` deliberadamente no realiza E/S y recibe una respuesta userinfo. Un proveedor de GitHub puede devolver `email: None` y usar la ceremonia de finalización de correo electrónico de Suprnova, o apuntar `userinfo_endpoint` a un adaptador de host que combine `/user` con el correo electrónico primario verificado. No trate una dirección no verificada o meramente pública como propiedad de la cuenta.

## Vinculación de sesión

El inicio OAuth requiere `SessionMiddleware`. Magnetar vincula la
ceremonia a un digest de la sesión del framework que la inicia, así que
el callback no puede moverse a otra sesión del navegador.

Un inicio de sesión correcto por contraseña, enlace mágico, passkey u
OAuth rota el ID de sesión y el token CSRF, registra el ID de usuario de
la aplicación y almacena una vinculación web opaca de Magnetar. Hidratar
remember-me rota tanto la credencial Magnetar como la vinculación de
sesión del framework.

## Iniciar un flujo OAuth

Usa `begin` en el handler de inicio del proveedor:

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// Devuelve una redirección HTTP a kickoff.authorization_url.
```

El `OAuthKickoff` devuelto contiene:

- `authorization_url`, la URL que se envía al navegador.
- `state`, el selector de un solo uso vinculado a la sesión inicial.

Magnetar es dueño de la generación de estado, la política PKCE, la persistencia de la ceremonia, el intercambio con el proveedor, la verificación de identidad y la limitación de abuso. El controlador host es dueño de la redirección HTTP y la ruta callback.

## Verificar o completar el callback

El callback tiene dos puntos de entrada:

| Método | Resultado | Efectos secundarios |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | Verifica la prueba del proveedor y devuelve proveedor, subject, email verificado y nombre visible sin crear sesión de aplicación. |
| `complete(code, state)` | `(User, Session)` | Resuelve la identidad mediante el motor host instalado, aplica la política de vinculación de cuentas y la compuerta de factores, rota la sesión del framework y devuelve el usuario propiedad del framework y los valores de sesión de Magnetar. |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` solo existe cuando el proveedor proporcionó un
email verificado. Persiste proveedor y subject como identidad externa
estable; el email no es un identificador estable del proveedor.

## Política de vinculación de cuentas

Completar OAuth no considera que poseer una cadena de email no verificada
pruebe que quien llama posee una cuenta existente.

El resultado puede exigir trabajo adicional en lugar de emitir una sesión:

- **Se requiere completar el email** devuelve HTTP 409 cuando la identidad
  necesita una ceremonia separada de email verificado.
- **Se requiere vinculación explícita** devuelve HTTP 409 cuando una cuenta
  verificada existente debe autorizar la vinculación.
- **Se requiere factor** devuelve HTTP 401 cuando la política exige un
  segundo factor antes de emitir la sesión.

Una finalización de email verificado que gana la frontera de primera
prueba de email recupera atómicamente una cuenta no verificada usurpada.
La transacción avanza la época, elimina credenciales provisionales,
revoca sesiones y credenciales remember antiguas y adjunta la cuenta del
proveedor verificada. Una cuenta verificada nunca se vincula solo por
email automáticamente.

## Sign in with Apple

Apple usa la misma fachada `Auth::oauth("apple")`, pero su callback suele
usar `response_mode=form_post`. Registra el callback como ruta `POST` y
pasa el campo `user` opcional del formulario mediante los métodos Apple:

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` incluye subject estable, email verificado opcional,
`email_verified` e `is_private_email`. Persiste subject como clave
estable. Apple puede proporcionar el nombre visible solo durante la
primera autorización, así que el adaptador conserva ese primer valor
`form_post`.

La verificación del token e identidad de Apple pertenece a la
implementación instalada del proveedor. Los proveedores actuales de
Magnetar exigen comprobaciones de firma, issuer, audience, expiración y
nonce, en lugar de confiar en el JSON decodificado del ID token.

## Inicio mágico por enlace

El inicio mágico usa el motor Magnetar instalado de contraseña/sesión. El
framework devuelve el token de un solo uso en texto plano, mientras la
aplicación posee la composición del correo y la forma de la URL:

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` aplica el presupuesto de abuso de autenticación antes de emitir el token. `consume` es de un solo uso, aplica la compuerta de factores, vincula la sesión resultante a la sesión de la solicitud del framework y devuelve el usuario y la sesión Magnetar.

Para una cuenta existente no verificada, consumir correctamente el
enlace es la primera prueba de email. La transacción recupera la cuenta
y elimina estado provisional de contraseña, passkey, cuenta vinculada,
dos factores, sesión y remember para que un usurpador anterior no
conserve acceso.

## Rutas que añadir

Una aplicación típica añade estas rutas:

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

Aplica `SessionMiddleware` a cada ruta de inicio/callback OAuth y passkey.
La sesión contiene el selector de ceremonia y vincula el viaje de ida y
vuelta al navegador que lo inició.

## Migración de autenticación

El crate `suprnova-magnetar` incluye un motor de migración consciente de
la forma para Torii, Suprnova web, Suprnova API y esquemas Magnetar
existentes. Es una superficie de biblioteca y ejemplo, no un subcomando
de la CLI `suprnova`.

Habilita la feature `migration` junto al driver de la base de datos de
origen y ejecuta un plan en seco antes de aplicar. Para PostgreSQL:

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

Usa `seaorm-mysql` o `seaorm-sqlite` cuando correspondan a los drivers
de la base de datos de origen y aplicación.

Añade `--apply` para aplicar el plan revisado. El runner vuelve a
comprobar huellas de origen y esquema, registra reintentos, rechaza
colisiones de identidad y usa importaciones transaccionales. Las
migraciones MySQL en la misma base usan un intercambio shadow protegido
por barrera de escritura, restauración reanudable y rutas de aborto.

Conserva el plan y el informe generados en los registros de despliegue.
No apliques un plan cuya huella de origen cambió después de revisarlo.

## Referencia

- Arranque predeterminado: `MagnetarConfig`, `PasskeyConfig`,
  `init_magnetar`.
- Fachadas: `Auth::oauth(provider)` y `Auth::magic_link()`.
- Instalación de OAuth: `MagnetarConfig::oauth`, `ReqwestOAuthTransport` y `FrameworkAbuseLimiter`.
- Biblioteca de migración: `magnetar::migration` del crate
  `suprnova-magnetar`.
- Autenticación bearer: `BearerTokenMiddleware`.

## Siguiente

- [Autenticación](authentication.md) cubre contraseña, passkey, guards,
  sesiones del framework e inicialización del motor.
- [Flujos de autenticación](auth-flows.md) cubre verificación de email,
  restablecimiento de contraseña, bloqueo y dos factores.
- [Correo](mail.md) cubre la entrega de enlaces mágicos propiedad de la
  aplicación.
- [Sesión](session.md) cubre la sesión de navegador que vincula las
  ceremonias OAuth y passkey.
