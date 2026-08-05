# Cache

O Suprnova vem com uma facade `Cache` no formato Laravel, apoiada por
um de dois drivers - em memória ou Redis - escolhido explicitamente no
boot via `CACHE_DRIVER`. A facade é uma camada fina sobre uma trait
`CacheStore`, então backends customizados se encaixam do mesmo jeito
que os embutidos.

## A facade

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // hit
}

Cache::forget("user:1").await?;
```

Todo método serializa através de `serde_json` na fronteira da facade,
então qualquer `T: Serialize + DeserializeOwned` faz o round-trip. A
trait por baixo da facade (`CacheStore`) só vê strings JSON opacas.

## Inicialização

O cache é vinculado durante a etapa de inicialização de drivers do
`Server::run()` (veja [Ciclo de vida da solicitação](lifecycle.md)).
`Cache::bootstrap` lê o `CacheConfig` configurado (ou constrói um a
partir do env) e despacha com base em `CacheConfig::driver`:

- `Memory` - vincula um `InMemoryCache` com o prefixo configurado e o
  TTL padrão. Sempre tem sucesso.
- `Redis` - conecta a `REDIS_URL` e vincula o `RedisCache` resultante.
  **Falha de forma fechada** se a URL estiver inalcançável. Não há downgrade
  silencioso para memória.

Workers (`queue:work`, `schedule:run`, `workflow:work`) passam pela
mesma inicialização, então um job usando `Cache::get` vê o mesmo
backend que o handler HTTP vê.

### Por que Suprnova diverge

A config `cache.php` do Laravel escolhe um backend padrão, e o Laravel
silenciosamente troca para `array` (in-process) quando um backend
malconfigurado falha em alguns caminhos de código. Isso é um padrão
produtivo para o `php artisan tinker` e uma armadilha em produção - um
único miss do Redis muda silenciosamente as garantias de todo flush de
tag e de toda aquisição de lock no app.

O Suprnova escolhe o padrão oposto. `CACHE_DRIVER=memory` é explícito
(e o padrão para `cargo run`), e `CACHE_DRIVER=redis` contra um Redis
inalcançável retorna um erro de `Server::from_config`. O binário sai
com código não-zero com uma mensagem de correção; supervisord/systemd
vê uma falha de boot em vez de um app funcionando pela metade.

## Configuração

| Env | Significado | Padrão |
|---|---|---|
| `CACHE_DRIVER` | `memory` ou `redis` | `memory` |
| `REDIS_URL` | URL do Redis (consultada apenas quando `driver=redis`) | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | Prefixo de chave aplicado a toda operação do backend | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | TTL padrão em segundos para `Cache::put(None)`; `0` significa nenhum padrão | `3600` |

`CACHE_DRIVER` não definida faz parse para `Memory`; qualquer outro
valor (sem diferenciar maiúsculas/minúsculas, aparado) que não seja
`memory`/`in-memory`/`inmemory`/`redis` retorna um erro no boot.

Você também pode construir a config programaticamente quando não
quiser parsing de env:

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` é determinístico - campos não definidos
recorrem a `CacheConfig::default()` em vez de reler o env.

### O contrato do `forever` vale entre backends

`Cache::forever` e `Cache::remember_forever` ignoram
`CACHE_DEFAULT_TTL` inteiramente; o valor nunca expira independente do
padrão configurado. `Cache::put(key, value, None)` sim aplica o
padrão - esse é o propósito de tê-lo.

A resolução do TTL padrão acontece na camada da facade. Os dois
backends `CacheStore` honram `None` literalmente na fronteira do
backend (sem expiração), e é por isso que `forever` de fato significa
forever tanto em memória quanto no Redis.

## Leituras, escritas, exclusões

```rust
use suprnova::Cache;
use std::time::Duration;

// Escreve com um TTL explícito
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// Escreve para sempre - ignora CACHE_DEFAULT_TTL
Cache::forever("config:features", &features).await?;

// Lê (None em miss ou quando expirado)
let session: Option<Session> = Cache::get("session:42").await?;

// Existência - true significa presente e não expirado
if Cache::has("session:42").await? { /* … */ }

// Negação no estilo Laravel
if Cache::missing("session:42").await? { /* aquecer */ }

// Lê-e-exclui em uma única chamada
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// Retorna true se a chave existia e foi removida
Cache::forget("session:42").await?;

// Apaga tudo (escopo por prefixo nos dois backends)
Cache::flush().await?;
```

