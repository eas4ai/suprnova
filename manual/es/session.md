# Sesiones

La sesión es la bolsa de clave/valor por usuario que sobrevive entre
solicitudes del mismo navegador. Suprnova incluye de serie un driver
respaldado por base de datos, lo cablea mediante `SessionMiddleware` y
expone la sesión activa a través de dos funciones libres - `session()`
para leer y `session_mut()` para escribir. Úsala siempre que un valor
deba sobrevivir a una solicitud pero no sea algo que la URL o un JWT
deban llevar.

## Cómo ve una solicitud la sesión

`SessionMiddleware` se ejecuta en cada solicitud y hace cinco cosas en
orden:

1. Lee el id de sesión y la marca de tiempo del último toque de actividad
   con éxito desde la cookie `suprnova_session` (cifrada con
   AES-256-GCM). Las cookies manipuladas, indescifrables o malformadas se
   tratan como ausentes.
2. Carga `SessionData` del almacén solo cuando una cookie válida nombra
   una sesión. Las solicitudes sin cookie arrancan con una sesión limpia
   en memoria y no lanzan a la base de datos una consulta que con
   seguridad no encontraría nada. Una cookie cuya fila ya no existe se
   limpia sin recrear una fila vacía. Un error de lectura del almacén
   registra un `warn!` y deja continuar a una solicitud sin estado, pero
   entonces una mutación del handler falla en cerrado en lugar de
   sobrescribir un estado almacenado desconocido.
3. Envejece los datos flash: `_flash.old.*` se descarta y `_flash.new.*`
   se renombra a `_flash.old.*`. Después de este paso, todo lo que la
   solicitud anterior dejó en flash es legible; todo lo que esta
   solicitud deje en flash será legible la próxima vez.
4. Vincula la sesión a una ranura task-local mientras dure el handler.
   `session()` y `session_mut()` consultan esa ranura.
5. Después de que el handler devuelve, persiste el estado de sesión sucio
   o un toque acotado de expiración deslizante, adjunta una cookie
   cifrada de reemplazo solo después de una escritura con éxito, y vuelca
   las cookies pendientes fuera de banda (por ejemplo, una cookie de
   "recuérdame" recién rotada). Una solicitud limpia y sin cookie no hace
   ninguna E/S contra el almacén de sesiones y no recibe ninguna cookie
   de sesión.

El paso 5 tiene una garantía de seguridad que vale la pena destacar: **si
la sesión se modificó en esta solicitud y la escritura en el almacén
falla, la respuesta se sustituye por un 500.** Devolver el éxito del
handler significaría entregarle al cliente una cookie para un estado que
la base de datos nunca registró - la siguiente solicitud cargaría una
sesión vacía y la mutación (login, rotación de CSRF, flash) se
desvanecería en silencio. Las solicitudes de solo lectura que fallan
únicamente en un toque de `last_activity` que ya correspondía registran
un `warn!`, conservan la cookie existente y siguen adelante.

## Leer la sesión

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // primera visita
    }
}
```

`session()` clona el `SessionData` actual. Devuelve `None` fuera de un
alcance de solicitud (un test unitario que no instaló el middleware, un
subcomando de la CLI). Para un valor tipado, `get::<T>` deserializa desde
el JSON subyacente; ante una clave ausente o un tipo equivocado obtienes
`None` y ningún pánico.

## Escribir en la sesión

`session_mut` toma un closure que recibe `&mut SessionData`:

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

El closure es síncrono - las guardas del bloqueo subyacente se liberan
antes de cualquier `.await`, así que esto se compone dentro de handlers
asíncronos sin mantener el bloqueo a través de las suspensiones. Todo lo
que serialices tiene que implementar `Serialize`; la deserialización en
`get` requiere `DeserializeOwned`.

La forma de closure (en lugar de devolver una guarda) es deliberada. Los
futures en Tokio pueden reanudarse en un hilo de trabajo distinto de
aquel en el que empezaron, así que la sesión tiene que vivir en una
ranura `task_local!` y tomarse prestada a través de una sección crítica
acotada por un alcance. La forma `|s|` hace explícito ese límite y te
impide mantener por accidente una guarda de mutex a través de un
`.await`.

## Datos flash

Los valores flash son visibles durante **una** solicitud posterior y
luego desaparecen. El patrón habitual: un controlador escribe un flash,
devuelve una redirección, y la página siguiente renderiza el flash.

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

En la siguiente solicitud:

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` elimina el valor al devolverlo. Para la variante que lee sin
consumir usa `get::<String>("_flash.old.status")`, pero la forma que
consume es la que suelen querer los controladores.

Está disponible la superficie flash completa de Laravel:

