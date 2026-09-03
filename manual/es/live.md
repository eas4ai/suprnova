# Live

Suprnova Live es el motor de interacción dirigido por el servidor del framework.
Un componente Live es un struct de Rust cuyo estado vive en el servidor, cuya
vista es una plantilla Askama y cuyas acciones se ejecutan sobre un protocolo
firmado desde un pequeño runtime de navegador que transforma en el sitio el
HTML re-renderizado. No hay un modelo de estado del lado del cliente que
mantener sincronizado, ninguna herramienta de build que instalar para usar el
runtime distribuido y ningún JavaScript inline en tus documentos.

Este capítulo cubre la superficie orientada a la aplicación: escribir un
componente, registrarlo, servir documentos e islas, los límites de seguridad
que cruza cada petición Live, subidas, actualizaciones asíncronas, assets,
pruebas, diagnóstico y recuperación. Todo lo que aparece aquí usa únicamente
`suprnova::live` y `suprnova::view`.

## Inicio rápido

Un proyecto creado con `suprnova new` está listo para Live: incluye
`src/live/mod.rs` con un registro de componentes vacío y una función
`routes()`, su bootstrap enlaza el registro y `cmd/main.rs` instala las rutas.
Genera un componente y luego compruébalo:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` escribe `src/live/counter.rs` y `templates/live/counter.html`,
registra el componente en `src/live/mod.rs` e imprime los siguientes pasos.
`live:check` compila tu aplicación y prueba cada vista registrada contra el
comprobador integrado.

## Escribir un componente

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` es el nombre registrado del componente. Usa un nombre con puntos en
  kebab-case como `app.counter`; la CLI deriva `<package>.<kebab>`.
- `view` es la identidad de la plantilla, relativa a la raíz de plantillas.
- Los campos `#[public]` se renderizan y viajan en el snapshot firmado. Los
  campos `#[model]` además aceptan propuestas del navegador mediante `live:model`.
- Los métodos `#[action]` son los únicos puntos de entrada que el navegador
  puede invocar. Reciben argumentos validados y pueden devolver resultados
  tipados como una redirección o un flash.

Cada tipo de campo debe implementar `Default`; una isla nueva parte de esos
valores por defecto salvo que un hook de montaje indique otra cosa.

## Vistas

Las vistas son plantillas Askama. La raíz de plantillas es `templates/` salvo
que un `askama.toml` nombre otros directorios, así que `live/counter.html` vive
en `templates/live/counter.html`:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

Las directivas usan la gramática cerrada `live:`: `live:click`, `live:submit`,
`live:model`, `live:upload`, `live:key`, `live:loading` y el resto del conjunto
documentado. El comprobador prueba cada directiva contra el componente: una
acción desconocida, un campo de modelo desconocido, un filtro `safe` sin
procesar o una violación de accesibilidad hacen fallar `live:check` con el
archivo, la línea y la columna.

Los documentos que colocan islas son vistas ordinarias declaradas con
`#[suprnova::view]`; el único valor sin escapar que aceptan es `TrustedHtml`
a través del filtro `trusted_html`.

## Registro y bootstrap

`src/live/mod.rs` posee el registro y las rutas:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Enlázalo durante el bootstrap para que el servidor, los workers y los comandos
`suprnova live:*` vean los mismos componentes:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

El registro es inmutable una vez que el runtime se ensambla. Un nombre de
componente o una vista duplicados, o un componente cuyas acciones necesitan
validación sin un puerto de validación, hacen fallar el registro con un
`RegistryError` tipado.

## Rutas

`Router::try_live()` instala el espacio de nombres reservado exactamente una
vez: `/__live/v1/action`, `/__live/v1/upload`, las rutas de control y el
handshake WebSocket de `/__live/v1/async/*`, y las rutas inmutables de
`/__live/v1/assets/*`. El arranque falla si una ruta de la aplicación puede
reclamar `/__live`.

