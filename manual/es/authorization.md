# Autorización

La autenticación responde _"¿quién eres?"_; la autorización responde
_"¿tienes permiso para hacer esto?"_ Suprnova ofrece una fachada `Gate`
con forma de Laravel, más la macro `#[policy]` para el cableado
orientado a recursos, con variantes sync y async de cada comprobación,
de modo que la misma superficie funciona tanto si el cuerpo de tu
política necesita una consulta a la BD como si solo necesita comparar
un campo de un struct.

## Inicio rápido

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// Permite que los usuarios opten por la ergonomía `user.can(action, &resource)`.
impl Authorizable for User {}

// Cablea una habilidad:
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// Devuelve 403 directamente desde un handler:
alice.authorize("update", &foreign_post)?;
```

## La superficie de `Gate`

### Definir habilidades

```rust
// Closure sync - se invoca directamente, sin future boxeado.
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Closure async - el future debe ser owned (sin préstamos que sobrevivan al retorno del closure).
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ...consulta a la BD, llamada RPC, etc.
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

Internamente se borra el tipo; el registro indexa por `(action,
TypeId<U>, TypeId<R>)`. Una compuerta de acción de `User` y una
compuerta de acción de `Comment` con el mismo nombre viven de forma
independiente - `Gate::has::<User, Post>("publish")` y
`Gate::has::<User, Comment>("publish")` responden por separado.

### Comprobar habilidades

| Método | Devuelve | Uso |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | Ramificación rápida |
| `Gate::denies(action, &user, &resource)` | `bool` | Inverso |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 403 ante una denegación desnuda; una denegación enriquecida lleva su propio status/message (consulta [Decisiones enriquecidas](#decisiones-enriquecidas-response-inspect-raw)) - hace cortocircuito en un handler con `?` |
| `Gate::inspect(action, &user, &resource)` | `Response` | Decisión completa: `allowed` + `message` + `code` + `status` HTTP |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | Como `inspect`, pero `None` = ninguna regla definida (frente a una denegación explícita) |
| `Gate::any(&[...], &user, &resource)` | `bool` | True si alguna permite |
| `Gate::none(&[...], &user, &resource)` | `bool` | True si ninguna permite |
| `Gate::check(&[...], &user, &resource)` | `bool` | True si todas permiten |

Cada método tiene una contraparte `_async` que funciona tanto para
compuertas registradas de forma sync como async, así que los handlers
no necesitan saber qué tipo de closure respalda la acción.

### Introspección

```rust
// ¿Está definida una habilidad?
Gate::has::<User, Post>("publish");  // bool

// ¿Qué habilidades existen? (ordenadas y sin duplicados por nombre de acción)
let all: Vec<String> = Gate::abilities();
```

`abilities()` elimina duplicados entre tipos de recurso: registrar
`"view"` tanto para `User`-sobre-`Post` como para `User`-sobre-`Comment`
produce una única entrada `"view"`. Útil para selectores de admin y
para los datos compartidos de Inertia.

### Semántica de compuerta ausente

Llamar a `allows` / `denies` / `authorize` sobre una acción que nunca se
registró **por defecto deniega**. Lo mismo para llamar a la API sync
sobre una compuerta registrada como async (la ruta sync no puede hacer
await - denegar por defecto hace que el bug emerja en los logs vía
`tracing::warn!` en lugar de dejarlo pasar en silencio). Las compuertas
registradas como async responden correctamente desde las rutas
`_async`.

## Políticas con `#[policy]`

Cuando un tipo de recurso tiene varias habilidades, agrúpalas en un
struct de política y deja que `#[policy]` registre cada método como una
compuerta:

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // Un método `-> bool` es una compuerta de permitir/denegar simple.
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // cualquiera puede listar posts
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // Un método `-> Response` puede llevar un mensaje + un status HTTP en la denegación.
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // oculta el post a los no administradores
        }
    }
}
```

Cada método se convierte en un `inventory::submit!`. `Server::serve`
drena el inventario vía `init_policies()` en el arranque, así que para
cuando llega la primera solicitud cada acción ya está registrada
(consulta [Arranque de la aplicación](bootstrap.md) para ver dónde
encaja esto en la secuencia de arranque). `init_policies()` vive en
`suprnova::authorization::init_policies` y es idempotente - llámalo
manualmente en tests que ejerciten el registro de políticas sin
levantar un servidor.

Los métodos de política son funciones asociadas sin estado que toman
`(user, resource)` - la misma forma que el `update(User $user, Post
$post)` de Laravel, donde `$this` es el objeto de política sin estado.
Cada método toma ambos argumentos para mantener una firma de compuerta
uniforme; `view_any` / `create` simplemente ignoran el recurso
(`_post`). Los métodos que no escribes no se registran, y una acción
sin registrar deniega por defecto.

### Mapeo de nombre de método → acción

El nombre del método se usa directamente como el segmento verbal de la
acción, con el recurso en kebab-case como sufijo:

| Método | Acción |
|---|---|
| `view` en `Post` | `"view-post"` |
| `view_any` en `Post` | `"view_any-post"` |
| `force_delete` en `UserProfile` | `"force_delete-user-profile"` |

Esto diverge de los nombres de acción en camelCase de Laravel
(`viewAny`, `forceDelete`) para mantener idiomática la superficie de
Rust - cada string de acción refleja el identificador de método que
autocompletarías en tu editor.

### Tipo de retorno: `bool` o `Response`

El tipo de retorno de un método de política selecciona cómo se
registra - y qué puede llevar una denegación:

| Tipo de retorno | Se registra vía | La denegación emerge como |
|---|---|---|
| `bool` | `Gate::define` | un `403` desnudo (`This action is unauthorized.`) |
| `Response` | `Gate::define_with` | el message, code, y status HTTP que lleva el `Response` |

Devuelve `bool` para un sí/no simple. Devuelve un `Response` (importado
desde `suprnova::authorization::Response`) cuando una denegación deba
llevar una razón o un status distinto de 403 - `Response::deny_with("…")`
para un mensaje, o `Response::deny_as_not_found()` para responder `404`
y ocultar la existencia del recurso. Ambos compilan a la misma
compuerta de tipo borrado (un `bool` se envuelve en un
permitir/denegar desnudo). Cualquier otro tipo de retorno - o la
ausencia de uno - es un error de compilación.

## El trait `Authorizable`

Conveniencia lista para usar, del lado del usuario, sobre las llamadas
a `Gate`:

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Azúcar sync
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 si deniega

// Azúcar async
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

Cada método tiene un cuerpo por defecto que delega al método `Gate`
correspondiente, así que `impl Authorizable for User {}` (sin cuerpo)
es suficiente. Es opt-in en lugar de un blanket-impl: no todo tipo que
se pueda pasar a `Gate::allows` está pensado para ser el sujeto de
`.can` - lo más habitual es que sea el `User` de tu aplicación.

## Patrones de composición

### Compuertas en grupos de rutas

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// El middleware comprueba el usuario autenticado; el handler autoriza la acción.
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... renderiza el formulario de edición
}
```

