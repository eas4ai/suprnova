# Visão geral do Frontend

Suprnova conecta handlers Rust a um frontend de página única via
[Inertia.js](https://inertiajs.com/) 3.4.0. Você escreve controladores em Rust
e páginas em Svelte, React ou Vue; o framework move props tipadas
entre eles sem uma API HTTP separada no meio.

## Três starters de primeira classe

`suprnova new <name>` cria com scaffold um projeto funcional. A flag `--frontend`
escolhe a camada SPA:

```bash
suprnova new my-app                       # Svelte 5 (padrão)
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

Todos os três scaffolds compartilham a mesma stack:

| Camada | Versão |
|---|---|
| Adaptador client Inertia | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| Ferramenta de build | Vite 8 |
| Styling | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | modo strict |

A escolha é por projeto. Não há framework "principal" no lado do
servidor - `inertia_response!` resolve a extensão que seu
scaffold escolhido usa (`.svelte`, `.tsx`, `.vue`), e `App::inertia_share`,
reloads parciais e geração de tipos TypeScript comportam-se identicamente
nos três.

## Arquitetura

```
                       Navegador
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  props tipadas de struct Rust       |
   |   +-------v-------------------------------+     |
   |   |        Adaptador client Inertia       |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON em XHR, HTML no carregamento inicial)
                          v
   +-------------------------------------------------+
   |                 Servidor Suprnova               |
   |   +------------------------------------------+  |
   |   |         Controladores / handlers         |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

A primeira requisição retorna uma shell HTML com o objeto de página inicial
incorporado no atributo `data-page` do nó mount. Visitas subsequentes
passam por `<Link>` / `router.visit`, enviam `X-Inertia: true` e recebem
um objeto de página JSON - o adaptador troca o componente sem um
reload completo.

## Uma volta de página completa

O controlador define suas props como um struct Rust, deriva
`InertiaProps` e passa o valor para a macro `inertia_response!`:

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

Algumas coisas que a macro faz para você. Primeiro, ela valida em tempo de
compilação que o arquivo do componente de página realmente existe em
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` - erros de digitação aparecem como
erro de build, não como 404 no navegador. Segundo, ela serializa o
struct `HomeProps`, desdobra-o em uma prop por chave de nível superior para que
reloads parciais possam filtrar, e resolve qualquer prop lazy ou deferred
contra `&req` antes de retornar. A macro avalia para um
`Result<HttpResponse, FrameworkError>`, que o tipo de retorno `Response`
aceita diretamente.

A página Svelte correspondente (o scaffold padrão):

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

Para os equivalentes de React e Vue, veja [Componentes de página](frontend-pages.md).

## Gerando tipos TypeScript

Todo struct `#[derive(InertiaProps)]` em seu `src/` torna-se uma
interface TypeScript em `frontend/src/types/inertia-props.ts`:

```bash
suprnova generate-types
```

Passe `--routes` e o mesmo comando também emite
`frontend/src/types/routes.ts` - pares de URL + método seguros quanto aos tipos
extraídos de sua macro `routes!` que funcionam diretamente com APIs Inertia v2+. A tabela completa de mapeamento de tipos e
o shape de route-helper vivem em [Tipos TypeScript](frontend-typescript-types.md).

## Dados compartilhados

Qualquer coisa que deva aparecer em cada página (o usuário autenticado, a
locale atual, metadados da app) é registrada uma vez no boot e mesclada em
cada resposta Inertia:

```rust
// Em bootstrap.rs
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// Dados compartilhados async / por requisição passam pelo trait.
App::register_inertia_shared(Arc::new(AppSharedData));
```

Três variações, em ordem de precedência (posterior vence na mesma chave):

| API | Quando o valor se materializa |
|---|---|
| `App::inertia_share(k, v)` | Sync, definido uma vez no boot |
| `App::inertia_share_lazy(k, \|\| async { ... })` | Por resposta, recomputado |
| `App::inertia_share_once(k, \|\| async { ... })` | Por resposta, depois cached no client |
| `App::register_inertia_shared(Arc::new(impl))` | Por requisição, vê `&req` |

Props por página anexadas no response builder sempre sobrescrevem dados
compartilhados na mesma chave.

