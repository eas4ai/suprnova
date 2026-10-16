# Contexto

`Context` é o conjunto de chave/valor por solicitação do Suprnova. É onde
você guarda dados que quer que todo chamador abaixo, dentro da mesma
solicitação, enxergue - um request id, um slug de tenant, um papel de
usuário, uma trilha de auditoria - sem levar o valor à mão por cada
assinatura de função. É o equivalente Suprnova da facade `Context` do
Laravel.

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

Recorra a ele quando:

- Uma linha de log, um job enfileirado, ou uma mensagem de transmissão
  precisa de metadados com escopo de solicitação (id de tenant, id de
  correlação, papel de usuário)
- Um helper profundamente aninhado precisa de um valor que o handler já
  tem, mas a cadeia de chamadas não deveria carregar um parâmetro por
  cada camada
- Você quer ler a query string da solicitação atual (`?page=3`,
  `?cursor=…`) a partir de código que não é um handler

`Context` **não** serve para estado entre solicitações. Ele está
vinculado à task Tokio atual e desaparece quando a solicitação termina.
Para coisas que sobrevivem a uma solicitação, use o
[Contêiner de serviços](container.md) ou o [Cache](cache.md).

## Os dois conjuntos

Todo escopo `Context` ativo carrega dois mapas de chave/valor e um slot
extra:

| Conjunto | Lido com | Aparece em `Context::all()` |
|---|---|---|
| **Visível** | `Context::get` | Sim |
| **Oculto** | `Context::hidden_get` | Não |
| **Query** | `Context::query_param` | Não (snapshot separado dos pares `?key=value` da URL) |

A separação entre visível e oculto é todo o motivo de existirem dois
conjuntos: serializadores de log que despejam `Context::all()` em saída
estruturada não vazam dados que você escondeu de propósito. Coloque
metadados de auditoria no conjunto visível; coloque chaves de API, tokens
bearer de OAuth e dados pessoais que você não quer nos logs no conjunto
oculto.

