# Pruebas de base de datos

El compañero específico de base de datos de [Pruebas](testing.md).
Donde ese capítulo cubre el harness de pruebas - `#[suprnova_test]`,
`describe!` / `test!`, `expect!`, y los fakes en proceso - este cubre
lo que cambia cuando una prueba necesita una base de datos: cómo
`TestDatabase` construye una, cómo funciona en realidad el aislamiento,
dónde se conectan las factories y los sembradores, y cuándo un SQLite
en memoria basta y cuándo no.

## Los dos constructores

Cada prueba de base de datos empieza construyendo un `TestDatabase`.
Dos constructores, dos intenciones.

### `TestDatabase::fresh::<Migrator>()`

Construye una base de datos SQLite en memoria, ejecuta el migrador de
extremo a extremo, y registra la conexión en el contenedor de pruebas
para que cualquier código que llame a `DB::connection()` o
`App::resolve::<DbConnection>()` la resuelva. Este es el valor por
defecto correcto para todo lo que toca un esquema real.

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // Consulta directamente cuando se quiera saltar la superficie del modelo:
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` es la implementación de `MigratorTrait` de la aplicación -
el mismo tipo que ejecuta el comando de producción `suprnova migrate`.
Al hacer pasar el migrador real por el esquema de pruebas se hace
imposible la desviación de esquema: una columna que el migrador olvidó
añadir no puede estar presente en silencio en la base de datos de
pruebas.

La macro `test_database!()` es azúcar sintáctico para el caso común
(`crate::migrations::Migrator`):

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// O con una ruta de migrador personalizada:
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

El mismo cableado de contenedor y registro, pero **no ejecuta ningún
migrador**. Úsalo cuando la prueba quiera control preciso sobre la
forma de las columnas - típicamente idas y vueltas de casts, pruebas
de la superficie SQL del constructor de consultas, o casos límite a
nivel de driver donde un migrador completo es excesivo o ruido:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// Luego se escribe directamente y se lee de vuelta con los helpers tipados:
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` es la base sobre la que se construye `fresh()` -
`fresh` la llama y luego ejecuta el migrador. Cualquier cosa que se
pueda hacer con `fresh` se puede hacer aquí; solo hay que traer el
propio DDL.

### `execute_unprepared`, `fetch_one`, `fetch_all`

`TestDatabase` re-exporta las tres formas de ejecución de SeaORM a las
que más se recurre en las pruebas, así los archivos de prueba no
tienen que traer `ConnectionTrait`:

