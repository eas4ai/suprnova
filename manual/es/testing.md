# Pruebas

Este es el capítulo hub para la superficie de pruebas de Suprnova - las
macros, la base de datos en proceso, los fakes del contenedor, y los
ayudantes de clave de cifrado a los que recurren tus binarios de test.
Los capítulos en profundidad viven junto a este: [Pruebas
HTTP](http-tests.md) para rutas + middleware, [Pruebas de base de
datos](database-testing.md) para todo lo relacionado con
`TestDatabase`, [Simulación y falsificaciones](mocking.md) para las
siete superficies externas (Mail, Notify, Queue, Bus, Events, Storage,
cliente HTTP). Lee este para aprender qué hay dentro de la caja; salta
a un capítulo hermano cuando necesites la versión larga.

## Las piezas

| Pieza | Rol |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | El pilar por defecto - cada test real en el framework usa esto |
| `#[suprnova_test]` | Azúcar sintáctico de macro de atributo - ejecuta `App::init()` + `App::boot_services()` y te construye una `TestDatabase` |
| `describe!` + `test!` | Macros de agrupación al estilo Jest, emparejadas con `expect!` para una salida de fallo con nombre |
| `expect!` | Macro de aserción fluida con matchers tipados (igualdad, option, result, string, vec, ordenamiento) |
| `TestDatabase::fresh` / `sqlite_memory` | SQLite en memoria + registro en el contenedor, con o sin tu migrator |
| `TestContainer::fake` / `scope` / `spawn` | Overrides de DI thread-local o task-local, herméticos entre tests en paralelo |
| `install_test_encryption_key[ring]` | `APP_KEY` determinista para tests que tocan casts cifrados o payloads firmados |
| Ayudantes `fake()` por superficie | Correo, Notificaciones, Cola, Bus, Eventos, Almacenamiento, HTTP - consulta [Simulación](mocking.md) |
| `TestResponse` | Aserciones fluidas sobre la tupla `(status, headers, body)` de un test HTTP; consulta [Pruebas HTTP](http-tests.md#fluent-response-assertions-with-testresponse) |
| `AssertableInertia` | Aserciones fluidas sobre un objeto de página Inertia; consulta [Pruebas HTTP](http-tests.md#testing-inertia-responses) |

No vas a recurrir a todo en un solo test. Un test de action típico usa
los primeros tres; un test cargado de DI añade `TestContainer`; un
test HTTP cambia `TestDatabase` por el pipeline de `handle_request`;
un test de pagos instala el keyring de cifrado.

## El pilar por defecto

Cada test real en el framework tiene este aspecto:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` abre una conexión `sqlite::memory:`
nueva, ejecuta tu migrator de punta a punta, y registra la conexión
en el contenedor de test. Cualquier código que llame a
`DB::connection()` o `App::resolve::<DbConnection>()` después se
resuelve contra ella - incluido el query builder de
`#[suprnova::model]` y cualquier servicio que resolvieras desde el
contenedor. Cuando `TestDatabase` se descarta, el registro se va con
ella.

La macro `test_database!()` es azúcar de una línea para el caso
`crate::migrations::Migrator`:

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

Para tests que quieren control preciso sobre la forma de las columnas
(idas y vueltas de casts, superficie SQL del query builder), usa
`TestDatabase::sqlite_memory()` - el mismo cableado de contenedor, sin
migrator. El DDL es tuyo. Consulta [Pruebas de base de
datos](database-testing.md) para el catálogo completo más los
ayudantes `execute_unprepared` / `fetch_one` / `fetch_all`.

## `#[suprnova_test]` - cuando quieres el azúcar

`#[suprnova_test]` es una macro de atributo que envuelve
`#[tokio::test]`, llama a `App::init()` + `App::boot_services()` para
que los tipos `#[injectable]` se resuelvan, y vincula una
`TestDatabase` nueva. Es azúcar sintáctico opcional sobre la forma
explícita de arriba, útil cuando un test resuelve servicios
registrados en el contenedor:

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

Si la función toma un parámetro `TestDatabase` (por nombre), la macro
vincula la base de datos nueva a ese nombre. Si no lo toma, la base
de datos se construye y se registra igualmente (así `DB::connection()`
funciona) - solo que no queda vinculada a una variable local.

Anula el migrator con la clave `migrator = …`:

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

Las claves desconocidas son un error de compilación (una errata
`migrtor = …` no mantendrá el migrator por defecto en silencio).

## `describe!` y `test!` - cuando agrupar ayuda

Para archivos de test donde la misma action tiene muchos casos, el
par `describe!` + `test!` al estilo Jest te da agrupación anidada y
una salida de fallo con nombre:

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // los grupos anidados se componen
        });
    });
});
```

`test!` acepta tres formas:

```rust
// Test async con parámetro TestDatabase
test!("creates a user", async fn(db: TestDatabase) { … });

