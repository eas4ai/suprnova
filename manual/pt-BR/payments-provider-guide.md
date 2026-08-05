# Escrevendo um adaptador de provedor de pagamento

Este guia percorre a construção de um crate adaptador de terceiros -
`suprnova-payments-mollie` - que se conecta à superfície de
pagamentos do Suprnova, neutra em relação ao provedor. No final você
terá um crate que se registra, passa pelo fluxo discriminador, e
pode ser colocado em qualquer app Suprnova com um único `cargo add`.

A mesma estrutura se aplica a qualquer provedor: Square, Braintree,
Adyen, ou qualquer outro com uma API HTTP.

### Por que Suprnova diverge

Laravel embute o Cashier como uma integração Stripe de primeira
classe. É excelente para o caminho da Stripe, mas codifica o
vocabulário de um provedor no framework - adicionar um segundo
provedor significa fazer fork do Cashier ou construir uma superfície
paralela ao lado dele.

Suprnova mantém todo provedor no mesmo contrato de cinco traits:
`Checkout`, `Subscription`, `CustomerStore`, `WebhookHandler`, e a
opcional `Payment` para provedores com captura no servidor. Código
de domínio só guarda `Arc<dyn PaymentProvider>` a partir do registry.
Trocar a Stripe pela Paddle (ou pelo adaptador Mollie que você está
prestes a escrever) é uma mudança de bootstrap, não uma mudança de
código. Os adaptadores de referência em
`crates/suprnova-payments-stripe/` e
`crates/suprnova-payments-paddle/` provam que o contrato de traits
se sustenta para dois modelos comerciais bem diferentes - gateway de
captura direta e Merchant of Record - e seu adaptador se encaixa na
mesma forma.

## 1. Crie o Crate Membro do Workspace

A partir da raiz do repositório:

```bash
cargo new --lib crates/suprnova-payments-mollie
```

Adicione-o ao seu `Cargo.toml` raiz:

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # adicione esta linha
]
```

(Os adaptadores de referência - `crates/suprnova-payments-stripe` e
`crates/suprnova-payments-paddle` - vivem neste mesmo diretório
`crates/` e são bons modelos para ler junto com este guia.)

**`crates/suprnova-payments-mollie/Cargo.toml`:**

```toml
[package]
name = "suprnova-payments-mollie"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Mollie payment adapter for Suprnova"

[dependencies]
suprnova = { path = "../../framework" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
inventory = "0.3"
tracing = "0.1"
tokio = { version = "1", features = ["macros", "rt"] }
# O seu SDK da Mollie:
mollie-rs = "0.1"
hmac = "0.12"   # para verificação HMAC de webhook
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. Organize os Arquivos-Fonte

Espelhe a estrutura usada pelos adaptadores já distribuídos:

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # struct MollieProvider, impl de PaymentProvider, from_env
├── checkout.rs     # impl de Checkout
├── customer.rs     # impl de CustomerStore
├── subscription.rs # impl de Subscription
├── webhook.rs      # impl de WebhookHandler
├── event_map.rs    # string de evento do provedor → NeutralEventKind
└── payment.rs      # impl de Payment (se a Mollie suportar server-capture)
```

## 3. `lib.rs` - a Struct do Provedor

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{Payment, PaymentProvider};

mod checkout;
mod customer;
mod event_map;
mod payment;
mod subscription;
mod webhook;

pub use event_map::mollie_event_to_neutral;

/// Adaptador Mollie para a superfície de pagamentos do Suprnova,
/// neutra em relação ao provedor.
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Chave de API da Mollie (`test_…` / `live_…`).
    api_key: String,
    /// Secret de assinatura de webhook - usado na verificação HMAC.
    webhook_secret: String,
    /// Cliente HTTP - compartilhado entre requisições.
    client: reqwest::Client,
}

impl MollieProvider {
    pub fn new(api_key: impl Into<String>, webhook_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            webhook_secret: webhook_secret.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Constrói a partir de variáveis de ambiente.
    ///
    /// Lê:
    /// - `MOLLIE_API_KEY`
    /// - `MOLLIE_WEBHOOK_SECRET`
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("MOLLIE_API_KEY")
            .map_err(|_| "MOLLIE_API_KEY not set".to_string())?;
        let webhook_secret = std::env::var("MOLLIE_WEBHOOK_SECRET")
            .map_err(|_| "MOLLIE_WEBHOOK_SECRET not set".to_string())?;
        Ok(Self::new(api_key, webhook_secret))
    }
}

impl PaymentProvider for MollieProvider {
    fn name(&self) -> &'static str {
        "mollie"
    }

    // Só sobrescreva `as_payment()` se você também implementar `Payment` (server-capture).
    // A impl padrão em `PaymentProvider` retorna `None` - omita este override
    // por completo se a Mollie for só-checkout / estilo MoR.
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` é a trait guarda-chuva - a cláusula de supertrait é
`Checkout + Subscription + CustomerStore + WebhookHandler`, então o
compilador vai se recusar a vincular seu provedor até as quatro
estarem implementadas. A quinta trait, `Payment`, é **opcional** -
só provedores que expõem captura do lado do servidor a implementam,
e `as_payment()` relata o resultado ao framework. O `as_payment()`
padrão retorna `None`, então omita o override por completo se seu
provedor não faz captura no servidor.

## 4. Implemente as Quatro Traits Obrigatórias

### `checkout.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    Checkout, PaymentError, PaymentResult, SessionMode, SessionPayload, StartSessionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Checkout for MollieProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        // Chame a API da Mollie para criar um pagamento ou pedido.
        // Mapeie a resposta para uma das variantes de SessionPayload.
        // A Mollie usa páginas de checkout hospedadas, então Redirect é o encaixe natural.
        let checkout_url = self.create_mollie_payment(&req).await
            .map_err(|e| PaymentError::Internal(format!("Mollie checkout error: {e}")))?;

        Ok(SessionPayload::Redirect {
            url: checkout_url,
            provider_session_id: "mollie_session_id_here".into(),
        })
    }
}