Las rutas de petición reservadas llevan una política estricta: cada petición
necesita hechos de sesión, origen, CSRF, principal, tenant y límite de tasa.
El framework registra la sesión y la prueba CSRF; tu aplicación adjunta el
resto con el guardián de rutas:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Instala las rutas desde el punto de entrada para que el runtime y el catálogo
de montajes estén listos antes de la primera petición:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Documentos e islas

Una ruta de documento declara sus islas una vez, las renderiza mediante
`LiveDocument` y emite las etiquetas de bootstrap:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` declara una isla que cualquier visitante puede
  renderizar; su estado es una semilla reutilizable promovida a instancia en la
  primera acción.
- `LiveMount::identity_bound` declara una isla que pertenece a la sesión y al
  principal actuales; la ruta de documento debe autenticar.
- Monta cada isla antes de `bootstrap` y llama a `bootstrap` una sola vez. El
  bootstrap emite el elemento de configuración inerte y las etiquetas script
  para la estrategia ESM o clásica, añadiendo los roles de subida y asíncrono
  cuando un componente montado los necesita y el puente Stimulus bajo demanda.
- La plantilla del documento coloca `{{ bootstrap|trusted_html }}` en `<head>`
  y cada isla donde corresponde.

## Límites de seguridad

Live nunca elude el middleware del framework. Lo que necesita cada petición:

| Hecho | Registrado por |
|---|---|
| Sesión | `SessionMiddleware` |
| Origen y CSRF | `CsrfMiddleware` con la verificación de origen activada |
| Principal | `AuthMiddleware` en su rama autenticada |
| Tenant | `LiveTenantMiddleware` con tu resolutor |
| Límite de tasa | `RateLimitMiddleware` en su rama permitida |

El runtime distribuido envía el tipo de medio Live y la cabecera propia del
navegador `Sec-Fetch-Site`; no lleva ningún token de sesión. El middleware
CSRF verifica esa prueba por sí mismo en cada petición Live, sea cual sea la
política de origen que configures: una petición Live del mismo origen pasa con
la disposición CSRF sin estado, mientras que una petición entre sitios o sin
cabecera recurre a la validación por token y es rechazada. Las rutas
ordinarias conservan la validación por token bajo la política predeterminada;
usar Live no relaja nada más:

```rust
global_middleware!(CsrfMiddleware::new());
```

Los visitantes anónimos renderizan semillas públicas y pueden actuar sobre
ellas cuando el guardián usa `AuthMiddleware::optional()`: un principal con
sesión iniciada se registra, un visitante anónimo continúa y el tipo de montaje
decide. Una semilla pública se promueve entonces para la propia sesión del
visitante en la primera acción, mientras que una isla ligada a identidad sigue
rechazando una petición sin prueba de principal. Con `AuthMiddleware::new()` el
guardián responde `401` a toda petición anónima antes de cualquier trabajo del
motor. Las islas ligadas a identidad requieren una sesión y un principal; el
tenant se liga al ámbito de la isla siempre que tu resolutor nombre uno, y un
resolutor que no pueda determinar el tenant debe devolver un error en lugar de
`None`. Todo rechazo es cerrado: un `409` por un snapshot obsoleto o
manipulado no lleva cuerpo, y los mensajes de producción nunca incluyen
snapshots, tokens, cookies ni HTML renderizado.

## Subidas

Declara una política de subida sobre un campo de modelo:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

La vista enlaza el campo con `<input type="file" live:upload="avatar">`. El
runtime crea, transfiere y completa la subida mediante `/__live/v1/upload`; el
archivo espera en cuarentena hasta que se ejecuta la acción de finalización
declarada, momento en que el framework lo entrega a tu `UploadFinalizer`.
Enlaza el finalizador, y cualquier escáner o validador, antes de que el runtime
se ensamble:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Las subidas se autorizan por campo y control a través del gate. Define las
capacidades `live:<component>.upload.<field>.<Control>` para `Create`,
`Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
`Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`
y `Fail`.

Un navegador que perdió su concesión de transferencia la readquiere mediante
una ruta que tu aplicación posee fuera del espacio de nombres reservado:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

