# Consola

Cada proyecto de Suprnova incluye un binario `console` - el despachador
de comandos en tiempo de ejecución para todo lo que necesita los tipos
compilados de la app: sembradores de base de datos, podadores, tareas
de mantenimiento de una sola vez, cualquier cosa que construirías
con el `php artisan` de Laravel. Los comandos son o bien structs
tipados que llevan `#[derive(Command)]` (construidos sobre
`clap::Parser`) o fns async anotadas con `#[command]`; el framework los
recopila mediante `inventory` en tiempo de enlazado, así que añadir un
comando nuevo es un solo archivo sin ningún registro central que
editar. Este es el análogo de Suprnova a `php artisan` - el mismo
script, el mismo proceso, el mismo espacio de direcciones, termina
cuando el handler retorna.

## Inicio rápido

La forma recomendada usa `#[derive(clap::Parser, Command)]` para
argumentos tipados:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        println!("{prefix}, {}!", self.name);
        Ok(())
    }
}
```

Coloca eso en `src/commands/greet.rs`, añade `pub mod greet;` a
`src/commands/mod.rs`, y ejecútalo:

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# (ayuda por comando generada por clap, incluidos los flags tipados)
```

Sin ningún registro central que editar. `#[derive(Command)]` envía una
`CommandEntry { name, description, clap_builder, handler }` vía
inventario; el binario de consola llama a
`suprnova::console::dispatch_argv_with_init(argv, init)`, que construye
un único árbol de parser de clap a partir de cada entrada registrada,
ejecuta el closure `init` de bootstrap solo cuando coincide un
subcomando real, y enruta el `ArgMatches` analizado hacia el handler
correcto.

### El camino más simple: `Vec<String>` en crudo

Para comandos triviales que no necesitan argumentos tipados, el
atributo `#[command]` sobre una fn async también funciona:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    println!("pong");
    Ok(())
}
```

Por debajo, ambos caminos terminan en el mismo registro
`CommandEntry`; la forma en crudo simplemente usa un subcomando de clap
con un `trailing_var_arg` para capturar el argv dentro del
`Vec<String>`. Prefiere la forma tipada para cualquier comando con
argumentos - obtienes `--help` por comando, análisis de valores,
valores por defecto, y pares de flags cortos/largos sin escribir un
parser a mano.

## El binario de consola

`suprnova new` genera dos binarios en cada proyecto nuevo:

- **`<project>`** (`cmd/main.rs` o `src/main.rs`) - el servidor HTTP,
  iniciado por `cargo run` o `suprnova serve`. De larga duración; sirve
  hasta que se lo mata.
- **`console`** (`src/bin/console.rs`) - el despachador de comandos en
  tiempo de ejecución. De una sola vez; termina cuando el handler
  retorna.

El `main` del binario de consola es pequeño y predecible:

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Expone la versión de este proyecto vía `--version` / `--help`.
    // env! resuelve a la versión de la app del usuario, no a la del framework.
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokio se ejecuta en modo `current_thread` - no hay trabajo que
paralelizar entre núcleos en un comando de una sola vez, y el pool
de workers del runtime multihilo sería pura sobrecarga.

Dos cosas a notar:

- **El bootstrap es perezoso.** El closure que se pasa a
  `dispatch_argv_with_init` solo se ejecuta cuando clap hace coincidir
  un subcomando registrado real. `console --help`, `console
  --version`, el caso de subcomando ausente, y los caminos de error de
  análisis se lo saltan todos - así que `console --help` funciona en un
  checkout recién clonado que todavía no tiene `DATABASE_URL`
  establecida.
- **`main` no imprime errores.** `dispatch_argv_with_init` posee todo
  el stderr de cara al usuario - hace `eprintln` del mensaje de error
  del handler (a menos que el error sea silencioso, como un fallo de
  análisis de clap que clap ya imprimió) e imprime la propia salida de
  ayuda / versión / error de análisis de clap. `main` es pura
  traducción `Result → ExitCode`; añadir un `eprintln!` redundante
  duplicaría la impresión.

Si quieres que un comando en particular se salte por completo un paso
de bootstrap costoso, condiciona ese paso a una variable de entorno en
lugar de enhebrar un flag de "bootstrap perezoso" a través del
framework.

## Comandos integrados

El framework registra por sí mismo un pequeño conjunto de comandos.
Enlazar el framework en un proyecto los trae automáticamente.

| Comando       | Qué hace                              |
|---------------|-------------------------------------------|
| `db:seed`     | Ejecuta cada `Seeder` registrado en orden. Acepta `--class=<Name>` (o un posicional simple) para ejecutar un único sembrador con nombre, igual que `php artisan db:seed --class=UserSeeder`. |
| `model:prune` | Recorre el registro `PrunerEntry` y elimina por la fuerza cada fila que devuelva cada alcance `Prunable` / `MassPrunable` registrado. `--model=<Name>` restringe a un solo tipo; `--pretend` reporta el número de filas sin modificar ninguna. |
| `--help` / `-h` | Lista los comandos disponibles; la `--help` por subcomando la construye clap a partir de los argumentos tipados. |
| `--version`   | Imprime la versión registrada por `set_version` (típicamente el `CARGO_PKG_VERSION` de tu app). Se omite por completo si `set_version` nunca se llamó. |

`db:seed` ejecuta lo que hayas registrado en `bootstrap::register()`
con `suprnova::seed::register::<MySeeder>()`. Sobre un registro vacío
imprime una advertencia y devuelve `Ok(())` - invocar `db:seed` antes
de registrar sembradores es un error benigno del usuario, no un error
del programador.

`db:seed` informa del progreso en una ejecución dirigida usando
`suprnova::two_column_detail`, que renderiza un nombre, una línea de
puntos y un estado como una sola línea de 80 columnas. Tus propios
comandos pueden llamarlo para conseguir el mismo aspecto.

> Los demonios worker (`queue:work`, `schedule:run`, `schedule:work`,
> `schedule:list`, `workflow:work`) **no** están en el binario de
> consola. Viven en el parser de clap del binario de la app/servidor
> (el mismo binario que sirve HTTP). La CLI global `suprnova` se mete
> en `cargo run --quiet -- <name>` para esos. Consulta la [sección de
> asimetría](#asimetría-con-suprnova-migrate) más abajo.

## Definir comandos

Dos macros, un registro. Elige la que se ajuste a la forma del comando.

### `#[derive(Command)]` - argumentos tipados (recomendado)

