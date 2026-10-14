# Migraciones de CLI

La CLI de desarrollador `suprnova` se mete en el binario de tu
aplicación para controlar el ejecutor de migraciones de SeaORM, así
que el mismo conjunto de migraciones se ejecuta tanto si lo lanzas
desde una terminal de desarrollador, desde CI, o implícitamente al
arrancar el servidor. Usa estos comandos para escribir archivos de
migración, aplicarlos, revertirlos, y mantener tus entidades de
SeaORM generadas sincronizadas con el esquema.

Para la API de creación de esquemas (tipos de columna, índices,
claves foráneas, el `MigrationTrait` completo), consulta
[Migraciones](migrations.md). Para insertar datos de prueba después
de que el esquema aterrice, consulta [Siembra de
datos](seeding.md).

## make:migration

Genera un archivo de migración nuevo bajo `src/migrations/` y lo
conecta al `Migrator` en `src/migrations/mod.rs`.

```bash
suprnova make:migration <name>
```

`<name>` se normaliza a snake_case. El generador reconoce los
patrones de nomenclatura estándar y los usa para elegir el enum
`DeriveIden`:

- `create_<table>_table` - genera el andamiaje de un cuerpo
  `create_table`
- `add_<column>_to_<table>` - genera el andamiaje de un stub para
  `alter_table`
- `drop_<table>_table` - genera el andamiaje de un cuerpo
  `drop_table`
- cualquier otra cosa - usa el nombre como identificador de la tabla

### Ejemplos

```bash
suprnova make:migration create_users_table
suprnova make:migration add_email_to_users
suprnova make:migration drop_legacy_sessions_table
```

### Archivo generado

El archivo se escribe en
`src/migrations/m{YYYYMMDD}_{HHMMSS}_<name>.rs` (por ejemplo
`m20260530_142301_create_users_table.rs`) y se añade al vec
`Migrator::migrations()`.

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

Edita el archivo generado para declarar tus columnas, índices, y
restricciones. Consulta [Migraciones](migrations.md) para la
superficie completa del constructor de esquemas.

## migrate

Ejecuta cada migración pendiente en `src/migrations/`.

```bash
suprnova migrate
```

La CLI se mete en `cargo run -- migrate` para que el ejecutor
`Application` de tu app haga el trabajo - el mismo binario, el mismo
`Migrator`, la misma conexión de base de datos que usaría `serve`.

```
Running migrations...
Migrations completed successfully!
```

El camino de serve / web:run ejecuta `migrate` automáticamente antes
de enlazar el socket, a menos que te la saltes con `--no-migrate` o
establezcas `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true` para seguir
adelante a pesar de un fallo. Un error de migración durante la
auto-migración sale con código distinto de cero antes de que el
servidor arranque; consulta `framework/src/app/mod.rs` para el
contrato fail-closed.

## migrate:status

Imprime el estado aplicado/pendiente de cada migración.

```bash
suprnova migrate:status
```

```
Estado de las migraciones:
...tabla con formato de SeaORM de migraciones aplicadas/pendientes...
```

El cuerpo del reporte viene de `MigratorTrait::status` de SeaORM, así
que el formato exacto sigue la versión de SeaORM de la que depende
tu app.

## migrate:rollback

Revierte la última migración aplicada (o las últimas `N`).

```bash
suprnova migrate:rollback [--step <N>]
```

| Opción | Por defecto | Descripción |
|---|---|---|
| `--step <N>` | `1` | Número de migraciones a revertir |

```bash
# Revierte una migración
suprnova migrate:rollback

# Revierte las últimas tres
suprnova migrate:rollback --step 3
```

```
Rolling back 3 migration(s)...
Rollback completed successfully!
```

El `down()` de cada migración se ejecuta en el orden inverso de
aplicación. Un `down()` que falla sale con código distinto de cero y
deja el resto de la cadena intacto - no se intenta nada más.

## migrate:fresh

Elimina todas las tablas de la base de datos y vuelve a ejecutar cada
migración desde cero.

```bash
suprnova migrate:fresh
```

```
WARNING: Dropping all tables and re-running migrations...
Database refreshed successfully!
```

Esto destruye todos los datos de la base de datos conectada. Está
pensado para el desarrollo local y la configuración de tests, no
para ningún entorno donde los datos importen.

### La protección de producción

Fuera de producción se ejecuta de inmediato, sin preguntar - eliminar
una base de datos local es rutinario, y una confirmación que siempre
respondes de la misma forma te entrena para dejar de leerla.

Cuando `APP_ENV` se resuelve a producción, exige dos tipos distintos
de prueba:

```bash
suprnova migrate:fresh --force   # …luego escribe el nombre del entorno cuando se te pida
```

1. **`--force`** demuestra la intención en el momento en que
   escribiste el comando.
2. **Una confirmación escrita en una terminal interactiva** demuestra
   que hay un humano presente.

El requisito de terminal es el sentido de la segunda prueba. Sin él,
`echo production | suprnova migrate:fresh --force` en un script de
despliegue respondería la pregunta automáticamente, y la confirmación
sería solo otro flag más. Así que un stdin no interactivo se rechaza
incluso con `--force`.

Cualquier cosa que no sea el nombre exacto del entorno aborta antes
de eliminar una sola tabla.

