# Descripción general de Frontend

Suprnova conecta handlers de Rust a un frontend de una sola página a través de
[Inertia.js](https://inertiajs.com/) 3.4.0. Escribes controladores en Rust
y páginas en Svelte, React o Vue; el framework mueve props tipificados
entre ellos sin una API HTTP separada en el medio.

## Tres iniciadores de primera clase

`suprnova new <name>` genera el andamiaje de un proyecto funcional. El flag `--frontend`
elige la capa SPA:

```bash
suprnova new my-app                       # Svelte 5 (predeterminado)
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

Los tres andamiajes comparten la misma pila:

| Capa | Versión |
|---|---|
| Adaptador de cliente Inertia | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| Herramienta de construcción | Vite 8 |
| Estilos | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | modo estricto |

La elección es por proyecto. No hay un framework "principal" en el
lado servidor - `inertia_response!` resuelve la extensión que
tu andamiaje elegido utiliza (`.svelte`, `.tsx`, `.vue`), y `App::inertia_share`,
recargas parciales y generación de props de TypeScript funcionan de forma idéntica
en los tres.

## Arquitectura

```
                       Navegador
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  props tipificados de struct Rust   |
   |   +-------v-------------------------------+     |
   |   |        Adaptador de cliente Inertia   |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON en XHR, HTML en primera carga)
                          v
   +-------------------------------------------------+
   |                  Servidor Suprnova              |
   |   +------------------------------------------+  |
   |   |       Controladores / handlers           |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

La primera solicitud devuelve un shell HTML con el objeto de página inicial
incrustado en el atributo `data-page` del nodo de montaje. Las visitas posteriores
pasan a través de `<Link>` / `router.visit`, envían `X-Inertia: true`, y obtienen
un objeto de página JSON - el adaptador intercambia el componente sin una
recarga completa.

## Recorrido completo de una página

El controlador define sus props como un struct de Rust, deriva
`InertiaProps`, y pasa el valor a la macro `inertia_response!`:

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

La macro hace algunas cosas por ti. Primero, valida en tiempo de compilación
que el archivo del componente de página realmente existe bajo
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` - los errores tipográficos aparecen como un
error de construcción, no como 404 en el navegador. Segundo, serializa el
struct `HomeProps`, lo despliega en un prop por clave de nivel superior para que
las recargas parciales puedan filtrar, y resuelve cualquier prop perezoso o diferido
contra `&req` antes de devolverlo. La macro se evalúa a un
`Result<HttpResponse, FrameworkError>`, que el tipo de retorno `Response`
acepta directamente.

La página Svelte correspondiente (el andamiaje predeterminado):

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

Para los equivalentes en React y Vue, consulta [Componentes de página](frontend-pages.md).

## Generación de tipos de TypeScript

Cada struct `#[derive(InertiaProps)]` en tu `src/` se convierte en una
interfaz de TypeScript en `frontend/src/types/inertia-props.ts`:

```bash
suprnova generate-types
```

Pasa `--routes` y el mismo comando también emite
`frontend/src/types/routes.ts` - pares de URL y método con seguridad de tipos extraídos
de tu macro `routes!` que funcionan directamente con APIs de Inertia v2+. La tabla completa de mapeo de tipos y
la forma del ayudante de rutas se encuentran en [Tipos de TypeScript](frontend-typescript-types.md).

## Datos compartidos

Cualquier cosa que deba aparecer en cada página (el usuario autenticado, el
locale actual, metadatos de la aplicación) se registra una vez al arranque y se fusiona en
cada respuesta de Inertia:

```rust
// En bootstrap.rs
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// Los datos compartidos asincronos / por solicitud pasan a través del trait.
App::register_inertia_shared(Arc::new(AppSharedData));
```

Tres variantes, en orden de precedencia (la posterior gana en la misma clave):

| API | Cuándo se materializa el valor |
|---|---|
| `App::inertia_share(k, v)` | Sincrónico, establecido una vez en el arranque |
| `App::inertia_share_lazy(k, \|\| async { ... })` | Por respuesta, recalculado |
| `App::inertia_share_once(k, \|\| async { ... })` | Por respuesta, luego en caché del cliente |
| `App::register_inertia_shared(Arc::new(impl))` | Por solicitud, ve `&req` |

