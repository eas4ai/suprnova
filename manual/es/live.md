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
  Un campo de modelo declara su temporización en el atributo, como
  `#[model(debounce = 250)]`; un debounce dura 100, 250 o 500 milisegundos,
  las duraciones que acepta `live:model.debounce.<n>ms`, y cualquier otra no
  compila.
  Un formulario enviado con `live:submit` propone cada control de modelo que
  contiene en una sola petición, que lleva hasta 127 campos además de la
  acción; `live:check` rechaza un formulario mayor.
  Una propuesta que el campo no puede decodificar, como un número vacío para
  un campo `u64`, es un error de validación en ese campo: la acción no se
  ejecuta, el campo conserva su valor y `live:error` muestra el error.
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
`live:key` nombra la identidad estable de un elemento a través de los morphs y es el único atributo de clave que escribe una plantilla: el runtime lo lee para la identidad del morph, para los controles de morph como `live:preserve.self` y para los ámbitos que conservan estado del navegador; `data-suprnova-live-key` es la grafía propia del motor en las raíces que él renderiza. Los valores de `live:key` y los ids de elemento dentro de una isla usan un solo alfabeto, el que el runtime comprueba en cada morph: primero una letra o un dígito ASCII, después letras, dígitos, `_`, `-`, `.` y `:`, como máximo 128 bytes, cada uno único en la isla. `live:check` rechaza una clave o un id literal fuera de ese alfabeto, y un id que un bucle repite.

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
vez: `/__live/action`, `/__live/upload`, las rutas de control y el
handshake WebSocket de `/__live/async/*`, y las rutas inmutables de
`/__live/assets/*`. El arranque falla si una ruta de la aplicación puede
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
runtime crea, transfiere y completa la subida mediante `/__live/upload`; el
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
evento se entrega a los handlers registrados de la isla. El polling es el
render fresco ordinario: el estado de la isla se pone al día cuando un
transporte no está disponible, pero las cargas de eventos publicadas entre
tanto no se reenvían a sus handlers, y el runtime lo informa como un stream
degradado en lugar de actual. Un componente que declara exactamente un stream
obtiene su raíz de isla suscrita a él; un componente con varios streams se
suscribe a cada uno mediante las llamadas registradas del runtime.

Un stream termina con la sesión que lo abrió. Cuando una sesión se destruye en
el nodo que sostiene el stream, por un cierre de sesión simple, invalidación,
regeneración del id o un "cerrar sesión en todas partes", toda membresía que
abrió allí se retira de inmediato y ningún evento posterior la alcanza. Una
sesión destruida en otro nodo la detecta la propia entrega: la sesión de cada
membresía se vuelve a comprobar contra el almacén de sesiones como mucho una
vez cada diez segundos, así que los eventos cesan dentro de ese intervalo. El
gate del stream se consulta de nuevo antes de cada entrega en cualquier caso,
de modo que un cambio de política termina la entrega de inmediato en todos los
nodos.

## Assets y uso sin build

El framework sirve los artefactos de runtime exactos revisados en
`/__live/assets/<identity>/<file>` con caché inmutable, validadores fuertes
y atributos de integridad en las etiquetas de bootstrap. Una política estricta
`script-src 'self'` se mantiene porque los documentos no contienen script
inline. Para publicar los mismos bytes en una CDN o en un directorio estático:

```bash
suprnova live:assets --out public/__live
```

La publicación es atómica y se niega a reemplazar un directorio cuyos bytes
difieren a menos que pases `--replace`.

## Biblioteca de componentes

Suprnova incluye los cimientos de una biblioteca de componentes para Live: una
hoja de estilos de tokens con una capa base y una familia de formularios de
componentes presentacionales construidos sobre controles nativos y el
vocabulario `live:model`, `live:error` y `live:loading`. La base es un
artefacto de runtime. Si un documento se suscribe, llega como un único enlace
de hoja de estilos bajo el mismo contrato de identidad, integridad y caché que
los scripts del runtime:

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

