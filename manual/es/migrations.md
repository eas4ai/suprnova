# Migraciones

Las migraciones describen cómo evoluciona el esquema - cada archivo es un pequeño struct de Rust con métodos `up()` y `down()` que el framework ejecuta en orden de timestamp. Úsalas cada vez que se cambien tablas, columnas, índices, o claves foráneas; ese cambio pasa del portátil a staging y a producción ejecutando el mismo comando migrate en cada lugar.

Las migraciones de Suprnova son migraciones de SeaORM por debajo. La CLI las genera, el `Migrator` las agrega, y `Application::migrations::<Migrator>()` las conecta al arranque de la app. Para la referencia completa por comando (flags, ejemplos de salida, códigos de salida) consulta [Referencia de migraciones de CLI](cli-migrations.md); este capítulo cubre qué poner *dentro* de los archivos.

## Crear migraciones

Genera un archivo de migración nuevo:

```bash
suprnova make:migration create_users_table
```

El generador escribe un archivo con timestamp bajo `src/migrations/` (creando el
directorio la primera vez) y lo registra en el `Migrator`:

```
src/migrations/
├── mod.rs                              ← el Migrator (gestionado por la CLI)
└── m20240115_120000_create_users_table.rs
```

El nombre del archivo es `m{AAAAMMDD}_{HHMMSS}_<nombre>.rs`; el orden es por
nombre de archivo, así que el prefijo de timestamp es lo que impone un orden de
aplicación determinista.

### Qué emite el generador

`make:migration create_users_table` produce este esqueleto:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    CreatedAt,
    UpdatedAt,
}
```

El generador infiere el nombre de la tabla a partir del nombre de la migración
(`create_X_table` → `X`, `add_Y_to_X` → `X`, `drop_X_table` → `X`). Cualquier
otro caso se convierte en el nombre literal.

### El Migrator

`src/migrations/mod.rs` reúne cada migración en un único `Migrator`
que `MigratorTrait` recorre. La CLI mantiene este archivo cuando se ejecuta
`make:migration`, así que rara vez se toca a mano:

```rust
pub use sea_orm_migration::prelude::*;

mod m20240115_120000_create_users_table;
mod m20240115_130000_create_posts_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240115_120000_create_users_table::Migration),
            Box::new(m20240115_130000_create_posts_table::Migration),
        ]
    }
}
```

Conecta el migrador al `main.rs` de la app para que `serve`, `migrate`,
`migrate:status`, `migrate:rollback`, y `migrate:fresh` vean todos la misma
lista:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)
        .bootstrap(my_app::bootstrap::bootstrap)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

El andamiaje escribe esto automáticamente al ejecutar `suprnova new`.

### Por qué Suprnova diverge

La mayor parte del framework oculta SeaORM deliberadamente - se escribe `#[suprnova::model]`
y `User::query().db_where(...)`, no `Entity::find().filter(...)`. Las migraciones
son el único lugar donde dejamos visible `sea_orm_migration::prelude::*`. Dos razones.

Primero, el DSL del constructor de esquemas es genuinamente bueno y re-aliasar cada
nombre en él (`Table`, `ColumnDef`, `Index`, `ForeignKey`, `Expr`, `ForeignKeyAction`,
`DeriveIden`, ...) compraría una línea de import más larga y nada más. Segundo,
los archivos de migración son Rust puro - el compilador de CI los verifica - y eso
atrapa más errores tipográficos que cualquier re-aliasado de DSL. Tratamos las migraciones
como esquema-como-código, y los nombres canónicos de SeaORM *son* el vocabulario del esquema.

Si alguna vez se necesita un tipo de SeaORM que el framework no ha re-exportado,
la vía de escape es `use suprnova::sea_orm;`. Casi nunca hace falta.

## Estructura de una migración

Cada migración tiene dos métodos:

```rust
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Aplica el cambio
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }

    // Revierte el cambio
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> { /* ... */ }
}
```

Ambos brazos devuelven `Result<(), DbErr>` - propaga los errores con `?` y el framework
convierte una migración fallida en una salida distinta de cero para que las canalizaciones
de despliegue se detengan.

## Operaciones de esquema

### Crear tablas

```rust
use sea_orm_migration::prelude::*;

async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Users::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Users::Email).string().not_null().unique_key())
                .col(ColumnDef::new(Users::Name).string().not_null())
                .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                .col(ColumnDef::new(Users::CreatedAt).timestamp().not_null())
                .col(ColumnDef::new(Users::UpdatedAt).timestamp().not_null())
                .to_owned(),
        )
        .await
}

// Define los identificadores de la tabla y de las columnas
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Name,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}
```

