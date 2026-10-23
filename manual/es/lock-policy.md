# Política de bloqueos

Suprnova es un único proceso Tokio de larga duración, no una flota de
workers PHP de corta duración. Todo registro global del proceso,
singleton y caché compartida que se vincule en el arranque sobrevive a
cada solicitud que lo toca. Eso cambia una cosa pequeña pero
consecuente en la forma de recurrir a `std::sync::Mutex` y a
`std::sync::RwLock`: un pánico mientras se sostiene una guarda
*envenena el bloqueo* durante el resto de la vida del proceso, y quien
llame después tiene que decidir qué hacer al respecto. Este capítulo
es la política de todo el proyecto para esa decisión - dos patrones
autorizados, cuándo elegir cada uno, y por qué nunca se debe recurrir
a un `.lock().unwrap()` sin procesar en el código del framework o de
la aplicación.

## Por qué existe este capítulo

En Laravel nunca había que pensar en bloqueos envenenados porque no
existían. PHP no comparte nada: un error fatal derriba el proceso de
una solicitud, la siguiente solicitud arranca en uno nuevo, y ningún
estado en memoria sobrevive para corromper nada. Suprnova funciona al
revés. El proceso arranca una sola vez, los registros se van poblando,
y permanecen vivos durante toda la vida del binario. Un handler que
entra en pánico mientras sostiene una guarda de escritura sobre un
`RwLock` global del proceso deja ese bloqueo *envenenado* - cada
`.read()` y `.write()` posterior devuelve `Err(PoisonError)` para
siempre, a menos que alguien lo recupere de forma explícita.

La expresión idiomática por defecto de Rust - `.lock().unwrap()` -
convierte ese `Err` en un pánico. Que luego se convierte en otro
bloqueo envenenado en algún punto más arriba de la pila. Que luego
derriba el siguiente subsistema que lo toque. Una sola solicitud
defectuosa termina en cascada convertida en un proceso medio muerto.

La política que sigue evita esa cascada.

> **Alcance.** Esto se aplica a `std::sync::Mutex` y `std::sync::RwLock`, que llevan estado de envenenamiento. Los primos asíncronos en `tokio::sync` (`Mutex`, `RwLock`, `Semaphore`) *no* se envenenan - un pánico mientras se sostiene una guarda `tokio::sync::Mutex` suelta la guarda de forma limpia y el siguiente `.lock().await` tiene éxito. Si tu ruta de ejecución frecuente es asíncrona y no necesitas adquirir la guarda desde un contexto síncrono (una implementación de `Drop`, un callback del framework, un subcomando de la CLI), prefiere las variantes de Tokio y la pregunta desaparece.

## Los dos patrones autorizados

Cada lugar del framework que sostiene un bloqueo `std::sync` usa
exactamente uno de dos patrones. Elige de la misma manera en tu propio
código.

### Patrón 1 - Mapear el envenenamiento a un error de retorno

Cuando quien llama ya devuelve `Result<_, E>` y un `?` más no cambia
su forma, se expone el envenenamiento como un error y se deja que la
solicitud falle limpiamente. El framework usa ayudantes internos
`pub(crate)` (`lock::read`, `lock::write`, `lock::lock`) que mapean
una guarda envenenada a
`FrameworkError::internal("<context> lock poisoned")`, incrustando una
etiqueta suministrada por quien llama para que los registros puedan
indicar qué subsistema se envenenó sin que cada sitio de llamada tenga
que envolver el error por su cuenta.

El patrón que codifican esos ayudantes es lo bastante corto como para
escribirse en línea en el código de la aplicación:

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use suprnova::FrameworkError;

static FEATURE_FLAGS: RwLock<HashMap<String, bool>> = RwLock::new(HashMap::new());

pub fn enable(flag: &str) -> Result<(), FrameworkError> {
    let mut guard = FEATURE_FLAGS
        .write()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    guard.insert(flag.to_string(), true);
    Ok(())
}

