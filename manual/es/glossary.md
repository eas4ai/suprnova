# Glosario

Términos específicos de Suprnova, definidos una sola vez. Si un
capítulo usa una palabra sin explicarla, la definición vive aquí. Las
entradas están en orden alfabético; sigue el enlace cruzado hacia el
capítulo que usa el término en contexto.

Un puñado de convenciones a tener en cuenta mientras lees el resto de
esta lista:

- **Trait** significa un trait de Rust - un contrato de comportamiento
  que implementas sobre un tipo. **Fachada** significa un struct de
  tamaño cero cuyos métodos estáticos son el punto de entrada a un
  subsistema (`Cache`, `Mail`, `Auth`, `Storage`, `Bus`, `Notify`,
  `Vector`, `DB`, `Schedule`, `App`).
- **Driver** significa un backend intercambiable detrás de una fachada
  o un registro - `CacheStore`, `QueueDriver`, `VectorDriver`,
  `RateLimiterDriver`, `MailDriver`. Los drivers se eligen al arrancar
  vía variables de entorno y se vinculan a través del contenedor.
- **Registro** significa una búsqueda global del proceso, poblada en
  tiempo de compilación vía `inventory` o al arrancar vía registro
  explícito - `ConnectionRegistry`, `MiddlewareRegistry`,
  `InertiaRegistry`, `ChannelRegistry`, `VectorRegistry`,
  `SupervisorRegistry`, `PaymentProviderRegistry`, `ScopeRegistry`.

## A

### Accesor