### Eliminar tablas

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_table(Table::drop().table(Users::Table).to_owned())
        .await
}
```

### Tipos de columna

| Método | Tipo en la base de datos | Notas |
|--------|---------------|-------|
| `integer()` | INTEGER | Entero de 32 bits |
| `big_integer()` | BIGINT | Entero de 64 bits |
| `small_integer()` | SMALLINT | Entero de 16 bits |
| `float()` | FLOAT | Punto flotante |
| `double()` | DOUBLE | Precisión doble |
| `decimal()` | DECIMAL | Punto fijo |
| `string()` | VARCHAR(255) | Cadena de longitud variable |
| `string_len(n)` | VARCHAR(n) | Cadena de longitud personalizada |
| `text()` | TEXT | Texto largo |
| `boolean()` | BOOLEAN | Verdadero/falso |
| `timestamp()` | TIMESTAMP | Fecha y hora |
| `date()` | DATE | Solo fecha |
| `time()` | TIME | Solo hora |
| `blob()` | BLOB | Datos binarios |
| `json()` | JSON | Datos JSON |
| `uuid()` | UUID | Tipo UUID |

### Modificadores de columna

```rust
ColumnDef::new(Column::Name)
    .string()
    .not_null()                                // Restricción NOT NULL
    .null()                                    // Permite NULL (por defecto)
    .default("value")                          // Valor por defecto
    .default(Expr::current_timestamp())        // Valor por defecto de función (p. ej. NOW())
    .unique_key()                              // Restricción UNIQUE
    .primary_key()                             // Clave primaria
    .auto_increment()                          // Autoincremento
```

Para claves primarias sustitutas, prefiere `big_integer().auto_increment().primary_key()`
en tablas reales - `INTEGER` (32 bits) está bien para tablas de consulta pequeñas, pero las
tablas `users`, `sessions`, y similares del andamiaje usan todas `BIGINT` porque
un contador de 4 bytes es el tipo de restricción que se lamenta tres años después.

## Añadir columnas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .add_column(
                    ColumnDef::new(Users::PhoneNumber)
                        .string()
                        .null()
                )
                .to_owned(),
        )
        .await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .drop_column(Users::PhoneNumber)
                .to_owned(),
        )
        .await
}
```

## Modificar columnas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .modify_column(
                    ColumnDef::new(Users::Name)
                        .string_len(500)  // Cambia VARCHAR(255) a VARCHAR(500)
                        .not_null()
                )
                .to_owned(),
        )
        .await
}
```

## Renombrar columnas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Users::Table)
                .rename_column(Users::Name, Users::FullName)
                .to_owned(),
        )
        .await
}
```

## Índices

### Crear índices

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_index(
            Index::create()
                .name("idx_users_email")
                .table(Users::Table)
                .col(Users::Email)
                .unique()  // Opcional: lo hace único
                .to_owned(),
        )
        .await
}
```

### Índices compuestos

```rust
manager
    .create_index(
        Index::create()
            .name("idx_posts_user_created")
            .table(Posts::Table)
            .col(Posts::UserId)
            .col(Posts::CreatedAt)
            .to_owned(),
    )
    .await
```

### Eliminar índices

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .drop_index(Index::drop().name("idx_users_email").to_owned())
        .await
}
```

## Claves foráneas

