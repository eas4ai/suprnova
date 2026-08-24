# Correo

El subsistema de correo de Suprnova refleja la API `Mail::to(...)->send(...)` de Laravel sobre Tokio. Una fachada `Mail`, nueve transportes (log, en memoria y previsualizaciones de archivos `.eml` para desarrollo/pruebas, SMTP y cinco proveedores HTTP: Postmark, SES, SendGrid, Mailgun y Resend), plantillas renderizadas con Tera usando como contexto los campos serializados del Mailable, entrega en cola y diferida sobre el sobre duradero de al menos una vez, y una guarda de pruebas `Mail::fake()` de la misma familia que `Bus::fake()` y `Cache::fake()`.

## Inicio rápido

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

El Mailable se serializa a JSON, que se convierte en el contexto de Tera para la plantilla; cada campo `pub` es accesible como `{{ field_name }}`.

## Configuración

`Server::serve` llama a `suprnova::mail::boot::bootstrap_from_env()` una vez al iniciar. Lee `MAIL_DRIVER` y vincula el transporte correspondiente. Si no se establece, usa el driver `log` por defecto.

| `MAIL_DRIVER` | Comportamiento |
|---------------|----------|
| `log`         | Emite un `tracing::info!` por envío  - sobre y cuerpos completos, como Laravel -  y descarta el mensaje. Es el valor por defecto fuera de producción. |
| `memory`      | Captura todos los mensajes en el proceso. Consulta `suprnova::mail::boot::captured_in_memory()`. |
| `file`        | Escribe un `.eml` RFC 5322 por envío en `MAIL_FILE_PATH` (por defecto, `storage/mail`) y después descarta el mensaje. Abre el archivo en un cliente de correo para comprobar renderizado, encabezados y adjuntos. |
| `smtp`        | Se conecta a un servidor SMTP (STARTTLS si se establecen credenciales; TCP sin cifrar en caso contrario). |
| `postmark`    | Envía JSON mediante POST al endpoint `/email` de Postmark. |
| `ses`         | Envía solicitudes firmadas con SigV4 mediante POST a `SendEmail` de Amazon SES. |
| `sendgrid`    | Envía JSON mediante POST a `/v3/mail/send` de SendGrid. |
| `mailgun`     | Envía `application/x-www-form-urlencoded` mediante POST (o `multipart/form-data` cuando hay adjuntos) a `/v3/{domain}/messages` de Mailgun. |
| `resend`      | Envía JSON mediante POST a `/emails` de Resend. |

### Producción falla de forma segura con un driver que descarta correo

`log`, `memory` y `file` renderizan un mensaje y lo descartan. Con `APP_ENV=production`, el arranque **se niega** a iniciar con cualquiera de ellos, e igualmente con un `MAIL_DRIVER` sin establecer o un valor que el build no reconoce, porque ambos terminan en el mismo transporte `log`:

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

El fallo que esto evita es silencioso: con el valor por defecto anterior, un despliegue que olvidaba `MAIL_DRIVER`  - o escribía `MAIL_DRIVER=SMTP` con la capitalización equivocada -  informaba que cada restablecimiento de contraseña se había enviado aunque nada saliera del proceso, y nadie lo descubría hasta que un usuario quedaba bloqueado.

Si un despliegue de producción realmente no quiere correo saliente (una réplica de solo lectura o un lanzamiento oscuro), reconócelo explícitamente:

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

Solo `1`, `true`, `yes` u `on` cuentan como consentimiento: `=false` o un error tipográfico deja la guarda activa. Con la anulación establecida, cada arranque advierte que el correo saliente no se entregará.

Nada cambia fuera de producción: `local`, `development`, `testing` y `staging` conservan el valor por defecto `log` y el comportamiento de advertir y volver al valor por defecto para drivers desconocidos.

### Producción falla de forma segura con una conexión SMTP sin cifrar

La misma regla se aplica a cómo se protege la conexión, en lugar de a si entrega. `MAIL_DRIVER=smtp` en producción debe resolverse en un transporte cifrado o el arranque falla.

