# suprnova serve

`suprnova serve` ejecuta tu backend y el servidor de desarrollo de Vite
juntos, con recarga en caliente en ambos lados, además de
regeneración automática de tipos de TypeScript cada vez que tocas una
estructura `#[derive(InertiaProps)]`. Es el único comando que
mantienes abierto en una terminal mientras desarrollas.

```bash
suprnova serve
```

Ambos procesos envían su stdout a la misma terminal con prefijos
`[backend]` y `[frontend]` en color, así puedes distinguir quién dijo
qué. `Ctrl+C` los cierra a ambos de forma limpia.

## Uso

```bash
suprnova serve [OPTIONS]
```

| Opción | Por defecto | Descripción |
|---|---|---|
| `-p, --port <PORT>` | `8765` (CLI) / `$SERVER_PORT` (env) | Puerto HTTP del backend |
| `--frontend-port <PORT>` | `5765` (CLI) / `$VITE_PORT` (env) | Puerto del servidor de desarrollo de Vite |
| `--backend-only` | `false` | Omite el servidor de desarrollo de Vite |
| `--frontend-only` | `false` | Omite el backend, y solo ejecuta Vite |
| `--skip-types` | `false` | No regenera los tipos de TypeScript ante cambios en Rust |
| `--no-restart` | `false` | No vuelve a crear un proceso de desarrollo que se haya caído; desmonta toda la sesión (comportamiento anterior). |
| `--restart-tries <N>` | `5` | Deja de reintentar un proceso después de este número de caídas consecutivas. Se ignora con `--no-restart`, que ya termina la sesión en la primera caída. |
| `--timestamps` | `false` | Anteponer a cada línea de salida una hora `HH:MM:SS`. |
| `--json` | `false` | Emitir un objeto JSON por línea (NDJSON) en stdout en lugar de texto con prefijos; consulta [Salida JSON](#salida-json). Combinarlo con `--timestamps` no es un error: no tiene efecto adicional porque cada evento ya contiene su propia marca de tiempo. |

Los flags de la CLI tienen prioridad sobre las variables de entorno,
que a su vez tienen prioridad sobre los valores por defecto
integrados. Un `.env` generado con el andamiaje viene con
`SERVER_PORT=8765` y `VITE_PORT=5765`; verás esos valores en uso a
menos que los sobrescribas con `--port`.

## Ejemplos

### Por defecto - ambos servidores

```bash
suprnova serve
```

Salida:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

Visita `http://127.0.0.1:8765` en tu navegador. El backend sirve el
shell HTML de Inertia y reenvía las peticiones de activos hacia Vite,
así que no necesitas visitar la URL de Vite directamente.

### Puertos personalizados

```bash
suprnova serve --port 3000 --frontend-port 3001
```

O configúralos en `.env` y ejecuta sin flags:

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### Solo backend

```bash
suprnova serve --backend-only
```

Útil para trabajar en un proyecto solo API, o cuando tu frontend ya se
está ejecutando en otra terminal (o en otra máquina, o en una vista
previa desplegada).

### Solo frontend

```bash
suprnova serve --frontend-only
```

Útil para trabajar en la UI sin pagar el costo de una recompilación de
Rust en cada guardado, o cuando el backend se está ejecutando en otra
shell (o en Docker).

### Proyecto solo API

Un proyecto con andamiaje de `suprnova new --api` no tiene directorio
`frontend/`. Ejecuta `serve` exactamente igual que en cualquier otro sitio:

```bash
suprnova serve
```

`serve` no ve ningún `frontend/package.json`, se salta el panel de Vite y
la generación de TypeScript que lo alimenta, y ejecuta el backend.
`--frontend-only` sigue siendo un error en un proyecto así: pide el único
panel que no existe.

### Omitir la generación de tipos

```bash
suprnova serve --skip-types
```

Desactiva el monitor de regeneración de TypeScript. Usa esto cuando
gestiones `frontend/src/types/inertia-props.ts` a mano, o cuando
estés trabajando lejos de cualquier código de Inertia y quieras una
salida más silenciosa.

## Qué hace en realidad

Cuando ejecutas `suprnova serve`, la CLI:

1. Carga `.env` desde el directorio actual.
2. Resuelve los puertos del backend y del frontend (flag de la CLI →
   variable de entorno → valor por defecto).
3. Verifica que estés en un proyecto de Suprnova - `Cargo.toml` debe
   existir (a menos que se use `--frontend-only`), y `--frontend-only`
   necesita un directorio `frontend/` con un `package.json`. Un proyecto
   que no lo tenga se sirve solo con el backend en lugar de rechazarse.
4. Regenera los tipos de TypeScript a partir de cualquier estructura
   `#[derive(InertiaProps)]` que encuentre en `src/`, y los escribe en
   `frontend/src/types/inertia-props.ts`. Se omite cuando el proyecto no
   tiene frontend.
5. Instala `cargo-watch` mediante `cargo install --locked --version
   "^8.5" cargo-watch` si todavía no está en el PATH (una sola vez,
   con el aviso "Installing..."). Se omite bajo `--frontend-only`.
   La versión está acotada porque `serve` controla `cargo watch -x`,
   cuyo significado no está garantizado entre versiones mayores;
   `--locked` construye el árbol de dependencias que `cargo-watch`
   publicó, en lugar de resolverlo de nuevo en el momento de la
   instalación. Un comando que instala software como efecto
   secundario de iniciar un servidor de desarrollo no debería,
   además, elegir las versiones por ti.
6. Ejecuta `npm install` en `frontend/` si `node_modules` todavía no
   existe. Se omite bajo `--backend-only`, y cuando el proyecto no tiene
   frontend.
7. Lanza `cargo watch -x 'run --bin <package-name>'` para el backend.
   `cargo-watch` vuelve a ejecutar el binario cada vez que cambia un
   archivo `.rs`.
8. Lanza `npm run dev` en `frontend/` para Vite, lo que te da HMR
   para los componentes de Svelte/React/Vue y las clases de Tailwind.
   Se omite bajo `--backend-only`, y cuando el proyecto no tiene
   frontend.
9. Inicia cada proceso adicional declarado en el `Suprnova.toml` del proyecto
   (consulta [Procesos de desarrollo adicionales](#procesos-de-desarrollo-adicionales)
   más abajo), cada uno con su propio prefijo `[name]` - workers de cola,
   lectores de logs, cualquier otra cosa que de otro modo tendrías que
   gestionar en otra terminal.
10. Inicia un monitor de archivos sobre `src/` que vuelve a ejecutar el
    generador de tipos cada vez que cambia un archivo `.rs`, una vez
    que la ráfaga de guardados ha estado en silencio durante 500 ms.
    Se omite cuando el proyecto no tiene frontend, igual que la
    generación de tipos del arranque del paso 4. El antirrebote espera
    hasta el final de la ráfaga, así que una ráfaga -
    `cargo fmt`, formatear al guardar en varios archivos, un cambio de rama -
    se agrupa en una única regeneración que se ejecuta *después* de la
    última escritura, en lugar de una que se dispara con el primer archivo
    y se pierde el resto.
11. Reenvía el stdout/stderr de cada hijo a tu terminal con un prefijo
    `[name]` (`[backend]`, `[frontend]` o el nombre configurado del proceso),
    opcionalmente con marcas de tiempo mediante `--timestamps` - o, con
    `--json`, como eventos NDJSON (consulta [Salida JSON](#salida-json) más
    abajo).

`Ctrl+C` indica al gestor que active su flag de apagado, mate a todos los hijos
y salga. Si un hijo termina por sí mismo - un error de compilación de Rust
demasiado grave para que `cargo watch` se recupere, un proceso de Vite caído o
un proceso de `Suprnova.toml` que falló - se vuelve a iniciar después de una
breve espera (200 ms, que se duplica con cada caída consecutiva, con un máximo
de 5 s; un proceso que permanece activo 30 s reinicia la subida) en lugar de
derribar la sesión. Pasa `--no-restart` para recuperar el comportamiento
anterior: la salida de cualquier hijo cierra toda la sesión de inmediato.

Un proceso que sigue cayéndose no se reintenta para siempre: `--restart-tries`
(por defecto `5`) limita cuántas caídas consecutivas reintenta `serve` antes
de rendirse con ese proceso - 30 s de actividad nueva restablecen el contador,
igual que el retraso de espera. Rendirse imprime un mensaje accionable y deja
de reintentar *solo* ese proceso; los demás (y la sesión misma) siguen
ejecutándose, en línea con el valor por defecto `concurrently --restart-tries=5`
de Laravel. Consulta [Solución de problemas](#un-proceso-sigue-en-un-bucle-de-caídas).

### Por qué Suprnova diverge

Los usuarios de Laravel normalmente ejecutan `php artisan serve` para el
backend y `npm run dev` en otra terminal, y la mayoría de los equipos disimulan
la división de dos terminales con un `Procfile` y `foreman`/`overmind`.
Suprnova distribuye ese multiplexor como un comando de CLI de primera clase.
Obtienes una sola terminal, un solo `Ctrl+C`, arranque automático de la cadena
de herramientas (`cargo-watch`, `npm install`) y un puente Inertia tipado que
regenera `frontend/src/types/inertia-props.ts` sobre la marcha, de modo que tus
componentes de Svelte/React/Vue siempre ven la forma actual de las props sin
sincronización manual de tipos.

El comando `dev` de Laravel también ofrece modos `--tabs` y `--stream`, cada
uno de los cuales renderiza la salida mediante una pequeña TUI de Node
(`@laravel/multiplex`). Suprnova no incluye la TUI: la salida con prefijos en
una sola terminal es la norma en el ecosistema de herramientas de desarrollo
de Rust (`cargo watch`, `bacon`, `just`), y un registro de procesos con
prefijos de colores ya proporciona la señal de «qué proceso dijo esto» que
ofrece una TUI. El trabajo subyacente de `--stream` - un flujo de eventos en
tiempo real y programable - se incluye como `--json` (consulta [Salida JSON](#salida-json));
la TUI multipanel de `--tabs` es un no deliberado, no una carencia: otro modelo
de interacción y otra biblioteca que mantener entre terminales para un
problema que esta página ya resuelve. Consulta la fila correspondiente en
[Paridad](parity.md#what-we-won-t-ship-and-why).

## Recarga en caliente

**Backend.** `cargo watch -x 'run --bin <package>'` es el bucle.
Reconstruye y reinicia el servidor ante cada cambio `.rs` en el proyecto.
Las reconstrucciones en frío después de tocar un crate pesado pueden tardar
varios segundos; los cambios incrementales en un solo archivo suelen tardar
menos de un segundo.

**Frontend.** El HMR de Vite inyecta los cambios de componentes en el mismo
lugar sin una recarga completa, preservando el estado del componente. Las
clases de Tailwind se actualizan en vivo a través del monitor de Tailwind v4.

**Tipos de TypeScript.** Cada vez que cambia un archivo `.rs`, el monitor de
tipos vuelve a ejecutar el generador. Si aparecen nuevas estructuras
`#[derive(InertiaProps)]` (o las existentes cambian de forma), el
`frontend/src/types/inertia-props.ts` regenerado dispara el HMR de Vite para
el componente que las importa.

## Procesos de desarrollo adicionales

`suprnova serve` siempre ejecuta el backend y Vite, pero la mayoría de los
proyectos necesita mantener más de dos procesos: un worker de cola, un
lector de logs o un capturador de correo. Decláralos en `Suprnova.toml`
en la raíz del proyecto; `serve` los inicia, les antepone prefijos y los
reinicia automáticamente junto al backend y el frontend:

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

Cada entrada necesita `name` y `command`; `args` no tiene valores por
defecto y `color` recibe uno de green/yellow/blue/white según el orden de
declaración (o puedes elegir uno de los ocho colores `console`:
black, red, green, yellow, blue, magenta, cyan, white). Los nombres deben
ser únicos. `Suprnova.toml` es opcional; un proyecto sin él funciona
exactamente como antes.
### Por qué Suprnova diverge

Laravel registra procesos `dev` adicionales desde PHP -
`DevCommands::register($command, $name)`, normalmente en el `boot()` de un
proveedor de servicios - porque `php artisan dev` ejecuta un multiplexor desde
el mismo proceso que ya arrancó la aplicación. `suprnova serve` es un binario
separado de tu aplicación; nunca enlaza ni ejecuta tu código Rust, y solo lanza
`cargo watch` y `npm`. No hay un arranque de la aplicación al que conectarse,
así que el registro debe ser datos que lea la CLI, no una llamada que haga tu
código - de ahí `Suprnova.toml` en lugar de una API
`DevProcesses::register()`.

## Salida JSON

Pasa `--json` y `suprnova serve` escribe un objeto JSON por línea (NDJSON)
en stdout, en lugar de texto coloreado con prefijo `[name]`. Mientras está
activo no envía nada más a stdout, por lo que puedes canalizarlo a `jq` u
otro consumidor JSON orientado a líneas. Cada línea tiene un campo `type`:

| `type` | Campos | Significado |
|---|---|---|
| `started` | `ts`, `name`, `pid` | Se inició por primera vez un proceso (backend, frontend o una entrada de `Suprnova.toml`). |
| `output` | `ts`, `name`, `stream` (`"stdout"` o `"stderr"`), `line` | Una línea de salida de un hijo, transportada como campo en lugar de pasarla sin procesar. |
| `exited` | `ts`, `name`, `code` (anulable) | Un proceso terminó. `code` es `null` si una señal lo mató en lugar de devolver un estado. |
| `restart_scheduled` | `ts`, `name`, `delay_ms` | Un proceso caído se volverá a iniciar después de `delay_ms` (consulta el esquema de espera anterior). |
| `restart_succeeded` | `ts`, `name`, `pid` | La recreación programada tuvo éxito; el proceso vuelve a ejecutarse con un PID nuevo. |
| `gave_up` | `ts`, `name`, `tries` | El proceso se cayó `tries` veces consecutivas (`--restart-tries`) y `serve` dejó de reintentarlo. La sesión y los demás procesos siguen ejecutándose. |
| `types_regenerated` | `ts`, `artifact` (`"inertia_props"` o `"lang_keys"`), `count` | El monitor de archivos regeneró un artefacto TypeScript en respuesta a un cambio `.rs`/`.ftl`. |
| `shutdown` | `ts` | La sesión se está apagando. Siempre es la última línea. |

Por ejemplo, un fallo de Vite y su recreación se ven así:

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` se combina con `--timestamps` en lugar de entrar en conflicto:
combinarlos no es un error, pero `--timestamps` no tiene ningún efecto
adicional, porque cada evento ya lleva su propio campo `ts`.

Esta es una salida legible por máquinas que analizan otras herramientas -
los nombres de los campos y los valores de `type` no se cambiarán ni
eliminarán sin una nota en el changelog. Trata un `type` no reconocido o un
campo adicional inesperado como algo que se debe ignorar, no como un error,
para que una versión futura pueda ampliar el esquema sin romper tu consumidor.

## Solución de problemas

### Puerto ya en uso

```text
[backend] Error: Address already in use (os error 98)
```

Encuentra y mata el proceso, o elige otro puerto:

```bash
lsof -i :8765
kill -9 <pid>

# o
suprnova serve --port 8081
```

### La instalación de `cargo-watch` falla

La CLI ejecuta `cargo install cargo-watch` si todavía no está en el
PATH. Si esa instalación falla (sin red, entorno restringido),
instálalo manualmente una vez:

```bash
cargo install cargo-watch
```

Después de eso, `suprnova serve` lo encontrará y no intentará
instalarlo de nuevo.

### Las dependencias del frontend se atascan

Si `npm install` falla a mitad del arranque, corrige la causa (que el
registro de npm sea alcanzable, espacio en disco, el lockfile en buen
estado) y ejecútalo manualmente:

```bash
cd frontend && npm install
```

Luego vuelve a ejecutar `suprnova serve`. La CLI solo ejecuta `npm
install` automáticamente cuando falta `node_modules`, así que una
instalación manual exitosa le permite omitir ese paso.

### La regeneración de tipos no detecta los cambios

El monitor sondea cada 2 segundos (usando `notify` con un intervalo de
sondeo, elegido por fiabilidad multiplataforma en lugar de las
particularidades de inotify) y aplica antirrebote a la regeneración,
limitándola a una vez cada 500 ms. Si un cambio no aparece:

- Confirma que el archivo esté bajo `src/` (el monitor no recorre
  recursivamente `crates/`, `cmd/`, ni `migrations/`).
- Confirma que la estructura realmente tenga
  `#[derive(InertiaProps)]`.
- Reinicia `suprnova serve` y observa el mensaje de arranque
  `Generated N type(s)` - si ves `No InertiaProps structs found`, el
  escáner no encontró nada que emitir.

### Un proceso sigue en un bucle de caídas

Si un hijo - backend, frontend o una entrada de `Suprnova.toml` - no puede
iniciarse (código incorrecto, binario ausente, conflicto de puertos), se vuelve
a iniciar según el esquema de espera descrito arriba en lugar de detenerse.
Mira las líneas `[name]` justo antes de cada aviso «respawning in …ms» para
encontrar el error real (un `error[E…]` de rustc, un ENOENT, lo que haya
imprimido el hijo). Corrige la causa; el siguiente intento de inicio la
recogerá automáticamente. Para detener los reintentos y ver el fallo una vez,
vuelve a ejecutar con `--no-restart`: la sesión se desmontará con la primera
caída, igual que se comportaba `suprnova serve` antes de que existiera esta
funcionalidad.

Después de `--restart-tries` (por defecto `5`) caídas consecutivas, `serve`
deja de reintentar ese proceso por sí mismo e imprime un mensaje que lo nombra:

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

Los demás procesos y la sesión misma siguen ejecutándose. Corrige la causa y
vuelve a ejecutar `suprnova serve` para recuperar el proceso abandonado; no
necesitas reiniciar toda la sesión.

## Siguiente

- [Instalación](installation.md) - consigue la CLI en tu máquina
- [Inicio rápido](quickstart.md) - un recorrido completo de la
  primera app
- [Estructura de directorios](structure.md) - qué generó
  `suprnova new` con el andamiaje
- [Generadores](cli-generators.md) - `make:controller`,
  `make:action`, etc.
- [Consola](console.md) - el binario `cargo run --bin console` por
  proyecto