La ruta exige los mismos hechos que una acción, responde solo a la sesión y al
principal que crearon la subida, y devuelve una concesión nueva con el estado
actual de la transferencia.

## Actualizaciones asíncronas

Un componente declara los streams que escucha; el runtime del navegador se
suscribe por SSE o WebSocket y recurre al polling como alternativa:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Define la capacidad `live:<component>.stream.<name>` para los suscriptores y
luego publica desde cualquier parte de la aplicación:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

Un refresh indica a las islas suscritas que se re-rendericen desde cero; un
evento se entrega a los manejadores registrados de la isla. El polling es el
render fresco ordinario: el estado de la isla se pone al día cuando un
transporte no está disponible, pero las cargas de eventos publicadas entre
tanto no se reenvían a sus manejadores, y el runtime lo informa como un stream
degradado en lugar de actual. Un componente que declara exactamente un stream
obtiene su raíz de isla suscrita a él; un componente con varios streams se
suscribe a cada uno mediante las llamadas registradas del runtime.

## Assets y uso sin build

El framework sirve los artefactos de runtime exactos revisados en
`/__live/v1/assets/<identity>/<file>` con caché inmutable, validadores fuertes
y atributos de integridad en las etiquetas de bootstrap. Una política estricta
`script-src 'self'` se mantiene porque los documentos no contienen script
inline. Para publicar los mismos bytes en una CDN o en un directorio estático:

```bash
suprnova live:assets --out public/__live
```

La publicación es atómica y se niega a reemplazar un directorio cuyos bytes
difieren a menos que pases `--replace`.

## Pruebas

`suprnova::live::testing` prepara el runtime y el catálogo de montajes de un
router para pruebas en proceso. Las pruebas de la aplicación en
`app/tests/live_*.rs` muestran el patrón completo: una base de datos en
memoria, una cookie de sesión sembrada, la pila de middleware global real y
peticiones a través de `handle_request`:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Decodifica el snapshot de una isla desde su atributo
`data-suprnova-live-snapshot`, envía una acción con la cookie de sesión y
`Sec-Fetch-Site: same-origin`, y comprueba el render aceptado. Un snapshot
obsoleto responde `409` con cuerpo vacío; un principal ausente responde `401`.

## Diagnóstico y operación

- `suprnova live:check` prueba cada vista registrada; `--allow-unproved`
  acepta estructuras dinámicas sobre las que el comprobador deliberadamente no
  se pronuncia.
- `suprnova live:inspect` informa del registro enlazado, los límites de
  configuración, las capacidades de subida instaladas, los servicios de runtime
  ensamblados y la identidad de assets sin exponer estado ni secretos.
- `LiveConfig` acota los bytes de petición y respuesta y la vida del contexto
  de confianza; enlaza uno personalizado antes de que el runtime se ensamble.
- Los errores llevan tipos cerrados como `live_document_context_rejected` e
  `invalid_live_bootstrap`; las etiquetas de telemetría son enumeraciones
  cerradas.

## Recuperación

- Un `409` indica al runtime que re-renderice la isla desde cero; la operación
  no se repite.
- Un transporte asíncrono cerrado se retira y el runtime se reconecta con una
  nueva generación de transporte; una generación obsoleta es rechazada.
- Una sesión que expira o rota invalida el trabajo ligado a identidad; la
  aplicación muestra su ruta de inicio de sesión y el visitante continúa desde
  un documento nuevo.

Live funciona completo sin RenderCache; cachear documentos Live es una
funcionalidad aparte con su propio capítulo cuando llegue.

## Referencia de la CLI

| Comando | Propósito |
|---|---|
| `suprnova live:make <name>` | Generar un componente y su vista y registrarlo |
| `suprnova live:check` | Probar cada vista registrada con el comprobador integrado |
| `suprnova live:inspect` | Informar del estado seguro de runtime, registro, proveedores y artefactos |
| `suprnova live:assets --out <dir>` | Publicar atómicamente los artefactos de runtime revisados |