`MAIL_SMTP_ENCRYPTION` acepta `starttls`, `tls` o `none` (`ssl` y `null` se aceptan como alias compatibles con Laravel). Si no se establece, se deriva de las credenciales:

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | Se resuelve en | Motivo |
|---|---|---|
| ambos establecidos | `starttls` | Las credenciales implican un relé real en el puerto de envío. |
| ninguno establecido | `none` | La ruta de captador local. Mailpit, MailHog y maildev escuchan sin autenticación en 1025 y no usan TLS. |

Así, un scaffold nuevo sigue funcionando sin configuración, y un despliegue de producción que nunca conectó las credenciales se detiene en vez de enviar silenciosamente en texto claro. Establece `MAIL_SMTP_ENCRYPTION=tls` para un relé que espera TLS implícito en 465, un modo que el transporte siempre admitió, pero al que antes ninguna combinación de variables de entorno podía llegar.

Un valor no reconocido hace fallar el arranque en *todos* los entornos, no solo en producción. `MAIL_SMTP_ENCRYPTION=tsl` es una transposición de un modo que cifra, por lo que tratarlo silenciosamente como «sin cifrado» sería exactamente el fallo que la variable existe para prevenir; es mejor fallar en la máquina del desarrollador que en el despliegue.

La vía de escape refleja la anterior:

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

Solo es defendible cuando el relé es alcanzable exclusivamente por una red privada  - un sidecar o un Postfix dentro de la VPC - . En cualquier otro caso, SMTP en texto claro pone las credenciales y cada enlace de restablecimiento de contraseña en la red, donde permanecen para cualquiera que escuche en el trayecto.

### El driver `log` registra el mensaje completo

Igual que el mailer `log` de Laravel: sobre *y* cuerpos renderizados.

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

Ese enlace es el punto. En desarrollo, la consola es donde lees el enlace de verificación o restablecimiento de contraseña que la aplicación acaba de «enviar», y un driver que lo oculta es un driver que nadie puede usar.

Es seguro aquí porque el driver no puede llegar a producción: el arranque se niega a iniciar con `MAIL_DRIVER=log` bajo `APP_ENV=production` (consulta arriba). Los cuerpos solo existen en la máquina de un desarrollador.

Si estableces `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` para ejecutar el driver `log` en un entorno desplegado, eliges colocar enlaces de portador de un solo uso en tus registros. Cualquiera que pueda leer esos archivos  - operadores, el agente de envío de registros, el bucket de retención o el agregador -  puede usarlos, y la caducidad del enlace no ayuda porque el envío de registros es más rápido que una persona leyendo su bandeja de entrada. Dimensiona para ello tu política de retención y acceso, o usa un driver que no imprima:

```env
# In-process capture - suprnova::mail::boot::captured_in_memory(), or Mail::fake() in tests
MAIL_DRIVER=memory

# Or write one .eml per send instead of a log line - see "Previewing mail as
# .eml files" below for the access-control trade this makes
MAIL_DRIVER=file
MAIL_FILE_PATH=storage/mail

# Or a local catcher (mailpit / maildev / mailhog), which renders the real mail in a UI
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### Entorno por driver

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # or `tls` for implicit TLS on 465, or `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

Cada proveedor HTTP también respeta una anulación `MAIL_<PROVIDER>_ENDPOINT` correspondiente que apunta a una URL regional o a un servidor simulado (útil para pruebas de integración con `wiremock`).

### Remitente de los flujos de autenticación: `MAIL_FROM` y `MAIL_FROM_NAME`

Los mailables incorporados de los flujos de autenticación  - verificación de correo, restablecimiento de contraseña y la notificación de contraseña cambiada -  resuelven el `From` de su sobre desde el entorno, en vez de mediante un `from()` codificado de forma fija:

```env
MAIL_FROM=no-reply@example.com        # bare address (required by the auth flows; fails closed if unset)
MAIL_FROM_NAME=Acme Support           # optional display name (since 0.5.9)
```

