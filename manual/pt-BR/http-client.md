# Cliente HTTP

A facade `Http` é o lado de saída do HTTP - o equivalente Rust do
helper `Http::` do Laravel. Você recorre a ela quando seu handler,
job, ou tarefa agendada precisa chamar a API de outra pessoa: um
gateway de pagamento, um geocoder, um alvo de webhook, uma mensagem
do Slack. Construtor fluente, JSON de entrada e saída, retries com
jitter, fakes de teste determinísticos que registram o que você
enviou. A mesma superfície que você usava no Laravel, com isolamento
task-local para que testes paralelos não vejam os fakes uns dos
outros.

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

Essa é a forma: `Http::<verb>(url)` retorna um `RequestBuilder`; você
encadeia configuração nele; `.send().await` retorna um
`ClientResponse`. O cliente por trás é um `reqwest::Client`
compartilhado único, com TLS rustls, timeout padrão de 30s, e um user
agent `suprnova/<version>` - construído de forma lazy na primeira
chamada.

## Os verbos

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

Todo verbo retorna um `RequestBuilder`. A URL pode ser qualquer
`impl Into<String>` - um `&str`, uma `String`, ou um `Cow<str>`.
Nenhum helper de construção de URL vem com a facade; formate a URL
você mesmo ou recorra a um crate de query-string.

## Corpos

Três formas de anexar um corpo. Cada uma substitui qualquer corpo
definido anteriormente.

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` aceita qualquer coisa que implemente
`serde::Serialize`. O `Content-Type` na rede é definido
automaticamente para `application/json`. Se a serialização falhar
(ex.: um map com uma chave que não é string), o builder registra o
erro e `send()` o expõe em vez de silenciosamente enviar um corpo
`null`.

### Form

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` serializa o valor como
`application/x-www-form-urlencoded`. O valor precisa serializar para
um objeto JSON; as chaves se tornam campos de formulário. Mesma
semântica de erro de corpo que `.json` - uma falha de serialização é
exposta através de `send().await?`, nunca como um corpo vazio
silencioso.

### Bytes crus

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` recebe qualquer coisa `impl Into<Bytes>`. Você é
responsável pelo header `Content-Type` - `.body` não define um.

## Headers e autenticação

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` acrescenta; o framework não faz dedupe, então
duas chamadas com o mesmo nome enviam dois headers e o reqwest os
junta conforme a semântica HTTP. Dois atalhos para os esquemas de
autenticação comuns:

- `.bearer_token(token)` - define `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - define `Authorization: Basic <b64>`;
  `password` é `Option<&str>`, então `.basic_auth("api-key", None)`
  codifica a forma `api-key:` que alguns provedores querem

## Timeouts

O cliente compartilhado tem um timeout padrão de 30 segundos.
Sobrescreva por solicitação quando precisar:

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` sobrescreve tanto o timeout de conexão quanto o total
da solicitação, para essa chamada específica. Não há um knob separado
de `connect_timeout` no builder; o cliente reqwest subjacente usa um
único timeout combinado.

## Redirecionamentos

O cliente compartilhado segue redirecionamentos por padrão (até o
limite do reqwest de 10) - o comportamento certo quando você está
chamando um endpoint confiável que responde `http → https` ou te
entrega uma URL de CDN.