O conjunto de query é populado automaticamente pelo middleware de
solicitação do framework a partir da query string da URL (veja
[A paginação lê parâmetros de query](#a-paginação-lê-parâmetros-de-query)
abaixo). Você normalmente só o lê, nunca escreve nele.

## O escopo ativo

Um escopo `Context` é instalado pelo framework em toda solicitação HTTP
que chega. Dentro de um handler, de um middleware, de um observer de
modelo, de um event listener, ou de qualquer outra coisa alcançável a
partir da task da solicitação, o escopo está vivo e as leituras e
escritas de `Context::*` funcionam sem cerimônia.

Fora de um escopo - código de boot inicial, um `tokio::spawn` puro que
não herda contexto, um teste unitário que não instala nenhum - toda
mutação é um **no-op silencioso** e toda leitura retorna `None`. O
contrato é: nunca um panic, não importa de onde você chame.

```rust
// Em um handler - o escopo está ativo, tudo funciona:
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// Fora de um escopo - no-op silencioso + None:
Context::add("user_id", 42i64);            // descartado
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

O contrato de nunca dar panic é deliberado. Código de biblioteca que toca
`Context` (um subscriber de log customizado, uma extensão de SDK) não
deveria precisar saber se está rodando dentro de uma solicitação ou no
boot - ele deveria simplesmente chamar `Context::get` e tratar `None`
como "não disponível no momento".

### Observabilidade para operações silenciosas

Um no-op verdadeiramente silencioso esconderia bugs (middleware fora de
ordem, contexto não propagado para uma task criada com spawn, leitura
acidental em tempo de boot). As operações de mutação do framework
continuam sem dar panic, mas emitem um evento `tracing::trace!` no target
`suprnova::context` sempre que descartam algo:

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

Três classes de evento:

| Evento | Quando dispara |
|---|---|
| `mutation discarded: no active scope` | `add`, `push`, `hidden_add`, `forget` chamados fora de qualquer escopo |
| `mutation discarded: value failed to serialize` | o impl `Serialize` do valor de `add`/`push`/`hidden_add` deu erro |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` acharam a chave, mas o JSON armazenado não corresponde ao `T` pedido |

A ausência simples - `get` em uma chave que nunca foi definida - continua
silenciosa, para que sondagens de "isto está definido?" não inundem os
logs. Habilite `RUST_LOG=suprnova::context=trace` quando suspeitar de um
bug de propagação; o caminho do no-op silencioso fica visível sem mudar
como o código de produção se comporta.

## Adicionando valores

### `Context::add` - substitui em uma chave

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // qualquer valor Serialize
```

A chave é `Into<String>`; o valor é qualquer tipo `Serialize`. O valor é
convertido para `serde_json::Value` uma vez no momento da escrita e
armazenado assim. Um `add` posterior na mesma chave substitui.

### `Context::push` - acrescenta a uma pilha

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` inicializa um array vazio na primeira chamada e acrescenta nas
chamadas seguintes. Se já existe um escalar na chave, ele é convertido
para um array `[scalar, new_value]` - `push` é tolerante com `add`s
anteriores na mesma chave.

### `Context::hidden_add` - escreve no conjunto oculto

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// Um despejo do conjunto visível (um emissor de log JSON, por exemplo)
// não os enxerga:
let all = Context::all();
assert!(!all.contains_key("api_key"));

// Mas você ainda pode lê-los deliberadamente:
let key: Option<String> = Context::hidden_get("api_key");
```

O conjunto oculto é chaveado de forma independente do conjunto visível -
um `hidden_add("user_id", 99)` e um `add("user_id", "alice")` coexistem
sem colisão. `Context::forget(key)` remove dos dois conjuntos em uma
única chamada.

## Lendo valores

### `Context::get` - leitura tipada do conjunto visível

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` é genérico sobre `T: DeserializeOwned`. O valor JSON armazenado é
desserializado a cada leitura. Retorna `None` quando:

- A chave não está definida
- Nenhum escopo está ativo na task atual
- O valor armazenado não desserializa para `T` (por exemplo, você
  armazenou um `i64` e pediu uma `String`)

O último caso emite um `tracing::trace!` para que o bug de tipo errado
seja observável - `Context::get` parecendo dizer "o valor não está
definido" quando na verdade diz "o valor tem a forma errada" é o tipo de
bug que custa uma hora para achar sem uma linha de log apontando para
ele.

### `Context::hidden_get` - leitura tipada do conjunto oculto

Mesma forma que `get`, lendo o conjunto oculto. Mesmo comportamento de
tracing para o tipo errado.

### `Context::has` - verificação de existência no conjunto visível

```rust
if Context::has("user_id") {
    // …
}
```

`has` só verifica o conjunto visível (use `hidden_get(...).is_some()` se
precisar sondar o conjunto oculto).

### `Context::all` - snapshot do conjunto visível

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

Retorna um `HashMap` vazio fora de um escopo. É isso que um emissor de
log JSON deveria chamar para injetar campos com escopo de solicitação em
toda linha de log - e é por isso que o conjunto oculto existe
separadamente.

### `Context::forget` - remove uma chave dos dois conjuntos

```rust
Context::forget("trail");          // remove do visível E do oculto
```

A remoção nos dois conjuntos é intencional. Se você armazenou dados
relacionados nos dois (por exemplo, `user_id` visível, `user_email`
oculto), um único `forget` limpa os dois.

## Lendo parâmetros de query

`Context::query_param` lê dos pares `?key=value` da URL capturados na
entrada da solicitação. O middleware de solicitação faz o parse da query
string uma única vez para o conjunto de query do escopo, e a partir daí
todo chamador abaixo consegue ler params individuais por nome sem refazer
o parse:

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

Retorna `None` quando o parâmetro está ausente ou nenhum escopo está
ativo. Chaves duplicadas seguem a semântica do Laravel em que a última
vence - o mesmo valor que você obteria do mapa de query parseado da
solicitação.

### A paginação lê parâmetros de query

É para isso que o conjunto de query existe. Os paginadores do Eloquent
leem `?page=` e `?cursor=` direto de `Context::query_param`, então um
handler que retorna um paginador não precisa levar o número da página à
mão:

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // Lê ?page=N da URL da solicitação via Context::query_param - sem
    // boilerplate de req.query(), sem passar parâmetro adiante.
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

Três pontos de entrada de paginador usam isso:

- `Builder::paginate(per_page)` - lê `?page=`
- `Builder::simple_paginate(per_page)` - lê `?page=`
- `Builder::cursor_paginate(per_page)` - lê `?cursor=`

Veja [Paginação](pagination.md) para a superfície completa.

## Propagando para tasks criadas com spawn

`tokio::spawn` inicia a task filha com um ambiente task-local novo - o
escopo `Context` do pai **não** flui para dentro. Um `tokio::spawn` puro
dentro de uma solicitação vê um `Context` vazio e toda leitura retorna
`None`.

Para levar o escopo para dentro de um spawn, tire um snapshot dele com
`Context::current()` e entre nele de novo dentro da filha com
`Context::scope`:

```rust
use suprnova::context::Context;

// Dentro de um handler de solicitação:
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // Agora `Context::get`, `Context::query_param`, etc. enxergam o
        // conjunto da solicitação pai.
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

O store retornado por `Context::current()` compartilha os mapas
subjacentes do pai via `Arc` - escritas da filha são visíveis para o pai
enquanto a filha segurar o clone. É exatamente isso que spawns de
auditoria e de log querem: a filha pode marcar chaves adicionais
(`Context::add("audit.completed", true)`) e a linha de log final do pai
as enxerga.

Se você precisa de um snapshot isolado (as escritas da filha não devem
vazar de volta), construa um `ContextStore` novo e copie para dentro
apenas as chaves de que precisa.

### Por que o `spawn` puro não propaga

Os task-locals do Tokio (`tokio::task_local!`) têm escopo de task de
propósito. Herdar automaticamente através de spawns significaria:

- Tarefas em background de longa duração fixariam os mapas de contexto do
  pai para sempre
- Um panic em uma task filha poderia envenenar o estado do pai
- O runtime teria que percorrer uma cadeia de ponteiros para o pai a cada
  leitura de task-local

A dança explícita de `Context::current()` + `Context::scope` torna a
propagação uma decisão deliberada em vez de um padrão escondido.

## Testes

Dentro de `#[tokio::test]` ou `#[suprnova_test]`, nenhum escopo `Context`
é instalado por padrão. A maior parte do código sob teste que toca
contexto trata o caso "sem escopo" com elegância (no-op silencioso +
leituras `None`), então testes unitários simples não precisam de nenhum
setup.

Duas situações em que o teste precisa de ajuda:

### Quando o código sob teste chama `query_param`

Os helpers de paginação leem `?page=` via `Context::query_param`. Um
teste unitário para "a página 3 retorna o offset certo" precisa que
`query_param` retorne `Some("3")`. Duas formas:

**`test_query_guard` (recomendado):**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // O código sob teste agora enxerga ?page=3
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` é dropado no fim do escopo - o override thread-local é apagado.
```

`test_query_guard` retorna uma guarda RAII. Mesmo que o corpo do teste
sofra panic, o `Drop` roda e limpa o override thread-local antes de a
thread do SO ser reciclada. A guarda é `#[must_use]` - vinculá-la a `_`
limpa imediatamente, o que quase nunca é o que você quer.

**`test_set_query` + `test_clear_query`, na mão:**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // apaga vazamento de qualquer irmão
    Context::test_set_query("page", "5");

    // … asserções …

    Context::test_clear_query();
}
```

Use a forma com guarda. O par manual existe para os casos em que você
precisa de vários overrides definidos e limpos de forma independente, mas
a guarda `#[must_use]` é mais difícil de usar errado.

