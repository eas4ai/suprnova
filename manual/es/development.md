# Desarrollo

El bucle día a día de Suprnova es un comando: `suprnova serve`. Ejecuta
el backend de Rust, el frontend de Vite y un regenerador de tipos de TypeScript
en un único proceso, cada uno monitoreando los archivos correctos. Este capítulo
cubre el servidor de desarrollo, cómo encajan las piezas de recarga en caliente
y los comandos que ejecutarás a diario. Para la configuración inicial, consulta
[Instalación](installation.md); para el recorrido de directorios, consulta
[Estructura de directorios](structure.md).

## El servidor de desarrollo

Desde la raíz de un proyecto con andamiaje:

```bash
suprnova serve
```

La CLI imprime dos URLs y luego un flujo continuo de
salida con prefijo de cada proceso hijo:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765

[backend]  Compiling links v0.1.0
[backend]  Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.21s
[backend]  Running `target/debug/links`
[frontend] VITE v6.0.1  ready in 312 ms
[frontend]   ➜  Local:   http://localhost:5765/
[types]    Watching for Rust file changes to regenerate types
```

Accedes a la URL del backend (`127.0.0.1:8765`). Vite sirve tu JS/CSS
a través de la integración de desarrollo de Inertia - no visitas `:5765` directamente.
Presiona `Ctrl+C` una vez y la CLI cierra ambos procesos hijos de manera limpia.

### Flags

| Flag | Por defecto | Qué hace |
|---|---|---|
| `-p`, `--port <N>` | `8765` | Puerto del backend |
| `--frontend-port <N>` | `5765` | Puerto de Vite |
| `--backend-only` | off | Salta el proceso hijo de Vite (trabajo solo API) |
| `--frontend-only` | off | Salta el proceso hijo del backend (trabajo de componentes contra un backend en ejecución en otro lugar) |
| `--skip-types` | off | Salta el generador de tipos de TypeScript y su monitor |

Los mismos puertos se pueden establecer en `.env` a través de `SERVER_PORT` y `VITE_PORT`.
Un flag en la línea de comandos prevalece sobre `.env`.

### Qué verifica antes de iniciar

Antes de generar nada, `suprnova serve`:

1. **Verifica que estés en un proyecto.** Aborta con un error claro si no hay
   `Cargo.toml` (o sin `frontend/` cuando ejecutas el frontend).
2. **Genera los tipos de TypeScript una vez.** Escanea `src/` buscando
   `#[derive(InertiaProps)]` y escribe
   `frontend/src/types/inertia-props.ts`. Se salta con `--skip-types` o
   `--frontend-only`.
3. **Instala `cargo-watch` si falta.** La primera ejecución en una máquina nueva
   ejecuta `cargo install cargo-watch` por ti, luego continúa.
4. **Ejecuta `npm install` si `frontend/node_modules` falta.** Sin
   pasos de instalación manual en un clon reciente.

## Recarga en caliente

Tres monitores se ejecutan simultáneamente dentro de `suprnova serve`:

- **`cargo watch -x 'run --bin <pkg>'`** conduce el backend. Cualquier cambio `.rs`
  en el proyecto dispara una recompilación y un
  reinicio dentro del proceso. Los errores de compilación se imprimen en la secuencia `[backend]` y el
  binario anterior permanece activo hasta la siguiente compilación exitosa.
- **Vite** conduce el frontend. Los cambios de componentes, estilos y activos
  se reemplazan en caliente en la pestaña del navegador abierta sin una recarga completa.
- **Monitor de tipos basado en `notify`** reejecuta el escáner de InertiaProps
  siempre que un archivo `.rs` cambia. Se desactiva a los 500ms para que una ráfaga de
  guardados regenere `inertia-props.ts` una sola vez. La salida aparece bajo el
  prefijo `[types]`.

Ese tercero es la parte que no tienes que pensar: renombra un campo
en una estructura `#[derive(InertiaProps)]` y la interfaz de TypeScript correspondiente
la sigue en el siguiente guardado. La página Svelte/React/Vue recoge
el nuevo tipo inmediatamente. Sin necesidad de invocación `suprnova generate-types`
durante el desarrollo normal.

### Por qué Suprnova diverge

La mayoría de pilas de Rust web hacen que la recarga en caliente sea tu problema - elige tu propio
monitor de archivos, escribe tu propio envoltorio de reinicio, ejecuta Vite en una
terminal separada. La mayoría de pilas de Laravel hacen que los tipos de TypeScript sean tu problema -
declárelos en dos lugares (PHP y TS) y manténlos sincronizados.
`suprnova serve` ejecuta ambos monitores, más el generador de tipos que
mantiene tus tipos de frontend honestos, como un proceso supervisado. El
runtime de Tokio hace que "muchas cosas a la vez" sea lo suficientemente barato como para que un bucle de
desarrollo pueda gastarlo libremente.

## Comandos día a día

El puñado que ejecutarás cada hora:

```bash
suprnova serve                    # inicia dev (backend + Vite + monitor de tipos)
suprnova make:controller orders   # genera el andamiaje de un controlador
suprnova make:migration add_idx   # genera el andamiaje de una migración
suprnova db:sync                  # ejecuta migraciones, regenera entidades de SeaORM
suprnova migrate:status           # ve lo que se ha aplicado
suprnova migrate:fresh            # suelta tablas + reejecuta desde cero
suprnova key:generate --show      # rota APP_KEY
cargo run --bin console <cmd>     # cualquier handler de consola anotado con `#[command]`
cargo test                        # ejecuta el conjunto de pruebas
```

`db:sync` es el atajo de desarrollo para "migración + regeneración de entidades en un
paso." En producción usas `suprnova migrate` simple porque no
quieres que la regeneración suceda en una máquina de release. La superficie del generador completo
está en [Generadores de código](cli-generators.md) y los
verbos de migración están en [Migraciones](migrations.md).

## Depuración

### Registro de eventos

Suprnova usa `tracing` de punta a punta. Filtra lo que se imprime con
`LOG_LEVEL` (la misma sintaxis que `EnvFilter` de `tracing-subscriber`):

```bash
# Salida de framework detallada
LOG_LEVEL=debug suprnova serve

# Silencia hyper pero detalla tu crate
LOG_LEVEL=info,my_app=debug,hyper=warn suprnova serve
```

El formato de salida se controla mediante `LOG_FORMAT` (`pretty` para legible por humanos,
`json` para parseable por máquina). El valor predeterminado de dev es `pretty`. Consulta
[Observabilidad](observability.md) para la superficie de registro completa.

### Consultas SQL

Activa el registro por consulta con una variable de entorno:

```env
DB_LOGGING=true
```

Esto encamina cada consulta de SeaORM a través de `tracing` a nivel `info` para que puedas
ver exactamente qué se está ejecutando. Déjalo apagado en producción a menos que estés
persiguiendo una consulta lenta específica - el volumen se vuelve ruidoso rápidamente.

### Trazas de retroceso

Rust estándar:

```bash
RUST_BACKTRACE=1 suprnova serve
```

Un pánico en un handler es atrapado y convertido en una respuesta 500
estructurada; la traza de retroceso aterriza en tus registros sin derribar el servidor.
Consulta [Modelo de errores](error-model.md) para saber cómo funciona ese contrato.

## Pruebas en el bucle

```bash
cargo test                        # espacio de trabajo completo
cargo test -p my_app              # solo tu crate de aplicación
cargo test some_test_name         # filtra por nombre
cargo test -- --nocapture         # muestra salida de println!/tracing
```

La ejecución de pruebas es Cargo simple. Los ayudantes del lado del framework
(`#[suprnova_test]`, `TestDatabase`, `expect!`, falsificaciones para Mail/Queue/
Storage/etc.) están documentados en [Pruebas](testing.md) y
[Pruebas de base de datos](database-testing.md). Se ejecutan bajo el mismo
`cargo test` que ya conoces.

## Trabajar con el worker SSR

Si tu aplicación usa renderizado SSR de Inertia, querrás el worker SSR
junto a `suprnova serve` durante el desarrollo:

```bash
# Terminal 1
suprnova serve

# Terminal 2
suprnova ssr:start
```

`ssr:start` ejecuta el worker SSR incluido bajo Node, Bun o Deno
(`--runtime`). `ssr:check` verifica que un worker en ejecución sea alcanzable.
Ambos están documentados en el capítulo de frontend - consulta
[Frontend](frontend.md).

## Cuando algo se ve mal

Una breve lista de clasificación de los contratiempos más comunes del bucle de desarrollo:

- **Puerto ya en uso.** Otro `suprnova serve` todavía está activo, o un
  backend anterior se ha atascado. `lsof -i :8765` para encontrarlo, o simplemente pasa
  `--port 8001`.
- **`cargo-watch` sigue recompilando.** Algún editor está reescribiendo archivos
  al guardar (formateadores, linters con autofix). Desactiva el formato al guardar
  para el proyecto, o delimita tu monitor con patrones de `CARGO_WATCH_IGNORE`.
- **Los tipos de TypeScript no se actualizan.** Ya sea que se pasó `--skip-types`,
  o el monitor tropezó con un error de análisis `.rs`. Mira las
  líneas `[types]` - imprime una advertencia y continúa en lugar de
  fallar todo el servidor.
- **Errores de Vite pero el backend está bien.** Ejecuta `npm install` en
  `frontend/` una vez (la CLI lo hace en el primer servidor, pero si
  borras `node_modules` no lo volverá a hacer hasta que ese directorio esté
  ausente nuevamente en un inicio reciente).

Cualquier otra cosa, el capítulo [Errores](errors.md) cubre patrones de clasificación más profundos.

## Siguiente

- [Instalación](installation.md) - configuración inicial de la CLI y un
  proyecto
- [Inicio rápido](quickstart.md) - construye una pequeña aplicación de punta a punta
- [Estructura de directorios](structure.md) - qué contiene cada directorio
- [Generadores de código](cli-generators.md) - cada comando `make:*`
- [Pruebas](testing.md) - `#[suprnova_test]`, falsificaciones y la base de datos de
  pruebas