impl MollieProvider {
    async fn create_mollie_payment(&self, req: &StartSessionRequest) -> Result<String, mollie_rs::Error> {
        // Conecte a chamada do SDK da Mollie aqui.
        // Retorne a URL de checkout hospedado.
        todo!("Mollie payment creation")
    }
}
```

### `customer.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::MollieProvider;

#[async_trait]
impl CustomerStore for MollieProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        // POST /v2/customers na Mollie
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        // PATCH /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        // GET /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        // DELETE /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

### `subscription.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    PaymentError, PaymentResult, SubscribeRequest, Subscription, SubscriptionResult,
    UpdateSubscriptionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Subscription for MollieProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        // POST /v2/customers/{id}/subscriptions
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        // PATCH /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // Define a data de cancelamento para o fim do período
        } else {
            // DELETE /v2/customers/{id}/subscriptions/{sub_id}
        }
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        // GET /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

Se seu provedor não suporta um método, retorne
`PaymentError::NotSupported`:

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - captura do lado do servidor (opcional)

Só implemente isto se seu provedor suporta cobranças diretas no
servidor contra um método de pagamento salvo. Remova o override de
`as_payment()` em `lib.rs` se você pular isto.

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    ChargeRequest, ChargeResult, Payment, PaymentError, PaymentResult, PaymentStatus,
    RefundRequest, RefundResult,
};

use crate::MollieProvider;

#[async_trait]
impl Payment for MollieProvider {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult> {
        // POST /v2/payments/{id}/refunds
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

## 5. Mapeie os Eventos do Provedor para `NeutralEventKind`

**`event_map.rs`:**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// Mapeia uma string de tipo de evento de webhook da Mollie para a
/// taxonomia neutra do framework.
/// Retorna `None` para eventos específicos do provedor que não têm
/// equivalente neutro.
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Pagamentos da Mollie
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Assinaturas da Mollie
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Pedidos/faturas da Mollie
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // Eventos de cliente
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // Específico do provedor - cai para raw_payload
        _                       => None,
    }
}
```

Cubra no mínimo os eventos listados acima. Para qualquer evento fora
da taxonomia neutra, retorne `None` - ele ainda é persistido em
`payments_webhook_events` sob `provider_event_type` + `raw_payload`
para que código de domínio possa lê-lo.

## 6. Implemente a Verificação de Assinatura de Webhook

**`webhook.rs`:**

A Mollie assina payloads de webhook usando HMAC-SHA256. Sempre
compare assinaturas em tempo constante para prevenir ataques de
temporização.

```rust,ignore
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    NeutralEventKind, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{MollieProvider, event_map::mollie_event_to_neutral};

type HmacSha256 = Hmac<Sha256>;

