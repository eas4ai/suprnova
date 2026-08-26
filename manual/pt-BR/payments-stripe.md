# Pagamentos - adaptador Stripe

`suprnova-payments-stripe` é o adaptador de referência para a
superfície de pagamentos do Suprnova, neutra em relação ao provedor.
Ele implementa as cinco traits de pagamento (`Checkout`, `Payment`,
`Subscription`, `CustomerStore`, `WebhookHandler`) contra a API da
Stripe via `async-stripe` 1.0.0-rc.5. Recorra a este capítulo quando
precisar saber exatamente qual endpoint da Stripe um método chama,
como o formato de assinatura do webhook é verificado, como
PaymentIntents fluem através de `ChargeResult`, ou quais tipos de
evento mapeiam para o enum de evento neutro.

Para as formas das traits em si, a configuração de variáveis de
ambiente, e o padrão de bootstrap, leia [Pagamentos](payments.md)
primeiro. Este capítulo é o aprofundamento específico da Stripe.

## Gateway, não Merchant of Record

Por padrão, a Stripe é um **gateway de pagamento**: você recebe os
fundos diretamente na sua própria conta bancária, e você é
responsável pela coleta e recolhimento de impostos, faturamento,
dunning, e tratamento de chargeback. Em contraste com a Paddle
([Pagamentos - Paddle](payments-paddle.md)), onde a Paddle é a
Merchant of Record - ela coleta os fundos, declara o imposto, e paga
a você líquido de taxas.

A consequência prática para este capítulo: `StripeProvider`
implementa `Payment` (você pode autorizar, capturar, reembolsar, e
anular um cartão no servidor). `PaddleProvider` não. A divisão de
traits existe porque os dois fluxos são genuinamente diferentes - não
porque faltou tempo.

### Stripe Managed Payments (opt-in de Merchant of Record)

O programa **Managed Payments** da Stripe move a Stripe para o
assento de Merchant of Record em transações elegíveis - a Stripe se
torna a vendedora legal, calcula, coleta, declara, e recolhe o
imposto sobre vendas/VAT/GST, e assume as contestações. O programa
tem restrições de integração rígidas:

- **Só Checkout hospedado.** Sessões precisam correr na página
  hospedada da Stripe. Elements/fluxos personalizados são excluídos -
  por isso o caminho pontual hospedado do adaptador (abaixo) é a
  única forma `OneOff` que compõe com ele.
- **Preços predefinidos com códigos de imposto elegíveis.** Itens de
  linha precisam referenciar objetos `price_…` cujos produtos
  carreguem um código de imposto rotulado como elegível para Managed
  Payments no dashboard da Stripe. Valores ad-hoc são rejeitados.
- **Habilitação da conta.** A conta Stripe precisa estar habilitada
  no programa; sessões carregando a flag numa conta não habilitada
  falham.

Ative-o por provedor com `.with_managed_payments(true)` ou
`STRIPE_MANAGED_PAYMENTS=true` - o adaptador então envia
`managed_payments[enabled]=true` ao criar sessões pontuais
hospedadas. Quando desligado (o padrão) o campo é omitido por
completo.

### Por que Suprnova diverge

Laravel embute o Cashier como uma integração Stripe de primeira
classe na documentação central. É conveniente, mas exclusivo da
Stripe - e adicionar um segundo provedor significa fazer fork do
Cashier ou construir uma superfície paralela.

Suprnova mantém a Stripe a distância de braço. O adaptador Stripe é
um crate que se registra contra as mesmas cinco traits que qualquer
outro provedor implementa. Seu código de domínio nunca nomeia
`StripeProvider`; ele chama `provider.charge(...)` contra
`Arc<dyn PaymentProvider>` resolvido a partir do registry, e o
comportamento da Stripe está a uma troca do comportamento da Paddle.
Quando você mais tarde adicionar a Mollie, ou conectar um gateway
regional que ainda não existe, você implementa as mesmas cinco
traits e o resto do seu app não se move.