pub fn is_enabled(flag: &str) -> Result<bool, FrameworkError> {
    let guard = FEATURE_FLAGS
        .read()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    Ok(guard.get(flag).copied().unwrap_or(false))
}
```

Dentro de un handler, `is_enabled(...)?` se colapsa a través de la
misma ruta `FrameworkError → HttpResponse` que usa cualquier otro
error del framework: el cliente recibe un 500 saneado con
`{"message": "Internal Server Error"}`, el log estructurado captura el
mensaje de envenenamiento etiquetado, el id de la solicitud se
conserva de principio a fin, y el resto del proceso sigue sirviendo
tráfico. Consulta el capítulo [Manejo de errores](errors.md) para ver
la ruta de conversión completa.

Usa este patrón cuando:

- Quien llama ya devuelve `Result` (la mayoría de las operaciones que pueden fallar lo hacen).
- Un bloqueo envenenado representa un fallo real e irrecuperable del subsistema - no hay ninguna "verdad parcial" sensata a la que recurrir.
- Interesa que los operadores *vean* el envenenamiento en los registros la próxima vez que se toque el subsistema. El mensaje etiquetado es la pista forense que queda.

El despachador de notificaciones del framework, el transporte de
correo, el registro de mailables, los oyentes de eventos de la base de
datos y el registro de conexiones con nombre usan todos este patrón.
Un pánico en cualquiera de ellos emerge como un 500 en la siguiente
solicitud que toque el registro; todo lo demás sigue funcionando.

### Patrón 2 - Recuperar en el sitio con `into_inner()`

Cuando la firma de quien llama *no* puede fallar (una consulta que
devuelve `bool`, una comprobación de enrutamiento en una ruta de
ejecución frecuente, una ruta de la que depende el ciclo de vida de la
solicitud) o cuando el estado compartido es estructuralmente seguro de
usar después de una escritura parcial, se recupera la guarda y se
continúa:

```rust
use std::collections::HashMap;
use std::sync::RwLock;

static ALLOWED_INCLUDES: RwLock<HashMap<&'static str, Vec<&'static str>>> =
    RwLock::new(HashMap::new());

pub fn allows(dto: &str, field: &str) -> bool {
    ALLOWED_INCLUDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(dto)
        .map(|fields| fields.contains(&field))
        .unwrap_or(false)
}

