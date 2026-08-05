# Contenedor de servicios

El contenedor es donde Suprnova mantiene los servicios de la
aplicación - el pool de conexión de la base de datos, el driver de
correo, el `Arc<MyService>` de la aplicación. Los valores se vinculan
en él en tiempo de arranque y se resuelven en handlers y workers. Es
el equivalente en Suprnova del contenedor de servicios de Laravel, con
una diferencia importante: la búsqueda es task-local primero, de modo
que los tests que se ejecutan de forma concurrente no ven las
vinculaciones de los demás.

## Las dos partes

| Tipo | Rol |
|---|---|
| `Container` | El registro subyacente: contiene vinculaciones, fábricas y singletons |
| `App` | La fachada global que realmente se usa - `App::bind`, `App::get`, etc. |

Casi siempre se llama a `App::*` en lugar de construir un `Container`
directamente. El contenedor es la maquinaria interna; la fachada `App`
es la API.

## Orden de búsqueda

Cada llamada a `App::get` / `App::make` comprueba **tres capas** en
orden:

```
        task-local
            │
            ▼  (fallo)
       thread-local
            │
            ▼  (fallo)
          global
            │
            ▼  (fallo)
          None
```

Esto importa porque:

- **El estado por solicitud pasa por task-local** - datos compartidos
  de Inertia, flash bag, request id. Cada solicitud obtiene su propia
  capa, de forma transparente.
- **Los tests usan thread-local** - `let _g = TestContainer::fake();`
  seguido de `TestContainer::bind(...)` vincula dentro de un hilo sin
  tocar el contenedor global, de modo que los tests en paralelo no
  contaminan servicios entre sí. La guarda limpia el contenedor de
  pruebas cuando se descarta (`drop`).
- **Los servicios de toda la aplicación pasan por global** -
  vinculados una vez en el arranque, resueltos en todas partes.

Rara vez importa en qué capa vive una vinculación - `App::bind` la
coloca donde tiene sentido, y `App::get` la encuentra dondequiera que
viva. El modelo solo importa cuando algo se comporta de forma
inesperada bajo concurrencia, y entonces el capítulo
[Pruebas](testing.md) tiene el detalle.

## Vinculación de un valor

Cinco formas de poner algo en el contenedor, según lo que se tenga:

### `App::singleton(value)` - propiedad, clonado en cada búsqueda

Para cualquier valor `T: Any + Send + Sync + 'static` que deba vivir
para siempre. El bound `Clone` está en el *getter* (`App::get`), no en
la vinculación - el valor se almacena una vez dentro de un `Arc` y se
clona a partir de ese `Arc` en cada `get`:

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

El valor se almacena una vez; `App::get::<MyConfig>()` devuelve un
clon. Usa esto para datos con forma de configuración simple, baratos
de clonar.

### `App::bind(Arc<T>)` - para traits y servicios compartidos

Para objetos trait o cualquier cosa que se quiera detrás de un `Arc`:

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` devuelve el clon de `Arc<T>` (un incremento barato
del contador de referencias atómico). Usa esto para cualquier servicio
compartido entre hilos, especialmente objetos trait.

### `App::factory(|| { … })` - construido bajo demanda

Cuando la construcción del valor debe ocurrir en el primer uso (o cada
vez):

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` registra una fábrica de *tipo concreto* (`Fn() -> T`);
`App::bind_factory` registra una fábrica de *objeto trait*
(`Fn() -> Arc<T>`). Ningún closure devuelve `Result` - maneja el fallo
de construcción dentro del closure (pánico en el arranque, o construye
un valor centinela), o usa un `App::singleton` / `App::bind` normal
después de construir el valor con `?`. Ambos invocan el closure fuera
de cualquier bloqueo del contenedor, de modo que una fábrica que
reentra en el contenedor no cae en deadlock, y un constructor costoso
no bloquea otras vinculaciones.

### `App::*_if_absent(value)` - registro amigable con el orden de arranque

A veces un servicio por defecto es registrado por un crate de
servicio, y la aplicación quiere anularlo solo cuando ya está
presente. Las variantes `_if_absent` permiten registrar un valor por
defecto que no sobrescribe una vinculación existente:

```rust
// Dentro de un starter o crate de biblioteca:
App::singleton_if_absent(DefaultMailDriver::new());

// En el bootstrap.rs de la aplicación:
App::singleton(MyCustomMailDriver::new());  // gana porque se ejecutó más tarde
```

`bind_if_absent`, `singleton_if_absent` y las variantes de fábrica
devuelven todas `bool` - `true` si realmente insertaron, `false` si ya
había una vinculación.

## Resolución de un valor

Dos métodos de lectura, más sus contrapartes que devuelven `Result`:

```rust
// Clonar el valor vinculado:
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Clonar el Arc:
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// Lo mismo pero con Result, para la forma idiomática `?` en rutas falibles:
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` y `resolve_make` devuelven `Result<_, FrameworkError>`
(específicamente la variante `ServiceNotFound` cuando falla la
búsqueda) - útil en rutas de handler donde un servicio faltante
debería emerger como un 500 con un registro adecuado, y no como un
pánico.

Comprobaciones de pertenencia (rara vez necesarias):

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## Dónde ocurre la vinculación

El lugar estándar es `src/bootstrap.rs` - una función que se ejecuta
una sola vez en el arranque:

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // Singletons simples
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // Servicios de objeto trait
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // Servicios perezosos (construidos en el primer uso)
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

