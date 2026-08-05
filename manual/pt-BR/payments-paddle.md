# Pagamentos - adaptador Paddle

O adaptador Paddle (`suprnova-payments-paddle`) conecta a Paddle à
superfície de pagamentos genérica do Suprnova. Recorra a ele quando
você quiser um provedor de pagamento que também cuide de imposto
sobre vendas, VAT, GST, dunning, faturamento, e reembolsos em seu
nome - a Paddle é uma Merchant of Record (MoR), o que significa que
ela é a vendedora oficial diante dos seus clientes e absorve a
superfície de conformidade que um gateway de captura direta como a
Stripe deixa para você.

Essa escolha muda o modelo mental. Seu código de domínio não é
*dono* da assinatura - a Paddle é. Você abre um checkout, o cliente o
completa, e o webhook `SubscriptionCreated` diz a você que a
assinatura agora existe. Não é possível criar uma assinatura via
API, e não é possível trocar seu conjunto de preços depois do fato.
É possível cancelar, é possível ler o estado, é possível atualizar
metadados de cobrança. O resto é da Paddle.

Este capítulo assume que você já leu [Pagamentos](payments.md) para
a superfície genérica de cinco traits. Aqui cobrimos o que é
verdadeiro *só* para a Paddle.

## Quando escolher a Paddle

Escolha a Paddle quando um ou mais destes forem verdadeiros:

- Você vende produtos digitais globalmente e conformidade fiscal
  (VAT, GST, imposto sobre vendas dos EUA) é um custo real no seu
  roadmap.
- Você não quer gerenciar retries de pagamento falhado, emails de
  dunning, ou emissão de recibos você mesmo.
- Você quer uma única fatura de uma única vendedora oficial para a
  contabilidade.
- Seu modelo de negócio é centrado em assinaturas, e você aceita que
  o provedor conduz o ciclo de vida da assinatura.

