# WebSockets

Las rutas WebSocket de Suprnova conviven con las rutas HTTP en el mismo
router. Registras una ruta y un handler; el framework detecta la
solicitud `Upgrade: websocket` en esa ruta, ejecuta la misma cadena de
middleware que ejecutaría un GET HTTP a esa ruta, completa el handshake
de RFC 6455, y llama a tu handler con un `WsSocket` tipado más el
`Request` original. No existe un servidor WebSocket separado - las
conexiones se actualizan desde el mismo listener de hyper que sirve tu
tráfico HTTP. El framework también rastrea cada handler generado en un
`JoinSet` por servidor, así que un apagado ordenado drena las
conexiones en vuelo antes de que el listener termine.

## Inicio rápido

Añade un `EchoHandler` y regístralo en `routes!`.

`src/ws/echo.rs`:

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs` (dentro de `routes! { ... }`):

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

Arranca la app y conéctate con `wscat`:

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

Cuando `recv_text()` devuelve `Ok(None)` significa que el peer cerró
la conexión; el bucle termina, el handler devuelve `Ok(())`, y el
framework envía un frame Close(1000) limpio.

## Ciclo de vida de una actualización

Un handshake de WebSocket es un GET HTTP con `Upgrade: websocket`. El
framework ejecuta el pipeline de solicitud completo sobre ella antes
de que fluya ningún frame:

1. **Coincidencia de ruta.** El router busca la ruta en la tabla de
   rutas WS; si no hay coincidencia, la solicitud cae al fallback
   HTTP.
2. **Política de origen.** Se aplica la [`OriginPolicy`](#política-de-origen)
   configurada. Una violación devuelve HTTP 403 sin actualización.
3. **Negociación de subprotocolo.** Si la ruta tiene
   `accepted_protocols`, el primer token ofrecido por el cliente que
   coincide se repite en la respuesta 101.
4. **Cadena de middleware.** `RequestIdMiddleware` se ejecuta más
   externo, seguido de todo el middleware registrado globalmente,
   seguido del middleware por ruta de la ruta. Una respuesta que no es
   2xx desde cualquier middleware hace cortocircuito en la
   actualización - el peer recibe el error HTTP, y el future de
   WebSocket se descarta limpiamente.
5. **Handshake.** `hyper_tungstenite::upgrade` produce el future que
   se resuelve en un `WebSocketStream`.
6. **Despacho al handler.** El `Request` (posiblemente reescrito por
   el middleware) y un `WsSocket` recién construido se entregan a
   `WebSocketHandler::handle`.
7. **Heartbeat + handler.** El framework genera una tarea de
   heartbeat por conexión y espera el future del handler bajo un span
   de tracing `ws.connection` que lleva el request id.
8. **Handshake de cierre.** Ante `Ok(())` el framework envía
   Close(1000); ante `Err(_)` envía Close(1011 "internal error"). Se
   espera al forwarder para que el frame de cierre se vacíe hacia la
   red antes de que la tarea rastreada de la conexión se reporte como
   terminada.

La semántica del valor de retorno está invertida respecto a HTTP: no
hay cuerpo. `Ok(())` significa desconexión limpia; `Err(_)` se
registra en el log y el peer ve Close(1011). De cualquier forma, la
conexión se derriba.

## La API de `WsSocket`

`WsSocket` es el handle bidireccional que el framework pasa a tu
handler. Internamente, el stream de tungstenite subyacente se divide
en mitades Sink + Stream: una tarea forwarder posee el sink y drena un
mpsc; los métodos de envío de cara al handler encolan sobre el mpsc.
El handler lee directamente de la mitad stream. Esta división
significa que el framework también puede empujar frames (pings de
heartbeat, dispersión del broadcaster) sin competir con la ruta de
envío del handler.

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

Encola un frame de texto UTF-8. Devuelve `Err` solo cuando la conexión
ya está cerrada.

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

Encola un frame binario. Acepta cualquier cosa `Into<Vec<u8>>`. Misma
semántica de error que `send_text`.

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) significa que el peer cerró.
```

