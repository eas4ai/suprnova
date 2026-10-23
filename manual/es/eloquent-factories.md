# Fábricas de Eloquent

Las factories producen instancias de modelo aleatorizadas para tests
y sembradores. La forma es la de Laravel:
`UserFactory::new().count(10).create_many().await?`. El contrato es
un trait más un builder fluido, con un atajo `#[derive(Factory)]`
para el caso común donde el modelo ya tiene una representación
aleatorizada razonable.

Este capítulo cubre cómo definir factories a mano y por derive,
componer overrides en "estados" reutilizables, IDs deterministas vía
`Sequence`, el punto de enganche `Persistable` que impulsa `create`,
y la diferencia entre `make` (en memoria) y `create` (persistido).
Para el contexto de escritura de tests donde las factories son más
útiles, consulta [Pruebas](testing.md).

## El trait `Factory`

El trait tiene exactamente un método requerido:

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` devuelve un modelo completamente poblado con cada
campo aleatorizado hacia el valor por defecto que tenga sentido. El
trait no lleva estado por instancia - los implementadores suelen ser
marcadores de tamaño cero (`struct UserFactory;`) para que quien
llama pueda alcanzar la factory por nombre sin retener un handle.

El trait también provee dos puntos de entrada de builder con
implementaciones por defecto:

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1, sin overrides
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // azúcar para new().count(n)
```

Cualquier otro método que vayas a llamar (`with`, `count`, `make`,
`create`, `create_many`, …) vive en `FactoryBuilder<M>`.

## Definir una factory a mano

La forma mínima escrita a mano combina un struct marcador con un
impl de `Factory` que sabe construir una instancia. Normalmente
recurrirás a esto cuando el modelo no derive `fake::Dummy` - quizá
porque algunos campos necesitan una semilla determinista (IDs de
relación en un rango conocido) o la representación aleatorizada
necesita conocer reglas de negocio:

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0` es un placeholder - `persist_via_seaorm` cambia
            // las columnas de clave primaria a `NotSet` antes de
            // insertar, para que la base de datos asigne el id real.
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

Los campos `__eager` y `__pivot` son el estado auxiliar de precarga
y de pivot que la macro `#[suprnova::model]` inyecta en cada struct
de Eloquent. Déjalos siempre en su valor por defecto - los llena el
query builder, no las factories.

`next_seq()` puede ser lo que quieras - un `static AtomicU64`, una
`Sequence` (cubierta más abajo), o un contador thread-local. El
punto es que `definition()` corre de cero en cada llamada dentro de
`make_many` / `create_many`, así que cualquier unicidad que
necesites tiene que venir de un contador al que la función pueda
llegar.

## `#[derive(Factory)]` para el caso común

Cuando el propio modelo implementa `fake::Dummy` - vía
`#[derive(Dummy)]` o un `impl Dummy<Faker> for Model` escrito a
mano - el derive colapsa el marcador + impl en una sola línea sobre
el modelo:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

El derive emite `pub struct PostFactory;` como un tipo hermano y un
`impl Factory for PostFactory` cuyo `definition()` llama a
`Faker.fake::<Post>()`. La visibilidad de la factory refleja la
visibilidad del modelo - un modelo `pub` obtiene una factory `pub`,
un modelo `pub(crate)` obtiene una factory `pub(crate)`.

### Sobrescribir el nombre generado

Por defecto `#[derive(Factory)]` emite `<Model>Factory`. Sobrescribe
vía el atributo `name`:

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

El valor debe analizarse como un identificador de Rust válido -
`name = "User Factory"` o `name = "user-factory"` falla al compilar
con un error claro que señala el span. La macro emite
`pub struct <Name>;` literalmente, así que cualquier cosa que no
pueda ser un nombre de tipo no puede ser un nombre de factory.

### `Dummy` escrito a mano para una aleatorización más rica

`#[derive(Dummy)]` funciona para structs con campos de tipo
primitivo, pero no te da control sobre las distribuciones o los
invariantes entre campos. Para cualquier caso no trivial, escribe el
impl de `Dummy` a mano y combínalo con `#[derive(Factory)]`:

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* campos … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

El crate `fake` se reexporta como `suprnova::__fake` para que quien
lo consume no necesite una línea `fake = "…"` separada en
`Cargo.toml`. Los tipos comunes también se reexportan bajo la raíz
del crate: `suprnova::{Dummy, Fake, Faker}`.

### Por qué `#[derive(Factory)]` solo acepta structs planos

El derive rechaza enums, unions, y modelos genéricos con un error de
compilación claro. Los enums y las unions no tienen una
representación por defecto con sentido. Los genéricos forzarían una
decisión sobre cómo la factory parametriza su modelo - y no hay un
buen valor por defecto, así que el derive se niega a adivinar.
Escribe el `impl Factory` a mano para esos casos.

