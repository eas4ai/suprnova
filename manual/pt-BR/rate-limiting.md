# Limitação de taxa

O Suprnova distribui duas superfícies de limitação de taxa
complementares:

| Superfície | Use quando... | Backend |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | Você quer aplicação estrita de janela deslizante contra armazenamento arbitrário (Redis ZSET, deque em memória) | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | Você quer limitadores nomeados no formato Laravel, callbacks de workflow de `attempt()`, ou headers de resposta `X-RateLimit-*` | Backend `Cache` (memory ou Redis) |

O driver de janela deslizante é a forma nativa do Suprnova -
um slot por solicitação, nenhuma chave de timer separada,
avaliação atômica de Lua no Redis. A facade Laravel é o que
apps migrados buscam e o que o padrão de limitador-nomeado /
callback-de-resposta exige. Os dois coexistem por design, e
uma rota pode usar camadas dos dois.

## SPI do driver de janela deslizante

`RateLimiterDriver` é a SPI de armazenamento para o algoritmo
de janela deslizante. Cada chave rastreia um deque de
timestamps de hit. Em todo `try_acquire`, entradas mais
antigas que `now - window` são removidas; se a contagem
restante estiver abaixo de `max_requests`, `now` é anexado e
a chamada aceita. Caso contrário, ela rejeita.

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait é o Option<Duration> até que o slot mais antigo no
    // bucket expire.
}
```

### Drivers embutidos

| Driver | Armazenamento | Selecionado via |
|--------|---------|--------------|
| `InMemoryRateLimiter` | `HashMap<String, Bucket>` por processo, com `tokio::time::Instant` para que testes `start_paused` possam controlar o relógio | `RATE_LIMIT_DRIVER=memory` (padrão) |
| `RedisRateLimiter` | Redis ZSET + check-and-record atômico em Lua | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` conecta o driver correspondente no
contêiner. Fora de produção, um valor de driver desconhecido
recai para memory com um log `warn!`.

### Produção falha de forma fechada no driver em memória

Em produção, resolver para o limitador em memória é uma falha
de boot:

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

O driver em memória mantém seus buckets no heap de um único
processo. Atrás de N réplicas, cada uma mantém sua própria
contagem, então um throttle de redefinição de senha de "5
tentativas por 15 minutos" é na verdade 5N, e todo deploy zera
todos eles. O limite que você configurou não é o limite que
você recebe - e nada avisa isso, porque as solicitações têm
sucesso, que é como um throttle funcionando se parece de
fora. Isso surge como um incidente de credential-stuffing ou
enumeração de contas, não como um erro.

Um valor de driver **não reconhecido** falha pelo mesmo
motivo: ele recai para memory. `RATE_LIMIT_DRIVER=Redis` - com
letra maiúscula - do contrário avisaria uma vez no boot e
deixaria silenciosamente uma implantação multi-réplica fazendo
throttling por processo. Esse é o caso mais provável de
chegar à produção, porque parece configurado.

Ou aponte ele para o Redis:

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

ou, se você realmente roda um único processo, diga isso:

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

Development, testing e **staging** não são afetados. Staging
deliberadamente não é bloqueado, pelo mesmo raciocínio da
verificação de mail: falhar de forma dura empurra times a
definir a sobrescrita globalmente, o que desarma a verificação
exatamente onde ela importa.

### `RateLimitMiddleware`

O wrapper HTTP em torno do driver. Construa com uma closure
`key_fn` para guiar a seleção de bucket por solicitação:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Na rejeição (acima da cota), ele retorna HTTP 429 com um
header `Retry-After`.

### Limitando por destinatário, não só por chamador

Um limite com chave de endereço responde *um cliente está
fazendo solicitações demais?*. Ele não consegue responder
*uma caixa de entrada está sendo inundada?*. Um atacante
espalhado por uma botnet, um pool de proxies, ou um único
`/64` IPv6 fica abaixo de todo orçamento por IP enquanto
envia a uma vítima milhares de emails de redefinição de
senha - a caixa de entrada é o recurso sendo exaurido, e o
endereço da vítima é a única coisa que essas solicitações
compartilham. O inverso também prejudica: atrás de um NAT
carrier-grade ou um gateway de escritório, limites por IP
punem uma multidão pelo comportamento de um membro.

`identity_key` usa como chave um bucket na conta que está
*sendo afetada*:

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Empilhe isso *ao lado* de um limitador por IP, em vez de
substituir um pelo outro. Cada um captura o que o outro não
consegue: por IP impede um host de enumerar muitos endereços;
por destinatário impede muitos hosts de atacar um único
endereço.

Três detalhes carregam a segurança:

- **`key_reads_body`** armazena o corpo em buffer (até o
  limite dado) antes que a chave seja computada, para que o
  campo possa ser lido tanto de um POST form-encoded quanto de
  uma query string. É opt-in porque bufferizar é trabalho que
  um chamador não autenticado consegue te fazer fazer; o
  limite o restringe. Um corpo acima do limite é rejeitado com
  413 em vez de passado sem chave - do contrário, preencher o
  corpo seria uma forma de escapar do limite.
- **`only_when`** pula o limitador para solicitações que não
  nomeiam ninguém. Sem isso, essas cairiam no fallback de
  endereço de `identity_key` e seriam contadas contra a cota
  *deste* limitador - e como um orçamento por destinatário
  normalmente é o mais apertado dos dois, ele se tornaria
  silenciosamente o limite vinculante para toda rota que não
  nomeia ninguém.
- **O valor é normalizado e tem hash aplicado.**
  `Alice@Example.com` e `alice@example.com` alcançam a mesma
  caixa de entrada e precisam compartilhar um bucket, ou o
  limite é contornado ao mudar a capitalização. O resultado é
  hasheado porque um backend de rate-limit é frequentemente um
  Redis compartilhado com controle de acesso mais fraco que o
  banco de dados primário, e um dump de chaves não deveria se
  ler como uma lista de quem está redefinindo sua senha.

### Política de erro de backend

`BackendErrorPolicy` governa o que acontece quando o *backend*
do limitador em si dá erro - por exemplo, o Redis está
inalcançável - distinto de uma solicitação legitimamente
excedendo sua cota. O backend não consegue tomar uma decisão,
então o middleware precisa escolher entre disponibilidade e a
garantia do limite.

| Política | Comportamento | Quando usar |
|--------|-----------|-------------|
| `FailOpen` (padrão) | Deixa a solicitação passar; loga em `warn` | A maioria das APIs públicas - uma queda do limitador não deveria derrubar o tráfego |
| `FailClosed` | Rejeita com HTTP 503 + `Retry-After: 1`; loga em `error` | Rotas sensíveis (login, redefinição de senha, pagamentos) onde tráfego sem limite durante uma queda de backend é peor que rejeitar brevemente |

Escolha com `.on_backend_error(BackendErrorPolicy::FailClosed)`
no middleware. Solicitações com cota exaurida são sempre 429
independentemente da política - a política só afeta o
fallthrough de erro de backend.

## Facade no formato Laravel apoiada em Cache

`RateLimiter` (a struct) espelha `Illuminate\Cache\RateLimiter`.
É um contador de janela fixa construído sobre a facade
[`Cache`](cache.md) do Suprnova. Use-o para limitadores
nomeados, workflows de `attempt()`, ou qualquer vez que você
queira os headers `X-RateLimit-*` que apps Laravel esperam.

### Layout de armazenamento

Para uma chave de contador de tentativas `K` com decay de `D`
segundos:

- `K` - contador i64 incrementado a cada `hit`. O valor
  inicial é 0 (via `Cache::add`).
- `K:timer` - i64 de unix-seconds-since-epoch de quando a
  janela termina, definido via `Cache::add` para que só o
  primeiro chamador em uma janela fixe o deadline.

As duas chaves carregam o mesmo TTL, então o cache as limpa
automaticamente quando a janela termina. Quando o contador
alcançou `max_attempts` mas o `:timer` já se foi,
`too_many_attempts` reseta o contador - isso é o que faz a
janela deslizar para frente depois de um período de cota
exaurida.

### API de contador

```rust
use suprnova::RateLimiter;

// Consome uma tentativa; inicia a janela se ausente.
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// Consome uma tentativa E testa o limite em uma única
// viagem de ida e volta atômica. Retorna `true` quando esse
// hit empurrou o bucket além de `max` (recuse a
// solicitação), `false` quando foi admitido. Use isso em vez
// de um par separado `too_many_attempts` + `hit`: verificar
// e depois consumir como duas chamadas deixa solicitações
// concorrentes escaparem do limite (uma corrida de
// check-then-act).
// `i64::MAX` como o max significa "sem limite" - sempre
// admite, ainda conta.
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* return 429 */ }

// Incrementa por N; útil para limites "com peso de custo"
// (cada solicitação consome mais de uma tentativa).
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// Lê a contagem atual (0 quando nunca teve hit ou expirou).
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// Número de segundos até a janela reabrir (0 quando
// nenhuma janela está aberta).
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// Retries restantes antes de disparar.
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left é o alias no formato Laravel de remaining.
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// O bucket está acima do limite AGORA MESMO (com a janela
// ainda aberta)?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// Descarta só o contador (o timer permanece - a janela
// ainda está fixada).
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// Descarta tanto o contador quanto o timer.
RateLimiter::clear("login:1.2.3.4").await?;
```

### Workflow de `attempt()`

