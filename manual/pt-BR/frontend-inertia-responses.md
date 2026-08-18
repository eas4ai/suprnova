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

- **O `&req` inicial é obrigatório.** A macro lê os headers `X-Inertia`,
  a URL, e os headers de filtragem de reload parcial da solicitação,
  então ela precisa do valor da solicitação (ou de uma referência). Sem
  ele, reloads parciais quebrariam silenciosamente.
- **A existência do componente é verificada em tempo de compilação.** A
  macro procura por
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; se nenhum
  arquivo casar, o build falha com uma sugestão "você quis dizer…?"
  tirada dos nomes de arquivo reais em disco. Paths aninhados funcionam
  da mesma forma - `inertia_response!(&req, "Admin/Dashboard", …)`
  resolve `frontend/src/pages/Admin/Dashboard.svelte` (ou a extensão do
  seu frontend).
- **A macro expande para um `Result` com `await`.** Seu handler deve
  retornar [`Response`](error-model.md) (que é
  `Result<HttpResponse, HttpResponse>`) ou outro tipo que absorva
  `FrameworkError` através de `?` / `From`. Falhas durante a
  serialização de props ou a construção da resposta são retornadas como
  `Err`, não como panics.

### Props no estilo JSON

Para prototipagem e páginas minúsculas você pode pular o struct tipado:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

A macro ainda valida o arquivo do componente. O trade-off é que você
perde a chain de props tipadas - sem `#[derive(InertiaProps)]`, sem
geração automática de TypeScript, sem verificação em tempo de
compilação de que o formato esperado pelo frontend corresponde.

### Override opcional de config

A macro aceita um `InertiaConfig` opcional ao final, para overrides por
resposta (configurações de SSR diferentes, um título padrão customizado
para uma página):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