## El builder fluido

`Factory::new()` / `Factory::times(n)` devuelven un
`FactoryBuilder<M>`. Cada operación es encadenable; nada ocurre
hasta que llamas a un método terminal (`make`, `make_one`,
`make_many`, `create`, `create_one`, `create_many`).

### `count(n)` - cuántas instancias

```rust
let user = UserFactory::new().make();             // 1 usuario
let users = UserFactory::new().count(10).make_many();  // 10 usuarios
let same = UserFactory::times(10).make_many();   // idéntico
```

`count(n)` es ignorado por `make` / `create` (siempre uno) y
respetado por `make_many` / `create_many`. `times(n)` es solo azúcar
para `Self::new().count(n)` y coincide con el
`Factory::times($n)` de Laravel.

### `with(|m| { … })` - overrides por llamada

`with` registra un closure que corre contra cada instancia
producida después de `definition()`. Varias llamadas a `with` se
componen en orden de registro, así que un override posterior
sobrescribe a uno anterior sobre el mismo campo:

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

Los overrides se guardan como `Box<dyn Fn(&mut M) + Send + Sync + 'static>`
para que el builder siga siendo `Send` - importante para las rutas
async `create` / `create_many`, que sostienen el builder a través de
un `.await` sobre el insert de SeaORM.

### `prepend(|m| { … })` - valores por defecto que quien llama aún puede sobrescribir

`prepend` inserta un closure al **frente** de la cadena de
overrides, así que corre **antes** de cualquier otro `with(...)`.
Úsalo dentro de un método de estado cuando quieras dar un valor por
defecto que quien llama todavía pueda sobrescribir con un
`.with(...)` posterior:

```rust
impl UserFactory {
    /// Método de estado - valores por defecto de admin, quien llama
    /// aún puede personalizarlos.
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// Quien llama gana en `role` porque su .with() viene después de los prepends.
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

Este es el equivalente en Suprnova del `Factory::prependState` de
Laravel. Es la primitiva correcta específicamente para los métodos
de estado - `with` perdería frente al `.with(...)` de quien llama,
que es lo opuesto de lo que un valor por defecto debería hacer.

### `when(cond, |b| { … })` - encadenado condicional

`when` hace pasar un flag a través de una cadena sin romper el
estilo fluido. El closure recibe el builder, devuelve el builder.
Cuando `cond` es falso, el builder pasa sin cambios:

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

Refleja el `Conditionable::when($cond, $cb)` de Laravel. La firma
`FnOnce(Self) -> Self` significa que puedes hacer `await` dentro del
closure siempre que hagas `.await` antes de devolver el builder.

### Métodos terminales

| Método | Devuelve | ¿Persistido? |
|---|---|---|
| `make()` | un `M` | no |
| `make_one()` | un `M` (fuerza count = 1) | no |
| `make_many()` | `Vec<M>` de `count` elementos | no |
| `create()` | `Result<M, FrameworkError>` | sí |
| `create_one()` | `Result<M, FrameworkError>` (fuerza count = 1) | sí |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | sí |

`make_one` y `create_one` son útiles cuando un método de estado ha
fijado `count` internamente a otra cosa y quien llama quiere
exactamente un resultado:

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // valor por defecto razonable para fixtures
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// El test solo quiere uno - `create_one` descarta el count(5).
let admin = admins_in_org(42).create_one().await?;
```

## Estados: combinaciones preconfiguradas reutilizables

Suprnova no incluye una tabla de búsqueda `state("name")`. En su
lugar, los estados son métodos comunes sobre el marcador de tu
factory que devuelven un `FactoryBuilder<M>` preconfigurado. El
patrón se compone por herencia - cada método de estado devuelve el
mismo tipo `FactoryBuilder<M>`, así que puedes encadenar más métodos
sobre el resultado:

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// Variante inactiva - superpone un valor por defecto `active: false`.
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// Variante admin - superpone rol + email verificado.
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// Componible: admin inactivo.
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// Compón también en el sitio de la llamada - encadena más overrides libremente.
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

La elección de `prepend` es deliberada: los overrides de un estado
son *valores por defecto* que quien llama todavía puede reescribir.
Si quieres que la configuración de un estado sea innegociable, usa
`with` en su lugar - va al final de la cadena y gana.

### Por qué no hay una búsqueda `state("name")`

