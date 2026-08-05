# Respostas Inertia

Respostas Inertia são como um handler do Suprnova envia estado para um
componente de página Svelte / React / Vue. Todo handler que renderiza
uma página Inertia retorna uma, construída ou através da macro
[`inertia_response!`](#a-macro-inertia-response) (para props eager
tipadas, verificadas em tempo de compilação) ou do builder
[`InertiaResponse`](#o-builder-inertiaresponse) (para todo o resto -
props lazy, props deferred, merge, once, scroll, flash). Este capítulo
cobre a superfície de resposta de ponta a ponta: a macro, o builder, as
features do protocolo v3 (reloads parciais, criptografia de histórico,
detecção de versão), dados compartilhados via `App::inertia_share*`, e
o flash bag carregado através de redirecionamentos.

Se você ainda não escolheu um frontend, [Visão geral do
Frontend](frontend.md) e [Componentes de página](frontend-pages.md) vêm
primeiro; este capítulo assume que a ponte SPA já está conectada e foca
no que seu handler retorna.

## A macro `inertia_response!`

A macro é o caminho mais curto de um handler até uma página eager
tipada. Ela recebe a solicitação atual, um nome de componente, e uma
expressão de props:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

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

Três coisas para saber:

- **O `&req` inicial é obrigatório.** A macro lê os headers
  `X-Inertia`, a URL, e os headers de filtragem de reload parcial a
  partir da solicitação, então ela precisa do valor da solicitação (ou
  de uma referência a ele). Sem isso, reloads parciais quebrariam
  silenciosamente.
- **A existência do componente é verificada em tempo de compilação.** A
  macro procura `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`;
  se nenhum arquivo corresponder, o build falha com uma sugestão "você
  quis dizer…?" obtida a partir dos nomes de arquivo reais no disco.
  Caminhos aninhados funcionam da mesma forma -
  `inertia_response!(&req, "Admin/Dashboard", …)` resolve para
  `frontend/src/pages/Admin/Dashboard.svelte` (ou a extensão do seu
  frontend).
- **A macro se expande para um `Result` aguardado (`await`).** Seu
  handler precisa retornar [`Response`](error-model.md) (que é
  `Result<HttpResponse, HttpResponse>`) ou outro tipo que absorva
  `FrameworkError` através de `?` / `From`. Falhas durante a
  serialização de props ou a construção da resposta são retornadas como
  `Err`, não como panics.

### Props no estilo JSON

Para prototipagem e páginas pequenas você pode pular o struct tipado:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

A macro ainda valida o arquivo do componente. A contrapartida é que
você perde a cadeia de prop tipada - sem `#[derive(InertiaProps)]`, sem
geração automática de TypeScript, sem verificação em tempo de
compilação de que o formato esperado pelo frontend corresponde.

### Sobrescrita opcional de configuração

A macro aceita um `InertiaConfig` opcional no final para sobrescritas
por resposta (configurações de SSR diferentes, um título padrão
customizado para uma página):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

