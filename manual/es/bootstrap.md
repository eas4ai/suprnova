# Arranque de la aplicación

`bootstrap.rs` es el único lugar donde la aplicación se configura
al arrancar. Las vinculaciones del contenedor, los oyentes de eventos,
los observadores, los supervisores y el middleware global - todo lo que
deba existir antes de que la primera solicitud llegue al servidor (o el
primer job salga de la cola) se registra aquí. No existe un andamiaje de
proveedores de servicios que ensamblar.

Hay dos ganchos, no uno. `register` abarca todo el proceso: lo ejecuta
cada subcomando, incluidos `queue:work`, `schedule:work`,
`workflow:work` y el binario de consola, no solo el servidor. Registra
ahí la conexión a la base de datos, las vinculaciones del contenedor,
los oyentes de eventos, los observadores, los supervisores y el registro
de jobs de worker. `register_http_stack`, conectado mediante
`.http_bootstrap`, se ejecuta solo en la ruta del servidor (`serve` /
`web:run`): el middleware global y `Inertia::install` pertenecen ahí.
La sección «Dónde se ubica bootstrap en el orden de arranque» explica
por qué existe esta división.
## La estructura

El punto de entrada de una aplicación con andamiaje construye una
[`Application`](lifecycle.md) de forma encadenada y la ejecuta.
Bootstrap consta de dos métodos del constructor:

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`, no `#[tokio::main]`

El atributo no es cosmético, y volver a cambiarlo rompe el arranque
con un mensaje que explica por qué.

Cargar `.env` escribe en el entorno del proceso, y `set_var` solo es
seguro mientras el proceso es monohilo. `#[tokio::main]` construye el
runtime *alrededor* de todo `main`, de modo que cada hilo de trabajo
ya existe antes de que se ejecute la primera instrucción - y
cualquiera de ellos puede llamar a `getenv` indirectamente a través de
la resolución DNS, el formateo de fechas o una dependencia en C. La
condición de carrera es silenciosa cuando falla, que es la peor
propiedad que puede tener una condición de carrera.

`#[suprnova::main]` conserva el mismo `async fn main` que se
escribiría de todos modos, y simplemente reordena dos cosas: carga el
entorno, luego construye el runtime, y luego ejecuta el cuerpo sobre
él. Acepta los mismos argumentos `flavor` y `worker_threads` que
`#[tokio::main]`.

Si `Application::run` detecta que el entorno nunca se cargó desde un
contexto monohilo, se niega a arrancar en lugar de advertir - una
aplicación que arranca "sin problemas" bajo `#[tokio::main]` es
precisamente la que corrompe semanas después una lectura de entorno
sin relación aparente.

El framework llama a `bootstrap_fn` una vez durante la secuencia de
arranque, después de cargar el entorno y de activar los drivers del
runtime (Cache, Queue, RateLimit, Mail), pero antes de construir el
enrutador. La misma llamada se ejecuta para los workers en segundo plano
(`queue:work`, `workflow:work`, `schedule:work`), de modo que un
observador o un oyente registrado aquí se dispara igual para una
inserción proveniente de un job en cola que para una proveniente de un
handler HTTP. `http_bootstrap_fn` se ejecuta inmediatamente después de
`bootstrap_fn`, pero solo en la ruta del servidor: los workers en segundo
plano y el binario de consola nunca lo llaman. [Ciclo de vida de la
solicitud](lifecycle.md) recorre la secuencia completa.

Las firmas de ambas funciones quedan fijadas por
`Application::bootstrap` y `Application::http_bootstrap`:

```rust
// src/bootstrap.rs
pub async fn register() {
    // base de datos, vinculaciones, observadores, oyentes,
    // supervisores, registro de jobs de worker
}

pub fn register_http_stack() {
    // middleware global, Inertia::install
}
```

`register` devuelve `()`; `register_http_stack` es síncrona, no
`async`. Ambas se conectan como cierres asíncronos en el punto de llamada
(`.http_bootstrap(|| async { bootstrap::register_http_stack() })`) porque
un puntero de función simple también puede servir como punto de entrada
para un arnés de pruebas sin incorporar `async` a la prueba. La
configuración que puede fallar usa `.expect("…")` con un mensaje que
explica cómo resolverlo - el arranque es el momento adecuado para fallar
de forma estrepitosa. La llamada de la aplicación de ejemplo es
`DB::init().await.expect("Failed to connect to database");`, de modo que
una `DATABASE_URL` ausente aborta el proceso en el arranque con el error
real impreso, en lugar de aparecer como un confuso "connection refused"
en la primera solicitud.

## Qué va dentro de bootstrap