- `flash(key, value)` - escribe para la siguiente solicitud
- `now(key, value)` - escribe solo para la solicitud actual
- `reflash()` - vuelve a poner en flash, durante un turno más, todo lo
  que ahora mismo es visible
- `keep(&["k1", "k2"])` - vuelve a poner en flash un subconjunto concreto
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - la bolsa de
  entrada de formulario que usan los ayudantes `Redirect::with_input` /
  `old()`

## Regenerar e invalidar

Tras un cambio de credenciales (login, restablecimiento de contraseña,
superar el 2FA) rotas el id de sesión para que un id fijado desde antes
del cambio deje de ser válido:

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // id nuevo, mismos datos
regenerate_csrf_token();        // token CSRF nuevo, mismo id y mismos datos
```

Para limpiar la sesión por completo (logout):

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // limpia los datos + acuña un token CSRF nuevo
```

Para un evento de seguridad que necesite revocar todas las sesiones de un
usuario (restablecimiento de contraseña en otro sitio, recuperación de la
cuenta, cierre de sesión forzado por un administrador):

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

`destroy_all_for_user` resuelve el `SessionStore` registrado por
`SessionMiddleware::new` o `with_store` y llama a `destroy_for_user` en ese
almacén configurado. Solo recurre a un `DatabaseSessionDriver` nuevo cuando
no se registró ningún almacén de sesión, como en un test o embedder que nunca
construyó el middleware.

## Ayudantes de autenticación

`auth_user_id()` devuelve el id del usuario autenticado en ese momento
(consultando primero el estado de autenticación con alcance de solicitud
y recurriendo después al campo persistido en la sesión):

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

Lo normal es manejar la autenticación a través de la fachada
[Auth](authentication.md) - `Auth::login`, `Auth::logout`,
`Auth::user()`. Los ayudantes de sesión son la capa de bajo nivel sobre
la que se apoyan esas fachadas; échales mano cuando necesites inspeccionar
la sesión en crudo o cuando estés implementando tu propio guard.

## Otras operaciones

La API de `SessionData` refleja la superficie de `Store` de Laravel:

| Método | Qué hace |
|---|---|
| `get::<T>(key)` | lectura tipada |
| `put(key, value)` | escritura tipada |
| `forget(key)` | elimina una sola clave |
| `forget_many(&[..])` | elimina varias claves |
| `flush()` | limpia todos los datos (conserva el id) |
| `has(key)` / `missing(key)` | comprobación de presencia |
| `has_any(&[..])` / `has_all(&[..])` | presencia en bloque |
| `all()` | toma prestado el mapa subyacente |
| `only(&[..])` / `except(&[..])` | clones filtrados |
| `pull::<T>(key)` | obtener y olvidar de una sola vez |
| `push(key, value)` | añade a un valor de tipo array |
| `increment(key, n)` / `decrement(key, n)` | contadores enteros |
| `remember::<T>(key, \|\| default())` | obtener, o calcular y guardar |
| `replace(&[(k, v), ..])` | vacía y luego escribe en bloque |
| `put_many(&[(k, v), ..])` | escritura en bloque que fusiona |
| `previous_url()` / `set_previous_url(url)` | lo que lee `Redirect::back` |
| `password_confirmed()` / `password_confirmed_at()` | marca de tiempo de "el usuario acaba de confirmar la contraseña" |

Echa mano de estas dentro de `session_mut` para las operaciones que mutan y de
`session()` para las lecturas. La ranura `previous_url` la puebla
automáticamente el middleware en las respuestas GET HTML con éxito, así que
`redirect()->back()` funciona sin que hagas nada. El middleware solo registra
una URL relativa a la raíz y del mismo origen: una ruta de solicitud que
empieza por `//` o `/\` (ambas se interpretan como relativas al protocolo por
un navegador) o que lleva un byte de control ASCII en cualquier parte (un
`TAB` o un salto de línea permite que un valor que solo parece relativo a la
raíz se convierta en una de esas dos formas una vez que el analizador de URL
del navegador lo elimina) nunca se almacena. `previous_url()` también repite
la misma regla en cada lectura, por lo que un valor escrito por una versión
anterior, antes de que existiera esa protección al escribir, se lee como
ausente en vez de recibir confianza. En cualquier caso, `Redirect::back()`,
`Redirect::refresh()` y `url::previous()` nunca pueden resolver a un
`Location` fuera de tu app a partir de un valor que guardó esta ranura.

## Configuración

Configura las sesiones mediante variables de entorno -
`SessionConfig::from_env` las lee en el arranque:

```env
# Duración en minutos. Gobierna tanto el TTL de la fila como el Max-Age de la cookie.
SESSION_LIFETIME=120

