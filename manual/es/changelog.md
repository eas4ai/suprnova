# Registro de cambios

Un registro legible, por versión, de lo que cambió en Suprnova. Cada
sección de versión es el registro de lanzamiento de esa versión. Una
versión se lanza cuando su commit de versión y la etiqueta
`v<version>` correspondiente se publican de forma atómica. Las más
recientes primero.

## Sin publicar

## 1.3.0 - 2026-08-24

### Seguridad

- **Magnetar ahora limita las mutaciones de credenciales y sesiones al
  actor autenticado y a la época de autenticación de la cuenta.** Las
  escrituras de contraseña, passkey, cuentas vinculadas, dos factores,
  sesión opaca, JWT, remember, OAuth y autorización de dispositivo
  rechazan actores obsoletos o revocados. La primera prueba correcta de
  restablecimiento de contraseña, enlace mágico o email verificado por
  OAuth avanza la época y elimina atómicamente credenciales, sesiones,
  estado remember y registros TOTP provisionales. Las cuentas verificadas
  conservan sus credenciales legítimas durante un restablecimiento.
  OAuth nunca vincula automáticamente una cuenta existente no verificada
  solo por el email.

- **`_previous.url` relativo al protocolo ya no puede producir una
  redirección abierta fuera de origen mediante `Redirect::back()`.**
  `SessionMiddleware` sanea la URL al escribir y `SessionData::previous_url()`
  repite la comprobación al leer; las rutas `//host`, `/\host` y las que
  contienen bytes de control ASCII se tratan como ausentes.

- **La comprobación de `Referer` de la redirección de validación de
  Inertia rechaza dos bypasses adicionales del mismo origen.** Se
  rechaza cualquier byte de control ASCII y también se sanea el fallback
  final de la ruta solicitada, con `/` como último recurso.

- **El texto cifrado de cookies ahora queda ligado al nombre lógico de
  la cookie mediante AAD v2 con contexto.** `Cookie::encrypted` /
  `Cookie::read_encrypted_for` impiden reutilizar un valor en otra
  ranura; la ventana de compatibilidad prueba v2 y después v1 en todo el
  anillo de claves.

- **Los prefijos de cookies de sesión y remember-me se validan en el
  arranque y se aplican al renderizar.** `SESSION_COOKIE_PREFIX=__Host-`
  exige `Secure`, `Path=/` y ningún `Domain`; `__Secure-` exige `Secure`.
  Las combinaciones inválidas fallan antes de servir.

### Añadido

- **La autenticación de Suprnova ahora se ejecuta en el motor interno
  Magnetar.** La fachada `Auth`, propiedad del framework, conserva los
  sitios de llamada existentes para contraseña, enlace mágico, passkey,
  OAuth, bearer, bloqueo, sesión y dos factores, a la vez que elimina la
  dependencia de Torii. El motor predeterminado instala de forma atómica
  adaptadores de contraseña/sesión y passkey, almacena las concesiones de
  entrega del ciclo de vida en la base de datos de la aplicación y
  comparte las identidades canónicas `i64` `app_users` de la aplicación.
- **Un ejecutor de migraciones de autenticación consciente de la forma
  ahora cubre fuentes de Torii, Suprnova web y Suprnova API.** Las
  ejecuciones de prueba vinculan un id de plan estable a huellas
  duraderas de filas y esquema, además de decisiones sobre identidades de
  destino. La aplicación usa importaciones transaccionales, registros de
  reintentos, limpieza propiedad de la forma y rechazo de colisiones.
  MySQL usa un intercambio en sombra protegido por barrera de escritura
  con diarios de precopia, paridad de filas y esquema, cambios de nombre
  reanudables y restauración que preserva la limpieza.

- **`MAIL_DRIVER=file` escribe un `.eml` RFC 5322 por mensaje** en
  `MAIL_FILE_PATH` (por defecto `storage_path("mail")`) para abrir el
  correo en un cliente; en producción falla cerrado salvo que se
  establezca `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **`FrameworkError::External` conserva el error envuelto** mediante
  `from_external`, `from_external_with` y `external_source()`; ambos
  constructores se convierten en HTTP 500.
- **Los logs 5xx muestran la cadena completa de errores.** `render_error_chain`
  alimenta el log, el payload de `ErrorOccurred` y `debug_message` con
  `APP_DEBUG=true`, sin cambiar los cuerpos saneados del cliente.
- **Los props de scroll admiten envoltorios y metadatos de paginación.**
  `scroll_wrapped`, `scroll_with_wrapped`, `try_scroll_wrapped` y
  `ProvidesScrollMetadata` reflejan la interfaz de Laravel; `match_on`
  también se emite en `matchPropsOn`.
- **`Prop::merge_with_path`, `match_on` multi-campo y props con resolver.**
  Se añaden `merge_lazy` / `merge_lazy_with`; las rutas anidadas se
  fusionan sin fusionar también la raíz.
- **Las recargas parciales `only` / `except` entienden notación de
  puntos.** `user.name` reduce el prop y `user.email` elimina solo ese
  campo; `except` gana ante una colisión y `Always` no se filtra.
- **Las claves de props con puntos se anidan en el wire.** `.with("user.name", …)`
  construye `props.user`, y las claves del registro compartido siguen la
  misma semántica de `Arr::set`.
- **`App::inertia_shared(key)` y `App::flush_inertia_shared()`** leen y
  limpian el registro estático compartido; los proveedores por solicitud
  no se eliminan.
- **`InertiaResponse::always_with(key, resolver)`** añade el hermano
  asíncrono de `.always`.
- **`InertiaSharedData::share` recibe ahora el nombre del componente**,
  para variar datos compartidos por página.
- **La composición de props Inertia** usa flags ortogonales con
  `Prop::eager` / `lazy` / `from_resolver` / `absent`, `InertiaResponse::prop`
  y los métodos `always`, `optional`, `defer`, `merge`, `once`, `scroll`.
- **Pausa y reanudación de colas.** Se añaden `Queue::pause`, `resume`,
  `pause_all`, `resume_all`, `is_paused`, `paused_queues`, los comandos
  `queue:pause` / `queue:resume`, `QUEUE_PAUSABLE=false` y sus eventos.
- **`suprnova::testing::TestResponse`** ofrece aserciones fluidas sobre
  `(status, headers, body)`, incluida `assert_session_has`.
- **`suprnova new` genera una entrada SSR** (`frontend/src/ssr.{ts,tsx}`)
  y el script `build:ssr` para Svelte, React y Vue.
- **`InertiaConfig::ssr_bundle_path` y `.ssr_ensure_bundle_exists`**
  comprueban el bundle antes de despachar al worker.
- **Los fallos de validación de Inertia redirigen con `303`** a la página
  del formulario y colocan los errores en flash; las solicitudes no
  Inertia conservan `422`, y `with_all_errors` conserva todos los mensajes.
- **`InertiaConfig::with_all_errors(bool)`** - conserva todos los mensajes de validación por campo en lugar de colapsarlos al primero. Es el equivalente de `Inertia\Middleware::$withAllErrors` de Laravel.
- **`AssertableInertia`** añade aserciones fluidas y recargas
  `reload_only`, `reload_except` y `load_deferred_props` mediante
  `with_reload`.

- **`Cookie::queue` / `queued` / `unqueue` / `expire`.** Un tarro
  task-local permite encolar una cookie para la próxima respuesta;
  `SessionMiddleware` lo drena junto a la cookie de sesión. Fuera de un
  scope de sesión las llamadas son no-op.
- **`HttpResponse::event_stream` y `stream_json`.** Equivalentes a
  `ResponseFactory::eventStream` / `streamJson`, con framing SSE y arrays
  JSON incrementales sobre el pipeline cancelable existente.
- **`suprnova serve` recrea procesos de desarrollo caídos.** Backoff
  exponencial, `--no-restart`, `--restart-tries`, `--timestamps`,
  `Suprnova.toml [[serve.process]]` y salida NDJSON `--json`.
- **`RequestBuilder::retry_when(predicate)`.** Recibe
  `RetryContext` y solo puede vetar un reintento que la política ya
  decidió hacer.
- **`#[model(touches = [...])]` ahora toca realmente.** Después de crear,
  guardar, actualizar o borrar un hijo, cada propietario `BelongsTo` nombrado
  en la lista recibe un `UPDATE <owner> SET updated_at = ? WHERE <key> = ?`,
  en el mismo executor que la escritura que lo desencadenó - dentro de
  `DB::transaction`, el toque se une a esa transacción y se revierte con ella.
  Los propietarios sin `updated_at` se omiten; los propietarios nulos o
  polimórficos provocan un error de compilación; los propietarios polimórficos
  aún no están soportados.
- **`without_touching_on::<M, _, _>(fut)`** - el
  `Model::withoutTouchingOn([M::class], $cb)` de Laravel. Suprime tanto
  `m.touch()` como cualquier cascada de propietario dirigida a `M`, mientras
  los propietarios de otros tipos siguen actualizándose. Los ámbitos se
  anidan, y el `without_touching` existente también suprime ahora la cascada
  de propietarios, además de las llamadas directas a `touch()`.
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - el
  `touchOwners()` de Laravel, para cuando escribiste la fila hija mediante
  una ruta que el framework no controla.
- **Reglas de validación con forma de valor:** `ArrayKeys` y `Distinct`
  implementan `ValueRule` sobre `serde_json::Value`.
- **`Job::delay()`** declara un retraso predeterminado para
  `Queue::push` y `Queue::bulk`; las variantes explícitas `later` ganan.
- **Ajuste de cola de notificaciones.** `queue`, `timeout`,
  `fail_on_timeout`, `max_tries` y `backoff` se transportan a cada
  `SendNotificationJob` mediante `EnvelopeOverrides`.
- **`Mail::on_queue` / `Mail::on_connection` y
  `Queue::push_with` / `later_with`.** Los overrides ganan a `Queue::route`
  y a los defaults de `Job`; los fakes ahora pueden afirmar cola y conexión.
- **`Application::http_bootstrap(f)`** separa el arranque HTTP del
  arranque de workers y consola, que ya no necesitan manifiesto frontend.
- **`Router::inertia(path, component, props)`** añade la superficie
  `Route::inertia` y devuelve un `RouteBuilder` nombrable.
- **Opciones de envío SES v2.** `TenantName`, `ConfigurationSetName` y
  `ListManagementOptions` admiten defaults de transporte y overrides por
  encabezado.
- **`without_cookies`** está disponible en todos los builders de
  respuesta; `Cookie::forget_with` borra cookies limitadas a ruta/dominio.
- **`Queue::fake()` estampa IDs de sobre** y expone `pushed_with_id`.
- **Evento de cola `UniqueJobSkipped`.** `Queue::push_unique` ahora
  despacha `queue::events::UniqueJobSkipped { job_name, unique_id, connection }`
  cuando suprime un duplicado, de modo que una deduplicación sea observable en
  lugar de silenciosa. El valor de retorno de la llamada no cambia
  (`Ok(false)`).
- **`model_keys()`** en builders y colecciones devuelve claves calificadas
  sin hidratar modelos.

### Corregido