### Comprobaciones de varias acciones

Una página de "lista todo lo que este usuario puede hacer sobre este
recurso":

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// O haz cortocircuito:
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### Autorización multi-compuerta

```rust
// Solo permite si el usuario puede hacer TODAS estas acciones sobre el recurso.
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // Combina comprobaciones.
}
```

### Compuertas en rutas de recursos

Cuando existe una superficie `Router::resource`,
`authorize_resource::<U, R>()` cablea la comprobación de habilidad
convencional sobre las siete rutas de una sola vez, así que no dependes
de que cada método de controlador se acuerde de autorizar:

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view, store→create, …
    .into();
```

Una habilidad denegada devuelve `403` antes de que el handler se
ejecute; una solicitud no autenticada falla en cerrado. La tabla
completa de acción → habilidad vive en el [capítulo de
enrutamiento](routing.md).

## Semántica async

El closure de `Gate::define_async` debe devolver un future **owned** -
el registro de tipo borrado no puede permitir que las referencias
`&user` o `&resource` sobrevivan al retorno del closure. Copia o clona
los campos que necesites dentro del bloque `async move {}` antes de
devolverlo:

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // copia el primitivo
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // Sin referencias a `user` / `post` aquí - solo las copias capturadas.
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Las compuertas sync funcionan de forma transparente desde la ruta async
(`Gate::allows_async` las despacha sin un `.await`), así que una base
de código puede registrar compuertas sync hoy y migrar habilidades
individuales a async más adelante sin cambiar los sitios de llamada.

## Postura ante bloqueos envenenados

El registro de `Gate` usa un `RwLock` internamente. Si el bloqueo llega
a envenenarse (un hilo entró en pánico mientras sostenía la guarda de
escritura), el registro **deniega de forma segura** - cada llamada
posterior a `authorize` devuelve `Unauthorized` en lugar de entrar en
pánico. Las llamadas de registro escriben en `tracing::error!` y
continúan. Esto coincide con la política más amplia del framework: un
bloqueo envenenado nunca aborta el proceso.

## Decisiones enriquecidas: `Response`, `inspect`, `raw`

Una compuerta `bool` desnuda solo responde permitir/denegar. Para una
denegación que lleve un *message*, un *code* de máquina, o un *status*
HTTP distinto de 403, registra la compuerta con `define_with` (o
`define_async_with`) y devuelve un `Response`:

```rust
use suprnova::authorization::Response;  // re-exportado en la raíz del crate como `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// Oculta la existencia de un recurso en lugar de admitir que existe:
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // un 404, no un 403
    }
});
```

Inspecciona la decisión completa con `Gate::inspect` (sync) /
`Gate::inspect_async`:

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - None aquí; Some(404) tras deny_as_not_found
```