## Construção

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Produção: leia a partir do env.
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// Testes / config explícita:
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` é `Clone` (barato - o `stripe::Client` subjacente é
apoiado em `Arc`) e guarda estes valores:

| Campo | Origem | Uso |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | HTTP `Authorization: Bearer …` em toda chamada de API |
| `publishable_key` | `pk_live_…` / `pk_test_…` | Exposta dentro de `SessionPayload::StripeElements` para que o frontend monte o Stripe.js sem uma consulta de config separada |
| `webhook_signing_secret` | `whsec_…` | Verificação HMAC-SHA256 do header `Stripe-Signature` |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS` (`true`/`1`) ou `.with_managed_payments(bool)` | Envia `managed_payments[enabled]=true` na criação de sessão pontual hospedada (veja [Managed Payments](#stripe-managed-payments-opt-in-de-merchant-of-record)) |

`from_env()` retorna `Result<Self, String>` - a mensagem de erro
nomeia a variável obrigatória que falta (`STRIPE_MANAGED_PAYMENTS` é
opcional; ausente significa desligado). Não há caminho de panic no
boot.

## Sessões de checkout

`Checkout::start_session` escolhe sua superfície Stripe a partir da
requisição:

| Formato da requisição | Objeto Stripe | Variante de `SessionPayload` |
|---|---|---|
| `OneOff` + `price_refs` não-vazio | Sessão de Checkout hospedada, `mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + `price_refs` vazio + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | Sessão de Checkout hospedada, `mode=subscription` | `StripeCheckoutRedirect` |

O caminho pontual hospedado envia `allow_promotion_codes=true`
(clientes podem digitar códigos de promoção na página da Stripe -
combine com a trait `Promotions` abaixo) e, quando o provedor está
configurado para isso, a flag de Managed Payments. Coloque o
literal de template `{CHECKOUT_SESSION_ID}` da Stripe na sua
`success_return_url` - a Stripe substitui pelo id `cs_…` real no
redirecionamento, e sua página de retorno o passa para
`session_status`.

`Checkout::session_status` mapeia `GET /v1/checkout/sessions/{id}`
para o `CheckoutSessionState` neutro:

| `status` / `payment_status` da Stripe | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` ou `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid` (liquidação atrasada) | `Complete { paid: false, … }` |

`payment_ref` carrega o id do PaymentIntent da sessão (`pi_…`) para
que páginas de retorno e varreduras possam correlacionar a sessão com
operações de `Payment` e o espelho `payments_transactions`.
`amount_total` é o total liquidado com descontos do lado do provedor
e imposto de Managed Payments já embutidos.

## Códigos de promoção

`StripeProvider` implementa a trait opcional `Promotions`
(`provider.as_promotions()` retorna `Some`). `create_promotion_code`
mapeia para `POST /v1/promotion_codes`: ele emite um código a partir
de um cupom pré-criado (`coupon_ref`), restrito a um cliente
(`customer_ref`), com um limite opcional de expiração e resgate. As
restrições são impostas pela Stripe no momento do resgate - um
código emitido para o cliente A é rejeitado quando o cliente B o
digita, códigos expirados são rejeitados, e `max_redemptions:
Some(1)` torna o código de uso único. Veja a seção `Promotions` de
[Pagamentos](payments.md) para o padrão de campanha.

## O ciclo de vida do PaymentIntent

A Stripe representa uma única tentativa de cobrança como um
**PaymentIntent**. O intent passa por estados; a trait `Payment` do
Suprnova conduz as transições. Todo método `Payment` de
`StripeProvider` mapeia para um endpoint `/v1/payment_intents/...`:

| Método de `Payment` | Endpoint Stripe | O que faz |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | Cria + confirma em uma única chamada contra um método de pagamento salvo. `capture_method: "manual"` faz o intent ir para `requires_capture`, **não** `succeeded`. |
| `capture` | `POST /v1/payment_intents/{id}/capture` | Liquida um intent previamente autorizado. Status `requires_capture` → `succeeded`. |
| `refund` | `POST /v1/refunds` | Reverte total ou parcialmente um intent capturado. |
| `void` | `POST /v1/payment_intents/{id}/cancel` | Libera uma autorização antes da captura. Status `requires_capture` → `canceled`. |
| `status` | `GET /v1/payment_intents/{id}` | Recupera o status atual (retorna `PaymentStatus`). |

### Autorizar primeiro, capturar depois

`StripeProvider::charge` **não** liquida os fundos imediatamente.
Ele envia `capture_method=manual` + `confirm=true`, o que autoriza o
cartão e reserva os fundos, e então espera uma chamada `capture`
explícita. Esse é o fluxo canônico em duas etapas:

```rust
use suprnova::payments::{
    PaymentProviderRegistry, ChargeRequest, ChargeResult,
    Money, Currency, PaymentStatus,
};

let provider = PaymentProviderRegistry::get("stripe").unwrap();
let payment = provider.as_payment()
    .expect("Stripe implements Payment");

let result = payment.charge(ChargeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    payment_method_ref: "pm_card_visa".into(),
    amount: Money::from_minor_units(2999, Currency::USD),
    description: Some("Pro plan, manual capture".into()),
    idempotency_key: Some("order-12345".into()),  // veja "Idempotência" abaixo
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // Autorizado - capture quando o pedido for enviado.
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // Step-up 3DS necessário - veja "3DS e SCA" abaixo.
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

Se você quer captura **imediata** - o pontual comum de e-commerce -
use `Checkout::start_session` com `SessionMode::OneOff` em vez disso.
Esse caminho cria um PaymentIntent com `automatic_payment_methods`
ativado e entrega o client secret ao frontend para que o navegador do
cliente confirme o intent no local. `Payment::charge` é para fluxos
conduzidos pelo servidor onde você já tem o método de pagamento
salvo do cliente e quer controle explícito de autorizar-depois-capturar
(típico de marketplaces, SaaS com cumprimento adiado, ou comércio com
envio dividido).

### Mapeamento de status

Status da Stripe se dobram no enum `PaymentStatus` do Suprnova:

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending` (autorizado, esperando captura) |
| `RequiresAction` | `Pending` (retornado como `RequiresClientAction` de `charge`) |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _novo status da Stripe (o enum é `#[non_exhaustive]`)_ | `Failed` |

O fallback `non_exhaustive` é intencional. A Stripe ocasionalmente
adiciona estados (ex.: ao introduzir novos tipos de método de
pagamento). Expô-los como `Failed` é o padrão conservador - seu app
trata a cobrança como ainda-não-confirmada até você atualizar o
adaptador.

### 3DS e SCA

A Autenticação Forte do Cliente europeia, as regras do RBI indiano,
e vários outros reguladores exigem que o titular do cartão se
autentique em um contexto de navegador separado. A Stripe expõe isso
como `requires_action` com um bloco `next_action`.

`StripeProvider::charge` traduz isso em uma de duas variantes de
`ChargeResult`:

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - guarde isto
    action_kind: "stripe_3ds", // tag específica da Stripe
    client_secret,             // entregue ao Stripe.js
    publishable_key,           // entregue ao Stripe.js
}
```

Quando o `next_action` do intent contém uma URL de redirecionamento
(alguns fluxos de autenticação são por redirecionamento de URL em
vez de modal no local), o resultado é reescrito como:

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // redirecione o navegador para aqui
    return_to: None,
}
```

Seu controller entrega o payload de `RequiresClientAction` para a
página Inertia; o frontend chama
`stripe.confirmCardPayment(client_secret, ...)` e o cliente completa
o 3DS. Quando a confirmação é bem-sucedida, a Stripe dispara
`payment_intent.succeeded` e a rota de webhook escreve a linha do
espelho. Veja
[Pagamentos - integração Frontend](payments-frontend.md) para os
trechos em Svelte / React / Vue.

### Void vs refund

`void` libera uma autorização **antes** da captura; `refund` reverte
um pagamento já capturado. Chamar `void` em um intent já capturado
vai falhar - a Stripe rejeita com uma mensagem contendo
`"already succeeded"` ou `"You cannot cancel"`, e o adaptador expõe
isso como `PaymentError::Validation` para que seu handler possa
distinguir um erro recuperável do usuário (use `refund` em vez
disso) de uma verdadeira interrupção do provedor. Qualquer outra
falha é `PaymentError::Provider`.

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* autorização liberada */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // Já capturado - chame refund em vez disso.
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // reembolso total
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund() não repassa isto - veja "Idempotência"
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Clientes

`StripeProvider` implementa `CustomerStore` contra `/v1/customers`.
O adaptador mapeia um `Customer` retornado para o `CustomerRef`
neutro, preservando o email e o `user_id` da sua aplicação:

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // o id de usuário do seu app
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// Persista isso junto à sua linha de User para que
// cobranças, assinaturas, e webhooks subsequentes resolvam de volta.
```

`update_customer`, `get_customer`, e `delete_customer` acessam
`POST /v1/customers/{id}`, `GET /v1/customers/{id}`, e
`DELETE /v1/customers/{id}` respectivamente. O delete da Stripe
retorna um envelope `DeletedCustomer` que o adaptador descarta - só
o sucesso/falha da chamada é propagado.

## Assinaturas

`StripeProvider::subscribe` faz POST para `/v1/subscriptions` com a
referência do cliente, um array `items[]`, e um `trial_period_days`
opcional:

```rust
use suprnova::payments::{SubscribeRequest, SubscriptionStatus};

let sub = provider.subscribe(SubscribeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    price_refs: vec!["price_pro_monthly".into()],
    trial_days: Some(14),
    idempotency_key: None,
    metadata: None,
}).await?;