Quando a URL da solicitação é influenciada por input não confiável,
esse padrão se torna um vetor de server-side request forgery (SSRF):
um endpoint hostil pode responder com um `3xx` cujo `Location` aponta
para um serviço interno ou um endereço de cloud-metadata
(`http://169.254.169.254/…`), e um cliente que segue redirecionamentos
o seguiria. Desabilite o seguimento de redirecionamento para essas
solicitações com `.no_redirects()`:

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// O 3xx é retornado como está, em vez de ser seguido - inspecione-o e
// rejeite em vez de deixar o cliente seguir o header Location.
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` roteia a solicitação através de um cliente
separado, que não segue redirecionamentos; o cliente padrão - e toda
solicitação que não o chama - fica inalterado. Este é o análogo, no
cliente geral, do bloqueio de redirecionamento que o sender de
web-push já aplica a endpoints de push controlados por atacante.

## Retentativas

`Http` fornece retentativas com exponential-backoff e jitter completo - a receita
da AWS, a mesma que Laravel usa. Ambos os modos de retentativa tratam falhas de
transporte para todos os métodos HTTP. Eles diferem quanto a uma resposta 5xx
recebida poder repetir `POST` e `PATCH`.

### `.retry(max_attempts, base_backoff)` - retentativas de transporte para todos os métodos

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` inclui a primeira tentativa, então `retry(4, ...)` tenta de novo
até três vezes depois da tentativa inicial. O atraso antes da tentativa `n+1` é
uma duração aleatória uniforme em `[0, base_backoff * 2^(n-1)]`, limitada a 30
segundos. Jitter completo, não exponential-backoff-mais-sleep-fixo, para que
muitos workers tentando novamente durante a mesma interrupção não se sincronizem
em um thundering herd.

`.retry()` tenta novamente falhas de transporte para todos os métodos. Se uma
resposta chegar, tenta novamente um status 5xx a menos que o método seja `POST`
ou `PATCH`. Ela retorna respostas 4xx e 2xx/3xx como estão. Depois de esgotar as
retentativas, a última resposta ou erro de transporte é retornado ao chamador.

Essa distinção importa para escritas. Uma falha de transporte em `POST` ou `PATCH`
pode significar que o servidor confirmou a escrita, mas a resposta se perdeu,
contudo o contrato atual ainda tenta novamente essa falha. Uma resposta 5xx
recebida para esses métodos é retornada após uma tentativa, a menos que o
chamador use `.retry_non_idempotent(...)`.

### `.retry_non_idempotent(...)` - opt-in para POST/PATCH

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

Quando você forneceu uma chave de idempotência que o upstream respeita, ou de
outra forma tornou a solicitação segura para repetição, troque para
`.retry_non_idempotent(...)`. Ela preserva as retentativas de erro de transporte
para todos os métodos e, adicionalmente, permite retentativas de respostas 5xx
para `POST` e `PATCH`. Ainda retorna respostas 4xx e 2xx/3xx como estão.

### `Retry-After` é respeitado em 503

Para um `503 Service Unavailable`, o framework respeita um header
`Retry-After` - tanto na forma delta-seconds (`Retry-After: 30`) quanto na forma
HTTP-date (`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`). A espera real é a maior
entre o backoff com jitter e a dica do `Retry-After`, ainda limitada a 30 segundos.
Um servidor hostil ou malconfigurado que retorne `Retry-After: 86400` não manterá
sua task parada por um dia.

### `.retry_when(predicate)` - estreite ainda mais a política

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .retry_when(|ctx| ctx.method == "GET")
    .send()
    .await?;
```

`retry_when` registra um predicado consultado antes de cada retentativa que a
política acima faria de outro modo. Ele pode vetar uma retentativa que de outra
forma seria elegível, mas não pode criar uma. Em particular, ele não pode
transformar uma resposta 2xx, 3xx ou 4xx em uma retentativa, e não pode tornar uma
resposta 5xx recebida repetível para `POST` ou `PATCH` sem
`.retry_non_idempotent(...)`. Ele é consultado antes das retentativas de erros de
transporte para todos os métodos, incluindo `POST` e `PATCH` configurados com
`.retry()` simples. Sem uma política `.retry(...)` ou
`.retry_non_idempotent(...)`, um `retry_when` isolado não tem nada a vetar.

O predicado recebe `RetryContext { attempt, method, url, outcome }`, em que
`outcome` é `RetryOutcome::TransportError` (o envio falhou antes de uma resposta
chegar) ou `RetryOutcome::Status(n)` (uma resposta 5xx elegível).

## Lendo a resposta

`ClientResponse` expõe status, headers, e três métodos de leitura de
corpo. Cada método de corpo consome a resposta.

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// Escolha um - cada um consome a resposta.
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` é insensível a maiúsculas/minúsculas. `.json::<T>()`
retorna `Result<T, FrameworkError>` e usa `serde_json` para
decodificar. `.text()` exige UTF-8 e expõe um `FrameworkError` se o
corpo não for UTF-8 válido.

