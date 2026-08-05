# Siembra de datos

Los sembradores pueblan la base de datos con datos de fixture - las filas que la app
necesita antes de que un usuario real haya hecho nada. Piensa en una cuenta de admin
por defecto, la lista canónica de países, las publicaciones de demostración en el
entorno de staging, los 50 usuarios + 200 publicaciones de los que depende el bucle
de iteración de desarrollo local. Son el hermano en tiempo de ejecución de las
[migraciones](migrations.md): las migraciones construyen el esquema vacío, los
sembradores lo llenan.

Un sembrador es un tipo de tamaño cero que implementa el trait `Seeder`. El framework
mantiene un registro ordenado global al proceso; el comando `console db:seed`
por proyecto ejecuta cada sembrador registrado en el orden de registro, o uno
específico vía `--class=<Name>`. La mayoría de los sembradores terminan siendo unas
pocas líneas que llaman a una [factory de modelo](eloquent.md) y dejan que la factory
haga el trabajo de generación de filas.

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Regístralo una vez en el arranque:

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

Después:

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

Ese es el bucle completo. El resto de este capítulo cubre las convenciones de
organización, los patrones de composición del registro para casos más grandes, el
flag de selección `--class`, la integración con factories, la vía de escape
`without_events`, y la decisión de cuándo sembrar frente a migrar frente a usar
una factory.

## Escribir un sembrador

Un sembrador es un tipo unitario más un impl de `Seeder`. `name()` es la clave del
registro (también contra lo que compara `db:seed --class=<Name>`), y `run()` es la
fn async que realiza las inserciones.

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` se re-exporta en la raíz del crate, así que `use suprnova::Seeder` es
suficiente - no hace falta entrar en `suprnova::seed::Seeder`. `async_trait`
también se re-exporta (`use suprnova::async_trait`) porque el método del trait
devuelve un future y Rust todavía no permite `async fn` en traits sin él.

El tipo de retorno `FrameworkError` es el mismo envoltorio de error que usa
cualquier otra superficie async del framework; propagar el `?` desde una llamada
a una factory o desde un `Model::create` es la forma esperada. Consulta
[Modelo de errores](error-model.md) para la taxonomía completa.

### Convención de organización

Refleja el directorio `database/seeders/` de Laravel, pero en la raíz de origen:

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // impl de Seeder, registrado en bootstrap.rs
└── …
```

Genera el archivo a mano - no hay ningún generador `make:seeder` (esto es un
archivo con unas diez líneas de código repetitivo). Las factories a las que llama
el sembrador reciben el mismo tratamiento.

### Un sembrador que ejecuta otros sembradores

El idioma de Laravel de un único `DatabaseSeeder::run` de nivel superior que orquesta
las siembras por modelo funciona también aquí. En lugar de registrar cinco
sembradores pequeños en el arranque y confiar en su orden de registro, registra
un sembrador compuesto y llama tú mismo al resto:

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 50 usuarios primero - la factory de publicaciones genera
        // author_id en 1..=50, así que las referencias se resuelven.
        UserFactory::new().count(50).create_many().await?;

        // 200 publicaciones que referencian los ids de usuario anteriores.
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

Este es el valor por defecto recomendado. Mantiene el orden de dependencia
(`users` antes de `posts`) dentro del sembrador en lugar de esparcido por el
archivo de arranque, y `db:seed --class=BaseSeeder` es una invocación de un
solo objetivo que ejecuta todo el paquete.

Si se quiere encadenar sembradores por nombre en lugar de por llamada directa a
la factory, usa `seed::run_one` desde dentro del sembrador compuesto:

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

Los sub-sembradores siguen necesitando estar registrados en `bootstrap.rs` para
que `run_one` los encuentre.

## El registro de sembradores

El framework mantiene un mapa ordenado global al proceso (`IndexMap<String, fn() -> _>`)
de cada sembrador registrado. Tres controles lo gobiernan.

### `register::<S>()`

Añade un sembrador al registro bajo su `Seeder::name()`:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Dos cosas que hay que saber sobre el registro:

- **El orden importa.** `run_all` visita los sembradores en el orden en que se
  registraron. Si `B` necesita filas de `A`, registra primero `A`.
- **Volver a registrar un nombre lo reemplaza en el sitio.** La posición mantiene
  su lugar original, el puntero a función cambia. Esto es intencional - permite
  que una prueba vincule un sembrador de prueba sobre el real sin desplazar el
  orden. En código de producción, registra cada sembrador exactamente una vez en
  el arranque.

