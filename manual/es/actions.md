# Acciones

Una acción en Suprnova es un struct con un único trabajo: mantener una
sola pieza de lógica de negocio detrás de un método. Es el análogo en
Rust de los controladores invocables de una sola acción de Laravel -
`RegisterUser`, `PublishPost`, `ChargeInvoice`. La acción vive en
`src/actions/`, lleva el atributo `#[injectable]` para que el contenedor
pueda resolverla, y expone un método `execute(...)` que los
controladores (y los jobs, y otras acciones) llaman. No existe una
macro `#[action]` ni ninguna imposición por parte del framework de "un
solo método" - la forma es una convención, y `#[injectable]` es la
maquinaria que hace que la convención resulte sencilla.

```rust
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct RegisterUserAction {
    // Inyecta las dependencias como campos - ver "Dependencias" más abajo
}

impl RegisterUserAction {
    pub async fn execute(&self, email: &str) -> Result<String, FrameworkError> {
        tracing::info!(action = "RegisterUser", email, "executed");
        Ok(format!("registered: {email}"))
    }
}
```

Resuélvela desde un handler con `App::resolve::<RegisterUserAction>()?`
y habrás separado tu lógica de dominio de la capa HTTP sin inventar una
clase base de capa de servicio. Ese es todo el patrón.

## Generar una acción

```bash
suprnova make:action RegisterUser
```

La CLI normaliza el nombre a PascalCase, añade `Action` si falta el
sufijo, y luego convierte el nombre de archivo a snake_case. Así:

| `make:action <Name>` | Nombre del struct | Archivo |
|---|---|---|
| `RegisterUser` | `RegisterUserAction` | `src/actions/register_user_action.rs` |
| `SendNotification` | `SendNotificationAction` | `src/actions/send_notification_action.rs` |
| `ProcessPayment` | `ProcessPaymentAction` | `src/actions/process_payment_action.rs` |
| `ChargeInvoiceAction` | `ChargeInvoiceAction` | `src/actions/charge_invoice_action.rs` |

El generador escribe el archivo y añade una línea
`pub mod register_user_action;` a `src/actions/mod.rs`. El stub emitido
compila de inmediato:

```rust
//! register_user_action action

use suprnova::{injectable, FrameworkError};

/// RegisterUserAction
///
/// Single-responsibility command resolved from the container. Inject any
/// dependencies as fields and the `#[injectable]` macro wires them at
/// resolve time.
#[injectable]
pub struct RegisterUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl RegisterUserAction {
    /// Execute the action.
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("RegisterUserAction executed".to_string())
    }
}
```

La firma - `async fn execute(&self) -> Result<_, FrameworkError>` - es
la forma segura para producción: asíncrona, devolviendo un `Result` que
se convierte mediante `?` directamente en un `HttpResponse` en el sitio
de llamada. El cuerpo es un marcador de posición; sustitúyelo por el
flujo de trabajo real.

## El atributo `#[injectable]`

`#[injectable]` es la única pieza de maquinaria del framework de la que
depende el patrón de acción. Se expande en tres cosas:

1. Un `#[derive(Clone)]` sobre el struct (y `Default` cuando no hay
   campos `#[inject]`).
2. Una entrada `inventory::submit!` para que el arranque pueda
   descubrir el tipo.
3. Un closure de auto-registro que `App::singleton_if_absent` ejecuta
   una vez durante `boot_services()`.

El contrato de la macro:

| Forma del struct | Comportamiento |
|---|---|
| Struct unitario (`pub struct Foo;`) | Deriva `Default + Clone`, registra `Default::default()` |
| Campos con nombre, ninguno `#[inject]` | Deriva `Default + Clone`, registra `Default::default()` |
| Campos con nombre con `#[inject]` | Deriva solo `Clone`; cada campo `#[inject]` se resuelve desde el contenedor en el arranque, los campos sin inject usan su valor por defecto |
| Struct de tupla | Rechazado en tiempo de compilación - "use named fields instead" |