#[async_trait]
impl WebhookHandler for MollieProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // Leia o header de assinatura que a Mollie envia.
        // Nome exato do header e esquema de assinatura - confira a documentação da Mollie para sua versão.
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // Calcula o HMAC-SHA256 esperado sobre o corpo bruto.
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // Decodifica a assinatura recebida, codificada em hex.
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // Comparação em tempo constante.
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // A Mollie envia JSON - faça o parse.
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // A Mollie usa tipos de recurso em vez de strings de tipo de evento em algumas formas de webhook.
        // Adapte para o que a sua versão do SDK enviar.
        let event_type = raw["resource"].as_str()
            .unwrap_or("unknown")
            .to_string();

        let neutral = mollie_event_to_neutral(&event_type);

        Ok(WebhookEvent {
            provider: "mollie".into(),
            provider_event_id: event_id,
            provider_event_type: event_type,
            neutral,
            raw_payload: raw,
        })
    }
}
```

Pontos-chave:

- `PaymentError::WebhookSignature(String)` é a única variante para
  qualquer falha de assinatura - header ausente, codificação
  malformada, incompatibilidade. A rota de webhook do framework
  trata todo `WebhookSignature(_)` como 401.
- Use `PaymentError::Validation(String)` para corpos que não fazem
  parse. A rota de webhook retorna 400 em qualquer falha de parse.
- O handler `webhook_routes` do framework chama `verify` antes de
  `parse_event`, e então hidrata dentro de uma transação de banco.
  Falhas de hidratação retornam 503 para que o provedor refaça a
  tentativa.
- Nunca registre em log o secret bruto ou a assinatura recebida.

### Hidratação da tabela espelho: `extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

Depois que `parse_event` retorna um `WebhookEvent`, a rota de
webhook do framework hidrata as tabelas espelho. Três métodos
opcionais da trait conduzem isso - todos têm implementações padrão
seguras e no-op, então um adaptador pode ser distribuído sem eles e
ainda passar pela camada de auditoria:

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` é a ponte entre o evento já processado e a lógica de
espelho do framework. Implemente-o para que o framework encontre a
entidade certa:

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

Para cada valor de `neutral`, preencha os IDs que o payload do
provedor expõe. Eventos de assinatura devem definir
`subscription_id` para que o framework possa chamar
`Subscription::get(id)` e atualizar o espelho a partir do estado
canônico. Eventos de cliente definem `customer_id`. Eventos de
pagamento / fatura definem `transaction_id`, mais `subscription_id`
quando for uma cobrança recorrente.

`PaymentSnapshot` é construído diretamente do payload do webhook -
não há callback `Payment::get`. Implemente-o para os neutros de
pagamento / fatura:

```rust,ignore
pub struct PaymentSnapshot {
    pub provider_transaction_id: String,
    pub provider_customer_id: String,
    pub provider_subscription_id: Option<String>,
    pub amount_total_minor: i64,
    pub amount_tax_minor: i64,
    pub currency: String,
    pub status: String,             // "succeeded" | "failed" | "refunded" | "disputed"
    pub paid_at: Option<DateTime<Utc>>,
    pub provider_metadata: Value,   // tipicamente o objeto de entidade do payload
}
```

A implementação de referência da Stripe lê
`data.object.{id,amount,currency,customer}` para eventos de
`PaymentIntent`/`Charge` e
`data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}`
para eventos de `Invoice`. A da Paddle lê
`data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}`.
Espelhe as convenções que combinam com a forma do payload do seu
provedor - o framework não se importa como você extrai, só que o
snapshot esteja correto.

Se você retornar `None` de `extract_payment_snapshot`, a linha de
auditoria ainda é escrita mas `payments_transactions` não é tocada.
Esse é o retorno correto para eventos de assinatura / cliente, ou
para qualquer evento de pagamento em que o payload não carregue
informação suficiente para popular uma linha.

`CustomerSnapshot` mantém a sincronia do espelho de cliente conduzida
pelo provedor (sem caminhos JSON fixos no framework):

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

O framework só faz `email = Set(snapshot.email)` quando o snapshot
fornece um; `provider_metadata` é sempre substituído pela visão do
provedor sobre o cliente (`updated_at` também é atualizado
independentemente). Linhas do espelho de cliente são sempre apenas
**atualizadas** - nunca inseridas - porque `user_id` é `NOT NULL` e
o app é dono do vínculo usuário ↔ cliente via
`CustomerStore::create_customer`.

### Semântica de falha

Se `extract_payload_ids` retorna `None` para `subscription_id` em um
evento de assinatura (ou para `customer_id` em um evento de
cliente), o framework trata isso como um erro `Validation`: a
transação de hidratação sofre rollback, `process_error` da linha de
auditoria é definido, e a resposta HTTP é **503 hydration-failed**
para que o provedor refaça a tentativa. Sucesso silencioso em um
payload malformado deixaria o espelho obsoleto sem visibilidade para
o operador - retries do provedor são o mecanismo de recuperação.

Esse contrato significa que o extrator de um adaptador precisa
preencher os IDs relevantes honestamente. Retornar `None` é
reservado para eventos que seu provedor não consegue traduzir de
forma alguma (ex.: um evento de pagamento sem ID de cobrança no
payload), não para "não me dei ao trabalho de fazer parse deste".

## 7. Registre no Boot do App

Dois mecanismos estão disponíveis - escolha um:

### Registro em tempo de execução (recomendado para apps com config via variável de ambiente)

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### Registro em tempo de compilação via `inventory`

Para crates adaptadores que querem registro sem configuração - útil
ao distribuir uma biblioteca que consumidores simplesmente instalam
com `cargo add` sem nenhuma fiação em tempo de boot:

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// Em lib.rs, num inicializador estático:
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` roda antes de `main`. O closure de fábrica é
chamado uma vez quando o registry é acessado pela primeira vez.