Un registro de estados indexado por nombre forzaría un cotejo de
strings en tiempo de ejecución para algo que el compilador puede
comprobar. Los métodos de estado te dan verificación en tiempo de
compilación (el typo `UserFactor::admn()` es un error duro) y
autocompletado completo del IDE. La composabilidad - encadenar
`Self::admin()` desde dentro de `inactive_admin()` - sale gratis.

## IDs deterministas con `Sequence`

`Sequence` es un contador monótono para inicializar campos únicos
por llamada. Cada llamada a `next()` devuelve 1, 2, 3, … de forma
atómica entre hilos:

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` es `const`, así que funciona como inicializador
`static`. El contador empieza en 0 e incrementa a 1 en la primera
llamada. Usa `reset()` entre tests si quieres un conteo limpio - la
macro `#[suprnova_test]` no hace esto por ti porque el framework no
puede saber qué sequences son tuyas:

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // empieza en 1 para este test
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` usa ordenamiento `SeqCst` - excesivo para "dame un id
único", pero mantiene el razonamiento trivial. Si una `Sequence`
alguna vez aparece en una ruta de ejecución frecuente, puedes
escribir la tuya con `Relaxed`.

## `Persistable`: el punto de enganche hacia tu almacenamiento

La familia de métodos `create` está disponible siempre que el
modelo implemente `Persistable`:

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

Un impl general en `factory::persist` cubre cada modelo SeaORM que
pueda `IntoActiveModel<ActiveModel>` - es decir, cada modelo que
emite la macro `#[suprnova::model]`. Sin boilerplate por modelo; si
`User` es un modelo, `UserFactory::new().create()` funciona.

El impl general toma `DB::connection()` e inserta. El `Self`
devuelto es lo que SeaORM entrega tras el insert - id asignado,
columnas por defecto resueltas, etc.

### Manejo de la clave primaria

Un impl `IntoActiveModel` de SeaORM marca cada campo - incluida la
PK - como `Set(value)`. Para los modelos producidos por una factory
la PK es un placeholder (`0` para `AUTO_INCREMENT i64`), así que un
insert directo colisiona en la segunda llamada con un fallo de
restricción UNIQUE.

`persist_via_seaorm` (el ayudante que respalda el impl general)
cambia cada columna de clave primaria a `NotSet` antes de insertar,
lo que deja que la base de datos asigne su propio id - la semántica
que las factories realmente necesitan:

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … bounds … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

Si realmente *quieres* asignar un id específico (test de replay,
restaurar un fixture por id), evita el ayudante y llama directamente
a `model.into_active_model().insert(db).await`.

### Persistir contra una conexión explícita

`persist_via_seaorm` toma la conexión como argumento. Útil cuando
quieres dirigir la persistencia contra una conexión que no es el
`DB::connection()` vinculado del framework - lo más habitual, un
handle `sqlite::memory:` específico en un test de integración:

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### Backends personalizados que no son SeaORM

Como el impl general apunta a cada tipo `ModelTrait`, no puedes
escribir `impl Persistable for MyOrm::Model` desde un crate externo
sin colisionar. Para persistencia personalizada que no es SeaORM
(Redis, Surreal, almacenes solo-blob), envuelve el modelo en un
newtype e impl `Persistable` sobre el envoltorio:

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

Una `Factory<Model = RedisCached<MyValue>>` obtiene entonces
`create` / `create_many` gratis.

## `make` frente a `create`: cuándo usar cada uno

`make` devuelve el modelo sin tocar la base de datos:

```rust
// Test unitario para una función pura - no necesita BD.
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` persiste y devuelve la versión posterior al insert:

```rust
// Test de integración - la acción necesita una fila real.
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

Recurre a `make` siempre que al test no le importe que la fila
exista. Recurre a `create` cuando vayas a consultar la fila de
vuelta, cuando una clave foránea necesite un id real, o cuando estés
poblando fixtures para un subsistema que lee la BD. Nota que
`create_many` persiste secuencialmente - si un insert posterior
falla, los inserts anteriores NO se revierten. `create` /
`create_many` pasan por el impl general de `Persistable`, que habla
directamente con el `DB::connection()` vinculado del framework - **no**
se suman a un alcance ambiental `DB::transaction(...)`. Si necesitas
atomicidad a través de un lote de inserts, baja al `Model::create(attrs!{...})`
del trait Model dentro del closure (esa ruta pasa por el mismo
ejecutor que respeta `CURRENT_TX`):

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## Comportamiento "after-creating"

Suprnova no incluye un callback con nombre `after_creating(|m| { … })`.
Dos patrones cubren los casos de uso para los que existe ese
callback en Laravel:

**1. La cadena - haz el seguimiento después de `create`/`create_many`:**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

Este es el patrón canónico cuando el id de un modelo necesita fluir
hacia un insert de seguimiento. `create` devuelve la fila persistida,
así que el id está disponible de inmediato.

**2. Observadores de modelo - reacciona sobre el ciclo de vida del
modelo, no sobre la factory:**

Usa los [Observadores de modelo](eloquent.md#observers) para
conectar comportamiento posterior al insert sobre el propio modelo en
vez de sobre la factory. El observador se dispara para
`User::create(...)`, `UserFactory::new().create()`, y cualquier otra
ruta de persistencia - exactamente lo que quieres cuando el
comportamiento es "cada vez que esta fila aterriza, haz X":

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

Los callbacks exclusivos de la factory invitarían a la divergencia
entre los inserts de test y los inserts reales. Los observadores se
mantienen consistentes en ambos.

## Sembradores

Las factories producen instancias; los sembradores las orquestan. Un
`Seeder` es un tipo de tamaño cero con un `run` async que sabe qué
poblar:

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // Usuarios primero - los posts referencian ids de usuario en 1..=50.
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

Registra el sembrador en `bootstrap.rs` para que el comando
`db:seed` del binario `console` del proyecto lo conozca:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Ejecuta a través del binario `console` del proyecto (cada app
generada incluye uno en `src/bin/console.rs`):

```bash
cargo run --bin console -- db:seed
```

Los sembradores corren en orden de registro. La idempotencia es
responsabilidad del sembrador - `run` no toma una instantánea ni
revierte, así que un sembrador que inserta sin condición produce
duplicados al volver a correrlo. Usa `migrate:fresh` seguido de
`db:seed` para partir de cero.

## Uniendo todo: un fixture de test completo

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // Arrange - un autor y un post en borrador que le pertenece.
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // Act.
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // Assert.
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

Tres patrones que vale la pena señalar:

- El `id` del autor fluye hacia el post a través de un closure
  `move` dentro de `.with(...)`. Las capturas son explícitas, lo que
  mantiene la relación visible en el sitio de la llamada.
- `create().await.unwrap()` es el idioma de test - se le permite al
  test entrar en pánico ante un fallo de setup porque un fixture roto
  es un test roto, no un modo de fallo elegante.
- Las factories se componen con el resto de la superficie de testing
  (`EventFacade::fake`, `Storage::fake`, `Mail::fake`, …) - ninguno
  de los fakes sabe nada sobre las factories, pero todo test que
  escribas los usará juntos.

### Por qué Suprnova diverge

Las factories de Laravel incluyen estados con nombre
(`->state('admin')`), sequences en tiempo de ejecución
(`->sequence(['name' => 'A'], ['name' => 'B'])`), y un callback
`afterCreating` registrado sobre la propia factory. Suprnova
descarta los tres y los reemplaza con primitivas con forma de Rust:

- **Los estados son métodos, no strings.** La comprobación de typos
  en tiempo de compilación y el autocompletado del IDE son ambos
  gratis; el único costo es "escribes `pub fn admin()` en vez de
  `protected function admin()`", lo cual no es ningún costo.
- **Las sequences son una primitiva separada.** `Sequence` hace una
  sola cosa (contador atómico) y es reutilizable fuera de la
  superficie de factory - puedes meter una en un generador de id de
  solicitud, un contador de paso de workflow, o un harness de test
  sin tener que explicar qué es.
- **After-creating está conectado al modelo, no a la factory.** El
  framework ya tiene [Observadores de modelo](eloquent.md#observers)
  exactamente para ese propósito. Añadir un mecanismo paralelo sobre
  la factory haría que el comportamiento en tiempo de test y el
  comportamiento en producción diverjan por construcción.

La superficie fluida - `count(10)`, `times(10)`, `with`, `prepend`,
`when`, `make`, `create`, `create_many`, `make_one`, `create_one` -
refleja directamente la de Laravel, así que la memoria muscular se
traslada sin necesitar un glosario.

## Siguiente

- [Pruebas](testing.md) - `#[suprnova_test]`, `TestDatabase`, las
  fachadas fake que combinan con los fixtures construidos por
  factory.
- [Eloquent](eloquent.md) - derivación de modelo, observadores, el
  pipeline de cast que corre cuando `create` persiste la salida de
  tu factory.
- [Migraciones](migrations.md) - el esquema contra el que necesitan
  existir tus factories; usa `migrate:fresh && db:seed` para un
  slate de fixture limpio.
- [Base de datos](database.md) - `DB::transaction`, enrutamiento
  multi-conexión, savepoints - a qué recurrir cuando `create_many`
  necesita atomicidad.
- [Contenedor de servicios](container.md) - cómo `App::resolve` y
  `App::make` encuentran los tipos de action y de servicio a los que
  llaman tus tests junto a las factories.