A maioria dos apps registra uma única config no boot via [`Inertia::install`](#bootstrap-inertia-install)
e nunca toca neste argumento - a config instalada já é o ponto de
partida de toda resposta. Passe uma aqui somente para sobrescrever a
config instalada para uma única página.

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

A macro cobre props eager tipadas. Todo o resto - lazy, optional,
deferred, mesclável, cacheado no cliente, flash, overrides de
criptografia de histórico - usa o builder diretamente:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: a closure roda apenas quando a prop for de fato enviada
        // (visita inicial, ou reload parcial que pede esta chave).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: nunca enviada em visitas iniciais; o cliente precisa
        // pedir a chave explicitamente via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: pulada na renderização inicial; o cliente emite um XHR
        // de acompanhamento e a closure roda então.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: anexa ao existente em reloads parciais ("carregar mais").
        .merge("rows", next_page().await?)
        // Once: cacheada no cliente entre navegações; o resolver é pulado
        // nas visitas seguintes, a menos que o servidor force refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: toast de uso único; aparece sob `page.flash`, não `props`.
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
| `.lazy(k, ‖)` | O resolver roda apenas quando a prop for enviada | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca na visita inicial; precisa ser solicitada explicitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Pulada na visita inicial; um XHR de acompanhamento dispara a resolução | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina com o estado existente do cliente em reloads parciais | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | O cliente faz cache entre navegações | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Paginação com scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor de uso único sob `page.flash` (não `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` padrão para o shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Criptografia de histórico por resposta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Força a rotação da chave de histórico **nesta** página | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Mantém o `#fragment` depois de uma visita Inertia | `Inertia::preserveFragment()` |

Métodos eager do builder têm irmãos `try_*` (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_flash`) que retornam
`Result<Self, FrameworkError>` quando a impl `Serialize` de um valor
pode falhar em runtime - os métodos infalíveis convertem o panic em um
500 via [o limite de panic](error-model.md), então recorra a `try_*`
quando você preferir tratar a falha explicitamente.

`.clear_history()` marca a resposta que você está construindo. Um
handler de logout redireciona, e o navegador descarta a resposta do
redirect - então é a página de login, não a resposta de logout, que
precisa carregar a flag. `App::clear_history()` é a solução para esse
caso - é uma função livre, não um método do builder, então não está na
tabela acima. Ela faz flash de uma flag de sessão de uso único que o
próximo objeto de página Inertia transforma em `clearHistory: true`.
Ela precisa de um scope de sessão, e sobrevive a exatamente um salto.

Chame-a **depois** de `Auth::logout()` /
`Auth::logout_and_invalidate()`, não antes - a invalidação faz flush da
sessão inteira, e a flag vive nessa sessão, então fazer flash dela
primeiro só leva a ela ser apagada pelo flush:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Estratégias de merge e scroll infinito

`.merge` (append), `.merge_prepend`, e `.deep_merge` cobrem os casos
comuns de "carregar mais". Para fazer merge por diferença - atualizar
linhas que o cliente já tem em vez de duplicá-las - recorra a
`.merge_with` com um `MergeStrategy` explícito carregando uma chave
`match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // a nova fatia de página
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` nomeia o campo pelo qual o cliente deduplica (emitido no
objeto de página como `matchPropsOn`), então um refetch que se sobrepõe
à janela atual substitui as linhas correspondentes no lugar em vez de
anexar cópias. `Prepend` e `Deep` aceitam o mesmo `match_on`.

Scroll infinito é a mesma maquinaria com metadados de paginação
anexados. `.scroll` / `.scroll_with` - ou `.paginate`, que adapta um
`LengthAwarePaginator` ou `CursorPaginator` diretamente - emitem
`scrollProps` ao lado dos dados, e o componente `<InfiniteScroll>` do
cliente conduz os fetches de próximo/anterior:

```rust
// `posts` é um CursorPaginator vindo do construtor de consultas.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

O framework lê a direção do merge no header de solicitação
`X-Inertia-Infinite-Scroll-Merge-Intent` que o cliente envia (`append`
ao rolar para baixo, `prepend` ao rolar para cima). Numa visita nova -
sem header de intent - `scrollProps["posts"].reset` é `true`, então o
cliente limpa seu acumulador antes de renderizar a primeira janela.

## Reloads parciais

O cliente do Inertia 3 pode solicitar um subconjunto das props de uma
página (ou um superconjunto, incluindo uma chave Optional ou Defer). O
protocolo usa três headers de solicitação:

| Header | Significado |
|---|---|
| `X-Inertia-Partial-Component` | O componente que está sendo recarregado parcialmente - precisa casar com o componente da resposta para que a filtragem se aplique. |
| `X-Inertia-Partial-Data` | Whitelist: chaves de prop separadas por vírgula a incluir. |
| `X-Inertia-Partial-Except` | Blacklist: chaves de prop separadas por vírgula a excluir. Vence a `Partial-Data` numa colisão de chave. |

Regras de filtragem:

- Props `Eager`, `Lazy`, `Merge`, `Once` e `Scroll` seguem a semântica
  de whitelist / blacklist.
- Props `Always` são enviadas de qualquer forma.
- Props `Optional` e `Defer` nunca aparecem numa visita padrão e só
  aparecem num reload parcial correspondente que liste a chave
  explicitamente.

O handler não precisa fazer nada de especial - registre toda prop
através do builder, e o framework consulta os headers ao serializar o
objeto de página.

O cache do lado do cliente de uma prop `once` é respeitado somente numa
visita Inertia **completa**. Num reload parcial que nomeia a chave
(`router.reload({ only: ['stats'] })`), o resolver roda e o valor é
enviado - o cliente pediu justamente porque quer um valor novo, e
respeitar ali a alegação de cache obsoleto dele não retornaria nada
para a chave que ele pediu.

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

## Flash e redirects

Dados de flash são estado de uso único que deve aparecer na próxima
renderização e sumir depois - mensagens toast, IDs de "recém-criado",
resumos de validação. O Suprnova os expõe sob `page.flash` em toda
resposta Inertia. Há três escritores:

```rust
// 1. Empurra para o flash bag da solicitação atual.
App::flash("toast", "Saved");

// 2. Anexa a uma resposta específica (mesmo efeito, só nesta resposta).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carrega através de um redirect via a facade Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

A forma `Redirect::with(key, value)` é o caminho entre handlers: o
valor cai na sessão sob `_flash.new.*`, o
[`SessionMiddleware`](csrf.md) da próxima solicitação o envelhece para
`_flash.old.*`, e o `InertiaResponse` do destino o expõe sob
`page.flash`.

O flash da mesma solicitação (o conjunto task-local) vence o flash de
sessão herdado numa colisão de chave, então um handler de destino pode
sobrescrever um valor de entrada apenas refazendo o flash da chave.

Chaves internas de sessão (qualquer coisa prefixada com `_`) são
filtradas de `page.flash` - `_old_input` para repopulação de formulário
e as flags de protocolo `_inertia.*` não vazam para o cliente.

### Helpers de Redirect

`Redirect` é a superfície completa do Laravel:

```rust
Redirect::to("/dashboard")                       // 302 para um path
Redirect::route("posts.show").with("id", "42")   // rota nomeada, params de rota
Redirect::back("/")                              // URL anterior registrada na sessão
Redirect::refresh()                              // mesma URL, GET novo
Redirect::guest(&req, "/login")                  // guarda a URL pretendida
Redirect::intended("/dashboard")                 // recupera a URL guardada
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL assinada
Redirect::to("/posts/42").preserve_fragment()    // mantém #frag na visita
```

Todas as variantes de `Redirect` aceitam `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name, map)`,
`.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`, etc. A
chain completa espelha o `RedirectResponse` do Laravel.

Para visitas Inertia que não são GET, o framework converte a resposta
automaticamente para `303 See Other` quando o
[`Inertia303Middleware`](#bootstrap-inertia-install) está instalado, para
que o navegador emita um GET de acompanhamento limpo em vez de reenviar
o PUT/PATCH/DELETE original para o alvo do redirect.

Para mandar o visitante para **fora** do app Inertia - um provedor de
pagamento, um endpoint de authorize do OAuth, um portal de cobrança
hospedado - use `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Um XHR do Inertia recebe `409` + `X-Inertia-Location` (o cliente
executa `window.location = url`); uma navegação dura recebe um `302` +
`Location` simples. O `InertiaResponse::location(url)` nu sempre
retorna a forma 409 - use-o somente onde já se sabe que a solicitação é
uma visita Inertia, porque um navegador que segue um `409` sem header
`Location` não tem para onde ir.

## Detecção de versão

O Inertia versiona o manifesto de assets para que um cliente de vida
longa não tente montar uma página do bundle de ontem contra o servidor
de hoje. Quando o header `X-Inertia-Version` do cliente não casa com a
versão configurada no servidor, o
[`InertiaVersionMiddleware`](#bootstrap-inertia-install) responde com
`409 Conflict` e um header `X-Inertia-Location` nomeando a nova URL - o
cliente Inertia capta isso e faz um reload de página inteira, pegando o
novo bundle.

O bounce refaz o flash da sessão primeiro. O cliente responde a um 409
com um GET de página inteira, e esse GET é uma solicitação nova - sem o
refazer do flash, um erro de validação ou uma mensagem de sucesso
flashada pela solicitação anterior é envelhecida antes que a página de
destino consiga lê-la, e o usuário perde sua mensagem de erro puramente
porque um deploy aterrissou no meio do envio. Isso exige o
`SessionMiddleware` registrado antes do middleware de versão.

Você define a versão através de `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Estática - a maioria dos apps. Embuta um identificador de tempo de compilação.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dinâmica - leia um hash de manifesto, um ID de deploy de contêiner, o que for.
// A closure roda em toda verificação de versão; faça cache dentro se não for barato.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Para resolução de versão assíncrona ou falível (por exemplo, ler um
hash de manifesto do S3), faça a leitura uma vez no boot e passe a
`String` em cache para `.version(...)`.

## Bootstrap: `Inertia::install`

A maioria dos apps instala os três middlewares de protocolo em uma
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

1. Falha de forma fechada se `cfg` resolver para modo de produção
   (`development == false` - o padrão sempre que `APP_ENV=production`)
   mas nenhum manifesto do Vite puder ser carregado de
   `cfg.manifest_path`. Esta é a guarda CFG-01: um boot de produção com
   um frontend não compilado dá erro de forma explícita em vez de
   recair silenciosamente para um caminho de asset legado hardcoded.
2. Registra o `InertiaHeadersMiddleware` - define `Vary: X-Inertia` em
   toda resposta e transforma um `200` vazio numa visita Inertia em um
   `303` de volta.
3. Registra o `InertiaVersionMiddleware` - emite o `409` +
   `X-Inertia-Location` quando cliente e servidor discordam sobre a
   versão de asset.
4. Registra o `Inertia303Middleware` - promove `302` para `303` em
   redirects Inertia que não são GET.

A ordem importa: o middleware de headers é registrado primeiro, então
ele é o mais externo e vê toda resposta - incluindo o `409` que o
middleware de versão retorna antes de o handler sequer rodar.

`install` também **retém a config**. Todo `InertiaResponse` construído
depois parte dela, então `.frontend(...)`, `.version(...)`,
`.default_title(...)`, `.ssr(...)` e `.encrypt_history(...)` definidos
aqui alcançam toda página sem que um handler passe nada. Um handler que
quer configurações diferentes para uma página ainda sobrescreve com
`.with_config(...)`; um app que nunca chama `Inertia::install` recebe
`InertiaConfig::default()`; e chamar `install` de novo substitui a
config retida.

`.with_config(...)` substitui a config por inteiro, `version` incluída.
O `InertiaVersionMiddleware` ainda resolve a versão que foi dada a
`Inertia::install`, então uma config aqui que não carrega o mesmo
`.version(...)` faz o objeto de página anunciar uma versão contra a
qual o middleware vai emitir um bounce - o cliente leva um carregamento
de página inteira a mais depois de visitar aquela página. Defina
`.version(...)` no override para que casem.

Registre o `SessionMiddleware` **antes de** `Inertia::install` se você
usa dados de flash. O middleware de versão refaz o flash da sessão
antes de mandar o cliente de volta, para que um erro flashado sobreviva
ao GET de página inteira de acompanhamento; ele só consegue fazer isso
dentro de um scope de sessão.

Pule a chamada apenas se você genuinamente não quiser um desses
middlewares (raro; os três fecham modos de falha reais - envenenamento
de cache entre as duas representações de uma URL, bundle obsoleto
silencioso, e replay de formulário no redirect).

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

O Suprnova conversa com um worker SSR fora do processo - tipicamente o
bundle `createServer()` de `@inertiajs/{svelte,react,vue}/server`
rodando sob Node / Bun / Deno - por loopback HTTP. Habilite-o na config
que você entrega a [`Inertia::install`](#bootstrap-inertia-install) -
essa config é o ponto de partida de toda resposta, então não há nada
para encanar através dos seus handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // URL do worker
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

O SSR vem desligado por padrão, e é uma propriedade da config: ligado
para toda resposta construída a partir da config instalada, desligado
para qualquer resposta que sobrescreva com um `.with_config(...)` que
não o define. Quando habilitado, o framework faz post do objeto de
página para `<url>/render` e inlineia `{ head, body }` no shell HTML.
Em erro ou timeout do worker, a resposta recai para CSR (uma
`<div id="app">` vazia que o cliente hidrata) e o hook
`on_ssr_error(...)` dispara; ligue `ssr_throw_on_error(true)` no CI
para transformar essas falhas em 500s duros.

Inicialize o worker separadamente - `suprnova ssr:start` é o executor
padrão quando seu projeto passa a ter uma entrada de SSR.

## Configuração

O comportamento do Inertia é configurado programaticamente via
`InertiaConfig`, e a config que você entrega a
[`Inertia::install`](#bootstrap-inertia-install) é a que serve de ponto
de partida para toda resposta. A única variável de ambiente que o
framework lê diretamente é `SUPRNOVA_FRONTEND` (`svelte` / `react` /
`vue`), e ela só fornece o nome de arquivo padrão do ponto de entrada e
as extensões de componente de página quando a config não diz nada - um
`.frontend(Frontend::React)` explícito na config instalada vence, e é o
que `suprnova new --frontend react` cria com scaffold. Todo o resto tem
formato de builder:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // sobrescreve SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // limita o fan-out de props lazy
    .url_resolver(|req| req.path_and_query()) // como `page.url` é derivada
    .production();                            // false → carrega do servidor de dev Vite
```

Padrões específicos de cada frontend:

| Frontend | Ponto de entrada padrão | Extensões de página |
|---|---|---|
| Svelte (padrão) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### O campo `url`

`page.url` é o path **e** a query string da solicitação
(`/users?page=2&sort=name`). O cliente o escreve em `history.state`,
então é o que a navegação para trás/frente e o `router.reload()`
reproduzem - descarte a query e toda página paginada ou filtrada volta
silenciosamente para a página um. O `InertiaVersionMiddleware` também
deriva seu `X-Inertia-Location` do path e da query da solicitação,
então, por padrão, um bounce 409 de versão de asset leva o navegador
exatamente para a URL que o objeto de página nomeou.

Sobrescreva a derivação com `url_resolver` quando a URL que o cliente
deve registrar difere da que chegou - um prefixo de locale pelo qual a
SPA não roteia, ou um path que um proxy reverso reescreveu:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

O resolver lê a solicitação através de `InertiaRequestExt`, e se aplica
a toda resposta construída a partir da config que você passa a
[`Inertia::install`](#bootstrap-inertia-install) - o lugar usual para um
resolver que deve valer para o app inteiro. Sobrescreva-o para uma
única resposta com `InertiaResponse::with_config(cfg)`. Um resolver
muda apenas `page.url`. O bounce 409 continua nomeando a URL que de
fato chegou - essa é a URL que o navegador precisa buscar - então, com
um resolver no lugar, as duas divergem deliberadamente.

O manifesto do Vite em `manifest_path` é carregado de forma lazy na
primeira solicitação e mantido em cache pelo tempo de vida do
processo - toda resposta construída a partir da config instalada
compartilha esse mesmo cache, então o arquivo é lido e interpretado uma
única vez.
Quando ele está ausente, as tags de asset de produção recaem para um
caminho legado hardcoded e um `tracing::warn!` dispara para que a
lacuna apareça nos logs.

### Por que Suprnova diverge

O adaptador Inertia do Laravel tem um único registro global de "dados
compartilhados" mais uma chamada `Inertia::share($k, $v)` por
solicitação. O modelo de uma-solicitação-por-processo do PHP torna isso
seguro: um processo novo por solicitação significa nenhum vazamento
entre visitantes concorrentes.

O modelo de processo do Rust é o oposto - um processo serve muitas
solicitações concorrentes em muitas threads. Então o registro vive no
[contêiner](container.md) (task-local → thread-local → global), não em
statics globais de processo. `App::inertia_share*` escreve no
`InertiaRegistry` do contêiner ativo, o que dá aos testes que usam
`TestContainer::fake()` um isolamento limpo sem precisar cancelar o
registro de nada. Mesma superfície do Laravel; maquinaria diferente por
baixo, porque o runtime é diferente.

Outras cinco escolhas com formato de Rust que vale sinalizar:

- **Resolvers de props lazy rodam concorrentemente**, limitados por
  `max_concurrent_resolvers` (padrão 16). Uma página com doze props
  lazy emite doze consultas paralelas dentro de uma única task do
  Tokio - foi para isso que construímos o framework sobre o Tokio.
  Ajuste o limite se uma página tem muitas props lazy, cada uma batendo
  em um serviço externo.
- **A verificação de componente em tempo de compilação** não é um
  recurso do Laravel de forma alguma, porque o PHP não consegue ver
  seus arquivos de frontend em tempo de compilação. O Suprnova
  consegue, então um erro de digitação em
  `inertia_response!("Dashbaord", …)` faz o build falhar com uma
  sugestão "você quis dizer Dashboard?" em vez de aparecer depois como
  um "componente não encontrado" em runtime.
- **Um `200` vazio numa visita Inertia vira um `303`, não um `302`.** O
  `onEmptyResponse` do Laravel retorna `redirect()->back()` (um 302) e
  depende da sua conversão posterior de `302 → 303` apenas para
  PUT/PATCH/DELETE. Um redirect substituído nunca é uma continuação do
  método original - o cliente tem que emitir um GET - então o Suprnova
  diz `303` diretamente em vez de deixar visitas GET num 302 que o
  cliente seguiria com o verbo original.
- **`Inertia::location($url)` são dois métodos aqui, não um.**
  `location(url)` mantém o contrato sempre-`409` do Laravel - ele
  antecede a forma ciente da solicitação, e consumidores presos a uma
  tag dependem desse formato não mudar. `location_for(&req, url)` é a
  forma mais nova, ciente da solicitação: `409` para um XHR do Inertia,
  `302` simples para uma navegação dura. Recorra a `location_for` em
  código novo.
- **`Inertia::clearHistory()` também são dois métodos aqui, não um.** O
  `.clear_history()` no builder marca uma única resposta;
  `App::clear_history()` faz flash da flag na sessão para que ela
  sobreviva a um redirect. O Laravel se safa com um método porque já é
  apoiado em sessão - o Suprnova mantém a forma local à resposta como
  padrão (sem dependência de sessão) e torna o caso entre redirects um
  opt-in explícito.

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
