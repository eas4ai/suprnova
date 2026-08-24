# Coleções Eloquent

`Collection<T>` é o tipo de coleção no formato Laravel do Suprnova - o
valor de retorno de `Builder::get`, `Model::all`, todo `pluck`, todo
terminal de carregamento de relação que produz mais de uma linha. É
um wrapper fino em torno de `Vec<T>` que faz deref para `&[T]`, então
todo método de slice já existente (`.len()`, `.iter()`, indexação,
`.contains(&v)`) funciona sem alteração. Por cima está a superfície
do Laravel: map, filter, pluck, group_by, sort_by, where_eq, sum, avg,
entre outros.

Este capítulo é a referência independente da superfície de coleção.
O capítulo pai [API Eloquent](eloquent.md) a resume; este capítulo
passa por todo método, o contrato de empréstimo vs consumo, a regra
de serialização que vai te pegar se você ignorá-la, e quando descer
para `Vec<T>` em vez disso.

## Sumário

- [De onde vêm as coleções](#de-onde-vêm-as-coleções)
- [Os dois blocos impl](#os-dois-blocos-impl)
- [Superfície genérica - funciona em qualquer `Collection<T>`](#superfície-genérica-funciona-em-qualquer-collection-t)
- [Superfície com conhecimento de model - `Collection<M>` onde `M: Model`](#superfície-com-conhecimento-de-model-collection-m-onde-m-model)
- [Eager loading em uma coleção](#eager-loading-em-uma-coleção)
- [Serialização - `to_array` vs serde](#serialização-to-array-vs-serde)
- [Empréstimo vs consumo](#empréstimo-vs-consumo)
- [Collection vs `Vec`](#collection-vs-vec)
- [`LazyCollection<M>` - resultados em streaming](#lazycollection-m-resultados-em-streaming)
- [Por que Suprnova diverge](#por-que-suprnova-diverge)
- [Próximos passos](#próximos-passos)

## De onde vêm as coleções

Todo terminal que retorna mais de uma linha te entrega uma
`Collection<M>`:

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

Você também pode envolver qualquer `Vec<T>` que já tenha:

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` implementa `Default`, `Clone`, `Serialize`,
`Deserialize`, `PartialEq`, e `IntoIterator` (tanto por valor quanto
por `&`). É `Send` quando `T: Send`.

## Os dois blocos impl

Os métodos em `Collection` se dividem em duas famílias com base no
parâmetro de tipo.

```rust
impl<T> Collection<T> { /* generic methods - work for any T */ }

impl<M> Collection<M> where M: Model { /* string-keyed model methods */ }
```

O bloco genérico te dá `map`, `filter`, `reject`, `chunk`, `first`,
`last`, `unique`, e uma versão baseada em closure de todo acessador
de coluna (`pluck_by`, `group_by_with`, `sort_with`, `key_by_with`).
Estes funcionam em `Collection<i32>`, `Collection<String>`,
`Collection<MyDto>`, qualquer coisa.

O bloco com conhecimento de model adiciona açúcar chaveado por string
(`pluck("name")`, `group_by("role")`, `sort_by("created_at")`,
`sum::<f64>("balance")`) que roteia por linha através do acessador
`Model::field_value` emitido pela macro. Estes só existem quando `T`
implementa `Model`.

Escolha a forma por closure quando puder - o type checker valida o
acesso ao campo. Escolha a forma chaveada por string quando estiver
seguindo a sintaxe do Laravel, ou quando o nome da coluna é um valor
de runtime.

## Superfície genérica - funciona em qualquer `Collection<T>`

### Leitura

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - de Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` recebem `&&T` porque o predicado roda
através de `Iterator::find` em `Iter<'_, T>`. Faça deref duas vezes
(`**n`).

### Transformando - consome `self`, retorna uma nova coleção

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` muda o tipo do elemento:

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` executa um efeito colateral e mantém a coleção para continuar
o encadeamento (o Suprnova diverge do Laravel aqui de propósito - veja
abaixo):

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### Agrupamento e ordenação chaveados por closure

```rust
use std::collections::HashMap;

// Agrupa itens por chave derivada de closure.
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// Indexa itens por chave derivada de closure (duplicatas posteriores sobrescrevem).
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// Ordena por comparador derivado de closure.
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// Remove duplicatas por chave derivada de closure.
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// Projeta cada item por closure em uma nova coleção.
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

O sufixo `*_with` / `*_by` é a convenção de nomenclatura universal
para "este método recebe uma closure" no bloco genérico. O bloco com
conhecimento de model descarta o sufixo e recebe uma string de nome
de coluna em vez disso.

### Reduzindo e agregando

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

Para agregados numéricos tipados em coleções de model, veja `sum` /
`avg` / `min` / `max` na seção com conhecimento de model - eles
funcionam em qualquer campo que desserialize para um tipo numérico.

### Operações de conjunto

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // alias de concat
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` são aliases - o Laravel traz os dois nomes. `diff`
/ `intersect` são O(n*m); se você tem coleções grandes, projete para
um `HashSet` primeiro.

### Amostragem aleatória

```rust
let one: Option<&i32>     = nums.random();        // pega um por empréstimo
let many: Collection<i32> = nums.clone().random_n(3); // escolhe 3
```

Os dois usam o RNG thread-local (`rand::rng()`). Passe um RNG
semeado manualmente se você precisar de determinismo em testes.

## Superfície com conhecimento de model - `Collection<M>` onde `M: Model`

Estes métodos só existem quando o tipo contido é um model Suprnova.
Eles roteiam leituras por linha através do acessador
`Model::field_value(name)` emitido pela macro, que retorna
`Option<serde_json::Value>`. Linhas cujo campo não existe ou não
desserializa para o tipo de destino são silenciosamente ignoradas -
combinando com o comportamento de chave ausente do Laravel.

### Projeção

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` toma por empréstimo (`&self`), então a coleção original ainda
está disponível depois. O parâmetro tipado (`::<String>`) é o tipo de
destino para o qual o valor JSON é desserializado.

`pluck_keyed` produz um `HashMap<K, V>` a partir de duas colunas:

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

Linhas posteriores sobrescrevem as anteriores para a mesma chave.

`model_keys` é o atalho para a chave primária e a única projeção que
retorna um `Vec` simples em vez de uma `Collection`:

```rust
let users: Collection<User> = User::query().get().await?;
let ids: Vec<i64> = users.model_keys();
```

Ele lê o campo de chave já hidratado, portanto não custa uma query. Quando
você quer apenas as chaves e ainda não carregou as linhas, use o terminal
do builder - `User::query().model_keys().await?` projeta a coluna de chave
sem hidratar nada. `Vec` em vez de `Collection` corresponde a
`modelKeys()` do Laravel e mantém as duas metades do par de acordo sobre
um único formato.

### Agrupamento e indexação

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

Os dois métodos convertem o valor da coluna em uma chave `String`.
Uma coluna numérica `id` chega como `"1"` / `"2"` - combinando com o
contrato de `groupBy('team_id')` do Laravel, em que a saída é sempre
chaveada por string independentemente do tipo subjacente.

Se você quiser chaves tipadas, use a forma por closure no bloco
genérico:

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### Filtragem

Os métodos `where_*` com conhecimento de model recebem
`serde_json::Value` porque comparam contra a forma codificada em JSON
da coluna:

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` e `where_in` descartam linhas cujo `field_value` retorna
`None`. `where_not_in` *mantém* linhas em que o campo está ausente -
a negação de "está no conjunto" é "não está no conjunto OU está
ausente".

### Ordenação

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

A comparação é best-effort entre formas de valor JSON: numérico vs
numérico e string vs string ordenam de forma limpa dentro do próprio
tipo; colunas heterogêneas mistas caem de volta para
`Ordering::Equal`. `None` ordena antes de qualquer valor presente
(espelha o `NULL FIRST` do Postgres para ASC).

Os dois métodos clonam o `Vec<M>` subjacente antes de ordenar porque
o comparador toma `m.field_value(field)` por empréstimo enquanto
`sort_by` precisa de `&mut [M]`. Se você tem um loop apertado, ordene
com `sort_with` no bloco genérico em vez disso - ele opera no lugar.

### Agregados

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` retorna `T::default()` quando nenhuma linha contribui com um
valor (zero para tipos numéricos). Os outros três retornam `None`
para que quem chama não divida por zero nem compare contra um padrão
fantasma.

O parâmetro tipado (`::<f64>`) é o destino de desserialização JSON.
Escolha o tipo numérico mais amplo que sua coluna razoavelmente usa -
`i64` para colunas inteiras, `f64` para decimal/float,
`chrono::DateTime<Utc>` para timestamps, etc.

## Eager loading em uma coleção

Quando você já tem uma `Collection<M>` e quer carregar relações em
cada linha, use `load` / `load_missing`:

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

Os dois métodos recebem `&mut self` (eles alteram o cache eager por
linha) e são `async`. Os dois aceitam a mesma sintaxe de caminho
pontilhado que `Builder::with([...])` aceita - `"posts"`,
`"posts.comments"`, `"posts.comments.author"`.

`load_missing` particiona por linha. Linhas que já têm a relação em
cache são deixadas em paz; linhas que não têm recebem o carregamento
em massa:

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// Algumas linhas já têm posts em cache. load_missing só toca no
// resto - e recursiona nos posts já cacheados em busca de `comments`.
users.load_missing(["posts.comments"]).await?;
```

A recursão roda em cada segmento de um caminho pontilhado mais longo.
Com `"a.b.c"`, cada linha é particionada em cada nível: `a` é
carregado só onde está ausente, depois, para as linhas que já tinham
`a`, `b` é carregado só onde está ausente nesses `a`s, etc.

Os dois métodos respeitam o roteamento de
`#[model(connection = "...")]` - eles resolvem a mesma conexão de
onde a linha foi originalmente carregada.

## Serialização - `to_array` vs serde

Esta é a única armadilha na superfície de coleção. Leia com atenção.

`Collection<T>` deriva `Serialize`. Então isto funciona:

```rust
let json: String = serde_json::to_string(&users)?;
```

Mas - a impl blanket `Serialize for Vec<T>` do serde chama
`T::serialize` diretamente em cada elemento. Isso **contorna** o
override de `Model::to_array()` que a macro `#[suprnova::model]`
emite. O que significa que contorna seus attributes de model
`hidden = ["password"]`, `visible = [...]`, e `appends = [...]`.

Se seu model tem campos hidden, **não** serialize a coleção através
do serde. Use `to_array()` ou `to_json()`:

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

Os dois métodos roteiam através de `Model::to_array()` para cada
linha, então o pipeline de filtro por model se aplica - campos hidden
continuam hidden, allowlists de visible são impostas, `appends`
vindos de acessador aparecem.

A mesma ressalva se aplica a qualquer coisa que chame
`serde_json::to_value(&collection)` por baixo dos panos:
`Inertia::render` quando você coloca uma coleção em props,
`JsonApi`/`Resource` se você passar models crus em vez de structs de
recurso, log shippers que codificam seus payloads via serde. O padrão
seguro é converter através de um tipo de recurso ([Recursos
JSON:API](eloquent-resources.md)) ou através de `to_array()` antes de
o valor chegar a qualquer codepath do serde.

Para coleções de tipos que não são model (`Collection<MyDto>`,
`Collection<String>`) o caminho do serde é tranquilo - o problema só
se aplica quando `T` é uma struct `#[suprnova::model]` com
hidden/visible/appends declarados.

## Empréstimo vs consumo

Os métodos se dividem claramente em dois contratos:

| Recebe | Métodos |
|---|---|
| `&self` (empréstimo) | `len`, `is_empty`, `is_not_empty`, `first`, `last`, `first_where`, `last_where`, `contains_where`, `random`, `as_slice`, `pluck_by`, `pluck`, `pluck_keyed`, `group_by`, `key_by`, `sum`, `avg`, `min`, `max`, `to_array`, `to_json` |
| `self` (consumo) | `map`, `filter`, `reject`, `each`, `reduce`, `chunk`, `take`, `skip`, `slice`, `reverse`, `shuffle`, `random_n`, `unique`, `unique_by`, `sort_with`, `sort_by`, `sort_by_desc`, `where_eq`, `where_in`, `where_not_in`, `concat`, `merge`, `diff`, `intersect`, `group_by_with`, `key_by_with`, `map_to_map` |
| `&mut self` | `load`, `load_missing` |

Se você quiser manter a coleção depois de uma chamada que consome,
dê `.clone()` antes da chamada. `Collection<T>: Clone` quando
`T: Clone`.

Um padrão prático: leia primeiro, transforme por último:

```rust
let users: Collection<User> = User::all().await?;

// Leituras por empréstimo primeiro - a coleção ainda está viva depois de cada uma.
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// Agora consome.
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## Collection vs `Vec`

O wrapper é intencionalmente fino. As rotas de conversão vão nos dois
sentidos e continuam baratas:

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` te dá todo método de slice automaticamente.
Isso inclui:

```rust
let users: Collection<User> = User::all().await?;

users.len();             // método de slice
users.iter();            // método de slice
users[0].name.clone();   // indexação de slice
users.contains(&u);      // método de slice
users.binary_search(&u); // método de slice
&users[1..4];            // subscrito de slice
```

`IntoIterator` é implementado duas vezes - para `Collection<T>` (por
valor) e `&Collection<T>` (por referência), então os dois funcionam:

```rust
for user in &users {           // itera por &User
    /* ... */
}

for user in users.clone() {    // itera por User (consome)
    /* ... */
}
```

`DerefMut` só produz `&mut [T]` - um slice, não um `Vec`. Isso
significa que a mutação no lugar de campos de elemento funciona:

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

Mas a mutação de `Vec` owned (`push`, `pop`, `clear`, `truncate`) não
está disponível diretamente na coleção - chame `into_vec()` primeiro:

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

Isso é deliberado. A superfície do Laravel trata uma coleção como um
snapshot imutável que você transforma com métodos encadeados; a
mutação owned da sequência interna é o contrato de `Vec`, não o
contrato de `Collection`.

### Quando descer para `Vec`

Use `into_vec()` quando:

- Você precisa de métodos específicos de `Vec` (`push`, `pop`,
  `swap_remove`, `drain`, `with_capacity`).
- Você está entregando os dados para uma API que recebe `Vec<T>` por
  valor e você não quer o wrapper na assinatura.
- Você está guardando as linhas a longo prazo na sua própria struct e
  a superfície do Laravel não te traz nada.

Para todo o resto - retornos de handler, transformações, props do
Inertia (desde que você respeite a [regra de
serialização](#serialização-to-array-vs-serde)) - mantenha a
`Collection<T>`.

## `LazyCollection<M>` - resultados em streaming

`Collection<M>` materializa toda linha em memória. Para datasets
grandes demais para caber, o builder oferece três terminais de
streaming que retornam `LazyCollection<M>` em vez disso:

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| Método | Estratégia |
|---|---|
| `Builder::lazy()` | Paginação por cursor de PK com o tamanho de lote padrão (1000) |
| `Builder::lazy_by_id(n)` | Paginação por cursor de PK com tamanho de lote `n` |
| `Builder::cursor()` | Alias do Laravel para `lazy()` |

Por baixo dos panos, `LazyCollection<M>` é um
`Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>`, mas
expõe `.next().await` diretamente, então você não precisa importar
`futures::StreamExt`. Cada `.next()` dispara a entrega da próxima
linha; a busca em lote subjacente só roda quando o buffer do lote
atual esvazia, então um consumidor lento não acumula linhas.

O wrapper é `Send` (então ele atravessa `tokio::spawn`) mas não
`Sync` - é um stream de consumidor único por construção.

Veja [Eloquent - chunking e iteração lazy](eloquent.md#chunking-and-lazy-iteration)
para a orientação completa sobre qual padrão de streaming escolher.

## Por que Suprnova diverge

O `Illuminate\Support\Collection` do Laravel é mutável:
`$c->filter(...)` modifica o array interno do mesmo objeto e retorna
`$this` para encadeamento. O PHP não tem ownership, então esse
contrato é invisível.

O Rust tem ownership, e fingir que não teria tornaria a superfície de
coleção desonesta. O Suprnova escolhe a forma de semântica de valor
em vez disso: toda transformação consome `self` e retorna uma nova
`Collection`. Você vê o custo no seu próprio código - se você quiser
manter a original, dá `.clone()`. Se não quiser, não dá.

Essa escolha se propaga pelo resto da superfície:

- **`each` retorna `Self`** em vez de `&self` para que uma chamada de
  efeito colateral (log, métricas) não quebre um encadeamento. O
  `each` do PHP roda por efeito e retorna a coleção; você não
  conseguiria fazer `$c->each(...)->filter(...)` de forma limpa sem
  buscar de novo. No Rust, movemos `self` adiante, mantendo o
  encadeamento fluido.

- **Alternativas chaveadas por closure para todo método chaveado por
  string.** `pluck_by`, `group_by_with`, `key_by_with`, `sort_with`,
  `unique_by`, `map_to_map`, `contains_where`. As closures deixam
  você ler campos que o type checker valida em vez de strings que o
  compilador não consegue ver. As formas chaveadas por string existem
  para paridade de sintaxe com o Laravel e para nomes de coluna
  decididos em runtime.

- **`sum` / `avg` / `min` / `max` recebem parâmetros tipados
  `::<T>`.** A versão PHP do Laravel faz cast na hora; no Rust, o
  destino de desserialização é parte da chamada. Linhas cujo valor
  não faz round-trip para `T` são silenciosamente ignoradas
  (combinando com o comportamento de chave ausente do Laravel), mas
  você escolhe o tipo intencionalmente.

- **`Deref<Target = [T]>`, não `Deref<Target = Vec<T>>`.** Uma
  `Collection` é conceitualmente um "snapshot de linhas", não um
  buffer mutável. Métodos de slice vêm através de `Deref`; se você
  quiser `push`/`pop`, `into_vec()` te dá o `Vec` cru e remove
  qualquer pretensão.

- **A serialização diverge em serviço da correção.** `to_array` e
  `to_json` roteiam através de `Model::to_array()` para que
  hidden/visible/appends por model se apliquem; o bypass da impl
  blanket `Serialize for Vec` do serde é documentado como a [armadilha
  que é](#serialização-to-array-vs-serde). O `toArray()` do Laravel
  faz o mesmo roteamento; só precisamos nomear a lacuna
  explicitamente porque usuários de Rust vão recorrer a
  `serde_json::to_string` por reflexo.

O trade-off é exatamente o mesmo que o Suprnova faz em todo lugar:
forma de superfície do Laravel, semântica de valor do Rust.

## Próximos passos

- [API Eloquent](eloquent.md) - o capítulo pai, com o construtor de
  consultas, relações, scopes, e o ciclo de vida completo do model.
- [Recursos JSON:API](eloquent-resources.md) - structs de recurso
  serializam coleções através de `IntoJsonResource` com sparse
  fieldsets e cadeias `?include=`; a forma certa para qualquer
  coleção que sai da sua API.
- [Frontend - Respostas Inertia](frontend-inertia-responses.md) - as
  regras para entregar coleções a props do Inertia sem cair na
  armadilha de serialização.
- [Validação](validation.md) - payloads de solicitação frequentemente
  produzem vetores que você envolve em `Collection` para
  processamento posterior.
- [Testes](testing.md) - padrões para fazer assertions sobre o
  conteúdo de coleções (tamanho, elementos contidos, ordem) dentro de
  testes de handler e de model.