Una transformación de lectura declarada en un modelo de Eloquent con
la macro `#[accessor]`. Se ejecuta cada vez que se lee la propiedad y
devuelve un valor calculado derivado de una o más columnas subyacentes
(`full_name` a partir de `first_name + last_name`, por ejemplo). El
dual de un [Mutador](#mutador). Ver
[Eloquent - Accesores y mutadores](eloquent.md#accessors-and-mutators).

### Acción

Una clase de servicio inyectable que encapsula una pieza de lógica de
negocio - un único método público, con dependencias inyectadas vía la
macro `#[injectable]`. El análogo en Suprnova de los invocables de una
sola acción de Laravel. Las acciones se vinculan como singletons en el
contenedor automáticamente y las resuelven handlers, jobs, y otras
acciones. Ver [Acciones](actions.md).

### Aplicación

El builder fluido en `Application::new()` que registra tus funciones
de config, bootstrap, rutas y migraciones, y luego llama a `.run()`
para despachar el subcomando de CLI del binario (`serve`, `migrate`,
`queue:work`, etc.). Uno por binario, vive en `src/app.rs`. Ver
[Ciclo de vida de la solicitud](lifecycle.md).

### Contador atómico

Una operación de caché (`Cache::increment`, `Cache::decrement`) que
muta un valor numérico en un único round-trip sin condiciones de
carrera de lectura-modificación-escritura. Respaldada por `INCR`/`DECR`
de Redis en el store de Redis, y por una guarda sostenida en el store
en memoria. Ver [Caché - Contadores atómicos](cache.md#atomic-counters).

### Authenticatable

El trait que implementa un tipo de usuario autenticado
(`get_auth_identifier() -> String`, `get_auth_password()`, etc.) para
que los guards y el middleware puedan hablar con él sin conocer el
struct de usuario concreto. Ver [Autenticación](authentication.md).

### Authorizable

El trait que le da a un tipo de usuario los puntos de entrada a las
policies (`can`, `can_any`, `cannot`) que usa la [Compuerta](#compuerta).
Ver [Autorización](authorization.md).

## B

### Esquema de backoff

La secuencia de retardos que un worker de cola espera entre reintentos
de un job que falla. `BackoffSchedule::linear`, `BackoffSchedule::exponential`,
o un `Vec<Duration>` personalizado. Ver [Cola - Esquemas de backoff](queues.md#backoff-schedules).

### Batch (cola)

Un grupo de jobs despachados juntos y rastreados como una unidad -
`PendingBatch::new().add(job).add(other).dispatch()` devuelve el id
del batch persistido. Útil cuando quieres repartir trabajo y ejecutar
un callback cuando el batch completo termine. Ver [Cola - Batches encolados](queues.md#queued-batches).

### `BelongsTo`

El tipo de relación inversa a `HasOne`/`HasMany` - el hijo lleva la
clave foránea, el padre está del otro lado. Uno de los once tipos de
relación de Eloquent. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### `BelongsToMany`

Un tipo de relación muchos a muchos que pasa por un tercer modelo, de
primera clase, [Pivot](#pivot). `BelongsToMany<Local, Related, Pivot>` -
el pivot se nombra en el tipo, no se sintetiza por convención de
strings. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Arranque de la aplicación

El `bootstrap_fn` que registras en el builder `Application` y que se
ejecuta una vez al arrancar (después de la config, antes de servir).
Aquí es donde vinculas servicios en el [Contenedor](#contenedor),
registras observers y oyentes de eventos, configuras encabezados por
defecto, etc. El análogo en Suprnova de los service providers de
Laravel, colapsado en una sola función. Ver
[Arranque de la aplicación](bootstrap.md).

### Broadcastable

El trait que implementa un [Evento](#evento) cuando debe empujarse a
los suscriptores de WebSocket, en lugar de (o además de) los oyentes
locales en el mismo proceso. El puente entre el dispatcher de eventos y
el [Broadcast Hub](#broadcasthub). Ver [Difusión](broadcasting.md).

### `BroadcastHub`

El trait que nombra "lo que reparte un mensaje a todos los
suscriptores de WebSocket de un canal" - la implementación en memoria
(`InMemoryBroadcastHub`) es la predeterminada; la implementación de
sea-streamer (`SeaStreamerBroadcastHub`) es la de despliegue en
producción multiproceso. Ver [Difusión - Fanout multiproceso](broadcasting.md#multi-process-fanout).

### Builder (Eloquent)

El objeto de consulta fluido que devuelve `Model::query()` - la
superficie encadenable donde construyes `where`, `order_by`, `with`,
`limit`, etc. antes de `.get()`, `.first()`, o `.paginate(...)`. Con
doble nombre: cada método de filtro existe tanto bajo su nombre de
Laravel (`db_where`, `db_or_where`) como bajo su sinónimo nativo de
Rust (`filter`, `or_filter`).
Ver [Eloquent - Constructor de consultas](eloquent.md#query-builder--dual-api).

### Comando de Bus

Un struct serializable despachado a través de `Bus::dispatch(cmd)` que
se enruta a un único `Handler<C>` registrado. Los comandos de Bus son
para trabajo en el mismo proceso cuyo resultado debe propagarse de
vuelta a quien llama - los [Job](#job) de cola son para trabajo que
debe persistirse y reintentarse en segundo plano. Ver [Bus](bus.md).

## C

### Driver de caché

El backend seleccionado (`memory` o `redis`) detrás de la fachada
`Cache`. Se elige al arrancar vía `CACHE_DRIVER` y se expone a través
del trait [CacheStore](#cachestore). Ver [Caché](cache.md).

### `CacheStore`

El trait que define la SPI del driver de caché - `get`, `put`,
`forget`, `increment`, etc. `InMemoryCache` y `RedisCache` son las
implementaciones que se ofrecen. Ver [Caché - Configuración](cache.md#configuration).

### Cast (Eloquent)

Una transformación bidireccional declarada con `casts!` en un modelo
de Eloquent - tipo de columna de BD ↔ tipo de Rust. Se incluyen 22
integrados (`AsBool`, `AsDateTime`, `AsJson`, `AsEncrypted`, `AsArray`,
etc.); un trait `Cast` implementado por el usuario cubre cualquier otro
caso. Ver
[Eloquent - Casts](eloquent.md#casts).

### Cadena (cola)

Una secuencia de [Job](#job) enlazados de modo que cada uno se ejecuta
solo si el anterior tuvo éxito. Se construye con `PendingChain::dispatch`
/ `Queue::chain`. Ver [Cola - Cadenas encoladas](queues.md#queued-chains).

### Canal (difusión)

El trait al que difunde un evento - `PublicChannel`, `PrivateChannel`,
o `PresenceChannel`. El struct del canal se nombra a sí mismo
(`fn name() -> String`) y autoriza la conexión
(`fn authorize(...)`); los canales privados y de presencia añaden
límites de trait más estrictos. Ver [Difusión - Canales](broadcasting.md#channels).

### Canal (notificación)

El trait que enruta una [Notification](#notification) a un mecanismo
de entrega - correo, base de datos, difusión, web push. Una
notificación nombra sus canales en `fn via(...)`; cada canal resuelve
el destino y envía. Distinto del trait de difusión del mismo nombre.
Ver [Notificaciones - Canales](notifications.md#channels).

### Contenedor

El registro de tres capas (task-local → thread-local → global) donde
se vinculan y resuelven los servicios a través de la fachada `App`. El
análogo en Suprnova del contenedor de servicios de Laravel, con capas
adicionales para el aislamiento por solicitud y por test. Ver
[Contenedor de servicios](container.md).

### Contexto (por solicitud)

La bolsa de valores tipados por solicitud, alcanzable desde cualquier
código en la misma tarea async - `Context::set::<T>(value)`,
`Context::get::<T>()`. Sobrevive a los task spawns cuando lo propagas
explícitamente. Distinto del contexto de feature flags que comparte el
nombre. Ver [Contexto](context.md).

### CORS

Cross-Origin Resource Sharing. La regla de seguridad del navegador que
condiciona un fetch de JavaScript desde el origen A hacia el origen B;
Suprnova ofrece `CorsMiddleware` para emitir los encabezados de
respuesta que señalan qué solicitudes cross-origin están permitidas.
Ver [CORS](cors.md).

### CSRF

Cross-Site Request Forgery. El ataque contra el que tiene que
defenderse una sesión con estado; Suprnova ofrece `CsrfMiddleware`
para exigir un token coincidente en cada solicitud que cambia estado.
Ver [Protección CSRF](csrf.md).

## D

### Fachada `DB`

El punto de entrada a la base de datos sin modelo - `DB::table(...)`,
`DB::transaction(...)`, `DB::raw(...)`. Para consultas que no encajan
en la forma de Eloquent (columnas dinámicas, agregados con joins, SQL
en bruto). Ver
[Eloquent - Fachada DB](eloquent.md#db-facade--model-less-queries).

### Disco

Un backend de almacenamiento con nombre, registrado a través de la
fachada `Storage` - `Storage::disk("s3")`, `Storage::disk("local")`.
Cada disco implementa [DiskExt](#diskext) y se indexa por su nombre de
registro. Ver [Sistema de archivos y almacenamiento](filesystem.md).

### `DiskExt`

El trait que implementa cada backend de almacenamiento - `put`, `get`,
`delete`, `list`, `signed_url`, etc. Respaldado por `opendal` por
debajo; se incluyen adaptadores para fs local, en memoria, S3, Azure
Blob y GCS. Ver [Sistema de archivos y almacenamiento](filesystem.md).

## E

### Eloquent

Toda la capa de ORM - trait `Model`, `Builder<M>`, relaciones, casts,
scopes, observers, eventos, eliminaciones suaves, prunable, factories.
El nombre de Laravel para lo que otros ecosistemas llaman un ORM; en
Suprnova se apoya sobre SeaORM (que el usuario no debería ver). Ver
[Eloquent](eloquent.md).

### Sobre (cola)

El struct envoltorio (`Envelope { payload, attempts, max_attempts,
delay, ... }`) que un driver de cola realmente serializa y almacena.
Aísla el payload del [Job](#job) de la plomería de la cola. Ver
[Cola](queues.md).

### Evento

Un struct clonable despachado a través de `EventDispatcher::dispatch(evt)`
y entregado a cada `Listener<E>` registrado. Suprnova
ofrece el trait, la fachada (`EventFacade`), el agregador `Subscriber`,
y ganchos para los [Oyentes encolados](#oyente-encolado). Ver [Eventos](events.md).

### Oyente de eventos

Ver [Oyente](#oyente).

## F

### Fachada

La convención de nombrado para un struct de tamaño cero cuyo bloque
`impl` contiene la API pública de un subsistema - `Cache`, `Mail`,
`Auth`, `Storage`, `Bus`, `Notify`, `Vector`, `DB`, `Schedule`, `App`.
Heredada de Laravel; en Suprnova la implementación subyacente se
resuelve a través del [Contenedor](#contenedor) en lugar de mediante
el magic-call de PHP. Ver [Contenedor de servicios](container.md).

### Factory (Eloquent)

La macro `#[derive(Factory)]` y el trait `Factory` que producen filas
de test realistas con valores por defecto impulsados por `fake` -
`UserFactory::times(5).create_many().await?`. La contraparte en Rust
de las factories de modelo de Laravel.
Ver [Macros - Factories](macros.md#factories).

### Fail-closed

Una política de fallo de driver donde una caída del backend hace que
la solicitud se rechace con un 5xx - usada por rate limit, session, e
idempotency cuando "es mejor rechazar que filtrar". Lo opuesto de
[Fail-open](#fail-open). Se configura vía
`BackendErrorPolicy::FailClosed`. Ver [Limitación de velocidad](rate-limiting.md).

### Fail-open

Una política de fallo de driver donde una caída del backend deja pasar
la solicitud (con una advertencia registrada) en lugar de rechazarla -
usada cuando la disponibilidad pesa más que el límite. Se configura
vía `BackendErrorPolicy::FailOpen`. Ver [Limitación de velocidad](rate-limiting.md).

### Indicador de características

Un booleano (o un valor tipado) indexado por nombre y evaluado contra
el usuario/contexto actual - `feature!(MyFeature)`. Respaldado por el
trait `Evaluator`; se ofrece un evaluator de base de datos y, encima,
uno con caché TTL. Ver [Indicadores de características](feature-flags.md).

### Fillable

La lista de permitidos en tiempo de compilación que dice qué columnas
del modelo pueden asignarse en masa desde un hash de atributos no
confiables - declarada en el struct del modelo vía el atributo
`#[fillable]` o el trait `Fillable`. El dual de `#[guarded]`. Ver
[Eloquent - Asignación masiva](eloquent.md#mass-assignment).

### Filesystem

Todo el subsistema de almacenamiento - la fachada `Storage`, los
[Disco](#disco)s registrados, el trait [DiskExt](#diskext), la copia
en streaming entre discos. Ver [Sistema de archivos y almacenamiento](filesystem.md).

### Form request

Un struct que implementa `FormRequest` (o derivado vía `#[request]`)
que extrae y valida el cuerpo de una solicitud antes de que se ejecute
el handler. El análogo componible y con seguridad de tipos de las
clases form-request de Laravel. Ver [Validación](validation.md).

### `FrameworkError`

El único enum al que se convierte cada fallo interno del framework.
Lleva su propia proyección a `HttpResponse` (`From<FrameworkError> for
HttpResponse`) que sanea los cuerpos 5xx y estampa un request id. Ver
[Modelo de errores](error-model.md).

## G

### Compuerta

El punto de entrada de autorización - `Gate::allows("update-post", user,
post)`. Se resuelve contra las policies registradas (declaradas vía la
macro `#[policy]`) y hace cortocircuito en el allow/deny. Devuelve un
`GateResponse` (reexportado como el `Response` de autorización). Ver
[Autorización](authorization.md).

### Scope global

Una restricción de consulta aplicada a cada llamada a `Model::query()`
hasta que se elimina explícitamente (`Builder::without_global_scope`).
Implementado vía el trait `GlobalScope` y registrado en el bootstrap.
Ver [Eloquent - Scopes](eloquent.md#scopes).

### Guard (auth)

La estrategia de autenticación con nombre, adjunta a una solicitud -
`session` (con estado, respaldada por cookie), `token` (sin estado,
bearer-token). Coexisten varios guards; `Auth::guard("api")` elige
uno. Ver [Autenticación](authentication.md).

### Guarded

La lista de bloqueo en tiempo de compilación que dice qué columnas del
modelo *no pueden* asignarse en masa. El dual de [Fillable](#fillable).
Ver [Eloquent - Asignación masiva](eloquent.md#mass-assignment).

## H

### `HasMany`

Un tipo de relación uno a muchos - el padre lleva la clave local, los
hijos llevan la clave foránea. Uno de los once tipos de relación de
Eloquent. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### `HasManyThrough`

Una relación que llega al modelo relacionado saltando a través de un
tercer modelo intermedio - `Country -> User -> Post`. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### `HasOne`

El hermano de una sola fila de [HasMany](#hasmany) - el padre lleva la
clave local, el hijo tiene la clave foránea, devuelve como máximo una
fila. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Fachada Hash

El punto de entrada para el hashing de contraseñas - `hash(password)`,
`verify(password, hash)`. Elige bcrypt o argon2 vía `HASH_DRIVER`;
`needs_rehash` te deja migrar usuarios entre algoritmos al iniciar
sesión. Ver [Hashing](hashing.md).

### Handler

La función async que devuelve una `Response` para una ruta que hizo
match - convertida a la forma de handler tipado del framework por la
macro `#[handler]`. Compuesta en el borde interior de la cadena de
middleware. Ver
[Enrutamiento](routing.md), [Controladores](controllers.md).

### `HttpError`

El trait que implementa un tipo de error definido por el usuario para
especificar cómo debe renderizarse como respuesta HTTP - status,
cuerpo, encabezados. Refleja las excepciones `Renderable` de Laravel.
Ver [Manejo de errores](errors.md).

### `HttpResponse`

El tipo concreto de respuesta HTTP que producen los handlers y el
middleware. Envuelve un código de estado, encabezados, y un cuerpo -
lo que realmente se escribe hacia el cliente. Ver [Respuestas](responses.md).

## I

### Clave de idempotencia

Un encabezado suministrado por el cliente (`Idempotency-Key`) que dice
"si ya procesaste una solicitud con esta clave, reproduce la misma
respuesta en lugar de volver a ejecutar el handler". Requerida para
que POST/PUT/PATCH/DELETE sean seguros ante reintentos; Suprnova
ofrece `Idempotency`, `Idempotent`, y `Replay` para envolver handlers.
Ver [Idempotencia](idempotency.md).

### Respuesta de Inertia

Una respuesta que devuelve un nombre de componente tipado más props
serializadas en lugar de HTML - el puente entre un handler de Rust y
una página de Svelte / React / Vue. Se construye con
`Inertia::render(...)` o la macro `#[derive(InertiaProps)]` más
`inertia_response!`. Ver
[Frontend](frontend.md), [Respuestas de Inertia](frontend-inertia-responses.md).

### `InertiaProps`

La macro derive que genera el impl de `Serialize` más los metadatos de
tipo de TypeScript para un struct usado como props de una página de
Inertia. Impulsa el comando `suprnova generate-types`. Ver
[Tipos de TypeScript](frontend-typescript-types.md).

## J

### Job

Un struct serializable que implementa el trait `Job` - tiene un método
`handle(self)`, se encola a través de `Queue::push(job)` (o
`Queue::push_later(job, when)` para un despacho retardado). Se
persiste en el almacenamiento del driver de cola y lo ejecuta un
worker. Ver [Cola](queues.md).

### Middleware de job

Los envoltorios componibles (`WithoutOverlapping`, `RateLimited`,
`ThrottlesExceptions`, `Skip`, `FailOnException`,
`SkipIfBatchCancelled`) que se ejecutan alrededor de la llamada
`handle` de un job. El equivalente en la cola del middleware HTTP. Ver
[Cola - Middleware de job](queues.md#job-middleware).

### `JobOutcome`

El enum discriminado que produce la resolución de un job -
`Completed`, `Failed`, `Released`, `Deleted`, `Skipped` - reportado a
través de los eventos de ciclo de vida del job y el contador de
métricas de la cola. Ver [Cola](queues.md).

## L

### Colección perezosa

La contraparte en streaming de [Collection](#collection-eloquent) -
`Model::query().lazy().await` devuelve un `LazyCollection<M>` que trae
filas de la base de datos en chunks en lugar de cargar cada fila en
memoria. Ver
[Eloquent - Chunking e iteración perezosa](eloquent.md#chunking-and-lazy-iteration).

### Paginador consciente de longitud

El paginador clásico de páginas numeradas (`Builder::paginate(per_page)`)
que ejecuta la consulta más un `COUNT(*)` - conoce el total de filas.
Ver [Eloquent - Paginación](eloquent.md#pagination).

### Oyente

El trait que implementa un handler de eventos - `Listener<E>::handle(evt)`.
Se registra con `EventDispatcher::listen::<E, _>(arc_listener)` o vía
el agregador `Subscriber`. Ver [Eventos](events.md).

### Guarda de bloqueo (caché)

El handle que devuelve `Cache::lock(key, ttl).acquire()`, que
representa exclusión mutua entre procesos - `LockGuard`. Liberar la
guarda libera el lock; abandonarla sin liberarla depende del TTL. Ver
[Caché](cache.md).

### Política de bloqueo

La política de todo el proyecto para manejar el envenenamiento de
`std::sync::Mutex` / `std::sync::RwLock` en un proceso de larga
duración - dos patrones autorizados (mapear a error o recuperar en el
sitio); nunca un `.lock().unwrap()` sin procesar. Ver [Política de bloqueos](lock-policy.md).

## M

### `Mailable`

El trait que implementa un mensaje de correo - `subject`, `to`, `cc`,
`bcc`, `view`, adjuntos. Escrito a mano o derivado vía la macro
`#[derive(NotificationMailable)]`; se envía a través de
`Mail::to(...).send(MyMail).await`. Ver [Correo](mail.md).

### Modo de mantenimiento

Un cambio en tiempo de solicitud que pone la aplicación fuera de línea
para todos, salvo una lista de permitidos - `maintenance_mode().set(payload)`.
Respaldado por `FileMaintenanceMode` (por defecto, un archivo
centinela) o `CacheMaintenanceMode` (respaldado por caché, para
despliegues multi-instancia); servido por `MaintenanceMiddleware`.
Reexportado en la raíz del crate.

### Middleware

Un envoltorio componible alrededor de un handler - ve la solicitud
antes, la respuesta después, y puede hacer cortocircuito devolviendo
`Err(resp)`. Se registra globalmente, por ruta, o por grupo; se
ejecuta en un orden fijo de afuera hacia adentro. Ver [Middleware](middleware.md).

### Modelo

Un struct anotado con `#[suprnova::model]` que nombra una tabla de
base de datos. El struct *es* el `Model` de SeaORM una vez que la
macro se expande - Suprnova no lo envuelve. Lleva CRUD vía el trait
`Model`, construcción de consultas vía `Model::query()`, factories,
casts, scopes, relaciones, observers. Ver [Eloquent](eloquent.md).

### Morph

Abreviatura de "polimórfico". Una relación morph permite que una sola
relación apunte a uno de varios tipos de modelo - `MorphTo` (un único
propietario de varios tipos posibles), `MorphMany`/`MorphOne` (la
inversa, agrupando hijos morphed), `MorphToMany`/`MorphedByMany`
(muchos a muchos entre tipos morphed). El framework mantiene un
[Registro](#registro) en tiempo de ejecución de mapeos `MorphTypeEntry`
entre strings discriminadores y tipos de Rust. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Mutador

Una transformación de escritura declarada con la macro `#[mutator]` -
se ejecuta cada vez que se asigna la propiedad, antes de que el valor
se guarde en el modelo. El dual de un [Accesor](#accesor). Ver
[Eloquent - Accesores y mutadores](eloquent.md#accessors-and-mutators).

## N

### Notifiable

El trait que implementa un usuario (o cualquier objeto que pueda
recibir notificaciones) - `route_for(channel)` devuelve la dirección
para el canal nombrado (dirección de correo, suscripción push, id de
usuario de difusión, etc.) o `None` para omitirlo. Ver
[Notificaciones - El trait Notifiable](notifications.md#the-notifiable-trait).

### Notification

El trait que implementa un mensaje de notificación - `channels()`
devuelve la lista de nombres de canal a los que debe repartirse; cada
canal vuelve a llamar a la notificación (vía traits por canal como los
métodos de payload de `MailRendering` / `DatabaseChannel`) para
obtener el payload específico del canal. Se despacha a través de
`Notify::send(&user, &notif).await`. Ver
[Notificaciones](notifications.md).

## O

### Observer

Un struct que implementa `Observer<M>` y escucha los eventos de ciclo
de vida de un modelo de Eloquent - `creating`, `created`, `updating`,
`updated`, `deleting`, `deleted`, `saving`, `saved`, `retrieved`,
`replicating`, etc. Se registra vía la macro
`#[suprnova::observer(M)]`; se drena del inventario al arrancar. Ver
[Eloquent - Observers y eventos de ciclo de vida](eloquent.md#observers-and-lifecycle-events).

### `OriginPolicy`

La elección de aplicación del middleware CSRF para el encabezado
`Origin` en solicitudes que cambian estado - `Strict` (debe coincidir
con el host), `AllowList`, o `None`. Ver [Protección CSRF](csrf.md).

## P

### Paginador

El resultado de una llamada a `.paginate(...)` - uno de tres tipos.
`LengthAwarePaginator` (páginas numeradas con un `COUNT(*)`),
`Paginator` (siguiente/anterior, sin total), `CursorPaginator` (cursor
opaco para iteración estable sobre un conjunto de resultados que se
mueve). Los tres serializan a un payload JSON con forma de Laravel.
Ver
[Eloquent - Paginación](eloquent.md#pagination).

### Límite de pánico

El envoltorio `AssertUnwindSafe(...).catch_unwind()` alrededor de la
cadena de middleware (y alrededor de cada handler de worker en segundo
plano) que convierte un pánico no manejado en un 500 saneado más un
evento `ErrorOccurred` registrado. Una red de seguridad, no un
contrato - las APIs públicas deben seguir devolviendo `Result`. Ver
[Ciclo de vida de la solicitud - Límite de pánico](lifecycle.md#5-panic-boundary--execute_chain_safely).

### Proveedor de pago

Un tipo que implementa el super-trait `PaymentProvider` (= `Checkout` +
`Subscription` + `CustomerStore` + `WebhookHandler`). Adaptadores de
referencia: `suprnova-payments-stripe` (gateway, impl completo de
`Payment`) y `suprnova-payments-paddle` (merchant-of-record, sin
`Payment`).
Ver [Pagos](payments.md), [Guía de proveedor](payments-provider-guide.md).

### Pivot

El modelo intermedio en una relación [BelongsToMany](#belongstomany) -
un `#[suprnova::model]` de primera clase, con su propio struct, casts,
y timestamps, nombrado explícitamente como el tercer parámetro de tipo
(`BelongsToMany<L, R, P>`). Suprnova no sintetiza un pivot implícito a
partir de un nombre de tabla. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Canal de presencia

Una variante de [Canal](#canal-difusión) donde el servidor rastrea
quién está suscrito en cada momento y emite eventos de entrada/salida
con los metadatos de cada miembro. Útil para indicadores de "quién
está en línea". Ver [Difusión - Canales de presencia](broadcasting.md#presence-channels).

### Canal privado

Una variante de [Canal](#canal-difusión) que exige autorización al
suscribirse - `authorize(...)` debe devolver true para el usuario que
se suscribe. Útil para streams de notificación por usuario. Ver
[Difusión - Canales](broadcasting.md#channels).

### Prunable

El trait que marca un modelo con eliminación suave (o consultable)
como elegible para limpieza por `model:prune` -
`Prunable::prunable_query()` devuelve el builder para las filas que
deberían irse. `MassPrunable` elimina en un único `DELETE WHERE`; el
comportamiento por defecto emite eliminaciones fila por fila para que
los observers se disparen. Etiquetado para el registro vía la macro
`#[prunable]`. Ver
[Eloquent - Prunable](eloquent.md#prunable).

## Q

### Cola

Todo el subsistema de trabajo en segundo plano - fachada `Queue`,
trait [Job](#job), [Sobre](#sobre-cola), drivers (memory, sync, redis,
database, null), worker, batches, cadenas. Ver [Cola](queues.md).

### Driver de cola

Un tipo que implementa `QueueDriver` (push, pop, release, etc.) - se
ofrecen `MemoryQueueDriver`, `SyncQueueDriver` (se ejecuta inline),
`RedisQueueDriver`, `DatabaseQueueDriver`, `NullQueueDriver`. Se elige
al arrancar vía `QUEUE_DRIVER`. Ver
[Cola - Drivers](queues.md#drivers).

### Worker de cola

El bucle de larga duración que saca sobres del driver de cola, ejecuta
el middleware de job alrededor del handler, y reporta el resultado.
Arranca a través del mismo ciclo de vida que el servidor HTTP, así que
los observers y oyentes se disparan de forma idéntica. Se inicia con
`cargo run -- queue:work`. Ver
[Cola](queues.md).

### Oyente encolado

Un `Listener<E>` que, al invocarse, persiste el payload del evento en
la cola y ejecuta `handle` en un worker en segundo plano en lugar de
en el mismo proceso. Útil cuando un oyente de eventos hace I/O que no
debería bloquear la ruta de despacho. Envuelto vía el adaptador
`QueuedListener`. Ver [Eventos](events.md).

## R

### Limitador de velocidad

Todo el subsistema de limitación de velocidad - `RateLimiter` (la
fachada respaldada por caché), el builder `Limit`,
`SlidingWindowConfig` (driver de ventana deslizante),
`RateLimitMiddleware` (montado en la ruta),
`ThrottleRequestsMiddleware` (alias con nombre de Laravel),
`BackendErrorPolicy` (fail-open frente a fail-closed). Ver
[Limitación de velocidad](rate-limiting.md).

### Redirección

Una [HttpResponse](#httpresponse) especializada que envuelve un
encabezado `Location` - construida vía `Redirect::to(...)`,
`Redirect::route(...)`, `Redirect::back()`, con cadenas
`.with(...)`/`.with_input(...)` para datos flash. Ver [Generación de URLs](urls.md), [Respuestas](responses.md).

### Registro

Una búsqueda global del proceso, poblada ya sea en tiempo de
compilación por `inventory` (`ModelEntry`, `RelationEntry`,
`MorphTypeEntry`, `ObserverEntry`, `PrunerEntry`, `TaskEntry`,
`PaymentProviderEntry`, `CommandEntry`) o al arrancar mediante registro
explícito (`ConnectionRegistry`, `MiddlewareRegistry`,
`InertiaRegistry`, `ChannelRegistry`, `VectorRegistry`,
`SupervisorRegistry`). Todos se drenan o se consultan durante la
secuencia de arranque.

### Relación

El trait que implementa cada tipo de relación - `BelongsTo`, `HasOne`,
`HasMany`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`,
`MorphTo`, `MorphOne`, `MorphMany`, `MorphToMany`, `MorphedByMany`. Un
modelo declara sus relaciones como métodos que devuelven un struct de
relación; el framework conduce la carga anticipada, `with(...)`,
consultas de existencia de relación, y touches en cascada desde el
trait. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Solicitud

El struct tipado de solicitud del framework - envuelve la solicitud
de hyper subyacente y expone `req.param("id")`, `req.json::<T>()`,
`req.form_data()`, `req.flash()`, etc. Reexportado como
`suprnova::Request`. Ver [Solicitudes](requests.md).

### `Response`

Suprnova vincula `http::Response` a `Result<HttpResponse,
HttpResponse>` - ambos brazos llevan una `HttpResponse`. Los cuerpos
de los handlers devuelven `Response`, propagan trabajo falible con
`?`, y el runtime colapsa ambos brazos con `result.unwrap_or_else(|e| e)`.
El tipo de decisión de autorización se reexporta como `GateResponse`
para evitar la colisión. Ver [Respuestas](responses.md),
[Ciclo de vida de la solicitud](lifecycle.md#the-response-contract).

### Resource

Dos cosas no relacionadas comparten el nombre; ambas se ofrecen.

1. **Resource de JSON:API** - un struct `#[derive(Resource)]` que
   serializa un modelo con la forma de JSON:API, con sparse fieldsets
   e includes. Ver [Recursos JSON:API](eloquent-resources.md).
2. **Enrutamiento de resource** - un helper de ruta que monta un
   conjunto CRUD `index`/`show`/`store`/`update`/`destroy` contra un
   impl de `ResourceController`. Ver [Enrutamiento](routing.md).

### macro `routes!`

La macro en tiempo de compilación que expande un DSL de enrutamiento
(`get!("/users", users::index)`, `group!`, `middleware!(Auth)`) en una
función factory de `Router`. La única fuente de verdad de rutas para
una aplicación. Ver [Enrutamiento](routing.md), [Macros](macros.md).

## S

### Scope (local)

Un fragmento de consulta reutilizable, declarado en un modelo de
Eloquent con la macro `#[scopes(Model)]` -
`Post::query().published().recent().get()`. Los scopes locales están
desactivados por defecto; solo se ejecutan cuando se invocan. La
contraparte de [Scope global](#scope-global). Ver
[Eloquent - Scopes](eloquent.md#scopes).

### Sembrador

Un tipo que implementa el trait `Seeder` y que puebla la base de datos
con datos iniciales - registrado a través de `suprnova db:seed`. A
menudo respaldado por una [Factory](#factory-eloquent). Ver [Eloquent](eloquent.md).

### URL firmada

Una URL cuya cadena de consulta lleva una firma HMAC
(`?signature=...&expires=...`) que demuestra que la produjo la
aplicación y que no se ha manipulado. Se construye vía `sign_url(...)`
/ `sign_route(...)`; se verifica mediante middleware o vía
`verify_signature(...)`. Ver [Generación de URLs - URLs firmadas](urls.md#signed-urls).

### Eliminaciones suaves

El patrón donde eliminar una fila de un modelo pone un timestamp
`deleted_at` en lugar de emitir `DELETE`. Opcional por modelo vía
`soft_deletes = true` en el atributo `#[suprnova::model]`;
`Model::query()` filtra automáticamente las filas eliminadas;
`with_trashed()` y `only_trashed()` las vuelven a incluir. Ver
[Eloquent - Eliminar y eliminaciones suaves](eloquent.md#deleting-and-soft-deletes).

### Fachada `Storage`

El punto de entrada al subsistema de filesystem -
`Storage::disk("s3")`, `Storage::disk("local")` - que devuelve una
implementación de [DiskExt](#diskext). Ver [Sistema de archivos y almacenamiento](filesystem.md).

### Subscriber

Un agregador que registra muchos oyentes en una sola llamada -
implementa `Subscriber::subscribe(dispatcher)` y se registra vía
`EventDispatcher::subscribe(subscriber)`. Ver [Eventos](events.md).

### Supervisor

El trait que implementa un actor de larga duración en segundo plano
(`Supervisor::run`) para vivir bajo el `SupervisorRegistry`. El
registro captura pánicos en el bucle de ejecución, aplica una
`RestartPolicy`, y vuelve a lanzarlo. El equivalente en Rust del
patrón de supervisor `gen_server` de Erlang. Ver
[Supervisores](supervisors.md).

## T

### Tarea

Un struct que implementa el trait `Task` - declara una expresión cron
o una frecuencia de más alto nivel (`daily()`, `every_minute()`) y se
ejecuta en el planificador. Se descubre en tiempo de compilación vía
el inventario `TaskEntry`. Ver [Programación de tareas](scheduling.md).

### Middleware terminable

Middleware que registra un gancho para ejecutarse *después* de que la
respuesta se haya escrito al cliente - implementado vía el trait
`Terminable`, capturado en un `TerminationSnapshot`, y despachado por
`dispatch_termination`. Útil para logging, flush de métricas,
auditoría post-vuelo. Ver [Middleware - Middleware terminable](middleware.md#terminable-middleware-post-response-hooks).

### Through (relación)

Una relación que salta a través de un tercer modelo intermedio -
[HasManyThrough](#hasmanythrough) y `HasOneThrough`. Ver
[Eloquent - Relaciones](eloquent.md#relationships).

### Timeout

El middleware que acota el tiempo de reloj de una única solicitud y
devuelve 504 cuando se excede el límite - `TimeoutMiddleware`.
Distinto de los timeouts de worker de cola (`TimeoutExceeded` del lado
de la cola) y de los timeouts de cliente HTTP. Ver [Timeout](timeout.md).

### `TypedCommand`

El trait del lado de console - implementado por structs
`#[derive(Command)]` - que le da a un comando de console argumentos
tipados (vía `clap`) y un método async `handle(self)`. Se registra en
el inventario `CommandEntry` en tiempo de compilación. Ver [Consola](console.md).

## U

### `UserId`

El identificador de cadena opaco que devuelve `Auth::id()`. Las rutas de
guard/provider del framework transportan la clave estable que usa el
`UserProvider` configurado; con `EloquentUserProvider<User>` normalmente
es la clave primaria convertida a cadena. Las fachadas de Magnetar
exponen un newtype `UserId`, pero vinculan su valor al ID canónico de la
aplicación antes de escribir el estado de sesión del framework. Mantener
el límite de solicitud con forma de cadena permite que IDs numéricos,
UUID y IDs opacos independientes del provider compartan los mismos
contratos de middleware y eventos. Ver [Autenticación](authentication.md).

## V

### VAPID

Voluntary Application Server Identification - la especificación IETF
para identificar a un emisor de web-push. Suprnova ofrece `VapidKey`,
`VapidSigner`, `VapidClaims`, y el `WebPushClient` que firma cada
solicitud push. Ver [Web Push](web-push.md).

### Fachada `Vector`

El punto de entrada al subsistema de búsqueda vectorial -
`Vector::driver("qdrant").await?.upsert(...)`. Respaldado por
implementaciones de `VectorDriver`: en memoria, Qdrant, Pinecone (bajo
feature), MariaDB nativo. Ver [Vector](vector.md).

### `VectorDriver`

El trait que implementa cada backend vectorial - `upsert`, `search`,
`delete`, `count`. Permite que el framework soporte múltiples bases de
datos vectoriales sin forzar una sola. Ver [Vector](vector.md).

## W

### Web Push

El protocolo de notificaciones push de la plataforma web - payloads
cifrados entregados a través del servicio de push del user agent.
Suprnova ofrece `WebPushClient` (firmante VAPID, parseo de
retry-after, tope de rechazo de 8 KiB) y `WebPushChannel` para la
entrega de [Notification](#notification). Ver [Web Push](web-push.md).

### Webhook

Una solicitud HTTP enviada por un tercero (proveedor de pagos,
proveedor de identidad, …) hacia tu aplicación para reportar un
evento. Suprnova trata cada webhook como idempotente por defecto - los
adaptadores de proveedor implementan `WebhookHandler::verify(...)` y
guardan el event id del proveedor en una restricción `UNIQUE` que
rechaza las repeticiones. Ver
[Pagos - Manejo de webhooks](payments.md#webhook-handling),
[Idempotencia](idempotency.md).

### Flujo de trabajo

Una pieza de trabajo en segundo plano, de larga duración y con estado,
compuesta de pasos tipados - macros `#[workflow]` y `#[workflow_step]`.
El valor de retorno de cada paso se persiste, así que un reinicio del
worker a mitad de un flujo de trabajo se reanuda desde el último paso
completado. La respuesta de Suprnova a los procesos en segundo plano
de varios pasos que no encajan en un solo [Job](#job). Ver
[Flujos de trabajo](workflows.md).

### `WsConfig`

La configuración de WebSocket por ruta - topes de tamaño de payload
(por defecto 1 MiB texto / 64 KiB binario), tamaño máximo de frame,
intervalo de ping, timeout de inactividad, política de origen. Usada
por las rutas `ws!()`. Ver [WebSockets](websockets.md).

### `WsSocket`

El handle de WebSocket tipado del framework, entregado a un handler
`ws!()`. Se divide en una mitad `Sink` (enviar) y una `Stream`
(recibir) vía `WsSocket::split()`; los pings/pongs los gestiona una
tarea de heartbeat con un `AbortHandle`, de modo que un handler
descartado siempre se desmonta limpiamente. Ver [WebSockets](websockets.md).

## Siguiente

- [Mapa de paridad con Laravel](parity.md) - comparación característica
  por característica contra Laravel 13
- [Variables de entorno](env-vars.md) - cada `env!` que lee el framework
- [Índice de documentación](documentation.md) - el mapa de capítulos