Escolha [a Stripe](payments.md#stripe) em vez disso quando você
quiser controle direto sobre a captura de cobrança, você mesmo
cuidar do seu imposto, ou precisar de chamadas
`charge`/`capture`/`refund` do lado do servidor a partir do seu
próprio código.

## Configuração

Adicione o crate:

```bash
cargo add suprnova-payments-paddle
```

Defina as quatro variáveis de ambiente:

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| Variável | O que é | De onde vem |
|---|---|---|
| `PADDLE_API_KEY` | Chave de API do lado do servidor (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) | Dashboard da Paddle → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | Secret do destino de notificação (`pdl_ntfset_…`) | Dashboard da Paddle → Developer Tools → Notifications → seu endpoint |
| `PADDLE_CLIENT_TOKEN` | Token seguro para o navegador (`live_…` / `test_…`) | Dashboard da Paddle → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox` (padrão) ou `production` | Sua escolha |

Registre o provedor no bootstrap. As duas formas são válidas:

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // A partir do env (recomendado):
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // Ou construa diretamente:
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

A rota de ingestão de webhook é registrada pelo helper
`webhook_routes(db.clone())` do framework - veja
[Pagamentos](payments.md#webhook-handling). Tanto `from_env()`
quanto `new()` retornam `Result` porque o
`paddle_rust_sdk::Paddle::new` subjacente valida a forma da chave de
API e a URL do endpoint no momento da construção.

## O modelo mental de MoR

A forma que surpreende usuários da Stripe:

```
Stripe (gateway):
    seu app   ─────────►  Stripe  ──►  bandeira do cartão
       │                    ▲
       └────── webhook ─────┘
    você é dono do estado da assinatura no seu banco; a Stripe é a executora

Paddle (Merchant of Record):
    seu app   ─►  link checkout  ─►  cliente   ──►  Paddle  ──►  bandeira do cartão
                                                       │
       ◄──────────────────  webhook  ──────────────────┘
    a Paddle é dona do estado da assinatura; seu banco é o espelho
```

No código, a diferença aparece em três pontos:

1. **Não é possível criar uma assinatura via API.** Chame
   `Checkout::start_session` com um preço recorrente; o cliente
   completa o widget da Paddle; o webhook `SubscriptionCreated`
   hidrata seu espelho.
2. **Não é possível trocar o conjunto de preços de uma assinatura
   via API.** A Paddle reserva mudanças de plano para o dashboard
   dela mesma ou para fluxos de migração que ela mesma controla.
3. **Não é possível remover um cliente.** Arquivar via atualização
   é a solução de contorno suportada.

Suprnova expõe essas restrições como `PaymentError::NotSupported` em
vez de disfarçá-las - veja a
[matriz de capacidades](#matriz-de-capacidades) abaixo.

## Fluxo de checkout

`Checkout::start_session` é a única forma de iniciar um pagamento
com a Paddle. O frontend abre o `transaction_id` resultante com o
paddle.js usando o `client_token` que você definiu no bootstrap:

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. Cria o cliente na Paddle (ou reaproveita um existente).
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. Abre uma sessão de checkout. A Paddle despacha pontual vs
    //    assinatura com base no *tipo de preço*, não no campo
    //    SessionMode abaixo.
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // ignorado pela Paddle (veja a nota)
        customer_ref: cus.provider_customer_id,
        price_refs: vec!["pri_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some(format!("checkout_{user_id}")),
        metadata: None,
    }).await?;

    Ok(session)
}
```

O `SessionPayload::PaddleInline` retornado carrega tudo que o
frontend precisa:

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Veja [Pagamentos - integração Frontend](payments-frontend.md) para
o código de montagem do paddle.js em Svelte / React / Vue.

### A Paddle despacha com base no tipo de preço, não no `SessionMode`

Uma pegadinha genuinamente específica da Paddle: o campo
`SessionMode::OneOff` / `SessionMode::Subscription` em
`StartSessionRequest` é **ignorado pelo adaptador Paddle**. A API da
Paddle tem um único endpoint `transaction_create`, e o provedor
inspeciona os IDs de preço fornecidos para inferir o fluxo - um
preço recorrente inicia uma assinatura, um preço pontual inicia uma
única cobrança. Com a Stripe o campo conduz o fluxo; com a Paddle é
o *preço* que conduz. Configure seu catálogo Paddle com os tipos de
preço corretos antes de apontar o adaptador para eles.

## Assinaturas chegam via webhook

Porque a Paddle é dona do ciclo de vida da assinatura, seu código de
domínio só *fica sabendo* de uma assinatura quando a Paddle avisa. O
fluxo:

```
seu app                         Paddle                    cliente
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │   conclui o checkout     │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   webhook subscription.created                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 tabelas espelho hidratadas;      │                          │
 linha de payments_subscriptions  │                          │
 tem provider_subscription_id     │                          │
```

O handler `webhook_routes(db)` do framework faz a hidratação para
você: ele chama `WebhookHandler::extract_payload_ids` para encontrar
o `subscription_id`, chama `Subscription::get(id)` para ler o estado
canônico, e faz upsert de `payments_subscriptions` +
`payments_subscription_items` dentro de uma transação. No momento em
que o webhook retorna 200, seu espelho está consistente com a
Paddle.

Há uma janela breve entre o cliente completar o widget e o webhook
chegar em que `payments_subscriptions` não tem linha nenhuma para a
nova assinatura. Dois padrões cobrem isso:

- **Use a URL de redirecionamento para UX imediata.** O
  `success_return_url` dispara do lado do cliente logo que a Paddle
  confirma a transação, então você pode mostrar "Assinatura ativa"
  sem esperar pelo webhook do lado do servidor.
- **Poll-and-render.** Depois do redirecionamento, atualize a página
  após um atraso curto para que o controller Inertia possa ler o
  espelho já hidratado.

## Matriz de capacidades

Nem todo método de toda trait faz o que seu equivalente na Stripe
faz. A tabela abaixo é a verdade. `subscribe()` e `update()` com
`new_price_refs.is_some()` são os únicos métodos que *sempre*
falham; o resto funciona, com as ressalvas anotadas.

| Método da trait | Comportamento |
|---|---|
| `Checkout::start_session` | Funciona. Despacha pontual vs assinatura com base no tipo de preço, não no `SessionMode`. |
| `Subscription::subscribe` | Sempre `NotSupported`. Assinaturas nascem da conclusão do checkout + webhook. |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | Funciona. Conecta a `subscription_cancel` com o `EffectiveFrom::NextBillingPeriod` padrão. |
| `Subscription::update(new_price_refs: Some(...))` | `NotSupported` na v1. A Paddle reserva a substituição de conjunto de preços para seus próprios fluxos de migração. |
| `Subscription::update` (no-op) | Funciona. Rebusca o estado atual via `subscription_get`. |
| `Subscription::cancel` | Funciona, mas `at_period_end` é **ignorado** - sempre agenda para o próximo período de cobrança. Veja [abaixo](#o-cancelamento-é-sempre-agendado). |
| `Subscription::get` | Funciona. |
| `CustomerStore::create_customer` | Funciona. |
| `CustomerStore::update_customer` | Funciona. |
| `CustomerStore::get_customer` | Funciona. |
| `CustomerStore::delete_customer` | `NotSupported`. Use `update_customer` com status `archived` se precisar. |
| `Payment::*` | A trait não é implementada. `provider.as_payment()` retorna `None`. |
| `WebhookHandler::*` | Funciona. |

Os invariantes de `Payment` não ser implementada, `subscribe` /
`delete_customer` retornando `NotSupported`, e a rejeição de
assinatura de webhook são fixados por testes sempre-ativos em
`crates/suprnova-payments-paddle/tests/integration.rs`, então a
matriz acima não sofre drift silenciosamente.

### O cancelamento é sempre agendado

`Subscription::cancel(id, at_period_end)` aceita o bool por
compatibilidade de trait mas **sempre se comporta como cancelamento
agendado** - o enum `EffectiveFrom` da Paddle é privado no
`paddle_rust_sdk` 0.18, então cancelamento imediato não é viável na
v1. O usuário mantém acesso até o fim do período de cobrança atual,
e nesse momento a Paddle dispara `subscription.canceled` e o
espelho vira `status` para `Canceled`.

Se você quer um "cancelar agora" no nível de UX que revoga o acesso
ao app imediatamente enquanto deixa a Paddle encerrar a cobrança em
segundo plano, condicione o acesso à sua própria flag
`subscription.status != Canceled && subscription.cancel_at_period_end == false`
e atualize a UI logo depois que `cancel()` retornar - o próximo
webhook vai confirmar.

### Remoção de cliente é "arquivar via atualização"

`delete_customer` retorna `PaymentError::NotSupported` porque a API
pública da Paddle simplesmente não expõe nenhum endpoint de remoção.
Se você precisa suprimir um registro de cliente na Paddle, chame
`update_customer` com o status `archived`. O adaptador do framework
não encapsula isso diretamente - o campo de metadados é a válvula de
escape:

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

Confirme o caminho exato do campo contra a sua versão da API Paddle
antes de colocar isso em produção - o SDK atualmente não modela o
enum `status` diretamente.

## Verificação de assinatura de webhook

A Paddle assina todo webhook com HMAC. O header `Paddle-Signature`
se parece com `ts=1716000000,h1=abcdef…`. O adaptador delega a
verificação ao `Paddle::unmarshal` do SDK, que:

- Faz parse do header
- Recalcula o HMAC usando sua `PADDLE_WEBHOOK_KEY`
- Rejeita assinaturas cujo timestamp está fora de
  `MaximumVariance::default()` (5 segundos no momento desta escrita -
  replays mais antigos do que isso são descartados)

O handler `webhook_routes` do framework chama `verify` antes de
qualquer outra coisa; uma falha retorna `401 invalid-signature` sem
vazar corpo nenhum. Você não escreve nada desse código você mesmo,
mas vale saber que a verificação é HMAC + tolerância de timestamp,
não uma comparação de secret estático.

## Forma do payload de webhook

Os métodos `extract_payload_ids`, `extract_payment_snapshot`, e
`extract_customer_snapshot` do adaptador conhecem a forma do payload
da Paddle para que o framework possa hidratar as tabelas espelho.
Mapeamento rápido:

| `event_type` do webhook | `NeutralEventKind` | Efeito no espelho |
|---|---|---|
| `transaction.completed`, `transaction.paid` | `PaymentSucceeded` | Faz upsert de `payments_transactions` |
| `transaction.payment_failed` | `PaymentFailed` | Faz upsert de `payments_transactions` (falhado) |
| `transaction.billed` | `InvoicePaid` | Faz upsert de `payments_transactions` com `provider_subscription_id` vinculado |
| `adjustment.created`, `adjustment.updated` | `PaymentRefunded` | Faz upsert de `payments_transactions` (reembolsado) |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → upsert de `payments_subscriptions` + itens |
| `subscription.updated`, `.activated`, `.paused`, `.resumed`, `.trialing` | `SubscriptionUpdated` | Igual ao anterior |
| `subscription.canceled` | `SubscriptionCanceled` | Igual; define `canceled_at`, vira o status |
| `customer.created` | `CustomerCreated` | Só-atualização: atualiza `email`/`metadata` se a linha do espelho existir |
| `customer.updated` | `CustomerUpdated` | Igual |
| qualquer outra coisa | `None` (não mapeado) | Só linha de auditoria - sem mudança no espelho |

A Paddle coloca o objeto de entidade diretamente sob `data` (não
`data.object` como a Stripe). Valores chegam como **strings de
unidades menores** (`"1234"` = 12,34 na unidade principal), não
decimais - o adaptador faz parse tanto da forma string quanto
numérica para compatibilidade futura. A moeda chega como
`currency_code`, em minúsculas, e o snapshot a converte para
maiúsculas.

### Valores inclusivos de imposto

A Paddle relata valores de transação **inclusivos de imposto**. O
espelho `payments_transactions` do framework divide isso:

- `amount_total_minor` - o valor total que o cliente pagou (imposto
  incluído)
- `amount_tax_minor` - o componente de imposto

O líquido de imposto é `amount_total_minor - amount_tax_minor`. Isso
difere da Stripe (que relata exclusivo de imposto com
`amount_tax_minor = 0`). Código que soma receita entre os dois
provedores precisa ser ciente de imposto:

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## Criação de cliente

`CreateCustomerRequest` mapeia diretamente para o `customer_create`
da Paddle:

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // o id de usuário do seu app
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // não repassado à Paddle na v1
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

Guarde `cus.provider_customer_id` junto ao registro do seu usuário.
Toda chamada subsequente (iniciar um checkout, buscar uma
assinatura, etc.) usa o ID de cliente da Paddle, não o ID de usuário
do app. A tabela espelho `payments_customers` carrega as duas
colunas, então uma única busca por índice funciona em qualquer
direção.

`update_customer` e `get_customer` passam direto para os métodos
equivalentes do SDK. `update_customer` aceita atualizações de
`email` / `name` e retorna o `CustomerRef` atualizado.
`get_customer` busca um snapshot da Paddle (não do espelho) - use
isso quando você precisar de uma leitura atualizada depois de uma
mudança fora de banda no dashboard da Paddle.

## A forma intencional de `NotSupported`

Um leitor não familiarizado com a base de código pode supor que
`PaymentError::NotSupported` em `subscribe()` e `delete_customer()`
é um TODO adiado. Não é. As restrições fazem parte da superfície de
produto da Paddle, e Suprnova as codifica em vez de emular mutações
locais que o provedor nunca vai honrar.

Cada mensagem de erro `NotSupported` aponta para o fluxo de trabalho
suportado:

- `subscribe`: "use `Checkout::start_session` with `SessionMode::Subscription`
  and await the `SubscriptionCreated` webhook"
- `update` com `new_price_refs`: "Paddle price-set replacement on existing
  subscription not in v1"
- `delete_customer`: "use `UpdateCustomer` with `archived` status"

Trate esse erro explicitamente quando você estiver escrevendo código
de domínio agnóstico de provedor:

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* caminho Stripe */ }
    Err(PaymentError::NotSupported(_)) => {
        // Caminho Paddle - arquiva via atualização em vez disso
        provider.update_customer(UpdateCustomerRequest {
            provider_customer_id: cus_id,
            email: None,
            name: None,
            metadata: Some(serde_json::json!({ "status": "archived" })),
        }).await?;
    }
    Err(e) => return Err(e),
}
```

### Por que Suprnova diverge

Laravel Cashier é exclusivo da Stripe e modela assinaturas como
propriedade do app: `$user->newSubscription('default', 'pri_pro')->create()`
tem a forma de como se a aplicação estivesse iniciando a assinatura.
Com um gateway de captura direta isso é preciso. Com uma MoR, é uma
mentira - o provedor é o ator, não o seu app.

A superfície de pagamentos do Suprnova é neutra em relação ao
provedor, então ela não toma partido. A superfície de traits
(`subscribe`, `update`, `cancel`, `get`) é a forma genérica; cada
adaptador implementa o que seu provedor expõe e retorna
`NotSupported` onde o modelo de produto do provedor diverge. O
adaptador Stripe implementa `subscribe`. O adaptador Paddle não,
porque a Paddle não permite. Esconder a diferença atrás de um
"create" local falso faria o adaptador mentir para você - Suprnova
prefere o `NotSupported` tipado com uma mensagem de migração na
string de erro.

A mesma divergência se aplica a `Payment` (captura do lado do
servidor). A Stripe a implementa; a Paddle não, e
`provider.as_payment()` retorna `None`. Código que precisa de
charge/capture/refund precisa verificar `as_payment().is_some()` em
vez de chamar ciegamente - veja
[Pagamentos](payments.md#payment--optional-server-side-capture).

## Testando sua integração

O crate inclui testes de invariante sempre-ativos (sem necessidade
de acesso à rede) mais um teste de integração condicionado por
variável de ambiente contra a API sandbox da Paddle:

```bash
# Invariantes sempre-ativos (rejeição de assinatura, formas de NotSupported):
cargo test -p suprnova-payments-paddle

