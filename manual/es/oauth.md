# OAuth, inicio de sesión con Apple e inicio mágico por enlace

Suprnova expone OAuth, Sign in with Apple y enlaces mágicos sin contraseña a través de la fachada `Auth` propiedad del framework. Magnetar proporciona los motores de credenciales, ceremonias, identidad, compuerta de factores y sesiones que hay detrás de esa fachada.

Los puntos de entrada públicos son:

- `Auth::oauth(provider)` para OAuth y Apple.
- `Auth::magic_link()` para el inicio de sesión por email sin contraseña.

Suprnova no instala rutas para estos flujos. La aplicación proporciona
handlers pequeños de inicio y callback y decide cómo entregar el correo
del enlace mágico.

## Inicializar Magnetar

Inicializa los motores predeterminados de contraseña, passkey, sesión,
bloqueo y dos factores después de `DB::init` y de que `APP_KEY` haya
inicializado `Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`MagnetarConfig` usa la conexión SeaORM de la aplicación. El motor
predeterminado crea su esquema cuando `apply_migrations` está habilitado,
que es el valor por defecto. Usa `.apply_migrations(false)` solo cuando
el despliegue prepara ese esquema por separado.

`init_magnetar` instala atómicamente los adaptadores de
contraseña/sesión y passkey. Una segunda instalación devuelve un error
en lugar de reemplazar el motor y dividir el estado de autenticación.

## Instalar el motor OAuth

La compatibilidad con OAuth se compila mediante la feature predeterminada
`magnetar-oauth` del framework, pero el registro del proveedor siempre es un
paso explícito en tiempo de ejecución. En una compilación
`--no-default-features`, habilita `magnetar-oauth` explícitamente.
`init_magnetar` no devuelve ni expone su host engine concreto interno, por lo
que el ejemplo siguiente solo se aplica a una aplicación que construye y
conserva su propio `MagnetarHostEngine`; no se puede añadir al ejemplo de
inicialización predeterminada anterior. La API pública actual no tiene un
método de conveniencia para añadir un registro OAuth a un motor ya instalado
mediante `MagnetarConfig`.

```rust,ignore
use std::sync::Arc;
use suprnova::magnetar_integration::install_magnetar_oauth_engine;


// Estos valores deben estar en el ámbito que construyó el host engine personalizado.
let oauth = host_engine.oauth_service(oauth_host_config)?;
install_magnetar_oauth_engine(Arc::new(oauth))?;
```

`MagnetarOAuthHostConfig` recibe una lista explícita de
`MagnetarOAuthProviderConfig`, transporte HTTP, limitador de abuso,
política de autorización y política de vinculación automática. Al
instalarlo, el registro es autoritativo; un proveedor desconocido falla
cerrado en lugar de recurrir a otra implementación.

Las implementaciones y expedientes de autenticación de clientes proceden
del crate `suprnova-magnetar`. La aplicación que construye el motor OAuth
debe declarar ese crate como dependencia directa con las features de
proveedor utilizadas. El framework no infiere IDs ni secretos desde
variables de entorno: léelos mediante la configuración de la aplicación
o un gestor de secretos y construye el registro durante el bootstrap.

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
- Instalación OAuth:
  `suprnova::magnetar_integration::install_magnetar_oauth_engine` y
  los tipos de configuración en `suprnova::magnetar_integration::engine`.
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