### Limite do corpo da resposta

Sem isso, um upstream lento ou hostil poderia fazer streaming de um
corpo ilimitado para a memória. Para proteger contra isso, toda
leitura de corpo bufferizada tem um limite - 25 MiB por padrão.
Sobrescreva globalmente no boot:

```rust
use suprnova::Http;

// Uma vez, em algum lugar do bootstrap.
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

Ou por solicitação, quando uma chamada legitimamente lida com um
payload maior:

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

Uma resposta que declara um `Content-Length` acima do limite é
rejeitada antes de qualquer corpo ser lido; o loop de streaming também
aplica o limite contra os bytes reais, para o caso de o
`Content-Length` estar ausente ou mentir.

## Válvula de escape - reqwest cru

O framework cobre os casos comuns. Quando você precisa de algo que
não expomos - corpos em streaming, uploads multipart, inspeção de
política de redirecionamento, upgrades de websocket - chame
`.into_inner()` para desembrulhar o `reqwest::Response` subjacente:

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` retorna `Err(FrameworkError::internal(...))` quando
chamado numa resposta fake - não há um `reqwest::Response` subjacente
nesse caso. O limite de corpo de resposta também deixa de valer uma
vez que você toma a resposta crua; a leitura passa a ser sua a partir
daí.

Para uploads multipart de saída hoje, desça direto para
`reqwest::Client` pela mesma rota de escape. Uma release futura pode
adicionar um builder `.multipart(...)` quando o padrão de demanda se
definir.

## Testando com `Http::fake`

Esta é a parte que você vai usar todo dia. `Http::fake` executa o
corpo do seu teste dentro de um escopo `tokio::task_local!` onde toda
chamada de saída é interceptada, capturada, e respondida com o que
quer que você tenha enfileirado.

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### Correspondendo respostas pré-fabricadas

`fake_response(method, url_substring, status, body)` enfileira uma
resposta pré-fabricada. A primeira solicitação de saída cujo método
corresponde (insensível a maiúsculas/minúsculas) e cuja URL contém
`url_substring` consome a entrada pré-fabricada e retorna essa
resposta. Use o método `"*"` para corresponder a qualquer método.

Solicitações subsequentes que correspondam caem para a próxima
entrada pré-fabricada da mesma forma, ou - se nenhuma corresponder -
retornam um `200 {}` vazio. Enfileire uma resposta pré-fabricada por
chamada esperada:

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// Dois GETs para /v1/customer recebem respostas distintas; um terceiro recebe 200 {}.
```

### Asserções

```rust
// Passa se pelo menos uma solicitação registrada corresponder.
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// Passa se nenhuma solicitação registrada corresponder.
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` expõe `method: String`, `url: String`,
`headers: Vec<(String, String)>`, e `body: Option<Vec<u8>>`. O
predicado executa contra toda solicitação registrada; falhas de
asserção imprimem a lista registrada com valores de header e corpos
redigidos (uma pequena allowlist de `Content-Type`, `Accept`, e
`User-Agent` é mostrada por completo; todo o resto é `<redacted>`).
Isso mantém bearer tokens e payloads de webhook fora dos logs de CI
mesmo quando uma asserção explode.

### Testes executam em paralelo com segurança

O estado do fake vive num `tokio::task_local!` - todo escopo de fake
é escopado à task que executa o teste, não ao processo. Dois testes
executando concorrentemente em tasks diferentes ganham, cada um, seu
próprio vec de solicitações registradas e sua própria fila de
respostas pré-fabricadas. Sem mutex compartilhado, sem ordem de
teste, sem `#[serial]`.

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // A solicitação do teste irmão para /b é invisível aqui.
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## A pegadinha da task spawnada