- `MAIL_FROM` **debe ser una dirección sin nombre visible.** Se eleva directamente al `From` del mensaje, por lo que un valor `"Name <addr>"` se trataría como la dirección completa y el transporte lo rechazaría.
- `MAIL_FROM_NAME` (opcional, añadido en **0.5.9**) adjunta un nombre visible, de modo que el encabezado se renderiza como `Acme Support <no-reply@example.com>`. Si no se establece o está en blanco, se conserva el comportamiento anterior de dirección sin nombre. Se lee al momento de enviar, por lo que también se aplica al correo de flujos de autenticación en cola.

Estas dos variables solo afectan a los mailables de los flujos de autenticación del framework. Tus propios `Mailable` establecen el remitente mediante `from()` (o el valor global `always_from`); consulta abajo.

## Previsualizar correo como archivos `.eml`

`MAIL_DRIVER=log` coloca los cuerpos renderizados en tu consola, lo que funciona para un mensaje de texto plano y mal para cualquier otra cosa. El driver `file` escribe los bytes que SMTP habría puesto en la red:

```
MAIL_DRIVER=file
MAIL_FILE_PATH=storage/mail
```

Cada envío produce un `<millis>-<seq>.eml` en ese directorio. Ábrelo con cualquier cliente de correo (Thunderbird, Apple Mail, `mutt -f`) para ver el mensaje como lo ve un destinatario: ambos cuerpos alternativos, cada adjunto y el conjunto completo de encabezados, incluidos `X-Priority`, `X-Tag`, `X-Metadata-*` y `Return-Path`.

El directorio se crea con el primer envío. Si no se establece `MAIL_FILE_PATH`, el correo llega a `storage_path("mail")`, la misma familia de rutas que usa cualquier otro consumidor de `storage/`, por lo que el directorio permanece dentro de la base de la aplicación incluso si un administrador de servicios inicia el proceso desde otro lugar. Un `MAIL_FILE_PATH` absoluto se usa tal cual; uno relativo se ancla al directorio base de la aplicación (`base_path`, anulable mediante `APP_BASE_PATH`).

### Por qué Suprnova diverge

Laravel no tiene un mailer de archivos; su mailer `log` escribe el MIME sin procesar en el canal de registros, lo que implica buscar en un archivo de registro un límite MIME para reconstruir un adjunto. Escribir un `.eml` real por mensaje hace que el artefacto se pueda abrir en vez de reconstruirlo. La contrapartida es que el correo se acumula en el disco: este driver nunca elimina nada, así que trata `MAIL_FILE_PATH` como espacio temporal.

### Cada archivo `.eml` es una credencial funcional y ninguno caduca por sí solo

Los correos de restablecimiento de contraseña y verificación de correo contienen enlaces de portador de un solo uso, y el driver `file` los escribe exactamente como SMTP los habría enviado, legibles por cualquiera que pueda abrir el archivo. A diferencia del flujo del driver `log`, este es almacenamiento duradero: nada elimina `MAIL_FILE_PATH`, por lo que un token escrito el primer día sigue allí y sigue siendo válido hasta que caduque, incluso el día cien. Da al directorio el mismo tratamiento de acceso que a un archivo de registro con enlaces de restablecimiento: mantenlo fuera del control de versiones, restringe quién puede leer el sistema de archivos del despliegue y límpialo según una programación si `file` funciona cerca de tráfico real.

## El trait Mailable

Los mailables son structs serializables que saben renderizarse a sí mismos. Los valores por defecto del trait renderizan con `tera::Tera::one_off` frente a los campos serializados del mailable:

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| Método | ¿Obligatorio? | Propósito |
|--------|-----------|---------|
| `mailable_name()` | sí | Nombre estable que se persiste en el sobre de la cola; renombrarlo rompe el correo en cola que está en vuelo. |
| `subject(&self)` | sí | Asunto calculado. Se usa literalmente cuando `subject_template_source` devuelve `None`. |
| `subject_template_source(&self)` | opcional | Plantilla Tera para el asunto; cuando es `Some`, tiene precedencia sobre `subject()` y se renderiza con `self` como contexto. Tiene la misma semántica que las fuentes de plantillas de cuerpo. |
| `html_template_source(&self)` | opcional | Plantilla Tera para el cuerpo HTML. Devuelve `None` para omitir HTML. |
| `text_template_source(&self)` | opcional | Plantilla Tera para el cuerpo de texto plano. Devuelve `None` para omitir texto. |
| `from(&self)` | opcional | Anula el valor por defecto global `noreply@localhost`. |
| `attachments(&self)` | opcional | Archivos que adjuntar. Cada uno es `name + bytes + mime`. |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | opcional | Anula estos métodos si quieres evitar Tera (Markdown → HTML, contenido prerenderizado, lógica de asunto personalizada, etc.). |