### `run_all()`

Ejecuta cada sembrador registrado en el orden de registro. Esto es lo que llama
la invocación desnuda `console db:seed`.

```rust
suprnova::seed::run_all().await?;
```

Se detiene ante el primer error. Los sembradores que ya se ejecutaron no se
revierten - `run_all` no envuelve una transacción alrededor del lote porque la
mayoría de los sembradores abarcan varios statements y muchos backends no anidan
transacciones con limpieza. Si se necesita semántica de rollback, abre la
transacción dentro del sembrador y mantén todo su trabajo dentro de ese alcance.

### `run_one(name)`

Ejecuta un sembrador con nombre sin ejecutar los demás. Este es el motor de
`db:seed --class=<Name>` y también es útil desde scripts puntuales:

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

Un fallo devuelve `FrameworkError::not_found("no seeder registered for \`X\`")`.
El comando de consola propaga eso a una salida distinta de cero y una línea en
stderr - sin no-op silencioso.

### `count()` e `is_registered(name)`

Dos helpers de lectura, ambos útiles en pruebas que afirman "el arranque conectó
los sembradores esperados":

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

Ambos devuelven cero / false ante un bloqueo de registro envenenado (tras
registrar un error), lo que mantiene las pruebas deterministas frente a un
pánico anterior en la cadena.

## El comando `db:seed`

`db:seed` es un comando de consola provisto por el framework - viene con el
framework y llega automáticamente al binario `console` del proyecto a través
del mismo registro de `inventory` que recoge los propios `#[command]`. Consulta
[Consola](console.md) para la mecánica del binario; esta sección cubre la
superficie específica de sembradores.

### Ejecutar todo

```bash
cargo run --bin console -- db:seed
```

Ejecuta cada sembrador registrado en orden. Sobre un registro vacío imprime una
advertencia a stderr (`db:seed: no seeders registered - nothing to run`) y
sale con cero - ese es el comportamiento correcto para "alguien ejecutó el
comando antes de registrar nada" y evita que las suites de prueba que no han
sembrado nada específico fallen.

### Ejecutar un sembrador

Tres formas aceptadas, en orden creciente de cuánto se sienten con la forma de
Laravel:

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

Las tres buscan el sembrador en el registro por nombre exacto y lo ejecutan. Un
nombre desconocido falla rápido:

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

Un flag malformado (`--class` sin valor siguiente, `--class=` con valor vacío,
`--class --force`) también falla rápido, con un diagnóstico que nombra la forma
esperada.

### Desde un binario ya compilado

En un despliegue containerizado o gestionado por systemd, el binario console vive
en `target/release/console` (o donde caiga el artefacto de release). La misma
sintaxis, sin `cargo` delante:

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

El binario console llama a `suprnova::console::dispatch_argv(std::env::args())`,
que enruta a través del mismo registro que `cargo run --bin console --`. No hay
ninguna ruta de despacho separada para los artefactos ya compilados.

## Componer con factories

Los sembradores casi siempre terminan llamando a [factories](eloquent.md). El
trait de factory sabe cómo construir una instancia aleatorizada de un modelo; el
sembrador secuencia las llamadas a la factory y cualquier cableado no
aleatorizable (credenciales de admin deterministas, filas de tablas unidas,
subidas de archivos).

El par mínimo de factory + sembrador:

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm cambia la PK a NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

El builder fluido vive en `FactoryBuilder<M>`; lo que se puede encadenar antes de
`create_many` coincide con Laravel:

```rust
// Construye una fila persistida con sobrescrituras:
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// Construye N filas persistidas, todas admins:
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// Estado condicional - aplica el closure solo cuando el flag está activo:
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` son los hermanos en memoria (sin insertar) para
pruebas unitarias que no quieren una ida y vuelta a la base de datos. Consulta el
capítulo [Eloquent](eloquent.md) para la superficie completa de factories
(incluyendo `prepend`, `Sequence`, y la macro `#[derive(Factory)]` que genera el
struct marcador a partir de un atributo `#[factory(model = "…")]`).

### La idempotencia es responsabilidad del sembrador

`run_all` no toma una instantánea ni envuelve una transacción; si un sembrador
inserta sin condiciones, volver a ejecutarlo produce duplicados. Las dos formas
estándar de hacer que un sembrador sea seguro de re-ejecutar:

- **Reiniciar primero.** El bucle de "borrar y resembrar" del desarrollo local
  normalmente hace `suprnova migrate:fresh && cargo run --bin console -- db:seed` -
  `migrate:fresh` elimina y reconstruye cada tabla, así que el sembrador siempre
  parte de vacío. Esta es la forma que usan la mayoría de los proyectos día a día.
- **Upsert / comprobar primero.** Para un sembrador que debe coexistir con datos
  ya existentes (una cuenta de admin por defecto en producción, la lista canónica
  de países), protege la inserción con una búsqueda o usa una consulta de upsert.

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## Silenciar eventos del modelo con `without_events`

Un sembrador que llama a `Model::create` en un bucle dispara cada evento del
ciclo de vida - `Creating`, `Saving`, `Created`, `Saved` - en cada fila. Eso
despierta a cualquier `Observer<M>` registrado, ejecuta cualquier oyente de
difusión en cola, y puede terminar encolando por accidente un centenar de jobs
en segundo plano que en realidad no se quieren. `seed::without_events` es el
análogo del `WithoutModelEvents` de Laravel:

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

Mientras el future interno está en espera, tanto la ruta de veto cancelable
(`dispatch_cancellable`) como la propagación posterior al evento
(`dispatch_after`) se cortan en corto hacia `Ok(())`. Los observadores quedan en
silencio, el difusor no se despierta, los jobs corriente abajo no se encolan.

El efecto tiene alcance de tarea - solo se silencia el trabajo realizado dentro
de `fut`. El trabajo concurrente en otras tareas (handlers de solicitud HTTP,
workers de cola corriendo en segundo plano, otros sembradores) sigue disparando
eventos con normalidad. Las llamadas anidadas se componen: un bloque
`without_events` interno hereda el flag externo.

### Las factories ya se saltan los eventos del modelo

Vale la pena saberlo porque cambia cuándo se recurre a `without_events`: las
factories persisten vía `ActiveModelTrait::insert` (el impl de `Persistable`
sobre el modelo de SeaORM), que no pasa por los métodos `create` / `save` del
trait `Model`. No hay ningún despacho de eventos de modelo que silenciar en una
ruta dirigida por factory. `seed::without_events` es para código que maneja
directamente el trait `Model` - típicamente porque se necesita la ergonomía de
forma en tiempo de ejecución que las factories evitan, o porque se está tocando
a mitad de la siembra un modelo que un observador debe atender en producción
pero no durante una carga de fixture.

En la práctica: si el sembrador es una pila de llamadas
`UserFactory::new().create_many()`, no se necesita `without_events`. Si es un
bucle escrito a mano de `User::create(attrs)`, probablemente sí.

## Usar sembradores en pruebas

El mismo registro que maneja el binario console se puede invocar desde un
`#[tokio::test]` - útil cuando se quiere un conjunto de fixtures conocido frente
a una prueba de integración:

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // Reinicia el registro para que las registraciones de una prueba anterior no se filtren.
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // Registra el sembrador que se quiere, ejecútalo, y afirma contra la
    // base de datos recién creada.
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …prueba de controlador contra los datos sembrados…

    seed::clear();
}
```

Dos notas sobre la forma de la prueba:

- `#[serial]` es obligatorio cuando la prueba muta el registro global al
  proceso - las pruebas paralelas que comparten el mismo registro entrarán en
  carrera. Añade `serial_test` como dependencia de desarrollo en el
  `Cargo.toml` del proyecto para obtener el atributo.
- `seed::clear()` es un helper `#[doc(hidden)]` solo para pruebas. No lo llames
  desde código de producción; el registro se construye una vez en el arranque
  y nunca se reinicia.

Consulta [Pruebas](testing.md) para las convenciones más amplias del harness de
pruebas (`#[suprnova_test]`, `TestContainer`, `TestDatabase::fresh::<Migrator>()`,
los fakes para cada superficie externa).

## Cuándo sembrar, migrar, o usar una factory

Estos tres patrones ponen filas en tablas. La decisión suele ser directa, pero
vale la pena nombrar las líneas divisorias explícitamente porque los equipos de
PHP a menudo las difuminan.