Va sobre `#[derive(clap::Parser)]`. Los campos del struct son los
argumentos del comando; clap analiza el argv dentro del struct; el
framework llama a tu `TypedCommand::run(self)`.

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - tipados, validados por clap
        Ok(())
    }
}
```

Atributos:

| Atributo    | Requerido | Propósito                                       |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | sí | El nombre de invocación en la CLI (`"users:purge"`, `"mail:send"`, `"greet"`). |
| `#[console(description = "...")]` | no | Descripción de una línea mostrada en la ayuda de nivel superior. |
| `#[arg(...)]` (clap) | n/a | Los propios atributos de campo de clap para flags cortos/largos, valores por defecto, parsers de valores, etc. |

También obtienes gratis la ayuda por comando autogenerada por clap
(`console users:purge --help`).

### `#[command]` - `Vec<String>` en crudo (casos simples)

Para comandos que no toman argumentos o que solo consumen posicionales
como una lista, el atributo sobre una fn async ya basta:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

La función anotada debe ser `async fn(Vec<String>) -> Result<(),
FrameworkError>`. La macro preserva la función original, así que
también puedes llamarla directamente desde Rust - útil para tests
unitarios que no quieren enhebrar strings de argv a través del
despachador.

Los nombres en ambas formas admiten el namespacing al estilo Laravel:
`mail:send`, `queue:work`, `db:fresh`. Los dos puntos son puramente
cosméticos - es un string contra el que el despachador compara
`argv[1]`.

## `suprnova make:command`

