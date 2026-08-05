# Cliente HTTP

La fachada `Http` es el lado saliente del HTTP - el equivalente en
Rust del helper `Http::` de Laravel. Recurres a ella cuando tu
handler, job o tarea programada necesita llamar a la API de otro: una
pasarela de pago, un geocodificador, un destino de webhook, un
mensaje de Slack. Builder fluido, JSON de entrada y salida, reintentos
con jitter, fakes de test deterministas que registran lo que
enviaste. La misma superficie que usabas en Laravel, con aislamiento
task-local para que los tests en paralelo no vean los fakes de los
demás.

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

Esa es la forma: `Http::<verbo>(url)` devuelve un `RequestBuilder`;
encadenas configuración sobre él; `.send().await` devuelve un
`ClientResponse`. El cliente que hay detrás es un único
`reqwest::Client` compartido, con TLS de rustls, un timeout por
defecto de 30 s, y un user agent `suprnova/<versión>` - construido de
forma perezosa en la primera llamada.

## Los verbos

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

Todo verbo devuelve un `RequestBuilder`. La URL puede ser cualquier
`impl Into<String>` - un `&str`, un `String`, o un `Cow<str>`. La
fachada no incluye ayudantes para construir URLs; formatea la URL tú
mismo o recurre a un crate de query strings.

## Cuerpos

Tres formas de adjuntar un cuerpo. Cada una sustituye cualquier
cuerpo establecido antes.

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` acepta cualquier cosa que implemente
`serde::Serialize`. El `Content-Type` en la red se establece
automáticamente en `application/json`. Si la serialización falla
(p. ej. un map con una clave que no es string), el builder registra
el error y `send()` lo expone en lugar de enviar en silencio un
cuerpo `null`.

### Formulario

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` serializa el valor como
`application/x-www-form-urlencoded`. El valor debe serializar a un
objeto JSON; las claves se convierten en campos de formulario. La
misma semántica de error de cuerpo que `.json` - un fallo de
serialización se expone a través de `send().await?`, nunca como un
cuerpo vacío silencioso.

### Bytes en crudo

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` toma cualquier cosa `impl Into<Bytes>`. El encabezado
`Content-Type` queda bajo tu responsabilidad - `.body` no establece
ninguno.

## Encabezados y autenticación

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` añade; el framework no elimina duplicados, así
que dos llamadas con el mismo nombre envían dos encabezados y reqwest
los junta según la semántica de HTTP. Dos atajos para los esquemas de
autenticación comunes:

- `.bearer_token(token)` - establece `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - establece `Authorization: Basic
  <b64>`; `password` es `Option<&str>`, así que
  `.basic_auth("api-key", None)` codifica la forma `api-key:` que
  algunos proveedores exigen

## Tiempos de espera

El cliente compartido tiene un timeout por defecto de 30 segundos.
Anúlalo por solicitud cuando lo necesites:

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` anula tanto el timeout de conexión como el de la
solicitud completa para esta única llamada. No hay un control
`connect_timeout` separado en el builder; el cliente de reqwest
subyacente usa un único timeout combinado.

## Redirecciones

El cliente compartido sigue las redirecciones por defecto (hasta el
tope de reqwest, 10) - el comportamiento correcto cuando llamas a un
endpoint de confianza que responde `http → https` o te entrega una
URL de CDN.

Cuando la URL de la solicitud está influida por una entrada no
confiable, ese valor por defecto se convierte en un vector de
server-side request forgery (SSRF): un endpoint hostil puede
responder con un `3xx` cuya `Location` apunte a un servicio interno o
a una dirección de metadatos de la nube
(`http://169.254.169.254/…`), y un cliente que siga redirecciones
terminaría yendo tras ella. Desactiva el seguimiento de redirecciones
para esas solicitudes con `.no_redirects()`:

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// El 3xx se devuelve tal cual en lugar de seguirse - inspecciónalo y
// rechaza en lugar de dejar que el cliente vaya tras el encabezado Location.
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` encamina la solicitud a través de un cliente
separado que no sigue redirecciones; el cliente por defecto - y toda
solicitud que no lo llame - se queda sin cambios. Este es el
equivalente para el cliente general del bloqueo de redirecciones que
el remitente de web push ya aplica a los endpoints de push
controlados por el atacante.

## Reintentos

`Http` incluye reintentos con backoff exponencial y jitter completo -
la receta de AWS, la misma que usa Laravel. Dos variantes,
distinguidas por si están dispuestas a repetir métodos no
idempotentes.

### `.retry(max_attempts, base_backoff)` - solo idempotentes

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` incluye el primer intento, así que `retry(4, ...)`
reintenta hasta tres veces más después del intento inicial. El
retardo antes del intento `n+1` es una duración aleatoria uniforme en
`[0, base_backoff * 2^(n-1)]`, topada en 30 segundos. Jitter
completo, no backoff-exponencial-más-espera-fija, así que muchos
workers reintentando ante la misma caída no se sincronizan en una
estampida.