## Reloads parciais e props lazy

O mesmo builder `InertiaResponse` expõe o toolkit completo de props do Inertia v3 -
eager, lazy, optional, deferred, merge, once - e Suprnova
honra os headers de reload parcial v3 (`X-Inertia-Partial-Data`,
`X-Inertia-Partial-Except`, `X-Inertia-Reset`,
`X-Inertia-Except-Once-Props`) automaticamente. O exemplo abaixo
anexa três props com regras de avaliação diferentes:

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

`inertia_response!` cobre o caso eager-props; tudo além disso
passa pelo builder. A superfície completa - `optional`, `merge`,
`once`, `scroll`, `flash`, `paginate`, SSR, incompatibilidade de versão, criptografia
de histórico - está documentada em
[Respostas Inertia](frontend-inertia-responses.md).

## Inicialização

Uma app com scaffold instala os dois middlewares críticos do protocolo em uma
chamada dentro de `bootstrap.rs`:

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` retorna `Result` - falha de forma fechada se `InertiaConfig` se resolve para
modo produção (o padrão em `APP_ENV=production`) mas nenhum manifesto Vite
puder ser encontrado, em vez de silenciosamente retornar a um caminho de
asset legado. Veja [Desenvolvimento vs produção](#desenvolvimento-vs-produção)
abaixo.

Isso registra `InertiaVersionMiddleware` (emite 409 + `X-Inertia-Location`
em incompatibilidade de versão de asset para que clients desatualizados façam reload) e `Inertia303Middleware`
(reescreve 302 - 303 em visitas Inertia não-GET para que o follow-up seja
inequivocamente um GET). Ambas costumavam ser opt-in; `Inertia::install` as torna
o padrão.

## Desenvolvimento vs produção

Em desenvolvimento, o servidor de dev Vite roda ao lado do backend e
serve assets habilitados para HMR:

```bash
suprnova serve
```

Isso inicia o servidor Rust e `vite` juntos. A shell HTML carrega
módulos de `http://localhost:5765`.

Para produção, compile o frontend uma vez e aponte o backend para o
manifesto com hash sob `public/assets/`:

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` deriva modo produção vs. desenvolvimento de
`APP_ENV` (via `Environment::detect().is_production()`) - `APP_ENV=production`
é o que faz a shell HTML carregar assets compilados em vez do servidor de dev Vite. `Inertia::install`
então falha no boot de forma alta se não conseguir encontrar um
manifesto para respaldar essa decisão, em vez de silenciosamente retornar a um
caminho hardcoded desatualizado.

Suprnova lê `public/assets/.vite/manifest.json` para resolver pontos de
entrada com hash mais qualquer importação transitiva para `modulepreload`. SSR é
opcional - faça opt-in apontando `InertiaConfig::ssr(...)` para um worker
`@inertiajs/{vue3,react,svelte}/server` em execução.

### Por que Suprnova diverge

Três partidas intencionais de como um setup típico de Inertia se parece
em outro lugar:

- **Validação de componentes em tempo de compilação.** A macro
  `inertia_response!` caminha por `frontend/src/pages/` em tempo de build
  e recusa expandir se o arquivo do componente está faltando, sugerindo
  a correspondência mais próxima. Você não pode entregar um controlador
  que aponta para uma página deletada.
- **Props tipadas como a fonte da verdade.** Props de página são structs Rust
  com `#[derive(InertiaProps)]`. `suprnova generate-types` os lê
  e escreve interfaces TypeScript - os tipos de frontend são derivados
  do backend, não mantidos em paralelo.
- **Svelte como o padrão.** A documentação do Inertia alcança Vue e
  React primeiro; o scaffolder Suprnova padrão é Svelte 5 (runes-on).
  React 19 e Vue 3.5 são primeira classe, não pensamentos posteriores - mesmo
  protocolo, mesmo pipeline de props, mesma saída do gerador.

## Próximos passos

- [Componentes de página](frontend-pages.md)
- [Respostas Inertia](frontend-inertia-responses.md)
- [Tipos TypeScript](frontend-typescript-types.md)
- [Roteamento](routing.md)
- [Controladores](controllers.md)