Cada regla vive dentro de la capa de cascada `suprnova-ui`, así que tus
propios estilos sin capa ganan sin pelea de especificidad. Cada valor visual
es una propiedad personalizada `--sn-` para color, fuente, espacio, radio,
sombra, movimiento, densidad y estado, con valores claros y oscuros:
sobrescribe un token en `:root` para cambiar el tema, o quita la capa y
conserva cada comportamiento, nombre y atributo de estado, porque un
componente estiliza sus estados desde los atributos que el checker prueba
(`aria-invalid`, `aria-busy`, `aria-expanded`, `aria-pressed`, `aria-current`,
`aria-selected`, `:disabled`), nunca desde una clase. Un preset `@theme` de
Tailwind CSS 4 mapea los tokens a los espacios de nombres de Tailwind;
Tailwind nunca es obligatorio. Los componentes se instalan con `live:add`, un
directorio cada uno bajo la raíz reservada `templates/suprnova-ui/`: la vista
con macros de Askama, la hoja de estilos, el JavaScript cuando el componente
lo tiene y el manifiesto que los nombra:

```bash
suprnova live:add field
suprnova live:add password-input
```

`live:add` registra el digest de cada archivo que escribe, así que una
ejecución posterior reemplaza un archivo que nunca se editó cuando la
biblioteca lo cambia, conserva un archivo editado y lo indica; `--force`
también reemplaza un archivo editado. Un componente de terceros se instala
desde su propio manifiesto con `--manifest`, bajo su propia raíz, y cada
archivo que nombra debe ser un archivo regular dentro del directorio del
manifiesto, nunca un enlace simbólico. Llama a las macros desde tus vistas,
sirve la hoja de estilos y el script con `try_live_ui_assets()` y enlázalos
desde el documento:

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

`try_live_ui_assets()` lee los archivos de `templates/suprnova-ui/` bajo la
ruta base de la aplicación en cada petición, así que distribuye ese directorio
con el binario e inicia la aplicación desde el directorio que lo contiene, o
asigna ese directorio a `APP_BASE_PATH`. Cuando el directorio no se puede
leer, la aplicación se niega a arrancar y lo nombra.

El checker expande las macros, así que `live:check` prueba una vista de la
biblioteca como cualquier otra. La familia de formularios hoy: campo,
etiqueta, entrada, área de texto, entrada numérica, deslizador, entrada de
búsqueda, entrada de contraseña con revelado, casilla y grupo de casillas,
grupo de radios, interruptor, selector, botón y botón enlace, grupo de
botones, fieldset, acciones del formulario, resumen de validación y entrada de
archivo. Los componentes de la biblioteca se llaman `suprnova.*` y el registro
rechaza ese prefijo desde cualquier otro crate; los elementos personalizados
son light DOM y llevan el prefijo `sn-`.

Cada control de valor recibe el valor actual de la isla, así que la página lo
muestra y un envío que no cambió nada lo devuelve sin cambios: `value=` para
una entrada, un área de texto, una entrada numérica, un deslizador, una
entrada de búsqueda y un selector de fecha, `checked=` para una casilla y un
interruptor, y `selected=` para un selector, un grupo de radios y un grupo de
casillas, que recibe la lista de valores marcados. Una contraseña y un código
de un solo uso nunca renderizan su valor. Las entradas de los grupos de radios
y de casillas llevan su valor como clave, así que una elección que el usuario
aún no ha enviado sobrevive a un nuevo renderizado. Un campo al que se vincula
más de una casilla se propone como la lista de valores marcados, y un grupo de
una sola casilla como un booleano. Un renderizado que debe reemplazar lo que
el usuario ha escrito, como el que responde a un reinicio, pasa un número de
secuencia como `authority=`, y el siguiente renderizado no pasa ninguno:

```html
{% call input::input("email", kind="email", value=email, authority=authority) %}{% endcall %}
{% call checkbox::checkbox_group("topics", "Topics", topic_options, topics) %}{% endcall %}
```