| Método | Se usa para |
| --- | --- |
| `execute_unprepared(sql)` | DDL o DML sin placeholders. Devuelve `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | SELECT de una fila. Falla si hay cero filas |
| `fetch_all(sql, bindings)` | SELECT de todas las filas |

Los bindings son `Vec<sea_orm::Value>` - la misma forma que usa la
ruta de consultas de producción. El backend de la conexión (SQLite
para ambos constructores) se suministra automáticamente, así que un
placeholder `?` es correcto.

## Cómo funciona en realidad el aislamiento

El modelo de base de datos nueva por prueba es el mecanismo de
aislamiento. Cada llamada a `fresh()` o `sqlite_memory()` abre una
conexión `sqlite::memory:` nueva, que bajo SQLite es una instancia de
base de datos completamente separada - sin esquema compartido, sin
filas compartidas, ninguna otra prueba puede verla. No hay envoltorio
de transacción, ningún trait `RefreshDatabase` al que optar y ningún
rollback que recordar: la *siguiente* prueba obtiene una base de datos
limpia y vacía porque construye la suya propia.

Cuando el valor `TestDatabase` se descarta, ocurren tres cosas, en
este orden:

1. El `TestContainerGuard` retenido limpia el contenedor de pruebas
   thread-local, así que cualquier `App::get::<DbConnection>()`
   posterior ya no encuentra la conexión de pruebas.
2. Si este era el *último* `TestContainerGuard` vivo en el proceso, el
   [`ConnectionRegistry`](database.md#named-connections) con nombre se
   borra. (Un conteo de referencias sobre `FAKE_GUARDS` garantiza que
   el drop de una prueba interna no pueda borrar el nombre de una
   conexión del que todavía depende una prueba externa concurrente -
   la trampa permanente que motivó el conteo de referencias.)
3. La propia conexión SQLite se descarta, lo que destruye la base de
   datos en memoria.

Porque el estado se reconstruye en lugar de revertirse, el aislamiento
es más fuerte que el envoltorio `BEGIN`/`ROLLBACK`: no hay estado
confirmado que pueda sobrevivir por error, ninguna rareza de
transacciones anidadas, ninguna desviación de contador de secuencia
entre pruebas. El coste es que se paga por ejecutar el migrador una
vez por prueba (insignificante para SQLite con la mayoría de esquemas;
si llega a ser un coste real, consulta "Compartir una base de datos
migrada entre pruebas" más abajo).

## Por qué el pool está fijado a una sola conexión

Ambos constructores construyen la base de datos con
`max_connections(1)` y `min_connections(1)`. Esto es determinante para
`sqlite::memory:`, no una política genérica.

`sqlite::memory:` es una base de datos por conexión - cada conexión
*nueva* en el pool sería una instancia de SQLite separada y vacía. Un
pool de tamaño 2 significaría que la mitad de las consultas ven la
base de datos migrada y la otra mitad ven una vacía. Fijar el pool a
una sola conexión hace que cada consulta en la prueba caiga sobre la
misma base de datos en memoria contra la que corrió el migrador.

La consecuencia: una prueba que ejercita concurrencia real de
conexiones (dos transacciones compitiendo, enrutamiento de réplicas,
un worker de cola golpeando la base de datos mientras un handler de
solicitud hace lo mismo) necesita una base de datos real. Consulta
"Cuando SQLite en memoria no basta" más abajo.

## Factories en pruebas

Las factories producen instancias de modelo aleatorizadas y
(opcionalmente) las persisten. La ruta de persistencia resuelve
automáticamente la conexión de pruebas vinculada - no hay ningún
cableado del lado de la factory para las pruebas.

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // Solo en memoria: lo más rápido, sin ida y vuelta a la base de datos.
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // Persiste una + devuelve el modelo tras la inserción (id asignado).
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // Masivo: persiste 50 en secuencia.
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

Dos patrones que vale la pena conocer:

**Las inserciones de la factory se saltan los eventos del modelo.** El
impl de `Persistable` que respalda a `create()` / `create_many()`
escribe directamente a través de `ActiveModelTrait::insert` de
SeaORM - *no* pasa por la superficie `Model::create` que despacha
`Creating` / `Created` / `Saving` / `Saved`. Una prueba que afirma "ningún
observador se dispara mientras se construye el fixture" no necesita
nada especial; una prueba que afirma "el observador `Created` SÍ se
disparó" debe usar `Model::create(...)` (o `save()`) en lugar de una
factory.

**`create_many` no transacciona.** Las inserciones son secuenciales.
Si una fila posterior falla, las filas anteriores no se revierten.
Envuelve la llamada en tu propio `DB::transaction` si una prueba
requiere atomicidad:

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

Consulta [Eloquent → Fábricas](eloquent-factories.md) para la
superficie completa de factories (estados, secuencias, relaciones
`with`, `count`, `times`, `make_one` / `create_one`).

## Sembradores en pruebas

Los sembradores son funciones que se han registrado en el registro de
sembradores del framework bajo un nombre estable. Dos patrones para
manejarlos desde las pruebas, uno por cada eje de intención.

### Ejecutar un solo sembrador por nombre

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### Ejecutar el conjunto completo de sembradores del arranque

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // parte de un registro vacío conocido
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

Dos detalles de contrato importantes:

**El registro de sembradores es global al proceso.**
`seed::register::<S>()` inserta en un `RwLock<IndexMap>` con clave
`S::name()`. Una prueba que mute el registro debería llamar a
`seed::clear()` al entrar, registrar los sembradores que necesita,
ejecutarlos, y volver a llamar a `clear()` al salir - y la propia
prueba debería ser `#[serial_test::serial]` para que dos pruebas
paralelas no compitan por el registro. `#[suprnova_test]` **no**
registra sembradores automáticamente; solo la llamada explícita
`seed::register::<>()` en el propio `bootstrap.rs` o en el cuerpo de
la prueba los pone en el registro.

**Semillas dirigidas por modelo frente a semillas dirigidas por
factory.** Un sembrador que recorre `User::create(...)` en un `for`
dispara `Creating` / `Saving` / `Created` / `Saved` por cada fila e
invoca a cada observador registrado. Para siembras masivas donde esa
propagación no se desea, envuelve el bucle en `seed::without_events`:

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

El silenciamiento tiene **alcance de tarea** - solo se silencia el
trabajo realizado dentro del future; los handlers de solicitud
concurrentes y los workers de cola siguen disparando eventos con
normalidad. Las factories (`create_many`) ya se saltan la ruta de
eventos, así que `without_events` es innecesario a su alrededor.