## 8. Passe pelo Teste Discriminador

Todo crate adaptador deve incluir um teste de integração que prove
que o contrato de traits está correto de ponta a ponta. Essa é a
prova de solidez - se esse teste passa, o provedor se conecta a
qualquer app Suprnova sem surpresas.

```rust,ignore
// tests/discriminator.rs (inside crates/suprnova-payments-mollie/)

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// Exige que MOLLIE_API_KEY e MOLLIE_WEBHOOK_SECRET estejam definidas.
/// Rode com: cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. Criar cliente
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. Iniciar sessão de checkout
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("discriminator_test_checkout".into()),
        metadata: None,
    }).await.expect("start_session failed");
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    // 3. Assinar diretamente (se seu provedor suportar; a Mollie pode exigir checkout)
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. Ler de volta
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. Cancelar no fim do período
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. Cancelar imediatamente
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. Verificar o invariante de as_payment()
    let p: &dyn PaymentProvider = &provider;
    // Se você implementou Payment: assert!(p.as_payment().is_some())
    // Se você NÃO implementou Payment: assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

Condicione testes de integração reais a `#[ignore]` para que
`cargo test` passe na CI sem credenciais. Rode-os explicitamente com
`-- --ignored` contra uma conta sandbox.

## 9. Referência das Variantes de `PaymentError`

O enum completo vive em `framework/src/payments/error.rs`. Escolha a
variante que combina com o que de fato deu errado:

| Variante | Quando usar |
|---|---|
| `Provider(String)` | A API do provedor retornou um erro que você não precisa traduzir mais |
| `Validation(String)` | Campos da requisição são inválidos, ou um corpo de webhook não faz parse |
| `NotSupported(String)` | O método não se aplica a este provedor (ex.: o `subscribe` da Paddle) |
| `Declined { reason, decline_code }` | Cartão recusado - repasse `decline_code` quando o provedor fornecer um |
| `Authentication(String)` | O provedor rejeitou sua chave de API ou credenciais |
| `NotFound(String)` | O ID de cliente, assinatura, ou transação não existe |
| `WebhookSignature(String)` | Qualquer falha de assinatura - header ausente, codificação malformada, ou incompatibilidade |
| `InvalidPhoneNumber(String)` | Validação E.164 falhou em fluxos de mobile money |
| `InvalidCountryCode(String)` | Validação ISO-3166-1 alpha-2 falhou |
| `Internal(String)` | Erro inesperado do SDK, falha de rede, falha de inicialização de HMAC, ou qualquer outro problema do lado do framework |

A rota de webhook mapeia isso para códigos de status:
`WebhookSignature(_)` → 401, `Validation(_)` de `parse_event` → 400,
qualquer outra coisa da hidratação → 503 (para que o provedor refaça
a tentativa).

Uma vez que seu adaptador compila e o teste discriminador passa:

- Adicione seu crate ao `Cargo.toml` do seu app com
  `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie`.
- Registre no bootstrap como mostrado no passo 7.
- Monte `webhook_routes(db.clone())` uma vez no boot do app - o
  mesmo handler despacha para todo provedor registrado pelo nome,
  então uma única montagem serve a Stripe, a Paddle, e seu novo
  adaptador.

## Próximos passos

- [Pagamentos](payments.md) - a superfície neutra em relação ao
  provedor e o Início Rápido
- [Pagamentos - adaptador Stripe](payments-stripe.md) - modelo
  completo para um adaptador de gateway
- [Pagamentos - adaptador Paddle](payments-paddle.md) - modelo
  completo para um adaptador Merchant-of-Record
- [Pagamentos Frontend](payments-frontend.md) - como renderizar o
  `SessionPayload` que seu adaptador retorna
- [Modelo de erros](error-model.md) - como `PaymentError` chega como
  um `HttpResponse`