`Cache::pull` **não** é atômico - é um `get` seguido de um `forget`,
no mesmo formato do `Repository::pull` do Laravel. Para dequeue
atômico use `Cache::lock` (veja abaixo).

### Renove um TTL sem reescrever

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

`touch` retorna `true` se a chave existia e o TTL foi estendido,
`false` caso contrário. O valor armazenado não é tocado.

## Add - grava se ausente (atômico)

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

`Cache::add` só escreve se a chave estiver vazia (ou tiver expirado).
Retorna `true` na escrita, `false` em contenção. **Atômico** nos dois
backends embutidos:

- `InMemoryCache` mantém um write-lock durante toda a verificação de
  existência + inserção
- `RedisCache` usa `SET key value NX EX ttl` (ou `NX` sem `EX`)

Implementações customizadas de `CacheStore` que não sobrescrevem
`add_raw` recaem para um check-then-put não atômico, espelhando o
fallback do `Repository::add` do Laravel para backends sem um `add`
nativo.

## Remember - obtém-ou-computa

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` chama sua closure apenas em um miss, e então armazena o
resultado. A closure retorna `Result<T, FrameworkError>`, então falhas
de domínio se propagam através do `?` em vez de envenenar o cache.

`Cache::sear(key, default)` é o alias no estilo Laravel para
`remember_forever`. Mesmo corpo, mesma semântica - é distribuído sob
os dois nomes para que código migrado se leia do mesmo jeito.

### Remember NÃO é stampede-safe

`remember` é um par `get`-depois-`put` não atômico. N misses
concorrentes para a mesma chave fria executam a closure N vezes e
escrevem N resultados. Isso corresponde exatamente ao
`Repository::remember` do Laravel, e está tudo bem para o caso comum
(a closure é idempotente, as escritas são idênticas).

Não está tudo bem quando:

- A closure é cara (1s+ para computar ou atinge um upstream lento)
- A chave é popular o bastante para que um evento de cache frio envie
  N requisições de uma vez ao backend
- A closure tem efeitos colaterais além de computar o valor

Para esses casos, envolva com `Cache::lock`:

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// Perdeu a corrida - quem venceu está computando. Leia o que quer
// que tenha sido escrito, ou recaia para um valor stale.
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## Locks

`Cache::lock` retorna um `LockGuard` segurando o token de ownership.
Locks são consultivos e entre processos quando apoiados no Redis.

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// Some(guard) significa que nós o possuímos. None significa que outro dono chegou primeiro.
```

A guarda expõe:

| Método | Uso |
|---|---|
| `guard.token()` | Lê o token de ownership (nome do lado Rust) |
| `guard.owner()` | Mesmo valor, alias no estilo Laravel |
| `guard.refresh(ttl)` | Estende o TTL - retorna `false` se não somos mais donos do lock |
| `guard.release()` | Libera se ainda somos donos do lock - retorna `false` se o token não corresponde mais |

Intencionalmente **não há auto-release via `Drop`**. Um lock Redis
precisa ser reconhecido através de fronteiras de processo;
auto-release no drop ou roubaria de volta silenciosamente um lock já
roubado (errado) ou esconderia falhas de release em panics de
destrutor (pior). O release é explícito para que erros se propaguem.

`refresh` permite que um job de longa duração estenda seu próprio
lock para evitar um timeout autoinfligido - veja
[Idempotência](idempotency.md) para o consumidor in-tree.

## Contadores atômicos

```rust
// Inicializa em 0 se ausente, depois incrementa. Retorna o novo valor.
let visits = Cache::increment("page:visits", 1).await?;

// Mesmo formato para passos negativos
let remaining = Cache::decrement("quota:remaining", 1).await?;

// Quantidade customizada
let total = Cache::increment("stats:downloads", 10).await?;
```

Atômico nos dois backends embutidos: `InMemoryCache` usa um
`HashMap::entry` protegido por write-lock; `RedisCache` usa
`INCRBY`/`DECRBY`. O valor armazenado é um inteiro codificado em JSON,
então `Cache::get::<i64>("page:visits")` faz o round-trip com a mesma
chave.

## Cache com tags

Tags deixam você invalidar uma família inteira de entradas
relacionadas com uma única chamada. O caso de uso clássico é caches
por recurso que precisam dar flush juntos quando o recurso muda.

