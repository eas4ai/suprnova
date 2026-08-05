# Idempotência

Quando um cliente tenta de novo um POST, você quer que a
segunda chamada seja segura. A rede é não confiável e
clientes tentam de novo - mas `POST /charges` nunca deveria
cobrar o cartão duas vezes, e `POST /orders` nunca deveria
produzir dois pedidos para um clique. Chaves de idempotência
são o contrato que diz "se você vir essa mesma chave de novo,
me dê a resposta original; não refaça o trabalho."

O `Idempotency` do Suprnova é uma facade fina sobre
`Cache::lock` que te dá três garantias escalonadas: só
dedupe, dedupe com retry em caso de falha, e replay de
resultado no estilo Stripe. Todas as três mantêm o lease do
lock vivo por todo o tempo que o corpo rodar, então um corpo
lento nunca pode deixar o lock expirar e um duplicado passar
despercebido.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // Roda exatamente uma vez por chave dentro da janela de 24 horas.
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* primeira chamada - id é o novo pedido */ {},
    Idempotent::FreshUnfenced(id) => {
        // O pedido foi feito, mas o lease do lock foi perdido no
        // meio do caminho, então outro chamador pode ter feito um
        // também. Reconcilie ou alerte - veja "Quando a
        // exclusividade é perdida" abaixo.
    },
    Idempotent::Duplicate => /* mesma chave já usada */ {},
}
```

## As três primitivas

| Método | Corpo roda | Duplicado vê | Falha libera o lock? | Use quando |
|---|---|---|---|---|
| `Idempotency::once` | exatamente uma vez por janela | marcador `Duplicate` | não | efeitos colaterais NUNCA podem se repetir (mail enviado, cobrança tentada) |
| `Idempotency::commit_on_success` | uma vez por sucesso por janela | marcador `Duplicate` | sim | falhas transitórias devem ser retentáveis, mas um sucesso se mantém |
| `Idempotency::remember` | uma vez por sucesso por janela | o valor de retorno original | sim | duplicados precisam receber o payload original, não um marcador |

As três vivem sob `suprnova::idempotency` e são reexportadas
da raiz do crate como `Idempotency`, `Idempotent`, e `Replay`.
Elas compartilham a mesma hash-de-chave, renovação de lease, e
semântica de lock - só a política de sucesso/falha difere.

### `Idempotency::once` - no-máximo-uma-vez

O contrato mais estrito. O primeiro chamador na janela de TTL
roda o corpo e recebe `Fresh(value)`. Todo chamador
subsequente dentro da janela recebe `Duplicate` e o corpo NÃO
roda de novo - mesmo que o corpo do primeiro chamador tenha
retornado `Err`. O TTL É a janela de dedupe.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// Envia um email de boas-vindas exatamente uma vez por
// registro, independente de quantas vezes o callback de
// registro tenta de novo.
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

Busque `once` quando o efeito colateral é do tipo "eu tentei;
mesmo que eu tenha falhado depois do efeito colateral, não
tente de novo" - enviar um email, postar para uma API externa
que não honra suas próprias chaves de idempotência, escrever
uma entrada de log de auditoria cuja escrita-dupla corromperia
analytics downstream.

### `Idempotency::commit_on_success` - ao-menos-uma-vez no sucesso, retry na falha

Como `once`, mas se o corpo retornar `Err`, o lock de dedupe é
liberado para que o próximo chamador dentro da janela de TTL
possa tentar de novo. Um corpo bem-sucedido mantém o lock pelo
resto da janela.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // Posta uma mensagem para um serviço upstream. Erros de
        // rede são transitórios - a próxima tentativa deveria
        // reentrar, não ser avisada de "já feito" quando nada
        // realmente aconteceu.
        social_media_client.post(&post).await
    },
)
.await?;
```

Use `commit_on_success` quando o corpo tem modos de falha
retentáveis (erros de rede transitórios, limites de taxa
upstream, credenciais expiradas que um refresh resolveria) e
você quer ao-menos-uma-vez no sucesso, mas quer que o lock
seja liberado em uma falha para que um retry possa reentrar.

### `Idempotency::remember` - replay de resultado no estilo Stripe