// Test async sin base de datos
test!("calculates the right sum", async fn() { … });

// Test síncrono
test!("adds numbers", fn() { … });
```

El envoltorio de test con nombre enhebra el nombre del test a través
de la maquinaria de `expect!`, así que un fallo se ve así:

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

Sin `describe!`/`test!` obtienes la salida estándar de `panic!`. Con
ellos, la ubicación y el nombre legible del test encabezan el
mensaje.

## `expect!` - el catálogo de matchers

`expect!(value)` devuelve un wrapper `Expect<T>`. Los matchers están
tipados a `T` - llamar a `to_be_some()` sobre un `String` es un error
de compilación, no un pánico en tiempo de ejecución.

```rust
use suprnova::expect;

// Igualdad (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// Booleano
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // comprobación de Some(5)

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// Ordenamiento (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

Puedes usar `expect!` fuera de `test!` - el archivo/línea en el
mensaje de fallo viene de `concat!(file!(), ":", line!())`. El
encabezado del test con nombre es lo único que la macro no añade por
sí sola.

## `TestContainer` - fakes de DI que no se filtran

El capítulo del contenedor cubre la [búsqueda de tres
capas](container.md) en detalle. Para los tests, los dos puntos de
entrada son `TestContainer::fake()` (thread-local) y
`TestContainer::scope(…).await` (task-local).

### Thread-local, el caso común

`TestContainer::fake()` devuelve una guarda. Hasta que la guarda se
descarta, las escrituras de `TestContainer::singleton` / `bind` /
`factory` aterrizan en la capa de override thread-local y ensombrecen
el contenedor global:

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` instalan su propia guarda de
`TestContainer::fake` internamente - no las apilas salvo que estés
probando el registro mismo.

### Task-local, para runtimes `multi_thread`

La capa thread-local se establece en el hilo de sistema operativo que
llamó a `fake()`. Un runtime de tokio `multi_thread` puede migrar tu
future a otro hilo de trabajo a través de un `.await`, y el override
desaparece en silencio. `TestContainer::scope` resuelve eso vinculando
el override al future en su lugar:

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

Las subtareas lanzadas con `tokio::spawn` no heredan los task-locals
de tokio; usa `TestContainer::spawn` en su lugar - captura el
contenedor del alcance actual y lo vuelve a instalar dentro del
future lanzado:

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // ve el fake
    });
    let _client = h.await.unwrap();
})
.await;
```

### Por qué existe un refcount de `FAKE_GUARDS`

El contenedor thread-local es por test, pero Suprnova también tiene
un `ConnectionRegistry` global para el proceso, indexado por nombre
(`__read_replica__`, etiquetas de conexión personalizadas), que
sobrevive a un reinicio thread-local. Un `Drop` naíf llamaría a
`ConnectionRegistry::clear()` cada vez que *cualquier*
`TestContainerGuard` desapareciera - borrando la conexión con nombre
de otro test concurrente a mitad de su ejecución.

La solución es un `AtomicUsize` para todo el proceso (`FAKE_GUARDS`).
`fake()` lo incrementa; `drop` lo decrementa; solo la transición de
vuelta a cero limpia el registro con nombre. Dos tests en paralelo
que usan `__read_replica__` están a salvo: quien suelte su guarda al
final es quien hace la limpieza.

No llamas a esto desde un test - se ejecuta desde el `Drop` de
`TestContainerGuard`. Solo necesitas saber que está ahí si estás
depurando un síntoma de "la conexión con nombre desapareció a mitad
del test", que suele significar que un test hermano olvidó esperar a
que su propia guarda se soltara primero.

## Ayudantes de test para la clave de cifrado

Los tests que ejercitan casts cifrados (`casts = { secret =
AsEncrypted }` en un `#[model(...)]`), payloads firmados, o el
fallback de clave anterior del keyring necesitan una `APP_KEY`
instalada dentro del proceso. El framework incluye dos ayudantes
solo-para-test bajo la feature `testing`:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // idempotente; clave determinista de 32 bytes en cero
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … cifra y vuelve a leer …
}
```

`install_test_encryption_key` es idempotente - la fachada `Crypt`
subyacente se respalda en un `OnceLock`, así que la segunda llamada
es un no-op. La mayoría de los binarios de test de casts la llaman
desde cada test que toca un cast cifrado; la primera gana, el resto
son gratis.

Para tests de rotación (escrituras bajo la clave antigua, lecturas
bajo la nueva), usa la variante de keyring:

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

El ayudante de keyring devuelve `true` solo si la llamada realmente
instaló el anillo (el `OnceLock` estaba vacío). Para acuñar texto
cifrado bajo una clave arbitraria en un test de rotación, usa
`suprnova::crypto::_test_encrypt_with` en lugar de instalar dos veces.

Ambos ayudantes son `#[doc(hidden)]` en la capa de crypto y se
reexportan bajo el módulo `testing` - son solo para test y se saltan
la ruta de validación de `APP_KEY` de producción.

