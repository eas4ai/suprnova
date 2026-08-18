# Correo

El subsistema de correo de Suprnova refleja la API `Mail::to(...)->send(...)`
de Laravel sobre Tokio. Una fachada `Mail`, ocho transportes (log y en
memoria para dev/tests, SMTP, y cinco proveedores HTTP - Postmark, SES,
SendGrid, Mailgun, Resend), plantillas renderizadas con Tera usando los
campos serializados del Mailable como contexto, entrega en cola y
diferida que viaja sobre el sobre duradero de al menos una vez, y una
guarda de pruebas `Mail::fake()` cortada de la misma tela que
`Bus::fake()` y `Cache::fake()`.

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

El Mailable se serializa a JSON, que se convierte en el contexto de
Tera para la plantilla; cada campo `pub` es accesible como
`{{ field_name }}`.

## Configuración

`Server::serve` llama a `suprnova::mail::boot::bootstrap_from_env()`
una vez al iniciar. Lee `MAIL_DRIVER` y vincula el transporte
correspondiente. Por defecto usa el driver `log` cuando no está
establecido.

| `MAIL_DRIVER` | Comportamiento |
|---------------|----------|
| `log`         | Emite un `tracing::info!` por cada envío - sobre y cuerpos completos, tal como hace Laravel - y descarta. Por defecto fuera de producción. |
| `memory`      | Captura cada mensaje en proceso. Ver `suprnova::mail::boot::captured_in_memory()`. |
| `smtp`        | Se conecta a un servidor SMTP (STARTTLS cuando hay credenciales establecidas, TCP plano en caso contrario). |
| `postmark`    | Hace POST de JSON al endpoint `/email` de Postmark. |
| `ses`         | Hace POST de solicitudes firmadas con SigV4 a `SendEmail` de Amazon SES. |
| `sendgrid`    | Hace POST de JSON a `/v3/mail/send` de SendGrid. |
| `mailgun`     | Hace POST de `application/x-www-form-urlencoded` (o `multipart/form-data` cuando hay adjuntos presentes) a `/v3/{domain}/messages` de Mailgun. |
| `resend`      | Hace POST de JSON a `/emails` de Resend. |

### Producción falla en cerrado sobre un driver que descarta el correo

`log` y `memory` renderizan un mensaje y lo descartan. Bajo
`APP_ENV=production`, el arranque **se rehúsa** a iniciar sobre
cualquiera de los dos - e igualmente sobre un `MAIL_DRIVER` sin
establecer o un valor que el build no reconoce, porque ambos aterrizan
en ese mismo transporte `log`:

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

El fallo que esto previene es silencioso: con el valor por defecto
anterior, un despliegue que olvidó `MAIL_DRIVER` - o escribió
`MAIL_DRIVER=SMTP` con mayúsculas equivocadas - reportaba cada
restablecimiento de contraseña como enviado mientras nada salía
jamás del proceso, y nadie se enteraba hasta que un usuario quedaba
bloqueado fuera.

Si un despliegue de producción de verdad quiere no enviar correo
saliente (una réplica de solo lectura, un dark launch), reconócelo
explícitamente:

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

Solo `1`, `true`, `yes`, u `on` cuentan como consentimiento -
`=false` o un error de tipeo deja la salvaguarda activa. Con la
anulación establecida, cada arranque advierte que el correo saliente
no se entregará.

Nada cambia fuera de producción: `local`, `development`, `testing`, y
`staging` mantienen el valor por defecto `log` y mantienen el
comportamiento de advertir-y-recaer para drivers desconocidos.

### Producción falla en cerrado sobre una conexión SMTP sin cifrar

La misma regla, aplicada a cómo se protege la conexión en lugar de a
si entrega. `MAIL_DRIVER=smtp` en producción debe resolver a un
transporte cifrado, o el arranque falla.