Una solicitud se reintenta cuando:

- El envío falla antes de que llegue una respuesta (conexión / DNS /
  timeout), o
- El estado de la respuesta es 5xx

Las respuestas 4xx y 2xx/3xx se devuelven tal cual. Al agotar los
reintentos, se devuelve a quien llama la última respuesta (o el
último error).

La forma `.retry()` se niega a reintentar `POST` o `PATCH`: esos
métodos no son idempotentes, y si el servidor ya confirmó la
escritura pero la respuesta se perdió en el camino de vuelta, una
repetición ciega duplicaría el efecto secundario. Llamar a `.retry()`
sobre un POST/PATCH sigue funcionando - solo significa "reintenta
ante errores de conexión antes de que la solicitud llegue al
servidor"; en cuanto vuelve un 5xx, se devuelve a quien llama tras un
solo intento.

### `.retry_non_idempotent(...)` - opt-in para POST/PATCH

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

Cuando has suministrado una clave de idempotencia que el upstream
respeta, o has hecho segura la repetición de la solicitud de otra
forma, cambia a `.retry_non_idempotent(...)` para meter a POST y
PATCH en el mismo comportamiento de reintento. Las reglas de
reintento son idénticas - los errores de conexión y las respuestas
5xx se reintentan; 4xx y 2xx/3xx pasan directo.

### Se respeta `Retry-After` en un 503

Para un `503 Service Unavailable`, el framework respeta un
encabezado `Retry-After` - en su forma de delta en segundos
(`Retry-After: 30`) o de fecha HTTP (`Retry-After: Tue, 15 Nov 1994
08:12:31 GMT`). La espera real es la mayor entre el backoff con
jitter y la pista de `Retry-After`, siempre topada en 30 segundos. Un
servidor hostil o mal configurado que devuelva `Retry-After: 86400`
no aparcará tu tarea durante un día.

## Leer la respuesta

`ClientResponse` expone el estado, los encabezados, y tres métodos de
lectura del cuerpo. Cada método de cuerpo consume la respuesta.

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// Elige uno - cada uno consume la respuesta.
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` no distingue mayúsculas de minúsculas. `.json::<T>()`
devuelve `Result<T, FrameworkError>` y usa `serde_json` para
decodificar. `.text()` exige UTF-8 y expone un `FrameworkError` si el
cuerpo no es UTF-8 válido.

### Tope del cuerpo de la respuesta

Un upstream lento u hostil podría, de otro modo, transmitir un cuerpo
sin límite hacia la memoria. Para protegerte de eso, cada lectura de
cuerpo en búfer está topada - 25 MiB por defecto. Anúlalo de forma
global en el arranque:

```rust
use suprnova::Http;

// Una vez, en algún lugar del bootstrap.
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

O por solicitud, cuando una sola llamada legítimamente maneja un
payload más grande:

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

Una respuesta que declara un `Content-Length` por encima del tope se
rechaza antes de leer nada del cuerpo; el bucle de streaming también
aplica el tope contra los bytes reales, por si `Content-Length` está
ausente o miente.

## Vía de escape - reqwest en crudo

El framework cubre los casos comunes. Cuando necesitas algo que no
exponemos - cuerpos en streaming, subidas multipart, inspección de la
política de redirección, upgrades de websocket - llama a
`.into_inner()` para desenvolver el `reqwest::Response` subyacente:

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` devuelve `Err(FrameworkError::internal(...))` cuando se
llama sobre una respuesta fake - no hay ningún `reqwest::Response`
subyacente en ese caso. El tope del cuerpo de la respuesta tampoco
se aplica ya en cuanto tomas la respuesta en crudo; el resto de la
lectura es cosa tuya.

Para las subidas multipart salientes, por ahora baja hasta
`reqwest::Client` directamente por la misma vía de escape. Una
versión futura podría añadir un builder `.multipart(...)` cuando el
patrón de demanda se defina por sí solo.

## Pruebas con `Http::fake`

Esta es la parte que usarás cada día. `Http::fake` ejecuta el cuerpo
de tu test dentro de un alcance `tokio::task_local!` donde cada
llamada saliente se intercepta, se captura, y se responde con lo que
hayas encolado.

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### Hacer coincidir respuestas prefabricadas

`fake_response(method, url_substring, status, body)` encola una
respuesta prefabricada. La primera solicitud saliente cuyo método
coincide (sin distinguir mayúsculas de minúsculas) y cuya URL
contiene `url_substring` consume la entrada prefabricada y devuelve
esa respuesta. Usa el método `"*"` para coincidir con cualquier
método.

Las solicitudes coincidentes posteriores caen a la siguiente entrada
prefabricada de la misma forma, o - si ninguna coincide - devuelven
un `200 {}` vacío. Encola una respuesta prefabricada por cada llamada
esperada:

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// Dos GET a /v1/customer obtienen respuestas distintas; un tercero obtiene 200 {}.
```