Una acción resuelta es un clon del singleton almacenado. El costo es un
`Clone` por cada llamada a `App::resolve::<Action>()?`, que para un
struct unitario o un struct de servicios envueltos en `Arc` es un
puñado de incrementos de contador de referencias. El estado pesado
pertenece detrás de servicios `Arc<dyn …>` que la acción inyecta, no
dentro de la acción misma.

### `#[inject]` ocurre en el arranque, no en cada llamada

Cuando el framework arranca, `App::boot_services()` recorre cada
registro `#[injectable]` y los ejecuta en un bucle de reintento de
punto fijo. Cada entrada intenta resolver sus campos `#[inject]` desde
el contenedor. Si una dependencia todavía no se ha registrado, la
entrada se difiere a la siguiente iteración. El bucle se ejecuta hasta
que todas las entradas tienen éxito o no se hace ningún progreso - y
ante un fallo, el framework devuelve un error estructurado que nombra
el tipo irresoluble o el ciclo.

La consecuencia práctica: **`App::resolve::<MyAction>()` clona el
singleton ya construido**. No ejecuta la resolución de `#[inject]` en
cada llamada. Cualquier cosa injectable de la que dependa una acción
debe estar registrada antes que la acción misma - ya sea mediante su
propio atributo `#[injectable]`, o mediante un `App::bind` /
`App::singleton` manual en tu función `bootstrap()`. El bucle de
reintento se ocupa del orden del inventario por ti; no inventa
servicios faltantes.

## Usar una acción desde un controlador

La forma estándar del handler: resolver, ejecutar, renderizar.

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::register_user_action::RegisterUserAction;

pub async fn store(_req: Request) -> Response {
    let action = App::resolve::<RegisterUserAction>()?;
    let result = action.execute("alice@example.com").await?;

    json_response!({ "ok": true, "result": result }).status(201)
}
```

Ambos puntos `?` funcionan porque ambos tipos de error se convierten en
`HttpResponse` mediante impls de `From` - `App::resolve` devuelve
`Result<T, FrameworkError>` y el conversor de errores del framework se
encarga del resto. Un registro de servicio faltante emerge como un 500
con el nombre del servicio en el registro estructurado, no como un
pánico. Consulta [Modelo de errores](error-model.md) para el panorama
completo.

Si prefieres evitar el `?` en el resolve - por ejemplo, en una ruta que
debería fallar de forma irrecuperable en el arranque -
`App::get::<RegisterUserAction>()` devuelve `Option<T>` y puedes usar
`.expect("registered at boot")` para fallar de forma estrepitosa si
cableaste algo mal.

## Acciones asíncronas que tocan la base de datos

Este es el camino que la mayoría de las acciones toma en realidad -
cargar o escribir a través de un modelo de Eloquent. Extrae el cuerpo
de tu dominio; la superficie es la misma.

```rust
use suprnova::{attrs, injectable, FrameworkError, Model};

use crate::models::todos::Todo;

#[injectable]
pub struct CreateRandomTodoAction;

impl CreateRandomTodoAction {
    pub async fn execute(&self) -> Result<Todo, FrameworkError> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 10000;

        Todo::create(attrs! {
            title: format!("Todo #{}", n),
            description: format!("created at {}", n),
            done: false,
        })
        .await
    }
}

#[injectable]
pub struct ListTodosAction;

impl ListTodosAction {
    pub async fn execute(&self) -> Result<Vec<Todo>, FrameworkError> {
        Ok(<Todo as suprnova::eloquent::Model>::all().await?.into_vec())
    }
}
```

`Todo::create(attrs!{...})` y `Todo::all()` provienen de la macro
`#[suprnova::model]`. Consulta [Eloquent](eloquent.md) para la
superficie del modelo. Ten en cuenta que `Model::all()` devuelve una
`Collection<Todo>` - el ejemplo llama a `.into_vec()` para entregarle
al controlador un `Vec` plano; también puedes devolver la `Collection`
directamente y dejar que el serializador la renderice.

Cableando eso en un controlador:

```rust
use suprnova::{App, Request, Response, ResponseExt, json_response};

use crate::actions::todo_action::{CreateRandomTodoAction, ListTodosAction};

pub async fn create_random(_req: Request) -> Response {
    let action = App::resolve::<CreateRandomTodoAction>()?;
    let todo = action.execute().await?;
    json_response!({ "ok": true, "todo": todo }).status(201)
}

pub async fn list(_req: Request) -> Response {
    let action = App::resolve::<ListTodosAction>()?;
    let todos = action.execute().await?;
    json_response!({ "ok": true, "todos": todos })
}
```

Dos `?` por handler; el controlador se mantiene como un adaptador
delgado entre HTTP y el dominio.

## Dependencias mediante `#[inject]`

Cuando una acción necesita colaboradores - un mailer, un logger, un
servicio de dominio - decláralos como campos y marca cada uno con
`#[inject]`:

```rust
use suprnova::{injectable, FrameworkError};

use crate::services::{MailerService, LoggerService};

#[injectable]
pub struct SendWelcomeEmailAction {
    #[inject]
    mailer: MailerService,
    #[inject]
    logger: LoggerService,
}

impl SendWelcomeEmailAction {
    pub async fn execute(&self, to: &str) -> Result<(), FrameworkError> {
        self.logger.info(&format!("welcome → {to}"));
        self.mailer.send_welcome(to).await
    }
}
```

Tanto `MailerService` como `LoggerService` deben estar registrados en
el contenedor antes de que esta acción arranque - ya sea con su propio
atributo `#[injectable]`, o mediante una llamada en `bootstrap()`:

```rust
// En src/bootstrap.rs
App::singleton(MailerService::from_env()?);
App::singleton(LoggerService::default());
```

Si falta alguna de las dos dependencias cuando el arranque ejecuta el
bucle de punto fijo, el arranque devuelve un error que nombra el tipo
no resuelto y el framework termina con un código de salida distinto de
cero en lugar de arrancar con un contenedor medio cableado.

Los campos que no son `#[inject]` recurren a `Default::default()`, así
que puedes mezclar dependencias inyectadas con estado sencillo sin
escribir un constructor.

## Cuándo usar una acción

La regla general: una acción existe cuando la misma pieza de trabajo se
dispara (o podría dispararse) desde más de un punto de entrada. Un
flujo de registro que se ejecuta tanto desde una ruta HTTP como desde
un job en cola pertenece a `RegisterUserAction`. Un handler puntual de
"renderiza esta página de índice" no necesita una acción - mantenlo en
el controlador.

| Buen ajuste | Ejemplo |
|---|---|
| Operaciones de negocio de varios pasos | `RegisterUserAction`, `CheckoutAction` |
| Trabajo compartido entre HTTP + cola | `IssueRefundAction` (despachada de las dos formas) |
| Lógica que vale la pena probar sin una solicitud | `CalculateTotalsAction` |
| Integraciones externas | `SendEmailAction`, `SyncInventoryAction` |
| Cualquier cosa que el controlador de otro modo pondría en línea + duplicaría | disparador de la regla de tres |

Comparada con un controlador, una acción es reutilizable, no tiene
vinculación con `Request`, y es trivial de llamar desde un test
(`App::resolve` + `await`). Un controlador se mantiene como un límite
consciente de HTTP que sabe traducir el resultado de una acción en un
`Response`.

| Controlador | Acción |
|---|---|
| Maneja una ruta | Reutilizable entre rutas, jobs, tareas programadas |
| Conoce `Request` / `Response` | Conoce tus tipos de dominio |
| Devuelve `Response` | Devuelve `Result<T, FrameworkError>` |
| Llama a acciones | Llamada por controladores (y otros) |

## Acciones, el bus y las colas

Las acciones no son el único lugar donde puede vivir la lógica de
negocio - el [Bus](bus.md) maneja comandos despachados con salidas
tipadas, y la [Cola](queues.md) maneja trabajo que debería ejecutarse
en un worker. Elige según cómo se invoque el trabajo:

| Quieres… | Recurre a |
|---|---|
| Lógica de negocio síncrona, invocable desde un controlador o un job | **Acción** (`#[injectable]` + `execute`) |
| Un comando tipado con un handler registrado, invocable vía `Bus::dispatch` | [Bus](bus.md) |
| Trabajo duradero, con reintentos, en segundo plano | [Cola](queues.md) |