Los props por página adjuntos en el generador de respuestas siempre sobrescriben los datos compartidos
en la misma clave.

## Recargas parciales y props perezosos

El mismo generador `InertiaResponse` expone el kit completo de props de Inertia v3 - ansioso, perezoso, opcional, diferido, fusión, una sola vez - y Suprnova
honra automáticamente los encabezados de recarga parcial de v3 (`X-Inertia-Partial-Data`,
`X-Inertia-Partial-Except`, `X-Inertia-Reset`,
`X-Inertia-Except-Once-Props`). El ejemplo a continuación
adjunta tres props con diferentes reglas de evaluación:

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` cubre el caso de props ansiosos; todo lo demás
pasa a través del generador. La superficie completa - `optional`, `merge`,
`once`, `scroll`, `flash`, `paginate`, SSR, desajuste de versión, cifrado de historial - está documentada en
[Respuestas de Inertia](frontend-inertia-responses.md).

## Arranque de la aplicación

Una aplicación con andamiaje instala los dos middlewares críticos de protocolo en una
llamada dentro de `bootstrap.rs`:

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` devuelve `Result` - falla de forma cerrada si `InertiaConfig` resuelve a
modo de producción (el predeterminado bajo `APP_ENV=production`) pero no se puede encontrar
un manifiesto Vite, en lugar de retroceder silenciosamente a una
ruta de activos heredada. Consulta [Desarrollo frente a producción](#desarrollo-frente-a-producción)
más abajo.

Que registra `InertiaVersionMiddleware` (emite 409 + `X-Inertia-Location`
en desajuste de versión de activos para que los clientes obsoletos se recarguen) e `Inertia303Middleware`
(reescribe 302 - 303 en visitas de Inertia que no sean GET para que la siguiente solicitud sea
inequívocamente un GET). Ambos solían ser opcionales; `Inertia::install` los hace
predeterminados.

## Desarrollo frente a producción

En desarrollo, el servidor de desarrollo Vite se ejecuta junto al backend y
sirve activos habilitados con HMR:

```bash
suprnova serve
```

Esto arranca el servidor Rust y `vite` juntos. El shell HTML carga
módulos desde `http://localhost:5765`.

Para producción, construye el frontend una vez y apunta el backend al
manifiesto con hash bajo `public/assets/`:

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` deriva el modo de producción frente a desarrollo de
`APP_ENV` (vía `Environment::detect().is_production()`) - `APP_ENV=production`
es lo que hace que el shell HTML cargue activos construidos en lugar del servidor
de desarrollo Vite. Luego `Inertia::install` falla en el arranque claramente si no puede encontrar un
manifiesto para respaldar esa decisión, en lugar de retroceder silenciosamente a una
ruta codificada obsoleta.

Suprnova lee `public/assets/.vite/manifest.json` para resolver
puntos de entrada con hash más las importaciones transitivas para `modulepreload`. SSR es
opcional - participa apuntando `InertiaConfig::ssr(...)` a un
worker `@inertiajs/{vue3,react,svelte}/server` en ejecución.

### Por qué Suprnova diverge

Tres desviaciones intencionadas de cómo se ve una configuración típica de Inertia
en otros lugares:

- **Validación de componentes en tiempo de compilación.** La macro `inertia_response!`
  camina por `frontend/src/pages/` en tiempo de construcción y se niega a expandirse si
  falta el archivo del componente, sugiriendo la coincidencia más cercana. No
  puedes enviar un controlador que apunte a una página eliminada.
- **Props tipificados como la fuente de verdad.** Los props de página son structs de Rust
  con `#[derive(InertiaProps)]`. `suprnova generate-types` los lee
  y escribe interfaces de TypeScript - los tipos de frontend se derivan
  del backend, no se mantienen en paralelo.
- **Svelte como predeterminado.** La documentación de Inertia llega primero a Vue y
  React; el generador de andamiaje de Suprnova por defecto es Svelte 5 (con runes).
  React 19 y Vue 3.5 son de primera clase, no reflexiones posteriores - mismo
  protocolo, mismo pipeline de props, misma salida del generador.

## Siguiente

- [Componentes de página](frontend-pages.md)
- [Respuestas de Inertia](frontend-inertia-responses.md)
- [Tipos de TypeScript](frontend-typescript-types.md)
- [Enrutamiento](routing.md)
- [Controladores](controllers.md)