Ele roda um callback só quando o bucket está dentro da cota; a
tentativa só é consumida quando o callback roda:

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* callback rodou, tentativa contada */ }
    None => { /* acima do limite, callback NÃO rodou */ }
}
```

Essa é a forma certa para formulários de login - você não
consome uma tentativa a menos que o trabalho realmente tenha
alcançado o callback.

### Limitadores nomeados

Registro no boot, resolução no momento da solicitação. O nome
do lado Laravel `for` é uma palavra-chave reservada do Rust,
então o nome primário do lado Rust é `define`; o alias literal
do Laravel é exposto via `r#for`.

```rust
use suprnova::{Limit, RateLimiter};

// No boot - `define` é o nome primário do lado Rust.
RateLimiter::define("api", |req| {
    // `req.ip()`, não o header `X-Forwarded-For` bruto - veja abaixo.
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Alias do lado Laravel - a mesma coisa sob a grafia de keyword-escape.
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// Resolve.
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

Um callback de limitador nomeado retorna um [`LimitResult`],
construtível a partir de:

- Um único `Limit` - aplica esse limite.
- Um `Vec<Limit>` - aplica todo limite; o primeiro a disparar
  vence.
- Um `HttpResponse` - faz short-circuit imediatamente com essa
  resposta (usado para "admin ganha acesso ilimitado" via
  `Limit::none()`, ou para recusar a solicitação
  completamente).

### Sanitizando chaves

`RateLimiter::clean_rate_limiter_key(key)` remove marcadores de
HTML-entity `&abc;` de uma chave - o Laravel usa isso para
strings fornecidas pelo usuário que fazem round-trip através
de `htmlentities`. O Suprnova reproduz o estágio de remoção
exatamente, mas NÃO antepõe a codificação `htmlentities` (que
só importa para entradas não-UTF-8, irrelevante para `String`
do Rust). A função é determinística e idempotente dentro do
Suprnova; consumidores que precisam de hashing byte-idêntico
com um serviço PHP devem rodar seu próprio pré-estágio
`htmlentities` na entrada.

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## Builder `Limit`

O tipo de dado retornado por callbacks de limitador nomeado.
Construtores atalho espelham o `Limit::per*` do Laravel:

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 10 por 1 segundo (max_attempts, decay_seconds)
Limit::per_minute(60);              // 60 por minuto
Limit::per_minutes(5, 100);         // 100 por 5 minutos (decay primeiro, assinatura Laravel)
Limit::per_hour(1_000);             // 1000/hora
Limit::per_hours(6, 5_000);         // 5000 por 6 horas
Limit::per_day(10_000);             // 10000/dia
Limit::per_days(7, 50_000);         // 50000 por 7 dias
Limit::new(123, Duration::from_secs(45));  // ctor simples

// Cadeia de builder.
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - define a chave do bucket. Chave vazia é
  "global" (todo chamador compartilha um bucket).
- `.response(callback)` - gera uma resposta customizada quando
  o limite dispara; o default é um 429 simples "Too Many
  Attempts.".
- `.after(callback)` - só consome a tentativa quando
  `callback(response)` retorna true. Uso canônico: contar só
  logins com falha (`after(|r| r.status_code() >= 400)`).

`Limit::none()` retorna um `Unlimited` (um `GlobalLimit` com
`max_attempts = i64::MAX`). Retorná-lo a partir de um
limitador nomeado é o padrão Laravel para bypass.
`GlobalLimit` em si é um wrapper fino em torno de `Limit` com
uma chave vazia, mantido para paridade com
`Illuminate\Cache\RateLimiting\GlobalLimit`.

## `ThrottleRequestsMiddleware`

Wrapper HTTP em torno da facade apoiada em Cache. Espelha
`Illuminate\Routing\Middleware\ThrottleRequests`. Três
construtores:

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// Limitador nomeado - resolve no momento da solicitação
// via RateLimiter::limiter(name).
ThrottleRequestsMiddleware::by_name("api");

// max/decay/prefix inline - a forma literal do Laravel
// `throttle:60,1`.
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Lista explícita de Limits - o primeiro a disparar vence;
// o mais idiomático em Rust.
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

Conecte isso em um grupo de rotas:

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### Use `req.ip()` como chave, nunca o header

`X-Forwarded-For` é fornecido pelo chamador. Um limitador com
chave baseada no header bruto é derrotado ao enviar um valor
diferente em cada solicitação - o atacante escolhe seu próprio
bucket, então a cota é por solicitação em vez de por cliente.

`Request::ip()` é a leitura segura. Ele retorna
`X-Forwarded-For` / `X-Real-IP` **só quando o peer TCP está
listado em `APP_TRUSTED_PROXIES`**, e do contrário o endereço
do peer, então um header vindo de qualquer um que não seja seu
próprio proxy é ignorado.

O corolário importa tanto quanto: com essa variável não
definida - o padrão - `req.ip()` atrás de um proxy terminante
retorna o endereço *do proxy* em toda solicitação, e todo
limite por IP no app entra em colapso em um único bucket
compartilhado. `ThrottleRequestsMiddleware::with(20, 1,
"login")` então significa 20 tentativas por minuto entre todos
os usuários combinados, que qualquer chamador pode gastar para
trancar todo mundo de fora. Fazer deploy atrás de nginx,
Traefik, um ALB ou Cloudflare significa definir
[`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies).