As duas APIs são condicionadas por `#[cfg(any(test, feature = "testing"))]` -
elas são compiladas nos binários de teste e nos builds de release que
optam pela feature `testing` para harnesses de teste de integração. Elas
não existem em builds de release comuns.

### Quando o código sob teste lê ou escreve em um escopo `Context`

Instale um explicitamente via `Context::scope`:

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

Ou semeie um conjunto de query na criação do escopo:

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` é o mesmo construtor que o middleware
de solicitação usa, então um teste que exercita o mesmo caminho de código
que a produção vê a mesma forma de conjunto de query.

### Por que o override thread-local existe

O override de parâmetro de query é um `thread_local!`, não um
task-local. Isso é deliberado: permite que testes instalem parâmetros de
query **sem envolver toda asserção em uma chamada `Context::scope`**. A
combinação é:

1. As leituras checam primeiro o override thread-local
2. Sem override, leem o conjunto de query do escopo task-local `CONTEXT`
3. Sem escopo também, retornam `None`

O lookup thread-local não custa efetivamente nada em produção (o override
está sempre vazio fora de builds de teste) e poupa quem escreve testes de
wrappers `Context::scope(...)` de boilerplate em torno de toda asserção
relacionada a paginação.

## Padrões comuns

### Marcar o request id em todo log

O framework já faz isso. O middleware de solicitação semeia
`_request_id` no conjunto visível para que jobs abaixo, transmissões e
despejos de log com `Context::all()` consigam ler o id por nome. O mesmo
middleware também abre um span do `tracing` carregando o id como campo do
span, que é o que o faz aparecer em toda linha de log emitida dentro da
solicitação - veja [Logs](logging.md) para o lado do subscriber. Ler o id
do `Context` é o caminho certo quando você precisa do valor como string
(por exemplo, para levá-lo a uma solicitação HTTP de saída como header de
correlação):

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### Levar o contexto de tenant para um job enfileirado

`Context` não se propaga automaticamente através da fronteira de
serialização / desserialização da fila - o worker roda em um processo
diferente do dispatcher, muitas vezes em outra máquina. Passe o que você
precisar para dentro do payload do job:

```rust
use suprnova::{Context, FrameworkError, Queue};