pub fn register(dto: &'static str, fields: &'static [&'static str]) {
    let mut guard = ALLOWED_INCLUDES
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(dto, fields.to_vec());
}
```

`PoisonError::into_inner()` devuelve la guarda a pesar del
envenenamiento. Las lecturas y escrituras posteriores prosiguen con
normalidad - el bloqueo sigue marcado como envenenado para las
consultas `is_poisoned()`, pero el flujo de datos queda restablecido.

El framework usa este patrón en `data::registry` (la lista de
inclusiones permitidas que se lee en cada respuesta JSON:API),
`auth::manager` (el mapa de proveedores de autenticación con nombre),
`app::paths` (la caché de rutas resueltas), los fakes de pruebas para
correo y eventos, y el mapa de claves de entorno cargadas en
configuración. En todos los casos, o bien ninguna persona que llama
tiene un `Result` que devolver, o el estado es de solo apéndice y es
estructuralmente seguro seguir usándolo.

Usa este patrón cuando:

- La firma de quien llama es simple (`bool`, `&str`, un clon de un valor almacenado), y cambiarla a `Result` obligaría a propagar el error en cada sitio de llamada - a veces en cada subsistema del framework.
- El estado compartido puede tolerar una escritura parcial. Los mapas de solo apéndice y las cachés son la forma típica: el peor caso es una entrada faltante o desactualizada, algo que quien llama ya maneja (denegar por defecto, recurrir al recurso principal, recalcular).
- La ruta de ejecución frecuente se recorre con tanta asiduidad que devolver un error en cada solicitud posterior sería, en términos operativos, peor que degradarse.

## Cómo elegir entre ellos

La regla de decisión, en una sola frase: **si el peor caso de usar el
estado tras el envenenamiento es una respuesta incorrecta con
consecuencias, se mapea a un error; si es una entrada faltante o
desactualizada que quien llama ya maneja, se recupera en el sitio.**

El razonamiento, paso a paso:

1. **¿La firma de quien llama es `Result<_, E>`?** Si no lo es, hay que recuperarse en el sitio - añadir `Result` a un `bool` suele ser una refactorización de todo el proyecto, y no vale la pena solo por un caso extremo de envenenamiento.
2. **Si se observara un valor escrito solo a medias, ¿la aplicación tomaría una decisión equivocada con consecuencias reales?** Cobrarle a un cliente equivocado, permitir una inclusión no autorizada, conceder acceso al tenant equivocado - eso es "sí, se mapea a un error". Devolver `false` a "¿está registrado este nombre?" y recurrir al pool principal - eso es "no, se recupera en el sitio".
3. **¿El estado es de solo apéndice o naturalmente idempotente ante un nuevo registro?** Si la respuesta es sí, recuperarse en el sitio es seguro. Si una escritura es una transición de máquina de estados que depende del valor anterior, es preferible mapear a un error para no agravar una corrupción.

Ante la duda, se mapea a un error. Una solicitud que devuelve 500 es
una señal evidente que se puede arreglar; las respuestas incorrectas
silenciosas no lo son.

## Nunca uses `.lock().unwrap()`

La forma prohibida:

```rust
// NUNCA - un solo pánico en cualquier punto del grafo de llamadas
// por debajo de esta línea envenena el bloqueo, y quien llame
// después convierte el envenenamiento en otro pánico.
let mut guard = SOMETHING.lock().unwrap();
```

`.expect("…")` es lo mismo con un mensaje más agradable. Ambos
convierten un `Err` de bloqueo envenenado en un pánico que la red
`AssertUnwindSafe(...).catch_unwind()` del ciclo de vida de la
solicitud atrapa y convierte en un 500 - esa red es una *última línea
de defensa*, no una licencia para saltarse la decisión anterior. Las
APIs públicas del framework y el código de la aplicación deben elegir
uno de los dos patrones autorizados.

Las dos excepciones en las que `.unwrap()` es aceptable sobre un
bloqueo `std::sync`:

- **Configuración de pruebas que busca *comprobar* que se llegó a envenenar el bloqueo** - el propio ayudante de inducción de envenenamiento de `framework/src/lock.rs` usa `.unwrap()` dentro del hilo que entra en pánico, y lo hace a propósito.
- **La ruta de error de una operación de envenenamiento que ya falló** - para cuando el código está dentro del hilo de `poison_rw(...)`, el pánico *es* precisamente el objetivo.

Si no estás en uno de esos casos, elige un patrón de la sección
anterior.

## ¿Y si mi función devuelve `bool`?

Esta es justo la situación en la que vive `ConnectionRegistry::has`.
Es una consulta `bool` en la ruta de ejecución frecuente del
enrutamiento de réplicas de lectura del ejecutor, invocada en línea
como `if ConnectionRegistry::has("read_replica").await { … }`.
Ampliarla a `Result<bool, FrameworkError>` obligaría a que quien llama
dentro del ejecutor propague el error con `?`, metiendo una ruta de
código de error interno en decisiones de enrutamiento que solo
necesitan un sí o un no.

El patrón de recuperación en el sitio resuelve esto - se devuelve
`false` y se deja que entre en acción la lógica de repliegue de quien
llama (aquí, el ejecutor vuelve al pool principal, que de todos modos
es el comportamiento seguro). Para asegurarse de que los operadores
sigan viendo la condición, se emite un `tracing::warn!` de una sola
vez la primera vez que se observa el envenenamiento:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;

static REGISTRY: RwLock<HashMap<String, ()>> = RwLock::new(HashMap::new());
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub fn has(name: &str) -> bool {
    match REGISTRY.read() {
        Ok(g) => g.contains_key(name),
        Err(_) => {
            // Seguro ante carreras: solo el primer observador registra.
            if !POISON_WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(
                    target: "myapp::registry",
                    "registry lock poisoned - `has({name})` degrading to false",
                );
            }
            false
        }
    }
}
```

La compuerta basada en `swap` importa: el envenenamiento de `RwLock`
es persistente, así que sin la compuerta cada llamada posterior
volvería a disparar la advertencia e inundaría los registros. Con la
compuerta, se obtiene exactamente una advertencia por proceso y por
registro, y un getter correspondiente que devuelve `Result` (`get`,
`register`) en el mismo registro expondrá el envenenamiento la próxima
vez que algo *realmente necesite* que la consulta tenga éxito. Eso da
a los operadores dos señales: una advertencia temprana de "algo anda
mal", y un 500 contundente en el momento en que una solicitud
realmente dependía del registro.

## Lo que el framework ya protege

No hace falta aplicar esta política a ningún estado que ya posea el
framework - eso ya está resuelto. En concreto:

- El registro de conexiones con nombre (`ConnectionRegistry::register`, `get`, `has`) mapea el envenenamiento a `FrameworkError::internal` en las escrituras y en las lecturas que devuelven `Result`; `has` se degrada a `false` mediante la compuerta de advertencia única.
- El despachador de notificaciones y el registro de fábricas, el registro de mailables, el transporte de correo, la captura en memoria de correo y los oyentes de eventos de la base de datos devuelven todos `FrameworkError::internal` ante el envenenamiento.
- La lista de inclusiones permitidas de `data::registry`, el mapa de proveedores de `auth::manager`, `app::paths`, la caché de claves de entorno cargadas y los fakes de pruebas en memoria se recuperan todos en el sitio.

Allí donde se llega a estos subsistemas a través de su API pública
(`Notification::send`, `Mail::send`, `Auth::user`, `DB::connection`,
la ruta de respuesta JSON:API), un bloqueo envenenado del framework
emerge como un 500 limpio - nunca como un pánico en el sitio de
llamada.

## Por qué Suprnova diverge

Laravel no tiene una política de bloqueos porque no tiene estado
compartido de larga duración. Cada solicitud PHP obtiene su propio
proceso, su propia memoria, sus propias copias de cada singleton. No
hay ningún registro en memoria que envenenar, ni existe el concepto de
que "la siguiente solicitud" herede el daño de la anterior - el
runtime garantiza siempre una pizarra limpia.

Suprnova está construido sobre Tokio, que ofrece precisamente ese
estado compartido de larga duración que PHP descarta. WebSockets
económicos, cachés en memoria, pools de conexiones que no hay que
pagar por reconstruir - todo esto necesita registros globales del
proceso que sobrevivan a cualquier solicitud individual. Esa capacidad
es justo el motivo de pasarse a Rust para este estilo de aplicación
(consulta la [introducción](introduction.md) para conocer la
motivación completa del framework). El costo de tenerla es que ahora
hay que pensar en qué ocurre cuando un hilo que entra en pánico deja
el estado compartido en una condición bloqueada, porque *sí* hay
estado compartido que dejar así.

La política de dos patrones es la respuesta más pequeña que conserva
la capacidad y elimina el costo. Se recupera en el sitio donde el
estado es seguro de seguir usando; se mapea a un error donde sea
preferible un 500 limpio a una respuesta incorrecta. Ambas opciones
dejan al resto del proceso sirviendo tráfico. Ninguna de las dos deja
un unwrap en pánico esperando para derribar el subsistema de más
arriba.

Esta es la misma forma que adopta la [decisión de fail-open frente a
fail-closed](rate-limiting.md) que el framework aplica a las cachés y
a los backends de limitación de velocidad inalcanzables: una elección
explícita de política en el sitio de llamada, no un valor por defecto.
El hecho de que todo sea asíncrono da estado de larga duración; el
framework da el manual de juego para mantenerlo honesto.

## Siguiente

- [Manejo de errores](errors.md) - cómo `FrameworkError::internal` se convierte en el 500 saneado que recibe el cliente, conservando el mensaje de envenenamiento etiquetado en el log estructurado.
- [Contenedor de servicios](container.md) - dónde viven realmente los registros globales del proceso que esta política protege, y por qué el alcance local a la tarea o al hilo evita que las pruebas hereden las vinculaciones unas de otras.
- [Ciclo de vida de la solicitud](lifecycle.md) - el límite de pánico (`execute_chain_safely`) que atrapa el unwrap de *último recurso* y lo convierte en un 500, para entender exactamente qué hace esta red de seguridad y por qué no es una excusa para saltarse la política anterior.
- [Limitación de velocidad](rate-limiting.md) - la historia paralela de `BackendErrorPolicy` para backends que pueden quedar *inalcanzables* en lugar de envenenados; el mismo principio de elección explícita, con un modo de fallo distinto.
- [Pruebas](testing.md) - cómo `TestContainer::fake` y la capa de contenedor local al hilo evitan que las pruebas en paralelo contaminen sus registros mutuamente, lo cual es el complemento en tiempo de prueba de todo lo relacionado con el manejo del envenenamiento.
