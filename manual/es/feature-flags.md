# Indicadores de características

El sistema de flags de Suprnova combina declaraciones `Feature` en
tiempo de compilación con anulaciones en tiempo de ejecución
persistidas en una tabla `features`. El valor de un flag en el
momento de evaluarlo se determina, en este orden, por:

1. Una fila con alcance en la tabla `features` - `user:42` o
   `team:staff`.
2. La fila global en la tabla `features` (alcance `""`).
3. El `default` en tiempo de compilación fijado en la declaración
   `Feature`.

Los cambios hechos desde el CRUD de administración se propagan a los
evaluadores activos antes de que la llamada de mutación retorne. Los
flags de interruptor de corte se desactivan de verdad en tiempo real,
no "dentro de la próxima ventana de TTL".

## Inicio rápido

```rust
// app/src/features.rs - aquí vive cada flag que tu app referencia.
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - conecta la cadena una sola vez durante el arranque.
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ... DB::init, sesión, etc.

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// cualquier handler - Feature::is_enabled() resuelve contra el contexto por solicitud.
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// cambia el flag desde una ruta de admin o la CLI:
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - None para cambios iniciados por el sistema
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ auditoría: quién lo cambió
//                                  |   └ enabled
//                                  └ scope_key: "" = global, "user:42" = anulación con alcance
```

La siguiente llamada a `NEW_CHECKOUT_FLOW.is_enabled()` observa
`true` - incluida cualquier entrada de evaluador cacheada, que fue
invalidada de forma síncrona dentro de `admin::upsert`.

## Las piezas

### `Feature<'a>`

La declaración en tiempo de compilación. Lleva el nombre del flag y
un valor por defecto para cuando está ausente.

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ por defecto: true (los pagos están habilitados hasta que se desactiven)
```

Centralizar cada declaración en `app/src/features.rs` te da:

- un único lugar donde buscar con grep cuando un operador pregunta
  "¿qué flags existen?"
- unicidad del nombre del flag en tiempo de compilación - un error
  de tipeo en el sitio de llamada no compila
- el lugar obvio para poner un comentario de documentación que
  explique qué controla el flag

Llama a `flag.is_enabled()` para leer contra el contexto ambiente
(configurado por [`FeatureMiddleware`](#featuremiddleware)), o a
`flag.is_enabled_in(Some(&ctx))` para pasar un
[`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)
específico.

Las macros `feature!` e `is_enabled!` también se reexportan desde
`suprnova::*` para los sitios de llamada que no quieren importar la
constante:

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