`tokio::task_local!` é escopado à task atual. Trabalho que passa por
`tokio::spawn` cai numa task nova e NÃO herda o fake - por padrão,
chamadas de saída a partir da future spawnada alcançam a rede real.
Dois helpers resolvem isso.

### `Http::fail_on_real_calls()` e `FailOnRealCallsGuard`

Ativa uma flag global ao processo que transforma toda chamada de
saída sem correspondência num `FrameworkError::internal(...)` em vez
de deixá-la alcançar a rede. Este é o análogo do Suprnova ao
`Http::preventStrayRequests()` do Laravel - ele captura exatamente o
bug que a pegadinha cria.

Use a guarda RAII para que a flag resete quando o teste termina,
mesmo em caso de panic:

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // Qualquer chamada HTTP de saída sem fake, de qualquer lugar dentro
    // deste teste - incluindo de uma task feita com `tokio::spawn` - dá
    // erro com uma mensagem nomeando a URL. Nenhuma I/O de rede
    // realmente acontece.
}
```

Guardas aninhadas se compõem corretamente: o `Drop` da guarda interna
restaura o estado ANTERIOR, não incondicionalmente "permitido". Então
um helper de teste interno que instala sua própria guarda dentro de
um escopo externo já guardado não desarma a guarda externa na saída.

A flag é global ao processo por design. O ponto é capturar uma future
feita com `tokio::spawn` que escapa silenciosamente de um escopo de
fake e acessa um terceiro real a partir do CI. Uma flag por task não
pegaria isso.

### `Http::spawn_with_fake_inheritance(future)`

Quando o código sob teste legitimamente faz spawn de uma task - um
worker de fila, um sincronizador em background, uma sub-task - e você
quer que suas chamadas de saída passem pelo fake do pai, troque
`tokio::spawn` por `Http::spawn_with_fake_inheritance`:

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // Executa numa task NOVA, mas o estado de fake do pai é
        // reinstalado no escopo task-local desta task. O send
        // é interceptado; a resposta é o 204 acima.
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // Solicitações registradas do filho aparecem aqui - o
    // Arc<Mutex<FakeState>> é compartilhado, não um snapshot.
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

Se nenhum escopo de fake está ativo quando você chama
`spawn_with_fake_inheritance`, é equivalente a `tokio::spawn` - o
filho executa sem nenhum contexto de fake. Então você pode usá-lo
incondicionalmente em código que às vezes é testado com `Http::fake`
e às vezes não.

### Precaução em duas camadas no setup de teste

Os dois se combinam. Um teste que quer ser explicitamente seguro os
pareia:

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // Se um erro de digitação na URL ou no método se afastar do fake,
        // a solicitação cai na guarda, que dá erro com uma mensagem
        // nomeando a URL - em vez de silenciosamente retornar um 200
        // vazio que esconde a incompatibilidade.
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

Sem a guarda, uma URL ou método que se afasta do fake cai
silenciosamente para um `200 {}` padrão, e seu teste passa mesmo com
o código de produção chamando um endpoint diferente. Com a guarda,
você falha de forma explícita na primeira incompatibilidade.

## Propagação de trace do OpenTelemetry

Quando o framework é construído com a feature `otel` e um propagador
W3C TraceContext está instalado, toda solicitação `Http::*` de saída
injeta `traceparent` (e `tracestate` quando não vazio) nos seus
headers - para que serviços downstream possam continuar o trace.
Nenhuma configuração no call site; o propagador lê
`opentelemetry::Context::current()` no momento do envio.

Sem um contexto OTel ativo, nenhum header é injetado e solicitações
de saída se parecem exatamente como antes. Veja
[Observabilidade](observability.md) para a configuração do
propagador.

## Por que Suprnova diverge

Três pequenas divergências da facade `Http::` do Laravel merecem ser
destacadas.

**Fakes task-local em vez de um mock store global ao processo.** O
`Http::fake()` do Laravel modifica um registry global ao processo;
testes se serializam sobre ele, ou você aceita que runners paralelos
podem competir (race). O `Http::fake` do Suprnova usa
`tokio::task_local!`, então dois testes em duas tasks veem, cada um,
o seu próprio fake - sem ordem de teste, sem mutex compartilhado. O
preço é que trabalho feito com `tokio::spawn` não herda o fake por
padrão, e é por isso que `Http::spawn_with_fake_inheritance` e
`FailOnRealCallsGuard` existem. Juntos, eles te dão a mesma garantia
de "não é possível acertar a produção por acidente" que o
`Http::preventStrayRequests()` dá no Laravel, com um escopo mais
estrito.

**As retentativas de 5xx recebidos recusam POST/PATCH por padrão.** O cliente
HTTP do Laravel tenta novamente qualquer método por padrão. `.retry(...)` do
Suprnova ainda tenta novamente falhas de transporte para `POST` e `PATCH`, mas
não tenta novamente uma resposta 5xx recebida para esses métodos. Use
`.retry_non_idempotent(...)` para optar por retentativas de respostas 5xx somente
depois de tornar a escrita segura para repetição, normalmente com uma chave de
idempotência que o upstream respeite.

**`retry_when` pode apenas estreitar, nunca ampliar.** O callback `$when` de
`retry()` do Laravel substitui inteiramente a decisão de "deve tentar novamente",
de modo que pode repetir status que o framework de outro modo não tocaria (um 404,
por exemplo). `retry_when` do Suprnova apenas veta uma retentativa que
`.retry(...)` ou `.retry_non_idempotent(...)` já decidiu fazer. Ele é consultado
para retentativas de erro de transporte em todos os métodos, incluindo `POST` e
`PATCH`, mas não pode transformar uma resposta 2xx, 3xx ou 4xx em uma retentativa
nem tornar uma resposta 5xx de `POST` ou `PATCH` elegível sob `.retry()` simples.

## Casos extremos e letras pequenas

- **`Http::*` é fechado para a v1.** Deliberadamente não expomos o
  `reqwest::Client` subjacente. Para expandir a superfície, adicione
  um método à facade em vez de recorrer ao `reqwest` diretamente -
  exceto pela válvula de escape documentada `into_inner()` sobre uma
  resposta real.
- **O cliente compartilhado é construído uma vez e vive para
  sempre.** Construído de forma lazy na primeira chamada a qualquer
  verbo `Http::*`, mantido num `OnceLock`. A pilha TLS rustls e o
  timeout padrão de 30s vêm embutidos.
- **Falhas de serialização de JSON/form falham de forma explícita.**
  Um builder `.json(&unserializable)` registra o erro e `send()` o
  retorna como `FrameworkError::internal(...)`. A solicitação nunca
  sai - não degradamos para um corpo `null`.
- **O teto de retry de 30s é rígido.** A matemática de backoff se
  limita a 30 segundos; a interpretação do `Retry-After` se limita a
  30 segundos; nenhum sleep de retry isolado mantém uma task parada
  por mais tempo que isso.
- **O limite global ao processo é de execução única.**
  `Http::set_max_response_bytes` é uma escrita num atômico global ao
  processo - defina-o uma vez no boot, depois sobrescreva por
  solicitação conforme necessário. Não existe uma chamada de "reset
  para o padrão".

## Próximos passos

- [Correio](mail.md) - email de saída, que usa padrões similares de
  fake / driver para testes
- [Notificações](notifications.md) - canais de notificação incluindo
  web push, todos compartilhando a mesma filosofia de fake de teste
- [Filas](queues.md) - jobs que fazem chamadas HTTP de saída, mais o
  padrão `spawn_with_fake_inheritance` para testar workers
- [Testes](testing.md) - `#[suprnova_test]`, `TestContainer`, e o
  resto da superfície de fakes
- [Observabilidade](observability.md) - configuração do propagador
  OTel que faz a injeção de `traceparent` funcionar