Al menos una de `html_template_source` o `text_template_source` debe devolver `Some` (o `render_html`/`render_text` deben producir contenido). Un mailable de cuerpo vacío se rechaza tanto al despachar (`Mail::send`) como al encolar (`Mail::queue`).

### Autoescape de Tera

El autoescape está **DESACTIVADO** porque los cuerpos de correo suelen ser HTML escrito a mano, donde el escapado `<>&` de Tera escaparía en exceso. Si tu cuerpo literal contiene `{{` por motivos ajenos a la plantilla (por ejemplo, texto de marketing que cita sintaxis Mustache), escápalo: `{% raw %}{{ literal }}{% endraw %}`.

## Construir mensajes

El builder de `Mail::to(...)` incorpora destinatarios, CC/BCC, respuesta a y una anulación del remitente por mensaje al despacho:

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // (display name, email)
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` acepta `&str`, `String` y tuplas `(name, email)`; `Mail::to(...)` acepta cualquier cosa que implemente `Into<Address>`.

## Adjuntos

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

Los adjuntos viajan mediante el método `Mailable::attachments`. Los cinco proveedores HTTP los manejan: Postmark/SendGrid/Resend mediante JSON (codificado en base64), SES mediante MIME sin procesar (pues `Content.Simple` no admite adjuntos) y Mailgun mediante `multipart/form-data` (la ruta codificada como formulario se usa cuando no hay adjuntos).

## Encolar

`Mail::queue(...)` construye un `SendMailJob` y lo inserta en la cola del framework. El worker reconstruye el mailable desde la fábrica registrada y lo despacha mediante el transporte vinculado:

```rust
// One-time: register every Mailable type the worker will see.
suprnova::mail::register_mailable_factory::<Welcome>()?;

// At send time:
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// Delayed:
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

Enruta un despacho en cola a una cola o conexión específica con `.on_queue(...)` / `.on_connection(...)`, o asigna al propio `Mailable` un valor por defecto mediante `Mailable::queue(&self)`:

```rust
Mail::to("alice@example.org")
    .on_queue("emails")
    .queue(Welcome { name: "Alice".into() })
    .await?;
```