La familia de overlays se entrega sobre los mismos cimientos: tooltip,
collapsible y accordion, popover, un menú desplegable de un solo nivel, dialog,
sheet y drawer. Cada uno mantiene su estado abierto mediante la primitiva
propia del navegador antes de que corra cualquier script: `details` para los
disclosures, el atributo `popover` para popovers y menús, y `dialog` para los
tres modales, que los elementos vendidos `sn-dialog`, `sn-sheet` y `sn-drawer`
abren con `showModal()` y cierran devolviendo el foco al disparador. Abrir y
cerrar nunca hace una petición Live; solo lo hace una acción que coloques
dentro de un overlay. Cada raíz de overlay lleva una clave estable y
`live:preserve.self`, así que un overlay abierto sobrevive a un morph que no
reemplazó su región:

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

El atributo `popover` fija la base soportada en Chrome y Edge 114, Firefox 128
y Safari 17. Donde existe el posicionamiento por ancla de CSS, el popover y el
menú se sitúan bajo su disparador; en otro caso el navegador los centra. El
modo de apertura única del accordion descansa en `details name`, que las
versiones soportadas más antiguas tratan como disclosures independientes. El
tooltip sigue abierto mientras el puntero pasa de su disparador a la burbuja,
así que su texto se puede leer o seleccionar.

Siguen la familia de feedback y la familia de navegación. Feedback: alert,
skeleton, spinner, progress, empty state y una región de toasts con una región
de flash a su lado. Cada uno presenta un estado que el servidor o el runtime
ya tienen. Un alert elige su rol según su variante y marca cada variante con
un glifo y una etiqueta oculta, nunca solo con color. Un spinner o skeleton se
enlaza con `live:loading.show` a una acción registrada y se entrega oculto,
de modo que el runtime lo revela tras su propio retardo y lo mantiene más allá
de su mínimo, y una acción rápida nunca lo hace parpadear. Progress es el
elemento nativo `progress` con etiqueta y lectura en texto, y lleva un valor
solo para trabajo determinado. El empty state toma su motivo (vacío, sin
resultados, sin permiso, desconectado) del estado renderizado en el servidor y
ofrece una acción siguiente solo donde quien lo llama la renderiza. Un toast
anuncia una vez desde una región de estado cortés y nunca toma el foco; el
elemento vendorizado `sn-toast-region` expira los toasts, pausa mientras el
puntero está sobre cualquier parte de un toast o el foco está dentro de él,
limita cuántos se muestran a la vez y responde al botón de cierre, con cada
toast con clave y preservado para que uno cerrado siga cerrado tras un morph. Un error crítico también pertenece a un alert; un toast nunca es su única superficie. Los toasts se renderizan dentro de un bucle, así que sus claves pasan por el filtro `live_key`, y la island que los monta lo expone con `pub mod filters { pub use suprnova::view::filters::live_key; }`. `live_key` hace fallar el renderizado de la isla con un valor fuera del alfabeto de claves, como una dirección de correo electrónico; genera las claves de esos datos con `live_key_digest`, que convierte cualquier valor en una clave estable dentro del alfabeto y se exporta del mismo modo desde `suprnova::view::filters`. La región de flash renderiza una sola vez lo que
la petición anterior dejó en la sesión:

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

Navegación: barra de cabecera, footer, sidebar con grupos plegables,
breadcrumbs, tabs, paginación y load more. Cada destino es un ancla con una
URL de ruta real y cada acción es un botón; el elemento actual lleva
`aria-current` desde el valor que usted enlaza, nunca desde la ubicación del
navegador. Los grupos del sidebar son `details` nativos, con clave y
preservados. Las tabs exigen un modo: `local`, con paneles con semántica de
tablist, teclas de flecha desde el elemento vendorizado `sn-tabs` y ninguna
petición al cambiar, o `route`, con tabs como anclas. Las tabs se anidan: una
instancia de tabs interior selecciona solo sus propias tabs y paneles. La
paginación también exige un modo: las páginas de ruta son enlaces canónicos
y las páginas Live son botones sobre sus acciones cuyo resultado refleja la
nueva query en la entrada de historial actual mediante `url_intent`, sin
entrada por página.
Load more es un botón sobre una acción registrada que añade a una lista con
claves, de modo que el morph conserva cada fila ya presente, y el control desaparece cuando usted lo renderiza agotado. Una reflexión de URL es un resultado del protocolo 2, así que una island que pagina mediante `url_intent` declara `minimum_protocol_version = 2`; sus filas con claves pasan por `live_key` como lo hace un toast:

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