assert!(matches!(
    sub.status,
    SubscriptionStatus::Trialing | SubscriptionStatus::Active
));

println!("Period ends at {}", sub.current_period_end);
for item in &sub.items {
    println!(
        "  {} × {} @ {:?}",
        item.quantity, item.provider_price_id, item.unit_amount,
    );
}
```

### Limites de período

A Stripe moveu os timestamps `current_period_start` /
`current_period_end` do Subscription pai para cada
`SubscriptionItem` na versão de API `2023-08-16`. Assinaturas
multi-item podem, em teoria, ter períodos de item divergentes, mas
na prática todo item de uma única assinatura compartilha o ciclo de
cobrança do pai. O adaptador toma o período do **primeiro item**
como o período do pai no `SubscriptionResult` retornado. Se você
genuinamente precisa de períodos por item, leia-os de
`sub.items[n]` - eles são preservados no snapshot.

### Cancelar no fim do período vs imediatamente

```rust
// Soft cancel - mantém acesso até current_period_end:
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// Cancelamento imediato - Stripe DELETE /v1/subscriptions/{id}:
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

Os dois caminhos acessam endpoints diferentes da Stripe. O soft
cancel é `POST /v1/subscriptions/{id}` com
`cancel_at_period_end=true` - a assinatura permanece ativa até o fim
do período de cobrança, e então a Stripe a finaliza. O cancelamento
imediato é `DELETE /v1/subscriptions/{id}` com `prorate=false` e
`invoice_now=false`.