`.on_queue(...)` tiene precedencia tanto sobre `Mailable::queue()` como sobre cualquier `Queue::route` registrada para el trabajo de despacho de correo: la misma regla de que «la anulación por inserción gana» que `Queue::push_with` aplica en todas partes. Consulta [Queues](queues.md#queue-routing).

La misma guarda de cuerpo vacío se ejecuta en la ruta de la cola, por lo que un Mailable mal configurado se rechaza al insertar, antes de crear un sobre.

## Telemetría

Cada envío pasa por `suprnova::mail::dispatch_with_telemetry`, que abre un `tracing::info_span!` llamado `mail.send` que incluye:

- `transport`: nombre del driver (`"postmark"`, `"smtp"`, `"in-memory"`, …)
- `to_count`, `cc_count`, `bcc_count`: número de destinatarios
- `has_html`, `has_text`: forma del cuerpo
- `attachment_count`: número de adjuntos
- `tag_count`, `metadata_count`: número de indicaciones para el proveedor
- `priority`: `1..=5`, o `0` cuando no se establece

Al completarse, el span emite `mail sent` (info) o `mail send failed` (warn) con `duration_ms`. El mismo envoltorio cubre `Mail::send`, el worker de cola `SendMailJob` y el canal de notificaciones `MailChannel`, por lo que el esquema del span es idéntico sin importar cómo se produjo el mensaje.

## Pruebas con `Mail::fake()`

`Mail::fake()` instala un transporte de captura en memoria durante la vida de la guarda RAII devuelta. Refleja `Bus::fake()` / `Queue::fake()` / `Cache::fake()`:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));
}
```

Cuando la guarda se descarta, se restaura el transporte previamente vinculado (si lo hubiera). Las pruebas que mezclan `Mail::fake()` con vinculación explícita de transportes no filtran estado.

`Mail::fake()` es `Send + Sync`; compártela entre awaits o threads según necesites.

## Transportes personalizados

El trait `MailTransport` es el punto de integración:

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// At boot:
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

Los transportes se ejecutan en el runtime de Tokio: la E/S asíncrona, el agrupamiento de conexiones y el envío concurrente son funcionalidades de primer nivel. No hay penalización por bifurcación por solicitud.

### Por qué Suprnova diverge

La capa Mailable de Laravel está construida sobre Symfony Mailer, que se ejecuta de forma síncrona dentro del ciclo de vida de la solicitud. `MailTransport` de Suprnova es `async fn send(&self, msg: &OutgoingMessage)` de extremo a extremo: los proveedores HTTP usan `reqwest`, la ruta SMTP usa un adaptador async de lettre y `dispatch_with_telemetry` envuelve cada envío en un span de Tokio `tracing`. Los proveedores de larga distancia no bloquean el hilo del handler, los pools de conexión sobreviven entre solicitudes y los envíos concurrentes en un handler son triviales: `tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))` hace lo que esperas.

La otra divergencia es la cancelación de eventos. Laravel modela un oyente `MessageSending` que puede devolver `false` y suprimir el envío (`events->until()`). El despachador de Suprnova no expone un canal de retorno de cortocircuito: `MessageSending` es solo de observación. Para bloquear un envío, recházalo en la capa Mailable (anula `render_html` / `render_text` para devolver un error) o envuelve la llamada a `MailBuilder::send` con tu propia guarda. La contrapartida es real: perdemos un hook de Laravel para mantener simple el contrato del despachador.

Una divergencia menor es un endurecimiento deliberado. Laravel permite que `MAIL_MAILER=log` siga funcionando en producción; Suprnova se niega a arrancar allí sin un reconocimiento explícito, porque un subsistema de correo que informa éxito y no entrega nada es el tipo de interrupción que nadie nota durante semanas. El propio driver `log` se comporta exactamente como el de Laravel  - mensaje completo, incluidos cuerpos y enlaces - , lo que lo hace útil en desarrollo, y el rechazo en producción es lo que lo mantiene seguro (consulta [El driver `log` registra el mensaje completo](#the-log-driver-logs-the-whole-message)).

## Buenas prácticas

### Registra las fábricas al arrancar, no por solicitud

`Mail::queue` y `Mail::later` insertan un `SendMailJob` que contiene el nombre del mailable y la carga útil JSON; el worker reconstruye el tipo concreto mediante `mailable_registry`. Registra una vez cada `Mailable` que pueda encolarse, cuando se ejecute `Server::serve`:

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

Un `Mail::queue` para un mailable no registrado llega a la cola, se ejecuta una vez, encuentra «unknown mailable», reintenta según la política de retroceso del sobre y termina en la cola de mensajes no entregables, con el coste de tiempo de observabilidad que no habrías gastado si la fábrica se hubiera vinculado al arrancar.

### Encola el correo para cualquier renderizado lento o poco fiable

Enviar correo en un handler de solicitud acopla la latencia de respuesta del usuario a tu servidor SMTP (o a la API HTTP de cualquier proveedor). Usa `Mail::queue` para cualquier cosa que supere un renderizado local síncrono de desarrollo, y `Mail::later` cuando quieras diferir el despacho: seguimientos de incorporación, correos de recordatorio o resúmenes programados.

```rust
// Bad: ties response time to the mail provider
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// Good: 200 OK returns immediately; the worker delivers the mail.
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Establece siempre `from` en un Mailable

El remitente predeterminado del framework es `noreply@localhost`: útil para detectar remitentes ausentes en desarrollo, pero no es un remitente que ningún proveedor acepte en producción. Anula `Mailable::from(&self)` (o establece `from = "..."` en el atributo `#[mail(...)]` de un `NotificationMailable`) para que cada mensaje despachado tenga una identidad de remitente real:

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