Consulta [Siembra de datos](seeding.md) para la superficie de
escritura de sembradores y [Eloquent → Fábricas](eloquent-factories.md)
para la relación entre ambos.

## Pruebas de base de datos seguras en paralelo

`cargo test` ejecuta las pruebas en paralelo por hilo. La expansión
por defecto de `#[suprnova_test]` (que es `#[tokio::test]`, es decir,
un runtime `current_thread` por prueba) interactúa de forma segura con
esto por dos razones:

- **Cada prueba obtiene su propia conexión `sqlite::memory:`.** Las
  pruebas no comparten estado de base de datos.
- **La conexión vinculada vive en el `TestContainer`
  thread-local.** Las pruebas no comparten vinculaciones del
  contenedor.

Lo que no hay que pensar: `DB::connection()`, `App::resolve`,
persistencia de factory, escrituras de trait de modelo - todo esto cae
de forma transparente en la base de datos correcta de cada prueba.

Lo que sí *hay* que pensar:

| Superficie | Por qué es global al proceso | Mitigación |
| --- | --- | --- |
| `ConnectionRegistry` (`DB::register_named`, `__read_replica__`) | Un único `RwLock<HashMap>` compartido por el proceso | `#[serial_test::serial]` para cualquier prueba que registre o lea conexiones con nombre |
| El registro de sembradores | Un único `RwLock<IndexMap>` | `#[serial_test::serial]` + `seed::clear()` al entrar y al salir |
| Los registros de observadores / scopes de Eloquent | Con clave por `TypeId::<M>()` | Cada prueba debería usar un struct de modelo único, o ser `#[serial]` y llamar al helper `clear()` del registro |
| El log de consultas con nombre (`DB::enable_query_log`) | Un único ring buffer global al proceso | `#[serial]` si las aserciones leen el log |

El conteo de referencias del registro de conexiones hace esto más
seguro de lo que suena: una prueba que sostiene un `TestContainerGuard`
mantiene el registro vivo incluso cuando la guarda de una prueba
*hermana* se descarta. Aun así se quiere `#[serial]` para las pruebas
que de verdad mutan el registro, para que sus lecturas y escrituras no
puedan entrelazarse.

### Advertencia sobre el runtime multihilo