## La feature `testing` y los builds de producción

`suprnova` expone sus ayudantes de test (`Storage::fake()`,
`TestContainer`, `TestDatabase`, ganchos de rotación de crypto como
`_test_install_key`) tras una feature de Cargo llamada `testing`. La
feature está en el conjunto por defecto, así que las suites de test
consumidoras los obtienen gratis:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1" }

[dev-dependencies]
# `testing` está activada transitivamente vía la dependencia de arriba - nada extra.
```

Los ganchos son `#[doc(hidden)]` y llevan el prefijo `_test_`, así que
no son alcanzables desde código de aplicación idiomático ni siquiera con
la feature activada. La salvaguarda que de verdad sostiene esto es
`Server::from_config`: valida `APP_KEY` en **cada** arranque, no solo
cuando el keyring está sin inicializar. Una clave de test preinstalada
no puede saltarse esa comprobación - el arranque falla rápido si
`APP_KEY` falta o está mal formada, sin importar si algo dentro del
proceso preinstaló una clave.

Si prefieres que los ayudantes no se enlacen en absoluto en tu artefacto
de producción (defensa en profundidad), depende de `suprnova` con las
features por defecto desactivadas y activa solo lo que publiques:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.1", features = ["testing", "..."] }
```

Esto es un endurecimiento, no una corrección - la validación en el
arranque cierra el exploit real sea cual sea la postura que elijas.

### Por qué Suprnova diverge

El harness de test en PHP de Laravel consigue el aislamiento de tests en
paralelo casi gratis porque el runtime es monohilo por solicitud y los
tests hacen fork de un proceso nuevo por archivo. El binario de test de
Suprnova es un único proceso que ejecuta muchos `#[tokio::test]` de
forma concurrente en uno o más hilos de worker. Un único contenedor
global significaría que el fake de un test se filtra a la búsqueda del
siguiente en cuanto se solapen en un hilo de worker.

Por eso `TestContainer` tiene ambas variantes - thread-local para el
caso común de `current_thread`, task-local para `multi_thread`. El
borrado con refcount de `FAKE_GUARDS` sobre el `ConnectionRegistry`
global del proceso existe por la misma razón: el estado compartido que
no se puede hacer por test debe al menos saber que no ha de borrarse
mientras otro test sigue apoyándose en él.

El catálogo de matchers (`expect!`) es tipado porque Rust lo permite. El
`expect(x).toBeSome()` de Jest solo sabe en tiempo de ejecución si `x`
es un `Option`; el `Expect<T>` de Suprnova lo sabe en tiempo de
compilación, así que un matcher equivocado es un error de compilación,
no un test inestable.

## Dónde vive cada pieza

| Pieza | Fuente |
|---|---|
| Macro de atributo `#[suprnova_test]` | `suprnova-macros/src/suprnova_test.rs` |
| Proc-macros `describe!` / `test!` | `suprnova-macros/src/describe.rs`, `test_macro.rs` |
| Macro `expect!` + matchers `Expect<T>` | `framework/src/lib.rs` (macro), `framework/src/testing/expect.rs` (impls) |
| `TestDatabase::fresh` / `sqlite_memory` / ayudantes | `framework/src/database/testing.rs` |
| Macro `test_database!` | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` | `framework/src/testing/inertia.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| Fakes por superficie (Correo, Notificaciones, Cola, Bus, Eventos, Almacenamiento, HTTP) | submódulos `testing` por dominio - consulta [Simulación](mocking.md) |

## Ejecutar los tests

Se aplican las invocaciones estándar de cargo:

```bash
# Todo el espacio de trabajo
cargo test --workspace

# Un solo crate
cargo test -p suprnova

# Un solo test por nombre (coincidencia de subcadena)
cargo test create_user_persists_it

# Con salida de println! y dbg!
cargo test -- --nocapture
```

Suprnova no incluye su propio test runner; el framework se integra
con el de cargo. Los tests de base de datos se ejecutan en paralelo
por defecto - el contenedor thread-local y el SQLite en memoria por
test están diseñados exactamente para eso.

## Siguiente

- [Pruebas HTTP](http-tests.md) - conducir el pipeline completo de la
  solicitud a través de `handle_request`
- [Pruebas de base de datos](database-testing.md) - `TestDatabase`,
  factories en tests, sembradores en tests, pruebas de BD seguras en
  paralelo
- [Simulación y falsificaciones](mocking.md) - los siete fakes de
  superficie externa y los patrones que comparten
- [Contenedor de servicios](container.md) - la búsqueda de tres capas
  que anula `TestContainer`
- [Modelo de errores](error-model.md) - las formas de
  `FrameworkError` sobre las que harás aserciones