O contrato para o qual o header HTTP `Idempotency-Key` foi
inventado. O primeiro chamador roda o corpo, armazena o valor
de sucesso, e recebe `Replay::Fresh`. Um chamador posterior
dentro da janela recebe `Replay::Replayed(<original value>)` -
o valor de retorno registrado, não um marcador. Um chamador
concorrente que chega *enquanto* o primeiro ainda está rodando
recebe `Replay::InProgress`.

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // Extrai o header para uma String própria antes de consumir `req` para o corpo.
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // Mesma resposta para o cliente, mas vale uma métrica: a
            // exclusividade não foi mantida durante todo o corpo.
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Note que `Fresh` e `Replayed` são tratados de forma idêntica
pela resposta voltada ao cliente - o ponto todo do `remember`
é que o segundo chamador não consegue saber se foi ele quem
rodou o corpo ou se recebeu o resultado registrado.

`InProgress` é o caso que vale a pena pensar sobre: um
duplicado chegou enquanto o corpo do primeiro chamador ainda
estava executando, então não há resultado registrado para
devolver ainda. `409 Conflict` com um header `Retry-After: 1`
é a resposta canônica - o cliente recua brevemente, então
tenta de novo, e a segunda tentativa ou compete com a original
pelo short-circuit do `Cache::get`, ou acerta `Replayed`.

## Material de chave

Os três métodos aceitam um `&str` arbitrário para a chave.
Antes de tocar o backend de cache, a chave recebe hash SHA-256
em um digest hex de 64 caracteres. Isso te dá três coisas:

1. **Tamanho de chave de backend limitado.** Um cliente que
   faz POST de um header `Idempotency-Key` de 10 KB ainda
   produz uma chave de cache de 64 bytes.
2. **Identificadores brutos não vazam para ferramentas de
   cache.** Se a chave contém um endereço de email, um id de
   sessão, ou um id de usuário interno, esses não aparecem em
   `redis-cli KEYS idem:*`.
3. **Nenhuma colisão de classe de caractere.** O que quer que
   o backend de cache interprete de forma especial
   (dois-pontos, caracteres de glob, bytes de controle) já se
   foi - o hash é só hex.

O hash é sobre a chave fornecida pelo usuário, não sobre o
prefixo da chave de cache - `Idempotency::once("k", …)` e
`Idempotency::once("k", …)` de dois call sites diferentes no
mesmo processo colidem de propósito. Dê namespace às suas
chaves você mesmo se não quiser isso:

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## Renovação de lease - o problema do corpo lento

Uma combinação naive de lock + TTL tem um bug de janela: se o
corpo roda por mais tempo que o TTL, o lock expira enquanto o
corpo ainda está rodando, e um segundo chamador pode adquirir
um lock novo e rodar o corpo de novo concorrentemente. O
contrato de dedupe quebra exatamente para as operações lentas
o suficiente para precisar dele.

O Suprnova resolve isso spawnando uma task em background que
renova o lock a um terço do TTL (com piso de 50 ms) durante
toda a duração do corpo. Um `tokio::select!` com ordenação
`biased` garante que o branch do corpo é o único que resolve a
future.

Um *erro* de refresh não é tratado como um lease perdido. Isso
significa que o backend não pôde ser consultado, não que outra
pessoa tomou o lock, então a renovação tenta de novo no próximo
intervalo e só desiste depois de várias falhas consecutivas.
Abandonar no primeiro blip garantia que o lease expiraria
mesmo quando o backend se recuperasse milissegundos depois.

### Quando a exclusividade é perdida

A renovação ainda pode genuinamente falhar: o token para de
corresponder, porque o lock expirou e outra pessoa o
reivindicou. Nesse momento, dois chamadores podem estar
rodando o mesmo corpo.

O corpo **não** é cancelado. No momento em que um lease é
perdido, ele já pode ter cobrado um cartão ou enviado uma
mensagem, e cancelar deixaria isso pela metade sem nada
registrando. O corpo roda até a conclusão e a perda é
reportada:

| Resultado | Significa |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | o corpo rodou, a exclusividade se manteve do início ao fim |
| `FreshUnfenced(v)` | o corpo rodou e produziu `v`, mas outro chamador pode ter rodado concorrentemente |

`FreshUnfenced` é uma variante separada, em vez de uma flag em
`Fresh`, especificamente para que um `match` exaustivo não
consiga ignorá-la por acidente. O que fazer com ela é você que
decide - reconciliar, alertar, compensar - mas tratá-la como
`Fresh` descarta o único sinal que você tem de que a garantia
não se manteve.