`MAIL_SMTP_ENCRYPTION` acepta `starttls`, `tls`, o `none` (`ssl` y
`null` se aceptan como alias compatibles con Laravel). Sin
establecer, se deriva de las credenciales:

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | Resuelve a | Porque |
|---|---|---|
| ambas establecidas | `starttls` | Las credenciales implican un relay real en el puerto de envío. |
| ninguna establecida | `none` | La ruta del catcher local. Mailpit, MailHog y maildev escuchan sin autenticación en 1025 y no hablan TLS. |

Así que un proyecto recién generado con andamiaje sigue funcionando
sin ninguna configuración, y un despliegue de producción que nunca
conectó las credenciales se detiene en lugar de enviar en claro
silenciosamente.
Establece `MAIL_SMTP_ENCRYPTION=tls` para un relay que espera TLS
implícito en el puerto 465 - un modo que el transporte siempre
soportó, pero que ninguna combinación de variables de entorno podía
alcanzar antes.

Un valor no reconocido hace fallar el arranque en *todo* entorno, no
solo en producción. `MAIL_SMTP_ENCRYPTION=tsl` es una transposición
de un modo que cifra, así que tratarlo en silencio como "sin
cifrado" sería exactamente el fallo que la variable existe para
prevenir - mejor fallar en la máquina del desarrollador que en el
despliegue.

La vía de escape refleja la de arriba:

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

Solo es defendible cuando el relay solo es alcanzable a través de una
red privada - un sidecar, o un Postfix dentro de la VPC. En
cualquier otro caso, el SMTP en texto claro expone las credenciales y
cada enlace de restablecimiento de contraseña en la red, sin cifrar,
y ahí se quedan para quien esté escuchando en el camino.

### El driver `log` registra el mensaje completo

Igual que el mailer `log` de Laravel: sobre *y* cuerpos renderizados.

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

Ese enlace es el punto clave. En desarrollo la consola es donde lees
el enlace de verificación o de restablecimiento de contraseña que la
app acaba de "enviar", y un driver que lo esconde es un driver que
nadie puede usar.

Es seguro aquí porque el driver no puede alcanzar producción - el
arranque se rehúsa a iniciar con `MAIL_DRIVER=log` bajo
`APP_ENV=production` (ver arriba). Los cuerpos solo existen jamás en
la máquina de un desarrollador.

Si estableces `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` para
correr el driver `log` en un entorno desplegado, estás eligiendo
poner enlaces bearer de un solo uso en tus logs. Cualquiera que
pueda leer esos archivos - operadores, el log shipper, el bucket de
retención, el agregador - puede usarlos, y la expiración del enlace
no ayuda porque el envío de logs es más rápido que una persona
leyendo su bandeja de entrada. Dimensiona tu política de retención y
acceso para eso, o usa un driver que no imprima:

```env
# Captura en proceso - suprnova::mail::boot::captured_in_memory(), o Mail::fake() en tests
MAIL_DRIVER=memory

# O un catcher local (mailpit / maildev / mailhog), que renderiza el correo real en una UI
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
MAIL_SMTP_ENCRYPTION=starttls   # o `tls` para TLS implícito en el 465, o `none`

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

Cada proveedor HTTP también honra una anulación
`MAIL_<PROVIDER>_ENDPOINT` correspondiente que apunta a una URL
regional o a un servidor simulado (útil para tests de integración
contra `wiremock`).

### Remitente de los flujos de auth: `MAIL_FROM` y `MAIL_FROM_NAME`

Los mailables integrados de los flujos de auth - verificación de
correo, restablecimiento de contraseña, y el aviso de cambio de
contraseña - resuelven su `From` de sobre desde el entorno en lugar
de un `from()` fijado a mano:

```env
MAIL_FROM=no-reply@example.com        # dirección desnuda (obligatoria para los flujos de auth; falla en cerrado si no está establecida)
MAIL_FROM_NAME=Acme Support           # nombre visible opcional (desde la 0.5.9)
```

- `MAIL_FROM` **debe ser una dirección desnuda.** Se inserta
  directamente en el `From` del mensaje, así que un valor
  `"Nombre <dirección>"` se trataría como la dirección completa y el
  transporte lo rechazaría.
- `MAIL_FROM_NAME` (opcional, añadida en la **0.5.9**) adjunta un
  nombre visible, así que el encabezado se renderiza como
  `Acme Support <no-reply@example.com>`. Sin establecer o en blanco
  mantiene el comportamiento anterior de dirección desnuda. Se lee en
  el momento del envío, así que también aplica al correo en cola de
  los flujos de auth.

Estas dos variables solo afectan a los mailables propios del
framework para los flujos de auth. Tus propios `Mailable`s fijan su
remitente a través de `from()` (o el valor por defecto global
`always_from`) - ver abajo.

## El trait Mailable

Los mailables son structs serializables que saben renderizarse a sí
mismos. Los valores por defecto del trait renderizan con
`tera::Tera::one_off` contra los campos serializados del mailable:

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
| `mailable_name()` | sí | Nombre estable persistido en el sobre de la cola - renombrarlo rompe el correo en cola en vuelo. |
| `subject(&self)` | sí | Asunto calculado. Se usa literal cuando `subject_template_source` devuelve `None`. |
| `subject_template_source(&self)` | opcional | Plantilla Tera para el asunto - cuando es `Some`, tiene precedencia sobre `subject()` y renderiza con `self` como contexto. Misma semántica que las fuentes de plantilla del cuerpo. |
| `html_template_source(&self)` | opcional | Plantilla Tera del cuerpo HTML. Devuelve `None` para omitir el HTML. |
| `text_template_source(&self)` | opcional | Plantilla Tera del cuerpo en texto plano. Devuelve `None` para omitir el texto. |
| `from(&self)` | opcional | Anula el valor por defecto global `noreply@localhost`. |
| `attachments(&self)` | opcional | Archivos a adjuntar. Cada uno es `name + bytes + mime`. |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | opcional | Anúlalos si quieres evitar Tera (Markdown → HTML, contenido pre-renderizado, lógica de asunto personalizada, etc.). |

Al menos uno de `html_template_source` o `text_template_source` debe
devolver `Some` (o `render_html`/`render_text` deben producir
contenido). Un mailable con cuerpo vacío se rechaza tanto al
despachar (`Mail::send`) como al encolar (`Mail::queue`).

### Autoescape de Tera

El autoescape está **DESACTIVADO** porque los cuerpos de correo
suelen ser HTML escrito a mano, donde el escapado `<>&` de Tera
sobre-escaparía. Si tu cuerpo literal contiene `{{` por razones que
no son de plantilla (por ejemplo, texto de marketing que cita la
sintaxis de Mustache), escápalo: `{% raw %}{{ literal }}{% endraw %}`.

## Construir mensajes

El builder `Mail::to(...)` enhebra destinatarios, CC/BCC,
reply-to, y una anulación del remitente por mensaje dentro del
despacho:

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // (nombre visible, email)
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` acepta `&str`, `String`, y tuplas `(name, email)`;
`Mail::to(...)` acepta cualquier cosa `Into<Address>`.

## Adjuntos

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

Los adjuntos viajan a través del método `Mailable::attachments`.
Los cinco proveedores HTTP los manejan - Postmark/SendGrid/Resend
vía JSON (codificado en base64), SES vía MIME en crudo (ya que
`Content.Simple` no soporta adjuntos), y Mailgun vía
`multipart/form-data` (la ruta form-encoded se usa cuando no hay
adjuntos).

## Encolado

`Mail::queue(...)` construye un `SendMailJob` y lo empuja a la cola
del framework. El worker reconstruye el mailable desde la fábrica
registrada y despacha a través del transporte vinculado:

```rust
// Una sola vez: registra cada tipo de Mailable que el worker verá.
suprnova::mail::register_mailable_factory::<Welcome>()?;

// En el momento del envío:
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// Diferido:
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

El mismo guardián de cuerpo vacío corre en la ruta de la cola, así
que un Mailable mal configurado se rechaza en el momento de encolar,
antes de crear ningún sobre.

## Telemetría

Cada envío se enruta a través de
`suprnova::mail::dispatch_with_telemetry`, que abre un
`tracing::info_span!` de `mail.send` que lleva:

- `transport` - nombre del driver (`"postmark"`, `"smtp"`,
  `"in-memory"`, …)
- `to_count`, `cc_count`, `bcc_count` - conteos de destinatarios
- `has_html`, `has_text` - forma del cuerpo
- `attachment_count` - cantidad de adjuntos
- `tag_count`, `metadata_count` - conteos de pistas para el proveedor
- `priority` - `1..=5`, o `0` cuando no está establecida

Al completarse, el span emite `mail sent` (info) o `mail send
failed` (warn) con `duration_ms`. El mismo envoltorio cubre
`Mail::send`, el worker de cola de `SendMailJob`, y el
`MailChannel` de notificaciones, así que el esquema del span es
idéntico sin importar cómo se produjo el mensaje.

## Pruebas con `Mail::fake()`

`Mail::fake()` instala un transporte de captura en memoria durante
la vida de la guarda RAII devuelta. Refleja `Bus::fake()` /
`Queue::fake()` / `Cache::fake()`:

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

Cuando la guarda se descarta, se restaura el transporte previamente
vinculado (si había alguno). Los tests que intercalan `Mail::fake()`
con vinculación de transporte explícita no filtran estado.

`Mail::fake()` es `Send + Sync`; compártela entre awaits o hilos
según necesites.

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

Los transportes corren sobre el runtime de Tokio - IO asíncrona,
pooling de conexiones, y envío concurrente son de primera clase. No
hay penalización de fork por solicitud.

### Por qué Suprnova diverge

La capa Mailable de Laravel está construida sobre Symfony Mailer, que
corre de forma síncrona dentro del ciclo de vida de la solicitud. El
`MailTransport` de Suprnova es `async fn send(&self, msg:
&OutgoingMessage)` de punta a punta: los proveedores HTTP usan
`reqwest`, la ruta SMTP usa un adaptador lettre asíncrono, y
`dispatch_with_telemetry` envuelve cada envío en un span de
`tracing` de Tokio. Los proveedores de larga distancia no bloquean
el hilo del handler, los pools de conexión sobreviven entre
solicitudes, y los envíos concurrentes dentro de un mismo handler son
triviales - `tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))`
hace justo lo que esperarías.

La otra divergencia es la cancelación de eventos. Laravel modela un
oyente de `MessageSending` que puede devolver `false` y suprimir el
envío (`events->until()`). El despachador de Suprnova no expone un
canal de retorno de cortocircuito - `MessageSending` es solo de
observación. Para condicionar un envío, rechaza a nivel del Mailable
(anula `render_html` / `render_text` para devolver un error) o
envuelve la llamada a `MailBuilder::send` con tu propia salvaguarda.
El intercambio es real: perdemos un hook de Laravel para mantener
simple el contrato del despachador.

Otra divergencia menor es un endurecimiento deliberado. Laravel se
conforma con dejar `MAIL_MAILER=log` corriendo en producción; Suprnova
se rehúsa a arrancar ahí sin un reconocimiento explícito, porque un
subsistema de correo que reporta éxito y no entrega nada es el tipo
de incidente que nadie nota durante semanas. El driver `log` en sí se
comporta exactamente como el de Laravel - mensaje completo, cuerpos y
enlaces incluidos - que es lo que lo hace útil en desarrollo, y el
rechazo en producción es lo que mantiene eso seguro (ver
[El driver `log` registra el mensaje completo](#el-driver-log-registra-el-mensaje-completo)).

## Buenas prácticas

### Registra las fábricas en el arranque, no por solicitud

`Mail::queue` y `Mail::later` empujan un `SendMailJob` que lleva el
nombre del mailable y el payload JSON - el worker reconstruye el
tipo concreto vía `mailable_registry`. Registra cada `Mailable`
encolable una sola vez en el momento de `Server::serve`:

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

Un `Mail::queue` para un mailable no registrado aterriza en la cola,
corre una vez, golpea "unknown mailable", reintenta según la
política de backoff del sobre, y se envía a fallidos - costando un
tiempo de observabilidad que no habrías gastado si la fábrica se
hubiera vinculado en el arranque.

### Encola el correo ante cualquier renderizado lento o poco confiable

Enviar correo dentro de un handler de solicitud acopla la latencia
de la respuesta al usuario con tu servidor SMTP (o la API HTTP del
proveedor que sea). Usa `Mail::queue` para cualquier cosa más allá
de un renderizado local síncrono de desarrollo, y `Mail::later`
cuando quieras que el despacho se difiera - seguimientos de
onboarding, correos de recordatorio, resúmenes programados.

```rust
// Mal: acopla el tiempo de respuesta al proveedor de correo
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// Bien: el 200 OK vuelve de inmediato; el worker entrega el correo.
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Fija siempre `from` en un Mailable

