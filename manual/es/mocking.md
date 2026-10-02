# Simulación y falsificaciones

Cada superficie externa de Suprnova viene con un fake dentro del
proceso que captura lo que tu código habría enviado - correo,
notificaciones, jobs encolados, comandos despachados, eventos
disparados, archivos escritos, llamadas HTTP salientes - y el
conjunto de aserciones correspondiente que ejecutas después de los
hechos. La forma siempre es la misma: instala el fake, ejecuta el
código bajo prueba, verifica lo que se capturó. Este capítulo es la
visión consolidada; cada capítulo de subsistema ([Correo](mail.md),
[Notificaciones](notifications.md), [Cola](queues.md),
[Bus](bus.md), [Eventos](events.md), [Almacenamiento de
archivos](filesystem.md), [Cliente HTTP](http-client.md)) cubre su
fake en profundidad.

## Los siete fakes

| Superficie      | Punto de entrada                                  | Estilo de aserción                    | Seguridad en paralelo                              | Capítulo                              |
|-----------------|----------------------------------------------------|---------------------------------------|----------------------------------------------------|--------------------------------------|
| Correo          | `Mail::fake()` → guarda `MailFake`                | métodos sobre la guarda               | necesita `#[serial]` - transporte global, sin serializador | [mail.md](mail.md)                   |
| Notificaciones  | `Notify::fake()` → `NotifyFakeGuard`              | funciones libres en `notifications::testing` | la guarda retiene un serializador para todo el proceso | [notifications.md](notifications.md) |
| Cola            | `suprnova::queue::testing::install_fake()`        | funciones libres en `queue::testing`  | la guarda retiene un serializador para todo el proceso | [queues.md](queues.md)               |
| Bus             | `suprnova::bus::testing::install_fake()`          | funciones libres en `bus::testing`    | la guarda retiene un serializador para todo el proceso | [bus.md](bus.md)                     |
| Eventos         | `EventFacade::fake()` → `EventFakeGuard`          | funciones libres en `events`          | la guarda retiene un serializador para todo el proceso | [events.md](events.md)               |
| Almacenamiento  | `Storage::fake()` → `StorageFakeGuard`            | métodos de `DiskAssertExt` sobre un disco | la guarda retiene un serializador para todo el proceso | [filesystem.md](filesystem.md)       |
| Cliente HTTP    | `Http::fake(\|\| async { … }).await`              | `assert_sent` / `assert_not_sent`     | task-local - verdaderamente concurrente entre tests | [http-client.md](http-client.md)     |

Unos pocos invariantes se mantienen en los siete:

- **El fake registra; el backend real no se ejecuta.** El correo no
  se envía, los jobs no se empujan al driver, los handlers no se
  ejecutan, los eventos se saltan sus oyentes, el HTTP no llega a la
  red, las escrituras de archivo van a un disco en memoria. El lado
  capturado lleva suficiente información para verificar qué habría
  pasado.
- **La guarda es RAII.** Descartarla restaura lo que hubiera antes
  (el transporte de correo anterior, un registro de almacenamiento
  limpio, ninguna grabación para eventos, etc.). Los tests no
  necesitan un paso de desmontaje.
- **El fake no miente sobre los errores.** Si tu código llama a
  `Bus::dispatch` para un comando no registrado, el fake sigue
  devolviendo `Err(_)` - solo se capturan los despachos exitosos.

## Las formas, y por qué difieren

Se repiten tres patrones. Saber qué patrón usa un fake te dice si
debes importar una función libre, llamar a un método sobre la guarda,
o envolver el cuerpo del test en un closure.

### Guarda-con-métodos (Correo)

`Mail::fake()` devuelve un `MailFake` cuyos propios métodos son las
aserciones. Esto es conveniente cuando quien verifica es *el* fake -
ya lo tienes vinculado a una variable local - pero es el único fake
con esta forma:

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### Guarda más funciones libres (Notificaciones, Cola, Bus, Eventos)

La guarda es un token que no hace nada, cuyo único trabajo es
mantener el fake instalado; las aserciones viven en un submódulo
`testing` junto a los internos del fake. Importa lo que necesites:

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

Esta es la forma más común porque generaliza limpiamente entre tipos,
ya que cada aserción es genérica sobre `J: Job` / `C: Command` / `E:
Event` en lugar de estar horneada dentro de un tipo de guarda. El
costo es un import adicional.

Cada push capturado lleva el id de envelope que le asignó el fake, por lo que un test
puede vincular lo que capturó con lo que vio un oyente:

```rust,ignore
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobQueued;
use suprnova::queue::testing::{install_fake, pushed_with_id};

let _queue = install_fake();
let _events = EventFacade::fake();

Queue::push(SendInvoice { order_id: 7 }).await?;

let (job, id) = pushed_with_id::<SendInvoice>().remove(0);
assert_eq!(job.order_id, 7);
assert_eq!(dispatched::<JobQueued>(|_| true)[0].id, id);
```

Bajo el fake no hay driver, así que el propio fake emite el par
`JobQueueing` / `JobQueued` que emitiría un push real, con el id que
registró. `bulk` y `push_unique` no emiten ninguno de estos eventos en la
ruta real, así que el fake tampoco los emite.

### Alcance-con-closure (HTTP)

`Http::fake` es el bicho raro. El HTTP saliente se ejecuta sobre
cualquier tarea de Tokio que esté viva en ese momento, así que el
estado del fake vive en un `tokio::task_local!`. No puedes instalarlo
una vez y dejarlo andar solo - tienes que envolver el cuerpo que
llama al cliente:

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

La recompensa: cada uno de los otros fakes mantiene un serializador
para todo el proceso, así que los tests en paralelo se ejecutan de
uno en uno, pero `Http::fake` es verdaderamente concurrente - cada
test obtiene su propio recorder task-local y nunca chocan entre sí.

### El trait de extensión de Storage

`Storage::fake()` devuelve una guarda *y* un disco en memoria por
defecto, pero sus aserciones cuelgan del disco mismo a través del
trait de extensión `DiskAssertExt`:

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

El trait de extensión está acotado a `#[cfg(any(test, feature =
"testing"))]`, así que el código de producción no puede llamar por
accidente a `disk.assert_exists(…)`.

## Seguridad en paralelo, en un párrafo