| Se quiere… | Usar |
|---|---|
| Que una columna exista | [Migración](migrations.md) |
| Una fila que debe existir para que la app arranque (el admin por defecto, la fila singleton de configuración del sitio, la lista canónica de monedas) | **Sembrador** - idempotente, corre en cada entorno, incluida producción |
| Un conjunto aleatorizado de filas para desarrollo local o staging (50 usuarios, 200 publicaciones, 1000 eventos) | Sembrador que llama a una factory |
| Una fila que necesita una prueba unitaria | [Factory](eloquent.md) llamada directamente dentro de la prueba |
| La forma de una fila | [Factory](eloquent.md) |

Los errores que se deben evitar:

- **No insertes datos desde una migración.** Las migraciones describen esquema,
  no estado. Una migración que inserta una fila por defecto correrá una vez
  sobre la base de datos de producción y luego nunca más - en el momento en que
  una columna cambia, se tiene una fuente de verdad bifurcada entre el historial
  de migraciones y el sembrador. Pon la inserción en un sembrador; si producción
  necesita la fila, ejecuta `console db:seed --class=DefaultsSeeder` como parte
  del despliegue.
- **No escribas datos de fixture en la prueba a mano.** Recurre a una factory.
  Cinco bloques `User::create(attrs!{ … })` en una prueba son cinco reescrituras
  en el momento en que se añade una columna NOT NULL. Un `UserFactory::new().create()`
  sobrevive.
- **No pongas datos de producción en un sembrador.** Un sembrador es para las
  filas que la aplicación necesita para funcionar, no para "aquí están los 8000
  registros históricos que estamos importando". Las importaciones son scripts
  puntuales (escribe un `#[command]` para ellas; consulta [Consola](console.md)).

### Por qué Suprnova diverge

Laravel viene con una clase `DatabaseSeeder` con un helper de caso especial
`call($seeders)` que el cargador de sembradores de Eloquent reconoce. Suprnova
no lo tiene - el registro es un `IndexMap` plano, cada sembrador es un igual, y
un sembrador compuesto llama a `seed::run_one(name)` (o simplemente llama
directamente a las sub-factories) para encadenar.

La razón es la misma compensación que se ve en otras partes de Suprnova: un
único registro genérico con una regla de orden es más fácil de razonar que una
jerarquía de clases con una raíz mágica. El patrón de Laravel funciona porque la
autocarga de clases de PHP y la reflexión estática de `make()` permiten que
`call([A::class, B::class])` encuentre e instancie esas clases por nombre; en
Rust estaríamos pidiendo al usuario que haga circular objetos de trait `dyn
Seeder`, lo que es más torpe que el registro de punteros a función que ya existe.

La convención del sembrador compuesto recupera la misma ergonomía - `BaseSeeder`
juega el papel que `DatabaseSeeder` juega en Laravel - sin necesidad de que el
framework consagre un nombre como especial.

## Registro en el arranque

Cada sembrador necesita una llamada a `seed::register` en `bootstrap.rs`, junto
al resto del cableado global al proceso (configuración, observadores,
supervisores, jobs de cola). El patrón tiene la misma forma que se usa en otras
partes del archivo de arranque:

```rust
// src/bootstrap.rs
pub async fn register() {
    // …configuración + vinculaciones del contenedor + cableado de auth…

    // Sembradores. El orden importa - run_all visita en orden de registro.
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …observadores, supervisores, jobs de cola…
}
```

Si se olvida registrar un sembrador, `console db:seed --class=X` falla con
"no seeder registered for `X`" - una señal clara en lugar de una omisión
silenciosa. Los helpers `seed::count()` y `seed::is_registered("…")` existen
precisamente para que una prueba pueda afirmar que el arranque registró cada
sembrador esperado.

Consulta [Arranque de la aplicación](bootstrap.md) para la estructura completa
del archivo y el orden en que el framework espera que se conecte cada
subsistema.

## Siguiente

- [Migraciones](migrations.md) - la mitad de esquema del par siembra/migración
- [Eloquent](eloquent.md) - modelos, factories, y la maquinaria `Persistable`
  a la que llama cada sembrador
- [Consola](console.md) - el binario `console` por proyecto que hospeda
  `db:seed` junto a los propios `#[command]`
- [Pruebas](testing.md) - `TestContainer`, `TestDatabase::fresh`, y el patrón
  `#[serial]` para pruebas que tocan el registro de sembradores
- [Modelo de errores](error-model.md) - qué es `FrameworkError` y cómo la
  forma `Result<(), _>` de `run` compone con el resto del framework