El remitente por defecto del framework es `noreply@localhost` - útil
para detectar remitentes faltantes en desarrollo, no un remitente que
ningún proveedor aceptará en producción. Anula `Mailable::from(&self)`
(o establece `from = "..."` en el atributo `#[mail(...)]` sobre un
`NotificationMailable`) para que cada mensaje despachado tenga una
identidad de remitente real:

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

La anulación por mensaje sobre `MailBuilder`
(`.from(("Operations", "ops@example.com"))`) tiene precedencia sobre
el valor por defecto del mailable - útil para envíos transaccionales
puntuales.

### Usa la cola para entrega al menos una vez, no la ruta directa

`MailBuilder::send` es como mucho una vez: si el transporte falla a
mitad de camino al despachar hacia dos proveedores, no puedes
reintentar sin arriesgar un doble envío. `MailBuilder::queue` viaja
sobre el sobre duradero de la cola, que soporta claves de
idempotencia y reintento a nivel de worker. Para cualquier correo que
no debas perder Y no debas enviar dos veces, encólalo con una clave
de idempotencia estable vinculada al evento de origen.

## Mensajes puntuales: `Mail::raw` y `Mail::html`

Cuando el correo es un único aviso transaccional que no justifica un
struct `Mailable` completo, dos atajos evitan el boilerplate:

```rust
use suprnova::mail::Mail;

// Texto plano
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

El closure recibe un [`MailBuilder`] precargado con el cuerpo y te
deja apilar destinatarios, asunto, remitente, etiquetas, metadatos,
prioridad, y cualquier otro método fluido de [`MailBuilder`] encima.
Estas rutas evitan el trait `Mailable` por completo - útiles para
envíos de prueba puntuales y notas transaccionales cortas.

## Valores globales por defecto: `always_from`, `always_reply_to`, `always_to`, `always_return_path`

Reflejando `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo` /
`alwaysReturnPath` de Laravel, la fachada Mail expone cuatro
setters globales:

```rust
use suprnova::mail::{Address, Mail};

// At boot:
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// "Bandeja única" para dev local - enruta TODO el correo a una dirección, descarta CC/BCC:
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// Revierte todo (los tests suelen llamar esto al finalizar):
Mail::forget_always()?;
```

La precedencia es conservadora - los valores por defecto solo
aplican cuando el mensaje despachado carece de un valor explícito:

| Campo | El valor por defecto aplica cuando |
|-------|---------------------|
| `always_from` | El `from` del mensaje es el valor por defecto del framework `noreply@localhost` |
| `always_reply_to` | El mensaje no tiene ningún `reply_to` explícito |
| `always_to` | Siempre - enruta cada mensaje a esta dirección, limpia CC/BCC |
| `always_return_path` | El mensaje no tiene ningún `return_path` explícito |

La misma precedencia aplica en la ruta de la cola: los mailables
encolados pasan por `apply_always_defaults` en el momento del
despacho del worker, así que los envíos directos y los envíos en
cola convergen en formas de sobre idénticas.

## Etiquetas, metadatos, prioridad, encabezados, Return-Path

Todo mensaje despachado puede llevar pistas para el proveedor con forma
de Laravel - etiquetas, pares clave/valor de metadatos, prioridad
RFC-2076, encabezados MIME personalizados y una dirección de Sender / de
retorno de rebotes. Se reenvían a los campos nativos de los proveedores
HTTP (`Tag` / `Metadata` / `Headers` de Postmark, `EmailTags` más
`Content.Simple.Headers` de SES, `categories` / `custom_args` /
`headers` de SendGrid, `o:tag` / `v:` / `h:` de Mailgun, `tags` /
`headers` de Resend) y a SMTP como encabezados RFC 5322.

En SES en concreto, los encabezados viajan en la forma de contenido que
use el mensaje: `Content.Simple.Headers` para un mensaje simple, líneas
de encabezado MIME reales para un mensaje con adjuntos (que SES solo
acepta como MIME crudo). Un nombre de encabezado se valida igual sin
importar en qué forma acabe el mensaje - se rechazan CR, LF y NUL (así
es como una cadena aportada por quien llama se convierte en un segundo
encabezado), y también un nombre vacío, un nombre de más de 76 bytes, un
byte no ASCII, o un `:` o un espacio en el nombre, igual que exige el
propio constructor de MIME crudo. Un nombre de encabezado repetido más
de una vez conserva todos los valores en la ruta del mensaje simple,
pero solo el último valor en la ruta con adjuntos - el mismo límite que
tiene SMTP.

Dos formas de adjuntarlos - a nivel de Mailable para valores por defecto
por tipo, o por mensaje en el builder:

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
// Por mensaje en el builder. El builder gana en colisiones de clave de metadatos; las etiquetas y los encabezados se unen.
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

Las constantes de los cinco niveles de prioridad viven en
`suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}`,
la misma escala entera `1..=5` que usa Laravel.

## Inspeccionar mensajes capturados

`OutgoingMessage` lleva ayudantes de inspección al estilo Laravel -
útiles tanto para aserciones de test como para auditoría en tiempo
de ejecución:

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

Las comprobaciones de destinatario no distinguen mayúsculas de
minúsculas en el email; las comprobaciones de metadatos, etiqueta,
asunto, y nombre de archivo adjunto son exactas.

## Fake de pruebas: superficie ampliada

`Mail::fake()` cubre TANTO la ruta enviada como la ruta encolada. El
correo enviado (vía `MailBuilder::send`) aterriza en el transporte en
memoria; el correo encolado (vía `.queue` / `.later`) aterriza en el
búfer de cola del fake.

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // Lado de enviados
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // Lado de encolados (para correos diferidos)
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // Compuesto
    fake.assert_outgoing_count(2);   // enviados + encolados
    fake.assert_not_outgoing("PasswordReset");
}
```