Seis de los siete fakes protegen un static global para todo el
proceso. Cada guarda, al construirse, toma un `std::sync::Mutex`
dedicado (`FAKE_SERIAL`) y lo retiene hasta soltarse. El efecto es
que dos `#[tokio::test]` cualesquiera que instalen el mismo fake se
ejecutan en serie dentro de un mismo proceso - sin necesitar
`#[serial]` del crate [serial_test](https://crates.io/crates/serial_test).
**Correo es la excepción**: la guarda `MailFake` intercambia el
`TRANSPORT` global sin tomar un serializador, así que los tests
concurrentes de `Mail::fake()` *sí* se pisarían entre ellos. Márcalos
`#[serial]`. **`Http::fake` también es una excepción**: es
task-local, no global para el proceso, así que los tests corren
verdaderamente en paralelo y nunca necesitan `#[serial]`.

Si entremezclas despacho real con despacho fake para la misma
superficie dentro de un mismo binario de test, la ruta real no toma
el serializador, así que puede competir con un test faked en
paralelo. Marca los tests de despacho real `#[serial]` en ese caso -
la documentación de cada capítulo lo señala donde aplica (consulta
[Bus](bus.md) para el ejemplo canónico).

## Correo - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| Aserción                                  | Verifica…                                            |
|--------------------------------------------|-----------------------------------------------------|
| `fake.assert_sent(\|m\| pred)`             | que al menos un mensaje capturado coincide           |
| `fake.assert_sent_to("…")`                 | que al menos un mensaje capturado se envió a ese correo |
| `fake.assert_not_sent(\|m\| pred)`         | que ningún mensaje capturado coincide                |
| `fake.assert_not_sent_to("…")`             | que ningún mensaje se envió a ese correo             |
| `fake.assert_sent_count(n)`                | que se capturaron exactamente `n` mensajes           |
| `fake.assert_nothing_sent()`               | que no se capturó nada                               |
| `fake.assert_queued("MailableName")`       | que al menos un mailable en cola tiene este nombre   |
| `fake.assert_queued_with(name, \|q\| …)`   | que un mailable en cola coincide con el predicado    |
| `fake.assert_queued_to("…")`               | que un mailable en cola se enrutó a ese correo       |
| `fake.assert_not_queued("MailableName")`   | que no hay ningún mailable en cola con este nombre   |
| `fake.assert_queued_count(n)`              | que hay exactamente `n` mailables en cola            |
| `fake.queued_on("…")`                      | mailables en cola enrutados a una cola                  |
| `fake.assert_queued_on(name, "…")`         | un mailable en cola con este nombre enrutado a una cola |
| `fake.queued_on_connection("…")`           | mailables en cola enrutados a una conexión              |
| `fake.assert_queued_on_connection(name, "…")` | un mailable en cola con este nombre enrutado a una conexión |
| `fake.assert_nothing_queued()`             | que no se encoló nada                                |
| `fake.assert_outgoing_count(n)`            | que enviados + encolados suman `n`                   |
| `fake.assert_nothing_outgoing()`           | que no se envió nada y no se encoló nada             |

`fake.captured()`, `fake.queued()`, `fake.sent(pred)`, `fake.sent_to(…)`,
`fake.queued_named(…)`, y `fake.queued_to(…)` devuelven los datos que
coinciden para que puedas construir aserciones a medida. Consulta
[Correo](mail.md) para la superficie completa, incluido cómo
`Mail::queue` se refleja en el fake incluso cuando `Queue::fake` no
está instalado.

`queued_on_connection` / `assert_queued_on_connection` leen
`QueuedSnapshot::connection`: el override de `.on_connection(...)`,
si lo hubiera, el mismo campo que lee
`Queue::fake` mediante `assert_pushed_on_connection` en la ruta de
job sencillo de abajo, de modo que los dos fakes siguen siendo
simétricos.

## Notificaciones - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| Aserción                                            | Verifica…                                          |
|------------------------------------------------------|---------------------------------------------------|
| `assert_sent(\|r\| pred)`                            | que al menos una notificación despachada coincide  |
| `assert_sent_to(route, "Name")`                      | que la notificación con nombre llegó a esta ruta por canal |
| `assert_sent_to_on(route, channel, "Name")`          | que se despachó por este canal a esta ruta         |
| `assert_sent_named("Name")`                          | que la notificación con nombre se despachó por algún canal |
| `assert_sent_times("Name", n)`                       | que hay exactamente `n` de la notificación con nombre |
| `assert_nothing_sent()`                              | que no se despachó ninguna notificación            |
| `assert_count(n)`                                    | que hay exactamente `n` en total entre todos los tipos y canales |
| `assert_nothing_sent_to(route)`                      | que no se despachó nada a esta ruta                |

`testing::recorded()` devuelve cada `FakeRecord` (nombre de la
notificación, canal, ruta, datos JSON) para aserciones más
detalladas. Los destinatarios de notificación se indexan por el
valor de `route_for` por canal, así que `assert_sent_to` toma el
string de la ruta (una dirección de correo para `"mail"`, el id como
string para `"database"`, …) - consulta
[Notificaciones](notifications.md) para el modelo de enrutamiento.

## Cola - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| Aserción                                        | Verifica…                                                       |
|--------------------------------------------------|----------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)`               | que al menos un push de `J` coincide                               |
| `assert_pushed_later::<J>(\|j, at\| pred)`     | que un push de `J` se programó para `at` (despacho retrasado)      |
| `assert_pushed_on_queue::<J>(queue)`           | que un push de `J` declaró `queue` mediante [`EnvelopeOverrides`](queues.md#per-push-overrides-with-envelopeoverrides) |
| `assert_pushed_on_connection::<J>(connection)` | que un push de `J` declaró `connection` mediante `EnvelopeOverrides` |

El lado de los datos devuelve los jobs tipados mismos:

- `pushed::<J>() -> Vec<J>` - cada push capturado de `J`
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` - lo
  mismo, con el timestamp programado de cada job
- `pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` - lo
  mismo, con los overrides por push declarados de cada job

Cada `Queue::push`, `Queue::push_later`, `Queue::later`,
`Queue::push_unique*`, y los despachadores de cadena/lote confluyen
todos en el mismo recorder. Consulta [Cola](queues.md) para la
semántica de `push_unique` bajo el fake (siempre registra y reporta
"pushed").

Solo `Queue::push_with` y `Queue::later_with` llevan un
`EnvelopeOverrides`, por lo que `pushed_with_overrides` registra
`EnvelopeOverrides::default()` para todos los demás puntos de entrada:
un `Queue::push` simple se interpreta bajo el fake exactamente como «no se
declaró ningún override», igual que si comprobaras que
`entries[0].1 == EnvelopeOverrides::default()`.
`assert_pushed_on_queue` / `assert_pushed_on_connection` comprueban el
override *declarado*, no el nombre de cola o conexión resuelto:
la resolución de `Queue::route` y de `Job::queue` / `Job::connection`
nunca se ejecuta bajo el fake (no hay un push al driver que resolver),
así que un job que en producción pasaría a una ruta o a un valor predeterminado
a nivel de job aparece aquí sin ningún override. Usa
`pushed_with_overrides` directamente para comprobar cualquier otra cosa que
lleve la superposición: `timeout`, `fail_on_timeout`, `max_tries`,
`backoff`.