Los constructores de `Response` reflejan los de Laravel: `allow()`,
`deny()`, `deny_with(msg)`, `deny_with_status(status, msg)`,
`deny_as_not_found()`, además de los builders `with_message` /
`with_code` / `with_status` / `as_not_found`.

### Cómo una denegación se convierte en un error

`Gate::authorize` colapsa la decisión a través de
`Response::authorize()`:

| Decisión | Resultado de `authorize` |
|---|---|
| permitida | `Ok(())` |
| `deny()` desnudo (sin message/code/status) | `FrameworkError::Unauthorized` (403, `"This action is unauthorized."`) |
| denegación enriquecida (con message y/o status establecidos) | `FrameworkError::Domain { message, status_code }` |

Así, `deny_as_not_found()` emerge como un 404, `deny_with_status(422,
"…")` como un 422, y `deny_with("…")` como un 403 que lleva tu mensaje.
El `code` se puede leer en el `Response` inspeccionado, pero **no**
viaja a través de `authorize` - `FrameworkError` no tiene campo `code`;
léelo desde `inspect()` si lo necesitas.

### `raw`: "denegado" frente a "indefinido"

`Gate::raw` (y `raw_async`) devuelve `Option<Response>`: `None`
significa *no se aplicó ninguna regla* - no se disparó ningún hook
`before`, no hay compuerta registrada, ningún hook `after` la rellenó -
a diferencia de un `Some(deny)` explícito. `inspect` normaliza ese
`None` a una denegación por defecto; `raw` lo preserva para
diagnóstico ("¿esta acción está gobernada siquiera?").

## Hooks `before` / `after`

`Gate::before` registra una comprobación que corre *antes* que
cualquier compuerta; el primer hook que devuelva `Some(decision)` hace
cortocircuito con todo. El uso canónico es una anulación global:

```rust
// Los administradores pueden hacer cualquier cosa.
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` corre *después* de la compuerta. Siguiendo la semántica
`??=` de Laravel, un hook after solo puede **rellenar** un resultado
indeciso (ninguna compuerta coincidió y ningún hook before se
disparó) - nunca puede sobrescribir un permitir/denegar ya producido.
Cada hook after igual se ejecuta, así que también sirve como el punto
de enganche para el registro de auditoría:

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // observa cada evaluación
    None                                    // solo registra; no cambia el resultado
});
```

Los hooks se indexan por el **tipo de usuario** `U`, no por recurso -
un hook se dispara para cada `(action, U, R)`. Pon la lógica específica
de recurso en la compuerta. Los hooks son predicados síncronos y
también se aplican a la ruta de evaluación async; para lógica de
autorización async, usa `define_async` / `define_async_with`.

### Por qué Suprnova diverge

El `Gate::forUser($user)->allows(...)` de Laravel revincula el resolver
*implícito* del usuario actual de la compuerta, de modo que la
siguiente comprobación se evalúa como ese usuario. La compuerta de
Suprnova toma el usuario de forma **explícita** en cada llamada, así
que "comprobar como un usuario distinto" es solo
`Gate::allows(action, &other_user, &resource)`. No hay ningún resolver
implícito que revincular - la API explícita es estrictamente más
general, lo que hace que `forUser` sea redundante en lugar de faltante.

El mismo razonamiento aplica al auto-descubrimiento de políticas de
Laravel por nombre de clase. Suprnova ata los métodos de política a la
clave de tipo borrado `(action, U, R)` en el momento del registro, así
que una política de `Post` y una política de `Comment` con el mismo
nombre de método registran dos compuertas distintas sin necesitar una
convención de nombres ni un escaneo de descubrimiento.

## Siguiente

- [Autenticación](authentication.md) - la mitad del lado del usuario:
  guards, `Auth::user()`, `Auth::user_as::<T>()`
- [Arranque de la aplicación](bootstrap.md) - dónde corre
  `init_policies()` en la secuencia de arranque, además de cómo
  registrar hooks before/after
- [Middleware](middleware.md) - emparejar `AuthMiddleware` con
  autorización a nivel de ruta
- [Modelo de errores](error-model.md) - cómo una denegación de
  compuerta colapsa en un 403, un 404, o un `FrameworkError::Domain` de
  status personalizado
- [Eventos](events.md) - escuchar los resultados de políticas vía
  `Gate::after` para el registro de auditoría