Perder um lease exige que o backend esteja inalcançável por
vários intervalos de refresh, ou uma pausa stop-the-world mais
longa que o TTL. É raro. Não é impossível, e antes era
invisível.

O resultado prático: escolha um TTL baseado na sua janela de
dedupe (`how long should a duplicate request be deduped?`),
não na duração de pior caso do seu corpo. Um corpo de 30
minutos com um TTL de 1 minuto está ótimo - o lock será
renovado cerca de noventa vezes durante a execução do corpo.

Um teste que exercita isso: um TTL de 200 ms com um corpo que
bloqueia por 500 ms, e um segundo chamador chegando aos 400
ms. Sem renovação, o segundo chamador reexecutaria o corpo.
Com renovação, ele vê `Duplicate`. O lock se mantém.

## Backend compartilhado

Dedupe entre processos exige um cache entre processos. O
backend em memória mantém locks em um `HashMap` por processo,
então duas instâncias de `cargo run` na mesma máquina não vão
ver as chaves de idempotência uma da outra. Implantações de
produção onde qualquer uma dessas coisas importa - múltiplos
processos de app, escalonamento horizontal, deploys blue/green
com janelas de tráfego se sobrepondo - precisam definir
`CACHE_DRIVER=redis` e fornecer um `REDIS_URL` alcançável.

O bootstrap falha de forma fechada: se `CACHE_DRIVER=redis` e o
Redis está inalcançável, o app se recusa a iniciar em vez de
fazer downgrade silenciosamente para memory por processo. Veja
[cache.md](cache.md) para o contrato completo do backend de
cache.

## Tratamento de erros

O `FrameworkError` do corpo se propaga através de
`Idempotency` sem alteração. Uma falha de aquisição de lock (o
Redis cai no meio da solicitação, o backend retorna um erro)
se propaga como um `FrameworkError` a partir da camada de
cache - não há fallback silencioso. O tipo de erro é o
`FrameworkError` padrão do framework, então handlers podem
fazer `?` dele até o conversor de erro do seu controller:

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Uma falha de release no caminho de `Err` de
`commit_on_success` ou `remember` é **logada, nunca
retornada** - o erro do corpo é o único erro que o chamador vê
nesse caminho. Um release que falha significa que o lock vai
se manter até o TTL expirar; um retry dentro da janela vai ver
`Duplicate` ou `InProgress` até então. Logs incluem a chave
hasheada (nunca o material de chave bruto) para que operadores
possam correlacionar sem vazar PII.

## Cancelamento

Se o chamador descarta a future de `Idempotency::remember`
antes de o corpo terminar, o corpo é cancelado como qualquer
outro branch de `tokio::select!` - o lock **não** é liberado,
e um duplicado chegando antes que o TTL expire vê `InProgress`
(depois, após o TTL, `Fresh` de novo). Esse é o default
seguro: um corpo pela metade cujos efeitos você não conhece
não deveria ser presumido seguro para tentar de novo. Envolva
corpos que mantêm efeitos colaterais não gerenciados em
`tokio::spawn` e junte o handle se você precisar tornar o
corpo não-cancelável.

## Integração com a fila