## Bus - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| Aserción                                           | Verifica…                                                      |
|-----------------------------------------------------|---------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)`                | que al menos un comando despachado de `C` coincide             |
| `assert_not_dispatched::<C>(\|c\| pred)`            | que ningún comando despachado de `C` coincide                  |
| `assert_dispatched_times::<C>(\|c\| pred, n)`       | que exactamente `n` comandos despachados de `C` coinciden      |
| `assert_nothing_dispatched()`                       | que cero comandos de cualquier tipo se despacharon bajo el fake activo |

Bajo el fake, `Bus::dispatch` devuelve `Ok(Dispatched::Captured)` en
lugar de ejecutar el handler. Los fallos reales - errores de
codificación/decodificación, ningún handler registrado antes de
instalar el fake - siguen emergiendo como `Err(_)`. Consulta
[Bus](bus.md).

## Eventos - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Aserción                              | Verifica…                                          |
|----------------------------------------|---------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)`   | que al menos un `E` despachado coincide            |
| `assert_dispatched_once::<E>()`        | que se despachó exactamente un `E`                 |
| `assert_dispatched_times::<E>(n)`      | que se despacharon exactamente `n` de `E`          |
| `assert_not_dispatched::<E>(\|e\| ..)` | que no se despachó ningún `E` que coincida         |
| `assert_nothing_dispatched()`          | que no se despachó ningún evento de ningún tipo    |
| `assert_listening::<E, L>()`           | que se registró un oyente `L` para `E`             |
| `has_dispatched::<E>()`                | `bool`: si se registró algún `E`                   |
| `dispatched::<E>(\|e\| pred)`          | clones `Vec<E>` de los eventos que coinciden       |
| `dispatched_count::<E>(\|e\| pred)`    | la cantidad de eventos que coinciden                |
| `dispatched_events()`                  | `HashMap<&'static str, usize>` de todos los despachos |

Dos variantes acotan lo que se falsea:

```rust,ignore
// Solo falsea estos - todo lo demás se despacha normalmente.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Falsea todos los eventos EXCEPTO estos.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Y una variante suprime sin registrar:

```rust,ignore
EventFacade::muted(async {
    // No se dispara ningún oyente, no se registra ningún evento.
    run_bulk_import().await;
})
.await;
```

`muted` NO adquiere el serializador, así que los alcances muted
pueden ejecutarse en paralelo. Consulta [Eventos](events.md) para la
maquinaria completa, incluido `assert_listening` (que observa los
registros de oyentes que ocurren *dentro* del alcance del fake
únicamente).

## Almacenamiento - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

La guarda preregistra un disco en memoria `"default"`, así que los
tests triviales no necesitan configurar ningún disco. Registra discos
adicionales bajo nombres personalizados con
`Storage::register_memory("audit_logs")` desde dentro del test, si el
código bajo prueba recurre a un disco distinto del predeterminado.

| Aserción                                        | Verifica…                                          |
|--------------------------------------------------|---------------------------------------------------|
| `disk.assert_exists(path).await`                 | que la ruta existe                                 |
| `disk.assert_contents(path, &expected).await`    | que el archivo coincide byte por byte con `expected` |
| `disk.assert_missing(path).await`                | que la ruta no existe                              |
| `disk.assert_count(dir, n, recursive).await`     | que `dir` contiene exactamente `n` entradas        |
| `disk.assert_directory_empty(dir).await`         | que `dir` no tiene entradas (de forma recursiva)   |

Las cinco entran en pánico ante un desajuste, con la ruta del disco en
el mensaje. Consulta [Almacenamiento de archivos](filesystem.md) para
la fachada `Storage` en sí y la historia de drivers (memory / fs / s3
/ azblob / gcs).

## Cliente HTTP - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` encola una
respuesta prefabricada. El método `"*"` coincide con cualquier
método. Cada entrada prefabricada se consume en la primera solicitud
que coincide; las solicitudes coincidentes posteriores o bien caen a
la siguiente entrada prefabricada, o devuelven un `200 {}` vacío.

| Ayudante                                     | Propósito                                                   |
|----------------------------------------------|-----------------------------------------------------------|
| `Http::fake(\|\| async { … }).await`         | instala el alcance de fake task-local                      |
| `fake_response(method, url_substring, …)`    | encola una respuesta prefabricada                          |
| `assert_sent(\|r\| pred)`                    | verifica que al menos una solicitud registrada coincide    |
| `assert_not_sent(\|r\| pred)`                | verifica que ninguna solicitud registrada coincide         |

### Las tareas lanzadas no heredan el fake por defecto