# Mais integração sandbox (exige PADDLE_API_KEY etc.):
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

Os testes de invariante são os que você deve espelhar no seu
próprio código se você construir abstrações específicas do
adaptador. Três formas de teste que vale copiar:

```rust
use suprnova::payments::*;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

#[test]
fn paddle_does_not_implement_payment_trait() {
    let p = PaddleProvider::new(
        "pdl_sdbx_apikey_test",
        "pdl_ntfset_test",
        "test_client",
        PaddleEnvironment::Sandbox,
    ).expect("provider construction");
    assert!(p.as_payment().is_none());
}

#[tokio::test]
async fn paddle_subscribe_returns_not_supported() {
    let p = /* ...como acima... */;
    let err = p.subscribe(SubscribeRequest {
        customer_ref: "ctm_test".into(),
        price_refs: vec!["pri_test".into()],
        trial_days: None,
        idempotency_key: None,
        metadata: None,
    }).await.unwrap_err();
    assert!(matches!(err, PaymentError::NotSupported(_)));
}

#[test]
fn webhook_verify_rejects_bad_signature() {
    let p = /* ...como acima... */;
    let mut headers = http::HeaderMap::new();
    headers.insert("paddle-signature", "ts=1234,h1=deadbeef".parse().unwrap());
    let ctx = WebhookContext {
        body: b"{}",
        headers: &headers,
        remote_addr: None,
    };
    assert!(matches!(p.verify(&ctx).unwrap_err(), PaymentError::WebhookSignature(_)));
}
```

