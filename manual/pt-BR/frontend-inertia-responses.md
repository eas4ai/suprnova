# Respostas Inertia

Respostas Inertia são como um handler do Suprnova envia estado para um
componente de página Svelte / React / Vue. Todo handler que renderiza
uma página Inertia retorna uma, construída ou através da macro
[`inertia_response!`](#the-inertia_response-macro) (para props eager
tipadas, verificadas em tempo de compilação) ou do builder
[`InertiaResponse`](#the-inertiaresponse-builder) (para todo o resto -
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

Para uma página sem lógica alguma - sobre, termos, privacidade - pule o
handler inteiro e declare a rota:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

Veja [Rotas](routing.md#router-level-redirects-and-views). O componente
ali é uma string de runtime, portanto não recebe a verificação de
existência em tempo de compilação desta macro - essa é a troca por não
escrever o handler.

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

A maioria dos apps registra uma única config no boot via [`Inertia::install`](#bootstrap-inertiainstall)
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
| `.always_with(k, ‖)` | Resolver assíncrono, ignora os filtros de reload parcial | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | O resolver roda apenas quando a prop for enviada | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca na visita inicial; precisa ser solicitada explicitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Pulada na visita inicial; um XHR de acompanhamento dispara a resolução | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina com o estado existente do cliente em reloads parciais | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | O cliente faz cache entre navegações | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate` (via `Inertia::paginate`) | Paginação com scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor de uso único sob `page.flash` (não `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` padrão para o shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Criptografia de histórico por resposta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Força a rotação da chave de histórico **nesta** página | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Mantém o `#fragment` depois de uma visita Inertia | `Inertia::preserveFragment()` |

Métodos eager do builder têm irmãos `try_*` (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_scroll_wrapped`, `try_flash`) que retornam
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

### Compondo flags em uma prop

Os métodos acima definem cada um uma flag. Uma prop pode carregar várias, e
algumas combinações são como o protocolo Inertia espera que páginas reais
funcionem: uma lista deferred que anexa ao que o cliente já renderizou, uma
prop de merge que o cliente armazena em cache entre navegações, uma prop
optional com sua própria chave de cache. Construa a prop com `Prop` e anexe-a
com `.prop(key, prop)`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

Essa prop é pulada na primeira renderização e anunciada em
`deferredProps`. O cliente emite sua solicitação de acompanhamento, o
resolver roda e o valor chega com uma instrução `mergeProps`, então ele é
anexado à lista já na tela em vez de substituí-la.

As flags se dividem em cinco grupos:

| Grupo | Métodos | Efeito |
|---|---|---|
| Visibilidade | `.always()`, `.optional()`, `.defer()` | Mutuamente exclusivos; a última chamada vence |
| Detalhes de defer | `.group(name)`, `.rescue()` | Lidos somente quando a prop é deferred |
| Merge | `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on(fields)`, `.merge_with_path(path)` | Como o cliente incorpora o valor e em qual caminho |
| Cache do cliente | `.once()`, `.as_key(key)`, `.until(ms)`, `.fresh()` | Se o cliente mantém o valor entre navegações |
| Scroll | `.scroll(metadata)`, `.scroll_wrap(key)` | Entrada `scrollProps` de scroll infinito mais metadados de merge incondicionais; `.scroll_wrap` é lido somente quando `.scroll` está definido |

As fontes são `Prop::eager(value)`, `Prop::lazy(closure)`,
`Prop::from_resolver(resolver)` para um resolver que você mesmo criou, e
`Prop::absent()` para uma prop que nunca chega à resposta - é o que
`when_loaded!` retorna para uma relação não carregada.

Duas regras merecem ser conhecidas antes de compor:

- **Visibilidade é uma configuração, não três flags.** `.always().optional()`
  é uma prop optional, e `.optional().always()` é uma prop always.
  Nenhuma é um erro; a chamada anterior é apagada.
- **Os metadados seguem as listas de reload parcial, não o valor.** As
  entradas `mergeProps`, `onceProps` e `scrollProps` de uma prop são emitidas
  sempre que a chave passa por `X-Inertia-Partial-Data` e
  `X-Inertia-Partial-Except`, mesmo numa visita em que o próprio valor é
  retido. É isso que transporta a instrução de merge entre as duas
  solicitações de uma prop deferred. Duas consequências decorrem disso:
  - Uma prop `.always().merge()` fora do conjunto solicitado ainda envia seu
    valor e não envia sua instrução de merge, então o cliente substitui em vez
    de anexar.
  - `scrollProps` tem uma condição extra além das listas: uma prop
    `.scroll().defer()` anuncia sua instrução de merge numa visita não parcial,
    mas não envia cursor ali, porque ainda não há nada na tela para um cursor
    descrever. Todo reload parcial correspondente recebe o cursor,
    independentemente de essa solicitação também resolver o valor.
  - `deferredProps` é o único bloco que as listas nunca governam. Ele é
    descartado por inteiro em qualquer reload parcial correspondente, não
    importa o que as listas digam - o `resolveDeferredProps` do Laravel retorna
    `[]` no momento em que a solicitação é parcial. Um reload parcial é o
    cliente trabalhando nos anúncios que já possui, então anunciar novamente
    as chaves deixadas de fora desta rodada faria com que ele voltasse a
    buscá-las. Um reload parcial direcionado a um componente *diferente* é uma
    visita padrão para todas as gates, anúncios incluídos.

`.group(name)` e `.rescue()` são armazenados em qualquer prop, mas só são
lidos quando a prop é deferred, então `.rescue().defer()` e
`.defer().rescue()` significam a mesma coisa. Uma prop scroll obtém sua
direção de merge do header `X-Inertia-Infinite-Scroll-Merge-Intent` do
cliente, então `.merge()` e `.prepend()` numa prop scroll são redundantes e
não são lidos. `.deep_merge()` é a exceção: ele direciona a prop para
`deepMergeProps` em vez de `mergeProps`, da mesma forma que o
`ScrollProp` do Laravel.

### Estratégias de merge e scroll infinito

`.merge` (append), `.merge_prepend` e `.deep_merge` cobrem os casos comuns
de "carregar mais". Para fazer merge por diferença - atualizar linhas que o
cliente já mantém em vez de duplicá-las - use `.merge_with` com um
`MergeStrategy` explícito contendo uma chave `match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // a nova fatia de página
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` nomeia o(s) campo(s) pelo(s) qual(is) o cliente faz deduplicação
(emitido no objeto de página como `matchPropsOn`) - um campo ou vários, da
mesma forma que `Prop::match_on` (abaixo) - para que um refetch que se
sobreponha à janela atual substitua as linhas correspondentes no lugar em
vez de anexar cópias. `Prepend` e `Deep` aceitam o mesmo `match_on`.

`MergeStrategy` é a forma de uma única chamada. `Prop::merge()` /
`.prepend()` / `.deep_merge()` / `.match_on(field)` são as mesmas
configurações como flags separadas, para quando a prop também precisar de uma
flag de visibilidade ou de cache - veja
[Compondo flags em uma prop](#composing-flags-on-one-prop).

`.match_on` aceita um campo ou vários em uma única chamada -
`.match_on(["id", "slug"])` e `.match_on("id").match_on("slug")` emitem o
mesmo `matchPropsOn`.

Para fazer merge somente de parte do valor de uma prop, em vez de tudo,
nomeie o campo aninhado com `.merge_with_path`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` agora carrega `"posts.data"` em vez de `"posts"`, então somente
`props.posts.data` é incorporado ao que o cliente já mantém -
`props.posts.meta` é substituído diretamente, como qualquer prop que não seja
de merge. As chamadas se acumulam, então uma prop com dois campos que aceitam
merge pode nomear cada um independentemente. Nomear um path desativa
completamente o merge no nível raiz para essa prop - uma prop com merge por
path nunca também faz merge do valor inteiro. `match_on` compõe com um path
incluindo o path no nome do campo (`"data.id"`, não `"id"`); o framework não
o infere para você. `.deep_merge()` ignora `.merge_with_path` - um merge
profundo já recursa em todo campo aninhado, então não há nada que um path
possa restringir.

O valor de uma prop de merge também pode vir de um resolver, via
`.merge_lazy` / `.merge_lazy_with` - o irmão baseado em resolver de `.merge`
/ `.merge_with`:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

O resolver roda somente quando a prop de merge realmente será enviada -
é pulado pela filtragem de reload parcial e por `.defer()`, como qualquer
prop apoiada por resolver.

Scroll infinito usa a mesma maquinaria com metadados de paginação anexados.
`.scroll` / `.scroll_with` - ou `.paginate`, que adapta diretamente um
`LengthAwarePaginator` ou `CursorPaginator` - emitem `scrollProps` ao lado
dos dados, e o componente `<InfiniteScroll>` do cliente conduz os fetches
seguinte/anterior:

```rust
// `posts` é um CursorPaginator vindo do construtor de consultas.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Uma prop scroll sempre carrega metadados de merge, não somente num fetch de
acompanhamento: por padrão ela faz append e muda para prepend somente quando o
header `X-Inertia-Infinite-Scroll-Merge-Intent` do cliente diz isso
(`append` ao rolar para baixo, `prepend` ao rolar para cima). `reset` é
independente desse header - é `true` exatamente quando o cliente nomeou a
chave em `X-Inertia-Reset`, o mesmo header que uma prop de merge comum lê.
Uma visita nova e não filtrada não envia nenhum dos dois headers, então recebe
`reset: false` e uma instrução append, em conformidade com o Laravel.

`.merge_with_path` não tem efeito numa prop scroll - o bloco scroll que
calcula sua instrução de merge lê a única chave de wrap de
`Prop::scroll_wrap`, não a lista acumulada de paths de `.merge_with_path`,
então `.scroll(metadata).merge_with_path("data")` armazena um path que nada
lê. `.scroll_wrap` - alcançado diretamente via `.prop(...)`, ou pelo atalho
de resposta `.scroll_wrapped` abaixo - é o equivalente de aninhamento para
uma prop scroll.

Uma prop scroll também respeita `.match_on(...)`, como qualquer outra prop
de merge - alcance-a via `.prop(...)`, pois nem `.scroll` nem `.match_on`
têm um atalho combinado no nível de resposta:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

O campo de correspondência usa como chave o lugar em que a prop realmente
faz merge: a chave simples quando não há wrap (`matchPropsOn:
["users.id"]`), ou `key.wrap_key` sob `.scroll_wrap(...)`
(`matchPropsOn: ["posts.data.id"]` para uma prop envolvida em `"data"`) -
assim a entrada sempre se alinha ao path de merge que o cliente incorpora,
em vez de silenciosamente nunca encontrar correspondência.

Quando o valor da prop já é uma estrutura envolvida - `{ data: [...],
meta: {...} }`, o formato que um recurso de API construído manualmente
normalmente retorna - fazer merge do objeto inteiro substituiria `meta` em
cada fetch. Aponte o merge para o campo de array em vez disso com
`.scroll_wrapped`:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` então nomeia `posts.data`, portanto o cliente incorpora as novas
linhas no array aninhado e deixa `meta` ser substituído integralmente a cada
vez. `.scroll_with_wrapped` e `try_scroll_wrapped` são os irmãos baseados em
resolver e falíveis, correspondentes a `.scroll_with` / `try_scroll`.

Um tipo fora do módulo `pagination` deste crate - um paginator de terceiros,
um cursor feito à mão - pode se descrever para `.scroll` implementando
`ProvidesScrollMetadata` em vez de construir `ScrollMetadata` campo a campo:

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`, `Paginator` e `CursorPaginator` também implementam
isso - veja [Paginação](pagination.md#inertia-integration---infinite-scroll-props).

### Aninhamento por notação de ponto

Uma chave contendo `.` é aninhada na resposta em vez de ser enviada como uma
chave literal - a notação de ponto baseada em `Arr::set` do Laravel
(`Inertia::share('user.name', …)`, `resolveArrayableProperties`):

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

é enviada como:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

e não como duas chaves literais `"user.name"` / `"user.locale"`. Duas
chamadas que compartilham um prefixo se acumulam num único objeto; uma chave
sem ponto não é afetada. Isso se aplica a todo método que anexa props -
`.with`, `.always`, `.lazy`, chaves do registro compartilhado - e a nada mais:
ele nunca recursa no *valor* de uma prop, então um objeto de
`errors` de validação mantém quaisquer nomes de campo pontuados que carregue
internamente. Não existe uma forma de escape para uma chave que precise manter
um ponto literal (`.with("config.json", …)` ainda aninha) - isso corresponde
ao Laravel, onde `Arr::set` também não tem mecanismo de escape.

## Reloads parciais

O cliente do Inertia 3 pode solicitar um subconjunto das props de uma página
(ou um superconjunto incluindo uma chave Optional ou Defer). O protocolo usa
três headers de solicitação:

| Header | Significado |
|---|---|
| `X-Inertia-Partial-Component` | O componente que está sendo recarregado parcialmente - precisa corresponder ao componente da resposta para que a filtragem se aplique. |
| `X-Inertia-Partial-Data` | Whitelist: chaves de prop separadas por vírgula a incluir. |
| `X-Inertia-Partial-Except` | Blacklist: chaves de prop separadas por vírgula a excluir. Vence `Partial-Data` em caso de colisão de chave. |

A filtragem lê uma coisa: a visibilidade da prop, definida por `.always()`,
`.optional()` ou `.defer()`. Uma prop sem nenhuma delas tem a visibilidade
padrão.

- Props de visibilidade padrão seguem a semântica de whitelist / blacklist.
- Props `.always()` são enviadas independentemente.
- Props `.optional()` e `.defer()` nunca são enviadas numa visita padrão e só
  aparecem num reload parcial correspondente que liste explicitamente a chave.

As flags de merge e scroll não entram nisso: elas decidem como o cliente
incorpora um valor que recebe, não se recebe um valor, então uma prop
`.defer().merge()` é filtrada exatamente como uma `.defer()` simples.
`.once()` também não entra nisso, embora não seja puramente uma instrução de
incorporação - numa visita completa em que o cliente informa que o valor já
está em cache, o servidor pula o resolver e não envia valor, como descreve a
nota abaixo. O que as três mudam é quais blocos de metadados vêm junto - veja
[Compondo flags em uma prop](#composing-flags-on-one-prop).

O handler não precisa fazer nada de especial - registre toda prop através do
builder, e o framework consulta os headers ao serializar o objeto de página.

O cache do lado do cliente de uma prop `once` é respeitado somente numa visita
Inertia **completa**. Num reload parcial que nomeia a chave
(`router.reload({ only: ['stats'] })`), o resolver roda e o valor é enviado -
o cliente pediu justamente porque quer um valor novo, e respeitar ali sua
alegação de cache obsoleto não retornaria nada para a chave que ele pediu.

### only/except aninhados (notação de ponto)

As entradas de `X-Inertia-Partial-Data` e
`X-Inertia-Partial-Except` podem nomear um path dentro do valor de uma prop,
não apenas a chave da própria prop. Um cliente que chama
`router.reload({ only: ['user.name'] })` envia
`X-Inertia-Partial-Data: user.name`, e a resposta reduz a prop `user` apenas
àquele campo:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` faz a poda da mesma forma, em vez de reduzir - `router.reload({
except: ['user.email'] })` deixa todos os outros campos de `user` no lugar.

Regras:

- Uma entrada simples (`user`) ainda significa a prop inteira. Se `only`
  nomear `user` e `user.name`, o valor inteiro é enviado - a entrada simples
  vence.
- Uma entrada também pode nomear um *ancestral* de uma chave de prop
  pontuada. Uma prop registrada sob `auth.user` - por `.with("auth.user", …)`
  ou `App::inertia_share("auth.user", …)` - participa de `only: ['auth']` e
  é enviada inteira, porque o chamador pediu a raiz `auth` inteira. Um
  `except: ['auth']` simples a descarta pelo mesmo motivo. O prefixo precisa
  terminar num limite de segmento, então uma prop não relacionada
  `authAgent.user` não é afetada por nenhum dos dois.
- `except` vence num path nomeado pelos dois headers, da mesma forma que vence
  no nível superior.
- Um path que não resolve contra o valor - um campo desconhecido ou um que
  atravesse um escalar ou um array em vez de um objeto - não contribui em nada
  para aquele path, sem descartar os campos irmãos solicitados junto dele.
- Props `Always` ignoram `only` / `except` completamente, inclusive a notação
  de ponto - sempre são enviadas inteiras.
- Props `Optional` e `Defer` ainda precisam da solicitação explícita para
  resolver de todo. Uma entrada pontuada (`permissions.read`) conta como essa
  solicitação para a chave de nível superior, e o valor resolvido é reduzido
  da mesma forma que o de uma prop `Eager`.
- Um `only` pontuado contra uma prop cujo valor atual não é um objeto - uma
  string, um número, um array - reduz para `{}`, não para o valor original.
  A reconciliação do cliente só faz deep merge quando tanto o valor em cache
  quanto o recebido são objetos
  (`inertia-3.6.1/packages/core/src/response.ts` `nestedTopKeys`); um objeto
  vazio falha nessa verificação contra um cache não objeto da mesma forma que
  um objeto preenchido falharia, então o objeto vazio substitui diretamente o
  escalar em cache em vez de fazer merge nele. Evite enviar uma solicitação
  pontuada contra uma prop que não tenha formato de objeto.
- Um `except` pontuado não exclui o campo no cliente - ele impede que o campo
  seja atualizado nesta resposta, e o merge do cliente o restaura do que já
  tinha em cache. `deepMergeObjects` constrói o objeto mesclado clonando
  primeiro o valor em cache e sobrescrevendo somente as chaves que o servidor
  realmente enviou; uma chave podada pelo servidor nunca é tocada, então
  sobrevive com seu valor antigo. No primeiro carregamento dessa prop pelo
  cliente (nada ainda em cache), o campo podado fica genuinamente ausente,
  pois não há cache de fallback - o comportamento de "restaurar do cache" só
  se aplica a uma página que o cliente já tenha visto.


## Dados compartilhados via `App::inertia_share*`

Algumas props são as mesmas em toda página Inertia - estado de autenticação,
o token CSRF, a locale atual, flags de todo o app. Registre-as uma vez na
inicialização e elas se mesclam em toda resposta:

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

Chaves compartilhadas são aninhadas nos pontos da mesma forma que `.with` -
dois compartilhamentos estáticos sob `"user.name"` / `"user.age"` chegam num
único objeto `user` na resposta. Leia um valor compartilhado de volta ou limpe
inteiramente o registro estático com `App::inertia_shared` /
`App::flush_inertia_shared` - os equivalentes de
`Inertia::getShared` / `Inertia::flushShared` no Laravel:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` lê somente o registro estático - retorna `None` para uma
chave registrada via `inertia_share_lazy` / `inertia_share_once` (não há uma
solicitação contra a qual resolvê-la, espelhando o `getShared` do Laravel,
que retorna a closure bruta em vez de invocá-la) e para um compartilhamento
de provider por trait por solicitação. `flush_inertia_shared` também limpa
somente o registro estático; um provider registrado via
`register_inertia_shared` não tem estado por solicitação para limpar.

Para dados compartilhados por solicitação (o usuário autenticado, flags com
escopo de solicitação), implemente
[`InertiaSharedData`](#per-request-shared-data) e registre o
singleton - o framework chama `share(&req, component)` em toda resposta
Inertia e mescla o resultado. `component` é a página que está sendo
renderizada, então um provider pode variar sua saída por página - veja abaixo.

### Precedência em colisão de chave

Quando a mesma chave aparece em mais de uma camada, a escrita mais recente
vence:

1. Registro estático (`App::inertia_share` / `App::inertia_share_lazy`)
2. Provider por solicitação via trait (`InertiaSharedData::share`)
3. Métodos do builder por resposta (`.with`, `.lazy`, etc.)

Isso permite que um handler sobrescreva um padrão compartilhado globalmente
para uma página, sem precisar desregistrar nada.

### Dados compartilhados por solicitação

O trait executa uma vez por resposta Inertia, com acesso à solicitação
**e** ao nome do componente da página - o `RenderContext` do Laravel
(`component`, `request`), passado como parâmetro simples em vez de um struct
wrapper, já que a solicitação já cobre a outra metade. As implementações
precisam de `async_trait` (reexportado como `suprnova::__async_trait`) e
`IndexMap` (reexportado como `suprnova::indexmap`):

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
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Variar por página: somente o dashboard de admin precisa das contagens de navegação.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// Na inicialização:
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignore `component` (`_component`) se seu provider não precisar variar por
página.


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
[`Inertia303Middleware`](#bootstrap-inertiainstall) está instalado, para
que o navegador emita um GET de acompanhamento limpo em vez de reenviar
o PUT/PATCH/DELETE original para o alvo do redirect.

### Falhas de validação

Quando um handler falha na validação durante uma visita Inertia, o framework
responde `303 See Other` de volta à página do formulário com os erros em
flash, em vez do JSON `422` que um cliente REST recebe. Isso não é
cosmético: o cliente Inertia trata qualquer resposta sem um header
`X-Inertia` como não Inertia e a renderiza no modal de erro em tela inteira,
então um `422` nunca chega a `form.errors`. Nada muda no handler - a ponte é
um dos middlewares que `Inertia::install` registra.

O destino é o `Referer` da solicitação quando ele é same-origin, depois a URL
anterior registrada na sessão e, por fim, a própria URL da solicitação que
falhou. Um `Referer` cross-origin é ignorado em vez de seguido, assim como
um que apenas parece same-origin: um `//` ou `/\` inicial (um navegador lê
qualquer um como relativo ao protocolo depois de converter uma barra
invertida em barra) e qualquer byte de controle ASCII em qualquer posição do
valor (o parser de URL remove tab e newline da string inteira antes de
comparar origens, então um byte de controle pode transformar o que parece um
path seguro em uma origem diferente quando o navegador navegar até ele) fazem
o fallback da mesma maneira. A mesma verificação se aplica ao fallback final
da URL, então nem um path de solicitação incomum pode virar um redirect
off-origin.

O valor de um campo é sua **primeira** mensagem, uma string simples - o
formato que o próprio tipo `ErrorValue` do Inertia descreve e ao qual
`$page.props.errors.email` se vincula. Defina
`InertiaConfig::with_all_errors(true)` para obter todas as mensagens como um
array; o tipo no cliente então precisa da augmentation correspondente:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Vários formulários na mesma página continuam isolados: envie
`X-Inertia-Error-Bag: <name>` com a visita, e os erros são colocados em flash
sob esse bag e lidos de volta nele, chegando como
`errors.<name>.<field>`.

A prop `errors` é sempre visível por padrão, portanto um reload parcial nunca
a filtra nem reduz. `only: ['users']` ainda envia o bag, assim como
`except: ['errors']`; `only: ['errors.email']` envia o bag inteiro em vez de
somente aquele campo. Esse é o formato do Laravel - o middleware dele
compartilha o bag como `Inertia::always(...)`, e `resolveAlways` reinjeta o
valor bruto depois da reconstrução de `only` / `except`. Isso importa porque o
cliente incorpora uma resposta parcial com
`{...current.props, ...response.props}`: um objeto `errors` vazio apagaria as
mensagens que já estão na tela, enquanto um objeto não filtrado as preserva.
A regra cobre as duas fontes - o bag colocado em flash na sessão e o próprio
`.with("errors", …)` de um handler. Uma flag de visibilidade explícita ainda
vence, então `.prop("errors", Prop::eager(…).optional())` se comporta como
optional.

Isso não faz duas coisas. Não refaz o flash de input antigo - o body da
solicitação já foi consumido quando a ponte roda, e um `useForm` do Inertia
mantém seu próprio estado após um envio que falhou, então não há nada a
repopular. E nunca toca numa resposta de Precognition: um `422` de dry-run é
exatamente o que o cliente solicitou.

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

O Inertia versiona o manifesto de assets para que um cliente de vida longa
não tente montar uma página do bundle de ontem contra o servidor de hoje.
Quando o header `X-Inertia-Version` do cliente não corresponde à versão
configurada no servidor, [`InertiaVersionMiddleware`](#bootstrap-inertiainstall)
responde com `409 Conflict` e um header `X-Inertia-Location` nomeando a nova
URL - o cliente Inertia o capta e faz um reload de página inteira, obtendo o
novo bundle.

O bounce refaz o flash da sessão primeiro. O cliente responde a um 409 com um
GET de página inteira, e esse GET é uma solicitação nova - sem o refazer do
flash, um erro de validação ou uma mensagem de sucesso flashada pela
solicitação anterior é envelhecida antes que a página de destino possa
lê-la, e o usuário perde sua mensagem de erro apenas porque um deploy
ocorreu no meio do envio. Isso exige `SessionMiddleware` registrado antes do
middleware de versão.

Por padrão você não define nada: `InertiaConfig` calcula o hash do manifesto
de build do Vite (`manifest_path`, padrão
`public/assets/.vite/manifest.json`) e usa os primeiros 16 bytes de seu
SHA-256, codificados em hexadecimal. O manifesto é o único arquivo que muda
a cada build e em nenhuma outra ocasião, então a versão sobe sozinha. Quando
não há manifesto para ler - desenvolvimento local, quando o Vite serve da
memória - ele recorre à string estática `"1.0"` e registra em `debug`.

Faça um override quando quiser outra coisa:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Padrão - calcula o hash do manifesto de build. Nada para escrever.
let cfg = InertiaConfig::new();

// Um local diferente para o manifesto; a versão o acompanha.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Estática - incorpora um identificador de tempo de compilação. Sobrevive a
// uma chamada posterior a `.manifest_path(...)`: uma versão explícita é deliberada.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dinâmica - um ID de deploy de contêiner, qualquer coisa. A closure roda em
// toda verificação de versão; faça cache dentro se não for barato.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

O manifesto é lido em toda verificação de versão, assim como
`hash_file` do Laravel - alguns KB fora do cache de página, e uma
recompilação é captada imediatamente. Se você mediu isso e quer eliminá-lo,
resolva uma vez no boot:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

Para resolução de versão assíncrona ou falível (por exemplo, ler um hash de
manifesto do S3), faça a leitura uma vez no boot e passe a `String` em cache
para `.version(...)`.


## Bootstrap: `Inertia::install`

A maioria dos apps instala os quatro middlewares de protocolo em uma única
chamada, a partir de `register_http_stack` - o hook de bootstrap somente HTTP,
que o caminho do servidor executa e os binários de queue, schedule, workflow e
console pulam (veja [Bootstrap](bootstrap.md)):

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …middleware global, na ordem em que você quer que rode
}
```

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Mantenha-a fora de `bootstrap::register`. `Inertia::install` falha de forma
fechada em produção quando o manifesto do frontend compilado está ausente,
que é exatamente o estado de uma imagem de worker ou console que não envia
`public/assets` - então instalá-la pelo hook de escopo do processo derruba
esses binários junto.

`Inertia::install` retorna `Result` e, em ordem:

1. Falha de forma fechada se `cfg` resolver para modo de produção
   (`development == false` - o padrão sempre que `APP_ENV=production`), mas
   nenhum manifesto Vite puder ser carregado de `cfg.manifest_path`. Esta é
   a guarda CFG-01: um boot de produção com frontend não compilado dá erro
   explicitamente em vez de recair silenciosamente num caminho de asset
   legado hardcoded.
2. Registra `InertiaHeadersMiddleware` - define `Vary: X-Inertia` em toda
   resposta e transforma um `200` vazio numa visita Inertia em um `303` de
   volta.
3. Registra `InertiaVersionMiddleware` - emite `409` +
   `X-Inertia-Location` quando cliente e servidor discordam sobre a versão do
   asset.
4. Registra `Inertia303Middleware` - promove `302` para `303` em redirects
   Inertia que não são GET.
5. Registra `InertiaValidationRedirectMiddleware` - transforma um `422` numa
   visita Inertia em um `303` de volta à página do formulário com os erros em
   flash. Veja [Falhas de validação](#validation-failures).

A ordem importa: o middleware de headers é registrado primeiro, então é o
mais externo e vê toda resposta - incluindo o `409` que o middleware de
versão retorna antes que o handler sequer rode. O middleware de redirect de
validação é registrado por último, então é o mais interno - mais próximo do
handler - e vê um `422` antes que os outros três middlewares tenham chance de
tocá-lo.

`install` também **retém a config**. Todo `InertiaResponse` construído depois
parte dela, então `.frontend(...)`, `.version(...)`, `.default_title(...)`,
`.ssr(...)` e `.encrypt_history(...)` definidos aqui alcançam toda página sem
que um handler passe qualquer coisa. Um handler que quer configurações
diferentes para uma página ainda sobrescreve com `.with_config(...)`; um app
que nunca chama `Inertia::install` recebe `InertiaConfig::default()`; e chamar
`install` novamente substitui a config retida.

`.with_config(...)` substitui a config por inteiro, incluindo `version`.
`InertiaVersionMiddleware` ainda resolve a versão que recebeu de
`Inertia::install`, então uma config aqui que não carregue o mesmo
`.version(...)` faz o objeto de página anunciar uma versão que o middleware
vai rejeitar - o cliente faz um carregamento de página inteira adicional
depois de visitar essa página. Defina `.version(...)` no override para que
correspondam.

Registre `SessionMiddleware` **antes de** `Inertia::install` se usar dados de
flash. O middleware de versão refaz o flash da sessão antes de devolver o
cliente, para que um erro flashado sobreviva ao GET de página inteira de
acompanhamento; ele só consegue fazer isso dentro de um escopo de sessão.

Pule a chamada somente se você genuinamente não quiser um desses middlewares
(raro; os quatro fecham modos de falha reais - envenenamento de cache entre
as duas representações de uma URL, bundle obsoleto silencioso,
reenvio de formulário em redirect e um `422` de validação terminando no
modal de erro do cliente em vez de chegar a `form.errors`).


## Elementos `<head>` controlados pelo servidor

O Inertia 3.5 adicionou uma opção de cliente para deixar o servidor decidir o
que vai em `<head>` - útil quando as meta tags dependem do registro que você
acabou de carregar e você não quer que o título e as tags OG vivam em dois
lugares.

Isso não precisa de nenhum suporte do framework. O cliente lê os elementos a
partir de uma **prop comum**, então qualquer handler pode fornecê-los:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
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
`data-inertia` em qualquer coisa que não tenha um, para poder fazer diff de
elementos de head entre navegações; forneça seu próprio
`data-inertia="og-title"` quando quiser identidade estável em vez de
correspondência posicional.

Escape qualquer coisa interpolada a partir de dados do usuário - essas
strings são injetadas como HTML, então as regras usuais se aplicam.


## SSR

O Suprnova conversa com um worker SSR fora do processo - tipicamente o bundle
`createServer()` de `@inertiajs/{svelte,react,vue}/server` executado sob
Node / Bun / Deno - por HTTP de loopback. Habilite-o na config que você
entrega a [`Inertia::install`](#bootstrap-inertiainstall) - essa config é o
ponto de partida de toda resposta, então não há nada para encanar através dos
seus handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // URL do worker
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

O SSR vem desligado por padrão e é uma propriedade da config: ligado para
toda resposta construída a partir da config instalada, desligado para
qualquer resposta que sobrescreva com um `.with_config(...)` que não o
defina. Quando habilitado, o framework faz POST do objeto de página para
`<url>/render` e inclui `{ head, body }` no shell HTML. Em caso de erro ou
timeout do worker, a resposta recai para CSR (uma `<div id="app">` vazia que
o cliente hidrata) e o hook `on_ssr_error(...)` dispara; ative
`ssr_throw_on_error(true)` no CI para transformar essas falhas em 500s
definitivos.

Antes de despachar qualquer coisa, o gateway pode verificar se o bundle SSR
compilado existe no disco - ative com `.ssr_bundle_path(...)`, apontando para
o convencional `frontend/bootstrap/ssr/ssr.js` (a verificação em si fica
ativada por padrão, `.ssr_ensure_bundle_exists(true)`, mas não tem efeito até
que um path seja definido - isso é deliberadamente não autodetectado, então
habilitar SSR contra um dublê de teste nunca exige também criar um bundle no
disco). Um bundle ausente recai imediatamente para CSR, sem pagar
`ssr_timeout` numa conexão que nunca teria sucesso. Isso espelha a config
`ensure_bundle_exists` do Laravel.

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` cria por scaffold `frontend/src/ssr.{ts,tsx}` e um script npm
`build:ssr` para cada starter. Faça o build e então inicialize o worker:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` verifica se o worker realmente responde - acessa a
própria rota `GET /health` do worker, que todo bundle `createServer()` expõe
sem código adicional.


## Configuração

O comportamento do Inertia é configurado programaticamente via
`InertiaConfig`, e a config que você entrega a
[`Inertia::install`](#bootstrap-inertiainstall) é a que serve de ponto
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
    .with_all_errors(false)                   // uma mensagem por campo, ou todas
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
[`Inertia::install`](#bootstrap-inertiainstall) - o lugar usual para um
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
compartilhados", além de uma chamada `Inertia::share($k, $v)` por solicitação.
O modelo de uma-solicitação-por-processo do PHP torna isso seguro: um processo
novo por solicitação significa nenhum vazamento entre visitantes concorrentes.

O modelo de processo do Rust é o oposto - um processo serve muitas
solicitações concorrentes em muitas threads. Então o registro vive no
[contêiner](container.md) (task-local → thread-local → global), não em
statics globais do processo. `App::inertia_share*` escreve no
`InertiaRegistry` do contêiner ativo, o que oferece aos testes que usam
`TestContainer::fake()` isolamento limpo sem precisar desregistrar nada. A
mesma superfície do Laravel; maquinaria diferente por baixo porque o runtime
é diferente.

Nove outras escolhas com formato de Rust que vale sinalizar:

- **Resolvers de props lazy rodam concorrentemente**, limitados por
  `max_concurrent_resolvers` (padrão 16). Uma página com doze props lazy emite
  doze consultas paralelas dentro de uma única task do Tokio - foi para isso
  que construímos o framework sobre o Tokio. Ajuste o limite se uma página
  tiver muitas props lazy, cada uma acessando um serviço externo.
- **A verificação de componente em tempo de compilação** não é um recurso do
  Laravel de forma alguma, porque o PHP não consegue ver seus arquivos de
  frontend em tempo de compilação. O Suprnova consegue, então um erro de
  digitação em `inertia_response!("Dashbaord", …)` faz o build falhar com
  uma sugestão "você quis dizer Dashboard?" em vez de aparecer depois como
  um "componente não encontrado" em runtime.
- **Um `200` vazio numa visita Inertia vira um `303`, não um `302`.** O
  `onEmptyResponse` do Laravel retorna `redirect()->back()` (um 302) e
  depende da conversão posterior de `302 → 303` apenas para
  PUT/PATCH/DELETE. Um redirect substituído nunca é uma continuação do
  método original - o cliente tem que emitir um GET - então o Suprnova diz
  `303` diretamente em vez de deixar visitas GET num 302 que o cliente
  seguiria com o verbo original.
- **`Inertia::location($url)` são dois métodos aqui, não um.**
  `location(url)` mantém o contrato sempre-`409` do Laravel - ele é anterior
  à forma ciente da solicitação, e consumidores vinculados a uma tag dependem
  que esse formato não mude. `location_for(&req, url)` é a forma mais nova,
  ciente da solicitação: `409` para um XHR do Inertia, `302` simples para uma
  navegação dura. Use `location_for` em código novo.
- **`Inertia::clearHistory()` também são dois métodos aqui, não um.**
  `.clear_history()` no builder marca uma resposta individual;
  `App::clear_history()` faz flash da flag na sessão para que ela sobreviva
  a um redirect. O Laravel se safa com um método porque ele já é apoiado em
  sessão - o Suprnova mantém a forma local à resposta como padrão (sem
  dependência de sessão) e torna o caso entre redirects um opt-in explícito.
- **`.lazy()` não é o `Inertia::lazy()` do Laravel.** O método do Laravel
  está deprecated e se comporta como `optional()` - `LazyProp` é um alias
  direto de `OptionalProp`, pulado por inteiro na visita inicial
  (`ResponseFactory.php:174-181`). O `.lazy()` do Suprnova é a convenção de
  closure simples que o próprio Laravel usa para uma prop callable sem
  wrapper algum - incluída sempre que a filtragem de reload parcial deixa a
  chave passar, inclusive em visitas padrão. Use `.optional()` para o
  comportamento pulado na visita inicial que o nome "lazy" sugere quando você
  vem do Laravel.
- **`only` / `except` aninhados fazem a redução depois da resolução, não
  antes.** `Response::resolvePartialProperties` do Laravel percorre o path
  pontuado pelo array de props bruto e ainda não resolvido, então um path para
  dentro de uma `LazyProp` ou `DeferProp` degrada para `null` - o percurso
  encontra uma closure não resolvida e para
  (`inertia-laravel-2.0.25/src/Response.php:273-297`). O Suprnova resolve
  primeiro o valor de toda prop - os resolvers são async, então não há um
  ponto síncrono onde todas sejam arrays simples como às vezes ocorre no
  Laravel - e depois reduz o valor JSON resultante. Um path aninhado
  desconhecido ou incompatível por tipo é descartado em vez de ser enviado
  como `null`, correspondendo ao que a própria reconciliação do cliente
  espera: ela faz deep merge de um objeto reduzido sobre o que já mantém
  (`inertia-3.6.1/packages/core/src/response.ts:414-425`), e um `null` solto
  apagaria um campo que o cliente já possui em vez de deixá-lo intacto.
- **`.scroll_wrapped` é opt-in, não automático.** O
  `Inertia::scroll($value, $wrapper = 'data', …)` do Laravel aninha por
  padrão a instrução de merge de toda prop scroll sob `"data"`, porque um
  recurso paginator do Laravel normalmente retorna
  `{ data: [...], links: {...}, meta: {...} }` e somente o array deve fazer
  merge. Os paginators integrados do Suprnova devolvem um array simples de
  linhas (`Vec<T>`, sem envelope), então `.scroll` / `.paginate` fazem merge
  na raiz da prop, e `.scroll_wrapped` existe para os casos que precisam do
  path aninhado.
- **Uma prop scroll envolvida prefixa os campos `match_on` para você.** Numa
  prop `.scroll_wrapped("posts", "data")`, `match_on("id")` emite
  `"posts.data.id"`. O Laravel emite o `"posts.id"` sem prefixo, que o
  próprio cliente não consegue alinhar ao alvo de merge, portanto a
correspondência nunca é acionada. O ponto de aninhamento é inequívoco aqui -
uma prop scroll tem no máximo um wrapper - então o Suprnova deriva o
prefixo em vez de fazer você digitá-lo. Escreva o nome do campo simples, não o path.


## Próximos passos

- [Componentes de página](frontend-pages.md) - como o frontend resolve
  um nome de componente para um módulo Svelte / React / Vue
- [Tipos TypeScript](frontend-typescript-types.md) - `suprnova generate-types` emite definições TS a partir dos seus structs
  `#[derive(InertiaProps)]`
- [Objetos Data](data.md) - `#[derive(Data)]` para DTOs com controle de
  include/allowlist por campo que compõe com reloads parciais
- [Modelo de erros](error-model.md) - como `Response`, o limite de
  panic, e `FrameworkError` atravessam as respostas Inertia
- [Contêiner](container.md) - o modelo de busca por trás de
  `App::inertia_share*` e `InertiaSharedData`
