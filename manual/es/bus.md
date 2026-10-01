# Bus

El Bus es el despachador de comandos **síncrono** de Suprnova. Defines un
`Command` tipado (`{ input, Output type }`), registras un `Handler` para él
en el arranque, y luego cualquier código del proceso puede llamar a
`Bus::dispatch(cmd).await` y recibir de vuelta un `Dispatched<T>` que lleva
el resultado tipado del handler.

El Bus se empareja con [`Queue`](queues.md) - su contraparte asíncrona. Son
dos fachadas deliberadamente separadas, no un único despachador que rutea:

| Si quieres…                                          | Usa            |
|-------------------------------------------------------|----------------|
| Ejecutar el trabajo *ahora*, en esta misma tarea, y recibir el resultado de vuelta | `Bus`          |
| Encolar el trabajo para un worker, con reintento ante fallos, de forma durable  | `Queue`        |

Quien llama elige explícitamente. Suprnova no incluye un marcador
`ShouldQueue` - en Tokio ambas rutas son no bloqueantes, así que la
selección explícita es más clara y más rápida que un enrutamiento
implícito.

## Inicio rápido

Diez líneas desde el comando hasta el despacho:

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // el id de cobro que recibimos de vuelta
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// En el arranque (una vez):
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// En un handler de solicitud:
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## Definir comandos

Un `Command` es cualquier struct serializable con un tipo `Output`
asociado y un `command_name()` único:

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

El `Output` es lo que devuelve el handler. Solo necesita ser `Send +
'static` - la ruta de despacho real mantiene los valores nativos vía
`Box<dyn Any>`, sin ida y vuelta por serde. Eso significa que salidas que
no son serde, como `Bytes`, handles opacos, o un `Arc<Mutex<…>>`, hacen
el viaje de ida y vuelta hacia quien llama como valores vivos. La cota
`Serialize + DeserializeOwned` sobre el propio `Command` es para la ruta de
captura del fake: `Bus::fake()` registra cada comando despachado como un
`serde_json::Value`, de modo que las aserciones basadas en predicados
(`assert_dispatched`, `assert_dispatched_times`) puedan decodificarlos e
inspeccionarlos.

`command_name()` debería ser un string estable, único por cada impl
concreta de `Command`. Aparece en los mensajes de fallo de
`assert_dispatched`/`assert_dispatched_times` y en los retornos de error
cuando no hay ningún handler registrado.

## Registrar handlers

Un `Handler<C>` es una función async tipada que toma el comando y
devuelve `Result<C::Output, FrameworkError>`:

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

Llama a `Bus::register::<C, H>(handler)` una vez por tipo de comando en el
arranque. El registro es global; volver a registrar el mismo `C`
sobrescribe el handler anterior (los tests se apoyan en esto para
intercambiar implementaciones) y emite un `tracing::warn!` para que una
vinculación duplicada proveniente de dos registros de servicio en tiempo
de arranque sea visible en el log.

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## Despachar

`Bus::dispatch::<C>(cmd)` ejecuta el handler registrado dentro del proceso
y devuelve un enum `Dispatched<C::Output>`:

```rust
pub enum Dispatched<T> {
    Executed(T),  // el handler se ejecutó, aquí está el resultado
    Captured,    // Bus::fake() estaba activo, el handler NO se ejecutó
}
```

`Dispatched<T>` tiene cuatro ayudantes:

- `.unwrap_executed()` - devuelve el valor, entra en pánico en `Captured`
- `.executed() -> Option<T>` - convierte a `Option`
- `.is_executed()` - predicado booleano
- `.is_captured()` - predicado booleano

Para los sitios de llamada en modo real, `.unwrap_executed()` es la forma
idiomática.

### `Bus::chain` - secuencial

`Bus::chain(Vec<C>)` ejecuta los comandos de uno en uno, deteniéndose en
el primer error (inclusive). Todos los comandos deben ser del mismo tipo.
Devuelve `Vec<Result<Dispatched<C::Output>, FrameworkError>>` - una
entrada por cada comando intentado.

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// Recolecta los ids de cobro exitosos hasta el primer fallo:
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` es homogéneo por diseño - el despachador devuelve
`Dispatched<C::Output>`, que solo está bien tipado cuando todas las
entradas comparten un mismo `Output`. Para cadenas heterogéneas al estilo
Laravel (tipos de job mezclados, donde cada paso dispara el siguiente),
usa [`Queue::chain`](queues.md) - la cola encierra cada job en un sobre
tipado y por eso no tiene la misma restricción.

### `Bus::batch` - concurrente

`Bus::batch(Vec<C>)` ejecuta los comandos de forma concurrente vía
`futures::join_all` y recolecta los resultados en el orden de entrada.
Tiene la misma restricción de tipo homogéneo que `chain`.

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` es homogéneo por la misma razón que `chain`. Para lotes
heterogéneos y persistidos con callbacks de progreso, eventos de ciclo de
vida, y un `BatchRepository`, usa [`Queue::batch`](queues.md).