A maioria dos apps registra um único config na inicialização via
[`Inertia::install`](#inicialização-inertia-install) e nunca toca nesse
argumento.

## `#[derive(InertiaProps)]`

`InertiaProps` emite uma impl `Serialize` cujos nomes de chave
correspondem aos nomes dos seus campos. Ela existe para que o caminho
de props tipadas permaneça conciso e para que o gerador TypeScript
(`suprnova generate-types`) tenha um marcador para encontrar:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Tipos aninhados compõem normalmente - campos podem ser `Vec<T>`,
`Option<T>`, structs aninhados, qualquer coisa que implemente
`Serialize`. Os tipos aninhados em si não precisam derivar
`InertiaProps`; eles só precisam de `Serialize`. Use
`#[derive(InertiaProps)]` no struct de props de *nível superior* e você
recebe a superfície TypeScript automática (veja [Tipos
TypeScript](frontend-typescript-types.md)) para a árvore inteira.

## O builder `InertiaResponse`

A macro cobre o caso de props eager tipadas. Todo o resto - lazy,
optional, deferred, mergeable, cacheado no cliente, flash, sobrescritas
de criptografia de histórico - usa o builder diretamente:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: a closure só executa quando a prop realmente vai ser
        // enviada (visita inicial, ou reload parcial que pede essa
        // chave).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: nunca enviada em visitas iniciais; o cliente
        // precisa pedir a chave explicitamente via
        // X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: pulada na renderização inicial; o cliente emite um
        // XHR de acompanhamento e a closure executa então.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: acrescenta ao existente em reloads parciais ("carregar mais").
        .merge("rows", next_page().await?)
        // Once: cacheada no lado do cliente entre navegações; o
        // resolver é pulado em visitas subsequentes a menos que o
        // servidor force um refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: toast único; aparece sob `page.flash`, não em `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Método | Propósito | Equivalente no Laravel |
|---|---|---|
| `.with(k, v)` | Prop eager, respeita a filtragem de reload parcial | prop tipada |
| `.always(k, v)` | Prop eager, ignora os filtros de reload parcial | `Inertia::always(…)` |
| `.lazy(k, ‖)` | O resolver só executa quando a prop for enviada | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca na visita inicial; precisa ser pedida explicitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Pulada na visita inicial; um XHR de acompanhamento dispara a resolução | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina com o estado existente no cliente em reloads parciais | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | O cliente cacheia entre navegações | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Paginação por scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor único sob `page.flash` (não `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` padrão para a shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Criptografia de histórico por resposta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Força a rotação da chave de histórico | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Mantém o `#fragment` depois de uma visita Inertia | `Inertia::preserveFragment()` |

Métodos eager do builder têm irmãos `try_*` (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`) que
retornam `Result<Self, FrameworkError>` quando a impl `Serialize` de um
valor pode falhar em tempo de execução - os métodos infalíveis convertem
o panic em um 500 via [o limite de panic](error-model.md), então use
`try_*` quando preferir tratar a falha explicitamente.

### Estratégias de merge e scroll infinito

`.merge` (acrescenta), `.merge_prepend`, e `.deep_merge` cobrem os casos
comuns de "carregar mais". Para fazer um diff-merge - atualizar linhas
que o cliente já tem em vez de duplicá-las - use `.merge_with` com uma
`MergeStrategy` explícita carregando uma chave `match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // a nova fatia de página
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` nomeia o campo em que o cliente deduplica (emitido no objeto
de página como `matchPropsOn`), então uma nova busca que sobrepõe a
janela atual substitui as linhas correspondentes no lugar em vez de
acrescentar cópias. `Prepend` e `Deep` recebem o mesmo `match_on`.

O scroll infinito é a mesma máquina com metadados de paginação anexados.
`.scroll` / `.scroll_with` - ou `.paginate`, que adapta um
`LengthAwarePaginator` ou `CursorPaginator` diretamente - emitem
`scrollProps` ao lado dos dados, e o componente `<InfiniteScroll>` do
cliente conduz as buscas seguinte/anterior:

```rust
// `posts` é um CursorPaginator vindo do construtor de consultas.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

O framework lê a direção do merge a partir do header de solicitação
`X-Inertia-Infinite-Scroll-Merge-Intent` que o cliente envia (`append`
ao rolar para baixo, `prepend` ao rolar para cima). Em uma visita nova -
sem header de intenção - `scrollProps["posts"].reset` é `true`, então o
cliente limpa seu acumulador antes de renderizar a primeira janela.

## Reloads parciais

O cliente do Inertia 3 pode pedir um subconjunto das props de uma
página (ou um superconjunto incluindo uma chave Optional ou Defer). O
protocolo usa três headers de solicitação:

| Header | Significado |
|---|---|
| `X-Inertia-Partial-Component` | O componente sendo recarregado parcialmente - precisa corresponder ao componente da resposta para que a filtragem se aplique. |
| `X-Inertia-Partial-Data` | Whitelist: chaves de prop separadas por vírgula a incluir. |
| `X-Inertia-Partial-Except` | Blacklist: chaves de prop separadas por vírgula a excluir. Vence sobre `Partial-Data` em colisão de chave. |

Regras de filtragem:

- Props `Eager`, `Lazy`, `Merge`, `Once`, `Scroll` seguem a semântica de
  whitelist / blacklist.
- Props `Always` são enviadas independentemente disso.
- Props `Optional` e `Defer` nunca aparecem em uma visita padrão e só
  aparecem em um reload parcial correspondente que liste a chave
  explicitamente.

O handler não precisa fazer nada especial - registre toda prop através
do builder, e o framework consulta os headers ao serializar o objeto de
página.

## Dados compartilhados via `App::inertia_share*`

Algumas props são as mesmas em toda página Inertia - estado de
autenticação, o token CSRF, a locale atual, flags de todo o app.
Registre-as uma vez na inicialização e elas se mesclam em toda resposta:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materializado uma vez na inicialização.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolvido por resposta (pulado por reloads parciais que
    // excluem a chave).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cacheado no cliente entre navegações - `share_once` executa na
    // primeira página que precisa dele, e então o cliente pula a
    // re-resolução via X-Inertia-Except-Once-Props até a chave de
    // cache mudar.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Para dados compartilhados por solicitação (o usuário autenticado, flags
com escopo de solicitação), implemente
[`InertiaSharedData`](#dados-compartilhados-por-solicitação) e registre
o singleton - o framework chama `share(&req)` em toda resposta Inertia
e mescla o resultado.

### Precedência em colisão de chave

Quando a mesma chave aparece em mais de uma camada, a escrita mais
recente vence:

1. Registro estático (`App::inertia_share` / `App::inertia_share_lazy`)
2. Provider por solicitação via trait (`InertiaSharedData::share`)
3. Métodos do builder por resposta (`.with`, `.lazy`, etc.)

Isso permite que um handler sobrescreva um padrão compartilhado
globalmente para uma página, sem precisar desregistrar nada.

### Dados compartilhados por solicitação

O trait executa uma vez por resposta Inertia, com acesso à solicitação.
As implementações precisam de `async_trait` (reexportado como
`suprnova::__async_trait`) e `IndexMap` (reexportado como
`suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// Na inicialização:
App::register_inertia_shared(Arc::new(AuthShare));
```

## Flash e redirecionamentos

Dados flash são estado único que deve aparecer na próxima renderização e
desaparecer depois - mensagens toast, IDs de "recém-criado", resumos de
validação. O Suprnova os expõe sob `page.flash` em toda resposta
Inertia. Há três formas de escrevê-los:

```rust
// 1. Empurra para o flash bag da solicitação atual.
App::flash("toast", "Saved");

// 2. Anexa a uma resposta específica (mesmo efeito, só nessa resposta).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carrega através de um redirecionamento via a facade Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

A forma `Redirect::with(key, value)` é o caminho entre handlers: o
valor cai na sessão sob `_flash.new.*`, o [`SessionMiddleware`](csrf.md)
da próxima solicitação o envelhece para `_flash.old.*`, e o
`InertiaResponse` do destino o expõe sob `page.flash`.

O flash da mesma solicitação (o bag task-local) vence sobre o flash de
sessão herdado em caso de colisão de chave, então um handler de destino
pode sobrescrever um valor de entrada apenas reflashando a chave.

Chaves de sessão internas (qualquer coisa prefixada com `_`) são
filtradas de fora de `page.flash` - `_old_input` para repopulação de
formulário e flags de protocolo `_inertia.*` não vazam para o cliente.

### Helpers de redirecionamento

`Redirect` é a superfície completa do Laravel:

```rust
Redirect::to("/dashboard")                       // 302 para um caminho
Redirect::route("posts.show").with("id", "42")   // rota nomeada, params de rota
Redirect::back("/")                              // URL anterior registrada na sessão
Redirect::refresh()                              // mesma URL, GET novo
Redirect::guest(&req, "/login")                  // guarda a URL pretendida
Redirect::intended("/dashboard")                 // resgata a URL guardada
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL assinada
Redirect::to("/posts/42").preserve_fragment()    // mantém #frag através da visita
```

Todas as variantes de `Redirect` aceitam `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name, map)`,
`.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`, etc. A
cadeia completa espelha o `RedirectResponse` do Laravel.

Para visitas Inertia não-GET, o framework converte automaticamente a
resposta para `303 See Other` quando o
[`Inertia303Middleware`](#inicialização-inertia-install) está instalado,
então o navegador emite um GET de acompanhamento limpo em vez de
resubmeter o PUT/PATCH/DELETE original ao destino do redirecionamento.

## Detecção de versão

O Inertia versiona o manifesto de assets para que um cliente de vida
longa não tente montar uma página do bundle de ontem contra o servidor
de hoje. Quando o header `X-Inertia-Version` do cliente não corresponde
à versão configurada do servidor, o
[`InertiaVersionMiddleware`](#inicialização-inertia-install) responde
com `409 Conflict` e um header `X-Inertia-Location` nomeando a nova
URL - o cliente Inertia capta isso e faz um reload completo da
página, recebendo o novo bundle.

Você define a versão através de `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Estático - a maioria dos apps. Embuta um identificador definido em
// tempo de build.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dinâmico - lê um hash de manifesto, um ID de deployment de contêiner,
// qualquer coisa. A closure executa em toda verificação de versão;
// cacheie internamente se não for barata.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Para resolução de versão async ou falível (por exemplo, ler um hash de
manifesto do S3), faça a leitura uma vez na inicialização e passe a
`String` cacheada para `.version(...)`.

## Inicialização: `Inertia::install`

A maioria dos apps instala os dois middlewares de protocolo em uma
única chamada:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …outros dados compartilhados, rotas, etc.
    Ok(())
}
```

`Inertia::install` retorna `Result` e, em ordem:

1. Falha de forma fechada se `cfg` se resolve para o modo produção
   (`development == false` - o padrão sempre que `APP_ENV=production`)
   mas nenhum manifesto Vite pode ser carregado a partir de
   `cfg.manifest_path`. Esta é a proteção CFG-01: uma inicialização de
   produção com um frontend não compilado falha de forma explícita em
   vez de silenciosamente recair para um caminho de asset hardcoded
   legado.
2. Registra o `InertiaVersionMiddleware` - emite o `409` +
   `X-Inertia-Location` quando cliente e servidor discordam sobre a
   versão do asset.
3. Registra o `Inertia303Middleware` - eleva `302` para `303` em
   redirecionamentos Inertia não-GET.

Pule a chamada somente se você genuinamente não quiser um desses
middlewares (raro; ambos fecham falhas reais - bundle obsoleto
silencioso e reenvio de formulário no redirecionamento).

## Elementos `<head>` controlados pelo servidor

O Inertia 3.5 adicionou uma opção de cliente para deixar o servidor
decidir o que vai em `<head>` - útil quando as meta tags dependem do
registro que você acabou de carregar, e você não quer que o título e as
tags OG vivam em dois lugares.

Isso não precisa de nenhum suporte do framework. O cliente lê os
elementos a partir de uma **prop comum**, então qualquer handler pode
fornecê-los:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

Faça opt-in no cliente:

```js
createInertiaApp({
  serverHead: true,        // lê a prop `head`
  // serverHead: 'meta',   // ou lê uma prop com outro nome
  // serverHead: (page) => [...],  // ou calcula a partir da página inteira
})
```

Cada string é um elemento HTML. O cliente estampa um atributo
`data-inertia` em qualquer coisa que não tenha um, para poder fazer diff
de elementos de head entre navegações; forneça seu próprio
`data-inertia="og-title"` quando quiser identidade estável em vez de
correspondência posicional.

Escape qualquer coisa interpolada a partir de dados do usuário - essas
strings são injetadas como HTML, então as regras usuais se aplicam.

## SSR

O Suprnova conversa com um worker SSR fora de processo - tipicamente o
bundle `createServer()` de `@inertiajs/{svelte,react,vue}/server`
rodando sob Node / Bun / Deno - via HTTP loopback. Ative-o na config:

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // URL do worker
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

O SSR vem desativado por padrão. Quando ativado, o framework envia o
objeto de página para `<url>/render` e embute `{ head, body }` na shell
HTML. Em erro ou timeout do worker, a resposta recai para CSR (uma
`<div id="app">` vazia que o cliente hidrata) e o hook
`on_ssr_error(...)` dispara; ative `ssr_throw_on_error(true)` em CI para
tornar essas falhas 500s duros em vez disso.

Inicie o worker separadamente - `suprnova ssr:start` é o executor padrão
uma vez que seu projeto tenha um entry point de SSR.

## Configuração

O comportamento do Inertia é configurado programaticamente via
`InertiaConfig`. A única env var que o framework lê diretamente é
`SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), que escolhe o nome do
arquivo de entry point padrão e as extensões de componente de página.
Todo o resto tem o formato de um builder:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // sobrescreve SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // limita o fan-out de props lazy
    .production();                           // false → carrega do servidor de dev Vite
```

Padrões específicos por frontend:

| Frontend | Entry point padrão | Extensões de página |
|---|---|---|
| Svelte (padrão) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

O manifesto Vite em `manifest_path` é carregado de forma lazy na
primeira solicitação e cacheado pela vida do processo. Quando está
faltando, as tags de asset de produção recaem para um caminho legado
hardcoded e um `tracing::warn!` dispara para que a lacuna apareça nos
logs.

### Por que Suprnova diverge

O adaptador Inertia do Laravel tem um único registro global de "dados
compartilhados" mais uma chamada `Inertia::share($k, $v)` por
solicitação. O modelo de processo-por-solicitação do PHP torna isso
seguro: um processo novo por solicitação significa nenhum vazamento
entre visitantes concorrentes.

O modelo de processo do Rust é o oposto - um único processo atende
muitas solicitações concorrentes através de muitas threads. Então o
registro vive no [contêiner](container.md) (task-local → thread-local →
global), não em statics globais de processo. `App::inertia_share*`
escreve no `InertiaRegistry` do contêiner ativo, o que dá aos testes
que usam `TestContainer::fake()` um isolamento limpo sem precisar
desregistrar nada. Mesma superfície do Laravel; maquinário diferente
por baixo, porque o runtime é diferente.

Duas outras escolhas moldadas pelo Rust que vale destacar:

- **Resolvers de prop lazy executam concorrentemente**, limitados por
  `max_concurrent_resolvers` (padrão 16). Uma página com doze props
  lazy emite doze queries paralelas dentro de uma única task Tokio - é
  exatamente para isso que construímos o framework sobre o Tokio. Ajuste
  o limite se uma página tiver muitas props lazy, cada uma acessando um
  serviço externo.
- **A verificação de componente em tempo de compilação** não é nem de
  longe uma feature do Laravel, porque o PHP não consegue ver os
  arquivos do seu frontend em tempo de compilação. O Suprnova consegue,
  então um erro de digitação em `inertia_response!("Dashbaord", …)`
  falha o build com uma sugestão "você quis dizer Dashboard?" em vez de
  aparecer como um "componente não encontrado" em tempo de execução
  mais tarde.

## Próximos passos

- [Componentes de página](frontend-pages.md) - como o frontend resolve
  um nome de componente para um módulo Svelte / React / Vue
- [Tipos TypeScript](frontend-typescript-types.md) - `suprnova
  generate-types` emite definições TS a partir dos seus structs
  `#[derive(InertiaProps)]`
- [Objetos Data](data.md) - `#[derive(Data)]` para DTOs com controle de
  include/allowlist por campo que compõe com reloads parciais
- [Modelo de erros](error-model.md) - como `Response`, o limite de
  panic, e `FrameworkError` atravessam as respostas Inertia
- [Contêiner](container.md) - o modelo de busca por trás de
  `App::inertia_share*` e `InertiaSharedData`
