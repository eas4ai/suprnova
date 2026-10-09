# Paginação

O Suprnova traz três paginadores que casam com a superfície do
Laravel linha por linha: length-aware (sabe o total), simple (uma
consulta por página), e cursor (keyset opaco). Os três derivam
`Serialize` para o JSON no formato Laravel que consumidores Inertia
e JSON:API já entendem - você busca uma página e a retorna; nada
mais é necessário.

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

Aquela chamada única executa o `COUNT(*)` e a busca de página
`LIMIT/OFFSET`, faz parse de `?page=N` a partir da solicitação
ativa, e retorna um `LengthAwarePaginator<User>` pronto para
embarcar. As duas contrapartes - `simple_paginate(20)` e
`cursor_paginate(20)` - retornam a mesma forma de valor com
trade-offs diferentes. O resto deste capítulo é sobre qual escolher,
o que cada uma custa, e como o JSON chega.

## Escolhendo um paginador

A forma mais rápida de escolher é a tabela de trade-offs:

| Método | Tipo | Consultas / página | Sabe o total? | Use quando |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2 (`COUNT(*)` + página) | sim | UI mostra páginas numéricas ou "página 3 de 17" |
| `simple_paginate(n)` | `Paginator<M>` | 1 (`LIMIT n+1`) | não | Tabelas grandes; um botão "Próxima" basta |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1 (`LIMIT n+1`) | não | Scroll infinito; páginas profundas em tabelas de alto tráfego |

A diferença de custo importa quando sua tabela é grande. `COUNT(*)`
sobre cem milhões de linhas é a consulta mais cara no seu orçamento
de solicitação. `simple_paginate` economiza a contagem.
`cursor_paginate` economiza a contagem *e* evita a varredura linear
de `OFFSET N` que penaliza toda solicitação de página profunda em
uma tabela grande - uma busca por cursor é `O(1)`-ish com o índice
certo, independentemente de onde no result set o usuário está.

### Por que Suprnova diverge

Os paginadores do Laravel trazem helpers de construção de URL -
`nextPageUrl()`, `previousPageUrl()`, o array `links` de descritores
`{url, label, page, active}` que o Blade renderiza. A impl
`Serialize` bruta do Suprnova emite a fatia de dados mais os
contadores; a construção de URL vive nos construtores de forma de
resposta que já possuem o contexto de URL:
[`Inertia::paginate`](frontend-inertia-responses.md) anexa metadados
de scroll do Inertia (identificadores de página, não URLs
absolutas); [`Resource::paginated`](eloquent-resources.md) anexa
`links.{self,first,last,prev,next}` do JSON:API conforme a
recomendação do JSON:API.

Dois motivos para a divisão. Primeiro, a URL que o cliente deve ver
depende de qual superfície de protocolo está renderizando - Inertia
se baseia em identificadores de página, JSON:API quer hrefs
absolutas. Segundo, o paginador não sabe a URL base da solicitação
por padrão; os helpers que a sabem podem anexar as URLs uma vez,
onde elas pertencem. Se você de fato precisa de URLs no paginador nu
(envelope JSON customizado, payload de telemetria, asserção de
teste), chame `with_path(...)` e use `url_for_page(n)` - abordado na
seção [Geração de URLs](#geração-de-urls-e-caminhos).

## `paginate` - length-aware

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

Os campos públicos da struct:

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // linhas nesta página
    pub current_page: u64,       // baseado em 1
    pub last_page: u64,          // baseado em 1; 0 quando total == 0
    pub per_page: u64,
    pub total: u64,              // toda linha em todas as páginas
    pub from: Option<u64>,       // índice baseado em 1 da primeira linha nesta página
    pub to: Option<u64>,         // índice baseado em 1 da última linha nesta página
    pub path: Option<String>,    // URL base para url_for_page (opcional)
}
```

O JSON que o `Serialize` derivado emite:

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` é omitido do JSON quando não definido; `from` e `to` são
`null` quando a página está vazia (nenhuma linha nesta página, ou a
página solicitada está além da última página).

### Lendo `?page=N` automaticamente