La familia de visualización de datos cierra el conjunto integrado.
Presentacionales: separator, scroll area, aspect image, card, badge, avatar y
grupo de avatares, list group, description list y stat card. Cada uno
conserva el orden del documento y la semántica nativa: el separator es un
`hr` o un rol separator etiquetado, el scroll area es una región etiquetada
enfocable que se desplaza de forma nativa, el aspect image es el propio `img`
con una proporción con nombre, la card es un article o una section
etiquetada por su propio encabezado con las acciones en un grupo etiquetado,
y la description list es un `dl`. Un badge siempre lleva su texto, un avatar
nombra a su persona en `alt` o en la etiqueta de sus iniciales, y la
tendencia de una stat card dice "Up", "Down" o "Flat" en texto antes del
delta, de modo que ningún estado depende solo del color. La list group
asigna clave a cada elemento mediante `live_key`, así que un reordenamiento
conserva cada nodo. El chart se renderiza en el servidor: la island llama a
`render_chart` de `suprnova::live::charts`, que dibuja marcas de barras o
líneas mediante `charts-rs` a partir de series tipadas acotadas y devuelve
marcado de confianza, y la macro renderiza el SVG junto a un resumen de
texto y una tabla de datos en un disclosure, de modo que el documento
canónico se lee sin la imagen y ningún script de gráficos llega al navegador:

```rust
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};

pub fn chart_svg(&self) -> TrustedHtml {
    render_chart(
        ChartKind::Bar,
        &["Apr", "May", "Jun"],
        &[ChartSeries::new("Revenue", vec![42.0, 47.0, 51.0])],
    )
    .expect("a bounded fixed series renders")
}
```

`render_chart` devuelve un error en lugar de dibujar un valor cuya magnitud
supera 1e9, más allá del rango que admite la aritmética de ejes del
renderizador.

La datatable es el último componente, con una island por tabla. Es una
`table` nativa con un caption que nombra el recuento de resultados,
encabezados de columna con `scope` y `aria-sort` en la columna ordenada.
Ordenar y filtrar son submits Live sobre los campos model de la island, los
cambios de página son botones Live, y la island declara el orden aplicado,
la dirección, el filtro y la página como campos `#[url]` y los refleja
mediante `url_intent` tras cada acción, así que la barra de direcciones
siempre contiene una URL compartible y el documento monta la misma vista a
partir de ella:

```rust
#[live(name = "app.invoices", view = "live/invoices.html", minimum_protocol_version = 2)]
pub struct Invoices {
    #[model]
    pub sort: String,
    #[url(key = "sort")]
    pub sorted_by: String,
    #[url(key = "dir")]
    pub direction: String,
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    #[url(key = "page")]
    pub page: u64,
    pub rows: Vec<Invoice>,
}
```

La familia live-native es la última: los componentes que solo tienen sentido sobre el runtime en ejecución. El widget de subida presenta el protocolo de subida incluido: su campo de archivo lleva `live:upload` para el campo de subida de la isla, su elemento `progress` es la raíz de progreso del runtime, y cancelar, reintentar y quitar actúan sobre la referencia temporal a través de `live:upload.cancel` y sus hermanos. Cada estado que el dominio conoce se renderiza como texto y se muestra según el `data-live-upload-state` de la raíz de progreso, y "ready" se lee como verificado pero no guardado, porque nada es durable hasta que corre la acción finalizadora:

```html
{% call upload::upload("attachment", "Attachment", accept="image/png") %}{% endcall %}
<button type="submit" live:loading.disabled="save_attachment">Save attachment</button>
```

El feed en vivo y la campana de notificaciones viven en una isla respaldada por un stream. El runtime escribe `data-live-stream-state` en la raíz de la isla y anuncia cada cambio en el elemento `[data-live-stream-status]` que renderizan las macros (Updates disconnected, Connecting to updates, Updates current, Updates degraded, Reconnecting to updates, Updates closed), de modo que un stream degradado, en reconexión o cerrado lo dice y solo el estado current se lee como actual. Los elementos del feed pasan por `live_key`. El menú de cuenta es un desplegable `details` con anclas y un formulario de cierre de sesión que envía con el token CSRF de la sesión; es un slot de stitch bajo RenderCache, así que una aplicación lo monta como su propia isla ligada a la identidad y el shell compartido nunca contiene el nombre del principal.

El nivel de elementos personalizados mejora controles nativos que nunca reemplaza. Cada elemento es una subclase de `HTMLElement` en light DOM definida solo por su propio archivo vendorizado, lleva el prefijo `sn-` y no guarda ningún valor de formulario, porque la entrada nativa que contiene es el control: bloquea el script y el formulario sigue enviando el mismo valor. La entrada OTP es una sola entrada nativa (`inputmode="numeric"`, `autocomplete="one-time-code"`, un patrón de longitud) sobre un modelo transitorio, y `sn-input-otp` refleja los caracteres escritos en celdas `aria-hidden`. El selector de fecha es una entrada `type="date"`, y sus tiras de año, mes y día son fieldsets de radios nativos dentro de contenedores CSS scroll-snap, así que tocar, hacer clic y las flechas seleccionan sin script; `sn-date-picker` compone una selección completa en la entrada. El combobox es el patrón accesible de combobox (`role="combobox"`, `aria-expanded`, `aria-activedescendant`, un `role="listbox"` de opciones) sobre una entrada nativa con una `datalist` para el caso sin script, y `sn-combobox` mueve la opción activa y selecciona. Por defecto, las opciones son la respuesta de tu servidor a la consulta: renderízalas para el campo de modelo en cada renderizado, y el elemento las muestra todas mientras la consulta a la que responden sea el texto de la entrada, sea cual sea la coincidencia de tu búsqueda, y mantiene oculta una respuesta a un texto anterior, de modo que un resultado obsoleto nunca reemplaza los resultados de una consulta más nueva. Pasa `remote=false` para una lista fija, que el elemento filtra por el texto escrito:

```html
{% call otp::input_otp("code", "One-time code") %}{% for index in cells %}{% call otp::otp_cell(index) %}{% endcall %}{% endfor %}{% endcall %}
{% call date::date_picker("when", "Renewal date", years, months, days, min="2026-01-01", max="2028-12-31") %}{% endcall %}
{% call combo::combobox("country", "Country", countries, query=country, placeholder="Type a country") %}{% endcall %}
```

### Por qué Suprnova diverge

Laravel incluye componentes Blade y el marcado de un kit de inicio; Suprnova
entrega la biblioteca a través del propio framework, con el vocabulario de
Live, sin que ninguna aplicación cliente posea la página. La apariencia viene
activada por defecto y se puede quitar sin que nada se rompa, que es lo que
headless significa aquí.

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

Live funciona completo sin RenderCache. Cachear documentos Live es tarea
de RenderCache; consulta [RenderCache](render-cache.md).

## Referencia de la CLI

| Comando | Propósito |
|---|---|
| `suprnova live:make <name>` | Generar un componente y su vista y registrarlo |
| `suprnova live:check` | Probar cada vista registrada con el comprobador integrado |
| `suprnova live:inspect` | Informar del estado seguro de runtime, registro, proveedores y artefactos |
| `suprnova live:assets --out <dir>` | Publicar atómicamente los artefactos de runtime revisados |