- **Los borrados lógicos de PostgreSQL ahora usan marcadores de posición
  adaptados al backend, y las escrituras de marcas de tiempo generadas
  respetan las conversiones declaradas.** `delete()` y `restore()` generan
  marcadores de posición ordinales de PostgreSQL en lugar de los marcadores
  `?` de MySQL y SQLite. Las escrituras generadas de creación, actualización,
  guardado, modificación de la marca de tiempo y borrado lógico también
  convierten las marcas de tiempo mediante el tipo de almacenamiento `Cast`
  declarado de cada campo, por lo que las columnas `TIMESTAMPTZ` nativas ya
  no reciben valores de texto. Gracias a
  [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) por informar de
  ambos defectos y enviar una corrección en
  [PR #3](https://github.com/eas4ai/suprnova/pull/3).
- **Las ejecuciones predeterminadas del workspace y de la puerta Magnetar ya
  no requieren servicios activos de PostgreSQL o MySQL.** Las suites de
  comportamiento específicas del backend son pruebas de calificación
  explícitas e ignoradas que siguen fallando cuando se invocan
  deliberadamente sin su base de datos configurada. Se eliminaron las pruebas
  exclusivas de accesibilidad y los requisitos permanentes del entorno de la
  puerta, por lo que los cambios no relacionados no tienen que asumir la
  configuración de una base de datos externa en cada ejecución de verificación.
- **`PartialFilter::narrow` ahora es pública**, para que integraciones
  externas reproduzcan el estrechamiento de recargas parciales.
- **`MailFake::QueuedSnapshot` admite `on_connection`.**
- **Las shares con claves punteadas aceptan antecesores en
  `only`/`except`**, respetando límites de segmento.
- **Los campos `lazy(deferred)` de `#[data]` respetan `?include=`** y un
  include no permitido se descarta antes de anunciar `deferredProps`.
- **`deferredProps` no se vuelve a anunciar en recargas parciales
  coincidentes.**
- **La prop `errors` permanece siempre visible** en recargas parciales,
  tanto si procede de sesión como del handler.
- **Los fakes observan `EnvelopeOverrides` por push** mediante
  `pushed_with_overrides`, `assert_pushed_on_queue` y
  `assert_pushed_on_connection`.
- **Un worker SSR que se quedaba bloqueado a mitad del cuerpo de respuesta
  podía colgar un render para siempre.** `SsrConfig::timeout` solo limitaba
  la espera de encabezados; ahora ambas fases comparten un único plazo, por
  lo que el timeout configurado limita toda la llamada SSR, como prometía su
  propia documentación.
- **Las cookies encoladas - incluida la cookie remember-me que establece
  `Auth::login_remember` - se perdían en silencio en tres rutas internas de
  fallo cerrado de `SessionMiddleware`.** Un fallo al leer o escribir la
  sesión, o al cifrar la cookie de sesión, ahora drena las cookies pendientes
  antes de devolver la respuesta sintetizada `500`, igual que un error del
  handler o una redirección. Esto no cubre un pánico no capturado, igual que
  las cookies encoladas de Laravel se pierden ante uno.
- **`Queue::push_unique` ahora respeta `Job::delay()`, igual que
  `Queue::push`, `Queue::push_with` y `Queue::bulk`.** Antes calculaba
  `available_at` directamente desde `Utc::now()`, por lo que un job con un
  retraso predeterminado se despachaba de inmediato mediante `push_unique`.
  `Queue::push_unique_later` y `Queue::later_unique` no cambian: ya reciben
  una marca de tiempo o retraso explícitos del llamador y nunca consultan
  `Job::delay()`.

### Cambiado

- **La rama de desarrollo actual usa SeaORM 2.0 y requiere Rust 1.94.0.**
  Suprnova conserva las estructuras de código fuente de Eloquent, `#[model]`,
  migración y fachada de base de datos. Las aplicaciones que llaman
  directamente a SeaORM deben importar `ExprTrait` para los métodos de
  expresión de SeaQuery y usar métodos de conexión `*_raw` explícitos para
  valores `Statement` preconstruidos. SeaQuery ahora está en la versión 1.0,
  y el controlador vectorial directo de MariaDB usa SQLx 0.9. Las bases de
  datos existentes no requieren ninguna migración de datos de la aplicación;
  los esquemas nuevos de PostgreSQL conservan claves primarias respaldadas por
  secuencias.
- **Se eliminaron otras tres dependencias sin uso.** `pretty_assertions` y
  `qrcode` salen del crate del framework (`totp-rs` ya incluye la feature `qr`,
  por lo que el aprovisionamiento QR para la inscripción de dos factores no se
  ve afectado), y `notify-debouncer-mini` sale de la CLI (`notify` permanece:
  los watchers de `serve` y `generate-types` lo usan directamente). Las tres
  se confirmaron sin uso mediante `cargo-udeps` y una búsqueda en todo el
  código fuente que cubre los doc tests.
- **`suprnova-macros` ya no depende de `serde` ni de
  `serde_derive_internals`.** Ninguna se usaba: las rutas `::serde::Serialize`
  que emiten las macros se resuelven en el crate descendiente, no en el propio
  crate de macros. No hay efecto sobre el código generado.
- **`match_on` de `MergeStrategy` ahora admite más de un nombre de campo.**
  `Append`, `Prepend` y `Deep` pasan de `match_on: Option<String>` a
  `match_on: Option<Vec<String>>`, de modo que
  `InertiaResponse::merge_with` / `merge_lazy_with` puede eliminar duplicados
  por varios campos, igual que ya podía
  `.prop(key, Prop::eager(v).match_on([...]))`. Antes los atajos del builder
  de respuestas eran menos expresivos que construir un `Prop` directamente.
  Consulta Actualización.
- **Los props de scroll ahora emiten semántica de `reset` y merge idéntica a
  Laravel.** `scrollProps[key].reset` es `true` exactamente cuando el cliente
  nombró `key` en `X-Inertia-Reset`, conforme a `resolveScrollProps` de Laravel,
  y no en toda visita sin encabezado `X-Inertia-Infinite-Scroll-Merge-Intent`.
  Un prop de scroll también lleva metadatos de merge siempre, con append como
  valor predeterminado: una visita nueva emite `reset: false` y una entrada
  `mergeProps`, donde antes emitía `reset: true` sin metadatos de merge. Una
  clave presente en `X-Inertia-Reset` se excluye de `mergeProps` /
  `prependProps` para esa respuesta, igual que un prop de merge normal.
- **`ssr:check` ahora verifica que la ruta `GET /health` del worker SSR
  responde 2xx**, en vez de confirmar solo que algo aceptó una conexión TCP.
  Todos los workers `@inertiajs/{vue3,react,svelte}/server` responden
  `/health` de fábrica, así que no hizo falta cambiar el worker: coincide con
  `Inertia\Ssr\HttpGateway::isHealthy()` de Laravel.
- **El prop `errors` de Inertia ahora lleva una cadena por campo, no un array.**
  Un saco de validación enviado por flash de sesión se renderiza como
  `{ email: "The email field is required." }` en vez de
  `{ email: ["The email field is required."] }`, conforme al valor
  predeterminado de Laravel y a `ErrorValue = string` de Inertia.
  `InertiaConfig::with_all_errors(true)` restaura la forma de array. Un prop
  `errors` que establezca el handler se transmite sin cambios, y el flash de
  sesión (`Redirect::with_errors`, `session.pull_errors_flash()`) sigue
  guardando arrays: solo cambia el prop renderizado de la página.
- **`Model::TOUCHES` pasó de ser una constante inherente a `EloquentModel`.**
  La cascada de toques del padre vive en el valor predeterminado del trait
  `Model`, y un valor predeterminado de trait no puede leer una constante
  inherente. `Comment::TOUCHES` sigue resolviendo, pero ahora necesita
  `use suprnova::EloquentModel;` en alcance. Los modelos sin atributo `touches`
  reciben el valor predeterminado vacío del trait.
- **`RelationEntry` ganó `related_updated_at_column`.** Todo lo que construya
  un `RelationEntry` a mano necesita ese campo adicional; no hay nada así
  dentro del árbol, porque la macro los emite todos.
- **`Router::view` ahora rechaza props que no sean un objeto JSON.** Antes los
  ignoraba en silencio, registrando una ruta que renderizaba un saco de props
  vacío sin diagnóstico. `null` sigue aceptándose como "sin props";
  `Router::try_inertia` es la forma que puede fallar.
- **La versión de assets de Inertia ahora toma por defecto un hash del
  manifiesto de build de Vite** en vez del literal `"1.0"`, de modo que un
  despliegue invalida clientes de larga duración sin que nadie tenga que
  recordar subir una cadena. `InertiaConfig::manifest_path(...)` vuelve a
  apuntar el resolver; un `.version(...)` / `.version_with(...)` explícito
  sigue ganando. Sin manifiesto en disco - desarrollo local - la versión
  vuelve a `"1.0"`, que es lo que toda aplicación veía antes. El nuevo
  `VersionResolver::from_manifest(path)` expone directamente el resolver.

### Obsoleto

- **`Cookie::read_encrypted` ahora es el lector heredado exclusivo de v1.**
  El código que acuña con `Cookie::encrypted` y lee con `read_encrypted` falla
  en tiempo de ejecución en el primer valor escrito después de esta versión;
  cambia a `read_encrypted_for(name, wire)`. Los puntos de entrada sin contexto
  `CryptPurpose::Cookie` también quedan sustituidos. Ambas eliminaciones están
  previstas para 1.4.0.

### Actualización

- **Las advertencias de descifrado de cookies ahora tienen dos ejes
  independientes.** Una advertencia `KeyOrigin::Previous(index)` significa
  volver a cifrar el valor bajo el `APP_KEY` actual y retirar esa clave previa
  solo después de que desaparezca la cola de rotación; una advertencia
  `AadVersion::Legacy` significa volver a emitir la cookie mediante la API
  vinculada al nombre antes de retirar el fallback de 1.4.0. Un valor puede
  informar de ambas.
- **`SESSION_COOKIE_PREFIX` es opcional.** Despliega `__Host-` solo con HTTPS,
  `SESSION_SECURE=true`, `SESSION_PATH=/` y sin `SESSION_DOMAIN`; los
  iniciadores HTTP locales lo dejan vacío. `CsrfMiddleware` con
  `with_session_config` conserva el nombre literal `XSRF-TOKEN`; usa
  `.xsrf_cookie_name("__Host-XSRF-TOKEN")` cuando el cliente esté configurado
  para ese nombre separado.
- **`DecryptOrigin` ahora es un struct `#[non_exhaustive]` de dos ejes.**
  Lee sus campos `key` y `aad` de forma independiente y conserva una
  estrategia de coincidencia compatible con comodín para los enums
  `KeyOrigin` / `AadVersion`.
- **`SessionConfig` y `CookieOptions` ahora son `#[non_exhaustive]`.** Los
  literales de struct y las actualizaciones funcionales de registro en código
  de aplicación deben pasar a `Type::default()` seguido de asignaciones a
  campos públicos o métodos builder.
- **`FrameworkError` ahora es `#[non_exhaustive]`.** Un `match` sobre él en
  tu código necesita un brazo comodín. Esta es la última versión en la que
  añadir una variante habría sido un cambio incompatible.
- **El campo `match_on` de `MergeStrategy::Append`/`Prepend`/`Deep` ahora es
  `Option<Vec<String>>`, no `Option<String>`.** Una llamada que construya
  directamente la forma literal del struct -
  `MergeStrategy::Append { match_on: Some("id".into()) }` - deja de compilar;
  envuelve el nombre del campo en un `Vec`:
  `Some(vec!["id".into()])`. `match_on: None` no cambia y no necesita ajustes.
- **Una recarga parcial coincidente ya no emite `deferredProps`.** El código
  que lee `page.deferredProps` de una respuesta de recarga parcial - un
  componente personalizado de carga diferida, un snapshot de test o una
  aserción end-to-end - encontrará ausente la clave donde antes enumeraba los
  props diferidos que la solicitud no nombró. Lee los anuncios en la visita
  inicial (no parcial), que es donde los coloca Laravel y donde los lee el
  cliente oficial.
- **Una entrada `except` desnuda ahora elimina las claves de props punteadas
  bajo ella.** `X-Inertia-Partial-Except: auth` antes dejaba en la respuesta
  un prop registrado bajo `auth.user`, porque la compuerta comparaba claves
  completas. Ahora se elimina. Si una página dependía de que una entrada
  `except` desnuda recortara solo la clave exacta, nombra la clave exacta
  (`except: ['auth.user']`) o usa una ruta punteada más estrecha.
- **`errors` ignora `only`/`except`.** Una recarga parcial que filtraba un prop
  `.with("errors", …)` del handler, o que lo estrechaba con una entrada
  punteada, ahora lo envía completo. Los tests que esperan un objeto `errors`
  recortado o vacío en una recarga parcial deben actualizarse. Para mantener
  deliberadamente el saco fuera de una respuesta, marca el prop:
  `.prop("errors", Prop::eager(…).optional())`, en vez de depender de las
  listas de recarga parcial.
- **`Prop::resolve_with_owner` también controla los props marcados.** Antes
  resolvía cualquier prop que no fuera `Prop::is_lazy()` - un valor eager o un
  resolver con un flag - sin consultar el conjunto de inclusión. Ahora
  controla todos los props respaldados por resolver y solo deja pasar sin
  control un valor ya materializado. En consecuencia, un campo
  `#[data(lazy(deferred))]` necesita `?include=<field>` en la solicitud antes
  de resolverse o anunciarse, igual que cualquier otra variante lazy. Añade el
  campo a la lista `?include=` o elimina el atributo `lazy(...)` si nunca debía
  ser opt-in.
- **`reset` del prop de scroll ya no sigue el encabezado de intención de
  merge.** El código que lee directamente `page.scrollProps[key].reset` - un
  componente personalizado de scroll infinito o un snapshot de test - verá
  `reset: false` (más una entrada `mergeProps`) en una revisita simple que
  antes leía `reset: true` sin metadatos de merge. El componente oficial
  `<InfiniteScroll>` solo se comporta distinto en una revisita simple: escucha
  `reset` en cada evento `router` `success`, no solo en un `router.reload()`,
  así que una revisita normal ya no borra su estado acumulado salvo que el
  servidor nombre la clave en `X-Inertia-Reset`, conforme a Laravel. Envía
  `X-Inertia-Reset: <key>` explícitamente donde dependías del comportamiento
  anterior.
- **`Prop::match_on` toma `impl MatchOnFields`, no `impl Into<String>`.** El
  nuevo bound permite nombrar varios campos en una llamada
  (`match_on(["id", "slug"])`), y su lista de impls está cerrada:
  solo `&str`, `String`, `[T; N]` y `Vec<T>`. No hay un impl genérico sobre
  `IntoIterator`: la coherencia lo rechaza frente a los impls de `&str` y
  `String`, porque nada impide que esos tipos obtengan un impl de
  `IntoIterator` más adelante. Tres tipos de argumento que antes compilaban
  ya no lo hacen: `&String`, `Cow<'_, str>` y `Box<str>`. Pasa un `&str` en
  el punto de llamada: `match_on(name.as_str())` para `&String`,
  `match_on(name.as_ref())` para `Cow<'_, str>`, `match_on(&*name)` para
  `Box<str>`.
- **Una entrada `only`/`except` punteada ahora estrecha su prop de nivel
  superior en vez de excluirla por completo.** Antes,
  `X-Inertia-Partial-Data: user.name` hacía que `should_include_eager`
  buscara una entrada exacta `"user"`, no encontrara ninguna y descartara
  silenciosamente todo el prop `user`: un cliente que pedía un campo no
  recibía nada. Ahora recibe `{ user: { name: ... } }`, como especifica el
  protocolo Inertia v3. El mismo arreglo se aplica a
  `should_include_optional`, y una entrada punteada (`permissions.read`) ahora
  cuenta como solicitud explícita para el prop de nivel superior `Optional` o
  `Defer`; un resolver que antes no se ejecutaba ahora sí puede ejecutarse.
  Observa el volumen de llamadas del resolver después de actualizar si tu app
  usa props `Optional`/`Defer` con tráfico de recarga parcial punteada.
- **`InertiaSharedData::share` ahora recibe el nombre del componente de
  página.** Añade un parámetro `component: &str` después de `req`:
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```
  Ignóralo (`_component`) si tu proveedor no necesita variar por página:
  `RenderContext` de Laravel lleva el mismo par (`component`, `request`) para
  `ProvidesInertiaProperties::toInertiaProperties`.
- **`Prop` es un struct, no un enum.** Sus variantes desaparecen; construye y
  lee props mediante métodos:
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (`Prop::lazy(closure)` no cambia)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` ->
    `prop.is_absent()`
  Los structs de payload `DeferConfig`, `MergeConfig`, `OnceConfig` y
  `ScrollConfig` se eliminan: sus campos ahora son flags de `Prop`.
  `Prop::is_deferred()` cambia de nombre a `Prop::has_resolver()`, que es lo
  que siempre significó. `DeferOptions`, `OnceOptions`, `MergeStrategy`,
  `ScrollMetadata` y cada método builder de `InertiaResponse` no cambian, así
  que una aplicación que solo usa el builder de respuestas no necesita
  editarse. Las aplicaciones que construyen props a mano - normalmente una
  implementación de `InertiaSharedData` - necesitan los renombres anteriores.
- **Esta corrección protege sesiones que ya tienes, no solo solicitudes
  futuras.** Basta actualizar: una cookie de sesión de una versión anterior
  puede llevar un `_previous.url` nunca saneado, y
  `SessionData::previous_url()` ahora lo descarta al leerlo por primera vez
  después de actualizar, en vez de confiar en que ya esté almacenado. No hace
  falta invalidar sesiones existentes, migrar la tabla de sesiones ni forzar
  otro login. Una solicitud cuya ruta parezca relativa al protocolo
  (`//host`) tampoco actualiza de ahora en adelante la URL anterior
  registrada; si la ruta `fallback!` de tu app dependía legítimamente de que
  esa ruta se convirtiera en destino de `Redirect::back()`, ya no lo hará.
  No hace falta cambiar código salvo que dependieras de ese caso límite, que
  ya era un riesgo de redirección abierta.
- **Elimina `[0]` de cada binding `errors.<field>` en tus páginas.** Con la
  nueva forma predeterminada `errors.email` es una cadena, así que
  `errors.email[0]` renderiza su primer carácter en vez del mensaje. Cambia
  también el tipo TypeScript de `string[]` a `string`. Si prefieres no tocar
  tus páginas, establece `InertiaConfig::with_all_errors(true)` en la config
  que pasas a `Inertia::install` y añade la ampliación de módulo
  `errorValueType: string[]` para `@inertiajs/core`. Los frontends starter
  distribuyen la nueva forma.
- **Un handler que escribiera a mano el redirect-back después de un fallo de
  validación puede eliminarse.** El puente ahora es automático; un handler
  que todavía redirija por sí mismo sigue funcionando, porque el middleware
  solo actúa sobre un `422` que lleva un objeto `errors` poblado.
- **Un hijo de `suprnova serve` que se caía ahora vuelve a arrancar en vez
  de terminar la sesión.** Si dependías de que un crash detuviera
  `suprnova serve` por completo (un smoke check de CI o un script que trata
  la salida como "algo va mal"), pasa `--no-restart` para restaurar
  exactamente ese comportamiento. Los reintentos también están limitados:
  un proceso que falla cinco veces seguidas deja de reintentarse (sube el
  límite con `--restart-tries` o usa `--no-restart`).
- **`Model::TOUCHES` ya no es una constante inherente.** El código que leía
  directamente `Comment::TOUCHES` necesita `use suprnova::EloquentModel;`
  (o `suprnova::eloquent::EloquentModel`) en alcance: la constante se movió
  allí para que la cascada de toques del padre, un valor predeterminado del
  trait `Model`, pueda leerla. Un `grep -rn TOUCHES` en tu aplicación
  encuentra cada punto de llamada; la mayoría no tiene ninguno porque antes
  la constante no hacía nada en tiempo de ejecución.
- **`RelationEntry` ganó un campo.** Solo el código que construye un
  `RelationEntry` a mano necesita cambios: añade
  `related_updated_at_column` al literal. Los registros de relaciones
  generados por macro ya lo emiten, así que una app normal que solo declare
  relaciones mediante `#[suprnova::model]` no se ve afectada.
- **`Router::view` con props no objeto ahora puede entrar en pánico al
  arrancar.** Antes se registraba en silencio con un saco de props vacío;
  `view` delega en `Router::inertia`, que requiere un objeto (o `null`) y
  entra en pánico de otro modo. Si una llamada a `view` puede llevar props no
  objeto, cambia a `Router::try_inertia` y gestiona el `Err`; por lo demás
  nada cambia.
- **El valor predeterminado del manifiesto de versión de Inertia puede
  cambiar tu cadena de versión en cuanto existe un build.** Una app o test
  que fija `X-Inertia-Version: 1.0` funciona solo hasta que aparece un
  manifiesto de Vite en disco; desde entonces la versión es el hash del
  manifiesto. Si necesitas la constante antigua, léela desde
  `VersionResolver::from_manifest(path)` o fija `.version(...)`
  explícitamente. Espera un ciclo único de recarga de página completa en el
  primer despliegue después de actualizar. El fallback sin manifiesto se
  exporta como `suprnova::MANIFEST_VERSION_FALLBACK`, así que ya no necesitas
  escribir `"1.0"` a mano.
- **Mueve el registro de `Inertia::install` y `global_middleware!` fuera de
  `bootstrap::register`.** Ponlos en una función nueva y pásala a
  `.http_bootstrap(...)`; la forma nueva del iniciador es un
  `register_http_stack()` síncrono llamado como
  `.http_bootstrap(|| async { bootstrap::register_http_stack() })`. Las apps
  que no lo hagan conservan el comportamiento actual, incluido el fallo de
  arranque del worker cuando falta el manifiesto del frontend.

## 1.2.4 - 2026-08-18

### Seguridad

- **El secreto de bypass del modo de mantenimiento se compara en tiempo
  constante.** `MaintenanceMiddleware` comparaba la URL secreta con una
  comparación de cadenas simple, que retorna en el primer byte distinto.
  Como el secreto es una credencial bearer que viaja en la ruta de la
  solicitud, esa diferencia de tiempo le revelaba a un atacante qué
  longitud de prefijo había acertado. La comparación ahora recorre la
  longitud completa en bytes vía `subtle::ConstantTimeEq`, y solo hace
  cortocircuito ante una discrepancia de longitud - la misma forma que
  la comparación de la cookie de bypass que tiene al lado.

- **`rules::Url` ahora rechaza las URIs de script.** La regla aceptaba
  cualquier esquema que `url::Url` supiera analizar, `javascript:` y
  `vbscript:` incluidos, así que una URL validada podía seguir
  desembocando en la ejecución de scripts al renderizarse dentro de un
  `href`. Ahora aplica la forma de la regla `url` de Laravel (el patrón
  `^(PROTOCOLS)://HOST` de `Illuminate\Support\Str::isUrl`): el esquema
  debe estar en la lista de permitidos de Laravel, ir seguido de `://`
  **y** ir seguido de un host no vacío - el grupo de host de Laravel no
  lleva `?`, así que un host ausente o vacío nunca coincide, ni siquiera
  con un esquema listado. La lista de esquemas y el requisito de `://`
  más host son literalmente los de Laravel; el host en sí lo analiza el
  crate `url` en vez de la regex de Laravel, así que unos pocos casos
  límite siguen difiriendo - un puerto fuera de rango se rechaza aquí y
  se acepta allá, y los hosts IDN se normalizan de forma distinta. El
  nuevo `Url::protocols(&[...])` refleja el `url:http,https` de Laravel;
  `HttpUrl` es ahora azúcar literal sobre él y conserva su propio
  mensaje. **Cambio de comportamiento:** una URL con un esquema no
  listado que antes validaba ahora falla - nombra el esquema con
  `Url::protocols(&["myapp"])` si tu intención era aceptarlo. Dos
  cambios de comportamiento más: `mailto:`, `data:` y `tel:` están en la
  lista de permitidos de Laravel por su nombre pero no llevan componente
  de autoridad, así que ahora fallan; y las rutas del estilo
  `file:///etc/passwd` - `scheme://` sin nada entre las dos últimas
  barras - ahora también fallan, porque una cadena vacía tampoco es un
  host. Ambos se derivan de la propia regla de `://` más host de
  Laravel.

- **Las respuestas de Inertia ahora anuncian `Vary: X-Inertia` en todas
  partes.** El encabezado se establecía solo en las propias respuestas
  del objeto de página. Las redirecciones, los 404, los 422 y las
  respuestas estáticas no llevaban ninguno, así que una caché compartida
  indexada solo por la URL podía servir el objeto de página JSON a una
  navegación completa del navegador, o el shell HTML a un XHR de
  Inertia. El nuevo `InertiaHeadersMiddleware` - registrado por
  `Inertia::install` como el más externo de los tres - lo establece en
  todas las respuestas, y convierte un `200` vacío en una visita de
  Inertia en un `303` de vuelta en lugar de una respuesta que el cliente
  rechaza por no ser de Inertia. `InertiaVersionMiddleware` ahora vuelve
  a poner en flash la sesión antes de su `409`, de modo que un error
  puesto en flash sobrevive al GET de página completa que el cliente
  hace a continuación.

- **Tres correcciones en las respuestas de Inertia.**
  `InertiaResponse::location_for(&req, url)` devuelve `409` +
  `X-Inertia-Location` para un XHR de Inertia y un `302` simple
  + `Location` para una navegación completa, así que un rebote de OAuth o
  SSO iniciado fuera de la SPA ya no queda atrapado en un `409` sin
  cuerpo. El `location(url)` existente conserva su forma de `409`
  siempre. El nuevo `App::clear_history()` pone en flash en la sesión el
  flag de limpieza de historial para que sobreviva a la redirección de
  cierre de sesión y aterrice en la página que de verdad se renderiza -
  el `.clear_history()` por respuesta marcaba solo la redirección que el
  navegador descarta, y dejaba descifrable el historial cifrado de la
  sesión anterior. Y una prop `once` ahora se omite solo en una visita
  completa de Inertia: un `router.reload({ only: ['stats'] })` explícito
  la vuelve a resolver en vez de no devolver nada.

- **El transporte de SES ahora envía encabezados de mensaje
  personalizados.** `Mail::to(..)
  .header("List-Unsubscribe", ...)` y `Mailable::headers()` se
  descartaban en silencio con `MAIL_DRIVER=ses`: el cuerpo de la
  solicitud `Content.Simple` no tenía campo `Headers` y el constructor
  de MIME crudo nunca leía `OutgoingMessage::
  headers`, aunque todos los demás transportes los reenvían. Ambas
  rutas de SES ahora los llevan - `Headers` como la lista
  `{Name, Value}` de SES v2, y el MIME crudo como líneas de encabezado
  reales - de modo que los enlaces de baja, los encabezados de hilo y
  las pistas de enrutamiento sobreviven a un cambio de driver. Los
  nombres de encabezado se validan por adelantado en ambas rutas - CR,
  LF y NUL (los bytes de inyección, que el transporte de Mailgun ya
  rechaza) y cualquier cosa que no sea un nombre de campo RFC 5322
  válido (espacios, dos puntos, no ASCII) - así que adjuntar un archivo
  nunca cambia si un mensaje se acepta.

### Corregido

- **Los fallos de validación anidados ahora llegan al cuerpo del 422.**
  Los fallos de `#[validate(nested)]` en un struct anidado o en un
  elemento de un `Vec<T>` validado se perdían entre el validador y la
  respuesta: la solicitud se rechazaba correctamente con 422, pero el
  mapa `errors` volvía vacío, así que no se renderizaba ningún mensaje y
  el cliente no podía saber qué campo tenía la culpa. Los fallos
  anidados ahora se aplanan a la notación con puntos de Laravel -
  `address.street`, `items.1.name`, `order.items.2.sku` - junto a los de
  nivel superior.

- **El `url` del objeto de página de Inertia conserva el query string.**
  `page.url` era solo la ruta de la solicitud, así que el cliente
  registraba `/users` para una visita a `/users?page=2&sort=name`. Cada
  navegación de atrás/adelante y cada `router.reload()` reproducían
  entonces la página sin su cursor de paginación, su orden ni sus
  filtros. Ahora es la ruta más el query string - la misma derivación
  que `InertiaVersionMiddleware` ya usaba para `X-Inertia-Location`, de
  modo que por defecto ambas coinciden byte a byte. El nuevo
  `InertiaConfig::url_resolver(...)` cambia cómo nombra la página el
  *objeto de página* (el `Inertia::resolveUrlUsing` de Laravel); el
  rebote de versión sigue nombrando la URL que llegó, porque esa es la
  URL que el navegador tiene que solicitar.

- **`Inertia::install` ahora aplica su config a todas las respuestas.**
  La config que se le pasaba a `Inertia::install` se leía para tres
  campos y luego se descartaba, así que cada `InertiaResponse`
  construido sin un `.with_config(...)` explícito se renderizaba desde
  `InertiaConfig::default()`. Una aplicación con andamiaje creada con
  `--frontend react` servía el punto de entrada de Svelte y ningún
  preámbulo de refresh de React salvo que `SUPRNOVA_FRONTEND` estuviera
  establecida en el entorno; el SSR habilitado en la config nunca
  llegaba a una respuesta; y la versión de assets del objeto de página
  venía de una config distinta a la del resolver del middleware de
  versión. La config instalada ahora se conserva en el registro de
  Inertia del contenedor y es desde donde parte `InertiaResponse::new`.
  El `.with_config(...)` por respuesta sigue teniendo prioridad, las
  aplicaciones que nunca llaman a `Inertia::install` no cambian, y una
  instalación fallida (que falla cerrado) no conserva nada. Como efecto
  secundario, el manifiesto de Vite de producción ahora se analiza una
  vez por proceso en lugar de una vez por respuesta.

- **Las aplicaciones con andamiaje ahora instalan los middlewares del
  protocolo de Inertia.** El `bootstrap.rs` que escribe `suprnova new`
  registraba los middlewares de sesión, locale, CSRF e include pero
  nunca llamaba a `Inertia::install`, así que una aplicación generada no
  tenía ni `InertiaVersionMiddleware` ni `Inertia303Middleware`: a un
  navegador que aún ejecutaba el bundle anterior nunca se le decía que
  recargara tras un despliegue, y un `PUT`/`PATCH`/`DELETE` que
  redirigía se quedaba en un `302` que el cliente podía seguir con el
  verbo original. La llamada ahora aterriza después de
  `SessionMiddleware` - donde el middleware de versión sí puede volver a
  poner la sesión en flash - con una constante `INERTIA_VERSION` con
  nombre que hay que subir cuando cambian los assets, y fija el frontend
  con el que se generó el proyecto (`.frontend(Frontend::React)` para
  `--frontend react`), de modo que el shell HTML carga el punto de
  entrada de Vite de ese framework en vez de recaer en el de Svelte. El
  `.env` generado ahora establece `SUPRNOVA_FRONTEND` para que coincida.
  El starter `--api` no cambia; no tiene frontend.

- **`Queue::push_unique` ya no informa de un job encolado como omitido.**
  El valor de retorno se calculaba con
  `matches!(outcome, Idempotent::Fresh(()))`, que plegaba
  `Idempotent::FreshUnfenced` a `false` - el resultado en el que el
  sobre *sí* se empujó pero el lease de deduplicación se perdió a mitad
  del push. A quien ramificaba sobre ese booleano se le decía que un job
  que estaba a punto de ejecutarse había sido suprimido por duplicado.
  Los tres resultados ahora se contemplan de forma exhaustiva: un lease
  perdido devuelve `true` con un `warn` que nombra el job y su clave
  única, y solo un duplicado real devuelve `false`. `push_unique_later`
  y `later_unique` comparten la ruta y quedan corregidos con ella.

### Cambiado


- **La línea base de paridad pasa a Laravel 13.25.0.** Las notas de
  lanzamiento de 13.23.0, 13.24.0 y 13.25.0 se rastrearon punto por
  punto hasta la propia superficie del framework. Todo lo que llegó a
  una ruta de código de Suprnova está corregido en esta versión o tiene
  una fila en [`parity.md`](parity.md) marcada como
  `not yet` o `by design no`.

### Actualización

Dos cambios pueden alterar una aplicación en ejecución sin ningún cambio
de código de tu parte.

- **Los ajustes de la config que le pasas a `Inertia::install` ahora
  surten efecto.** Se leían para tres campos y se descartaban. Si tu
  config de instalación establece `.ssr(...)`, el SSR está ahora
  activado: arranca el worker (`suprnova ssr:start`) antes de desplegar,
  o quita la llamada a `.ssr(...)`. `.entry_point`, `.assets_base_url`,
  `.default_title` y `.encrypt_history(...)` establecidos ahí también
  llegan ahora a la página.

- **`rules::Url` rechaza más cosas.** Valores que antes pasaban y ya no:
  cualquier esquema fuera de la lista de permitidos de Laravel,
  `javascript:` y `vbscript:` entre ellos; `mailto:`, `data:` y `tel:`,
  que están en la lista de permitidos pero no llevan host tras `://`; y
  `scheme://` con un host vacío, como `file:///path`. Si tu intención
  era aceptar un esquema, nómbralo: `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Corregido

- **Los casts de fecha y hora ahora leen el texto `CURRENT_TIMESTAMP` nativo
  de la base de datos.** `AsDateTime`, `AsImmutableDateTime` y
  `AsOptionalDateTime` siguen escribiendo RFC-3339 canónico, pero las lecturas
  también aceptan el texto de PostgreSQL con zona horaria y los valores de
  SQLite/MySQL sin zona. Los valores sin zona se interpretan como UTC.

## 1.2.2 - 2026-08-14

### Corregido

- **Los valores anulables no textuales ahora funcionan en todas las
  escrituras basadas en atributos en PostgreSQL.** `Builder::update_all` y
  `Builder::upsert` tipados, `DB::table().insert/update` sin modelo y los
  atributos adicionales de pivots many-to-many emiten los nulls JSON
  explícitos como `NULL` de SQL, mientras siguen vinculando todos los valores
  no nulos. Esto conserva el tipo de la columna de destino en vez de enviar un
  parámetro null tipado como texto que PostgreSQL rechaza para columnas bigint,
  integer, boolean, timestamp y otras no textuales. Los upserts de varias filas
  ahora también rechazan columnas ausentes o adicionales en vez de convertir
  silenciosamente una fila mal formada a null. Los timestamps automáticos de
  pivots many-to-many se vinculan como datetimes UTC tipados en vez de texto.

### Seguridad

- **La puerta de lanzamiento ahora distingue los metadatos inactivos del
  lockfile de las dependencias compiladas en todo el workspace.** Cargo
  registra la dependencia de compatibilidad opcional rkyv 0.7 no utilizada de
  rust_decimal en `Cargo.lock`; la puerta ahora demuestra que ni rkyv ni su
  crate de derivación son alcanzables desde ningún miembro del workspace,
  feature, target o arista de dependencia. La excepción correspondiente de
  RustSec es responsabilidad del proyecto, expira el 2026-11-14 y debe
  eliminarse cuando rust_decimal deje de registrar esa dependencia opcional
  heredada.

## 1.2.1 - 2026-08-09

### Cambiado

- **Suprnova se trasladó de la organización de GitHub `entrepeneur4lyf` a
  `eas4ai`.** Las URL del
  repositorio en los metadatos de paquetes, la documentación, los ejemplos de
  dependencias y las plantillas de andamiaje ahora usan `github.com/eas4ai`.
  Los proyectos nuevos también usan el correo de autor supervisado
  `shawn@eas4ai.com`. Esta versión no cambió el comportamiento en runtime.

## 1.2.0 - 2026-08-05

### Añadido

- **El manual se distribuye en siete idiomas.** `manual/es/`, `manual/fr/`,
  `manual/de/`, `manual/pt-BR/`, `manual/ja/` y `manual/zh-Hans/` llevan
  cada uno el manual completo de 104 capítulos - cada capítulo, la tabla
  de contenidos y este registro de cambios - traducido desde la fuente en
  inglés. El inglés sigue siendo canónico: la estructura de los capítulos,
  los bloques de código, los identificadores, los comandos de CLI y las
  variables de entorno se mantienen byte a byte idénticos a la fuente,
  así que un capítulo traducido nunca puede discrepar del inglés sobre lo
  que hace el framework - solo decirlo en el idioma del lector.

  Las traducciones se produjeron y revisaron para suprnova.app, que
  renderiza este manual como su `/docs`. Cada sección lleva allí un
  registro de revisión: los veredictos se registran contra hashes de
  contenido tanto del inglés como de la traducción, dos revisores
  independientes deben aprobar los bytes exactos para que una sección
  cuente como aprobada, y los glosarios por idioma fijan las decisiones
  de terminología (qué términos quedan en inglés, cuáles toman la
  palabra nativa, y por qué). Las correcciones son bienvenidas en
  cualquiera de los dos repositorios - un arreglo aquí llega al sitio en
  su siguiente sincronización.

## 1.1.0 - 2026-08-02

### Añadido

- **Cadenas de fallback por locale.** `LocalizationConfig` gana
  `parents` (`APP_LOCALE_PARENTS`, pares `child=parent` separados por
  comas, o el builder encadenable `.parent(child, parent)`): un locale
  puede heredar de un hermano configurado antes de caer más atrás
  hasta el `fallback_locale` global - `pt-PT` de `pt-BR`, `en-AU` de
  `en-GB`, y así sucesivamente, de forma transitiva.
  `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has` recorren todos
  la cadena, el locale actual primero, así que esto funciona para
  cualquier driver `Translator`, no solo el incluido. Un par mal
  formado, un locale inválido, un hijo nombrado dos veces, o un ciclo
  (incluyendo un locale que se nombra a sí mismo como su propio padre)
  falla de forma estrepitosa al cargar la configuración en lugar de
  degradarse en tiempo de solicitud.

  Los catálogos servidos se mantienen aplanados de antemano:
  `FluentTranslator` ahora construye el catálogo
  `/_suprnova/lang/<locale>.ftl` de cada locale como un pliegue - el
  catálogo del framework incrustado en la base para los locales
  `en`/`en-*`, luego la cadena de padres configurada del locale, luego
  sus propios archivos `*.ftl` - así que un locale encadenado sigue
  siendo un único archivo autocontenido que el navegador obtiene una
  sola vez, sin necesitar conciencia de la cadena del lado del
  cliente. El aplanado cubre solo los padres configurados; el
  `fallback_locale` terminal sigue siendo un fallback a nivel de la
  fachada `Lang`, no horneado en los bytes servidos.

  Esto hace prácticos los catálogos de tipo delta: un directorio
  `lang/pt-PT/` puede contener solo el puñado de strings que de
  verdad difieren de `lang/pt-BR/`, en lugar de un catálogo duplicado
  completo. La fusión que lo hace posible funciona a nivel del AST de
  Fluent - el valor de un hijo reemplaza el del padre, los atributos
  se fusionan por nombre (un override que no menciona un atributo ya
  no lo pierde), las expresiones select se reemplazan enteras (las
  categorías de plural CLDR dependen del locale, así que fusionar
  variante por variante no sería coherente), y las entradas
  exclusivas del hijo se añaden. Consulta la nueva sección "Cadenas de
  fallback" de `manual/localization.md` para el contrato completo.

### Cambiado

- **`LocalizationConfig` ganó el campo `parents`.** `from_env()` y el
  builder no se ven afectados; un constructor de struct literal (tests
  que construyen un `LocalizationConfig` a mano) necesita un campo
  más.
- **El texto de catálogo servido ahora está normalizado por el
  serializador para cada locale**, y la fusión multiarchivo
  intra-locale (varios archivos `.ftl` en un directorio de locale)
  ahora pasa por la misma fusión a nivel de AST que las cadenas de
  padres, en lugar de la simple sobrescritura de bundle. Las
  traducciones resueltas no cambian salvo por las dos mejoras
  estrictas de abajo; los bytes subyacentes rotan de todos modos -
  `ETag`/`?v=<hash>` rota una vez tras la actualización. Las mejoras:
  un override ya no descarta en silencio los atributos que no
  menciona, y un override de solo atributos ya no elimina el valor
  propio del mensaje (antes era un error o una resolución de
  fallback; ahora resuelve al valor del override anterior).

## 1.0.0 - 2026-08-02

### Añadido

- **Localización.** Catálogos de mensajes en `lang/<locale>/*.ftl`
  ([Fluent](https://projectfluent.org)), una fachada `Lang` con la
  macro `__!("key", name: value)`, detección de locale por
  solicitud (`LocaleMiddleware`: sesión → cookie →
  `Accept-Language` → `APP_LOCALE`), y formateo consciente del locale
  para números, moneda, fechas, horas, listas, y tiempos relativos
  sobre ICU4X. `manual/localization.md` es el capítulo.

  Las reglas de validación integradas dejan de fijar el inglés como
  literal. Cada una devuelve un mensaje con clave (`validation-min`
  más sus argumentos y un fallback en inglés), traducido una sola vez
  en el límite de serialización - así que una app en español obtiene
  errores de validación en español con solo añadir
  `lang/es/validation.ftl`, sin envolver reglas y sin una copia
  bifurcada de los mensajes del framework. Los nombres de campo se
  humanizan mediante una búsqueda `field-<name>`. `Rule::passes` (y
  `ContextualRule` / `AsyncRule`) ahora devuelven
  `Result<(), ValidationMessage>`; el cuerpo `Err("…".into())` de una
  regla personalizada sigue compilando y sigue renderizando
  textualmente, pero la firma de tu `impl` necesita el nuevo tipo.

  El navegador obtiene los mismos bytes que resolvió el servidor: el
  catálogo fusionado se sirve en `/_suprnova/lang/<locale>.ftl` con
  un ETag y una forma `?v=<hash>` inmutable, los tres starter kits lo
  parsean con `@fluent/bundle`, y `suprnova generate-types` emite una
  unión `MessageKey` para que renombrar un mensaje apunte al
  compilador de TypeScript hacia cada sitio de llamada.

  Fluent en lugar de arrays PHP al estilo Laravel porque un solo
  formato tiene que servir tanto al servidor como al navegador, y
  porque las categorías de plural CLDR son lo que hace correctos al
  ruso, al polaco y al árabe - los rangos enteros de `trans_choice`
  no pueden, que es por lo que aquí no existe `trans_choice`. Detrás
  de una feature `localization` activada por defecto;
  `--no-default-features` sigue compilando y sigue validando, usando
  los fallbacks en inglés incrustados.

- **`IntoInertiaScroll` para `Paginator`.** El trait estaba
  implementado para `LengthAwarePaginator` y `CursorPaginator` pero
  no para el paginador simple, así que los resultados de
  `simple_paginate` no podían alimentar `Inertia::paginate` en
  absoluto - a pesar de que los propios docs de módulo de `simple.rs`
  lo señalan como la ruta de generación de URL. Eso dejaba a las
  colecciones Inertia paginadas por offset ante una elección entre un
  `COUNT(*)` por solicitud y construir a mano los metadatos de
  scroll. `next_page` viene de la sonda de desbordamiento
  `LIMIT n+1` en lugar de una última página calculada, dado que no
  hay total del que calcular una.

### Corregido

- **`suprnova generate-types` emitía un archivo distinto en cada
  ejecución.** El orden topológico sembraba su cola de trabajo
  iterando un `HashMap`, y Rust aleatoriza el orden de iteración de
  hash por proceso, así que ejecuciones consecutivas ordenaban las
  mismas interfaces de forma distinta. La salida es un artefacto
  versionado, así que cada ejecución producía un diff - y un archivo
  generado que cambia sin motivo es uno que la gente deja de
  regenerar, tras lo cual deja de describir en silencio el Rust que
  dice describir. El recorrido de directorios también está ordenado,
  así que la salida tampoco depende ya del orden del sistema de
  archivos. Dos ejecuciones sobre la misma fuente ahora son
  idénticas byte a byte.

- **`topological_sort` hacía lo contrario de lo que decía su
  comentario de documentación**, emitiendo los dependientes antes que
  las dependencias. Inofensivo - una interfaz de TypeScript puede
  referenciar otra declarada más adelante en el mismo archivo - así
  que se corrige el comentario en lugar del orden, lo cual habría
  reordenado sin ningún beneficio un archivo versionado.

## 0.9.1 - 2026-08-01

Tres defectos, todos encontrados al ejecutar la app dogfood bajo un
harness contenerizado en lugar de leyendo el código. Cada uno de
ellos es invisible para una suite de tests que nunca detiene un
proceso de la forma en que producción lo detiene.

Se combinan en un orden específico: un despliegue progresivo manda
SIGKILL a un worker en medio de un job (el primero), y ese job
entonces toma una ruta de reclamo que nunca contó el intento (el
segundo).

### Corregido

- **`schedule:work`, `queue:work` y `workflow:work` ignoraban
  SIGTERM.** Cada uno seleccionaba solo sobre
  `tokio::signal::ctrl_c()`, que instala un handler de SIGINT - así
  que SIGTERM no tenía handler en ninguna parte del proceso, y
  SIGTERM es lo que envían `docker stop`, Coolify, systemd y
  Kubernetes. Los tres ya tenían un drenado acotado y cuidadoso
  detrás de ese `select!`; nunca se había ejecutado bajo un
  supervisor. Medido antes de la corrección: un `docker stop` sobre
  un contenedor `queue:work` agotaba toda su ventana de gracia de
  40s y salía con 137 con el job en curso destruido. Como PID 1 - que
  es lo que ejecuta un contenedor - el kernel descarta sin más un
  SIGTERM sin manejar, así que el proceso no moría mal; no moría en
  absoluto hasta el SIGKILL. `Server::run` ya manejaba ambas señales
  correctamente y ahora comparte su socket de escucha, lo cual además
  cierra una ventana de señal perdida en el bucle del planificador.

- **Un job que mataba a su worker nunca podía enviarse a fallidos.**
  Un job cuyo *handler* falla recibe un nack y su intento se cuenta,
  así que se envía a fallidos tras `max_tries`. Un job que *mata a su
  worker* - OOM, abort, segfault, o el SIGKILL de arriba - no resuelve
  nada; su reserva simplemente caduca, y todos los drivers solían
  reentregarlo idéntico byte a byte. Un job así es inmortal: mata a
  cada worker que lo reclama, vuelve sin cambios, y mata al
  siguiente, mientras algo siga reiniciando workers. Los tres drivers
  ahora contabilizan el intento en el momento en que se enteran de
  que un worker murió, porque cambiar `QUEUE_DRIVER` no debe cambiar
  si un trabajo envenenado puede detenerse. `attempts` ahora
  significa "entregas a un worker" en lugar de "fallos del handler" -
  documentado en `manual/queues.md`, porque un worker perdido por
  motivos ajenos también gasta un intento.

- **...y el job agotado ahora se envía a fallidos antes de
  despacharse.** Contar el intento era necesario pero no suficiente.
  Cada decisión de enviar a fallidos vivía en la ruta de resolución
  del worker, la cual asume que el handler retorna - así que nunca se
  ejecutaba justo para los jobs que no podían retornar. Con solo la
  corrección del driver, el contador subía (medido: 0 → 1 → 2 a
  través de tres workers matados) y nada actuaba sobre ello. El
  presupuesto ahora se gasta antes de que el handler se ejecute.
  Detectado solo al volver a ejecutar el experimento del contenedor
  después de que la primera corrección pareciera correcta.

- **Los demonios no tenían subscriber de tracing.** `serve` obtiene
  uno de `init_telemetry`; `queue:work`, `schedule:work`,
  `schedule:run` y `workflow:work` pasan por una ruta de arranque
  distinta y no obtenían nada, así que cada línea `tracing::` que
  emitían no iba a ninguna parte y `LOG_LEVEL` era inerte para ellos.
  Eso es la mayor parte de lo que tienen que decir - un worker
  enviando un job a fallidos, un planificador saltándose un tick que
  perdió, un bloqueo que no pudo liberar. En un contenedor la única
  salida visible era el banner de arranque, y el proceso parecía
  inactivo mientras hacía todo eso. Dos de los defectos de este
  lanzamiento fueron invisibles hasta que esto se corrigió.

- **Un envío a fallidos sin store de jobs fallidos vinculado era una
  eliminación silenciosa.** El paso de persistencia estaba dentro de
  un `if let Some(store) = ..`, así que sin store la rama no
  coincidía y la ejecución caía hasta el ack - más silencioso que la
  ruta de fallo justo encima, que al menos deja la reserva intacta.
  Un store ausente se trataba como más exitoso que uno roto. Ahora
  registra el sobre completo en ERROR, porque eso es lo que
  `queue:retry` reencola: la diferencia entre trabajo recuperable a
  mano y trabajo que dejó de existir.

- **`QUEUE_DRIVER=database` ahora vincula un store de jobs fallidos.**
  `failed_jobs` es parte del contrato de ese driver - `queue:retry` lo
  lee y `Queue::retry_failed` no puede funcionar sin él - pero
  `bootstrap_from_env` conectaba el driver y dejaba el store sin
  establecer, así que una cola respaldada por base de datos enviaba a
  fallidos hacia la nada a menos que la app vinculara uno a mano.
  Configurable mediante `QUEUE_FAILED_DB_TABLE`. Solo para este
  driver: `memory` es efímero por construcción y `redis` no tiene
  tabla en la que escribir.

- **La latencia de reclamo de Redis ahora sigue a
  `--visibility-timeout`.** El flag fija el umbral de inactividad de
  XAUTOCLAIM, pero un reloj separado gobierna con qué frecuencia mira
  un consumidor, y el driver lo dejaba en el valor por defecto de 30s
  de sea-streamer - así que `--visibility-timeout 5` en realidad
  significaba "hasta 35 segundos". El intervalo ahora sigue el
  timeout configurado, acotado a 1s..=30s para que un timeout corto
  no pueda convertirse en una tormenta de XAUTOCLAIM y uno largo solo
  pueda hacer el reclamo más rápido que antes.

### Añadido

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** -
  ejecuta una tarea programada exactamente una vez por cada tick
  vencido, a través de las réplicas. Sin esto nada elige un líder
  para un tick: cada proceso `schedule:work` evalúa la programación
  de forma independiente, y se midieron tres réplicas ejecutando
  cada tarea vencida tres veces, cada minuto, sin variación. Un job
  de facturación nocturna en tres réplicas facturó a cada cliente
  tres veces.

  `without_overlapping()` no cubre esto y no puede: su bloqueo usa
  como clave la tarea y se libera cuando el handler retorna, así que
  una tarea rápida lo libera antes de que una segunda réplica mire.
  `on_one_server` usa como clave la tarea *y el tick* y sostiene el
  bloqueo más allá del handler, dejándolo expirar por TTL. Los dos se
  combinan.

  Opt-in, igual que Laravel. Diverge de Laravel al fallar cerrado: la
  elección es solo tan compartida como la caché detrás de ella, así
  que un arranque de producción con `CACHE_DRIVER=memory` y una tarea
  de un solo servidor se rechaza, nombrando las tareas responsables,
  con `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` para los
  despliegues que de verdad ejecutan un solo planificador.

### Cambiado

- `manual/deployment.md` ya no dice "ejecuta exactamente un proceso
  `schedule:work`" como única opción, y gana una sección
  **Detención limpia** que cubre las ventanas de drenado por
  subsistema, cómo dimensionar la gracia de terminación de una
  plataforma por encima de ellas, y por qué PID 1 hace que un
  handler de señal ausente sea peor de lo que suena.

## 0.9.0 - 2026-07-31

### Seguridad

- **La emisión de autenticación solo se podía limitar por quien llama,
  nunca por destinatario.** Un límite con clave de dirección responde
  "¿es un cliente ruidoso?"; no puede responder "¿se está inundando un
  buzón?". Un atacante repartido en una botnet o en un único `/64` de
  IPv6 se mantenía por debajo de cualquier presupuesto por IP mientras
  llenaba la bandeja de entrada de una víctima con correo de
  restablecimiento de contraseña, y nada en el framework podía
  expresar el límite que lo habría detenido - una función de clave
  podía leer la ruta, los encabezados, y el query string, pero no un
  cuerpo form-encoded, así que la dirección era invisible justo en la
  ruta que la lleva.

  `identity_key` indexa un cubo sobre la cuenta que está siendo objeto
  de la acción. Lee primero el query string y luego un cuerpo de
  formulario almacenado en búfer, así que una sola función de clave
  cubre ambas formas; el valor se recorta y se pone en minúsculas,
  porque `Alice@Example.com` llega al mismo buzón que
  `alice@example.com` y un límite que se elude manteniendo pulsada la
  tecla shift no es un límite; y se le aplica hash, porque un backend
  de límite de velocidad suele ser un Redis compartido con un control
  de acceso más débil que la base de datos primaria.

  Dos nuevos constructores de middleware lo respaldan.
  `key_reads_body(cap)` almacena el cuerpo en búfer antes de usarlo
  como clave - opt-in, porque almacenar en búfer es trabajo que un
  llamador no autenticado te hace hacer, y un cuerpo por encima del
  tope se rechaza con 413 en lugar de dejarlo pasar sin clave.
  `only_when(pred)` se salta un limitador por completo para
  solicitudes sobre las que no tiene nada que decir, que es lo que
  evita que un presupuesto por destinatario apilado se convierta en
  silencio en el límite vinculante en rutas que no nombran
  destinatario.

  La app dogfood ahora apila ambos en su grupo de emisión: 10 cada 5
  minutos por dirección, 3 cada 15 minutos por destinatario.

Una revisión de las rutas de sesión, contraseña, OAuth, y passkey de
Torii encontró ocho defectos, todos corregidos en el fork fijado
(`suprnova-torii-rs` `968b0be`).

- **Las sesiones expiradas se podían refrescar de vuelta a la vida.**
  El `refresh` del repositorio de sesión de SeaORM no tenía predicado
  de expiración y extendía `expires_at` incondicionalmente, y
  `OpaqueSessionProvider::refresh_session` se saltaba la comprobación
  `is_expired()` que hace `get_session`. Un token retenido más allá de
  su expiración se podía renovar indefinidamente. Corregido en ambas
  capas. No alcanzable a través de la propia superficie de Suprnova -
  ni `Torii` ni el framework exponen el refresco de sesión - pero es
  API pública de ambos crates.
- **El formulario de login filtraba, por temporización, qué cuentas
  existen.** La autenticación retornaba en cuanto el email no
  coincidía, saltándose Argon2 por completo: medido en 54µs para una
  dirección desconocida frente a 719ms para una contraseña incorrecta,
  una brecha de ~13.000x legible a través de la red. Ambas rutas de
  fallo ahora verifican contra un hash señuelo para que cuesten lo
  mismo. Esta *sí* era alcanzable a través del login por contraseña de
  Suprnova.
- **El claim `iss` del JWT se escribía pero nunca se verificaba.** La
  fijación de algoritmo ya era correcta - `alg: none` y la confusión
  HS/RS nunca fueron posibles - pero el emisor era decorativo, así que
  dos servicios que compartieran una clave de firma aceptarían las
  sesiones el uno del otro. Ahora se hace cumplir cuando hay un emisor
  configurado.
- **Un verificador PKCE de un solo uso se podía reclamar dos veces.**
  El consumo era una lectura seguida de una eliminación, así que dos
  callbacks de OAuth para el mismo `csrf_state` podían ambos leerlo
  antes de que aterrizara ninguna de las dos eliminaciones. Ahora se
  reclama en una sola operación - `DELETE ... RETURNING` en Postgres,
  una eliminación por clave primaria cuyo conteo de filas afectadas
  decide el ganador en SeaORM.
- **Las sesiones expiradas se listaban como activas.**
  `find_by_user_id` no tenía filtro de expiración, y las filas
  expiradas sobreviven hasta que se ejecuta la limpieza, así que una
  pantalla de "dispositivos en los que iniciaste sesión" ofrecía a los
  usuarios sesiones muertas para revocar sin decir nada sobre la que
  sí estaba viva.
- **Una búsqueda de passkey se llamaba `authenticate`.**
  `PasskeyService::authenticate_credential` de Torii tomaba un ID de
  credencial y devolvía el usuario propietario, y
  `PasskeyAuth::authenticate` acuñaba una sesión a partir de eso.
  Torii almacena passkeys - no lleva ninguna dependencia de WebAuthn y
  no puede verificar una aserción, así que lo único que esas llamadas
  probaban era que quien llamaba conocía un ID de credencial: un
  valor que el navegador envía en claro y que `allowCredentials`
  entrega a cualquiera que pueda iniciar una ceremonia. Renombradas a
  `find_user_by_credential` y `create_session_for_verified_credential`,
  ambas documentando que la verificación es responsabilidad de quien
  llama. No alcanzable a través de Suprnova, que maneja `webauthn-rs`
  por sí mismo (ver `torii_integration::passkey`) y solo llega a Torii
  para el almacenamiento de credenciales.
- **Un challenge de WebAuthn era repetible durante todo su TTL.**
  Ningún backend consumía un challenge al leerlo, y el `get_challenge`
  de SeaORM también ignoraba `expires_at` por completo, devolviendo
  challenges expirados como vigentes. Las lecturas ahora excluyen
  filas expiradas en ambos backends, y un nuevo `take_challenge`
  reclama uno exactamente una vez - la misma forma de
  eliminación-decide-el-ganador que la corrección de PKCE.

### Cambios incompatibles

- **Azure Blob Storage y Google Cloud Storage se movieron detrás de
  las nuevas features `filesystem-azure` y `filesystem-gcs`.**
  `Storage::register_azblob`, `register_azblob_with`, `register_gcs`,
  `register_gcs_with`, `AzBlobConfig` y `GcsConfig` ya no existen a
  menos que actives la feature correspondiente. Si usas alguno de los
  dos backends, añádelo a tu dependencia:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  Obtienes un error de compilación que nombra el elemento faltante,
  no un fallo en tiempo de ejecución.

  Ambos crates de servicio de opendal arrastran `rsa`, que lleva
  RUSTSEC-2023-0071 (el ataque de temporización Marvin) sin ninguna
  versión corregida upstream. Eran los únicos crates que activaban
  `reqsign-core/jwt`, la feature detrás de la cual está el `rsa`
  opcional de `reqsign-core`, así que ponerlas detrás de una feature
  corta de golpe las tres rutas de opendal hacia él. `rsa` ahora es
  *evitable*: `--no-default-features --features
  filesystem,database-postgres` resuelve sin él y sigue teniendo el
  subsistema de almacenamiento. Antes ninguna combinación de features
  podía deshacerse de él conservando siquiera el almacenamiento.

  Un build por defecto de fábrica todavía arrastra `rsa` -
  `database-mysql` es una feature por defecto y `sqlx-mysql 0.8.6`
  depende de él de forma no opcional - así que la excepción de
  auditoría sigue abierta. S3 deliberadamente **no** está detrás de
  una feature: `reqsign-aws-v4` toma `reqsign-core` sin `jwt`, así que
  el driver de S3 nunca aportó una ruta hacia él, y ponerlo detrás de
  una feature rompería el backend de nube más usado sin eliminar
  nada.

### Añadido

- **`suprnova --version`**, con `-v` además de la `-V` por defecto de
  clap. Pedirle su versión a una CLI con el flag que usa cualquier
  otra CLI no debería imprimir un error de uso.

### Corregido

- **Dos operaciones de Redis no tenían cota superior.** El vaciado de
  tags de la caché leía todo el conjunto de miembros de un tag con
  `SMEMBERS` y eliminaba clave por clave, así que un tag con una
  membresía grande estancaba la conexión y una escritura concurrente
  se podía perder entre la lectura y la eliminación; los tags ahora se
  basan en generación, se vacían de forma atómica, y se escanean con
  un `SSCAN` acotado. El paso de promoción de la cola retrasada movía
  cada job vencido en un único `ZRANGEBYSCORE` sin cota, así que un
  backlog que vencía junto producía un único script enorme; ahora
  promociona por lotes.
- **Dos drenados de apagado esperaban para siempre.** Tanto
  `schedule:work` en Ctrl-C como el worker de flujo de trabajo tras la
  cancelación esperaban (`await`) cada tarea en curso sin plazo, así
  que una tarea que nunca retornaba mantenía el proceso abierto hasta
  el `SIGKILL` - un operador ve un demonio que "no se detiene". Ambos
  ahora esperan una gracia acotada, luego abortan lo que queda y
  reportan el conteo.
- **El barrido de fijación de versión del lanzamiento solo reconocía
  una de las dos sintaxis de fijación**, así que nunca se descubría
  ningún archivo que llevara una línea `cargo install --tag vX.Y.Z`
  sin fragmento de dependencia. `suprnova-cli/README.md` llevaba tres
  lanzamientos diciéndoles a los lectores que instalaran v0.6.0;
  `manual/cli.md` y `manual/cli-new.md` se quedaban en v0.7.2;
  `manual/installation.md` llevaba ambas formas y tenía una
  actualizada mientras la otra se congelaba. El descubrimiento y la
  reescritura ahora leen de una única tabla de patrones, y las reglas
  de un archivo se derivan de su contenido.
- **`cargo doc` fallaba para cualquier build con `filesystem` pero sin
  `testing`** - siete enlaces intra-doc de `Storage::fake` no podían
  resolverse, y `lib.rs` deniega los enlaces rotos. `testing` es una
  feature por defecto, así que ningún paso de la puerta había
  construido nunca esa combinación; `check-feature-matrix.sh` ahora sí
  lo hace.
- **Las migraciones de Torii no se podían reproducir sobre su propio
  esquema**, así que una base de datos que la tuviera sin la tabla de
  seguimiento `torii_migrations` - restaurada desde un dump que se la
  saltó, o migrada a mano - no se podía traer bajo gestión. Todo
  `Table::create()` llevaba `.if_not_exists()`; ninguna de las 19
  llamadas a `Index::create()` lo llevaba, ni tampoco el alter
  `ADD COLUMN locked_at`, así que la reproducción navegaba por las
  tablas y moría en el primer `CREATE INDEX`. Corregido en el fork
  fijado (`suprnova-torii-rs` `a0f956d`) mediante `has_index` /
  `has_column` en lugar de `IF NOT EXISTS`, que sea-query descarta en
  silencio para MySQL - la corrección sintáctica habría dejado roto un
  build con las features por defecto.
- **Una migración de Torii fallida abortaba el proceso en lugar de
  devolver un error.** `SeaORMStorage::migrate` hacía unwrap del
  migrador y devolvía `Ok(())` incondicionalmente, así que el mapeo
  que hace `init_torii` del fallo a un `FrameworkError` era código
  inalcanzable.
- **La propia tabla `users` de una app suprimía en silencio la de
  Torii**, porque `.if_not_exists()` no puede distinguir "ya es mía"
  de "ya es de alguien más". La migración reportaba éxito y la
  autenticación fallaba más tarde por una columna faltante - la razón
  por la que el starter `--api` nombra su tabla `app_users`. La
  migración de Torii ahora avisa en tiempo de migración cuando una
  tabla `users` existente carece de columnas que necesita, nombrando
  las columnas y el remedio. Se queda como aviso en lugar de fallo
  duro para que los despliegues existentes sigan arrancando.
- **Las guías de despliegue de Railway y DigitalOcean apuntaban la
  comprobación de salud de la plataforma a una ruta que podía sondear
  Postgres.** Ambas plataformas reinician el contenedor cuando esa
  comprobación falla, así que seguir el consejo convertía un parpadeo
  de la base de datos en un bucle de reinicio en cada réplica. Ambas
  ahora usan `/_suprnova/health/live`, con la base de datos sondeada a
  mano desde la consola. Las rutas heredadas siguen resolviendo; nada
  de lo ya desplegado necesita cambios.

## 0.8.0 - 2026-07-30

Remediación de una auditoría externa de red team. La auditoría
devolvió 19 hallazgos P1 y un veredicto NO-GO para el 1.0; este
lanzamiento cierra **los diecinueve**, más varios defectos
encontrados al corregirlos que la auditoría no había nombrado.

Varias correcciones convierten deliberadamente una config mal hecha
en silencio en un arranque rechazado. Lee **Actualización** antes de
desplegar - una app de producción que ha estado funcionando
felizmente podría no arrancar.

### Actualización

Tres configuraciones que antes arrancaban con una advertencia (o en
silencio) ahora fallan cerrado en producción. Cada error nombra la
variable que lo desbloquea, y cada una tiene una anulación explícita
para el despliegue donde el riesgo está genuinamente ausente.

- **Un driver de correo que no entrega.** `MAIL_DRIVER` sin
  establecer, `log`, `memory`, o un valor no reconocido resolvían
  todos a un transporte que renderiza el correo y lo descarta - así
  que los restablecimientos de contraseña reportaban éxito sin que se
  enviara nada. Anulación: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **SMTP en texto claro.** Tres de las cuatro combinaciones de
  credenciales aterrizaban en un transporte sin cifrar, y el caso de
  ambas sin establecer registraba una advertencia y enviaba de todos
  modos. Anulación: `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **El limitador de velocidad en memoria.** Sus cubos viven en el
  heap de un proceso, así que detrás de N réplicas cada cuota es en
  realidad N× y cada despliegue las reinicia. Apunta
  `RATE_LIMIT_DRIVER` a `redis`, o establece
  `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true` si de verdad ejecutas
  un único proceso. Un valor de driver *no reconocido* falla por la
  misma razón, porque recaía en memory - `RATE_LIMIT_DRIVER=Redis`,
  con mayúscula, es el caso con más probabilidad de llegar a
  producción porque parece configurado.

Desarrollo, testing y staging quedan intactos en los tres casos.
Staging deliberadamente no está protegido por esta compuerta: hacerla
fallar de forma estricta empuja a los equipos a establecer la
anulación de forma global, lo que desarma la comprobación justo donde
importa.

Dos cambios de comportamiento que no son fallos de arranque:

- **`fill` y `first_or_new` rechazan valores mal formados.** Un valor
  que no se puede decodificar al tipo de su campo solía convertirse
  en el `Default` de ese campo y devolver `Ok` -
  `fill(attrs!{ age: "abc" })` fijaba `age = 0` y reportaba éxito.
  Ahora devuelve un `ValidationError` que nombra el campo, y deja el
  modelo intacto. Las columnas desconocidas se siguen saltando en
  silencio (paridad con Laravel), y el ensanchamiento numérico sigue
  funcionando.
- **`/_suprnova/health?db=true` ya no devuelve el error del driver.**
  El detalle se mueve al registro; el cuerpo conserva
  `"database": "error"`. Los builds de debug lo siguen incluyendo.
  Los dashboards que parsean `status` / `database` no se ven
  afectados.
- **`url::signature_has_not_expired` ahora requiere una firma
  válida**, y está obsoleta. Solía responder `true` para una URL
  falsificada - una firma mala no está "caducada", porque nunca tuvo
  una caducidad que incumplir - así que cualquier handler que
  dependiera solo de ella aceptaba falsificaciones. Ahora es idéntica
  a `has_valid_signature`. Si la usabas para distinguir *caducada* de
  *inválida* (para renderizar "solicita un enlace nuevo" en lugar de
  un 403), cambia a `url::signature_verdict`, que devuelve los tres
  estados. Esto diverge deliberadamente de
  `URL::signatureHasNotExpired` de Laravel.

Dos adiciones que necesitan algo de ti solo si optas por ellas:

- **`QueueDriver` ganó `settle` y `release`**, ambos con
  implementaciones por defecto, así que las implementaciones de
  driver existentes siguen compilando sin cambios. Implementa
  `settle` si tu backend puede confirmar una escritura de seguimiento
  y un ack en una sola transacción; implementa `release` si puede
  reencolar un mensaje reservado in situ.
- **La contabilidad de lotes ahora puede ser durable.**
  `DatabaseBatchRepository` necesita dos tablas nuevas, `job_batches`
  y `job_batch_settlements` - añádelas a tus migraciones, igual que
  `jobs` y `failed_jobs`. El esquema está en `manual/queues.md`. Nada
  cambia si te quedas en `MemoryBatchRepository`.

### Seguridad

- **Slowloris (SEC-07).** El timeout de lectura de encabezados de
  hyper estaba documentado como 30s pero era inerte - solo se arma
  cuando se instala un timer en el constructor de conexión, y no se
  instalaba ninguno. Un cliente podía retener una conexión, y un
  permiso de `SERVER_MAX_CONNECTIONS`, indefinidamente. Ahora armado
  y configurable mediante `SERVER_HEADER_READ_TIMEOUT`.
- **Subidas multipart (SEC-05).** El tope se aplicaba a las cargas de
  cada parte individual pero no al stream crudo, así que un cuerpo
  podía superar el límite en conjunto. Ahora se acota en el stream.
- **HMAC de webhook con clave vacía (SEC-08).** Ambos adaptadores de
  pago aceptaban un secreto en blanco, que verifica cualquier cosa.
  Rechazado en ambos.
- **Parseo de firma de Paddle (P2-11).** Un `paddle-signature` de
  longitud impar o no hexadecimal llegaba al SDK fijado y entraba en
  pánico dentro de él. Ahora se valida primero: una firma mal formada
  es un 401.
- **Inscripción de passkey y tokens de restablecimiento (SEC-01,
  SEC-02).** La inscripción anónima contra un email existente, la
  inscripción por alguien que no es el propietario, y la inscripción
  del propietario sin reautenticación reciente se rechazan cada una
  con estados distintos. Un login por contraseña ahora estampa la
  ventana de reautenticación.
- **`dev:tls` (SEC-10).** Un proyecto podía elegir la CA en la que
  confía el comando.
- **Docker Compose generado (P2-12).** Publicaba Postgres y Redis en
  todas las interfaces con credenciales de las que se hizo commit en
  este mismo repositorio. Ahora enlazado a loopback con contraseñas
  generadas por cada andamiaje, `.env` escrito con 0600, y objetivos
  symlink rechazados.
- **Endpoint de salud (P2-01, CI-05).** Decidía si consultar la base
  de datos con `query.contains("db=true")` - una prueba de subcadena,
  así que `?nodb=true` también ejecutaba la sonda. Ahora se parsea
  correctamente. El 503 ya no incrusta el error del driver, que
  nombraba hosts, puertos, esquemas y versiones.
- **Limitación de la emisión de credenciales (P2-02).** Las cuatro
  rutas de emisión de autenticación en la app de referencia no
  llevaban límite de velocidad en absoluto, y la única ruta que sí lo
  tenía indexaba su cubo sobre el encabezado crudo `x-forwarded-for` -
  que cualquier cliente puede variar por solicitud para conseguir un
  cubo nuevo. Ambas corregidas; el presupuesto de emisión se comparte
  entre las cuatro rutas, así que rotar entre ellas no lo multiplica.
- **Un paso de cadena reentregado volvía a empujar a su sucesor bajo
  un id nuevo (DATA-02b, parcial).** La resolución empuja el
  siguiente eslabón de la cadena *antes* de hacer ack,
  deliberadamente: hacer ack primero significa que una caída en esa
  ventana pierde la cadena de forma permanente, y un duplicado es
  recuperable donde una pérdida silenciosa no lo es. Pero el sobre
  del sucesor recibía un `Uuid::new_v4()` nuevo en cada push, así que
  el duplicado producido por ese intercambio era indistinguible de un
  paso nuevo legítimo - para el driver, para un outbox, y para el
  handler.

  Ese último es el coste real. El contrato de entrega del framework
  es de al-menos-una-vez y su respuesta a los duplicados es "los
  handlers deben ser idempotentes" - pero un handler que usa como
  clave `env.id`, el único identificador que recibe, no podía
  satisfacer ese contrato para un job encadenado, porque el
  duplicado llegaba bajo un id nuevo cada vez. El contrato era
  insatisfacible por construcción.

  El id del sucesor ahora es un UUIDv5 derivado del de su
  predecesor, el cual es estable a través de las propias reentregas
  de ese predecesor. Un paso reentregado vuelve a empujar el id que
  empujó antes. Sin cambio de esquema, sin campo nuevo, sin
  dependencia nueva.

  Esto hace **detectable** al duplicado, que es la primitiva que le
  faltaba al resto de DATA-02b. No hace que el push sea atómico con
  el ack (eso necesita el outbox), y nada rechaza todavía el
  duplicado a la entrada. Ambos siguen abiertos.
- **Las URLs firmadas verificaban una URL y ejecutaban otra
  (SEC-04).** La forma canónica colapsaba los pares de query en un
  mapa, así que una clave repetida conservaba solo su **último**
  valor - mientras que `Request::query_param` devolvía el
  **primero**. Un `?user=victim` legítimamente firmado se podía por
  tanto repetir como `?user=attacker&user=victim` con la firma
  original intacta: la verificación canonicalizaba sobre `victim` y
  pasaba, y el handler actuaba sobre `attacker`.

  La forma canónica ahora lleva cada par, ordenados por
  `(key, value)`, así que la firma cubre el multiconjunto exacto de
  parámetros - añadir, eliminar, o sustituir cualquier valor rompe el
  HMAC. Un `signature` o `expires` repetido se rechaza de plano, ya
  que dos de cualquiera de los dos no deja ninguna respuesta no
  arbitraria sobre cuál manda.

  `Request::query_param` ahora resuelve una clave repetida a su
  último valor, igual que `query_params` y `Context::query_param`;
  era el único de los tres que discrepaba, y esa discrepancia era la
  otra mitad del defecto. **Los enlaces firmados existentes siguen
  funcionando** - sin claves repetidas los bytes del payload no
  cambian, lo cual un test fija, porque un cambio de forma canónica
  que invalidara en silencio todo enlace de restablecimiento de
  contraseña pendiente sería peor que el bug.

  Seis tests de regresión, incluyendo ambos órdenes de ataque, una
  clave legítimamente repetida que debe seguir firmando y
  verificando, y la garantía de reordenamiento. *Sin* cambios:
  `signature_has_not_expired` sigue reportando una firma falsificada
  como "no caducada". Ese es el comportamiento de Laravel, se asentó
  deliberadamente como una corrección de documentación, y tiene su
  propio test que lo fija contra una "corrección" bien intencionada.
- **RBAC bajo Postgres.** Verificado contra un Postgres real en lugar
  de solo SQLite.
- **Cuatro avisos de RustSec eliminados, no renovados.** El driver de
  Pinecone se reescribió contra la API REST de Pinecone, descartando
  `pinecone-sdk 0.1.2` - cuyo lanzamiento más reciente data del
  2024-09-06 - y con él `tonic 0.11 → rustls 0.22 →
  rustls-webpki 0.102` y RUSTSEC-2026-0049 / -0098 / -0099 / -0104.
  Los cuatro se corrigieron upstream en `rustls-webpki >= 0.103.13`,
  que este workspace ya resolvía para sus otros usuarios de TLS; un
  crate abandonado mantenía el árbol en la línea vulnerable.
  `.cargo/audit.toml` baja de cinco exclusiones a una. Ver
  **Cambiado** para lo que esto significa para la API del driver.
- **Las excepciones de auditoría ahora caducan.** Cada entrada en
  `.cargo/audit.toml` lleva un `OWNER` y una fecha `EXPIRES`, y
  `scripts/check-audit.sh` hace fallar la puerta de release ante un
  propietario faltante, una fecha faltante o no parseable, o una
  caducada. `cargo audit` no tiene noción de una exclusión que
  caduque, así que una añadida "temporalmente" se quedaba hasta que
  alguien releía el archivo. La entrada restante (RUSTSEC-2023-0071,
  `rsa`, que no tiene ninguna versión corregida) tiene propietario y
  fecha.
- **Las afirmaciones de alcanzabilidad se verifican, no se asumen.**
  `scripts/check-feature-matrix.sh` resuelve árboles de dependencias
  reales y verifica que ningún build - incluyendo `--all-features`,
  que es lo que `cargo audit` en realidad lee - contenga
  `pinecone-sdk`, `rustls-webpki 0.102.x` ni `tonic 0.11.x`. Una
  excepción justificada por un comentario que nada verifica deja de
  ser cierta la primera vez que alguien añade una dependencia.

### Corregido

- **Cada liberación en una cola respaldada por base de datos era un
  no-op en silencio.** `JobOutcome::Released` - un bloqueo
  `WithoutOverlapping` ocupado, un backoff de limitador de velocidad -
  estaba implementado como "empujar una copia, y luego hacer ack del
  original". El id del sobre es la clave primaria de la tabla `jobs`,
  así que la copia colisionaba con la fila que todavía sostenía la
  reserva viva y el push fallaba con
  `UNIQUE constraint failed: jobs.id`. El worker entonces
  correctamente se negaba a hacer ack, así que el retardo solicitado
  nunca se aplicaba, no se disparaba ningún evento `JobReleased`, y el
  job simplemente se quedaba aparcado hasta que la expiración de
  visibilidad lo reentregaba. Las liberaciones ahora son una única
  llamada al driver, hecha in situ.
- **Un despacho de lote parcial dejaba huérfanos los jobs que ya
  había encolado (DATA-02).** Cuando un `driver.push` fallaba a mitad
  del bucle, `PendingBatch::dispatch` eliminaba la fila del lote -
  pero los sobres ya en la cola seguían marcados con ese id de lote,
  así que cada uno de ellos se resolvía contra un lote que ya no
  existía, devolviendo `Err(batch not found)` en cada entrega, para
  siempre. El lote ahora se resuelve en su lugar: los jobs no
  despachados se registran como fallos y el lote se cancela, así que
  los que sí quedaron en cola se resuelven con normalidad y los
  callbacks terminales igual se disparan.
- **Nada probaba que `url::has_valid_signature` rechaza una URL
  falsificada.** Encontrado al verificar la corrección de SEC-04: toda
  la suite del framework pasaba con la salvaguarda principal de URL
  firmada reescrita para aceptar cualquier firma.
- **Una app con andamiaje no podía migrar su base de datos ni
  construir su imagen (REL-01b).** Ningún andamiaje declaraba
  `default-run`, así que los nueve envoltorios de CLI que invocan
  `cargo run` fallaban en un proyecto recién creado. El Dockerfile
  generado tenía cinco defectos independientes - un `COPY` de
  lockfile faltante, `npm ci` sin lock, una etapa de caché que
  stubeaba uno de los dos binarios declarados, un build de frontend
  copiado desde una ruta que vite nunca crea, y una copia faltante de
  `frontend/src/pages` que `inertia_response!` valida en tiempo de
  compilación. La imagen de un andamiaje de fábrica no podía
  construirse.
- **`docker:init` emitía un único Dockerfile para cada tipo de
  proyecto.** En un proyecto `--api` su primera instrucción,
  `COPY frontend/package.json`, fallaba de plano. Los proyectos API
  ahora obtienen un Dockerfile sin frontend.
- **Placeholders de SQL (DATA-01).** Se renderizan por backend en
  lugar de asumir un único dialecto.
- **Resolución de cola (DATA-02a, P2-06c).** Los follow-ups se
  resuelven antes de que se haga ack de la reserva, y un error de
  liberación de bloqueo ya no convierte un job ya exitoso en un
  reintento.
- **Un lote cancelado disparaba `Catch`, nunca `Then`.**
- **`Builder::clone` descartaba en silencio el plan de carga
  anticipada (P2-09a).** `User::query().with("posts")` clonado en
  cualquier sitio - paginación, `count()`, cualquier scope que clone -
  devolvía filas sin relaciones y sin error.
- **Los registros de presencia perdían miembros (P2-08).** Se tomaba
  una instantánea del registro antes de suscribirse, así que
  cualquiera que se uniera en esa ventana no aparecía en ninguno de
  los dos, de forma permanente.
- **Pinecone serializaba cada adquisición de índice (P2-14).** El
  bloqueo de escritura se sostenía a través de dos viajes de ida y
  vuelta por red, y el `RwLock` justo de tokio hacía que un índice
  frío estancara a todos los índices calientes.
- **El watcher de tipos descartaba ráfagas (P2-13).** El antirrebote
  de flanco ascendente regeneraba en el primer archivo de una ráfaga y
  perdía el resto sin una ejecución final, así que el último guardado
  nunca surtía efecto.
- **`ssr:check` se podía colgar, y solo probaba una dirección
  (P2-13).** El DNS se ejecutaba completamente fuera del timeout, y
  solo se probaba la primera dirección resuelta - así que un host con
  un registro AAAA y sin ruta IPv6 reportaba el worker caído mientras
  escuchaba en v4.
- **`suprnova serve` instalaba `cargo-watch` sin fijar (P2-13).**
  Ahora con `--locked` y una cota de versión mayor.
- **El bumper de lanzamiento reescribía cinco READMEs y nada más.**
  Cuatro capítulos del manual y un comentario de documentación pública
  fijaban etiquetas que ningún lanzamiento actualizaba jamás - el
  comentario de documentación tenía dos lanzamientos de retraso. El
  descubrimiento ahora reemplaza la lista mantenida a mano, y la
  prueba de humo grepea el árbol actualizado de forma independiente en
  lugar de confiar en el propio paso de verificación del bumper.
- **`db:sync` trataba el esquema de la base de datos como entrada
  confiable (CLI-01).**
- **`migrate:fresh` queda detrás de `--force` más una confirmación
  tipada (CLI-02)**, tanto en el binario de la app como en la CLI.
- **El driver de correo `log` ahora registra el mensaje completo**,
  como hace Laravel, y ya no escribe enlaces bearer en el registro en
  producción.

### Añadido

- **Resolución terminal atómica (`QueueDriver::settle`, DATA-02).**
  El sucesor de la cadena y el ack ahora se confirman juntos en
  `DatabaseQueueDriver`, cerrando la ventana en la que una caída entre
  ambos, o bien perdía el resto de una cadena, o ejecutaba su
  siguiente paso dos veces. La eliminación con clave de reserva
  funciona a la vez como valla: un worker cuya visibilidad expiró a
  mitad de ejecución no confirma nada y reporta `Settled::Stale`, así
  que no puede encolar trabajo para un mensaje que ahora posee otro
  consumidor. Los drivers que no pueden hacer esto responden
  `Settled::Unsupported` y mantienen el orden documentado de
  push-antes-que-ack.
- **`DatabaseBatchRepository` (DATA-02).** La contabilidad de lotes
  sobrevive a un reinicio, y `pending_jobs`/`failed_jobs` se derivan
  de filas de resolución con clave `(batch_id, job_id)` en lugar de
  almacenarse y decrementarse - así que un job reentregado no puede
  llevar a un lote a "terminado" mientras sus otros jobs todavía se
  están ejecutando, y la salvaguarda se mantiene a través de procesos
  en lugar de dentro de uno solo.
- **`/_suprnova/health/live` y `/_suprnova/health/ready`.** Actividad
  no toca nada; preparación sondea las dependencias. Conectar una
  comprobación de base de datos a una sonda de actividad convierte un
  parpadeo de la base de datos en un reinicio progresivo de cada
  réplica, que es justo lo que invitaba el único endpoint anterior.
  `/_suprnova/health` sigue funcionando exactamente como está
  documentado.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Secreto compartido opcional
  para la sonda de preparación, comparado en tiempo constante. Sin
  él, preparación responde 404 - indistinguible de una ruta no
  enrutada, porque *es* el propio 404 del router. Sin establecer por
  defecto, para que las sondas existentes sigan funcionando.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, con `ssl`
  y `null` aceptados como alias compatibles con Laravel. Sin
  establecer, se deriva de las credenciales, reproduciendo
  exactamente el comportamiento anterior. Esto también hace
  alcanzable el TLS implícito en el puerto 465: el transporte lo
  soportaba, pero ninguna combinación de variables de entorno podía
  seleccionarlo.
- **`SERVER_MAX_CONNECTIONS` y `SERVER_HEADER_READ_TIMEOUT`**
  documentadas en `manual/env-vars.md`, donde antes faltaban por
  completo.

### Cambiado

La propia conclusión de la auditoría fue que la puerta pasaba en 470s
y no atrapaba ninguno de los 19 P1. La mayor parte del trabajo de
tests de este lanzamiento apunta a eso.

- **Postgres se ejecuta en la puerta.** Doce tests a través de seis
  archivos nunca se habían ejecutado. Dos de ellos resultaron apuntar
  `DROP TABLE` a cualquier Postgres que hubiera en `localhost:5432`
  por defecto, y ninguno había inicializado `Crypt`, así que ambos
  fallaban la primera vez que se ejecutaban.
- **Las aserciones de andamiaje leen los bytes que recibe un
  usuario**, después de la sustitución, en lugar de la fuente de la
  plantilla. Encontró un proyecto API que enviaba un comentario de
  documentación que nombraba una base de datos literalmente
  `{package_name}`, y un `.env.example` que anunciaba cinco claves de
  correo que el framework nunca lee.
- **Inyección de fallos en la cola.** La pérdida de ACK, la
  reentrega, el vencimiento de la reserva, y el despacho parcial se
  conducen mediante un decorador que hace fallar una operación
  nombrada en una llamada nombrada, así que cada caso es determinista
  en lugar de una carrera con sleep.
- **Los adaptadores de pago tienen tests negativos.** El `verify()`
  de Stripe nunca se había ejercitado con una firma *válida*, así que
  toda ruta de rechazo que depende de llegar a la comparación HMAC
  estaba sin probar.
- **El driver de Pinecone habla REST.** *Cambio incompatible, detrás
  de la feature `vector-pinecone`, desactivada por defecto.* La
  motivación está bajo **Seguridad**; los cambios de superficie son:
  - `client()` desaparece - ya no existe `PineconeClient`. En su
    lugar están `control_plane_get`, `control_plane_post` y
    `data_plane_post`, que alcanzan *cualquier* endpoint de Pinecone
    con tus propios tipos de solicitud y respuesta sobre el
    transporte autenticado y con host resuelto del driver. Eso es
    estrictamente más alcance del que tenía la vía de escape
    anterior.
  - `json_to_metadata` → `metadata_from_json`, y los metadatos ahora
    son `serde_json::Map` en lugar de `prost_types::Struct`.
    `decode_match_fields` → `decode_match`, que toma un
    `PineconeMatch`. `namespace()` devuelve `&str`.
  - Nuevo: `with_control_plane`, `with_api_version`, `with_index_host`
    (fija un host conocido y se salta el viaje de ida y vuelta al
    plano de control), `index_host`, y los tipos de red
    `PineconeVector` / `PineconeMatch`.
  - `from_env` sigue leyendo `PINECONE_API_KEY` y
    `PINECONE_CONTROLLER_HOST`, y ahora también
    `PINECONE_API_VERSION`.
  - La versión de la API REST está fijada, no flotante - `2025-04`,
    la versión contra la que se escribieron las formas de solicitud
    y respuesta del driver.
  - Ya no se serializa nada. El driver anterior cacheaba un `Index`
    por nombre detrás de un `tokio::Mutex` porque `pinecone-sdk`
    solo lo exponía detrás de `&mut self`; el nuevo cachea un string
    de host y comparte el pool de conexiones de `reqwest`.
  - Un host aprendido del plano de control siempre se contacta por
    `https`, cualquiera que sea el esquema que lleve la respuesta.
  - `Debug` está implementado a mano con la clave de API redactada,
    así que un `#[derive(Debug)]` sobre un struct que contiene un
    driver no puede imprimirla.
- **Tests de contrato de red para Pinecone.** Los tests de
  integración en vivo necesitan una `PINECONE_API_KEY` y por tanto no
  pueden ejecutarse en la puerta - lo cual dejaba los nombres de campo
  de una reescritura REST (`topK`, `includeMetadata`, `vectorCount`)
  apoyados en nada. Trece tests ahora ejercitan el driver contra un
  fake local de `wiremock` y verifican el método, la ruta, los
  encabezados y el cuerpo JSON exactos que envía por la red, además de
  que un no-2xx nunca se decodifica como resultado y de que un mensaje
  de error nunca lleva la clave de API. Fijan el driver al contrato
  *documentado* de Pinecone; solo los tests marcados `#[ignore]`
  pueden confirmar que la documentación coincide con el servicio en
  vivo.

## 0.7.2 - 2026-07-28

### Corregido

- **`generate-types` resuelve structs de prop anidados sin derives.**
  El generador de 0.7.1 degradaba a `unknown` cualquier campo de prop
  cuyo tipo no derivara `InertiaProps`/`Data` - así que volver a
  ejecutar el generador (o el watcher de `suprnova serve`) sobre un
  proyecto con un archivo de tipos versionado reemplazaba interfaces
  reales como `Array<AdminArticleRow>` por `unknown` y rompía el
  chequeo de tipos en toda la app. Los structs planos definidos en
  cualquier parte de `src/` ahora resuelven a sus interfaces reales,
  de forma transitiva desde las raíces de prop; `unknown` (con una
  advertencia) queda reservado para tipos que el proyecto
  genuinamente no define - tipos de crates externos, enums, tuple
  structs.

### Cambiado

- **La generación de `routes.ts` es opt-in.** `generate-types` ya no
  deja caer `frontend/src/types/routes.ts` en cada proyecto sin
  pedirlo; pasa `--routes` para generarlo.

- **Dependencias de los starters de frontend actualizadas.** Los
  andamiajes nuevos de `suprnova new` ahora fijan versiones actuales:
  Vite ^8.1.5, Tailwind CSS ^4.3.3, Svelte ^5.56.8
  (vite-plugin-svelte ^7.2.0, svelte-check ^4.7.4), React ^19.2.8
  (plugin-react ^6.0.4), Vue ^3.5.40 (plugin-vue ^6.0.8,
  vue-tsc ^3.3.8), y `@types/node` ^24 (la línea de tipos de Node 24
  LTS). TypeScript se queda deliberadamente en ^6.0.3: es la última
  6.x, y el rango de peer de svelte-check (`^5 || ^6`) todavía no
  admite TypeScript 7. Los tres starters se verificaron de punta a
  punta (`npm install` + `npm run build`) contra el conjunto
  actualizado.

## 0.7.1 - 2026-07-27

Una pasada de corrección de defectos sobre el enrutamiento de colas
de 0.7.0, a partir de una revisión completa posterior al lanzamiento.

### Corregido

- **Los jobs encadenados ya no pierden su cola declarada.**
  `ChainLink` capturaba `max_tries`, `timeout`, y `backoff` de un job
  en el momento de construir la cadena, pero no su `Job::queue()`, así
  que un job que aterrizaba en su cola declarada al empujarse
  directamente aterrizaba en `default` al despacharse como parte de
  una cadena - el nivel "job" del orden de resolución
  ruta → job → default desaparecía en silencio para las cadenas. La
  cola declarada ahora se captura en el link y se resuelve exactamente
  igual que un push directo. Los payloads de cadena escritos antes de
  este lanzamiento se decodifican sin cambios (`serde(default)`), y un
  link sin cola declarada serializa idéntico byte a byte a lo que
  escribía 0.7.0.
- **Los registros de job fallido llevan la cola en la que murió el
  job.** La ruta de envío a fallidos del worker fijaba
  `queue = "default"` a fuego en cada registro `FailedJob`, así que
  los fallos de un job enrutado eran invisibles para un operador que
  filtrara el store de fallidos por el pool al que pertenecen. El
  registro ahora lleva la cola del sobre (`default` para los jobs no
  enrutados).
- **La nota de actualización de 0.7.0 subestimaba la migración de
  `jobs`.** Decía "los workers sin filtrar no se ven afectados y no
  necesitan migración", pero `DatabaseQueueDriver::push` nombra la
  columna `queue` en su `INSERT` sea o no enrutado el job - un
  binario 0.7.0 contra una tabla sin migrar falla **cada push**, esté
  filtrado o no. La sección de 0.7.0 de abajo y `manual/queues.md`
  están corregidas: en el driver de base de datos el `ALTER TABLE` es
  obligatorio para cada despliegue, y debe ejecutarse antes de
  desplegar los binarios (los binarios antiguos listan sus columnas
  explícitamente, así que migrar primero es seguro).

- **El README ya no anuncia una macro `#[job]`.** Tal macro no
  existe - los jobs implementan el trait `Job`. La fila de colas
  ahora describe la superficie real, incluyendo el enrutamiento de
  colas de 0.7.0.

### Cambiado

- **La ruta de lanzamiento ahora actualiza las referencias de versión
  del README.** `bump-workspace-version.py` reescribe la etiqueta de
  instalación fijada del README, el ejemplo del modelo de
  distribución, y la línea de MSRV de forma atómica junto con los
  manifiestos, y un README reformulado que deja de coincidir con un
  patrón hace fallar de forma estrepitosa el lanzamiento. El README
  llevaba anunciando v0.6.0 desde que se publicó v0.7.0 porque nada en
  la ruta de lanzamiento lo tocaba.
- **El enrutamiento de conexión se documenta como solo resolución de
  nombre.** `Job::connection()` y el campo de conexión de
  `Queue::route` resuelven el *nombre* de conexión que llevan los
  eventos de ciclo de vida `JobQueueing` / `JobQueued`; un único
  driver global de proceso sigue recibiendo cada push, así que no
  seleccionan un driver distinto. El rustdoc y `manual/queues.md`
  antes daban a entender una selección de driver que no existe. La
  dimensión de cola no se ve afectada - se honra de punta a punta. Los
  drivers por conexión siguen siendo trabajo futuro.
- `ChainLink` ganó un campo público `queue: Option<String>`, lo cual
  rompe la construcción por struct-literal de links de cadena. Los
  links construidos mediante `ChainLink::from_job` - la ruta normal -
  no se ven afectados.

### Actualización

Si vienes de ≤ 0.6.x en el driver de cola de base de datos, aplica la
migración de 0.7.0 de abajo **antes** de desplegar los binarios; es
obligatoria para cada despliegue en ese driver, no solo los que usan
`--queue`. La propia 0.7.1 no necesita migración.

## 0.7.0 - 2026-07-26

### Seguridad

- **`ammonia` actualizado a 4.1.4 (RUSTSEC-2026-0213).** Las versiones
  hasta la 4.1.3 permiten XSS vía las etiquetas de animación SVG
  `animate` y `set`. `ammonia` es el sanitizador al final del
  pipeline de markdown de Suprnova (`comrak` → `syntect` →
  `ammonia`), así que cualquier app que renderizara Markdown
  suministrado por el usuario a través de `content` estaba expuesta.
  El aviso se publicó el 2026-07-21 - después de que se publicara
  v0.6.5 - así que **todo lanzamiento hasta el v0.6.5 inclusive está
  afectado**. Actualizar el framework es la corrección; no se
  requieren cambios de código en la aplicación.

### Añadido

- **Enrutamiento de colas.** Los jobs se pueden despachar a una cola
  y conexión específicas, y los workers se pueden dedicar a colas
  específicas - la superficie `Queue::route(...)` de Laravel 13,
  tipada. Un job declara su propio hogar con `Job::queue()` /
  `Job::connection()`; un operador lo anula de forma centralizada con
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` en
  `bootstrap::register()`, sin editar el job. La resolución es ruta,
  luego job, luego default global, y un campo `None` en una ruta
  difiere en lugar de limpiar. `queue:work --queue=billing,default`
  solo drena esas colas. Los jobs no enrutados pertenecen a
  `default`, así que nunca quedan varados. Los jobs encadenados
  resuelven rutas por nombre, ya que un link de cadena guarda su job
  con el tipo borrado.
- **`QueueDriver::pop_from`.** Pop con filtro, con una implementación
  por defecto que **rechaza** un filtro que no puede honrar en lugar
  de drenar en silencio toda la cola - un worker al que se le dice
  que drene `billing` y que en silencio lo drena todo es
  indistinguible de un despliegue que funciona hasta que el pool
  equivocado se come los jobs equivocados. Los drivers de memoria y
  de base de datos filtran de forma nativa. Los drivers
  personalizados siguen compilando y heredan el comportamiento por
  defecto, que falla de forma estrepitosa.
- **Documentado el esquema de la tabla `jobs`.** `manual/queues.md`
  ahora lleva el DDL que `DatabaseQueueDriver` realmente espera, que
  antes solo se podía descubrir leyendo el SQL del driver.
- **Documentada la opción `serverHead` de Inertia.** Los elementos
  `<head>` dirigidos por el servidor (Inertia 3.5.0) no necesitan
  ningún soporte del framework: el cliente los lee de una prop
  ordinaria, así que cualquier handler ya puede suministrarlos. Ver
  `manual/frontend-inertia-responses.md`.

### Cambiado

- `Envelope` ganó un campo `queue: Option<String>`. Es
  `serde(default)` y se omite cuando está ausente, así que un sobre
  no enrutado serializa idéntico byte a byte a lo que escribían las
  versiones anteriores - el test de formato en la red congelado pasa
  sin cambios, no hay incremento de `schema_version`, y las flotas de
  versión mixta interoperan durante una actualización progresiva.
- `WorkerConfig` ganó un campo `queues: Vec<String>` (vacío = drenarlo
  todo, el comportamiento anterior).
- Eliminado `ROADMAP.md`. Sus principios de diseño viven en
  `manual/introduction.md`, el acuerdo de trabajo en
  `manual/contributions.md`, y el material de despliegue y escalado
  en `manual/deployment.md`; las listas de publicado/planeado se
  habían quedado obsoletas. El puntero de `README.md` hacia él para
  "la relación con upstream" ya estaba colgando - esa atribución
  vive en `LICENSE`.
- Los frontends de andamiaje ahora fijan
  `@inertiajs/{svelte,react,vue3}` en `^3.6.1` (desde `^3.4.0`). El
  rango 3.4.0 → 3.6.1 es solo del lado del cliente - auditado contra
  el changelog upstream y el contrato `Page` en
  `packages/core/src/types.ts`, cada encabezado `X-Inertia-*` que
  envía el cliente 3.6.1 ya estaba manejado.
- `scripts/release.sh` ahora publica el release de GitHub por sí
  mismo, con notas tomadas de la sección `CHANGELOG.md` de la
  versión. Antes esto era un "siguiente paso" manual que se saltaba,
  que es por lo que v0.5.10 y v0.6.1–v0.6.3 son solo etiqueta y la
  página de Releases se quedó en una versión obsoleta. El preflight
  se ejecuta antes de la puerta para que un `gh` o una sección de
  changelog faltantes fallen en segundos, y la publicación se salta
  automáticamente a menos que `origin` sea GitHub.

### Actualización

Las tablas `jobs` existentes en el driver de cola de base de datos
**deben** añadir la nueva columna - `push` la nombra en su `INSERT`
sea o no enrutado el job, así que una tabla sin migrar falla cada
push. Migra primero, y despliega los binarios después (los binarios
antiguos listan sus columnas explícitamente e ignoran la nueva, así
que ese orden es seguro):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corregido en 0.7.1 - esta nota originalmente afirmaba que los
despliegues sin filtrar no necesitaban migración.)*

## 0.6.5 - 2026-07-21

### Añadido

- **Checkout puntual alojado en el adaptador de Stripe.**
  `Checkout::start_session` con `SessionMode::OneOff` y `price_refs`
  no vacío ahora crea una Checkout Session alojada (`mode=payment`,
  un ítem de línea por price ref, `allow_promotion_codes=true`) y
  devuelve `SessionPayload::StripeCheckoutRedirect`. La ruta de
  Elements de solo `amount_hint` no cambia; las dos formas se eligen
  por solicitud.
- **Compatibilidad con Stripe Managed Payments (merchant of record).**
  `StripeProvider::with_managed_payments(true)` - o
  `STRIPE_MANAGED_PAYMENTS=true` en `from_env()` - envía
  `managed_payments[enabled]=true` en la creación de sesión puntual
  alojada. Desactivado por defecto; el campo se omite por completo
  para que las cuentas no inscritas no se vean afectadas.
- **`Checkout::session_status`.** Método de trait nuevo (por defecto:
  `PaymentError::NotSupported`) que reporta el estado del lado del
  proveedor de una sesión como el nuevo `CheckoutSessionState`
  neutral (`Open` / `Complete { paid, payment_ref, amount_total }` /
  `Expired`). La implementación de Stripe mapea
  `GET /v1/checkout/sessions/{id}`; `payment_ref` lleva el id de
  PaymentIntent de la sesión para la correlación con la tabla de
  copia local. Esta es la primitiva de verificación del lado del
  servidor para las páginas de retorno de redirección y las barridas
  de reconciliación.
- **Trait de capacidad `Promotions`.** `create_promotion_code` acuña
  un código restringido a un cliente, opcionalmente con expiración y
  con tope de canjes, a partir de un cupón precreado. Se consulta vía
  el nuevo `PaymentProvider::as_promotions()` (por defecto `None`).
  Implementado para Stripe (`POST /v1/promotion_codes`) y para el
  mock.
- **Actualizaciones de `MockPaymentProvider` para lo anterior.**
  Registra cada solicitud `start_session` (`recorded_sessions()`),
  preestablece `session_status` por id de sesión
  (`script_session_status()` - las sesiones conocidas sin
  preestablecer reportan `Open`, los ids desconocidos `NotFound`), e
  implementa `Promotions` con solicitudes registradas
  (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

### Corregido

- **Los agregados de Eloquent decodifican de forma consistente entre
  backends de base de datos.** Las expresiones `count`, `sum`, `avg`,
  `min`, y `max` generadas ahora usan un único alias interno estable
  para el resultado. PostgreSQL ya no devuelve falsos ceros ni
  `None` porque su driver etiqueta las columnas de agregado de forma
  distinta a SQLite, y los errores de columna faltante o tipo
  incompatible ahora se propagan en lugar de quedar por defecto en
  silencio.
- **Las eliminaciones masivas no pueden usar expresiones de tabla
  suministradas por quien llama.** El SQL de eliminación ejecutable
  siempre deriva su objetivo del `M::TABLE` estático y validado del
  modelo. El argumento heredado del renderizador público sigue siendo
  compatible con el código fuente pero no puede redirigir ni inyectar
  el objetivo de la eliminación.

## 0.6.3 - 2026-07-15

### Añadido

- **Las lecturas crudas tipadas pueden quedarse en la conexión fijada
  de una transacción.** `Transaction::backend()` expone el backend
  activo y `Transaction::query_all(Statement)` ejecuta SQL de
  agregado tipado o personalizado a través de la transacción
  preservando la instrumentación de `QueryExecuted`. Las aplicaciones
  ya no necesitan una consulta a nivel de pool ni acceso al ejecutor
  privado cuando una decisión con alcance de bloqueo depende de
  columnas de resultado calculadas.

## 0.6.2 - 2026-07-15

### Corregido

- **Los predicados crudos vinculados son neutrales respecto al
  backend.** `filter_raw` y `where_raw` de Eloquent ahora aceptan
  marcadores de vinculación `?` portables en cada backend de base de
  datos; el renderizado de PostgreSQL los rebasa a posiciones `$N`
  monótonas a través de predicados anteriores, subconsultas de
  relación, cláusulas HAVING, y ramas UNION. Los fragmentos de
  PostgreSQL numerados existentes se normalizan por su orden local de
  marcadores, mientras que los estilos mezclados y los desajustes en
  el conteo de vinculaciones fallan la validación antes de hacer I/O.
  El escáner consciente de SQL preserva los signos de interrogación
  dentro de strings entre comillas, identificadores, comentarios, y
  cuerpos con comillas de dólar; `??` emite un operador de signo de
  interrogación literal en un fragmento crudo vinculado.

## 0.6.1 - 2026-07-15

### Añadido

- **Limpieza de sesión supervisada y observable.**
  `SessionMiddleware::install` usa la cadencia configurable de
  `SESSION_GC_INTERVAL` (una hora por defecto), mientras que
  `session_gc_metrics()` expone marcas de tiempo locales al proceso
  de ejecución, éxito, fallo, filas eliminadas, y último resultado
  para las superficies de operaciones protegidas.
- **Toques de sesión deslizante acotados.** `SESSION_TOUCH_INTERVAL`
  controla la cadencia mínima de escritura de actividad (cinco
  minutos por defecto) y está acotada a la mitad de la vida de la
  sesión para que las sesiones activas no puedan expirar entre
  toques.

### Corregido

- **Las solicitudes sin estado ya no crean sesiones durables.** Las
  solicitudes sin una cookie de sesión válida no realizan ninguna
  lectura ni escritura en el store de sesión y no reciben ninguna
  cookie de sesión a menos que el manejo cree estado. Las sesiones
  limpias existentes evitan los upserts incondicionales y el
  desgaste de cookies, las cookies heredadas migran en su siguiente
  solicitud, y las cookies cuyas filas respaldo han expirado se
  limpian sin recrear sesiones vacías.

## 0.6.0 - 2026-07-10

### Añadido

- **Subsistemas del framework opt-in con valores por defecto
  retrocompatibles.** El almacenamiento en sistema de archivos, los
  drivers de base de datos SQLite/Postgres/MySQL, el driver de vector
  de MariaDB, y Web Push ahora tienen features de Cargo explícitas.
  Los builds por defecto existentes conservan todas estas
  capacidades, mientras que los consumidores con
  `default-features = false` pueden elegir cero drivers o solo la
  superficie de almacenamiento/base de datos/vector/push que usan. La
  matriz de features ejecutable verifica los perfiles de cero
  drivers, driver individual, mínimo de Nation X, por defecto, y
  todas las features.
- **Importación de clave privada VAPID P-256 cruda.**
  `VapidKey::from_bytes` acepta un escalar P-256 big-endian de 32
  bytes validado, junto a la ruta existente de importación/exportación
  PKCS#8 PEM.

### Cambiado

- **Los JWT de VAPID ahora se firman directamente con P-256.** Web
  Push ahora serializa el encabezado/claims ES256 de RFC 8292 y los
  firma con `p256`, eliminando la dependencia genérica de JWT
  mientras preserva las claves generadas, los round trips de PEM, la
  codificación de clave pública, y el límite de vida de 24 horas.
- **Actualización de dependencias de seguridad.** Se actualizaron
  dependencias vulnerables del framework, incluyendo bcrypt y
  ammonia, y se redujeron las features activadas de Comrak
  conservando el resaltado de sintaxis.
- **Rust 1.91.1 es el MSRV del lanzamiento.** Cada paquete del
  workspace declara el mismo `rust-version`, los Dockerfiles
  generados fijan la imagen de builder correspondiente, y la puerta
  de release completa compila el perfil de sistema de archivos
  soportado con la cadena de herramientas exacta de Rust 1.91.1.
- **Fijación de seguridad de OpenDAL 0.58.** La feature de sistema de
  archivos fija el commit `88717391eb72c9839d3f8e79fccad9f22fc3a1b4`
  de `eas4ai/opendal`, un fork mínimo basado exactamente en
  el commit oficial `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` de
  Apache OpenDAL. El fork solo cambia las declaraciones de Reqsign
  que usan OpenDAL core más S3, GCS, y Azure Blob, así que los
  consumidores downstream resuelven el commit oficial
  `b49cd2996b9d2d9944e84481f8835ff55b188b97` de Apache Reqsign y
  `quick-xml` 0.41.0. Se necesita un fork porque los patches de
  Cargo en la raíz de un repositorio de dependencia no se propagan a
  los consumidores; el grafo publicado de otro modo podría restaurar
  el `quick-xml` 0.38/0.40 vulnerable.

### Corregido

- **Metadatos de versión de lanzamiento atómicos.** El incremento de
  versión de lanzamiento ahora actualiza `workspace.package.version`
  y cada dependencia de ruta interna versionada en una única
  operación validada, prepara todos los manifiestos afectados, y
  demuestra un workspace `0.6.0` temporal con
  `cargo check --workspace` antes del lanzamiento. Las versiones de
  lanzamiento se validan como SemVer 2.0 estricto, incluyendo la
  regla del cero inicial numérico en prerelease. Las pruebas de humo
  desechables sobre un remoto ficticio, agnósticas de versión,
  derivan un lanzamiento de parche posterior tanto de la fuente
  actual como de una fuente ya en `0.6.0`, rechazan árboles de
  lanzamiento con cambios staged/unstaged/untracked antes de la
  puerta, demuestran que la publicación atómica de commit/etiqueta
  revierte ambas referencias cuando se rechaza una etiqueta, y
  demuestran la secuencia normal de lanzamiento sin tocar el remoto
  real. Las versiones de lanzamiento deben aumentar según la
  precedencia de SemVer, incluyendo las transiciones de prerelease.
  Los artefactos de build de las pruebas de humo siempre se quedan
  dentro de su workspace temporal, ignorando cualquier
  `CARGO_TARGET_DIR` de quien llama.
- **El rustdoc cubre cada límite de feature soportado.** El módulo de
  OAuth enlaza al `OAuthAuth::complete` público, y la matriz
  ejecutable construye el rustdoc de cero drivers, por defecto, y
  todas las features sin dependencias.
- **La validación de stream del sistema de archivos tiene alcance de
  sesión.** Los escritores, listadores, y copiadores del sistema de
  archivos local resuelven y confinan sus rutas una vez antes del
  primer I/O en lugar de una vez por chunk/ítem, mientras que las
  operaciones de cierre/aborto activadas siempre llegan al backend
  para la limpieza. El confinamiento de traversal y symlink existente
  se sigue haciendo cumplir para un sistema de archivos confiable;
  las comprobaciones de canonicalizar-y-luego-abrir no eliminan las
  carreras contra un principal que muta el árbol de forma
  concurrente.

### Seguridad

- **La puerta de release falla cerrado.** `release.sh` delega en la
  puerta completa canónica antes de editar manifiestos o crear
  commits/etiquetas; esa puerta siempre ejecuta `cargo audit`, trata
  un binario `cargo-audit` faltante como un error, y se detiene ante
  cualquier fallo de auditoría. También construye y audita un
  consumidor de sistema de archivos downstream aislado, verificando
  las revisiones de fuente exactas de OpenDAL/Reqsign y que no haya
  `quick-xml` por debajo de 0.41. No se añadieron exclusiones de
  aviso nuevas.

## 0.5.10 - 2026-07-03

### Corregido

- **`generate-types` ya no descarta structs autorreferenciales.** Un
  struct con un campo que referencia su propio tipo (un nodo de árbol
  con `children: Vec<Self>`, p. ej. una vista de comentarios
  encadenados) creaba una autoarista en el grafo de dependencias de
  tipos, fijando su grado de entrada por encima de cero, así que el
  orden topológico de Kahn nunca lo emitía - dejando a cada interfaz
  que lo referenciaba con un nombre de tipo colgante que hacía fallar
  `svelte-check`/`tsc`. Las autoaristas ahora se eliminan antes de
  ordenar, y cualquier struct atrapado en un ciclo de referencias
  (recursión mutua) se emite en orden arbitrario en lugar de
  descartarse, ya que las interfaces TS pueden referenciarse entre sí
  sin importar el orden de declaración.

## 0.5.9 - 2026-07-01

### Añadido

- **`MAIL_FROM_NAME` - nombre para mostrar opcional en los correos de
  flujo de autenticación.** Los mailables de verificación de email,
  restablecimiento de contraseña, y contraseña cambiada ahora
  renderizan su encabezado `From` como `"Name <address>"` cuando
  `MAIL_FROM_NAME` está establecido (leído en el momento de envío
  para que sobreviva el round-trip de serde de la cola). `MAIL_FROM`
  se queda como una dirección desnuda; dejar `MAIL_FROM_NAME` sin
  establecer o en blanco conserva el comportamiento anterior de
  dirección desnuda. Sin cambios en ningún sitio de llamada - los
  mailables leen la variable de entorno por sí mismos.

## 0.5.8 - 2026-06-30

### Corregido

- **Los helpers de ruta de `generate-types` siempre son TypeScript
  válido.** Cuando varias rutas en un módulo comparten un handler
  (p. ej. una lista de permitidos de `static_files::serve` que
  mapea muchas URLs de favicon/activos), la primera conservaba el
  nombre del handler y el resto obtenía una clave derivada de la
  ruta - pero la ruta solo se saneaba parcialmente (`/ { } -` → `_`),
  así que una extensión de archivo filtraba un `.` a la clave:
  `favicon_16x16.png: (...) => ...`. Eso es acceso a miembro, no un
  nombre de propiedad, así que `tsc`/`svelte-check` rechazaban el
  `routes.ts` generado. Las claves derivadas ahora se sanean a
  identificadores legales - cada carácter no alfanumérico se
  convierte en `_` y un dígito inicial se prefija - así que
  `favicon-16x16.png` → `favicon_16x16_png` y `2fa.json` →
  `_2fa_json`. Los nombres de handler únicos no se tocan.

## 0.5.7 - 2026-06-30

### Corregido

- **`generate-types` ya no emite referencias de tipo colgantes.** Un
  campo de prop cuyo tipo es un struct que no deriva
  `InertiaProps`/`Data` (o un tipo externo que el generador no puede
  ver) se emitía como un identificador desnudo - p. ej.
  `user: UserInfo` - produciendo TypeScript que falla
  `tsc`/`svelte-check` porque esa interfaz nunca se escribe. Tales
  referencias ahora degradan a `unknown` (`user: unknown`;
  `Vec<T>` → `Array<unknown>`; `Option<T>` → `unknown | null`), así
  que la salida generada siempre pasa el chequeo de tipos, y
  `generate-types` imprime una advertencia nombrando el tipo sin
  resolver y el campo que lo referencia, con la corrección (derivar
  `InertiaProps`/`Data` sobre él). Los parámetros genéricos y los
  tipos InertiaProps/Data anidados resueltos no se ven afectados.

## 0.5.6 - 2026-06-29

### Cambiado

- **Sign in with Apple: verificación JWKS con RS256.** Sube
  `suprnova-apple-rs` a v0.3.1 - los ID tokens de Apple ahora se
  verifican contra el JWKS publicado por Apple (RS256) en lugar de
  confiar en ellos estructuralmente.

## 0.5.5 - 2026-06-28

### Añadido

- **Propósito de token `MagicLink`.** Nueva variante `MagicLink` en el
  enum `TokenPurpose` de flujo de autenticación, para tokens de
  inicio de sesión sin contraseña por enlace mágico.

## 0.5.4 - 2026-06-28

### Cambiado

- **Finalización de OAuth componible.** Divide la finalización
  genérica de OAuth en `verify_oauth_identity` (verificar + resolver
  la identidad) y un `complete` delgado, para que las apps puedan
  verificar una identidad OAuth sin disparar todos los efectos
  secundarios de finalización de sesión.

## 0.5.3 - 2026-06-28

### Corregido

- **Metadatos de versión de workspace corregidos.** v0.5.2 se
  etiquetó y publicó antes de que se preparara su incremento de
  versión de `Cargo.toml`, así que la etiqueta v0.5.2 publicada
  todavía lee `version = "0.5.1"`. v0.5.3 vuelve a cortar el
  lanzamiento con la versión de workspace correcta - sin cambio de
  código (la división de OAuth de v0.5.2 no se ve afectada).

## 0.5.2 - 2026-06-28

### Cambiado

- **Finalización de Apple componible.** Divide la finalización de
  Sign-In con Apple en `verify_apple_identity` + un `complete_apple`
  delgado, reflejando la división genérica de OAuth. (Nota: la
  etiqueta v0.5.2 publicada lleva un campo de versión `0.5.1`
  obsoleto - corregido en v0.5.3.)

## 0.5.1 - 2026-06-28

### Cambiado

- **Crate de Apple renombrado.** Redirige la dependencia de Apple al
  repositorio renombrado `suprnova-apple-rs`.

## 0.5.0 - 2026-06-28

### Añadido

- **Sign in with Apple.** Intercambio de token OAuth + verificación
  de ID token + upsert de usuario para Apple; endpoints well-known de
  Apple y el modo de respuesta `form_post`; campos específicos de
  Apple en `OAuthProviderConfig`; `AppleKeyPair` reexportado para que
  las apps configuren Sign-In con Apple sin una dependencia directa
  de `apple`.

### Corregido

- Omite los parámetros PKCE de la URL de autorización de Apple
  (Apple rechaza la solicitud cuando están presentes).

### Dependencias

- Consume la corrección de magic-auth de `torii`; añade `apple-rs`
  v0.3.0.

## 0.4.1 - 2026-06-26

### Rendimiento

- Preasigna el tamaño de `MiddlewareChain` para eliminar
  reasignaciones de `Vec` por solicitud.

### Corregido

- Hace que la ruta del archivo de mantenimiento (down-file) sea a
  prueba de colisiones bajo ejecuciones de tests en paralelo.

### Documentos

- Compila y comprueba los ejemplos de doc del framework
  (`ignore` → `no_run`); reconcilia las notas de distribución con los
  GitHub Releases etiquetados; ignora todo el árbol `docs/`.

## 0.4.0 - 2026-06-22

### Cambiado

- **La distribución se rastrea vía git; no fijas etiquetas.** Las
  apps con andamiaje dependen de
  `suprnova = { git = "…/suprnova.git" }` y siguen la rama por
  defecto; las actualizaciones se obtienen con
  `cargo update -p suprnova`. Las versiones se publican como GitHub
  Releases etiquetados (`v0.4.0`, …) para el changelog, pero
  `Cargo.lock` ya fija el commit resuelto exacto - así que los builds
  siguen siendo reproducibles sin fijar a mano una `tag` o un `rev`.
  Los docs de instalación ya no presentan la fijación de commit como
  la ruta de actualización.

## 0.3.0 - 2026-06-21

### Añadido

- **Instrumentación de consultas para lecturas de Eloquent** -
  `Builder::get`, `Model::find`, `find_many`, y `all` ahora emiten
  `QueryExecuted`, así que los SELECT de modelo y las consultas de
  carga anticipada emergen en `DB::listen` y en el registro de
  consultas en memoria junto a las escrituras y las consultas crudas.
  Añade el terminal de lectura instrumentado
  `ExecutorChoice::statement_all`.
- **Autorización de rutas de recurso** -
  `ResourceRoutes::authorize_resource::<U, R>()` adjunta la
  comprobación de habilidad convencional a cada ruta de recurso
  generada como middleware por ruta (paridad con `authorizeResource`
  de Laravel). El mapa acción→habilidad es `index`/`show` → `view`,
  `create`/`store` → `create`, `edit`/`update` → `update`,
  `destroy` → `delete`. Una sola llamada pone una compuerta a toda la
  superficie de siete acciones en lugar de depender de que cada
  cuerpo de controlador recuerde un `Gate::authorize`.
- **Golpe de límite de velocidad atómico** -
  `RateLimiter::hit_and_check(key, max, decay)` incrementa una
  ventana fija y la comprueba en un único viaje de ida y vuelta,
  devolviendo si el cubo está ahora por encima de su límite
  (`i64::MAX` significa sin límite).
- **Helper de comparación en tiempo constante** -
  `constant_time_eq(a, b)` (respaldado por subtle) para la
  verificación de firma de webhook; los docs de
  `WebhookHandler::verify` ahora exigen comparación de digest en
  tiempo constante.
- **Cliente de Inertia a 3.4.0** - los andamiajes de
  Svelte/React/Vue ahora fijan `@inertiajs/{svelte,react,vue3}` en
  `^3.4.0` (desde `3.1.1`), incorporando los modos de `router.poll`,
  `usePoll` dinámico, `Inertia.once`, la corrección de cancelación de
  InfiniteScroll, y el `onSuccess` esperado (`awaited`) de Form. El
  servidor ya emite la superficie completa de page-object y
  encabezados de 3.4.0 (once-props, la familia de scroll
  prepend/deep-merge, `matchPropsOn`, props rescatadas/compartidas),
  así que esto es una actualización de vigencia del cliente sin
  cambio de protocolo.
- **Tope de conexión opcional** - `SERVER_MAX_CONNECTIONS` (y el
  `Server::max_connections(n)` programático) acota las conexiones
  activas concurrentes con un semáforo en el bucle de aceptación,
  aplicando contrapresión en el nivel TCP. Sin establecer - o `0` -
  deja las conexiones sin límite (el valor por defecto, sin cambios).
  Un respaldo para emparejar con un reverse proxy y `LimitNOFILE`, no
  un reemplazo del limitador de velocidad upstream.
- **Optar por no seguir redirecciones** -
  `RequestBuilder::no_redirects()` enruta una solicitud a través de
  un cliente HTTP que no sigue redirecciones, así que un `3xx` se
  devuelve tal cual en lugar de perseguirse. Úsalo cuando la URL de
  la solicitud está influida por entrada no confiable, para cerrar
  un vector de SSRF basado en redirección (un endpoint hostil
  redirigiendo hacia un host interno o de metadatos de nube). El
  cliente por defecto sigue siguiendo redirecciones, siguiendo la
  convención general de clientes.

### Seguridad

- **Las rutas de recurso** fallan cerrado ante el downcast de tipo
  borrado del registro de autorización en lugar de entrar en pánico,
  y las denegaciones de `authorize_resource` / las solicitudes no
  autenticadas se rechazan antes de que se ejecute el handler.
- **El limitador de velocidad** cierra una carrera de
  comprobar-y-luego-golpear de ventana fija incrementando y
  comparando de forma atómica (`hit_and_check`).
- **El middleware `RateLimited` de cola** ahora admite jobs a través
  de ese `hit_and_check` atómico en lugar de un par separado de
  `too_many_attempts` + `hit`, así que los workers concurrentes ya no
  pueden pasar todos la comprobación de presupuesto antes de que
  ninguno incremente, y sobreadmitir más allá de `max_attempts`.
- **Los validadores de subida** (`mimetypes` / `mime`) hacen
  sniffing de contenido sobre los bytes subidos en lugar de confiar
  en el `Content-Type` suministrado por el cliente.
- **La salvaguarda de rutas del sistema de archivos** canonicaliza
  las rutas para atrapar traversal de symlink fuera de la raíz de
  almacenamiento, más allá de las comprobaciones léxicas previas de
  `../` / absolutas / UNC.
- **Auth** cierra un timing oracle de login sin contraseña - una
  cuenta que coincide pero no tiene contraseña, a la que se le da una
  contraseña, ahora ejecuta una verificación de coste fijo, tanto en
  el proveedor de usuario de Eloquent como en el de base de datos - y
  `dummy_verify` acciona el hasher configurado para que la ruta de
  usuario sin coincidencia sea en tiempo constante.
- **Eloquent** valida los identificadores de columna en las rutas de
  proyección `pluck` / `value` / `pluck_keyed` / `sole_value` y
  `sum` / `avg` / `min` / `max`.
- **Pagos** - el verificador del proveedor mock falla cerrado fuera
  de un entorno de desarrollo, y las IPs de origen de webhook se
  resuelven a través de `TrustedProxiesConfig` (`req.ip()`) en lugar
  de un encabezado `X-Forwarded-For` crudo.
- **La salvaguarda de rutas del sistema de archivos** ahora recorre
  hacia arriba hasta el ancestro *existente* más cercano cuando un
  objetivo de escritura todavía no existe, cerrando un escape de
  symlink donde un symlink intermedio plantado con un padre inmediato
  faltante se colaba más allá de la salvaguarda.
- **`DB::init_with`** valida el entorno antes de conectar (igual que
  `DB::init`), así que el fallback de SQLite de desarrollo ya no
  puede arrancar en silencio en producción a través de ese punto de
  entrada.
- **El servido de archivos estáticos** rechaza dotfiles (`.env`,
  `.git/config`, `.htpasswd`, cualquier segmento que empiece por
  `.`), no solo el traversal `.`/`..`.
- **Los webhooks de pago** serializan los reintentos concurrentes del
  mismo evento sin procesar con un bloqueo `FOR UPDATE` + una
  recomprobación, y tratan las violaciones de unicidad de la tabla de
  copia local como "ya aplicado" benigno; `payments_subscription_items`
  gana un `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** fija por defecto el discriminador de modelo al nombre de
  tipo totalmente calificado, así que dos tipos autenticables que
  comparten un nombre de hoja ya no pueden heredar los roles/permisos
  el uno del otro.
- **`invalidate_session()`** rota el id de sesión (no solo la vacía),
  cerrando una brecha de fijación de sesión; el middleware de cola
  `WithoutOverlapping` libera su bloqueo de caché incluso cuando el
  job entra en pánico.
- **Los proveedores de correo** acotan las lecturas del cuerpo de
  respuesta de error (8 KiB), igual que el cliente de web push, así
  que un endpoint hostil no puede llevarse por delante la memoria del
  emisor.
- **Web push** desactiva el seguimiento de redirecciones HTTP en el
  cliente por defecto, así que un endpoint de push influido por un
  atacante ya no puede redirigir con `3xx` un POST de notificación
  hacia un host interno o de metadatos de nube (SSRF). Una
  redirección ahora emerge como un push rechazado en lugar de como
  una solicitud seguida en silencio.
- **El adaptador de Stripe** `Debug` redacta el secreto de firma del
  webhook *y* imprime un placeholder para el `stripe::Client` (que
  lleva la clave secreta de API en su encabezado de auth), así que
  ningún secreto puede llegar a los registros a través de un `{:?}`
  de `StripeProvider`, sin importar el propio `Debug` del cliente
  upstream.
- **El adaptador de Stripe** `from_env` rechaza credenciales
  presentes pero en blanco, fallando cerrado en lugar de construir un
  cliente con un secreto HMAC de webhook vacío (y por tanto
  falsificable).
- **La verificación de email de OAuth** falla cerrado para
  proveedores no reconocidos: un payload de userinfo que lleva un
  `email` pero no un flag `email_verified` ya no se trata como
  verificado. Un proveedor desconocido ahora debe afirmar
  `email_verified: true` o exponer un endpoint de emails verificados,
  cerrando un vector de vinculación/toma de cuenta para apps que
  indexan cuentas por email. Google (solo `true` explícito) y GitHub
  (verificado por el contrato de `/user`) no cambian.

### Corregido

- **La carga anticipada anidada** (`with(["posts.comments"])`) ahora
  es un número constante de consultas - el segmento final se carga en
  una única consulta `IN` por lotes a través de todos los padres en
  lugar de una consulta por padre (N+1).
- **`where_has`/`where_doesnt_have`** cualifican las columnas del
  closure con la tabla objetivo, así que una columna presente tanto
  en pivot como en el objetivo ya no produce un error de columna
  ambigua en relaciones many-to-many.
- **El `delete`/`force_delete`/`touch` de soft-delete y el `persist`
  de factory** honran el enrutamiento `#[model(connection = "…")]`
  de un modelo (igual que `restore` y las otras rutas de escritura)
  en lugar de recaer en el pool primario.
- **`Maybe::Missing` de JSON:API** usa un centinela de red no
  colisionable, así que datos de usuario con la forma
  `{"__missing__": true}` ya no se eliminan en silencio.
- **Las notificaciones en cola** honran `should_send` (veto por
  canal) y `after_sending`, reverificados en el worker - antes solo
  lo hacía la ruta síncrona.
- **Los jobs liberados** empujan la copia de reintento antes de hacer
  ack del original, así que un error transitorio de push del driver
  ya no descarta el job.
- **Los webhooks de ajuste (reembolso) de Paddle** indexan la
  actualización de la copia local sobre el id de transacción
  referenciado y leen los montos de `data.totals`, en lugar de
  insertar una fila de monto cero bajo el id de ajuste.
- **Las URLs de SQLite** que llevan un query string
  (`sqlite://db.sqlite?mode=rwc`) construyen una URL de conexión
  válida de una sola query y un nombre de archivo en disco limpio.
- **HTTP** acota los valores `q` de `Accept` a `[0,1]` y hace cumplir
  el `max_body_bytes` de un `FormRequest` incluso cuando el cuerpo ya
  estaba almacenado en búfer; la config de **WebSocket** rechaza
  `max_missed_pings < 2` (con 1 se cerraba cada conexión en su primer
  ping).
- **Cron** usa semántica OR entre día-del-mes y día-de-la-semana
  cuando ambos están restringidos (paridad Vixie/POSIX); el
  `plain_text`/los excerpts de Markdown preservan la puntuación
  espaciada intencional; `CachedEvaluator` acota el crecimiento de su
  caché; `SupervisorRegistry::start_all` ya no genera el doble en una
  segunda llamada; el contenedor de pruebas se recupera in situ de un
  bloqueo envenenado.
- **El backoff de reinicio del supervisor** vuelve al piso de 100 ms
  tras una ejecución que se mantiene activa al menos el tope de 60 s,
  así que un demonio que se ejecutó de forma saludable durante un
  buen tramo y luego sale se reinicia con prontitud en lugar de
  heredar un backoff que había escalado durante una ráfaga de fallos
  anterior. Un bucle de caídas cuyas ejecuciones nunca alcanzan el
  umbral igual escala hasta el tope, así que el reinicio nunca
  enmascara a un supervisor inestable.
- Se corrigieron docs obsoletos sobre `filter_op` (los operadores se
  validan contra una lista de permitidos), las URLs firmadas (no
  compatibles byte a byte con las firmas absolutas por defecto de
  Laravel), `UniqueIdKind::is_valid` (un helper para quien llama, no
  conectado automáticamente en `find`), y el tope de longitud de
  identificador (128, no 64).

### Documentación

- Se documentó la autorización de rutas de recurso
  (`authorize_resource`) en los capítulos de enrutamiento y
  autorización, y el contador atómico `hit_and_check` en el capítulo
  de límite de velocidad.

## 0.2.0 - 2026-06-21

Añade control de acceso basado en roles, un pipeline de renderizado
de contenido Markdown / docs, y servido nativo de archivos estáticos.

### Añadido

- **RBAC de nivel 2** - trait `HasRoles`; roles + permisos con un
  join `role_has_permissions`; `PermissionMiddleware` /
  `RoleMiddleware` (ambos fallan cerrado / deniegan por defecto); la
  migración `CreateRbacTables`; y los helpers `create_role` /
  `create_permission` / `give_permission_to_role`.
- **Renderizado de contenido** - renderizado de Markdown y un
  pipeline de build de docs: `MarkdownRenderer`, `build_docs`,
  `DocsCatalog` / `DocsChapter`, extracción de encabezados y
  `slugify_heading`. El HTML renderizado se sanea (comrak + syntect +
  ammonia).
- **Servido nativo de archivos estáticos** - handler de fallback
  `StaticFiles::public()` para servir un directorio `public/` en la
  raíz web, reemplazando controladores de lista de permitidos por
  activo hechos a mano en las apps.

### Corregido

- Las apps recién generadas heredan una fijación de compatibilidad
  `time = 0.3.47` a nivel de framework, evitando conflictos de
  coherencia de Rust 1.96 provenientes de `time 0.3.48` en las
  resoluciones de dependencias de andamiajes recién creados.

### Documentación

- Se documentaron los dos starter kits publicados - **Nebula** (auth
  de nivel Breeze) y **Pulsar** (sitio de producto + comunidad) - a
  través del manual, el README, y el roadmap; se reestructuró el
  roadmap en torno a la superficie publicada; y se reconciliaron las
  referencias de versión en toda la documentación.

## 0.1.0 - 2026-06-10

El lanzamiento inicial de Suprnova. Suprnova es un framework web para
Rust inspirado en Laravel, un fork de Kit llevado en su propia
dirección. El objetivo de paridad actual es Laravel 13.x.

Este lanzamiento usa el modelo de distribución por git: los
consumidores del framework dependen de
`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`,
y la CLI se instala con `cargo install --git`.

### Añadido

#### HTTP, enrutamiento y middleware

- `Router` con grupos de rutas, prefijos, restricciones de parámetro,
  rutas con nombre
- Registro de rutas validado en tiempo de compilación vía la macro
  `routes!`
- Enrutamiento de recursos (`Router::resource`) que produce las siete
  rutas estándar
- URLs firmadas (funciones libres `url::signed_route` /
  `url::temporary_signed_route`, más `Redirect::signed_route` /
  `Redirect::temporary_signed_route`)
- Helpers de redirección - `Redirect::to`, `Redirect::back`,
  `Redirect::route`, `Redirect::with_input`, `Redirect::with_errors`,
  `with_flash`
- Trait de middleware con capas global, de grupo, y por ruta
- Middleware integrado - CORS, CSRF, sesión, timeout de solicitud, id
  de solicitud, throttle / throttle de login, verificación de URL
  firmada, autenticado, email verificado, fuerza bruta
- Helpers de abort (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - adaptador público para servir una
  única solicitud hyper contra un router + cadena de middleware

#### Puente de frontend con Inertia.js

- `#[derive(InertiaProps)]` con emisión de tipos de TypeScript
- Macro `inertia_response!` con validación de componente en tiempo de
  compilación
- Tres frontends de starter de primera clase - **Svelte 5** (con
  runes), **React 19**, **Vue 3.5** - todos sobre Inertia 3.1.1 +
  Vite 8 + Tailwind v4
- Recargas parciales (`only` / `except`), props diferidas, layout
  persistente, historial cifrado, preservación de scroll
- `Inertia::paginate(component, key, paginator)` para conectar un
  paginador a una prop de Inertia

#### ORM al estilo Eloquent (sobre SeaORM)

- Macro de atributo `#[suprnova::model]` que emite una entidad SeaORM
  y el struct Eloquent de cara al usuario en una sola pasada
- Trait `Model` completo - `create`, `find`, `find_or_fail`,
  `find_many`, `all`, `query`, `save`, `update`, `delete`,
  `force_delete`, `refresh`, `fresh`, `replicate`, `replicate_into`,
  `increment`/`decrement`, `destroy`, `is`/`is_not`,
  `to_array`/`to_json`
- Asignación masiva fillable / guarded con el envoltorio `Attrs`
- 22 casts de atributo - booleanos, enteros, floats, fechas, enums,
  hasheado, cifrado, JSON, colecciones, dinero, datetime con zona
  horaria
- Accesores / mutadores vía `#[suprnova::model]`
- Timestamps automáticos (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) con `force_delete`, `restore`,
  `trashed`, `only_trashed`, `with_trashed`
- Once tipos de relación - `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Enums morph por familia + registro morph con rotación de
  `APP_KEY_PREVIOUS`
- Carga anticipada vía `.with(...)`, `.with_count(...)`,
  `.load_missing(...)`
- Motor EXISTS correlacionado para `has` / `where_has`
- Dieciséis eventos de ciclo de vida (`retrieving`, `retrieved`,
  `creating`, `created`, `updating`, `updated`, `saving`, `saved`,
  `deleting`, `deleted`, `restoring`, `restored`, `force-deleting`,
  `force-deleted`, `replicating`, `trashed`)
- Trait `Observer<M>` con auto-registro por método vía el inventario
- Scopes locales vía `#[scopes(M)]`, scopes globales vía
  `GlobalScope`
- Superficie `Collection<M>` de Laravel - `pluck`, `key_by`,
  `group_by`, `where_in`, `first_where`, `contains_where`,
  `partition`, etc.
- Tres paginadores - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - todos serializando a JSON con forma Laravel
- `chunk` / `lazy` / `cursor` para iteración masiva de filas sin OOM
- Bloqueo a nivel de fila `lock_for_update` / `shared_lock`
- Query builder `DB::table(...)` con `DynamicRow` para consultas
  ad-hoc
- `DB::transaction(...)` con savepoints, reintento ante deadlock,
  separación de lectura/escritura multi-conexión
- `DB::listen(...)` + eventos `QueryExecuted` / `TransactionBegan` /
  `TransactionCommitted` / `TransactionRolledBack`
- Trait `Prunable` + comando de consola `model:prune`
- Métodos helper de consulta `dump` / `dd`
- `#[model(unique_id="...")]` para claves primarias UUID / ULID

#### Autenticación

- Trait `Authenticatable` + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Varios guards con nombre (sesión web, token de API)
- Flujo de verificación de email - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, URLs de verificación firmadas,
  `EmailVerificationMail`
- Flujo de restablecimiento de contraseña - `PasswordReset`, tokens
  con throttle, `PasswordChangedMail`, evento
  `PasswordResetLinkSent`
- TOTP de dos factores - inscripción, verificación, códigos de
  recuperación, protección contra repetición
- Fuerza bruta / throttle de login - con clave de IP + identificador,
  `LoginThrottleMiddleware`
- Cookies remember-me con tokens opacos estables
- Seis eventos de auth - `LoginAttempted`, `LoggedIn`,
  `Authenticated`, `LoggedOut`, `PasswordResetLinkSent`,
  `EmailVerified`
- Sesiones de navegador respaldadas por el fork de Torii en
  `github.com/eas4ai/suprnova-torii-rs`

#### Autorización

- Fachada `Gate` - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (variantes síncrona + asíncrona)
- Macro `#[policy(Model)]` para registro de políticas
- Autoautorización de rutas de recurso

#### Pagos

- Superficie de cinco traits agnóstica de proveedor - `Checkout`,
  `Payment`, `Subscription`, `CustomerStore`, `WebhookHandler`
- Trait paraguas `PaymentProvider` + consulta de capacidades vía
  `as_payment()`
- Copia local en BD - `customers`, `subscriptions`,
  `subscription_items`, `payments`, `refunds`,
  `payment_webhook_events` (UNIQUE para idempotencia)
- Enum `SessionPayload` etiquetado por flujo (de una sola vez frente
  a suscripción)
- Dos adaptadores de referencia como crates del workspace -
  `suprnova-payments-stripe` (gateway, implementación completa de
  `Payment`), `suprnova-payments-paddle` (Merchant of Record, sin
  implementación de `Payment`)
- Proveedor mock para tests

#### Cola, jobs, lotes y cadenas

- Trait `Job` - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers - `sync`, `null`, `redis`, `database`
- Trait `JobMiddleware` - seis middleware integrados
- Lotes y cadenas - `Queue::batch(jobs).dispatch()`, builder de
  cadena fluido, cancelación, seguimiento de progreso
- Store de jobs fallidos con reproducción
- Worker con apagado ordenado, concurrencia configurable,
  recuperación de pánico vía `catch_unwind`, métricas de resolución
- Doce eventos de cola que cubren el encolado, el procesamiento, el
  fallo, la liberación, y el ciclo de vida del worker

#### Difusión y WebSockets

- Macro `ws!()` + `Router::ws` para endpoints de WebSocket tipados
- División Sink/Stream de `WsSocket`
- Supervisores con auto-reinicio vía el trait `Supervisor`
- `BroadcastHub` con canales `Channel`, `Private`, `Presence`
- Protocolo con sobre JSON, presence join/leave/here, TTL de
  presencia configurable con recuperación ante caídas
- Puente `Broadcastable` hacia `EventDispatcher`
- Heartbeat con cierre ante ausencia de pong, con drenado
  configurable de `WS_TASKS`
- Middleware de WebSocket por ruta
- Valores por defecto más seguros de 1 MiB / 64 KiB + factory
  `WsConfig::generous()`
- Política de origen + cierre 1011 ante violación de protocolo

#### Notificaciones y correo

- Trait `Notification` + `Notify::send(recipient, notification).await`
- Mailable + renderizado de plantillas Markdown
- Canales de base de datos / correo / difusión / web push
- Firma VAPID + cifrado de payload ECE de RFC 8291 (vía
  `suprnova-web-push`)
- Validación de subject VAPID, parseo de retry-after, tope de 8 KiB
  en el cuerpo de rechazo
- Trait `Notifiable` para tipar destinatarios

#### Eventos

- Despachador de eventos tipado - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Eventos `saving`/`updating` cancelables (devuelven
  `EventResult::cancel`)
- Oyentes encolables

#### Filesystem

- `Storage::disk("name")` con soporte multi-driver - local, S3,
  Azure, GCS vía OpenDAL
- Mover, copiar, existe, tamaño, mime, última modificación,
  prepend/append
- Subidas y descargas en streaming

#### Caché

- `Cache::store("name")` + registro de driver
- Drivers - memory, redis (con connect-timeout acotado), database,
  file
- `remember`, `forever`, `tags`, incremento/decremento atómico,
  bloqueos

#### Base de datos vectorial

- Trait `VectorDriver` con cuatro drivers - en memoria, Qdrant
  (mapeo de ID UUID-5), Pinecone (IDs de string nativos),
  `VECTOR(N)` nativo de MariaDB + índices HNSW (11.7+)
- Distancia coseno / producto punto / euclidiana

#### Binario de consola y CLI

- Binario `console` por proyecto - análogo en Rust de `php artisan`,
  ejecuta comandos definidos por el usuario vía
  `#[suprnova::console::command]`
- `#[derive(Command)]` para argumentos tipados
- CLI `suprnova` - `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`,
  `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- Flag `--version`
- Plantillas de andamiaje para starters de backend + API a través de
  tres frontends

#### Indicadores de características

- `DatabaseEvaluator` con carga de instantánea
- `CachedEvaluator` con TTL
- Extractor `FeatureMiddleware`
- Superficie CRUD de admin
- Trait `FeatureSync` para propagación en menos de un segundo entre
  procesos

#### Programación

- Parser de expresiones cron
- `Schedule::task(...)` con predicados componibles
- Bloqueos de un solo servidor, prevención de solapamiento,
  seguimiento de despacho
- Comando de consola `schedule:run`

#### Validación

- Integración con `validator` 0.20
- Macros `#[request]` + `#[derive(FormRequest)]`
- Tope de tamaño por formulario `#[form_request(max_body_bytes = N)]`
- Opt-out `#[form_request(custom_hooks)]` para un `impl FormRequest`
  escrito por el usuario
- Hooks de ciclo de vida - `authorize`, `after_validation`,
  `after_validation_async`

#### Drivers de base de datos

- Soporte respaldado por SeaORM para SQLite, Postgres, MySQL,
  MariaDB
- Detección de driver basada en URL
- Sistema de migraciones + `migrate`, `migrate:rollback`,
  `migrate:status`, `migrate:fresh`, `migrate:refresh`

#### Cliente HTTP

- Fachada `Http` - `get` / `post` / `put` / `patch` / `delete` que
  devuelven un `RequestBuilder`; `.send().await` produce un
  `ClientResponse`
- TLS con rustls, timeout por defecto de 30s, user-agent
  `suprnova/<version>`
- Métodos encadenables `json` / `form` / `body` / `header` /
  `bearer_token` / `basic_auth` / `timeout`
- `RequestBuilder::retry(max_attempts, base_backoff)` - backoff
  exponencial para fallos transitorios y 5xx; respeta `Retry-After`
- Guarda de test `Http::fake(|| async { ... }).await` con
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Cifrado

- Fachada estática `Crypt` + `EncryptionKey` (`crypto::*`);
  AES-256-GCM con nonces aleatorios de 12 bytes
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- Vinculación AAD `CryptPurpose` que impide la repetición entre
  protocolos
- Rotación de `APP_KEY_PREVIOUS`
- Comando de CLI `suprnova key:generate` para acuñar claves nuevas

#### Pruebas

- Macro de test asíncrono `#[suprnova_test]`
- `TestDatabase::fresh::<Migrator>()` con instancias seguras en
  paralelo
- `TestContainer::bind` para mocks por test
- Helpers de test HTTP - `Test::get`, `Test::post`, JSON / form /
  multipart
- Fakes de Queue / Mail / Notification / Event
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Cambiado

- Los flujos de verificación de auth y restablecimiento de
  contraseña ahora operan a través del proveedor de usuario
  configurado en lugar de los internos de Torii.
- Las apps generadas deben implementar `get_auth_password`; los
  ejemplos de andamiaje ahora fallan de forma estrepitosa en lugar de
  permitir que el login falle siempre en silencio.
- La puerta de release local está conectada a `scripts/release.sh`, y
  el repo incluye un hook de pre-push forzoso para fmt, clippy,
  tests, docs, y builds de features.
- La documentación de los puertos de desarrollo del andamiaje
  se movió a los valores por defecto actuales de backend/frontend
  (`8765` / `5765`), con `dev:tls` y `--with-portless` documentados.
- `MAIL_FROM` se valida antes de emitir tokens de verificación o
  restablecimiento, evitando filas huérfanas de flujo de
  autenticación cuando la configuración de correo es inválida.

### Corregido

- Deriva de la plantilla de andamiaje de React respecto al starter
  publicado.
- Los grupos de rutas raíz ya no generan rutas `//` duplicadas.
- Las redirecciones de ruta literal ahora se despachan a través de la
  ruta de enrutamiento prevista.
- Los tests de fanout de difusión ahora manejan los resultados de
  `track` / `untrack`.
- El driver de correo `log` emite el cuerpo de texto renderizado, así
  que los enlaces de verificación y restablecimiento de contraseña
  emergen en los registros de desarrollo local.
- La cobertura de restablecimiento de contraseña fija el
  comportamiento de revocación de sesión y remember-me.

### Notas

- **Modelo de distribución**: basado en git de punta a punta.
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`;
  la CLI vía `cargo install --git`. Nada se publica en crates.io.