A camada de fila usa `Idempotency::commit_on_success`
internamente para implementar `Queue::push_unique`. Se você
quer que um job seja enfileirado no máximo uma vez por janela
de `Job::unique_for()` por `Job::unique_id(&self)`, você não
precisa chamar `Idempotency::*` você mesmo:

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // Ganhamos a corrida; o job está na fila.
} else {
    // Outro chamador já enfileirou isso; trate como sucesso.
}
```

Veja [queues.md](queues.md) para o contrato completo de
unicidade de job.

## Ingress de webhook de pagamento

O handler de webhook de pagamentos NÃO usa `Idempotency::*`. O
ingress de webhook tem um requisito mais estrito - todo evento
precisa ser auditável, mesmo na primeira entrega, então a
linha de auditoria é a fonte da verdade e a chave de de-dupe é
a constraint `UNIQUE(provider, provider_event_id)` do banco de
dados. `Idempotency::remember` armazenaria o payload da
resposta no cache; o handler de webhook armazena o *envelope
completo do evento mais o resultado do processamento* em
`payments_webhook_events`, o que significa que um operador
pode reproduzir ou reprocessar eventos offline lendo a tabela.

Os dois padrões são complementares. Use `Idempotency::*` para
chaves guiadas pelo cliente com dedupe escopado por TTL; use
uma tabela de auditoria indexada por `UNIQUE` para ingress de
webhook guiado pelo provedor que precisa de auditabilidade
além do TTL do cache. Veja [payments.md](payments.md) para o
contrato de webhook.

### Por que Suprnova diverge

O `Cache::lock` do Laravel é uma primitiva; o contrato de
idempotência no estilo Stripe (registrar o resultado,
reproduzi-lo, distinguir em-progresso de duplicado) é deixado
como uma receita userland. Todo projeto Laravel que precisa
disso acaba escrevendo a mesma dança de lock-e-cache,
geralmente com um destes três bugs:

1. **Nenhuma renovação de lease.** Um corpo que sobrevive ao
   TTL reexecuta concorrentemente em um chamador duplicado. O
   lock estava lá; ele só expirou no momento errado.
2. **Release no caminho de sucesso.** Liberar o lock quando o
   corpo tem sucesso abre uma janela entre `body() -> Ok` e o
   próximo chamador adquirindo um lock novo - exatamente a
   janela que o dedupe deveria fechar.
3. **Chaves brutas no backend de cache.** Headers
   `Idempotency-Key` fornecidos pelo cliente vão direto para
   chaves do Redis, vazando PII em ferramentas de operador e
   produzindo tamanhos de chave sem limite.

O Suprnova distribui a receita como uma primitiva de primeira
classe, para que todo chamador receba a mesma renovação de
lease, a mesma semântica de release fail-closed, a mesma
segurança de chave hasheada. Os três métodos (`once`,
`commit_on_success`, `remember`) nomeiam as três políticas
entre as quais você de fato precisa escolher - escolha a que
combina com o modelo de falha do seu corpo e siga adiante.

## Testes

`Idempotency` resolve seu `CacheStore` através do contêiner,
então testes que vinculam um `InMemoryCache` recebem um cache
novo e isolado por teste:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

O próprio `framework/tests/idempotency.rs` do framework cobre
a superfície do contrato: supressão de duplicado, expiração de
TTL, política de release erro-vs-sucesso, renovação de lease
através de durações de corpo que sobrevivem ao TTL, a corrida
do `InProgress`, e o caso em que o `release_lock` do próprio
cache dá erro. Leia esses testes se você quiser ver o
comportamento exato com o qual pode contar.

## Pegadinhas

- **`Idempotency::once` consome a janela em caso de erro.** Um
  primeiro chamador que falha ainda mantém o lock até o TTL
  expirar. Use `commit_on_success` se você quer retries dentro
  da janela.
- **`Idempotency::remember` armazena `T` no backend de
  cache.** A chave tem hash aplicado, mas o *payload* é
  serializado com serde e escrito no backend. Não coloque
  segredos em um valor reproduzido que não deve aparecer no
  seu backend de cache.
- **Dois processos precisam de um cache compartilhado.**
  Dedupe em memória é por processo. Correção entre processos
  exige `CACHE_DRIVER=redis` (ou outro backend entre
  processos).
- **TTLs abaixo de 150 ms não são testados com lease.** O piso
  de renovação é 50 ms, então um TTL de 100 ms renova a cada
  50 ms mais ou menos - ok para o contrato, mas os testes de
  lease do framework rodam com `ttl >= 1s`. Use janelas de
  dedupe realistas; uma janela de idempotência medida em
  milissegundos geralmente significa que o contrato não é bem
  a ferramenta certa.
- **O cancelamento do corpo não libera o lock.** Um corpo
  cancelado deixa o lock se mantendo até o TTL expirar. Essa é
  a escolha fail-closed; organize seus timeouts para que o
  cancelamento corresponda ao que um chamador duplicado
  deveria ver.

## Próximos passos

- [cache.md](cache.md) - a primitiva de lock subjacente e a
  seleção de `CACHE_DRIVER`.
- [queues.md](queues.md) - como `Queue::push_unique` se
  constrói sobre `Idempotency::commit_on_success` para dedupe
  em nível de job.
- [payments.md](payments.md) - ingress de webhook que usa
  idempotência de linha-de-banco-de-dados em vez de dedupe por
  chave de cache, e quando buscar qual.
- [rate-limiting.md](rate-limiting.md) - middleware adjacente
  que usa o mesmo backend `Cache` para aplicação de janela
  deslizante.
- [middleware.md](middleware.md) - como fatorar a extração de
  chave de idempotência em um middleware reutilizável sobre
  suas rotas POST/PUT.