La anulación por mensaje en `MailBuilder` (`.from(("Operations", "ops@example.com"))`) tiene precedencia sobre el valor por defecto del mailable; es útil para envíos transaccionales puntuales.

### Usa la cola para entrega de al menos una vez, no la ruta directa

`MailBuilder::send` es como máximo una vez: si el transporte falla a mitad
de un despacho a dos proveedores, no puedes reintentar sin arriesgar un envío
duplicado. `MailBuilder::queue` usa una entrega duradera de al menos una vez
y expone el enrutamiento de cola y conexión, pero no acepta una clave de
idempotencia. Un job de correo redeliverado puede enviar dos veces. Si un
mensaje debe deduplicarse, coloca una guarda de idempotencia a nivel de
aplicación o un mecanismo de deduplicación compatible con el proveedor en un
job encolado personalizado, en lugar de afirmar que `MailBuilder` acepta una
clave.

## Mensajes puntuales: `Mail::raw` y `Mail::html`

Cuando el correo es un único mensaje transaccional que no justifica un struct `Mailable` completo, dos atajos omiten el código repetitivo:

```rust
use suprnova::mail::Mail;

// Plain text
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("Hi")
        .from("hello@example.com")
}).await?;
```

El cierre recibe un [`MailBuilder`] con el cuerpo precargado y te permite añadir destinatarios, asunto, remitente, tags, metadata, prioridad y cualquier otro método fluido de [`MailBuilder`]. Estas rutas omiten por completo el trait `Mailable`; son útiles para pings de prueba de una sola vez y notas transaccionales breves.

## Valores por defecto globales: `always_from`, `always_reply_to`, `always_to`, `always_return_path`

Como `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo` / `alwaysReturnPath` de Laravel, la fachada Mail expone cuatro setters globales:

```rust
use suprnova::mail::{Address, Mail};

// At boot:
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// Local-dev "single inbox" - route ALL mail to one address, drop CC/BCC:
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// Roll everything back (tests typically call this at teardown):
Mail::forget_always()?;
```

La precedencia es conservadora: los valores por defecto solo se aplican cuando el mensaje despachado carece de un valor explícito:

| Campo | El valor por defecto se aplica cuando |
|-------|---------------------|
| `always_from` | El `from` del mensaje es el valor por defecto del framework `noreply@localhost` |
| `always_reply_to` | El mensaje no tiene un `reply_to` explícito |
| `always_to` | Siempre; enruta cada mensaje a esta dirección y elimina CC/BCC |
| `always_return_path` | El mensaje no tiene un `return_path` explícito |

La misma precedencia se aplica en la ruta de cola: los mailables en cola pasan por `apply_always_defaults` al despacharse en el worker, por lo que los envíos directos y los en cola convergen en formas de sobre idénticas.

## Tags, metadata, prioridad, encabezados y ruta de devolución

Cada mensaje despachado puede llevar indicaciones para proveedores al estilo Laravel: tags, pares clave/valor de metadata, prioridad RFC-2076, encabezados MIME personalizados y una dirección Sender / de devolución. Se reenvían a los campos nativos de los proveedores HTTP (Postmark `Tag` / `Metadata` / `Headers`, SES `EmailTags` más `Content.Simple.Headers`, SendGrid `categories` / `custom_args` / `headers`, Mailgun `o:tag` / `v:` / `h:`, Resend `tags` / `headers`) y a SMTP como encabezados RFC 5322.

En SES específicamente, los encabezados se transmiten en la forma de contenido que use el mensaje: `Content.Simple.Headers` para un mensaje sencillo, líneas de encabezado MIME reales para un mensaje con adjuntos (que SES solo acepta como MIME sin procesar). Un nombre de encabezado se valida de la misma manera sin importar la forma final del mensaje: CR, LF y NUL se rechazan (así es como una cadena proporcionada por quien llama se convierte en un segundo encabezado), al igual que un nombre vacío, uno de más de 76 bytes, un byte no ASCII, o un `:` o espacio en el nombre, conforme a lo que exige el propio constructor de MIME sin procesar. Un nombre de encabezado repetido más de una vez conserva todos los valores en la ruta de mensaje sencillo, pero solo el último valor en la ruta con adjuntos, el mismo límite que tiene SMTP.