Lee la tabla `features` en una instantánea en memoria en el arranque
y en cada [`reload()`](#control-de-flujo-propagación-de-flags). La ruta de
ejecución frecuente (`is_enabled`) es totalmente síncrona - sin
consulta a la BD por solicitud, sin `block_on` dentro del evaluador.

Orden de resolución en la búsqueda, del más específico al menos
específico:

1. `user:{id}` - cuando el contexto de la solicitud lleva un
   `UserIdField`.
2. `team:{name}` - cuando el contexto lleva un `TeamField`.
3. `""` - el flag global.
4. `None` - la fila no existe, y el valor por defecto en tiempo de
   compilación toma el control.

### `CachedEvaluator`

Memoiza las búsquedas `(feature, user, team)` detrás de un `DashMap`
con un TTL que tú elijas. La ruta de ejecución frecuente sigue siendo
síncrona; las entradas se descartan de forma síncrona cuando
[`admin::upsert`](#crud-de-administración) escribe un flag.

Un TTL de cero degenera en "sin caché" - cada llamada recae en el
evaluador interno. Útil para apps con pocos flags que quieren la
maquinaria de propagación sin la caché.

### `FeatureMiddleware`

Abre un contexto de featureflag por solicitud, poblado por
extractores definidos por el usuario. Por defecto:

- `user_id` - desde `Auth::id()`.
- `team` - ninguno.

Sobrescribe cualquiera de los dos vía el builder:

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // Personalizado: toma el valor de un encabezado en lugar de la sesión.
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// o: .with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### CRUD de administración

`suprnova::features::admin` es la capa de persistencia de la tabla
`features`. Úsala desde handlers de administración, herramientas de
CLI, scripts de despliegue - donde sea que un flag necesite cambiar:

```rust
use suprnova::features::admin;

// Crea o actualiza un flag global.
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// argumentos: name, scope_key, enabled, description, actor_id

// Anulación con alcance de usuario (tiene prioridad sobre la global).
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// Elimina una fila por completo - el flag vuelve a su valor por defecto en tiempo de compilación.
admin::delete("kill-switch.payments", "", actor_id).await?;

// Lectura para una tabla de UI de administración.
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

Cada mutación dispara el [evento](#eventos) correspondiente y llama a
[`features::sync::notify`](#control-de-flujo-propagación-de-flags), de modo
que cualquier evaluador activo vinculado en el contenedor de la App
se refresca antes de que la llamada retorne.

`actor_id: Option<String>` es el puntero de auditoría. Pasa el id de
usuario del operador (el mismo que emite tu capa de auth); deja
`None` para cambios iniciados por el sistema (CLI, migración de
despliegue, etc.).

## Control de flujo: propagación de flags

El trait que hace posible que "el cambio del admin sea visible de
inmediato":

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

Quienes lo implementan reaccionan a las mutaciones:

- `DatabaseEvaluator::on_flag_changed` llama a `self.reload()` -
  trae la instantánea completa.
- `CachedEvaluator::on_flag_changed` llama a
  `self.invalidate(feature)` - descarta cada entrada cacheada para
  ese nombre.

La cadena canónica es un `CompositeFeatureSync`, que **ordena las
fuentes de datos antes que las cachés** - las cachés deben
invalidarse *después* de que la fuente de datos se refresque, o un
lector concurrente puede golpear la caché vacía, recaer en la fuente
de datos desactualizada, y repoblar la caché con el valor viejo.

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // fuentes de datos primero
    vec![cached.clone() as Arc<dyn FeatureSync>],   // cachés en segundo lugar
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` resuelve
`Arc<dyn FeatureSync>` desde el contenedor y espera (`await`) a
`on_flag_changed`. Es un no-op cuando no hay ningún sync vinculado -
el comportamiento correcto para herramientas de administración fuera
de proceso que solo escriben en la BD y no tienen ningún evaluador
activo que refrescar.

## Helper de arranque

`bootstrap_database_cached(ttl)` conecta todo en una sola llamada:

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// Opcional: conserva features.database para programar recargas
// periódicas o exponer vistas de diff para admins. La mayoría de
// las apps descartan el handle y dejan que el refresco impulsado
// por notify haga el trabajo.
```

Qué hace:

1. Construye un `DatabaseEvaluator` contra la conexión primaria a la
   BD.
2. Lo envuelve en un `CachedEvaluator` con el TTL solicitado.
3. Llama a `install_evaluator(cached)` - fija el valor por defecto
   global de featureflag *y* activa un rastreador "instalado"
   propiedad del framework, para que el middleware no registre la
   advertencia de "sin evaluador".
4. Construye un `CompositeFeatureSync` con el orden de slots
   correcto y lo vincula en el contenedor de la App.

Devuelve `BootstrappedFeatures { database, cached }` para quienes
llaman y quieren handles directos a cualquiera de las dos capas.

Si tu topología no es `Cached(Database)` - una caché respaldada por
Redis, una fuente de sync remota, una cadena multinivel - conecta la
cadena a mano usando las mismas primitivas. `bootstrap_database_cached`
es una comodidad, no un contrato.

## Migraciones

El framework es propietario del esquema de la tabla `features`:

```rust
// app/src/migrations/mod.rs
vec![
    // ... las migraciones de tu app ...
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

Esquema:

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` lleva el tipo de alcance inline (`"user:42"`,
`"team:staff"`, `""` para global), así que la ruta de lectura se
mantiene como una única búsqueda de cadena contra un índice único.

## IDs de usuario y de equipo

`UserIdField` y `TeamField` son extensiones tipadas guardadas en
`Context::extensions` de featureflag. Ambas son de tipo cadena, así
que los ids opacos de torii (UUID / ULID) y las columnas numéricas
`users.id` conviven detrás de la misma forma.

Construir un contexto a mano (fuera del middleware):

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // ids de usuario de tipo string - UUIDs, ULIDs, cualquier cosa opaca.
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// los ids numéricos también funcionan - el framework convierte i64 → String en el momento de on_new_context.
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## Eventos

Dos eventos se disparan desde la ruta del CRUD de administración:

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

Escúchalos a través del despachador de eventos del framework para
alimentar un log de auditoría, una alerta de Slack, o cualquier
pipeline downstream que necesites:

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` no dispara un evento en la ruta de lectura.** Cada
solicitud que comprueba un flag multiplicaría el volumen de eventos
por la cantidad de flags comprobados - aceptable para una historia de
auditoría de mutaciones, prohibitivo para trazado en la ruta de
lectura. Si tu despliegue necesita auditoría muestreada en la ruta
de lectura, coloca encima un evaluador personalizado que registre en
un canal de log acotado (un stream de Redis o una cola de
dispersión, según la escala).

## Detección de evaluador ausente

Si `FeatureMiddleware` está instalado pero no se registró ningún
evaluador vía `install_evaluator` / `bootstrap_database_cached`,
cada flag devuelve en silencio su valor por defecto en tiempo de
compilación - una mala configuración grave, y que hay que detectar en
QA. El middleware emite exactamente un `tracing::warn!` por proceso,
en la primera solicitud que observa este estado:

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

El cambio usa un `AtomicBool::swap`, así que una tormenta de
solicitudes concurrentes en el arranque se serializa a una única
emisión de advertencia, no una por worker.

## Pruebas

Dos patrones, según qué estés verificando.

### Test unitario de un `Feature` en aislamiento

Usa `featureflag::evaluator::with_default` para acotar un evaluador
de reemplazo dentro de un closure síncrono:

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` es un helper solo para tests que
arranca su propio SQLite y ejecuta `CreateFeaturesTable`, para que el
test se mantenga hermético. No lo uses en rutas de producción.

### Test de integración de la propagación de punta a punta

Usa `TestDatabase::fresh::<TestMigrator>()` para la BD y
`TestContainer::bind` (NO `App::bind`) para el `FeatureSync` - los
tests en paralelo en el mismo proceso, si no, se sobrescribirían la
vinculación de los demás vía el contenedor global:

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // se propaga al instante
}
```

Consulta `framework/tests/features.rs` para el conjunto completo de
tests de composición.

### Por qué Suprnova diverge

Laravel Pennant resuelve cada flag contra la base de datos a
demanda (con memoización opcional a nivel de driver por solicitud).
El modelo de PHP de un proceso por solicitud hace que un golpe a la
BD por solicitud sea barato, porque la conexión es dedicada y muere
con la solicitud.

El modelo de proceso de Suprnova es lo opuesto - un único binario de
larga duración que sirve miles de solicitudes concurrentes. Un golpe
a la BD por solicitud en cada comprobación de flag multiplicaría la
carga del pool de conexiones por la cantidad de comprobaciones de
flag. La cadena de dos capas (instantánea de `DatabaseEvaluator` +
TTL de `CachedEvaluator`) es la respuesta nativa de Rust: la ruta de
ejecución frecuente es totalmente síncrona contra datos en memoria, y
el trait `FeatureSync` da a los cambios iniciados por el operador una
propagación en menos de un segundo, sin una recarga por sondeo. La
forma es la misma que en Pennant - define un flag, compruébalo en un
handler, anúlalo desde una ruta de administración. La maquinaria es
distinta porque el runtime es distinto.

## Notas de diseño

- **¿Por qué un evaluador síncrono en vez de asíncrono?** El
  `is_enabled` de featureflag es la ruta de ejecución frecuente. Un
  evaluador asíncrono forzaría un `block_on` (propenso a deadlock) o
  empujaría a cada handler a hacer `.await` en cada lectura de flag
  (un desastre de ergonomía). El framework conecta lo síncrono con
  lo asíncrono mediante una instantánea en memoria que
  `FeatureSync` refresca de forma asíncrona.

- **¿Por qué un trait `FeatureSync` separado en lugar de extender
  `Evaluator`?** El `Evaluator` de featureflag es propiedad de un
  crate upstream; no podemos añadirle métodos. `FeatureSync` es un
  trait hermano que las apps implementan sobre los mismos tipos
  concretos. El objeto de trait se vincula por separado en el
  contenedor de la App, de modo que un proceso puede apilar varios
  evaluadores sin dejar de enrutar las notificaciones
  correctamente.

- **¿Por qué `set_flag` es `pub` en `DatabaseEvaluator`?** Comodidad
  para tests. La ruta de escritura de producción es
  `admin::upsert`; `set_flag` existe para que los tests puedan
  sembrar flags sin montar un oyente de `EventFacade`. Ambas rutas
  llaman a `features::sync::notify`, así que el contrato de
  propagación se mantiene de cualquier forma.

- **¿Por qué no hay un evento `FeatureRetrieved`?** Volumen. Un
  handler que comprueba diez flags por solicitud dispara diez
  eventos por solicitud - para un servicio de 1k solicitudes/s eso
  son 36M eventos/hora, muy por encima de la relación
  señal-ruido de cualquier pipeline de auditoría. La auditoría en la
  ruta de mutación (`FeatureUpdated` / `FeatureDeleted`) es lo que se
  entrega; el muestreo en la ruta de lectura, si hace falta, se
  apila encima vía un wrapper de evaluador personalizado.

## Siguiente

- [Middleware](middleware.md) - `FeatureMiddleware` va después de
  `SessionMiddleware`; este capítulo cubre el orden y la pila global
- [Eventos](events.md) - escucha `FeatureUpdated` / `FeatureDeleted`
  para alimentar logs de auditoría, alertas de Slack, o pipelines
  downstream
- [Contenedor de servicios](container.md) - cómo se resuelve la
  vinculación `dyn FeatureSync`, y por qué existe
  `TestContainer::bind` para los tests en paralelo
- [Pruebas](testing.md) - los patrones `TestDatabase::fresh::<M>()` y
  `TestContainer::fake` de los que depende este capítulo
- [Autenticación](authentication.md) - `Auth::id()` es el extractor
  de id de usuario por defecto, y alimenta `actor_id` en las
  mutaciones de administración

Externo: la [documentación del crate featureflag](https://docs.rs/featureflag)
cubre las primitivas upstream `Evaluator`, `Context`, y `Feature`.
`suprnova::features::admin` es la fachada CRUD completa - usa
`cargo doc --open -p suprnova` para explorarla.
