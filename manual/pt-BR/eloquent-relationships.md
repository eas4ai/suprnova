# Relacionamentos Eloquent

[Eloquent](eloquent.md) cobre a superfície de relacionamento do dia a
dia - sintaxe de declaração, a tabela de opções, o encadeamento
básico por tipo. Este capítulo é o mergulho profundo específico de
relacionamento: como uma chamada `user.posts()` de fato se resolve
para SQL, como o eager loader evita N+1, como o motor de existência
(`has` / `where_has` / `where_belongs_to`) renderiza subconsultas
`EXISTS` correlacionadas, como o polimorfismo sobrevive à falta de
late static binding do Rust, e o que decorre do sistema de tipos
quando os onze tipos de relação precisam coexistir em um único
trait.

Se você é novo no Eloquent do Suprnova, leia primeiro
[Eloquent](eloquent.md#relationships) - aquela página ensina a
sintaxe de declaração. Esta página assume que você já tem um model
com um bloco `relations = { ... }` e quer entender o que está por
baixo.

## Os onze tipos de relação

Todo tipo de relação em [`RelationKind`][relations] é um destes:

| Tipo                  | Lado       | Cardinalidade | Entre famílias | Pivot |
|-----------------------|------------|-------------|-----------------|-------|
| `HasOne<R>`           | pai        | um          | não             | - |
| `HasMany<R>`          | pai        | muitos      | não             | - |
| `BelongsTo<R>`        | filho      | um          | não             | - |
| `BelongsToMany<R, P>` | qualquer um| muitos      | não             | sim   |
| `HasOneThrough<B, R>` | pai        | um          | não             | - |
| `HasManyThrough<B, R>`| pai        | muitos      | não             | - |
| `MorphOne<R>`         | pai        | um          | sim             | - |
| `MorphMany<R>`        | pai        | muitos      | sim             | - |
| `MorphTo`             | filho      | um          | sim (n alvos)   | - |
| `MorphToMany<R, P>`   | pai        | muitos      | sim             | sim   |
| `MorphedByMany<R, P>` | parceiro m2m| muitos     | sim (inverso)   | sim   |

"Entre famílias" significa que o *tipo* da linha relacionada varia -
um `Comment` pode pertencer a um `Post` ou a um `Video`, não a uma
única tabela-pai fixa. Isso é polimorfismo, e o Suprnova o trata via
[o registro polimórfico](#o-registro-polimórfico) mais um enum por
família.

[relations]: https://docs.rs/suprnova

### O que a macro emite

Quando você escreve:

```rust
use suprnova::model;

#[model(table = "users", relations = {
    posts: HasMany<Post>,
})]
pub struct User {
    pub id: i64,
    pub name: String,
}
```

`#[suprnova::model]` se expande em cinco coisas para `posts`:

1. **Método de relação** - `fn posts(&self) -> HasMany<Self, Post>`.
   Retorna um wrapper lazy carregando `self.id` mais os metadados de
   FK; nenhum SQL roda ainda.
2. **Acessador carregado** - `fn posts_loaded(&self) -> &[Post]`. Lê
   do cache eager depois de `User::with(["posts"])`. Slice vazio
   quando nenhum eager load rodou.
3. **Acessador de contagem** - `fn posts_count(&self) -> u64`. Lê do
   mesmo cache depois de `User::with_count(["posts"])`.
4. **Braço de dispatcher** - um match arm no método inerente
   `__eager_load` do model. O eager loader procura `"posts"` e roda
   a query `IN`.
5. **Entrada de inventory** - um `inventory::submit!(RelationEntry { ... })`,
   para que a relação seja enumerável em runtime (ferramentas de
   admin, o motor de existência, o dispatcher polimórfico - todos
   percorrem isto).

Você nunca vê (4) ou (5). Eles impulsionam o resto deste capítulo.

## Resolução lazy: como `user.posts()` se torna SQL

`user.posts()` retorna um wrapper `HasMany<User, Post>`, não um
resultado de query. O wrapper carrega o valor de PK do pai mais o
nome da coluna FK, e um `Builder<Post>` pré-filtrado com
`WHERE posts.user_id = ?` já aplicado. Nada tocou o banco de dados
ainda.

```rust
use suprnova::Direction;

// No SQL.
let posts_q = user.posts();

// SQL: SELECT * FROM posts WHERE user_id = ? ORDER BY id DESC LIMIT 5
let recent = user.posts()
    .order_by("id", Direction::Desc)
    .limit(5)
    .get()
    .await?;

// SQL: SELECT COUNT(*) FROM posts WHERE user_id = ?
let n = user.posts().count().await?;
```

A superfície de API dual ([Eloquent → Nota de nomenclatura](eloquent.md#naming-note-dual-api))
é respeitada no wrapper: tanto `.filter("col", v)` quanto
`.db_where("col", v)` funcionam, de forma idêntica. A superfície
encadeável em `HasOne` / `HasMany` / `MorphOne` / `MorphMany` cobre
`filter` / `db_where` / `order_by` / `latest` / `oldest` / `limit` /
`take`. Relações Through e m2m polimórficas expõem só seus métodos
terminais - elas passam por costuras de SQL escritas à mão, não por
um `Builder<R>`, então não podem compor com a cadeia padrão. Veja
[Relações Through](#hasonethrough-e-hasmanythrough) e [m2m
polimórfico](#morphtomany-e-morphedbymany) abaixo.

### Soft deletes acompanham

Quando o tipo relacionado implementa [`SoftDeletes`](eloquent.md#soft-deletes-flag),
o wrapper de relação herda seu scope global. `user.posts().get()`
esconde posts trashed do mesmo jeito que `Post::query().get()` faz.
Três forwarders furam esse comportamento:

```rust
let alive = user.posts().get().await?;                 // padrão: só vivos
let all = user.posts().with_trashed().get().await?;    // vivos + trashed
let dead = user.posts().only_trashed().get().await?;   // só trashed
```

`with_trashed` / `only_trashed` existem em `HasOne`, `HasMany`,
`MorphOne`, `MorphMany`, `BelongsToMany`, `MorphToMany`,
`MorphedByMany`, e `BelongsTo`. Estão deliberadamente ausentes de
`HasOneThrough` e `HasManyThrough` - veja a [lacuna de soft-delete do
Through](#through-soft-deletes-v1) abaixo.

## Um-para-um: `HasOne` e `BelongsTo`

`HasOne` é o pai dizendo "este filho tem uma coluna apontando para
mim". `BelongsTo` é o filho dizendo "eu tenho uma coluna apontando
para o pai". Os dois rodam um único `WHERE fk = ? LIMIT 1` e
retornam `Option<R>`.

```rust
// HasOne - pai → filho
let profile: Option<Profile> = user.profile().first().await?;

// BelongsTo - filho → pai
let owner: Option<User> = profile.user().first().await?;
```

`BelongsTo` adiciona uma facilidade no formato do Laravel que os
outros não precisam: `with_default`. Quando a FK do filho é null OU
a linha do pai foi deletada, `first()` retorna o substituto da
closure em vez de `None`:

```rust
#[model(table = "comments", relations = {
    author: BelongsTo<User> {
        with_default = || User { id: 0, name: "Guest".into(), .. },
    },
})]
pub struct Comment { /* ... */ }

// Sempre retorna Some(User) - o autor real ou o substituto Guest.
let display: Option<User> = comment.author().first().await?;
```

O dispatcher de eager-load respeita o mesmo fallback - os caminhos
lazy e eager compartilham o comportamento padrão, então código de
template que imprime `comment.author_loaded()[0].name` não precisa
ramificar.

## Um-para-muitos: `HasMany`

`HasMany` é a relação de cardinalidade muitos do lado do pai. O
terminal `.get()` retorna uma
[`Collection<R>`](eloquent.md#collections) - o wrapper no formato
Laravel em torno de `Vec<R>` - então a superfície com conhecimento de
model se compõe:

```rust
let titles = user.posts()
    .order_by("created_at", Direction::Desc)
    .limit(10)
    .get()
    .await?
    .pluck::<String>("title");
```

`latest()` e `oldest()` são açúcar para
`order_by("created_at", Direction::Desc)` e `Asc` respectivamente -
eles só se resolvem contra models que declaram uma coluna
`created_at`, que a macro `#[suprnova::model]` adiciona
automaticamente sempre que timestamps estão ativos (o padrão).

## Muitos-para-muitos: `BelongsToMany<R, P>` e o pivot de primeira classe

`BelongsToMany` é muitos-para-muitos através de uma tabela de junção.
O pivot do Suprnova é ele próprio uma struct `#[suprnova::model]` com
suas próprias migrações, seus próprios acessadores, seus próprios
eventos. Essa é a divergência - veja [abaixo](#por-que-suprnova-diverge-pivot-é-um-model-de-verdade).

```rust
#[model(table = "users", relations = {
    roles: BelongsToMany<Role, RoleUser> {
        with_pivot = ["assigned_at"],
        with_timestamps,
    },
})]
pub struct User { /* ... */ }

#[model(table = "role_user", primary_key = "id")]
pub struct RoleUser {
    pub id: i64,
    pub user_id: i64,
    pub role_id: i64,
    pub assigned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Mutadores rodam contra a linha de pivot:

```rust
use suprnova::attrs;

user.roles().attach(role.id).await?;
user.roles().attach_with(role.id, attrs! { assigned_at: now }).await?;
user.roles().detach(role.id).await?;
user.roles().sync([role_a.id, role_b.id, role_c.id]).await?;
```

`sync` lê o conjunto de pivot atual, calcula
`attach_set = ids - current` e `detach_set = current - ids`, e roda
os deltas dentro de uma transação. Duplicatas no conjunto de entrada
colapsam pela forma em string JSON, então `sync([1, 1, 2])` faz o que
você espera.

A leitura passa pela estratégia de duas queries:

```rust
// Consulta 1: SELECT roles.*, role_user.* via INNER JOIN, delimitada por user_id.
// Consulta 2: SELECT role_user.* para o mesmo join, para estampar __pivot por linha.
let roles = user.roles().get().await?;

// Cada role carrega o contexto de pivot que a macro tornou acessível:
for r in &roles {
    let pivot = r.pivot::<RoleUser>().expect("loaded via BelongsToMany");
    println!("{} assigned at {:?}", r.name, pivot.assigned_at);
}
```

### Por que Suprnova diverge: pivot é um model de verdade

O pivot do Laravel é um conjunto opaco por atributo
(`$role->pivot->note`). O Suprnova exige que você declare a struct do
pivot porque o sistema de tipos do Rust precisa das colunas em tempo
de compilação - e depois de pagar por essa declaração, o pivot recebe
o mesmo tratamento `#[suprnova::model]` que qualquer outra tabela:
migrações, eventos, observers, factories, soft-delete.
`r.pivot::<RoleUser>()` retorna uma referência tipada; sem lookups de
atributo chaveados por string, sem surpresas em runtime quando uma
coluna é digitada errado.

O custo é uma struct extra por tabela de pivot. O benefício é que o
pivot pode carregar comportamento - lógica de domínio, regras de
validação, colunas de auditoria - sem escapar para SQL cru.

## `HasOneThrough` e `HasManyThrough`

Relações de dois saltos: `A → B → C` em que `B` é um model
intermediário cuja FK aponta para `A`, e `C` é o alvo final cuja FK
aponta para `B`. Exemplo clássico: `Country` tem muitos `User`s;
`User` tem muitos `Post`s; `Country::posts()` salta os dois em uma
única ida e volta ao SQL.

```rust
#[model(table = "countries", relations = {
    posts: HasManyThrough<User, Post>,
})]
pub struct Country { /* ... */ }

// Um único INNER JOIN: SELECT posts.* FROM posts
//   INNER JOIN users ON posts.user_id = users.id
//   WHERE users.country_id = ?
let posts: Collection<Post> = country.posts().get().await?;
```

`HasOneThrough` tem a mesma forma, mas `.get()` retorna `Option<C>`
(combinando com a semântica de cardinalidade um) e `.first()` é seu
alias.

Wrappers Through expõem só seus terminais - `get` / `first` / `count`
mais os setters de chave (`first_key` / `second_key` / `local_key` /
`second_local_key`). Eles não passam por um `Builder<C>`, então não
podem encadear `.filter(...)` ou `.order_by(...)`. Se você precisa
filtrar através do join, recorra a dois saltos de relação explícitos.

### Through soft-deletes (v1)

Relações Through usam SQL `INNER JOIN` cru em vez do pipeline
`Builder<C>`, então o scope global de soft-delete que `C::query()`
instalaria (`WHERE c.deleted_at IS NULL`) **não** é aplicado.
Intermediários trashed e alvos trashed participam do JOIN.

Isso diverge do Laravel, em que `hasManyThrough` filtra tanto `B`
quanto `C` por `deleted_at IS NULL` quando os models declaram
`SoftDeletes`. Até a correção chegar, quem chama e precisa de
leituras Through com scope deve encadear as duas relações
explicitamente:

```rust
// Em vez de country.posts().get():
let users = country.users().get().await?;
let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
let posts = Post::query().filter_in("user_id", user_ids).get().await?;
// Os dois scopes de soft-delete, de User e Post, se aplicam.
```

## Relações polimórficas

Uma FK polimórfica é um par de colunas: `<name>_id` (a chave primária
da linha) mais `<name>_type` (uma string identificando *em qual
tabela* o id vive). Uma linha `Comment` pode apontar para um `Post`
ou um `Video` sem adicionar nem uma coluna `post_id` nem `video_id`.

O Suprnova traz quatro tipos polimórficos: `MorphOne`, `MorphMany`,
`MorphTo`, e o par m2m `MorphToMany` / `MorphedByMany`. Todos
compartilham uma peça de infraestrutura: [o registro
polimórfico](#o-registro-polimórfico).

### `MorphOne<R>` e `MorphMany<R>` - lado pai

`MorphOne` e `MorphMany` espelham `HasOne` e `HasMany`, mas
sobrepõem o discriminador `<name>_type`. O builder interno vem
pré-filtrado com `WHERE <name>_id = ? AND <name>_type = ?`, então
filhos polimórficos apontando para *outras* famílias nunca aparecem
no resultado.

```rust
#[model(table = "posts", morph_type = "post", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Post { /* ... */ }

#[model(table = "videos", morph_type = "video", relations = {
    comments: MorphMany<Comment> { name = "commentable" },
})]
pub struct Video { /* ... */ }

let post_comments = post.comments().get().await?;     // só commentable_type = 'post'
let video_comments = video.comments().get().await?;   // só commentable_type = 'video'
```

`morph_type = "post"` é a string que o pai registra na coluna
`commentable_type` do filho. O padrão é o nome da struct em
snake_case, mas sobrescrever é a escolha certa para qualquer model
que você for lançar em produção - refatorações de renomeação de
tabela não deveriam quebrar a chave polimórfica.

### `MorphTo` e o enum por família

`MorphTo` vive do lado da tabela polimórfica. Quem declara define a
*lista de alvos* de antemão:

```rust
#[model(table = "comments", relations = {
    commentable: MorphTo { name = "commentable", targets = [Post, Video] },
})]
pub struct Comment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
}
```

A macro emite um enum por família no local da declaração:

```rust
// Emitido pela macro - você não escreve isso.
pub enum CommentableMorph {
    Post(Post),
    Video(Video),
    Unknown(String, i64),     // fallback para <name>_type não registrado
}
```

E `comment.commentable()` retorna um helper de busca cujo `.get()`
se resolve para o enum:

```rust
match comment.commentable().get().await? {
    CommentableMorph::Post(post) => println!("on post: {}", post.title),
    CommentableMorph::Video(video) => println!("on video: {}", video.url),
    CommentableMorph::Unknown(t, id) => {
        eprintln!("orphaned commentable_type={t} id={id}");
    }
}
```

### Por que Suprnova diverge: enum por família

O `morphTo` do Laravel retorna `mixed` - o dynamic dispatch do PHP
resolve o método em runtime. O Rust não tem late static binding,
então o Suprnova torna a família explícita. Os benefícios superam o
custo de digitação:

- **`match` exaustivo** - o compilador te avisa quando um novo alvo
  de morph chega e você esqueceu de tratá-lo.
- **`Unknown(String, id)` é type-safe** - linhas órfãs de uma classe
  de model pai removida aparecem como uma variante, não causam
  panic.
- **A lista de alvos documenta o esquema** - ler a declaração de
  `MorphTo` te diz todo tipo que pode estar do outro lado. Nenhuma
  query ao banco de dados é necessária para enumerá-los.

### Restrição v1: `MorphTo` é só `i64`

`MorphTo::morph_id` é fixado em `i64`. Alvos polimórficos precisam
portanto usar chaves primárias `i64`, e a coluna `<name>_id` da
tabela polimórfica também precisa ser `i64`. Models cuja PK é
`String` ou `Uuid`-via-string não podem ser alvos de `MorphTo` na v1.
A v2 vai parametrizar o tipo do id de morph para que toda a rede de
PK possível (`i64` / `String` / `Uuid`) seja aceita.

Esta é uma restrição só do lado polimórfico-inverso. `MorphOne` /
`MorphMany` / `MorphToMany` / `MorphedByMany` funcionam bem com
qualquer forma de PK - eles leem o `id` já tipado do pai diretamente.

### `MorphToMany` e `MorphedByMany`

Muitos-para-muitos polimórfico através de um único pivot. Um lado é
"morphable" (`Post.tags()`, `Video.tags()` - os dois passam pelo
mesmo pivot `taggables`). O outro é o parceiro m2m compartilhado
(`Tag.posts()`, `Tag.videos()` - mesmo pivot, escaneado do outro
jeito).

```rust
#[model(table = "tags", relations = {
    posts: MorphedByMany<Post, Taggable> {
        name = "taggable",
        target_morph_type = "post",
    },
    videos: MorphedByMany<Video, Taggable> {
        name = "taggable",
        target_morph_type = "video",
    },
})]
pub struct Tag { /* ... */ }

#[model(table = "posts", morph_type = "post", relations = {
    tags: MorphToMany<Tag, Taggable> { name = "taggable" },
})]
pub struct Post { /* ... */ }

#[model(table = "taggables", primary_key = "id", timestamps = false)]
pub struct Taggable {
    pub id: i64,
    pub tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
}
```

`MorphToMany` é o lado que altera - `attach` / `attach_with` /
`detach` / `sync` todos vivem lá. `MorphedByMany` é somente leitura:
toda chamada `tag.posts()` retorna só taggables tipados `Post`, toda
`tag.videos()` retorna só taggables tipados `Video`, sem mistura em
uma coleção.

Altere a partir do lado morphable:

```rust
post.tags().attach(rust_tag.id).await?;
post.tags().sync([rust_tag.id, async_tag.id]).await?;
```

Leia a partir de qualquer um dos dois:

```rust
let tags_on_post: Collection<Tag> = post.tags().get().await?;
let posts_with_rust_tag: Collection<Post> = rust_tag.posts().get().await?;
```

## O registro polimórfico

Toda struct anotada com `#[suprnova::model(morph_type = "...")]` emite
um [`MorphTypeEntry`][morph] via `inventory::submit!` em tempo de
compilação. O registro impulsiona três coisas:

1. **Dispatch do enum por família** - `MorphTo.get()` lê a string
   `<name>_type` da linha filha e a procura para achar a variante de
   enum certa.
2. **Filtragem de alvo do `MorphedByMany`** - `target_morph_type = "post"`
   se resolve através do registro para garantir que a string de tipo
   é real.
3. **Verificações de sanidade** - `find_morph_type("post")` retorna
   `None` se nenhum model se registrou com essa string, distinguindo
   "deliberadamente não registrado" de "erro de digitação".

```rust
use suprnova::{morph_types, find_morph_type, find_morph_type_by_id};
use std::any::TypeId;

for entry in morph_types() {
    println!("{} -> {}", entry.morph_type, entry.type_name);
}

if let Some(e) = find_morph_type("post") {
    assert_eq!(e.table, "posts");
}

let by_id = find_morph_type_by_id(TypeId::of::<Post>());
```

[morph]: https://docs.rs/suprnova

Models sem um attribute `morph_type = "..."` deliberadamente não se
registram - o registro é opt-in. Um model `User` não-polimórfico não
contribui nada para ele, o que é o que torna `find_morph_type("user")`
retornando `None` um sinal útil.

## Consultando por existência de relação

`has` / `where_has` / `doesnt_have` / `where_relation` /
`where_belongs_to` formam o motor de existência de relação do
Suprnova. Todos renderizam como subconsultas `EXISTS (...)`
correlacionadas contra o **próprio SELECT do pai** - sem JOIN, sem
linhas de pai duplicadas, sem GROUP BY.

```rust
// Usuários com pelo menos um post.
let with_posts = User::query().has("posts").get().await?;

// Usuários com pelo menos três posts.
let prolific = User::query().has_count("posts", ">=", 3).get().await?;

// Usuários com pelo menos um post PUBLICADO.
let published_authors = User::query()
    .where_has::<Post, _>("posts", |q| q.filter("published", true))
    .get()
    .await?;

// Usuários SEM posts.
let empty_users = User::query().doesnt_have("posts").get().await?;

// Usuários sem posts em DRAFT (podem ainda ter publicados).
let clean = User::query()
    .where_doesnt_have::<Post, _>("posts", |q| q.filter("published", false))
    .get()
    .await?;

// Atalho: where_has + uma única coluna == match.
let same = User::query()
    .where_relation("posts", "published", true)
    .get()
    .await?;

// where_belongs_to - FK direta = ? NESTA tabela (EXISTS não é
// necessário, já que a FK está na linha filha).
let mine = Post::query()
    .where_belongs_to("author", user.id)
    .get()
    .await?;
```

### Como funciona

O motor percorre o inventory de relação no momento de construção da
query. Para cada relação nomeada, ele pega a `RelationEntry` e
renderiza a forma de SQL apropriada por tipo:

- `HasOne` / `HasMany` / `MorphOne` / `MorphMany` →
  `EXISTS (SELECT 1 FROM child WHERE child.<fk> = parent.<pk>)`.
  Tipos morph adicionam `AND child.<name>_type = '<parent_morph_type>'`.
- `BelongsTo` →
  `EXISTS (SELECT 1 FROM parent WHERE parent.<pk> = child.<fk>)`.
- `BelongsToMany` / `MorphToMany` → faz join através do pivot:
  `EXISTS (SELECT 1 FROM pivot WHERE pivot.<parent_fk> = parent.<pk> ...)`.
- Relações Through → fazem join através do intermediário.

A forma de closure (`where_has::<R, _>(rel, |q| ...)`) constrói um
`Builder<R>` interno; quaisquer termos WHERE que esse builder produz
caem dentro do corpo da subconsulta. A numeração de placeholder é
monotônica através de toda a statement, então o motor funciona
corretamente com parâmetros no estilo `$1` do Postgres.

`where_belongs_to` é a única exceção que não renderiza um EXISTS. A
FK do belongs-to vive na *própria* linha do pai, então um
`WHERE child.<fk> = ?` direto é exatamente o SQL certo - nenhuma
subconsulta é necessária. Se o nome da relação é desconhecido no
inventory do pai, o motor emite `WHERE 1 = 0`, para que a query
retorne nada de forma segura.

### Por que isso é melhor que LEFT JOIN

O motor `has` / `whereHas` mais antigo do Laravel costumava emitir
JOINs e duplicar linhas de pai; a reescrita para EXISTS correlacionado
chegou no Laravel 9. O Suprnova entrega EXISTS desde o primeiro dia.
As vantagens: sem duplicatas no result set, sem workarounds de GROUP
BY para agregados, sem necessidade de `DISTINCT`, e o otimizador do
banco de dados vê uma subconsulta real em vez de um JOIN através do
qual ele não consegue empurrar predicados. Para
`has_count(rel, ">=", n)` o motor renderiza
`(SELECT COUNT(*) FROM child WHERE ...) >= n` diretamente - uma
query, um plano.

## Eager loading - `with`, `with_count`, agregados `with_*`

O `user.posts().get()` lazy faz uma query por pai. Isso é N+1 quando
você tem muitos usuários:

```rust
// Ruim: 1 consulta para usuários + 100 consultas para posts.
let users = User::query().limit(100).get().await?;
for u in &users {
    let posts = u.posts().get().await?;
    /* ... */
}
```

`with(["posts"])` colapsa isso para duas queries no total -
independentemente da contagem de pais:

```rust
// Bom: 1 consulta para usuários + 1 IN-query para todos os posts.
let users = User::query()
    .with(["posts"])
    .limit(100)
    .get()
    .await?;

for u in &users {
    for post in u.posts_loaded() {       // lê do cache, sem SQL
        println!("{}: {}", u.name, post.title);
    }
}
```

Caminhos aninhados também funcionam - nomes de relação separados por
ponto recursam:

```rust
let users = User::query()
    .with(["posts.comments.author"])
    .get()
    .await?;
// 4 consultas: users, posts IN users.id, comments IN posts.id, authors IN comments.user_id.
```

### `with_count` e agregados

`with_count` adiciona um agregado `COUNT(*) GROUP BY parent_fk` por
relação, carregado ao lado dos pais - uma query extra por relação:

```rust
let users = User::query().with_count(["posts"]).get().await?;
for u in &users {
    println!("{} has {} posts", u.name, u.posts_count());
}
```

Quatro variantes de agregado se acumulam: `with_sum`, `with_avg`,
`with_min`, `with_max`. A forma da chave de cache é
`<rel>_<kind>_<col>`, então acumular vários agregados na mesma
relação não colide:

```rust
let users = User::query()
    .with_count(["posts"])
    .with_sum(("posts", "views"))
    .with_avg(("posts", "views"))
    .get()
    .await?;

for u in &users {
    println!(
        "{}: {} posts, {} views total, {} avg",
        u.name,
        u.posts_count(),
        u.posts_sum_of("views").unwrap_or(0.0),
        u.posts_avg_of("views").unwrap_or(0.0),
    );
}
```

Veja [Eloquent → Eager loading → Layout do cache](eloquent.md#cache-layout)
para o contrato de armazenamento completo.

### Eager loads restritos - `with_where`

`with_where` filtra quais linhas filhas entram no cache eager sem
perder pais que não têm filhos correspondentes:

```rust
use suprnova::Builder;

let users = User::query()
    .with_where(("posts", |q: Builder<Post>| q.filter("published", true)))
    .get()
    .await?;
// Cada u.posts_loaded() contém só posts publicados.
// Usuários com zero posts publicados ainda aparecem no result set -
// o posts_loaded() deles retorna um slice vazio.
```

`with_where` difere de `where_has` na intenção: `where_has` filtra o
conjunto de pais ("usuários que têm pelo menos um post publicado");
`with_where` filtra o cache eager ("para todos os usuários, carregue
só os posts publicados deles"). Use os dois juntos quando você quiser
os dois efeitos.

O predicado é um `Fn`, não um `FnOnce`, então um builder carregando
um pode ser clonado e rodado mais de uma vez. Uma closure que quer
consumir um valor capturado deveria cloná-lo internamente:

```rust
let wanted = vec!["rust".to_string(), "web".to_string()];
let users = User::query()
    // `wanted.clone()` dentro, não um `move` do próprio `wanted` - a
    // closure pode rodar uma vez por clone do builder.
    .with_where(("posts", move |q: Builder<Post>| q.filter_in("tag", wanted.clone())))
    .get()
    .await?;
```

### Clonar uma consulta mantém seu plano de eager-load

`Builder` é `Clone`, e o clone carrega o plano de eager-load com ele,
então o padrão "construa uma consulta base, derive várias a partir
dela" funciona:

```rust
let base = User::query().with(["posts"]).filter("active", true);

let first_page = base.clone().limit(20).get().await?;
let total = base.count().await?;
// As linhas de first_page têm posts_loaded() populado.
```

### Por que Suprnova diverge

O `$query->with(...)` do Laravel clona livremente porque arrays do
PHP copiam na atribuição. O Rust tem que dizer o que um clone
significa para uma closure type-erased, e até a v0.7.2 o Suprnova
respondia descartando o plano - o clone tinha sucesso, a query tinha
sucesso, e as relações simplesmente ficavam ausentes. Compartilhar o
predicado através de um `Arc` torna o clone total, ao custo do
bound `Fn` acima.

Eager loading dentro de `chunk` / `chunk_by_id` / `lazy` continua
sendo um erro explícito em vez de um N+1 silencioso por chunk.
Reaplique `.with(...)` dentro da closure por chunk quando você
quiser esse comportamento.

### Carregando em coleções já buscadas

Quando você busca uma `Collection<M>` sem um plano de eager-load,
você pode conectar um depois do fato:

```rust
let mut users = User::query().get().await?;

users.load(["posts"]).await?;                 // incondicional
users.load_missing(["posts.comments"]).await?; // pula o que já está carregado
```

`load_missing` percorre o cache `__eager` de cada pai e só dispara a
query `IN` para linhas que ainda não carregaram a relação. Útil em
loops em que alguns pais foram eager-loaded antes na solicitação e
outros não.

### Excluindo - `without`

`without` remove relações nomeadas do plano de eager, útil quando um
scope base adiciona padrões que você não quer para esta chamada:

```rust
let users = User::query()
    .with(["profile", "posts", "team"])
    .without(["team"])     // remove team do plano
    .get()
    .await?;
```

## A válvula de escape

Quando uma relação não se encaixa em nenhum dos onze tipos - árvores
recursivas, polimorfismo através de chaves que não são id, pivots de
três vias, qualquer coisa sob medida - escreva o método à mão. A
macro não impede isso; você só não recebe o acessador carregado nem
o braço de dispatcher de eager-load para essa relação.

```rust
impl User {
    /// Personalizado: o post mais recente independente da forma da FK.
    pub async fn latest_post(&self) -> Result<Option<Post>, FrameworkError> {
        Post::query()
            .filter("user_id", self.id)
            .latest()
            .first()
            .await
    }
}
```

O trade-off é explícito: métodos escritos à mão não aparecem no
inventory de `relations()`, o motor de existência não sabe sobre
eles, e o eager loader não pode incluí-los em um plano. Para casos
isolados isso é tranquilo. Para qualquer coisa que você queira usar
com `with(["..."])`, declare-a como um tipo de relação de verdade,
mesmo que você tenha que usar as opções da macro para forçá-la a se
encaixar.

## Próximos passos

- [Eloquent](eloquent.md) - a superfície de model do dia a dia; a
  sintaxe de declaração de relação vive lá.
- [Banco de dados](database.md) - conexões, transações,
  multi-driver, a camada mais baixa em que tudo se apoia.
- [Migrações](migrations.md) - o lado de esquema das colunas de FK
  que essas relações precisam que existam.
- [Construtor de consultas](eloquent.md#query-builder-dual-api) - a
  superfície de API dual para a qual os wrappers de relação
  encaminham.
- [Recursos Eloquent](eloquent-resources.md) - transformar relações
  carregadas em payloads JSON:API para o cliente.
