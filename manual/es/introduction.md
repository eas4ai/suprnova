# Introducción

Suprnova es un framework web para Rust que ofrece la experiencia de desarrollador
de Laravel sobre Tokio. Se escriben controladores y modelos de estilo Eloquent;
el framework proporciona concurrencia, seguridad de tipos e implementación de un
único binario.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// Luego, en cualquier parte:
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

Si hubieras escrito eso en Laravel la semana pasada, la versión Rust anterior
se sentiría idéntica - la misma forma de cadena, los mismos nombres de métodos,
los mismos valores por defecto. La diferencia está en lo que ocurre internamente:
Tokio en lugar de FPM, un único binario en lugar de un runtime PHP, verificaciones
de tipos en tiempo de compilación en cada columna.

## Por qué existe Suprnova

Laravel solucionó el problema de productividad para el desarrollo web de backend.
Los patrones funcionan. Después de diez años de refinamiento, muy poco se interpone
en tu camino cuando estás construyendo un producto real. Pero el modelo request-per-process
de PHP mantiene dos cosas fuera del alcance: conexiones de larga duración económicas
(WebSockets, SSE, notificaciones enviadas por el servidor sin polling) e I/O concurrente
trivial dentro de un handler de solicitud.

Rust te ofrece ambas gratuitamente con Tokio. El problema es que el ecosistema web
de Rust te hace construir la capa de productividad tú mismo: elige un crate HTTP,
elige un ORM, elige una herramienta de migración, elige una cola, conecta todo junto,
diseña tus propias convenciones. Cada aplicación reinventa lo que Laravel ya estandarizó.

Suprnova es lo que sucede cuando copias las convenciones de Laravel a Tokio.
Obtienes:

- **La misma superficie** - `routes!`, `Auth::user()`, `Cache::remember`,
  `Mail::send`, `Queue::push`, `Storage::disk("s3")`, `Notify::send`,
  `Schedule::call`, `Gate::allows`, el constructor de consultas Eloquent,
  eliminación suave, fábricas, observadores, difusión, todo
- **Motor diferente** - async en todas partes, conexiones de larga duración como
  ciudadanos de primera clase, único binario estáticamente enlazado, sin prefork,
  sin opcache, sin FPM
- **Seguridad de tipos** - tus modelos, rutas y cargas útiles de eventos se comprueban
  en tiempo de compilación; los refactores rotos no llegan a producción
- **Una verdadera historia de frontend** - Inertia.js se conecta a iniciadores de
  Svelte 5, React 19 o Vue 3.5, sin necesidad de mantener una API separada

## Principios de diseño

Estos son los principios a los que se adhieren los autores del framework.
Explican por qué un capítulo dice lo que dice.

**1. La paridad proviene del registro de cambios de Laravel.** Cuando Laravel
lanza una característica, Suprnova la sigue. La línea de base actual es Laravel 13.x
y cada subsistema entregado ha sido auditado contra él. El
[Mapa de paridad con Laravel](parity.md) es la tabla explícita característica por característica.

**2. Divergir intencionalmente cuando Rust hace las cosas mejor.** Donde Laravel
hizo una elección moldeada por PHP que no tenemos que hacer en Rust, Suprnova elige
la moldeada por Rust y lo indica. El ejemplo más grande es la concurrencia:
WebSockets, difusión, workers en segundo plano y HTTP/2 server-push
son de primera clase, no agregados. Donde lo verás señalado en un
capítulo, busca los cuadros **"Por qué Suprnova diverge"**.

**3. Sin restricciones.** Laravel restringe algunas características a un backend
(p. ej. búsqueda vectorial vía Postgres `pgvector`). Suprnova trata los backends
como drivers - `Vector::driver("qdrant")`, `Vector::driver("pinecone")`,
`Vector::driver("mariadb")`, `Cache::driver("redis")`, `Mail::driver("ses")`.
Tú eliges la herramienta correcta; nosotros no elegimos por ti.

**4. Suprnova es la superficie de la API.** Internamente utilizamos SeaORM, hyper, Tokio,
serde, sqlx, validator, lettre, y muchas más. Nada de eso debería
aparecer en tu código. Dependes de `suprnova::*`. Re-exportamos todo
lo que tocas - incluyendo `Entity`, `Column`, `ActiveModel`,
`QueryFilter`, etc. de SeaORM - bajo la raíz del framework. La vía de escape
(`use suprnova::sea_orm;`) existe para el raro caso donde la superficie curada
no cubre, pero casi nunca deberías necesitarla.

## Qué viene incluido

Un mapa no exhaustivo. La lista completa está en [`documentation.md`](documentation.md).