## Pruebas

Instala el fake al principio del test. `install_fake()` adquiere un mutex
`FAKE_SERIAL` para todo el proceso durante la vida de la guarda, así que
dos tests `Bus::fake()` en paralelo no pueden pisarse el almacén de
capturas el uno al otro - el segundo se bloquea hasta que la primera
guarda se descarta. Aun así debes marcar el test con `#[serial]` si un
test hermano en el mismo binario llama al `Bus::dispatch` real: quien hace
un despacho real no adquiere `FAKE_SERIAL`, así que sin `#[serial]` puede
competir con un test fake en paralelo y observar `is_active() == true`.
`FAKE_SERIAL` elimina el riesgo de fake contra fake, `#[serial]` elimina
el de real contra fake.

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

El fake captura los comandos despachados sin ejecutar sus handlers. Una
llamada a `Bus::dispatch` devuelve `Ok(Dispatched::Captured)` (sin salida
de handler) en lugar de `Executed`. Los errores reales - fallos de
codificación/decodificación, un handler registrado que falta antes de
instalar el fake - siguen apareciendo como `Err(_)`.

`install_fake()` devuelve una `BusFakeGuard`. Descártala (es RAII) y el
fake se limpia y se libera el mutex `FAKE_SERIAL`. La forma habitual es
`let _guard = install_fake();` al principio del test.

### Superficie de aserciones

| Aserción                                            | Verifica…                                                   |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)`                       | al menos un comando de tipo `C` que coincide con `pred`           |
| `assert_not_dispatched::<C>(pred)`                   | cero comandos de tipo `C` que coincidan con `pred`                  |
| `assert_dispatched_times::<C>(pred, count)`          | exactamente `count` comandos de tipo `C` que coincidan con `pred`       |
| `assert_nothing_dispatched()`                        | cero comandos de cualquier tipo despachados bajo el fake activo |

Las cuatro entran en pánico con `Bus::fake() must be active` si no hay
ningún fake instalado. Las que están acotadas por tipo entran en pánico
con `expected … dispatched <command_name> …` cuando la cuenta no coincide.
`assert_nothing_dispatched` entra en pánico con `expected no dispatched
commands but found <n>`.

## Cuándo usar `Queue` en su lugar

Recurre a [`Queue`](queues.md) cuando quieras cualquiera de estas cosas:

- **Durabilidad a través de reinicios.** Un job encolado sobrevive a una
  caída del proceso si el driver es `database` o `redis`.
- **Reintentos con backoff.** El worker de la cola aplica `Job::max_tries` +
  `Job::backoff` (exponencial / fijo / secuencia) en cada fallo.
- **Timeout por job.** `Job::timeout` + `Job::fail_on_timeout` son
  respetados por el bucle del worker.
- **Ejecución retrasada.** `Queue::later(duration, job)` o
  `Queue::push_later(job, at)`.
- **Deduplicación / idempotencia.** `Job::unique_id` + `Queue::push_unique`
  impiden que el mismo job se vuelva a encolar durante un TTL
  configurable.
- **Desacoplar a quien llama del worker.** Ejecuta los jobs en una flota
  separada de workers `cargo run --bin app -- queue:work`.

Recurre a `Bus` cuando quieras cualquiera de estas cosas:

- **Dentro del proceso, ejecución inmediata.** Sin serialización entre
  procesos.
- **Resultado tipado de vuelta a quien llama.** `Dispatched<C::Output>`
  lleva el valor de retorno tipado del handler hasta el sitio de la
  llamada.
- **Composición síncrona.** Un handler de solicitud que descompone el
  trabajo en llamadas `Command` más pequeñas y lee cada resultado en
  secuencia.

Una app típica usa ambos: las rutas de solicitud síncronas despachan
operaciones que devuelven resultado a través de `Bus`, y el trabajo
"dispara y olvida" / durable se encola a través de `Queue`.

## Siguiente

- [Cola](queues.md) - la contraparte asíncrona, drivers, worker, política
  de reintento, cadenas y lotes heterogéneos
- [Eventos](events.md) - despachador pub/sub (un evento → muchos oyentes)
- [Flujos de trabajo](workflows.md) - trabajo con estado y de larga
  duración que sobrevive a reinicios, para cuando una cadena no basta
- [Pruebas](testing.md) - `#[suprnova_test]`, los fakes del contenedor, y
  el patrón de serializador para todo el proceso que usa `Bus::fake()`