`paginate(n)` lê a página atual de `?page=N` na solicitação ativa
via `Context::query_param`. Valores ausentes, vazios, não-numéricos,
e zero são fixados em `1`. Não há nada para conectar - se uma
solicitação está em escopo, o parâmetro é lido.

### Múltiplos paginadores em uma página

Quando uma página renderiza mais de uma lista paginada, dê a cada
uma sua própria chave de query-string com `paginate_using`:

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` também define `page_name` no paginador retornado
para que `url_for_page` construa URLs com a mesma chave:

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"  (quando path está definido)
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### Predicados de posição de página

O conjunto completo de predicados `AbstractPaginator` do Laravel é
implementado:

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // não estamos na página 1 OU mais páginas existem
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - fatia da página, não o total
```

`count()` é o tamanho da fatia, não o total - a forma `Countable` do
Laravel; para o total use o campo `total` diretamente.

## `simple_paginate` - uma consulta, sem contagem

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // havia uma linha extra além de per_page?
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

O truque está no SQL. `simple_paginate(20)` emite `LIMIT 21`, olha
se a 21ª linha voltou, define `has_more` a partir disso, e trunca
`data` de volta para 20. Uma consulta por página; nenhum `COUNT(*)`.

Você desiste de `total`, `last_page`, `from`, e `to`. Em troca você
consegue paginar tabelas onde `COUNT(*)` é caro demais para executar
em toda carga de página. A superfície de UI é botões "Próxima" /
"Anterior", não "página 7 de 142".

O mesmo conjunto de predicados do paginador length-aware é
implementado: `has_more_pages()`, `on_first_page()`, `on_last_page()`,
`has_pages()`, `is_empty()`, `is_not_empty()`, `count()`.