Ayudantes adicionales:

| Helper | Propósito |
|--------|---------|
| `fake.captured()` | Todos los mensajes enviados |
| `fake.count()` | Conteo de enviados |
| `fake.queued()` | Todos los `QueuedSnapshot` encolados |
| `fake.queued_count()` | Conteo de encolados |
| `fake.outgoing_count()` | Enviados + encolados |
| `fake.sent(predicate)` | Filtra enviados por predicado |
| `fake.sent_to(email)` | Filtra enviados por destinatario |
| `fake.queued_named(name)` | Mailables encolados de un nombre dado |
| `fake.queued_to(email)` | Mailables encolados a un destinatario |
| `fake.assert_sent_count(n)` | Conteo exacto de enviados |
| `fake.assert_queued_count(n)` | Conteo exacto de encolados |
| `fake.assert_outgoing_count(n)` | Total exacto |
| `fake.assert_nothing_sent()` | Búfer de enviados vacío |
| `fake.assert_nothing_queued()` | Búfer de encolados vacío |
| `fake.assert_nothing_outgoing()` | Ambos vacíos |
| `fake.assert_sent_to(email)` | Al menos uno enviado al destinatario |
| `fake.assert_not_sent_to(email)` | Ninguno enviado al destinatario |
| `fake.assert_queued(name)` | Al menos uno encolado de ese nombre |
| `fake.assert_queued_with(name, fn)` | Al menos uno encolado de ese nombre que coincide con el predicado |
| `fake.assert_queued_to(email)` | Al menos uno encolado a ese destinatario |
| `fake.assert_not_queued(name)` | Ninguno encolado de ese nombre |