# Segundos mínimos entre escrituras de expiración deslizante (5 minutos por defecto).
# En tiempo de ejecución esto se acota por debajo de la duración de la sesión.
SESSION_TOUCH_INTERVAL=300

# Cadencia en segundos de la recolección supervisada de filas caducadas (1 hora por defecto).
SESSION_GC_INTERVAL=3600

# Nombre de la cookie en el cliente.
SESSION_COOKIE=suprnova_session

# Atributos de la cookie
SESSION_SECURE=true          # exige HTTPS; POR DEFECTO ES true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # opcional; sin establecer = solo el host
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_COOKIE_PREFIX=       # empty | __Secure- | __Host-
SESSION_PARTITIONED=false    # opt-in a CHIPS
SESSION_EXPIRE_ON_CLOSE=false # true → omite Max-Age, el navegador la descarta al cerrar

# Conexión de BD con nombre para el almacén de sesiones (opcional)
SESSION_CONNECTION=sessions

# Duración en minutos del token/cookie de "recuérdame" (30 días por defecto)
REMEMBER_LIFETIME=43200
```

Vale la pena señalar algunos valores por defecto:

- **`SESSION_SECURE` es `true` por defecto.** Las sesiones enviadas por
  HTTP en claro serían un riesgo de fuga de credenciales, así que el flag
  secure viene activado por defecto. Para el desarrollo local sobre HTTP,
  pon `SESSION_SECURE=false` en tu `.env` local.
- **`HttpOnly` está siempre activado.** No hay ningún mando para
  desactivarlo - exponer la cookie de sesión a JavaScript renuncia a la
  principal protección contra XSS, y hoy no hay ninguna razón legítima
  para quererlo.
- **`SameSite` es `Lax` por defecto.** `Strict` bloquea la sesión en la
  mayoría de las navegaciones GET entre sitios (incluidos los enlaces de

  vuelta desde el correo); `Lax` es la respuesta correcta habitual.

### Endurecimiento del prefijo del nombre de cookie

`SESSION_COOKIE_PREFIX=__Host-` hace que el navegador bloquee al host las
cookies de sesión y remember-me. Una cookie `__Host-` debe ser `Secure`, usar
`Path=/` y omitir `Domain`; una cookie `__Secure-` debe ser `Secure`.
Suprnova aplica estas reglas al renderizar a partir del nombre final de la
cookie, así que el orden del builder y las cookies encoladas reciben la misma
protección.

`Config::init` valida el prefijo, `SESSION_DOMAIN` y `SESSION_PATH` durante
el arranque y falla antes de servir cuando la combinación no es válida. La
aplicación en tiempo de renderizado aún fuerza `Secure` para cualquiera de
los prefijos y reescribe una ruta `__Host-` a `/`; elimina un `Domain` en
`__Host-` y registra una advertencia porque estrecha el alcance solicitado.
El navegador descarta silenciosamente una cookie con prefijo no válido, así
que revisa el diagnóstico de arranque antes del despliegue.

Para el desarrollo local con HTTP, deja el prefijo vacío y establece
`SESSION_SECURE=false` solo en el entorno local. Para producción, despliega
HTTPS, conserva `SESSION_SECURE=true`, usa `SESSION_COOKIE_PREFIX=__Host-`,
mantén `SESSION_PATH=/` y deja `SESSION_DOMAIN` sin establecer.

Lista de comprobación de despliegue:

1. Confirma que el origen público usa HTTPS, incluidas las comprobaciones de
   estado y la primera redirección.
2. Establece `SESSION_COOKIE_PREFIX=__Host-`, `SESSION_SECURE=true` y
   `SESSION_PATH=/`.
3. Elimina `SESSION_DOMAIN`; el validador de arranque lo rechaza con
   `__Host-`.
4. Inspecciona la primera respuesta `Set-Cookie` para ver
   `__Host-suprnova_session`, `Secure` y `Path=/`, sin `Domain`.

### Por qué Suprnova diverge

Laravel no expone un control de prefijo de cookie de primera clase en su
configuración de sesión. Suprnova hace que el prefijo sea un valor de
configuración con validación al arrancar porque el modo de fallo es silencioso
en el navegador: una cookie no válida se descarta antes de que el código de
aplicación pueda informar un fallo de sesión.

Para la configuración programática usa el builder fluido:

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1 hora
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```

`SessionConfig` es `#[non_exhaustive]`; usa un valor por defecto y
asigna el campo público cuando la configuración programática necesite un
prefijo:

```rust
use suprnova::{CookiePrefix, SessionConfig};

let mut config = SessionConfig::default();
config.cookie_prefix = CookiePrefix::Host;
```

## El cableado