### Añadir claves foráneas

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Posts::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Posts::Id)
                        .integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Posts::UserId).integer().not_null())
                .col(ColumnDef::new(Posts::Title).string().not_null())
                .col(ColumnDef::new(Posts::Content).text().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_posts_user")
                        .from(Posts::Table, Posts::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}
```

### Acciones de clave foránea

| Acción | Descripción |
|--------|-------------|
| `Cascade` | Elimina/actualiza las filas hijas automáticamente |
| `SetNull` | Pone la clave foránea a NULL |
| `SetDefault` | Pone la clave foránea a su valor por defecto |
| `Restrict` | Impide eliminar/actualizar si hay referencias |
| `NoAction` | Similar a Restrict |

## Flujo de trabajo de migración

Un cambio típico pasa por cuatro pasos:

```bash
# 1. Genera el archivo (crea src/migrations/m{ts}_create_posts_table.rs
#    y actualiza src/migrations/mod.rs).
suprnova make:migration create_posts_table

# 2. Edita src/migrations/m{ts}_create_posts_table.rs para definir el esquema.

# 3. Aplica la migración.
suprnova migrate

# 4. Regenera los archivos de entidad de SeaORM a partir del esquema vivo para
#    que los modelos compilen contra la nueva forma. `db:sync` también ejecuta
#    cualquier migración pendiente primero (usa --skip-migrations para omitir
#    ese paso).
suprnova db:sync
```

`db:sync` escribe el pegamento de entidad autogenerado en `src/models/entities/<table>.rs`
y un stub editable por el usuario en `src/models/<table>.rs`. Volver a ejecutarlo actualiza
los archivos de entidad; los stubs del usuario se dejan intactos salvo que se pase
`--regenerate-models` (que los sobrescribe - mantén los métodos personalizados en otro
lugar o haz control de versiones antes de ejecutarlo).

### Auto-migración al servir

`suprnova serve` y `suprnova web:run` aplican cualquier migración pendiente antes de
abrir el socket HTTP. La política por defecto es de **fallo cerrado**: si `up()`
falla, el proceso aborta con salida distinta de cero antes del bind, así que una
migración rota nunca puede alcanzar el tráfico.

Dos vías de escape:

| Flag / entorno | Efecto |
|---|---|
| `--no-migrate` (en `serve` / `web:run`) | Omite el paso de auto-migración por completo. Útil cuando las migraciones se ejecutan desde un paso de despliegue separado. |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` | Vuelve a optar por el comportamiento heredado de registrar-y-continuar. El proceso sigue arrancando ante un error de migración. No se recomienda en producción. |

Los workers en segundo plano (`queue:work`, `workflow:work`, `schedule:run`) *no*
auto-migran - asumen que el esquema ya está en su lugar cuando arrancan, ya que
ejecutar migraciones desde N workers de forma concurrente entraría en carrera.

### Ejecutar migraciones en pruebas

`TestDatabase::fresh::<Migrator>()` levanta una base de datos SQLite en memoria
aislada, ejecuta cada migración, y vincula la conexión al contenedor de pruebas
para que `DB::connection()` y `#[inject]` la resuelvan:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn users_table_is_created() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // `db` se descarta al final de la prueba, limpiando el contenedor.
}
```

Consulta [Pruebas de base de datos](database-testing.md) para el patrón completo
(factories, seguridad en paralelo, elegir un driver real en vez de SQLite en memoria).

## Buenas prácticas

### Escribe siempre el reverso de las migraciones

Implementa siempre `down()` para permitir rollbacks:

```rust
// Bien: migración reversible
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.create_table(/* ... */).await
}

async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager.drop_table(/* ... */).await
}
```

### Usa nombres descriptivos

```bash
# Bien: describe el cambio
suprnova make:migration add_email_verified_to_users
suprnova make:migration create_order_items_table
suprnova make:migration add_index_to_posts_slug

# Mal: nombres vagos
suprnova make:migration update_users
suprnova make:migration change_table
```

### Un cambio por migración

Mantén cada migración centrada en un único cambio:

```bash
# Bien: migraciones separadas
suprnova make:migration create_categories_table
suprnova make:migration add_category_id_to_posts

# Evitar: varios cambios no relacionados en una sola migración
```

### Prueba las migraciones en ambos sentidos

Antes de hacer commit, verifica que ambas direcciones funcionen:

```bash
suprnova migrate           # Aplica
suprnova migrate:rollback  # Revierte
suprnova migrate           # Aplica de nuevo
```

## Comandos de CLI de un vistazo

| Comando | Descripción |
|---------|-------------|
| `suprnova make:migration <name>` | Crea una migración nueva |
| `suprnova migrate` | Ejecuta todas las migraciones pendientes |
| `suprnova migrate:status` | Muestra el estado de las migraciones |
| `suprnova migrate:rollback` | Revierte la última migración |
| `suprnova migrate:rollback --step 3` | Revierte las últimas 3 migraciones |
| `suprnova migrate:fresh` | Elimina todas las tablas y vuelve a ejecutar cada migración |
| `suprnova db:sync` | Ejecuta las migraciones y regenera los archivos de entidad |
| `suprnova db:sync --skip-migrations` | Regenera los archivos de entidad sin aplicar migraciones |
| `suprnova db:sync --regenerate-models` | Además sobrescribe los stubs de modelo editables por el usuario |

Consulta [Referencia de migraciones de CLI](cli-migrations.md) para la referencia
completa por comando (flags, ejemplos de salida, códigos de salida).

## Siguiente

- [Referencia de migraciones de CLI](cli-migrations.md) - referencia flag por flag para `migrate*` y `db:sync`
- [Base de datos](database.md) - configuración de conexión, transacciones, división lectura/escritura
- [Eloquent](eloquent.md) - la capa de modelo que alimentan las migraciones
- [Siembra de datos](seeding.md) - poblar tablas una vez que su esquema existe
- [Pruebas de base de datos](database-testing.md) - `TestDatabase::fresh::<Migrator>()` y los patrones seguros en paralelo