La misma compuerta se aplica al propio subcomando del binario de tu
aplicación (`./app migrate:fresh --force`), que es el que realmente
ejecuta un despliegue de producción.

## db:sync

Regenera los archivos de entidad de SeaORM en `src/models/entities/`
a partir del esquema actual de la base de datos, y (cuando existe un
`src/bin/migrate.rs`) ejecuta primero las migraciones pendientes.

```bash
suprnova db:sync [--skip-migrations] [--regenerate-models]
```

| Opción | Descripción |
|---|---|
| `--skip-migrations` | Omite el paso de migraciones y solo regenera las entidades |
| `--regenerate-models` | Sobrescribe también los archivos `src/models/<table>.rs`, no solo `src/models/entities/<table>.rs` |

### Qué hace

1. (Opcional) Ejecuta las migraciones pendientes. El andamiaje por
   defecto no distribuye un `src/bin/migrate.rs`, así que este paso
   es un no-op e imprime `Migration binary not found, skipping
   migrations`. En un proyecto por defecto, ejecuta primero
   `suprnova migrate`, y luego `suprnova db:sync
   --skip-migrations`.
2. Se conecta a `DATABASE_URL`, introspecciona cada tabla de usuario
   (omitiendo `seaql_migrations` y cualquier nombre que empiece por
   `_`), y escribe un archivo de entidad por tabla en
   `src/models/entities/<table>.rs`.
3. Escribe un archivo de modelo delgado de cara al usuario en
   `src/models/<table>.rs` - pero solo si ese archivo todavía no
   existe, para que tus accesores, scopes, y hooks de observador
   escritos a mano sobrevivan.
4. `--regenerate-models` anula la protección del paso 3 y sobrescribe
   esos archivos de usuario. Úsalo cuando todavía no los hayas
   personalizado, o cuando tengas una copia de seguridad.

### Flujo de trabajo típico

```bash
# 1. Escribe una migración
suprnova make:migration create_posts_table
# (edita src/migrations/m..._create_posts_table.rs)

# 2. Aplícala
suprnova migrate

# 3. Regenera las entidades para que la tabla nueva sea alcanzable desde el código
suprnova db:sync --skip-migrations
```

### Por qué Suprnova diverge

Laravel tiene un único `artisan` global que posee cada comando del
framework, incluyendo `db:seed`. Suprnova divide esto en dos:

- La CLI de desarrollador `suprnova` (este capítulo) posee el
  andamiaje de proyectos, los generadores, y los comandos de
  migración. Se instala una vez por máquina de desarrollador vía
  `cargo install` y se mete en el binario de tu app para hacer el
  trabajo que necesita el `Migrator` de la app.
- Un binario `console` por proyecto, construido a partir del
  `src/bin/console.rs` de tu proyecto, posee `db:seed`, tus handlers
  anotados con `#[command]`, `queue:work`, `schedule:run`,
  `workflow:work`, y otras tareas de una sola vez que necesitan el
  bootstrap de tu app, las vinculaciones del contenedor, y los
  observadores registrados.

Los comandos de migración viven en la CLI de desarrollador porque
tienen una forma determinista que no depende de tu bootstrap. Todo lo
que necesita tu contenedor de servicios o tus sembradores registrados
vive en el binario console por proyecto. Consulta
[Consola](console.md) para la superficie completa de la consola.

## db:seed

No es un comando de la CLI `suprnova`. Ejecuta los sembradores a
través del binario console por proyecto:

```bash
cargo run --bin console -- db:seed
cargo run --bin console -- db:seed --class=UsersSeeder
```

El registro de sembradores, las reglas de orden, y la coincidencia de
`--class` se cubren en [Siembra de datos](seeding.md). El framework
distribuye `db:seed` como un comando de consola integrado - tu
andamiaje lo obtiene sin ningún cableado de tu parte, pero lo invocas
a través de `console`, no a través de `suprnova`.

## Resumen

| Comando | Qué hace |
|---|---|
| `suprnova make:migration <name>` | Genera el andamiaje de un archivo de migración nuevo y lo registra en `Migrator` |
| `suprnova migrate` | Ejecuta las migraciones pendientes |
| `suprnova migrate:status` | Muestra el estado aplicado/pendiente |
| `suprnova migrate:rollback [--step N]` | Revierte las últimas `N` migraciones (por defecto 1) |
| `suprnova migrate:fresh` | Elimina todas las tablas y vuelve a ejecutar cada migración |
| `suprnova db:sync [--skip-migrations] [--regenerate-models]` | Regenera las entidades de SeaORM a partir del esquema en vivo |
| `cargo run --bin console -- db:seed` | Ejecuta los sembradores registrados (console por proyecto, no la CLI `suprnova`) |

## Siguiente

- [Migraciones](migrations.md) - API del constructor de esquemas:
  tablas, columnas, índices, claves foráneas
- [Siembra de datos](seeding.md) - cómo escribir sembradores y el
  comando de consola `db:seed`
- [Consola](console.md) - el binario `console` por proyecto y los
  handlers `#[command]`
- [Base de datos](database.md) - conexiones, drivers, transacciones,
  el constructor de consultas
- [Descripción general de CLI](cli.md) - cada subcomando de
  `suprnova` de un vistazo