### Aserciones

```rust
// Pasa si al menos una solicitud registrada coincide.
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// Pasa si ninguna solicitud registrada coincide.
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` expone `method: String`, `url: String`, `headers:
Vec<(String, String)>`, y `body: Option<Vec<u8>>`. El predicado se
ejecuta contra cada solicitud registrada; los fallos de aserción
imprimen la lista registrada con los valores de encabezado y los
cuerpos redactados (una pequeña lista de permitidos con
`Content-Type`, `Accept`, y `User-Agent` se muestra completa; todo lo
demás es `<redacted>`). Eso mantiene los bearer tokens y los payloads
de webhook fuera de los registros de CI incluso cuando una aserción
falla estrepitosamente.

### Los tests se ejecutan en paralelo de forma segura

El estado del fake vive en un `tokio::task_local!` - cada alcance de
fake está acotado a la tarea que ejecuta el test, no al proceso. Dos
tests que se ejecutan de forma concurrente en tareas distintas
obtienen cada uno su propio vec de solicitudes registradas y su
propia cola de respuestas prefabricadas. Sin mutex compartido, sin
orden de test, sin `#[serial]`.

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // La solicitud a /b del test hermano es invisible aquí.
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## La trampa de la tarea lanzada

`tokio::task_local!` está acotado a la tarea actual. El trabajo que
pasa por `tokio::spawn` cae en una tarea nueva y NO hereda el fake -
por defecto, las llamadas salientes desde el future lanzado llegan a
la red real. Dos ayudantes lo resuelven.

### `Http::fail_on_real_calls()` y `FailOnRealCallsGuard`

Activa un flag global para el proceso que convierte cualquier
llamada saliente sin coincidencia en un `FrameworkError::internal(...)`
en lugar de dejarla llegar a la red. Este es el equivalente en
Suprnova de `Http::preventStrayRequests()` de Laravel - atrapa
exactamente el bug que crea esta trampa.