`#[suprnova_test]` se expande a `#[tokio::test]` con el runtime
`current_thread` por defecto, así que la ruta del contenedor
thread-local siempre funciona. Si se opta explícitamente por el
runtime multihilo en una prueba:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // PROBLEMA: las tareas generadas con `tokio::spawn` pueden correr
    // en un hilo de trabajo distinto del que construyó el
    // TestDatabase. No verán la vinculación del TestContainer
    // thread-local, y DB::connection() devolverá el valor del
    // contenedor global (de producción) o un error.
}
```

Dos soluciones, según lo que haga la prueba:

1. **Acceso directo a la conexión** - `db.conn()` sigue devolviendo la
   `&DatabaseConnection` correcta sin importar qué hilo de trabajo la
   lea. Si la prueba solo habla con la base de datos a través del
   handle `db` (no a través de `DB::connection()`), el runtime
   multihilo funciona bien.

2. **`TestContainer::scope`** - envuelve el cuerpo de la prueba en
   `TestContainer::scope(async { ... }).await` y vincula los fakes (y
   la conexión de base de datos) dentro de él. Ese alcance vincula el
   contenedor a la capa task-local, que se preserva a través de los
   awaits incluso cuando el runtime salta el future entre hilos de
   trabajo. Para subtareas generadas, usa `TestContainer::spawn` (no
   `tokio::spawn` puro) para que el contenedor task-local se capture y
   se reinstale dentro del future generado.

Consulta [Contenedor de servicios → Orden de búsqueda](container.md)
para la estratificación completa task-local / thread-local / global.

## SQLite en memoria frente a un Postgres / MySQL / MariaDB real

`TestDatabase` es intencionalmente solo-SQLite. El driver está fijado
a `sqlite::memory:`; no existe `TestDatabase::postgres()`,
`fresh_with_url()`, ni ninguna variante controlada por entorno. Para
la inmensa mayoría de la superficie de pruebas - CRUD de modelos, forma
del constructor de consultas, idas y vueltas de casts, carga de
relaciones, orden de disparo de observadores, semántica de eliminación
suave - SQLite en memoria es la herramienta correcta: cero
configuración, cero red, milisegundos por prueba, aislamiento
perfecto, ningún servicio externo que mantener vivo en CI.

Hay cuatro casos en los que SQLite en memoria no basta:

1. **SQL específico del driver.** Una consulta que use `LATERAL` de
   Postgres, operadores `JSONB`, `ON CONFLICT ... WHERE`, funciones de
   ventana de MySQL, o cualquier otra superficie específica de un
   dialecto no correrá sobre SQLite. La ruta de modelo+builder intenta
   mantenerse genérica, pero una prueba SQL en bruto que afirme una
   salida con forma de Postgres necesita Postgres.
2. **Concurrencia bajo contención real de conexiones.** SQLite en
   memoria es de una sola conexión (consulta "Por qué el pool está
   fijado a una sola conexión"). Las pruebas que hacen competir dos
   transacciones, ejercitan enrutamiento de réplicas de lectura bajo
   carga, o miden el reintento por deadlock necesitan un servidor
   multiconexión.
3. **Superficies vectoriales / NoSQL / temporales.** El driver
   `VECTOR` de MariaDB de Suprnova, la integración con Qdrant, la
   integración con Pinecone, y drivers no-SQL similares no se pueden
   modelar en absoluto en SQLite.
4. **Pruebas de humo de paridad con producción.** Un puñado de
   pruebas de "¿esto realmente funciona en la base de datos real a la
   que se despliega?", con puerta hacia CI, vale la pena mantenerlas
   incluso cuando la capa de pruebas unitarias es SQLite.

Para los cuatro casos el patrón es el mismo: salir por completo de
`TestDatabase`, construir un `DbConnection` contra una variable de
entorno con forma `DATABASE_URL` suministrada por el operador, poner
la prueba tras una puerta de entorno para que se salte cuando la
variable esté ausente, y marcarla `#[serial]` para que dos de ellas no
compitan por la base de datos real compartida. El patrón `MARIADB_URL`
en `framework/tests/vector_mariadb.rs` es el ejemplo canónico:

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Ejercita SQL específico de Postgres directamente contra `conn`.
}
```

La convención permanente: nombrar la variable de entorno según el
driver objetivo (`POSTGRES_TEST_URL`, `MYSQL_TEST_URL`,
`MARIADB_URL`), imprimir una línea de omisión para que quien ejecute
la suite localmente vea que la prueba se saltó (no que pasó en
silencio), y documentar la variable de entorno en el comentario de
documentación inicial del módulo de pruebas para que CI pueda
cablearla.

## Un ejemplo resuelto

El patrón completo de dogfooding de la aplicación, combinando todo
este capítulo:

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. Registro de sembradores vacío.
    seed::clear();

    // 2. Base de datos en memoria nueva con el migrador de la app.
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. Registra los sembradores que le importan a la prueba.
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. Ejecuta la siembra dentro de without_events para que la
    //    propagación de observadores no intente encolar jobs (no hay
    //    ninguna cola corriendo aquí).
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. Lee de vuelta a través de la superficie del modelo y de la
    //    conexión en bruto.
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. Ejercita la ruta del observador cancelable sobre un modelo nuevo.
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

El paso 5 es la parte que demuestra el cableado: la consulta del
modelo y el `fetch_one` en bruto están leyendo ambos la misma base de
datos en memoria - la superficie del modelo porque la búsqueda de
`DB::connection()` encontró la vinculación del `TestContainer`, el
`fetch_one` en bruto porque `db.conn()` devuelve directamente esa
misma conexión.

## Referencias cruzadas

- [Pruebas](testing.md) - el harness de pruebas, `expect!`,
  `describe!`, `test!`, fakes.
- [Base de datos](database.md#testing) - la sección de pruebas a
  nivel de superficie que introduce `TestDatabase`.
- [Eloquent → Fábricas](eloquent-factories.md) - sintaxis de
  definición de factories, estados, secuencias, relaciones.
- [Siembra de datos](seeding.md) - escritura de sembradores, orden,
  idempotencia.
- [Contenedor de servicios](container.md) - búsqueda task-local frente
  a thread-local frente a global, que decide a qué resuelve
  `DB::connection()` dentro de una prueba.
- [Simulación y falsificaciones](mocking.md) - `Storage::fake`,
  `Mail::fake`, `Queue::fake`, `Notification::fake`, y el patrón de
  vinculación de traits para intercambiar clientes HTTP fake y otras
  superficies externas.
- [Pruebas HTTP](http-tests.md) - manejar handlers a través de la pila
  de enrutamiento con un `TestDatabase` vinculado.