// Em um handler:
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

Quando o worker processa `SendInvoice`, instale um escopo `Context` novo
no topo de `Job::handle` e semeie de novo as chaves de que precisa a
partir do payload do job - `Context::scope(ContextStore::default(), async
{ ... })` envolvendo o corpo. Aí qualquer log ou helper profundamente
aninhado que o job chame enxerga o mesmo id de tenant que enxergaria
dentro de uma solicitação.

É aqui também que `hidden_add` se paga - o job pode buscar e guardar uma
chave de API uma vez na entrada do escopo, e toda chamada HTTP abaixo,
dentro do job, a lê via `Context::hidden_get` sem buscar de novo. Veja
[Fila](queues.md) para a forma da trait `Job`.

### Trilha de auditoria ao longo de uma solicitação

```rust
Context::push("audit.steps", "validated_input");
// … mais trabalho …
Context::push("audit.steps", "charged_card");
// … mais trabalho …
Context::push("audit.steps", "sent_receipt");

// Em um middleware no momento da resposta:
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

Um middleware no momento da resposta que roda depois do handler pode
despejar a trilha de auditoria em uma única linha de log, em vez da linha
de debug individual de cada etapa espalhada pelo log da solicitação.

### Conjunto oculto para credenciais de extensão de SDK

```rust
// Na entrada da solicitação, depois da autenticação:
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// Bem no fundo de uma chamada de SDK:
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

Logs que despejam `Context::all()` não mostram a chave. O conjunto oculto
é o lugar certo para qualquer credencial que o handler precise passar
fundo em uma pilha de chamadas sem expô-la às superfícies de log.

## Por que Suprnova diverge

A facade `Context` do Laravel (introduzida no Laravel 11) é a
inspiração - mesmos nomes de método, mesma separação visível/oculto,
mesmo contrato de "silencioso fora de uma solicitação". Duas diferenças
vêm do runtime do Rust:

**A propagação assíncrona é explícita, não mágica.** O `Context` do
Laravel flui através de jobs enfileirados automaticamente porque o
Laravel serializa o conjunto de contexto no payload do job no momento do
dispatch. O modelo async do Rust não tem uma única "solicitação atual"
para dentro da qual thread-locals fluam - `tokio::spawn` começa do zero,
e a fronteira da fila envolve serialização entre processos. O Suprnova
expõe a primitiva de propagação (`Context::current()` +
`Context::scope`) e deixa você optar por ela na fronteira, em vez de
fingir que tasks herdam um contexto que elas não herdam.

**Leituras com o tipo errado são observáveis.** `get::<T>` sobre um valor
armazenado com outro tipo retorna `None` silenciosamente no Laravel (é
PHP, os tipos não eram impostos no momento da escrita de qualquer forma).
No Suprnova a leitura emite um `tracing::trace!` porque o caso de tipo
errado indica um bug real - o valor foi escrito em algum lugar, só que
não com o tipo com que você está lendo. O trace permite encontrá-lo em
execuções instrumentadas sem mudar o contrato de nunca dar panic.

A terceira divergência é mecânica: o `Context` do Suprnova é construído
sobre `tokio::task_local!`, então seu tempo de vida está vinculado à task
Tokio, não a qualquer estado global. Leituras entre threads enxergam o
escopo da **task que está rodando naquela thread no momento**, não seja
lá qual escopo tenha sido instalado por último. É isso que torna a mesma
facade `Context` segura de chamar a partir de um thread pool, de um
actor, ou de um corpo de `spawn_blocking` - desde que você propague o
escopo para dentro do spawn.

## Onde isso vive

| Assunto | Arquivo |
|---|---|
| facade `Context` + `ContextStore` | `framework/src/context/mod.rs` |
| Instalação do escopo na solicitação HTTP | `framework/src/logging/request_id.rs` |
| Chamadores de `Context::query_param` (paginação) | `framework/src/eloquent/builder.rs` |
| Reexportações | `framework/src/lib.rs` (`pub use context::{Context, ContextStore}`) |

## Próximos passos

- [Ciclo de vida da solicitação](lifecycle.md) - onde o escopo `Context`
  é instalado em toda solicitação
- [Contêiner de serviços](container.md) - para estado entre solicitações
  que sobrevive a uma única task
- [Logs](logging.md) - como `Context::all()` acaba em linhas de log
  estruturadas
- [Paginação](pagination.md) - o principal leitor, abaixo, de
  `Context::query_param`
- [Testes](testing.md) - padrões de `test_query_guard` e `Context::scope`
  para testes unitários