Usa la guarda RAII para que el flag se reinicie cuando el test
termine, incluso ante un pánico:

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // Cualquier llamada HTTP saliente sin fake desde cualquier lugar
    // dentro de este test - incluida una tarea lanzada con
    // `tokio::spawn` - falla con un mensaje que nombra la URL. No
    // ocurre ninguna E/S de red real.
}
```

Las guardas anidadas se componen correctamente: el `Drop` de la
guarda interna restaura el estado ANTERIOR, no "permitido" sin
condiciones. Así que un ayudante de test interno que instala su
propia guarda dentro de un alcance guardado externo no desarma la
guarda externa al salir.

El flag es global para el proceso por diseño. El objetivo es atrapar
un future lanzado con `tokio::spawn` que escapa en silencio de un
alcance de fake y hace ping a un tercero real desde CI. Un flag por
tarea se perdería eso.

### `Http::spawn_with_fake_inheritance(future)`

Cuando el código bajo prueba lanza legítimamente una tarea - un
worker de cola, un sincronizador en segundo plano, una subtarea - y
quieres que sus llamadas salientes pasen por el fake del padre,
sustituye `tokio::spawn` por `Http::spawn_with_fake_inheritance`:

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // Se ejecuta en una tarea NUEVA, pero el estado de fake del
        // padre se vuelve a instalar en el alcance task-local de esta
        // tarea. El envío se intercepta; la respuesta es el 204 de
        // arriba.
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // Las solicitudes registradas desde el hijo aparecen aquí - el
    // Arc<Mutex<FakeState>> se comparte, no se toma una instantánea.
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

Si no hay ningún alcance de fake activo cuando llamas a
`spawn_with_fake_inheritance`, es equivalente a `tokio::spawn` - el
hijo se ejecuta sin ningún contexto de fake. Así que puedes usarlo
sin condiciones en código que a veces se prueba con `Http::fake` y a
veces no.

### Cinturón y tirantes en la configuración del test

Los dos se combinan. Un test que quiere ser estrepitosamente seguro
los empareja:

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // Si una errata en la URL o el método se desvía del fake, la
        // solicitud cae hasta la guarda, que falla con un mensaje que
        // nombra la URL - en lugar de devolver en silencio un 200
        // vacío que oculta el desajuste.
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

Sin la guarda, una URL o un método que se desvía del fake cae en
silencio a un `200 {}` por defecto, y tu test pasa a pesar de que el
código de producción llama a un endpoint distinto. Con la guarda,
fallas de forma estrepitosa ante el primer desajuste.

## Propagación de traza de OpenTelemetry

Cuando el framework se compila con la feature `otel` y hay instalado
un propagador de TraceContext de W3C, cada solicitud saliente de
`Http::*` inyecta `traceparent` (y `tracestate` cuando no está vacío)
en sus encabezados - para que los servicios corriente abajo puedan
continuar la traza. Sin ninguna configuración en el sitio de la
llamada; el propagador lee `opentelemetry::Context::current()` en el
momento del envío.

Sin un contexto de OTel activo, no se inyecta ningún encabezado y las
solicitudes salientes se ven exactamente como antes. Consulta
[Observabilidad](observability.md) para la configuración del
propagador.

## Por qué Suprnova diverge

Vale la pena señalar dos pequeñas divergencias respecto a la fachada
`Http::` de Laravel, ambas forzadas por el modelo de runtime.

**Fakes task-local en lugar de un almacén mock global para el
proceso.** `Http::fake()` de Laravel muta un registro global para
todo el proceso; los tests se serializan sobre él, o aceptas que los
runners en paralelo puedan competir entre sí. El `Http::fake` de
Suprnova usa `tokio::task_local!`, así que dos tests en dos tareas
ven cada uno su propio fake - sin orden de test, sin mutex
compartido. El precio es que el trabajo lanzado con `tokio::spawn` no
hereda el fake por defecto, que es por lo que existen
`Http::spawn_with_fake_inheritance` y `FailOnRealCallsGuard`. Juntos
te dan la misma garantía de "no puedes tocar producción por
accidente" que da `Http::preventStrayRequests()` en Laravel, con un
alcance más estricto.

**Los reintentos rechazan POST/PATCH por defecto.** El cliente HTTP
de Laravel reintenta cualquier método por defecto. El `.retry(...)`
de Suprnova es solo para idempotentes; los métodos no idempotentes
necesitan un opt-in explícito con `.retry_non_idempotent(...)`. El
razonamiento es que una respuesta 5xx desde un endpoint de escritura
con frecuencia significa "confirmé la escritura y luego se perdió la
respuesta" - repetirla a ciegas duplica un cargo, un reembolso, una
dispersión. Obligamos a quien llama a decidir: ¿has suministrado una
clave de idempotencia que el upstream respeta? Si sí, mete POST/PATCH
en los reintentos. Si no, acepta el 5xx.

## Casos límite y letra pequeña

- **`Http::*` está cerrado para la v1.** Deliberadamente no exponemos
  el `reqwest::Client` subyacente. Para hacer crecer la superficie,
  añade un método a la fachada en lugar de recurrir directamente a
  `reqwest` - salvo vía la vía de escape documentada `into_inner()`
  sobre una respuesta real.
- **El cliente compartido se construye una vez y vive para siempre.**
  Se construye de forma perezosa en la primera llamada a cualquier
  verbo `Http::*`, y se guarda en un `OnceLock`. La pila de TLS de
  rustls y el timeout por defecto de 30 s quedan horneados dentro.
- **Los fallos de serialización de JSON/formulario fallan de forma
  estrepitosa.** Un builder `.json(&unserializable)` registra el
  error y `send()` lo devuelve como `FrameworkError::internal(...)`.
  La solicitud nunca sale - no degradamos a un cuerpo `null`.
- **El tope de 30 s en los reintentos es duro.** La aritmética del
  backoff topa en 30 segundos; la interpretación de `Retry-After`
  topa en 30 segundos; ningún sueño de reintento individual aparca
  una tarea por más tiempo.
- **El tope global para el proceso es de una sola vez.**
  `Http::set_max_response_bytes` es una escritura sobre un atómico
  global para el proceso - establécelo una vez en el arranque, y
  luego anúlalo por solicitud según lo necesites. No hay ninguna
  llamada de "restablecer al valor por defecto".

## Siguiente

- [Correo](mail.md) - correo saliente, que usa patrones de fake /
  driver similares para los tests
- [Notificaciones](notifications.md) - canales de notificación
  incluido web push, todos comparten la misma filosofía de test-fake
- [Cola](queues.md) - jobs que hacen llamadas HTTP salientes, más el
  patrón `spawn_with_fake_inheritance` para probar workers
- [Pruebas](testing.md) - `#[suprnova_test]`, `TestContainer`, y el
  resto de la superficie de fakes
- [Observabilidad](observability.md) - la configuración del
  propagador de OTel que hace que se active la inyección de
  `traceparent`
