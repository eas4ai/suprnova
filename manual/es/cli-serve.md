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
   existir (a menos que se use `--frontend-only`) y debe existir un
   directorio `frontend/` (a menos que se use `--backend-only`).
4. Regenera los tipos de TypeScript a partir de cualquier estructura
   `#[derive(InertiaProps)]` que encuentre en `src/`, y los escribe en
   `frontend/src/types/inertia-props.ts`.
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
   existe. Se omite bajo `--backend-only`.
7. Lanza `cargo watch -x 'run --bin <package-name>'` para el backend.
   `cargo-watch` vuelve a ejecutar el binario cada vez que cambia un
   archivo `.rs`.
8. Lanza `npm run dev` en `frontend/` para Vite, lo que te da HMR
   para los componentes de Svelte/React/Vue y las clases de Tailwind.
9. Inicia un monitor de archivos sobre `src/` que vuelve a ejecutar el
   generador de tipos cada vez que cambia un archivo `.rs`, una vez
   que la ráfaga de guardados ha estado en silencio durante 500 ms.
   El antirrebote espera hasta el final de la ráfaga, así que una
   ráfaga de cambios - `cargo fmt`, formatear al guardar en varios
   archivos, un cambio de rama - se agrupa en una única regeneración
   que se ejecuta *después* de la última escritura, en lugar de una
   que se dispara con el primer archivo y se pierde el resto.
10. Reenvía el stdout/stderr de ambos hijos a tu terminal con los
    prefijos `[backend]` y `[frontend]`.

`Ctrl+C` le indica al gestor que active su flag de apagado, mate a
ambos procesos hijos, y salga. Si alguno de los procesos termina por
sí mismo - normalmente por un error de compilación de Rust demasiado
grave para que `cargo watch` se recupere, o por un conflicto de
puertos - el gestor lo trata como una señal de apagado y derriba al
otro.

### Por qué Suprnova diverge

Los usuarios de Laravel normalmente ejecutan `php artisan serve` para
el backend y `npm run dev` en otra terminal, y la mayoría de los
equipos disimulan la división de dos terminales con un `Procfile` y
`foreman`/`overmind`. Suprnova distribuye ese multiplexor como un
comando de CLI de primera clase. Obtienes una sola terminal, un solo
`Ctrl+C`, arranque automático de la cadena de herramientas
(`cargo-watch`, `npm install`), y un puente Inertia tipado que
regenera `frontend/src/types/inertia-props.ts` sobre la marcha, de
modo que tus componentes de Svelte/React/Vue siempre ven la forma
actual de las props sin sincronización manual de tipos.

## Recarga en caliente

**Backend.** `cargo watch -x 'run --bin <package>'` es el bucle.
Reconstruye y reinicia el servidor ante cada cambio `.rs` en el
proyecto. Las reconstrucciones en frío después de tocar un crate
pesado pueden tardar varios segundos; los cambios incrementales en un
solo archivo suelen tardar menos de un segundo.

**Frontend.** El HMR de Vite inyecta los cambios de componentes en el
mismo lugar sin una recarga completa, preservando el estado del
componente. Las clases de Tailwind se actualizan en vivo a través del
monitor de Tailwind v4.

**Tipos de TypeScript.** Cada vez que cambia un archivo `.rs`, el
monitor de tipos vuelve a ejecutar el generador. Si aparecen nuevas
estructuras `#[derive(InertiaProps)]` (o las existentes cambian de
forma), el `frontend/src/types/inertia-props.ts` regenerado dispara
el HMR de Vite para el componente que las importa.

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

### El backend termina en silencio justo después de iniciar

Cuando cualquiera de los procesos hijos termina, el gestor también
apaga al otro. Si el backend murió por un error de compilación, las
líneas `[backend]` justo antes del mensaje "Servers stopped." mostrarán
el `error[E…]` de rustc. Corrige el error de compilación y vuelve a
ejecutar.

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