Hay dos maneras de adjuntarlos: en el nivel Mailable para valores por defecto por tipo, o por mensaje en el builder:

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// Per-message on the builder. Builder wins on metadata-key collisions; tags + headers union.
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

Las constantes de los cinco niveles de prioridad viven en `suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}`, la misma escala entera `1..=5` que usa Laravel.

### Opciones de envío de SES

El `SendEmail` v2 de Amazon SES toma tres opciones además del propio mensaje. Fíjalas en el transporte o anúlalas por mensaje mediante un encabezado:

```rust
use suprnova::mail::ses::SesMailTransport;

let transport = SesMailTransport::new(key, secret, "us-east-1")
    .tenant_name("acme")                                  // TenantName
    .configuration_set_name("transactional")              // ConfigurationSetName
    .list_management("newsletter", Some("weekly"));       // ListManagementOptions
```

| Encabezado del mensaje | Campo de SES | Forma |
|---|---|---|
| `X-SES-TENANT-NAME` | `TenantName` | el nombre del tenant |
| `X-SES-CONFIGURATION-SET` | `ConfigurationSetName` | el nombre del conjunto de configuración |
| `X-SES-LIST-MANAGEMENT-OPTIONS` | `ListManagementOptions` | `my-list`, `contactListName=my-list` o `my-list; topicName=weekly` |

Un encabezado siempre supera el valor por defecto del transporte, por lo que un transporte multiinquilino más un encabezado por mensaje cubre el caso habitual:

```rust
Mail::to(&user.email)
    .header("X-SES-TENANT-NAME", &tenant.slug)
    .send(WelcomeMail { name: user.name.clone() })
    .await?;
```

Estos encabezados son directivas de transporte, no contenido del mensaje: se consumen al construir la solicitud y nunca se renderizan en el MIME que llega al destinatario.

### Por qué Suprnova diverge

Laravel lee `X-SES-TENANT-NAME` y `X-SES-LIST-MANAGEMENT-OPTIONS` del mensaje, pero expone `ConfigurationSetName` solo mediante el array de opciones del transporte, por lo que cambiar los conjuntos de configuración por mensaje implica un segundo transporte. Suprnova da a los tres las mismas dos fuentes, añadiendo un encabezado `X-SES-CONFIGURATION-SET`. La precedencia de encabezado sobre transporte coincide con Laravel, donde las opciones derivadas del mensaje se fusionan sobre las configuradas.

## Inspeccionar mensajes capturados

`OutgoingMessage` incluye ayudantes de inspección al estilo Laravel, útiles tanto para aserciones de pruebas como para registro de auditoría en tiempo de ejecución:

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

Las comprobaciones de destinatario no distinguen mayúsculas y minúsculas en el correo electrónico; las comprobaciones de metadata, tag, asunto y nombre de archivo del adjunto son exactas.

## Fake de prueba: superficie ampliada