| Área | Qué incluye |
|---|---|
| **HTTP** | macro `routes!`, controladores, middleware, solicitudes, respuestas, vinculación de modelo de ruta, URLs firmadas, enrutamiento de recursos, ayudantes de redirección, CORS, CSRF, claves de idempotencia, tiempo de espera, limitación de velocidad, errores estructurados con recuperación de pánico |
| **Base de datos** | SeaORM bajo el capó, multi-driver (Postgres, MySQL, MariaDB, SQLite), migraciones, sembradores, constructor de consultas, transacciones con puntos de guardado, división de lectura/escritura de múltiples conexiones |
| **Eloquent** | macro `#[suprnova::model]`, los 11 tipos de relación, carga anticipada, eliminación suave, podable, alcances (locales + globales), 16 eventos del ciclo de vida, observadores, 22 conversiones incorporadas, accesores/mutadores, tres paginadores, iteración chunk/lazy/cursor, colecciones, replicación |
| **Autenticación** | Sesiones con estado, IDs de usuario opacos, múltiples guards, proveedores de Eloquent + base de datos, hash de contraseña (bcrypt + argon2), macros de política, compuertas, verificación de correo, restablecimiento de contraseña, limitación de fuerza bruta, 2FA TOTP, recordarme, OAuth vía integración torii |
| **Frontend** | Puente Inertia v3, plantillas de inicio de Svelte 5 / React 19 / Vue 3.5, `#[derive(InertiaProps)]` tipado, recargues parciales, generación automática de tipos TypeScript |
| **Segundo plano** | Cola con drivers memory/sync/redis/database/null, lotes, cadenas, middleware de trabajo, almacén de trabajos fallidos, binario de consola `#[command]`/`#[derive(Command)]`, planificador de traits `Task`, trabajo `#[workflow]` de larga duración con estado, trait `Supervisor` con reinicio automático de captura de pánico, bus de comandos, despachador de eventos |
| **Tiempo real** | macro `ws!()` para handlers WebSocket tipados, canales de difusión (público, privado, presencia), fanout sea-streamer, eventos enviados por el servidor, web push (VAPID) |
| **Caché y almacenamiento** | Drivers de caché Memory, Redis, Database; operaciones atómicas; caché etiquetado; bloqueos de caché; sistema de archivos con drivers fs/memory/s3/azblob/gcs; protección contra traversal de ruta; almacenamiento vectorial con múltiples backends |
| **Correo y notificaciones** | trait `Mailable`, drivers para SMTP/SES/Mailgun/Postmark/SendGrid/Resend (más in-memory & log para pruebas), `Notifiable` con canales mail/database/broadcast/webpush |
| **Validación y datos** | `#[derive(Validate)]`, solicitudes de formulario, validación asíncrona, `#[derive(Data)]` para conjuntos de inclusión de recargue parcial, `#[derive(Resource)]` para JSON:API |
| **Pagos** | Superficie de proveedor genérico (gateway/MoR/redirect-flow), adaptadores de referencia para Stripe y Paddle, tablas de copia local con idempotencia de webhook, componentes de checkout de Inertia |
| **Indicadores de características** | Evaluador de base de datos, evaluador en caché con TTL, middleware de característica, propagación sub-segundo vía trait de sincronización |
| **Pruebas** | `#[suprnova_test]`, `expect!`, `TestDatabase`, falsificaciones para cada superficie externa (Mail, Notify, Queue, Bus, Events, Storage, Http) |
| **CLI** | generador de andamiaje `suprnova new` (Svelte/React/Vue), ejecutor dev `serve`, `migrate*`, `db:sync`, `db:seed`, generadores `make:*`, `model:prune`, binario de consola por proyecto |

## Listo para producción

El framework tiene un alcance de grado de producción y está probado. A partir de
la rama HEAD actual:

- Cada superficie de Laravel 13.x en los 30 dominios documentados está entregada
- Cada problema señalado por revisión de código independiente ha sido resuelto
- El conjunto de pruebas del espacio de trabajo se aprueba en cada cambio
- Cada API pública en `framework/src/lib.rs` está documentada - un
  elemento público sin documentar hace fallar la compilación

A partir de **v1.0.0** la API pública es estable: las aplicaciones fijan una etiqueta
de lanzamiento (`tag = "v<version>"` - la etiqueta es el lanzamiento; no hay
publicación crates.io), y un cambio que rompa la compatibilidad solo se integra
detrás de un cambio de versión cuya sección en el [CHANGELOG](changelog.md) lo indica.

## Elige tu ruta de lectura

| Eres… | Comienza con |
|---|---|
| Un desarrollador de Laravel | [Desde Laravel](from-laravel.md) |
| Un desarrollador de Rust que ha usado Axum/Actix/Rocket | [Desde Rust web](from-rust-web.md) |
| Ambos, o ninguno, y solo quieres crear | [Instalación](installation.md) → [Inicio rápido](quickstart.md) |
| Buscando una característica específica | [`documentation.md`](documentation.md) (la TOC maestra) |
| Preguntándote "¿Suprnova tiene X?" | [Mapa de paridad con Laravel](parity.md) |