El nombre de la función `register` coincide con el valor por defecto
del andamiaje (`src/bootstrap.rs::register`); el tipo de retorno es
`()`, no `Result`. Los errores de vinculación que ocurren durante el
arranque (por ejemplo, fallos de conexión de un driver) deben
propagarse a través del constructor del driver/servicio, no desde el
propio `register` - consulta [Arranque de la aplicación](bootstrap.md)
para el cableado de arranque completo.

El framework también llama al propio contenedor durante el arranque:

- `App::init()` se ejecuta primero, inicializando el registro
- `App::boot_services()` resuelve las dependencias en tiempo de
  arranque (drivers, claves de cifrado, etc.) - los servicios de la
  aplicación ven un framework completamente inicializado
- El `bootstrap_fn` de la aplicación se ejecuta después de eso, de
  modo que puede confiar en que los servicios del framework estén
  disponibles

Consulta [Arranque de la aplicación](bootstrap.md) para el orden de
arranque completo.

## Datos compartidos de Inertia

El contenedor es también donde viven los datos compartidos de Inertia.
Tres APIs de conveniencia lo hacen explícito:

```rust
use suprnova::App;

// Valor inmediato (eager) - serializado una vez y reutilizado en cada respuesta de Inertia.
App::inertia_share("appName", "Suprnova");

// Valor perezoso - el resolver se ejecuta por respuesta. Úsalo para datos por solicitud
// que necesitan trabajo asíncrono.
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// Añade una única entrada flash al flash bag por solicitud.
App::flash("message", "Saved!");
```

Esto se lee desde `Container::inertia()`, que devuelve
`&Arc<InertiaRegistry>` - se puede interactuar con él directamente si
se necesita acceso de nivel más bajo. Consulta [Inertia /
Frontend](frontend.md) para ver cómo terminan los datos compartidos en
la respuesta de la página.

## ¿Por qué tres capas?

La cascada task-local → thread-local → global existe por una sola
razón: **aislamiento bajo concurrencia**. Tres cosas se benefician de
esto:

**Aislamiento por solicitud.** El flash bag de Inertia se vincula por
solicitud a través de la capa task-local. Dos solicitudes concurrentes
no ven el flash de la otra porque sus contenedores task-local no se
superponen. La vinculación se evapora cuando termina la tarea de la
solicitud.

**Aislamiento por test.** Un test que vincula un driver de correo fake
no debería ver un fake vinculado por un test hermano.
`TestContainer::fake()` devuelve una guarda thread-local, y
`TestContainer::bind` / `TestContainer::singleton` enrutan las
escrituras hacia el alcance activo. Los tests en paralelo permanecen
herméticos:

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … este test usa FakeMailer
    // un test hermano que se ejecuta en paralelo no lo ve
}
```

Para runtimes de tokio multihilo - donde el future puede migrar entre
hilos de trabajo - usa `TestContainer::scope(async { ... })` en su
lugar; eso instala un override task-local que sobrevive a la
migración.

**Anulación en el arranque.** El código de la aplicación puede anular
los valores por defecto registrados por crates de biblioteca. Las
variantes `_if_absent` junto con la búsqueda en capas permiten que los
crates de biblioteca registren sus valores por defecto de forma
limpia, sin entrar en conflicto con las anulaciones de la aplicación.

## Patrones comunes

### Vincular un struct que contiene el pool de la DB

Esto casi nunca se hace directamente - el framework vincula el pool de
la DB por sí mismo. Pero si existe un subsistema propio con un recurso
compartido costoso:

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// más tarde:
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` devuelve `Option<Arc<T>>` y se combina con `.expect(...)`;
`App::resolve_make` devuelve
`Result<Arc<T>, FrameworkError::ServiceNotFound>` y se combina con `?`
en código falible. Usa el que coincida con la forma de manejar errores
de quien llama.

### Sustituir un valor por defecto por un fake en los tests

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### Construcción perezosa costosa

```rust
// Construye el modelo de embeddings en la primera solicitud, no en el arranque.
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

Para la construcción falible que necesita mostrarle un error
estructurado al operador, construye el valor en `bootstrap()` con `?`
y llama a `App::bind(...)` una vez que esté listo.

## Por qué Suprnova diverge

El contenedor de Laravel tiene un único alcance global - las
vinculaciones son globales, y aislar entre tests requiere la
disciplina de `setUp` / `tearDown` más la transacción de base de datos
por test del framework. El modelo de solicitud-por-proceso de PHP hace
esto seguro por accidente: un proceso nuevo por solicitud significa
que el contenedor se reinicia cada vez.

El modelo de procesos de Rust es lo opuesto - un solo proceso atiende
muchas solicitudes concurrentes en muchos hilos. Un contenedor
solo-global significaría que un test en un hilo podría ver un fake
vinculado por otro, o que una solicitud podría ver los datos por
solicitud de otra solicitud. Por eso Suprnova tiene la cascada de tres
capas: task-local para lo que es por solicitud, thread-local para lo
que es por test, global para lo que es de toda la aplicación.

La API del contenedor es la misma que la de Laravel; la maquinaria de
búsqueda es diferente porque el runtime es diferente.

## Siguiente

- [Arranque de la aplicación](bootstrap.md) - dónde va el código de
  vinculación
- [Configuración](configuration.md) - registro de configuración tipada
  junto con los servicios
- [Pruebas](testing.md) - `TestContainer::fake` y `#[suprnova_test]`
- [Política de bloqueos](lock-policy.md) - por qué importa la
  recuperación de bloqueos envenenados en una aplicación respaldada
  por contenedor