Una función `bootstrap` real hace un número reducido de cosas distintas. Las
subsecciones siguientes describen las responsabilidades de bootstrap
disponibles para una aplicación; la aplicación de ejemplo ejercita la mayoría,
pero no todas. Para la inicialización predeterminada actual de Magnetar, usa
como referencia de trabajo la plantilla `src/bootstrap.rs` del andamiaje de la
API.

### Conexión a la base de datos

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` lee `DatabaseConfig` (registrado por `config_fn`) y abre el
pool. La conexión se almacena en el [contenedor](container.md) como un
singleton - `DB::connection()` / `DB::get()` la resuelve desde
cualquier lugar. `DB::init_with(config)` es la vía de escape para
pruebas y herramientas cuando se quiere apuntar a algo distinto de la
URL derivada del entorno.

### Motor de autenticación de Magnetar

Las aplicaciones que usan las fachadas integradas de contraseña,
passkey, enlace mágico, bearer, bloqueo, remember u OAuth inicializan
Magnetar después de que la base de datos y `APP_KEY` estén listos:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

La `MagnetarConfig` predeterminada vincula las identidades de aplicación a la
tabla canónica `app_users`. El andamiaje generado de pila completa usa un
modelo `users` y no inicializa Magnetar, así que no añadas sin cambios el
inicializador predeterminado a ese andamiaje. Usa el modelo `app_users` del
andamiaje de la API, o construye un `MagnetarHostEngine` y una vinculación
`AuthSchema` personalizados para tu tabla `users` existente. Mantén el
`UserProvider` del framework y la vinculación del host Magnetar sobre la misma
identidad de aplicación. El andamiaje de la API, no `app/src/bootstrap.rs`, es
la referencia de trabajo actual para inicializar `MagnetarConfig` de forma
predeterminada.

Magnetar abarca todo el proceso porque los workers de cola, los
planificadores, los handlers HTTP y el middleware de sesión usan los mismos
almacenes de credenciales y sesiones. Coloca `init_magnetar` en `register`, no
en `register_http_stack`. El instalador se ejecuta una sola vez y falla si ya
hay otro motor instalado.

El andamiaje de la API lee `PASSKEY_RP_ID` y `PASSKEY_RP_ORIGIN` en el
arranque de la aplicación. Esos nombres son convenciones del andamiaje,
no variables de entorno propias del framework.
### Middleware global

El middleware global solo se aplica a HTTP, así que pertenece a
`register_http_stack`, no a `register`:

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` registra una capa que se ejecuta en cada solicitud,
incluidas las no enrutadas (404, preflight de OPTIONS). El orden de
registro es el orden en que se ejecuta la cadena - de afuera hacia
adentro. El framework coloca su propio `RequestIdMiddleware` en la
posición más externa; todo lo que se agregue queda dentro de él.
[Middleware](middleware.md) explica la forma completa de la cadena,
incluida la capa por ruta.

### Vinculaciones del contenedor

El contenedor acepta todo lo que se le coloque; las macros son azúcar
sintáctico sobre la fachada [`App`](container.md).

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // Trait → singleton (lo envuelve en Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Singleton concreto:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Fábrica (construida en cada resolución):
    factory!(|| RequestLogger::new());

    // O llama a la fachada directamente para un control más fino:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

Las vinculaciones de objetos de trait son la forma más habitual -
vincular una interfaz y dejar que los handlers y las pruebas
sustituyan la implementación. El capítulo [Contenedor de
servicios](container.md) tiene la API de vinculación completa,
incluidos `bind_factory!`, las variantes `_if_absent` y el modelo de
búsqueda de tres capas.

### Oyentes de eventos y observadores

El despachador está activo tan pronto como se ejecuta bootstrap - los
oyentes registrados aquí ven cada despacho posterior.

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Los observadores de Eloquent (`#[suprnova::observer(M)]`) se recopilan
mediante `inventory::submit!` en tiempo de compilación. Una sola
llamada drena el inventario en el despachador:

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

La llamada es idempotente - volver a ejecutar bootstrap (un worker que
arranca por segunda vez) no registra dos veces los adaptadores de los
oyentes. [Eventos](events.md) cubre el despacho y la escritura de
oyentes; [Eloquent](eloquent.md) cubre los observadores.

### Supervisores

Las tareas en segundo plano de larga duración declaradas mediante el
trait `Supervisor` e `inventory::submit!` arrancan con una sola
llamada:

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Cada supervisor se ejecuta en su propia tarea con bucle de reinicio y
un límite de pánico; un supervisor que entra en pánico se registra en
el log y se reinicia, sin que se le permita derribar el proceso.
Consulta [Supervisores](supervisors.md) para conocer el trait y la
política de reinicio.