`QueuedSnapshot::decode::<M>()` deserializa el payload de vuelta al
`M` concreto, así que los predicados con verificación de tipos
funcionan sin código repetitivo de decodificación a medida.

## Eventos: `MessageSending` y `MessageSent`

Cada despacho exitoso dispara dos eventos del framework:

- `MessageSending` - inmediatamente ANTES de la llamada al
  transporte. Los oyentes observan la forma del mensaje
  (destinatarios, asunto, etiquetas, flags de forma del cuerpo).
- `MessageSent` - inmediatamente DESPUÉS de una llamada exitosa al
  transporte. Los oyentes observan la misma forma; los envíos
  fallidos no emiten este evento.

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

Ambos eventos son solo de observación - el despachador no modela un
canal de cancelación al estilo Laravel. Ver
[Por qué Suprnova diverge](#por-qué-suprnova-diverge) arriba para el
rodeo de condicionamiento.

## Comodidad para varios destinatarios: `Mail::cc` y `Mail::bcc`

La fachada Mail expone tres puntos de entrada - `to`, `cc`, `bcc` -
que todos devuelven un `MailBuilder` nuevo. Usa el que coincida con
la intención de enrutamiento dominante:

```rust
// Start with a cc / bcc when the message is primarily an audit copy.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

La misma superficie fluida aplica sin importar con cuál punto de
entrada empieces.

### Testea contra `Mail::fake()`, no contra el transporte vinculado

`Mail::fake()` instala un transporte de captura local al proceso
durante la vida de la guarda RAII y restaura lo que estuviera
vinculado antes. Los tests que la usan no necesitan limpiar
globales en cada entrada/salida - la semántica de drop se encarga de
eso. Combina `#[serial_test::serial]` con `Mail::fake()` para tests
que mutan el global del transporte; los tests concurrentes se
pisarían entre sí de otro modo.

## Siguiente

- [Notificaciones](notifications.md) - `Notify::send` se dispersa
  entre los canales de correo, base de datos, y web push;
  `#[derive(NotificationMailable)]` es el atajo dirigido por macro
  sobre el trait `Mailable`
- [Colas](queues.md) - el sobre duradero sobre el que viajan
  `Mail::queue` y `Mail::later`
- [Eventos](events.md) - escuchar `MessageSending` / `MessageSent`
  más el modelo más amplio del despachador
- [Pruebas](testing.md) - `Mail::fake()` junto a las demás guardas
  `*::fake()`
- [Configuración](configuration.md) - registro de configuración
  tipada para las credenciales de servicio

## Referencia

- Trait: `suprnova::mail::Mailable`
- Fachada: `suprnova::mail::Mail`
- Arranque: `suprnova::mail::boot::bootstrap_from_env()`
- Transportes: `LogMailTransport`, `InMemoryMailTransport`, `SmtpMailTransport`, `PostmarkMailTransport`, `SesMailTransport`, `SendGridMailTransport`, `MailgunMailTransport`, `ResendMailTransport`
- Job de cola: `suprnova::mail::SendMailJob`
- Guarda de pruebas: `suprnova::mail::MailFake`
- Ayudante de telemetría: `suprnova::mail::dispatch_with_telemetry`