`Mail::fake()` cubre **AMBAS** rutas, la enviada y la en cola. El correo enviado (mediante `MailBuilder::send`) llega al transporte en memoria; el correo en cola (mediante `.queue` / `.later`) llega al búfer de cola del fake.

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // Sent-side
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // Queued-side (for delayed mails)
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // Composite
    fake.assert_outgoing_count(2);   // sent + queued
    fake.assert_not_outgoing("PasswordReset");
}
```

Ayudantes adicionales:

| Ayudante | Propósito |
|--------|---------|
| `fake.captured()` | Todos los mensajes enviados |
| `fake.count()` | Número de enviados |
| `fake.queued()` | Todos los `QueuedSnapshot` en cola |
| `fake.queued_count()` | Número en cola |
| `fake.outgoing_count()` | Enviados + en cola |
| `fake.sent(predicate)` | Filtra los enviados mediante predicado |
| `fake.sent_to(email)` | Filtra los enviados por destinatario |
| `fake.queued_named(name)` | Mailables en cola con un nombre dado |
| `fake.queued_to(email)` | Mailables en cola para el destinatario |
| `fake.assert_sent_count(n)` | Número exacto de enviados |
| `fake.assert_queued_count(n)` | Número exacto en cola |
| `fake.assert_outgoing_count(n)` | Total exacto |
| `fake.assert_nothing_sent()` | Búfer de enviados vacío |
| `fake.assert_nothing_queued()` | Búfer de cola vacío |
| `fake.assert_nothing_outgoing()` | Ambos vacíos |
| `fake.assert_sent_to(email)` | Al menos uno enviado al destinatario |
| `fake.assert_not_sent_to(email)` | Ninguno enviado al destinatario |
| `fake.assert_queued(name)` | Al menos uno en cola con ese nombre |
| `fake.assert_queued_with(name, fn)` | Al menos uno en cola con ese nombre que coincide con el predicado |
| `fake.assert_queued_to(email)` | Al menos uno en cola para el destinatario |
| `fake.assert_not_queued(name)` | Ninguno en cola con ese nombre |

`QueuedSnapshot::decode::<M>()` deserializa la carga útil de nuevo en el `M` concreto, por lo que los predicados comprobados por tipos funcionan sin código repetitivo de decodificación a medida.

## Eventos: `MessageSending` y `MessageSent`

Cada despacho correcto dispara dos eventos del framework:

- `MessageSending`: inmediatamente **ANTES** de la llamada al transporte. Los oyentes observan la forma del mensaje (destinatarios, asunto, tags y flags de forma del cuerpo).
- `MessageSent`: inmediatamente **DESPUÉS** de una llamada de transporte correcta. Los oyentes observan la misma forma; los envíos fallidos no emiten este evento.

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

Ambos eventos son solo de observación: el despachador no modela un canal de cancelación al estilo Laravel. Consulta [Por qué Suprnova diverge](#why-suprnova-diverges) arriba para la solución de control.

## Comodidad para varios destinatarios: `Mail::cc` y `Mail::bcc`

La fachada Mail expone tres puntos de entrada  - `to`, `cc` y `bcc` -  que devuelven un `MailBuilder` nuevo. Usa el que se ajuste a la intención de enrutamiento dominante:

```rust
// Start with a cc / bcc when the message is primarily an audit copy.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

La misma superficie fluida se aplica sin importar el punto de entrada con el que empieces.

### Prueba con `Mail::fake()`, no con el transporte vinculado

`Mail::fake()` instala un transporte de captura local al proceso durante la vida de la guarda RAII y restaura lo que estuviera vinculado antes. Las pruebas que lo usan no necesitan limpiar globales en cada entrada/salida: la semántica de descarte se encarga de ello. Combina `#[serial_test::serial]` con `Mail::fake()` para las pruebas que mutan el global del transporte; de otro modo, las pruebas concurrentes se interferirían entre sí.

## Siguiente

- [Notificaciones](notifications.md)  -  `Notify::send` se distribuye entre los canales de correo, base de datos y webpush; `#[derive(NotificationMailable)]` es el atajo basado en macros sobre el trait `Mailable`.
- [Colas](queues.md)  -  el sobre duradero sobre el que viajan `Mail::queue` y `Mail::later`.
- [Eventos](events.md)  -  escuchar `MessageSending` / `MessageSent` y el modelo más amplio del despachador.
- [Pruebas](testing.md)  -  `Mail::fake()` junto con las demás guardas `*::fake()`.
- [Configuración](configuration.md)  -  registro de configuración tipada para credenciales de servicio.

## Referencia

- Trait: `suprnova::mail::Mailable`
- Fachada: `suprnova::mail::Mail`
- Arranque: `suprnova::mail::boot::bootstrap_from_env()`
- Transportes: `LogMailTransport`, `InMemoryMailTransport`, `FileMailTransport`, `SmtpMailTransport`, `PostmarkMailTransport`, `SesMailTransport`, `SendGridMailTransport`, `MailgunMailTransport`, `ResendMailTransport`
- Job de cola: `suprnova::mail::SendMailJob`
- Guarda de prueba: `suprnova::mail::MailFake`
- Ayudante de telemetría: `suprnova::mail::dispatch_with_telemetry`