`SessionMiddleware` se instala como middleware global en el bootstrap de
tu aplicación. El orden del middleware importa: la sesión tiene que ir
antes que [CSRF](csrf.md), porque CSRF lee el token por sesión.

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` registra además el supervisor de GC configurado.
    // Usa `SessionMiddleware::new(config)` si prefieres programar tú
    // mismo el GC vía `Schedule`.
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` registra una tarea de gc
[supervisada](supervisors.md) que llama a `gc()` cada
`SESSION_GC_INTERVAL` (una vez por hora por defecto). La variante
`install_with_gc(config, interval).await` toma un intervalo propio;
`new(config)` se salta la tarea de gc (útil si prefieres llamar a `gc()`
desde una entrada de [Schedule](scheduling.md)). La tarea supervisada
participa en el drenaje de apagado del framework, así que el bucle de gc
sale limpiamente ante `Ctrl-C` / `SIGTERM` en lugar de ser abortado a la
fuerza.

Los endpoints de operaciones protegidos pueden exponer el estado del
recolector sin consultar la tabla de sesiones:

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

Para usar un almacén que no sea de base de datos - para tests, o para un
driver respaldado por Redis que escribas tú mismo - implementa
`SessionStore` y pásalo vía `with_store`:

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

## La tabla de sesiones

El driver por defecto espera una tabla `sessions` con esta forma (la
entidad de SeaORM en `framework/src/session/driver/database.rs` es la
fuente de la verdad):

| Columna | Tipo | Notas |
|---|---|---|
| `id` | VARCHAR PK | id de sesión alfanumérico en minúsculas de 40 caracteres |
| `user_id` | VARCHAR NULL | id del usuario autenticado (cadena, admite ids opacos) |
| `payload` | TEXT | mapa de datos de sesión serializado en JSON |
| `csrf_token` | VARCHAR | token CSRF por sesión |
| `last_activity` | TIMESTAMP | último acceso; gobierna la caducidad + el GC |

Junto a la tabla vienen dos índices: `idx_sessions_user_id` (para
`destroy_for_user`) e `idx_sessions_last_activity` (para `gc()`).

Una aplicación creada con el andamiaje incluye una migración
`create_sessions_table` que se ajusta a esta forma. Si traes tus propias
migraciones, replica los nombres de las columnas exactamente - SeaORM los
resuelve posicionalmente y una columna renombrada no encajará.

### Por qué Suprnova diverge

Dos sitios donde Laravel tomó una decisión con forma de PHP que Tokio nos
permite tomar de otra manera:

**Recolección de basura.** Laravel ejecuta una lotería de 2/100 en cada
solicitud: cada solicitud tiene un 2 % de probabilidad de disparar el GC
de sesiones en línea. En PHP funciona porque cada solicitud levanta un
proceso nuevo de todas formas. En Tokio tenemos workers de larga vida,
así que `SessionMiddleware::install` registra una única tarea
[supervisada](supervisors.md) que llama a `gc()` a intervalos fijos. Sin
sobrecarga por solicitud y sin sorpresas probabilísticas - programación
explícita en lugar de una lotería, y el bucle de reinicio del supervisor
atrapa los pánicos, de modo que un solo gc defectuoso no mata al demonio.

**`session_mut` en forma de closure.** Laravel te entrega
`$request->session()` y te deja llamar a métodos sobre él. Nosotros no,
porque los handlers de Suprnova son futures y un future puede reanudarse
en un hilo de trabajo distinto de aquel en el que empezó. La sesión vive
en una ranura `task_local!` de Tokio, lo que significa que el acceso
prestado tiene que ocurrir dentro de un alcance. La forma de closure hace
explícito ese alcance e impide estáticamente el error de mantener una
guarda de mutex a través de un `.await`.

**Fallo en cerrado ante las escrituras sucias.** Un toque de actividad
acotado que falla registra un `warn!` y deja pasar la solicitud con su
cookie existente (el estado visible para el usuario está intacto). Una
escritura fallida de una sesión *modificada* - login, flash, rotación de
CSRF - devuelve un 500. Entregarle en silencio al cliente una cookie para
un estado que el almacén nunca registró haría que un login "con éxito" se
desvaneciera en la siguiente solicitud; mejor sacar el fallo a la luz de
forma estrepitosa.

## Siguiente

- [Autenticación](authentication.md) - `Auth::login`, los guards, la
  cadena de proveedores de usuario
- [Flujos de autenticación](auth-flows.md) - restablecimiento de
  contraseña, 2FA, limitación de fuerza bruta, "recuérdame"
- [CSRF](csrf.md) - cómo se comprueba el token CSRF de la sesión en las
  escrituras
- [Middleware](middleware.md) - escribir tu propio middleware que lea o
  escriba la sesión
- [Ciclo de vida de la solicitud](lifecycle.md) - dónde se sitúa
  `SessionMiddleware` en la cadena