Para testes locais de ponta a ponta sem acessar a Paddle de forma
alguma, o framework distribui o `MockPaymentProvider`. Como a
Paddle, o `as_payment()` do mock retorna `None` (sem captura do lado
do servidor), então código que decide com base em
`as_payment().is_some()` segue o mesmo caminho sob o mock que vai
seguir sob a Paddle. O `subscribe()` do mock retorna `Ok` (diferente
da Paddle), então testes que precisam verificar o branch de
`NotSupported` devem usar o `PaddleProvider` real. Vincule o mock nos
testes em vez do provedor real:

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ...exercite seu controller contra o mock...
}
```

## Checklist de produção

Antes de mudar para `PADDLE_ENVIRONMENT=production`:

- [ ] As quatro variáveis de ambiente estão definidas nos secrets de
  produção, não commitadas
- [ ] A URL do endpoint de webhook está registrada nas configurações
  de *Notifications* do dashboard da Paddle, e o secret de destino
  que você gerou lá corresponde a `PADDLE_WEBHOOK_KEY`
- [ ] O catálogo tem IDs de preço live (não sandbox), e os IDs que
  você referencia em `price_refs` existem no catálogo live
- [ ] Seus `success_return_url` e `cancel_return_url` apontam para
  endpoints HTTPS (a Paddle rejeita HTTP em produção)
- [ ] Você decidiu como seu app responde quando `subscribe()`,
  `delete_customer()`, ou `update(price_refs)` retornam
  `NotSupported` - ou trate isso no código ou documente que esses
  fluxos são só-MoR
- [ ] Você testou sob estresse a UX de cancelamento: o cancelamento
  é sempre agendado, então "você cancelou mas ainda tem acesso até
  DATA" é a mensagem que sua UI deveria mostrar
- [ ] Você testou sob estresse o webhook de chegada da assinatura:
  há uma janela em que o cliente já pagou mas o espelho ainda não
  tem linha
- [ ] Você está agregando a receita corretamente: valores da Paddle
  são inclusivos de imposto, valores da Stripe são exclusivos de
  imposto

## Próximos passos

- [Pagamentos](payments.md) - a superfície genérica de cinco traits
  e o contrato de hidratação do espelho do handler de webhook
- [Pagamentos - integração Frontend](payments-frontend.md) -
  checkout inline do paddle.js em Svelte / React / Vue
- [Pagamentos - guia do provedor](payments-provider-guide.md) -
  escreva seu próprio crate adaptador de ponta a ponta
- [Configuração](configuration.md) - registro de configuração
  tipada em que as variáveis de ambiente da Paddle se encaixam
- [Inicialização da aplicação](bootstrap.md) - onde
  `PaymentProviderRegistry::bind` de fato vive no seu app