### Cabeçalhos de resposta

Toda resposta envolvida carrega:

- `X-RateLimit-Limit` - o `max_attempts` configurado.
- `X-RateLimit-Remaining` - retries restantes para esse
  bucket.

Respostas 429 adicionalmente carregam:

- `Retry-After` - segundos até a janela reabrir.
- `X-RateLimit-Reset` - unix-seconds-since-epoch de quando o
  bucket reabre.

Isso corresponde exatamente à forma de
`ThrottleRequests::getHeaders` do Laravel.

### Limitador nomeado ausente

Quando uma rota está conectada a `by_name("X")` mas nenhum
limitador sob `X` foi registrado, o middleware retorna HTTP
503 com um corpo que nomeia o limitador ausente. O Laravel
lança `MissingRateLimiterException`; nós expomos isso como uma
resposta HTTP, para que um boot mal configurado não cause
panic na worker thread.

### Composição de driver vs. facade

Os dois middlewares podem coexistir em um único router.
Coloque em camadas o driver de janela deslizante para justiça
de baixo nível, e então o throttle apoiado em Cache para
limites nomeados por endpoint:

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## Configuração

A SPI do driver é configurada via variáveis de ambiente; a
facade apoiada em Cache é configurada onde quer que seu
backend do [`Cache`](cache.md) esteja configurado (memory ou
Redis).

| Variável | Usada por | Padrão |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | Bootstrap da SPI do driver | `memory` (recusado em produção - veja acima) |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | Sobrescrita de fail-closed de produção | não definido |
| `RATE_LIMIT_REDIS_URL` | Driver Redis | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Prefixo de chave do Redis | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Facade `RateLimiter` apoiada em Cache (veja [`Cache`](cache.md)) | vários |

## Migração a partir do Laravel

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` ou `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` middleware | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` middleware | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` headers | Mesmos headers, mesma forma |

### Por que Suprnova diverge

O Laravel distribui uma forma: `Illuminate\Cache\RateLimiter`
(contador de janela fixa apoiado em Cache) com
`Illuminate\Routing\Middleware\ThrottleRequests` como seu
wrapper HTTP. O Suprnova distribui tanto essa forma *quanto*
uma SPI de driver de janela deslizante nativa, porque duas
perguntas reais precisam de duas respostas reais.

Um contador apoiado em Cache é a resposta certa para "eu tenho
limitadores nomeados, callbacks de resposta, after-callbacks
para contar só logins com falha, e quero ser compatível na
fonte com migrações do Laravel." É a resposta errada para "eu
preciso de aplicação exata de janela deslizante de
um-slot-por-solicitação contra um Redis ZSET com avaliação
atômica de Lua e sem chave de timer separada." Essa segunda
pergunta é o que a maioria dos serviços Rust que batem nos
limites de concorrência do Tokio realmente têm, então
`RateLimiterDriver` + `RateLimitMiddleware` existem ao lado,
não atrás de uma feature flag.

A política de erro de backend também é uma adição do Suprnova.
O middleware do Laravel nunca expõe uma decisão de "o
limitador está quebrado" porque o ciclo de vida por
solicitação do PHP a esconde - a próxima solicitação recebe um
processo novo. Um worker Tokio de longa duração que perde o
Redis por dez segundos precisa decidir o que fazer com as
solicitações chegando durante essa janela;
`BackendErrorPolicy::FailOpen` (padrão) vs `FailClosed` é essa
decisão exposta explicitamente.

## Próximos passos

- [Middleware](middleware.md) - como middleware se compõe,
  roda, e faz short-circuit na cadeia de solicitação
- [Cache](cache.md) - o backend sobre o qual a facade
  `RateLimiter` no formato Laravel é construída
- [Configuração](configuration.md) - config tipada para os
  backends de cache e Redis
- [Fluxos de autenticação](auth-flows.md) -
  `LoginThrottleMiddleware` e o padrão de lockout por
  força-bruta se apoiam nesta superfície
- [Modelo de erros](error-model.md) - por que
  `Result<HttpResponse, HttpResponse>` deixa o middleware
  fazer short-circuit de forma limpa