### `update()` é intencionalmente limitado

`UpdateSubscriptionRequest` tem dois campos sobre os quais o
adaptador age: `cancel_at_period_end` e `new_price_refs`. O primeiro
é suportado; o segundo retorna `PaymentError::NotSupported`:

```rust
provider.update(UpdateSubscriptionRequest {
    provider_subscription_id: "sub_1234".into(),
    new_price_refs: Some(vec!["price_team_yearly".into()]),
    cancel_at_period_end: None,
    idempotency_key: None,
}).await
// → Err(PaymentError::NotSupported(
//      "Stripe price-set replacement on existing subscription not in v1. \
//       Cancel the subscription and create a new one with the new price set."
//   ))
```

Este é um dos poucos lugares em que `NotSupported` é a resposta
honesta, e não um adiamento. A substituição de conjunto de preços
na Stripe exige apagar e recriar itens da assinatura - a forma varia
por provedor (pro rata, ancoragem de ciclo de cobrança,
comportamento de trial retido) e reduzir isso a uma única API neutra
esconderia mais do que ajudaria. O caminho recomendado é cancelar a
assinatura existente e chamar `subscribe` de novo com o novo
conjunto de preços, aplicando sua própria política de pro rata se
precisar de uma.

## Webhooks

A Stripe envia webhooks assinados com HMAC-SHA256 no formato:

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` faz parse do header, recalcula HMAC-SHA256
sobre `"{timestamp}.{raw_body}"` usando o secret de assinatura do
webhook, e faz uma comparação em **tempo constante** contra todo
valor `v1=` no header. Múltiplos valores `v1=` existem durante a
rotação do secret de assinatura - a Stripe sobrepõe os secrets
antigo e novo por uma janela para que você possa reassinar e fazer
deploy sem um corte abrupto (flag-day).

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

O adaptador aceita a requisição se **qualquer** valor `v1=`
corresponder. Um header sem `t=` ou sem nenhum valor `v1=` é
rejeitado como `PaymentError::WebhookSignature`. Bytes não-ASCII em
qualquer lugar do header também são rejeitados - a Stripe nunca os
envia, e tratá-los como inválidos é mais seguro do que substituir por
um caractere de substituição.

Você nunca chama `verify` diretamente. O `webhook_routes(db.clone())`
do framework registra `POST /webhooks/payments/{provider}` e invoca
`verify` + `parse_event` + os extratores de payload do adaptador para
toda requisição que chega lá. Veja [Idempotência](idempotency.md)
para o comportamento de auditoria ciente de retry - incluindo a
regra de que eventos previamente falhados tentam a hidratação de
novo quando o provedor refaz a tentativa.

### Mapeamento de evento → neutro

Tipos de evento da Stripe mapeiam para o `NeutralEventKind` do
Suprnova via a função `stripe_event_to_neutral`. A tabela de
mapeamento:

| Tipo de evento Stripe | `NeutralEventKind` |
|---|---|
| `payment_intent.succeeded` | `PaymentSucceeded` |
| `payment_intent.payment_failed` | `PaymentFailed` |
| `charge.refunded` | `PaymentRefunded` |
| `charge.dispute.created` | `PaymentDisputed` |
| `customer.subscription.created` | `SubscriptionCreated` |
| `customer.subscription.updated` | `SubscriptionUpdated` |
| `customer.subscription.deleted` | `SubscriptionCanceled` |
| `customer.subscription.paused` | `SubscriptionUpdated` |
| `customer.subscription.resumed` | `SubscriptionUpdated` |
| `customer.subscription.trial_will_end` | `SubscriptionUpdated` |
| `invoice.payment_succeeded` / `invoice.paid` | `InvoicePaid` |
| `invoice.payment_failed` | `InvoiceFailed` |
| `customer.created` | `CustomerCreated` |
| `customer.updated` | `CustomerUpdated` |
| _qualquer outra coisa_ | `None` |

Eventos que mapeiam para `None` (sinais de fraude do Radar,
repasses, transferências de saldo, eventos do ciclo de vida da
contestação além de `created`) ainda são persistidos na tabela de
auditoria `payments_webhook_events` - eles simplesmente não
alimentam as tabelas espelho. Se você precisa deles, leia diretamente
de `event.raw_payload` em um handler personalizado.

O mapeamento também é reexportado na raiz do crate para que você
possa usá-lo fora da rota de webhook:

```rust
use suprnova_payments_stripe::stripe_event_to_neutral;
use suprnova::payments::NeutralEventKind;