```rust
use suprnova::Cache;
use std::time::Duration;

// Armazena sob uma ou mais tags
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// Caminho de atualização: descarta toda chave marcada com `user:1`
Cache::flush_tags(&["user:1"]).await?;
```

A associação a tags é **por entrada**: cada escrita marcada com tags
instala o conjunto de tags daquela escrita como a fonte da verdade da
entrada, substituindo quaisquer tags anteriores. Duas consequências
que vale a pena conhecer:

- Um `Cache::put` sem tags sobre uma chave anteriormente marcada
  **limpa** as tags da entrada. Um `flush_tags` subsequente da tag
  antiga não vai excluir o valor sem tags que está ativo.
- Sobrescrever `tags_put(&["a"], …)` com `tags_put(&["b"], …)` faz a
  entrada responder somente a `flush_tags(&["b"])`.

Referências stale de forward-index são removidas durante a varredura
de flush e em `flush()`, então elas não se acumulam indefinidamente
para tags que são escritas mas nunca sofrem flush.

## Dois backends

| Recurso | `InMemoryCache` | `RedisCache` |
|---|---|---|
| Compartilhado entre processos | Não | Sim |
| Persistência | Não | Sim, se o Redis estiver configurado para isso |
| `add` atômico | Sim (write-lock) | Sim (`SET NX`) |
| `increment`/`decrement` atômico | Sim (write-lock) | Sim (`INCRBY`/`DECRBY`) |
| Cache com tags | Sim | Sim |
| Locks | Sim | Sim (entre processos) |
| TTL sub-segundo | Sim (`tokio::time::Instant`) | Sim (`PX`/`PEXPIRE`) |
| Selecionado via | `CACHE_DRIVER=memory` (padrão) | `CACHE_DRIVER=redis` |

Não há um driver de cache de banco de dados - os dois backends acima
são os que o framework traz. Backends customizados podem implementar
`CacheStore` e se vincular ao contêiner diretamente; veja o padrão de
injeção de teste abaixo.

### Expiração em memória

`InMemoryCache` remove entradas expiradas **de forma lazy na
leitura**: `get_raw`, `has`, e `add_raw` descartam uma entrada na
primeira vez que a observam expirada. Chaves reacessadas nunca
acumulam cadáveres.

Uma carga de trabalho que escreve um conjunto de chaves de curta
duração com alta cardinalidade e nunca as lê de volta não tem esse
gatilho. Chame `InMemoryCache::purge_expired()` a partir de uma task
periódica nesse caso - ela retorna a contagem de entradas removidas. O
Redis lida com sua própria expiração no lado do servidor; o
equivalente não é necessário lá.

### Precisão de TTL do Redis

Todo TTL do Redis passa por `PX` / `PEXPIRE`, não `EX` / `EXPIRE`.
Isso evita duas armadilhas:

- `Duration`s sub-segundo truncariam para `0 seconds` sob `EX`, que o
  Redis rejeita (`SET … EX 0`) ou, pior, interpreta como "exclua a
  chave" (`EXPIRE key 0`).
- `Duration::ZERO` é limitado a 1 ms antes da chamada, então nenhum
  dos dois caminhos de rejeição é alcançável a partir do código do
  usuário.

## Testes

Vincule um `InMemoryCache` ao `TestContainer` e a facade o resolve
como qualquer outro backend:

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` escreve no escopo thread-local, então testes
paralelos não vazam estado de cache uns nos outros. Veja o capítulo
[Contêiner de serviços](container.md) para o modelo de lookup em três
camadas.

## Padrões

Alguns formatos recorrentes que vale a pena nomear:

```rust
// Chaves hierárquicas separadas por dois-pontos - a mesma convenção que o Laravel usa
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// TTL pela volatilidade dos dados
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// Invalidação por tag ao redor de uma escrita
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## Próximos passos

- [Configuração](configuration.md) - como `Config::register` e as
  variáveis de env se combinam
- [Limitação de taxa](rate-limiting.md) - a facade `RateLimiter` no
  formato Laravel é construída em cima do `Cache`
- [Idempotência](idempotency.md) - o middleware de dedupe de
  solicitação usa `Cache::lock` de ponta a ponta
- [Contêiner de serviços](container.md) - como `CacheStore` é
  vinculado e resolvido
- [Modelo de erros](error-model.md) - o que `Cache::*` retorna quando
  o Redis está inalcançável no meio de uma solicitação