### Registro de jobs de worker

Los jobs en cola y los mailables que los workers necesitan despachar
por nombre se registran a sí mismos en el arranque:

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

Sin esto, el worker no tiene forma de asociar un sobre encolado con el
tipo que lo gestiona.

## El gancho posterior al arranque: `booted()`

Bootstrap *registra*; `booted()` *resuelve*. El constructor toma un
segundo callback que se dispara después de que el servidor termina su
propio arranque de servicios, pero antes de que empiece a aceptar
conexiones. Úsalo cuando necesites leer algo que el propio framework
vinculó durante el arranque:

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` es síncrono y se ejecuta después de `Server::from_config` -
los drivers ya están activos, las claves de cifrado ya se cargaron,
tus vinculaciones ya existen. La mayoría de las aplicaciones no
necesita este gancho; recurre a él cuando un efecto secundario de una
sola vez posterior al arranque necesite ver un contenedor ya
completamente construido.

## Un `bootstrap.rs` completo

Esta composición representativa no es un extracto literal de la aplicación
de ejemplo. Mantiene el registro de todo el proceso en `register` y la
configuración exclusiva de HTTP en `register_http_stack`. La inicialización de
Magnetar se muestra por separado arriba porque su esquema de usuarios de
aplicación debe coincidir con el proveedor de usuarios del framework.

```rust
//! Arranque de la aplicación - registra servicios, oyentes, middleware
//! global y la capa de Inertia.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── Base de datos
    DB::init().await.expect("Failed to connect to database");

    // ── Proveedor de autenticación
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());

    // ── Hub de difusión + registro de canales
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Oyentes de eventos + puentes
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Discos de almacenamiento (S3 activado por entorno en producción)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Registro de jobs de worker
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observadores + supervisores
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Indicadores de características
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── Middleware global (de afuera hacia adentro, en orden de registro)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Capa del protocolo Inertia (sin fijar versión: el valor
    // predeterminado obtiene un hash del manifiesto de Vite, por lo que
    // una compilación del frontend actualiza la versión por sí sola;
    // consulta «Detección de versión» en frontend-inertia-responses.md)
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

Observa el ritmo: cada bloque hace una sola cosa, llama a una o dos
APIs, y o bien tiene éxito o falla con un mensaje claro. Nada de esto
es ingenioso; las funciones son largas porque la aplicación tiene muchas
piezas en movimiento, no porque el patrón de bootstrap sea complicado.

## Cuándo usar bootstrap y cuándo `#[injectable]`

`#[injectable]` es una macro que registra automáticamente un singleton
en el `inventory` del contenedor en tiempo de compilación. Es la
opción correcta para servicios que no necesitan nada más que sus
dependencias `#[inject]` para construirse:

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

Estos se resuelven solos; bootstrap no necesita tocarlos.

Bootstrap es el lugar adecuado cuando la construcción necesita
cualquier otra cosa - una variable de entorno, una struct de
configuración ya construida, una vinculación `dyn Trait`, una decisión
de runtime, una llamada de configuración asíncrona, o el registro de
algo que no es en sí mismo un servicio (un oyente, un observador, un
mapeo de job en cola, una capa de middleware global).

| Usa `#[injectable]` para | Usa `bootstrap` para |
|---|---|
| Singletons concretos sin configuración de runtime | Cualquier cosa que sea `dyn Trait` |
| Servicios construidos a partir de otros injectables | Cualquier cosa asíncrona en el arranque |
| Grafo de DI por defecto | Valores derivados del entorno |
| | Oyentes de eventos, observadores, supervisores |
| | Middleware global |
| | Registro de jobs de worker + mailables |

Se pueden mezclar libremente. Los servicios `#[injectable]` ya son
visibles en el contenedor cuando se ejecuta `bootstrap`, de modo que
una vinculación dentro de bootstrap puede leerlos.

## Dónde se ubica bootstrap en el orden de arranque

La secuencia completa (extraída de [Ciclo de vida de la
solicitud](lifecycle.md)):

1. `Config::init(".")` - carga `.env`, detecta el entorno
2. `init_policies()` - drena el inventario de `#[policy]`
3. Se ejecuta tu `config_fn` (registro de configuración tipada)
4. Se ejecutan las migraciones (auto-migración en `serve`)
5. **Se ejecuta tu `bootstrap_fn`** ← `bootstrap::register`
6. **Se ejecuta tu `http_bootstrap_fn`, solo en la ruta del servidor**
   ← `bootstrap::register_http_stack`
7. Se ensamblan las rutas a partir de tu `routes_fn`
8. `Server::from_config` arranca los drivers y el contenedor
9. Se disparan tus `booted_fn`
10. El servidor empieza a aceptar conexiones

Los workers en segundo plano (`queue:work`, `workflow:work`,
`schedule:work`) y el binario de consola comparten los pasos 1-5 y 8:
ejecutan `bootstrap_fn`, pero nunca el paso 6, ya que solo `serve` /
`web:run` ejecuta `http_bootstrap_fn`. Por eso un oyente o un observador
que registres en `register` llega a las rutas de código de los workers
exactamente igual que llega a los handlers HTTP, mientras el middleware
global y `Inertia::install` de `register_http_stack` no se ejecutan en
procesos que nunca sirven HTTP.

### Por qué Suprnova diverge

Laravel ejecuta el `register()` y el `boot()` de cada proveedor de
servicios también para los comandos `artisan` y los workers de cola, no
solo para las solicitudes HTTP. Puede hacerlo porque su integración con
Vite resuelve las URL de los recursos de forma diferida, en el momento
de renderizar, a partir de la directiva Blade `@vite`. Un worker que
nunca renderiza una vista nunca toca el manifiesto, de modo que una
compilación ausente no se detecta.

`Inertia::install` de Suprnova resuelve el manifiesto una vez, durante el
arranque, y en producción falla de forma cerrada cuando falta - por
diseño, para que un despliegue mal configurado no pueda servir URL de
recursos que apunten a un servidor de desarrollo de Vite inexistente.
Esa decisión es precisamente lo que rompe una imagen de worker o de
consola que (correctamente) no incluye `public/assets`: el fallo que
Laravel difiere hasta el momento de la solicitud se produciría en
Suprnova al iniciar el proceso, en cada subcomando. Separar la
superficie de arranque entre `bootstrap` y `http_bootstrap` conserva la
comprobación de fallo cerrado, pero solo donde corresponde: en la ruta
del servidor que realmente renderizará una página de Inertia.

Laravel también divide el propio arranque entre varios proveedores de
servicios: cada uno implementa `register()` y `boot()`, se recopilan en
`config/app.php`, y Laravel los recorre en dos pasadas (primero todos
los `register`, después todos los `boot`) para que un servicio pueda
depender de las vinculaciones de otro proveedor sin una ceremonia de
ordenamiento en el código de la aplicación. La clase del proveedor
proporciona una unidad de organización cuando una aplicación acumula
docenas de subsistemas distintos.

Suprnova reduce eso a dos funciones - `register` y
`register_http_stack` - en lugar de un par `register`/`boot` por cada
proveedor. Las razones son:

- **La división en dos pasadas `register`/`boot` resuelve un problema
  de ordenamiento que Rust no tiene.** `#[injectable]` y
  `bootstrap_singletons` del contenedor ya resuelven los grafos de
  dependencias sin ordenamiento visible para quien escribe el código.
  Las vinculaciones se registran en línea; la maquinaria de búsqueda se
  ocupa del resto.
- **Dos funciones son más fáciles de leer que diez.** Un colaborador
  nuevo abre `bootstrap.rs` y ve cada vinculación, cada oyente, cada
  observador y cada capa de middleware en uno de dos lugares. La
  fragmentación al estilo de proveedores oculta lo que la aplicación
  hace en realidad.
- **El auto-registro al estilo inventario cubre el resto.**
  Observadores, supervisores, tareas programadas, políticas y handlers
  de cola se recopilan a sí mismos en tiempo de compilación mediante
  `inventory::submit!`. Bootstrap drena los inventarios con llamadas
  únicas (`bootstrap_observers`, `SupervisorRegistry::start_all`) en
  lugar de enumerar cada uno.

Donde Laravel se gana la división en proveedores es en la distribución
de bibliotecas: un crate que envía sus propias vinculaciones querría un
punto de entrada de registro al que una aplicación pudiera adherirse
sin editar su propio bootstrap. El análogo de Suprnova es una función
pública `pub async fn register()` en la raíz del crate y una llamada de
una línea desde el `bootstrap` de la aplicación. El costo en ergonomía
es una línea; la ganancia en legibilidad lo es todo en un solo lugar.

## Siguiente

- [Ciclo de vida de la solicitud](lifecycle.md) - el orden de arranque
  completo y dónde se dispara `bootstrap_fn`
- [Contenedor de servicios](container.md) - `App::bind` /
  `App::singleton` / `App::factory` y la búsqueda en tres capas
- [Configuración](configuration.md) - el registro de configuración
  tipada que se ejecuta antes de bootstrap
- [Middleware](middleware.md) - la composición de la cadena para las
  capas registradas con `global_middleware!`
- [Eventos](events.md) - el despachador al que se conectan los oyentes
  y los observadores