assert_eq!(
    stripe_event_to_neutral("payment_intent.succeeded"),
    Some(NeutralEventKind::PaymentSucceeded),
);
assert_eq!(
    stripe_event_to_neutral("radar.early_fraud_warning.created"),
    None,
);
```

### Extração de payload

Depois que `verify` e `parse_event` são bem-sucedidos, o framework
chama `extract_payload_ids`, `extract_payment_snapshot`, e
`extract_customer_snapshot` para extrair os campos que alimentam as
tabelas espelho (veja [Eloquent](eloquent.md) para o padrão
subjacente de ler-do-seu-próprio-banco). A Stripe é estruturalmente
consistente: todo webhook coloca a entidade relevante em
`data.object`, com `id` como sua chave primária.

Os extratores tratam quatro famílias de evento:

- **Eventos de assinatura** - extraem `data.object.id` (o id da
  assinatura) e `data.object.customer`.
- **Eventos de cliente** - extraem `data.object.id` (o id do
  cliente).
- **Eventos de PaymentIntent / Charge** - extraem `data.object.id`,
  `data.object.amount`, `data.object.currency`,
  `data.object.customer`, e (só para `payment_intent.succeeded`)
  `data.object.created` como `paid_at`.
- **Eventos de fatura** - extraem `data.object.id`, o ponteiro de
  cliente, `data.object.subscription` (só cobranças recorrentes),
  `amount_paid` (recuando para `amount_due`), `tax`, `currency`, e
  `data.object.status_transitions.paid_at`.

Qualquer outra coisa retorna `None` dos extratores de snapshot; a
linha de auditoria ainda é gravada.

## Tabelas espelho

Seis tabelas sustentam a superfície de pagamentos no banco de dados
da sua aplicação. Aplique a migração do framework junto com as suas:

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... suas migrações ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

As tabelas criadas são `payments_customers`,
`payments_payment_methods`, `payments_subscriptions`,
`payments_subscription_items`, `payments_transactions`, e
`payments_webhook_events`. A rota de webhook as hidrata dentro de
uma única transação de banco por evento - estado parcial nunca é
observável, e a linha de auditoria carrega `process_error` através
dos retries para que falhas permaneçam visíveis aos operadores.

## Idempotência

Idempotência de saída nas chamadas de API da Stripe e idempotência
de entrada nas entregas de webhook são duas histórias separadas.
Leia-as como tal.

### Saída: cobertura por método

A Stripe suporta idempotência de requisição via o header HTTP
`Idempotency-Key` - a mesma chave com o mesmo corpo retorna o mesmo
objeto de resposta por uma janela de replay de 24 horas; um corpo
divergente retorna um erro. O adaptador Stripe do Suprnova **não**
repassa uniformemente o campo `idempotency_key` do DTO para esse
header hoje. O comportamento real no momento desta escrita:

| Método | Campo do DTO | O que o adaptador faz |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | Repassado no corpo do POST como `idempotency_key=...` (não no header HTTP). A API da Stripe **não** lê chaves de idempotência no corpo do formulário, então é melhor tratar isso como não efetivo até o adaptador migrar para o caminho do header de requisição. |
| `Payment::refund` | `RefundRequest::idempotency_key` | Silenciosamente descartado - o campo não é repassado. |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | Silenciosamente descartado. |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | Silenciosamente descartado. |

Se você depende de semântica no-máximo-uma-vez para retries de
cobrança/reembolso contra a Stripe hoje, condicione o retry no seu
próprio call site (uma chave de domínio determinística persistida no
seu banco, com um índice único impedindo a segunda inserção) até o
adaptador conectar o header. Os campos do DTO são aceitos pela API
mas atualmente não são honrados até a rede - defina-os como `None`
em testes e em código de produção para que a lacuna fique explícita,
e não assuma que a Stripe está deduplicando seus retries.

Esta é uma lacuna conhecida no adaptador v1 e uma candidata a
correção no próximo release; a forma da superfície permanece a
mesma quando a fiação chegar.

### Entrada: deduplicação de webhook

A idempotência de webhook é tratada pelo framework do lado da
ingestão e está totalmente conectada. Todo evento chega em
`payments_webhook_events` com um índice ÚNICO em
`(provider, provider_event_id)`. Entregas duplicadas de um evento já
processado retornam 200 para a Stripe imediatamente sem refazer a
hidratação; duplicatas de um evento **falhado** anteriormente
tentam a hidratação de novo, então o retry do provedor é seu
mecanismo de recuperação. Veja [Idempotência](idempotency.md) para
o contrato completo de auditoria + retry.

## Testes

O adaptador é apoiado em hyper e usa rustls na frente. Testes que
constroem um `StripeProvider` precisam de um provedor de
criptografia registrado; instalamos o `ring` exatamente uma vez em
`#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use suprnova_payments_stripe::StripeProvider;
    use std::sync::OnceLock;

    fn install_crypto_provider() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parses_subscription_webhook_ids() {
        let p = provider();
        let event = /* construct WebhookEvent with raw_payload */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

Para testes de integração que acessam o sandbox real da Stripe,
defina `STRIPE_SECRET_KEY` e afins no seu env de teste. Para testes
unitários dos seus próprios controllers, prefira `MockPaymentProvider`
do framework - ele implementa as cinco traits com retornos
previsíveis e zero rede.

## Próximos passos

- [Pagamentos](payments.md) - a superfície de traits, o registry, o
  padrão de bootstrap, e o `SessionPayload` com tag de fluxo.
- [Pagamentos - Paddle](payments-paddle.md) - a contraparte
  Merchant-of-Record; as mesmas cinco traits, divisão de
  responsabilidade diferente.
- [Pagamentos - guia do provedor](payments-provider-guide.md) - como
  escrever um adaptador para um gateway que o Suprnova não
  distribui.
- [Pagamentos - integração Frontend](payments-frontend.md) - despacho
  em Svelte / React / Vue sobre `SessionPayload.flow`, incluindo o
  loop de confirm-card-payment do Stripe.js.
- [Idempotência](idempotency.md) - o contrato de auditoria + retry
  que torna o tratamento de webhook seguro sob entrega
  at-least-once.
- [Eloquent](eloquent.md) - consulte as tabelas espelho junto com
  seus próprios models; tudo é apenas uma entidade SeaORM.