`tokio::spawn` no lleva los task-locals hacia el future lanzado, así
que el trabajo que escapa de la tarea padre también escapa del fake.
Dos herramientas resuelven esto:

```rust,ignore
// Cinturón y tirantes: convierte cada llamada saliente sin fake en un error contundente.
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // Opt-in explícito: este hijo ve el estado del fake del padre.
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` es RAII - instálala al principio de un test y
cualquier llamada saliente que no dé con un fake activo falla en
lugar de tocar la red. `Http::spawn_with_fake_inheritance` es el
opt-in explícito para tareas que deben compartir el estado de fake
del padre. Consulta [Cliente HTTP](http-client.md) para la discusión
completa.

## Difusión

La difusión por WebSocket tiene un fixture de test paralelo, pero su
forma difiere lo suficiente como para vivir en su propio capítulo:
`RecordingBroadcastHub` es un `BroadcastHub` real que registra cada
sobre publicado sin dejar de entregarlo a los suscriptores en vivo.
Vincúlalo en lugar de `InMemoryBroadcastHub` y llama a
`hub.broadcasts()` / `hub.assert_broadcast(channel, event)`. Consulta
[Difusión](broadcasting.md) para el modelo de difusión y el uso del
hub de grabación.

## Dónde vive cada fake

| Superficie    | Fuente                                | Reexport de la fachada                        |
|---------------|---------------------------------------|----------------------------------------------|
| Correo        | `framework/src/mail/mod.rs`           | `suprnova::{Mail, MailFake}`                 |
| Notificaciones | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| Cola          | `framework/src/queue/testing.rs`      | `suprnova::queue::testing::*`                |
| Bus           | `framework/src/bus/testing.rs`        | `suprnova::bus::testing::*`                  |
| Eventos       | `framework/src/events/testing.rs`     | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| Almacenamiento | `framework/src/filesystem/testing.rs` | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP          | `framework/src/http_client/fake.rs`   | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

Los módulos `testing` y `fake` están acotados detrás de una feature
de Cargo llamada `testing`. Está en el conjunto de features por
defecto, así que cualquier test que dependa de `suprnova` obtiene los
ayudantes gratis. Los propios ganchos son `#[doc(hidden)]` donde se
podrían alcanzar por accidente desde código de aplicación; la
salvaguarda que sostiene todo es la validación de `APP_KEY` de
`Server::from_config`, que se ejecuta en cada arranque sin importar
qué ayudantes de test estén compilados. Consulta
[Pruebas](testing.md) para la historia de las compilaciones de
producción.

## Por qué estas formas, no una sola forma

Una única forma uniforme sería más prolija sobre el papel y peor en
la práctica. Cada forma existe porque el estado subyacente tiene una
semántica de concurrencia distinta:

- El transporte **de Correo** es un `Arc<dyn MailTransport>` global
  intercambiado por la guarda. Las aserciones por método sobre la
  guarda devuelta atan quien verifica a la instalación específica, lo
  que hace imposible llamar a las aserciones cuando no hay ningún
  fake activo.
- **Notificaciones / Cola / Bus / Eventos** verifican sobre payloads
  tipados heterogéneos - cada aserción es genérica sobre el tipo de
  evento/job/comando. Las funciones libres en un módulo `testing` se
  componen con parámetros de tipo con más limpieza que un conjunto de
  métodos escrito a mano sobre una guarda.
- Las aserciones de **Almacenamiento** son por disco, no por fake -
  el mismo `disk.assert_exists(…)` funciona contra un disco de
  memoria falseado o contra un disco `s3` real en un conjunto de
  pruebas de integración. Ponerlas sobre el disco vía un trait de
  extensión mantiene esa simetría.
- **HTTP** tiene que seguir a las tareas, no a la pila de llamadas.
  `Http::fake` es el único fake cuyo alcance no se puede expresar
  como una guarda - la semántica de spawn obliga a un closure.

Si alguna vez te encuentras buscando un ayudante que no existe, lee
el capítulo correspondiente; la superficie de test pública está
documentada de forma exhaustiva por subsistema.

## Siguiente

- [Pruebas](testing.md) - la macro `#[suprnova_test]`, `TestDatabase`,
  `expect!`, y `TestContainer::fake`
- [Pruebas HTTP](http-tests.md) - conducir `handle_request`
  directamente sin abrir un socket
- [Pruebas de base de datos](database-testing.md) - la historia de la
  base de datos en memoria por test
- [Contenedor de servicios](container.md) - `TestContainer::fake`
  para intercambiar servicios inyectados