## `cursor_paginate` - keyset opaco

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // None na última página
    pub prev_cursor: Option<String>,  // None na primeira página
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` e `prev_cursor` estão sempre presentes como chaves
JSON (`null` quando ausentes) para que schemas de cliente possam
confiar na presença do campo; `path` é omitido quando não definido.

### Como cursores funcionam na rede

O cliente passa o cursor da página anterior através de
`?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` decodifica o cursor, percorre o filtro de keyset
(`pk > boundary ASC` para `next`; `pk < boundary DESC` para `prev`,
revertido de volta para ASC), busca `LIMIT n+1` linhas, e reemite
`next_cursor` / `prev_cursor` conforme os vizinhos da página
existem. É bidirecional - o cliente pode andar para frente e para
trás sem perder sua posição.

A paginação por cursor **substitui** qualquer `ORDER BY` existente
no construtor. Uma ordem total estável sobre a chave primária é
exigida para que o filtro de keyset corte a tabela de forma
determinística; um cursor com `ORDER BY random_score()` arbitrário
pularia e duplicaria linhas. Se você precisa de uma ordenação que
não seja por PK, mude para `paginate` / `simple_paginate`.

### Cursores são criptografados e autenticados

Os cursores do Suprnova **não** são o texto plano base64-JSON do
Laravel. O cursor na rede é o limite de keyset (um `sea_orm::Value`
tipado - `Int`, `BigInt`, `Uuid`, datetimes, decimals, strings,
bytes) mais uma tag de direção, codificado em JSON e então selado
com AES-256-GCM via o keyring `Crypt` do framework (vinculado a
`CryptPurpose::Cursor`, então um ciphertext de cursor nunca pode ser
reproduzido em nenhuma outra superfície - cookie, secret de 2FA,
cast).

Isso significa três coisas na prática:

1. **Sem adulteração.** Um cliente que inverte bits em `?cursor=`
   recebe um 400 `Invalid pagination cursor`, não uma página
   diferente de dados.
2. **Sem vazamento de informação.** O valor de limite (frequentemente
   uma chave primária, às vezes um timestamp) é selado dentro do
   cursor - clientes não conseguem enumerar ranges editando-o.
3. **Limites tipados fazem round-trip sem perdas.** O envelope na
   rede marca a variante do SeaORM (`"BigInt"`, `"Uuid"`, etc.),
   então na decodificação o valor se revincula com o mesmo tipo SQL
   que a coluna original emitiu. Nenhum bug de coerção de string
   entre Postgres / MySQL / SQLite.

Não há fallback em texto plano. Se `Crypt` não estiver
inicializado - o que deveria ser impossível depois de
`Server::from_config` - a codificação falha em vez de emitir um
cursor forjável.

### Por que Suprnova diverge

O paginador de cursor do Laravel é apenas-para-frente por padrão e o
cursor na rede é um blob JSON codificado em base64 - legível,
editável, reproduzível. O cursor do Suprnova é bidirecional (casando
com a superfície `cursorPaginate()` que o Laravel adicionou depois)
e é autenticado de ponta a ponta para que o cliente não consiga
construir ou alterar um. O ecossistema Rust já tem AES-GCM como
primitiva; usá-lo custa ao framework uma impl de trait extra e dá a
todo cursor uma propriedade de segurança que um payload base64 em
texto plano não consegue oferecer.

## A facade - `Pagination::length_aware` / `Pagination::cursor`

A maioria dos capítulos deste manual mostra paginação através do
construtor Eloquent, porque esse é o caminho comum. Se você está
construindo um `Select<E>` do SeaORM diretamente - digamos, fazendo
join em uma consulta sem model para um relatório - a facade
`Pagination` é a superfície equivalente:

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // ou qualquer Select<E> do SeaORM
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

A facade também oferece `length_aware_on(conn, ...)` e
`cursor_on(conn, ...)` para rotear para uma conexão nomeada
específica, e uma forma tipada `cursor(query, cursor, per_page,
order_col)` que recebe a coluna de keyset explicitamente - usada
quando o cursor ordena por algo além da chave primária.

As regras de roteamento casam com o construtor Eloquent. Uma
`DB::transaction` ambiente é honrada (tanto o COUNT quanto a
consulta de página executam na conexão da transação), e uma conexão
`__read_replica__` registrada é usada automaticamente para leituras.
O sentinel `__primary__` seleciona o pool padrão quando você quer
contornar a replica.

## Validação - `per_page == 0`

Os três métodos rejeitam `per_page == 0`:

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

O erro renderiza como HTTP 400 com o corpo de erro padrão. Não há
uma "página vazia" silenciosa - um tamanho de página zero está
sempre errado e é rejeitado no call site, casando com o construtor
Eloquent e a facade `Pagination`. A mesma validação vive em
`cursor_paginate`, `simple_paginate`, `Pagination::length_aware`,
`Pagination::length_aware_on`, `Pagination::cursor`, e
`Pagination::cursor_on` - uma regra, seis pontos de entrada.

O valor de `current_page` é **fixado**, não validado: `0` se torna
`1`, números negativos de um frontend defensivo não podem acontecer
(o parser é `u64`), e qualquer `?page=N` maior que `last_page`
retorna um paginador com `data` vazio mais `from`/`to` de `None`.
Andar além do fim é um engano do cliente, não um erro.

## Forma do erro

| Condição | Variante | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Cursor adulterado / inválido | `FrameworkError::Domain` (`"Invalid pagination cursor"`) | 400 |
| `Crypt` não inicializado na decodificação do cursor | `FrameworkError::Internal` | 500 |
| Incompatibilidade de variante do cursor em `decode_cursor` | `FrameworkError::Internal` | 500 |
| Falha subjacente do BD | `FrameworkError::Database` | 500 |

O caso de cursor adulterado é o que vale lembrar. Cursores são lidos
diretamente da rede - a query string `?cursor=…` é entrada de
atacante por definição, e base64 com bits invertidos e ciphertext
reproduzido são modos de falha esperados, não bugs do servidor. A
etapa de descriptografia faz downgrade para um 400
`Invalid pagination cursor` para que falhas disparáveis pelo cliente
não poluam o canal de telemetria 500. A mensagem estática não dá ao
cliente nada com que sondar.

Falhas pós-descriptografia (parse de JSON, dispatch de variant-tag,
parse de direção) permanecem 500 - qualquer sequência de bytes que
sobreviveu à autenticação AEAD foi produzida por *nós*, então um
payload malformado depois desse ponto é um bug do framework que
vale sinalizar.

## Geração de URLs e caminhos

O paginador bruto carrega um campo `path` opcional. Quando definido,
`url_for_page(n)` e a emissão de link de cursor o usam para
construir query strings:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

Quando o caminho base já carrega uma query string, o separador muda
para `&` para que a URL permaneça bem formada:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

Se `path` não estiver definido, `url_for_page` recai para uma query
relativa nua: `?page=2`. O nome do parâmetro de página vem de
`with_page_name(...)` (com padrão `"page"`); `paginate_using(name,
n)` o define automaticamente para que as URLs geradas usem a mesma
chave da qual o paginador foi acionado. O nome do parâmetro é
form-urlencoded, então até um nome com caracteres reservados não
consegue corromper a URL.

Paginadores de cursor têm a mesma forma: `with_path(...)` define a
base, `with_cursor_name(...)` sobrescreve a chave de query (padrão
`"cursor"`), e o construtor de links do JSON:API os capta
automaticamente.

A maioria das apps não chama `url_for_page` diretamente - elas
entregam o paginador para uma das duas superfícies de integração
abaixo, que constroem as URLs da forma certa para seu protocolo.

## Integração com Inertia - props de scroll infinito

Para front-ends Inertia, o helper `Inertia::paginate(component, key,
paginator)` anexa o paginador como um prop de scroll:

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

Os três paginadores funcionam aqui - `LengthAwarePaginator`,
`Paginator`, e `CursorPaginator`. O page-name dos metadados vem do
próprio paginador: `"page"` para os dois paginadores de offset,
`"cursor"` para `CursorPaginator`. O cliente recebe as linhas sob a
chave de prop escolhida mais um descritor `ScrollMetadata` com
`current_page`, `next_page`, `previous_page` (identificadores de
página para os paginadores de offset; strings de cursor para
paginadores de cursor) - que os helpers `useInfiniteScroll` /
`WhenVisible` do Inertia consomem para scroll infinito.

Cada paginador constrói esse descritor por `ProvidesScrollMetadata` - a
mesma interface que o adaptador de paginador do Laravel satisfaz
(`ProvidesScrollMetadata::getPageName` / `getPreviousPage` / `getNextPage`
/ `getCurrentPage`). Um paginador que este crate não conhece - o tipo de
cursor de um crate de terceiros, um resultado de repositório escrito à mão -
pode implementar os quatro métodos e entregar ao framework um `ScrollMetadata`
que o cliente Inertia já entende. Consulte [Respostas Inertia](frontend-inertia-responses.md#merge-strategies-and-infinite-scroll).

`simple_paginate` vale destacar, porque uma listagem sobre uma
tabela grande o bastante para fazer do `COUNT(*)` o custo dominante
da solicitação é exatamente onde uma página de coleção Inertia
sofre:

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // sem COUNT, uma consulta
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

Seu `next_page` vem da sondagem de overflow `LIMIT n+1` em vez de
uma última página computada, já que não há total a partir do qual
computar uma. O cliente recebe "há outra página" em vez de "há
4.812 páginas" - que é tudo que uma UI de scroll infinito jamais lê.

### Projetando linhas antes de saírem

Paginadores não têm `map` / `through` (os do Laravel têm).
Reconstrua a partir dos campos públicos em vez disso - os contadores
e cursores descrevem a *consulta*, então eles atravessam uma
mudança de tipo de linha sem alteração:

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

Vale fazer isso em vez de serializar o model diretamente sempre que
a rota é não autenticada e o model carrega algo que o chamador não
deveria ver. Um cursor sobre uma tabela de usuários entrega uma
página por vez, mas eventualmente entrega todas as páginas.

O mesmo helper existe como um método encadeável em
`InertiaResponse::paginate(key, paginator)` se você quiser misturar
um paginador com outros props:

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

Veja [Respostas Inertia](frontend-inertia-responses.md) para o
modelo de prop mais amplo.

## Integração com JSON:API - `Resource::paginated`

Para consumidores JSON:API, `Resource::paginated(paginator)`
constrói o envelope completo:

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

A resposta carrega:

- `data` - toda linha renderizada através do `IntoJsonResource` do
  model.
- `meta.pagination` - `{ total, per_page, current_page, last_page }`
  para length-aware; `{ next_cursor, prev_cursor }` para cursor.
- `links.{self,first,last,prev,next}` - hrefs absolutas para o
  paginador length-aware (construídas a partir de `path`);
  `links.{prev,next}` para o paginador de cursor.

Os dois tipos de paginador implementam a trait `Paginated<T>` que
`Resource::paginated` consome - não há caminho de código separado
para length-aware vs. cursor. Se você construir um tipo customizado
parecido com paginador que implemente `Paginated<T>`, ele se compõe
da mesma forma.

Veja [Recursos JSON:API](eloquent-resources.md) para o modelo de
recurso.

## Envelopes JSON customizados

Se nem Inertia nem JSON:API casam com seu cliente, embarque o
paginador diretamente através de `json_response!`:

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

Ou simplesmente entregue o paginador inteiro - a impl `Serialize`
derivada emite a forma documentada acima:

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

Os campos são públicos; remodele conforme seu contrato exigir.

## Roteamento entre conexões

A paginação respeita o mesmo roteamento multiconexão que o
construtor Eloquent usa. Dentro de uma `DB::transaction(...)` o
COUNT e a consulta de página executam ambos na conexão da
transação - eles nunca se dividem entre conexões, então a contagem
nunca discorda da página que ela descreveu. Uma `__read_replica__`
registrada é usada automaticamente para leituras fora de uma
transação. Para fixar um paginador em uma conexão nomeada
específica, use as variantes `_on(connection, ...)` na facade
`Pagination`, ou `Builder::on("replica_b").paginate(20)` do lado do
Eloquent.

Veja [Eloquent - roteamento multiconexão](eloquent.md) para o
contrato de roteamento.

## Quando recorrer a qual

Uma árvore de decisão aproximada:

- **UI de página numérica faz parte do design** → `paginate`. Você
  precisa de `last_page` para renderizar "Página 3 de 17", e o custo
  do COUNT é aceitável no tamanho da sua tabela.
- **Apenas botões "Próxima" / "Anterior", tabela grande** →
  `simple_paginate`. Uma consulta por página; você desiste de
  `total` e `last_page`, mas o carregamento de página cai pela
  metade.
- **Scroll infinito** → `cursor_paginate`. Cursores bidirecionais
  significam que o cliente pode continuar rolando além da página
  1000 sem o OFFSET escanear milhares de linhas primeiro.
- **Final de um feed de alto tráfego e apenas-acréscimo** →
  `cursor_paginate`. Ordenação por keyset na chave primária é segura
  sob concorrência: linhas novas caem além do cursor, nunca dentro
  dele. Paginação baseada em OFFSET pula linhas sob inserções.
- **Construindo um `Select<E>` fora de um model Eloquent** →
  `Pagination::length_aware` / `Pagination::cursor`. Mesmos
  trade-offs; a facade é o equivalente sem model.

Na dúvida, comece com `paginate`. Mude para `simple_paginate` quando
o `COUNT(*)` aparecer no seu log de consultas lentas. Mude para
`cursor_paginate` quando páginas profundas começarem a dominar o
tempo de solicitação, ou quando a UI for scroll infinito.

## Onde cada peça vive

| Peça | Arquivo |
|---|---|
| Facade `Pagination`, trait `Paginated<T>` | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>` (simple) | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`, `CursorDirection`, `encode_value`, `decode_value` | `framework/src/pagination/cursor.rs` |
| Ponte `IntoInertiaScroll` | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`, `InertiaResponse::paginate` | `framework/src/inertia/facade.rs`, `framework/src/inertia/response.rs` |
| `Resource::paginated`, `JsonApi::paginated` | `framework/src/resources/response.rs` |

## Próximos passos

- [API Eloquent](eloquent.md) - a camada de model que aciona todo
  paginador retornado de `Builder::paginate*`
- [Construtor de consultas](queries.md) - as consultas sem model que
  se compõem com `Pagination::length_aware` e `Pagination::cursor`
- [Respostas Inertia](frontend-inertia-responses.md) - como props de
  scroll anexam paginadores a páginas Inertia
- [Recursos JSON:API](eloquent-resources.md) - `Resource::paginated`,
  links, meta, e a trait `Paginated<T>`
- [Modelo de erros](error-model.md) - a regra de validação
  `FrameworkError::param` e o downgrade de adulteração de cursor