Mezclar está bien: un `BusHandler` o un `Job` a menudo simplemente
resuelve una acción y llama a su `execute`. La acción contiene la
lógica de dominio; el bus o la cola contienen los metadatos de
despacho.

## Layout de archivos

Lo que emite `make:action`, más el espacio para agrupar:

```
src/
├── actions/
│   ├── mod.rs                          // pub mod register_user_action;
│   ├── register_user_action.rs
│   ├── send_welcome_email_action.rs
│   └── billing/                        // agrupa por dominio cuando el directorio crece
│       ├── mod.rs
│       ├── charge_invoice_action.rs
│       └── issue_refund_action.rs
├── controllers/
└── main.rs
```

Nada en el framework exige este layout; el generador escribe en
`src/actions/` porque esa es la convención. Mueve una acción a
`src/billing/actions/` y seguirá funcionando - `#[injectable]` es
agnóstico de la ubicación.

## Probar una acción

Como una acción es solo un struct resoluble por el contenedor con un
método `async`, la superficie de pruebas es `App::resolve` + `await`.
El mismo fixture de pruebas `TestDatabase` usado en otros lugares
funciona aquí:

```rust
use suprnova::{describe, expect, test, App};
use suprnova::testing::TestDatabase;

use crate::actions::todo_action::ListTodosAction;
use crate::models::todos::Todo;

describe!("ListTodosAction", {
    test!("returns all todos", async fn(_db: TestDatabase) {
        Todo::create(suprnova::attrs! { title: "Test", description: "", done: false })
            .await
            .unwrap();

        let action = App::resolve::<ListTodosAction>().unwrap();
        let todos = action.execute().await.unwrap();

        expect!(todos).to_have_length(1);
    });
});
```

Consulta [Pruebas](testing.md) para la superficie completa de
`describe!` / `test!` / `expect!` y para `TestContainer::fake` cuando
quieras inyectar un fake-mailer o fake-gateway en una acción bajo
prueba.

## Por qué Suprnova diverge

Los controladores de una sola acción de Laravel - clases con un método
`__invoke` en `App\Actions\` - se construyen por solicitud. El
contenedor resuelve la clase, ejecuta la inyección por constructor, y
la instancia se descarta cuando la respuesta sale. El modelo de proceso
por solicitud de PHP hace que eso sea esencialmente gratis.

Las acciones de Suprnova son singletons residentes en el contenedor:
construidas una sola vez en el arranque con los campos `#[inject]`
resueltos en ese momento, y clonadas en cada `App::resolve`. El patrón
encaja con Rust porque clonar un struct de servicios envueltos en `Arc`
cuesta unos pocos incrementos de contador de referencias, mientras que
construir y descartar un struct en cada solicitud forzaría cada campo a
pasar por una asignación. La convención con forma de Laravel - un
struct, un método, nombrado según la operación - sobrevive intacta; el
cableado que hay debajo tiene la forma de Tokio.

La otra división intencional: los controladores se mantienen como
funciones libres (consulta [Controladores](controllers.md)), de modo
que la capa HTTP es una transformación pura de solicitud a respuesta
sin superficie de DI propia. La inyección de estilo constructor ocurre
en el límite de `#[injectable]`, dentro de la acción, que es donde
pertenece.

## Siguiente

- [Controladores](controllers.md) - las funciones libres orientadas a HTTP que resuelven y llaman a las acciones
- [Contenedor de servicios](container.md) - qué hacen en realidad `App::resolve`, `App::singleton`, y la búsqueda en tres capas
- [Bus](bus.md) - despacho de comandos tipados cuando quieres un handler registrado en lugar de una acción resuelta
- [Pruebas](testing.md) - `App::resolve` + `TestContainer::fake` para pruebas de acciones herméticas
- [Modelo de errores](error-model.md) - cómo `?` sobre `App::resolve::<Action>()?` y `action.execute().await?` colapsa en una respuesta limpia