Devuelve el siguiente mensaje de texto, descartando en silencio los
tipos de frame que a un handler solo-texto no se espera que le
importen:

- `Message::Binary` - payload binario del peer
- `Message::Ping` - ping iniciado por el peer (tungstenite gestiona el pong automáticamente)
- `Message::Pong` - respuesta pong del peer a un heartbeat del
  framework (el contador de pings perdidos se reinicia a cero como
  efecto secundario)
- `Message::Frame` - variantes de frame en crudo provenientes de
  contextos del lado del servidor; nunca se esperan en esta capa

Un frame tragado desaparece; no hay forma retroactiva de verlo. Si el
handler necesita observar frames binarios o códigos de cierre, usa
[`recv`](#recv) desde la primera lectura.

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

Devuelve el siguiente mensaje de cualquier tipo, incluyendo Binary,
Ping, Pong, y Close. `Pong` de todos modos reinicia el contador de
pings perdidos como efecto secundario antes de ser devuelto.
`Ok(None)` significa que el stream subyacente terminó.

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

Encola un frame de cierre y retorna. El forwarder escribe el frame en
el sink, llama a `close()` sobre el sink, y termina. Los envíos
posteriores sobre el mismo socket devuelven `Err` porque el forwarder
ya no existe. Siempre devuelve `Ok(())` inmediatamente después de
llamar a `close`.

`close` valida sus argumentos por adelantado contra RFC 6455 §7.4 +
§5.5.1:

- `code` debe satisfacer `CloseCode::is_allowed()`. Los códigos
  reservados o inválidos (1004, 1005, 1006, 1015, cualquiera por
  debajo de 1000, cualquiera por encima de 4999) se rechazan con
  `Err` y **no se envía ningún frame** - la conexión permanece
  abierta y quien llama puede reintentar con un código válido. Usa
  1000 para un cierre normal, 1001-1013 para las razones definidas,
  3000-3999 para códigos registrados en IANA, o 4000-4999 para
  códigos privados de la aplicación.
- `reason` está topado a 123 bytes (el límite de 125 bytes del frame
  de control menos los dos bytes del código). Las razones más largas
  se rechazan sin encolar nada.

### Por qué Suprnova diverge

Los frameworks de PHP añaden soporte de WebSocket como un proceso
separado (ratchet, soketi, pusher). La ruta WebSocket de Suprnova vive
en el mismo `routes! { ... }` que tus rutas HTTP, servida por el mismo
listener de hyper, drenada por la misma ruta de apagado ordenado. Hay
un solo binario, una sola configuración, un solo deploy. Las
conexiones de larga duración son de primera clase porque Tokio las
hace baratas; el framework no tiene que disculparse por ellas.

## Parámetros de ruta

Las rutas WebSocket admiten la misma sintaxis de captura `{param}`
que las rutas HTTP. Los valores capturados están disponibles en el
`Request` que se pasa al handler.

```rust
// En routes!:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` devuelve `Result<&str, ParamError>`; el `?` propaga
un `FrameworkError::ParamError` si falta el segmento, lo cual hace que
el handler devuelva `Err` y que el framework envíe Close(1011). En la
práctica la captura siempre está presente cuando la ruta coincidió -
la ruta de error es una red de seguridad contra errores de tipeo en
el nombre del parámetro.

Los segmentos estilo Express `:id` también se aceptan
(`ws!("/ws/rooms/:id", h)`) y se convierten internamente a la forma
de matchit.

Para la API completa de `Request` - encabezados, cookies, cadena de
consulta, dirección del peer - ver [la documentación de
requests](requests.md).

## Middleware por ruta

Encadena `.middleware(M)` sobre la entrada `ws!`. Varios middleware se
componen de izquierda a derecha y se ejecutan en el mismo orden fijo
en que se ejecutaría una solicitud HTTP a la misma ruta:
`RequestIdMiddleware` más externo, luego todo el middleware
registrado globalmente, luego la cadena de ruta, luego el handler.

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

Una respuesta que no es 2xx desde cualquier middleware hace
cortocircuito en la actualización. El peer recibe el rechazo (p. ej.
401, 403) con `X-Request-Id` establecido, el future de WebSocket sin
despertar se descarta limpiamente, y el handler nunca se llama. Esta
es la capa correcta para las comprobaciones a nivel de transporte:
quién puede abrir la conexión en absoluto, de dónde viene la
conexión, cuántas conexiones concurrentes por identidad.

Un middleware puede sustituir un `Request` modificado llamando a
`next(modified_req)`. El terminador captura lo que sea que la cadena
finalmente deje pasar, y eso es lo que el handler ve como su
argumento `Request`. Un middleware que resuelve identidad (una
búsqueda de sesión, una comprobación de token) puede adjuntar el
resultado mediante extensiones de `Request`; el handler lo lee de
vuelta de la misma forma en que lo hacen los controladores HTTP.

Las variantes directas sobre `Router` (`Router::ws`,
`Router::ws_with_middleware`, `Router::ws_with_config`,
`Router::ws_with_middleware_and_config`) cubren la misma superficie
para código que construye un `Router` fuera de la macro. Cada una
tiene una contraparte falible `try_*` que devuelve
`Err(FrameworkError)` ante patrones duplicados o malformados en lugar
de entrar en pánico.

### Por qué Suprnova diverge

La mayoría de los ecosistemas o bien se saltan el middleware en las
actualizaciones de WebSocket (la convención de Node) o bien fuerzan
una ceremonia de registro separada para el "middleware de WebSocket"
(la convención de .NET / Spring). Suprnova trata la actualización
como el GET HTTP que en realidad es: se ejecuta la misma cadena, en
el mismo orden, con la misma semántica de cortocircuito. No hay un
segundo concepto que aprender - `AuthMiddleware`, `RateLimitMiddleware`,
`RequestIdMiddleware`, `CorsMiddleware` funcionan en rutas WS porque
funcionan en cualquier ruta. La aplicación de origen es la única
arruga adicional, y es una propiedad de `WsConfig`, no un middleware
separado.

## Autenticación al conectar

El handler recibe el `Request` reescrito por el middleware. Tres
patrones funcionan bien, en orden creciente de integración con el
resto del framework:

**Patrón 1 - bearer token en línea dentro del handler.** El más
simple. Funciona sin ningún middleware de auth. `wscat`, los clientes
de navegador, y los balanceadores de carga pasan los encabezados sin
problema.

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**Patrón 2 - controlar la actualización con un middleware por ruta.**
Rechaza las aperturas no autorizadas antes de que fluya ningún frame.
Separación de responsabilidades más limpia; el handler solo ve
conexiones autenticadas.

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` devuelve 401 ante solicitudes no autenticadas; la
actualización se aborta con la respuesta de rechazo y el handler
nunca se llama.

**Patrón 3 - control por middleware más relectura en el handler.** El
middleware hace cortocircuito en las aperturas no autorizadas; el
handler entonces vuelve a leer la misma credencial (token, cookie,
etc.) que sabe que ahora está presente para identificar qué usuario
acaba de conectarse:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // El middleware ya verificó el bearer; solo llegamos aquí si era válido.
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**Patrón 4 - deja que el middleware autentique y lee el resultado.**
Preferido cuando ya se ejecuta un middleware de auth sobre la
actualización. La identidad que resolvió se transporta en la propia
solicitud:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` proviene del middleware de sesión/token, no de algo
    // que el cliente haya enviado en un frame.
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

Esto es lo que hace significativo el hook `authorize` de un canal de
difusión privado: recibe el mismo `Request`, así que puede controlar
el acceso según una identidad derivada del servidor en lugar de un
valor elegido por el cliente. Antes de que existiera `auth_user_id`,
un canal no tenía nada confiable que consultar, y el placeholder
obvio - "aceptar cualquier suscriptor cuyo frame de suscripción lleve
un token que parezca correcto" - no es en absoluto un control de
acceso.

Los accesores thread-local que funcionan en los controladores HTTP -
`session()`, `Auth::user()`, la bolsa `Context` por solicitud - de
todos modos **no** están poblados dentro de un handler de WebSocket.
Los alcances task-local de la cadena de middleware se desenrollan
cuando la cadena retorna; el handler se ejecuta en una tarea recién
generada que solo hereda el request id y el id de auth resuelto. Lee
todo lo demás que el handler necesite directamente del `Request`
(encabezados, cookies mediante `req.cookie("...")`, parámetros
capturados, el bearer token mediante `req.bearer_token()`) - esos sí
sobreviven hasta la tarea del handler.

### Por qué Suprnova diverge

Laravel autoriza los canales de difusión mediante un endpoint HTTP
separado (`/broadcasting/auth`), así que el callback del canal se
ejecuta en una solicitud ordinaria con la sesión completa disponible.
Suprnova en cambio autoriza dentro del proceso durante la
actualización - una sola conexión, sin una segunda ida y vuelta - lo
cual significa que la identidad tiene que transportarse
explícitamente a través del límite del spawn en lugar de volver a
buscarse.

## `WsConfig`

`WsConfig` controla el comportamiento por conexión. Los valores por
defecto apuntan a endpoints públicos de cara al navegador - cada
conexión activa reserva un búfer de tungstenite del tamaño de
`max_message_size`, así que el framework usa valores pequeños por
defecto y deja que las rutas que necesiten más eleven los límites
explícitamente.

| Campo                 | Por defecto    | Tipo            | Efecto |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | Con qué frecuencia el framework envía un frame Ping para mantener viva la conexión. |
| `max_message_size`    | 1 MiB          | `usize`         | Tamaño máximo del mensaje reensamblado, en bytes. Los mensajes más grandes son rechazados por tungstenite. |
| `max_frame_size`      | 64 KiB         | `usize`         | Tamaño máximo de un único frame de WebSocket, en bytes. |
| `max_missed_pings`    | 2              | `usize`         | Pongs perdidos consecutivos antes de que el heartbeat cierre la conexión con código 1011. `usize::MAX` desactiva la aplicación de este límite. |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | Comprobación del encabezado `Origin` aplicada en el momento de la actualización. Ver [Política de origen](#política-de-origen). |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | Tokens `Sec-WebSocket-Protocol` aceptados por el servidor. Vacío significa que no hay negociación. Ver [Subprotocolos](#subprotocolos). |

Anulaciones recomendadas según el caso de uso:

- **Chat / notificaciones / posiciones de cursor** - los valores por
  defecto están bien. Baja `ping_interval` a 5-10s si tu LB tiene un
  timeout de inactividad agresivo.
- **Feeds internos de confianza** (dispersión servidor-a-servidor,
  exportación masiva, transferencias binarias grandes) - parte de
  `WsConfig::generous()`, que eleva `max_message_size` a 64 MiB y
  `max_frame_size` a 16 MiB manteniendo el resto de los valores por
  defecto.
- **Un payload sobredimensionado específico** (una ruta que sube
  archivos de audio de 256 MiB) - establece los campos directamente;
  no apliques el límite más grande a rutas que no lo necesitan.

El struct de configuración se puede construir con `Default` y cada
campo es público:

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

Aplica la anulación por ruta ya sea en la entrada `ws!` o en
`Router::ws_with_config`:

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` se valida en el momento de registrar la ruta. Un
`ping_interval` en cero o un `max_missed_pings` en cero corromperían
la tarea de heartbeat; ambos se rechazan en el arranque en lugar de
entrar en pánico en la primera conexión.

### Heartbeat y cierre al no recibir pong

Para cada conexión actualizada, el framework genera una tarea de
heartbeat que envía `Ping(b"")` cada `ping_interval`. En cada tick el
contador de pings perdidos se incrementa; en cada Pong del peer se
reinicia a cero. Si el contador llega a `max_missed_pings`, el
heartbeat envía Close(1011 "no pong response") y la conexión se
derriba. Establece `max_missed_pings` en `usize::MAX` para desactivar
la aplicación de este límite (los pings siguen fluyendo, pero la
conexión nunca se cierra por pongs faltantes).

El primer tick se consume al iniciar la tarea, así que el peer
obtiene al menos un intervalo completo de gracia antes del primer
ping.

## Política de origen

Los navegadores siempre envían un encabezado `Origin` en los
handshakes de WebSocket. A diferencia de `fetch()` /
`XMLHttpRequest`, las actualizaciones de WebSocket no están
protegidas por el middleware de token CSRF (el handshake no lleva
ningún token), así que una comprobación de `Origin` del mismo origen
es lo único que se interpone entre una página maliciosa y un
endpoint WS privilegiado sobre la sesión de un usuario que ya inició
sesión. El framework aplica la política configurada antes de que se
llame a `hyper_tungstenite::upgrade`; una violación devuelve HTTP 403
sin actualización.

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| Variante     | Comportamiento |
|--------------|----------|
| `SameOrigin` (por defecto) | Permite solo cuando el host de `Origin` (y el puerto, si está presente) coincide con el encabezado `Host` de la solicitud. Un `Origin` ausente se rechaza. El scheme no se compara (TLS termina en un punto anterior, así que el servidor no puede saber con fiabilidad si el scheme público era https o http). |
| `AllowAny`   | Se salta la comprobación. Úsalo solo para endpoints que no son de navegador (servidor a servidor, apps nativas, mocks de test). |
| `AllowList(Vec<String>)` | Permite solo cuando `Origin` coincide exactamente (sin distinguir mayúsculas/minúsculas) con uno de los orígenes suministrados. Cada entrada es la forma completa `scheme://host[:port]` que enviaría un navegador. |

Los clientes que no son de navegador (herramientas CLI, servidores,
apps nativas) típicamente no envían un encabezado `Origin`. Las rutas
que sirven exclusivamente a esos clientes deberían usar `AllowAny`;
las rutas que sirven a ambos deberían usar `AllowList` enumerando
cada origen de frontend de producción.

## Subprotocolos

Un subprotocolo de WebSocket es un token de nivel de aplicación (p.
ej. `graphql-transport-ws`, `jsonrpc-2.0`) que el cliente y el
servidor acuerdan durante el handshake. Rellena `accepted_protocols`
para participar:

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

Cuando el cliente ofrece `Sec-WebSocket-Protocol`, el framework elige
el primer token ofrecido por el cliente (en el orden de preferencia
del cliente según RFC 6455 §4.2.2) que coincide con
`accepted_protocols`, comparado sin distinguir mayúsculas/minúsculas,
y lo repite en la respuesta 101. Si el cliente ofreció protocolos
pero ninguno coincidió, la actualización de todos modos tiene éxito
sin encabezado `Sec-WebSocket-Protocol` - RFC 6455 entonces exige que
el navegador falle la conexión del lado del cliente, lo cual es el
comportamiento correcto (un servidor que continuara estaría hablando
en silencio el protocolo equivocado).

Cuando `accepted_protocols` está vacío, la negociación se omite por
completo - la respuesta de actualización omite
`Sec-WebSocket-Protocol` y el cliente recurre al manejo de protocolo
por defecto.

## Despliegue en producción

El framework gestiona el handshake y la E/S de frames. No necesitas
ninguna configuración adicional del lado del framework para
producción.

**La terminación de TLS ocurre en un punto anterior.** Los clientes
se conectan a `wss://` en nginx, Caddy, o el balanceador de carga en
la nube; el proxy retira el TLS y reenvía `ws://` sin más al
framework. El framework no necesita una feature `rustls` ni un
certificado TLS.

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` y `proxy_send_timeout` deben ser lo bastante
largos para cubrir los huecos de inactividad entre heartbeats. Con el
`ping_interval` por defecto de 30s, 3600s es un techo cómodo.

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

Caddy gestiona `Upgrade` / `Connection` automáticamente al hacer de
proxy; las directivas `header_up` explícitas de arriba son solo para
claridad.

### Balanceadores de carga en la nube (AWS ALB, GCP GLB)

Activa el soporte de WebSocket en la regla del listener (AWS ALB hace
esto automáticamente cuando el protocolo del target group es HTTP/1.1
con las sticky sessions desactivadas). Asegúrate de que el timeout de
inactividad del balanceador de carga sea al menos tan largo como
`ping_interval`; el heartbeat del framework mantiene la red activa,
pero el LB descarta las conexiones que le parecen inactivas desde su
perspectiva.

## Apagado ordenado

Cada handler de WebSocket generado se rastrea en el `JoinSet`
`WS_TASKS` del servidor. Ante `Ctrl-C` o una señal de apagado externa,
el listener deja de aceptar conexiones nuevas y `Server::run` drena
el conjunto antes de que el proceso termine. El future del handler no
se resuelve hasta que el handshake de cierre se ha vaciado: después
de que el `handle` del usuario retorna, el framework espera al
forwarder para que el frame final Close(1000) o Close(1011) se
escriba hacia la red antes de que la tarea de la conexión se reporte
como terminada. En un apagado limpio los peers ven un cierre normal,
no un reset de TCP.

Los handles completados se cosechan de forma oportunista durante la
vida del servidor, así que el `JoinSet` no crece sin límite bajo una
operación de larga duración.

## Referencia

| Símbolo | Propósito |
|---|---|
| `suprnova::ws::WebSocketHandler` | Trait: `async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`. `Send + Sync + 'static`. |
| `suprnova::ws::WsSocket` | Handle bidireccional. Métodos: `send_text`, `send_binary`, `recv_text`, `recv`, `close`. `close` valida el código y la longitud de la razón por adelantado. |
| `suprnova::ws::WsConfig` | Configuración por conexión. Campos: `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`. Constructores `Default` + `generous()`. Validada en el registro. |
| `suprnova::ws::OriginPolicy` | `SameOrigin` (por defecto), `AllowAny`, `AllowList(Vec<String>)`. Se aplica en el momento de la actualización. |
| `ws!(path, Handler)` | Forma de macro para `routes! { ... }`. Devuelve un `WsRouteDef` que admite `.config(WsConfig)` y `.middleware(M)` en cualquier orden. |
| `Router::ws(path, handler)` | Registro directo. Devuelve `Router`. |
| `Router::ws_with_config(path, handler, cfg)` | Anulación de `WsConfig` por ruta. |
| `Router::ws_with_middleware(path, handler, mws)` | Lista de middleware por ruta. |
| `Router::ws_with_middleware_and_config(...)` | Ambas cosas. |
| `Router::try_ws*` family | Contrapartes falibles - devuelven `Err(FrameworkError)` ante patrones duplicados o malformados en lugar de entrar en pánico. |

## Siguiente

- [Difusión](broadcasting.md) - canales, presencia, el protocolo en la
  red sobre `ws!`
- [Eventos enviados por el servidor](sse.md) - push unidireccional
  para navegadores detrás de proxies estrictos
- [Enrutamiento](routing.md) - en qué se expanden realmente `routes!`
  y `ws!`
- [Middleware](middleware.md) - escribir middleware que controle el
  acceso a HTTP y WS de forma uniforme
- [Solicitudes](requests.md) - encabezados, cookies, query,
  extensiones sobre el `Request` que recibe tu handler