El generador de la CLI coloca un stub ejecutable. El archivo
generado usa la **forma tipada** (`#[derive(Parser, Command)]` +
`impl TypedCommand`) - ese es el valor por defecto recomendado, y te da
gratis `--help` por comando:

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs (pub struct CacheClear con #[console(name = "cache:clear")])
# → a src/commands/mod.rs se le añade `pub mod cache_clear;` (se crea si falta)
```

El stub es ejecutable tal cual - `cargo run --bin console --
cache:clear` imprimirá `cache:clear: not yet implemented` y devolverá
`Ok(())` para que puedas cablearlo e iterar. Rellena los campos del
struct para argumentos tipados y sustituye el cuerpo de
`TypedCommand::run`.

Normalización de nombres:

| Entrada          | Archivo              | Nombre del comando   |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

Si la entrada contiene `:`, el namespace con dos puntos se preserva
literalmente. En caso contrario, el nombre de la fn de Rust queda en
snake_case y el nombre del comando en kebab-case.

Asegúrate de que `pub mod commands;` esté declarado en `src/lib.rs`
para que el envío del inventario sea alcanzable por enlazado desde el
binario de consola. El generador incluye esto para proyectos nuevos y
emite una advertencia bien visible si falta; si lo quitaste, el bloque
`inventory::submit!` del archivo nuevo compilará pero nunca acabará en
el registro.

### Por qué Suprnova diverge

El framework deliberadamente **no** crea un comando de CLI global
`suprnova` para tareas en tiempo de ejecución como `db:seed`. Un
binario global no puede cargar estáticamente los sembradores, las
factories o los fns async `#[command]` de tu app sin:

- delegar en `cargo run --bin app -- ...` (lento - compilación completa
  por invocación, lo que anula el propósito), o
- carga dinámica (demasiada complejidad para v1)

Así que el proyecto del usuario produce un binario `console`.
Ejecútalo directamente:

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravel resuelve el mismo problema con `php artisan` - un script por
proyecto que arranca el framework y despacha hacia los comandos
definidos por el usuario. PHP puede hacer esto dinámicamente porque el
código del framework vive junto al del usuario en tiempo de ejecución.
El modelo de compilación-y-enlazado de Rust descarta eso, así que
enviamos el despachador como una biblioteca (`suprnova::console::*`) y
dejamos que cada proyecto enlace su propio binario `console` de una
línea.

### Asimetría con `suprnova migrate`

Hay tres caminos de invocación de comandos distintos en un proyecto de
Suprnova, y la asimetría es **estructural** - no intentes unificarlos:

| Superficie de comandos                                   | Invocación                                              | Por qué                                                 |
|---------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------|
| `suprnova new`, `suprnova make:*`, `suprnova serve`, `suprnova key:generate`, … | Binario de CLI global (instalado vía `cargo install --git`) | Generadores de archivos y herramientas de andamiaje; no necesitan código del usuario. |
| `suprnova migrate`, `suprnova migrate:status`, `suprnova schedule:run`, `suprnova schedule:work`, `suprnova schedule:list`, `suprnova workflow:work` | La CLI global se mete en `cargo run --quiet -- <name>` contra el binario de la app/servidor | Demonios de larga duración y trabajo de esquema que posee el mismo parser de clap de `Application::run`. El `queue:work` del binario del servidor también vive aquí - `cargo run --bin <app> -- queue:work`. |
| `console db:seed`, `console model:prune`, `console <your-command>` | Binario `console` por proyecto (`src/bin/console.rs`) | Comandos de una sola vez que necesitan tipos del usuario (sembradores, comandos, modelos podables) compilados en el crate del usuario. |

La división es intencional. El binario del servidor ya necesita un
parser de clap para elegir entre `serve`, `migrate`, `queue:work`,
etc.; los demonios que comparten su ciclo de vida viven ahí. El binario
de consola existe para todo lo demás - de corta vida, definido por el
usuario, rico en tipos. Los comandos nuevos en tiempo de ejecución
pertenecen a `#[command]` / `#[derive(Command)]` despachados por el
binario `console` del proyecto.

## Buenas prácticas

### Mantén los handlers pequeños; alcanza los servicios compartidos a través del contenedor

Un `#[command]` es el envoltorio con forma de CLI; la lógica de negocio
debería vivir en una `Action`, un servicio, o un método sobre un
modelo. El handler analiza los argumentos, resuelve el servicio desde
el contenedor, y reenvía. Eso mantiene la misma lógica comprobable
desde un test unitario, una ruta HTTP, y la consola.

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` devuelve `Result<T,
FrameworkError::ServiceUnresolved(_)>` - la variante con `?` de
`App::get` (que devuelve `Option`). Consulta [Contenedor de
servicios](container.md) para la superficie completa.

### Usa namespaces para comandos relacionados

Agrupa con `:`: `mail:send`, `mail:retry`, `mail:queue:work`. El
despachador lo trata como opaco, pero los humanos escanean `mail:*`
mejor que `send-mail`, `retry-mail`, `mail-queue-work`.

### No imprimas datos estructurados - devuélvelos

Los handlers de consola imprimen en stdout para salida legible por
humanos. Si una herramienta downstream necesita consumir la salida,
escribe una variante `console <name> --json` que emita JSON legible
por máquina hacia stdout y una línea de estado hacia stderr. No hagas
que el camino legible por humanos sea responsable de ambas audiencias.

### Trata los códigos de salida como el contrato

`FrameworkError` → `ExitCode::FAILURE` es el único camino de fallo. No
hagas `std::process::exit(custom_code)` desde dentro de un handler -
devuelve `Err(...)` y deja que el `main` del binario traduzca. Las
herramientas futuras (gates de CI, workers supervisados) solo tienen
que leer el código de salida.

## Referencia

| Símbolo                                    | Propósito                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command` (derive)              | Registra un struct que deriva `clap::Parser` como un comando de consola tipado. Se combina con `TypedCommand`. |
| `suprnova::TypedCommand` (trait)          | Trait con `async fn run(self) -> Result<(), FrameworkError>` - el cuerpo de un comando tipado. |
| `suprnova::command` (atributo)           | Registra una fn async que toma `Vec<String>` como un comando de consola de argumentos en crudo. |
| `suprnova::console::dispatch_argv(argv)`  | Construye el árbol de parser de clap a partir de cada entrada registrada, analiza el argv, enruta hacia el handler. Sin init perezosa - conveniente para tests y llamadores programáticos. |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | Igual que `dispatch_argv` pero ejecuta el closure `init` entre el análisis de argv de clap y el handler emparejado. El init solo se dispara cuando coincide un subcomando real - los caminos de `--help` / `--version` / error de análisis se lo saltan. Esto es lo que usa el binario `console` con andamiaje. |
| `suprnova::console::set_version(&'static str)` | Registra el string de versión expuesto vía `--version` y en `--help`. Llámalo una vez al inicio de `main`. El primer registro gana. |
| `suprnova::console::find(name)`           | Busca un comando registrado por nombre exacto.   |
| `suprnova::two_column_detail(left, right)` | Renderiza un nombre, una línea de puntos y una palabra de estado como una sola línea de progreso de 80 columnas. Refleja el `$this->components->twoColumnDetail(...)` de Laravel. |
| `suprnova::console::list()`               | Todos los comandos registrados, ordenados por nombre.      |
| `suprnova::CommandEntry`                  | Registro de inventario: `{ name, description, clap_builder, handler }`. Enviado por ambas macros. |
| `suprnova::CommandHandler`                | El tipo de puntero a función del handler: `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`. |
| `FrameworkError::silent()` / `.is_silent()` | Construye / detecta un error que el despachador NO imprimirá en stderr. Se usa internamente para suprimir impresiones dobles cuando clap ya escribió un error de análisis en la terminal. |

## Siguiente

- [Arranque de la aplicación](bootstrap.md) - qué se ejecuta dentro del closure de `dispatch_argv_with_init`
- [Contenedor de servicios](container.md) - `App::resolve` vs `App::get`, y cómo un handler alcanza los servicios compartidos
- [Siembra de datos](seeding.md) - qué invoca en realidad `db:seed`
- [Eloquent](eloquent.md) - `Prunable`, `MassPrunable`, y cómo `model:prune` recorre el registro
- [Programación de tareas](scheduling.md) - la asimetría: los demonios del planificador viven en el binario de la app, no en la consola
