# Pagamentos

A superfície de pagamentos do Suprnova é neutra em relação ao
provedor. Você escolhe um crate adaptador - Stripe, Paddle, ou um que
você mesmo escreva - registra-o no boot, e seu código de domínio
chama as mesmas quatro traits centrais (mais uma quinta opcional para
captura do lado do servidor) independentemente de qual provedor está
por trás. Tabelas espelho no seu banco de dados são mantidas em
sincronia por webhooks, então seu código de domínio lê do seu próprio
banco em vez de acessar a API do provedor a cada consulta.

Nenhum recurso está condicionado a um único provedor. O modelo de
captura direta da Stripe e o modelo de Merchant of Record da Paddle
cabem ambos no mesmo contrato de trait. A única superfície que
difere é `Payment` (captura do lado do servidor), que é opcional - a
Paddle não precisa dela, então a Paddle não a implementa. Provedores
anunciam sua capacidade sobrescrevendo
`PaymentProvider::as_payment()` para retornar `Some(&dyn Payment)`;
quem chama consulta isso em tempo de execução.

## Por que Suprnova diverge

Laravel embute o Cashier como uma integração Stripe de primeira
classe na documentação central. É conveniente, mas exclusivo da
Stripe - adicionar um segundo provedor significa fazer fork do
Cashier ou construir uma superfície paralela. Suprnova trata
provedores de pagamento como trata drivers de cache e storage: um
conjunto de traits genérico, adaptadores intercambiáveis. Seu código
de domínio nunca nomeia `StripeProvider` ou `PaddleProvider`; ele
chama `provider.subscribe(...)` contra `Arc<dyn PaymentProvider>`
resolvido a partir de um registry, e o provedor por trás disso está a
uma mudança de bootstrap de ser outra coisa.

## Início rápido

Adicione o crate adaptador. Até o Suprnova lançar sua versão v0.1, o
framework e seus crates adaptadores são consumidos via git em vez de
via crates.io:

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Registre o provedor e o router de webhook no boot. O router de
webhook é um `Router` normal que você compõe no seu
`routes::register()`:

```rust,ignore
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_stripe::StripeProvider;

pub async fn register() {
    let stripe = StripeProvider::from_env().expect("Stripe env vars not set");
    PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
}
```

```rust,ignore
// src/routes.rs
use std::sync::Arc;
use suprnova::payments::webhook_routes;
use suprnova::container::App;
use suprnova::Router;
use sea_orm::DatabaseConnection;

/// `Application::routes(routes::register)` chama isto uma vez no boot.
/// Começamos a partir do router de webhook de pagamentos, e então
/// empilhamos o resto das rotas do app com chamadas normais de
/// `.get(...)` / `.post(...)`.
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ... o resto das suas rotas ...
        .into()
}
```

`webhook_routes(db)` retorna um `Router` contendo apenas
`POST /webhooks/payments/{provider}`. Como `Router::get` e
`Router::post` cada um retorna um `RouteBuilder` que se converte de
volta a `Router` via `.into()`, encadear por cima do router de
pagamentos é a forma mais direta de compor. Se você já usa a macro
`routes!{}` para suas rotas normais, jogue o POST do webhook no mesmo
bloco - `webhook_routes` é um wrapper de conveniência em torno de uma
única chamada `Router::new().post(...)`.

No seu controller, busque o provedor, crie um cliente, e abra uma
sessão de checkout:

```rust,ignore
// src/controllers/billing.rs
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("stripe")
        .ok_or_else(|| PaymentError::Internal("stripe not registered".into()))?;

    let customer = provider.create_customer(CreateCustomerRequest {
        user_id,
        email,
        name: None,
        metadata: None,
    }).await?;

    provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: customer.provider_customer_id,
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: None,
        metadata: None,
    }).await
}
```

Esse `SessionPayload` vai para as suas props de página Inertia. O
frontend despacha com base em `payload.flow` para renderizar o widget
certo - veja
[Pagamentos - integração Frontend](payments-frontend.md).

## Escolhendo um adaptador

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Variáveis de ambiente exigidas:

| Variável | Descrição |
|---|---|
| `STRIPE_SECRET_KEY` | Chave secreta (`sk_live_…` / `sk_test_…`) |
| `STRIPE_PUBLISHABLE_KEY` | Chave publicável (`pk_live_…` / `pk_test_…`) |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | Secret de assinatura do endpoint de webhook (`whsec_…`) |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// A partir do env (recomendado em produção):
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// Ou construa diretamente:
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripe implementa toda trait, incluindo a opcional `Payment`
(captura do lado do servidor via PaymentIntents) e `Promotions`
(emissão de códigos de promoção via `/v1/promotion_codes`). Tanto
`provider.as_payment()` quanto `provider.as_promotions()` retornam
`Some`.

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Variáveis de ambiente exigidas:

| Variável | Descrição |
|---|---|
| `PADDLE_API_KEY` | Chave de API (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) |
| `PADDLE_WEBHOOK_KEY` | Secret do destino de notificação (`pdl_ntfset_…`) |
| `PADDLE_CLIENT_TOKEN` | Token do lado do cliente (`live_…` / `test_…`) |
| `PADDLE_ENVIRONMENT` | Opcional, padrão `"sandbox"` |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// A partir do env:
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// Ou construa diretamente:
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddle é um Merchant of Record - ela administra impostos, dunning, e
todo o ciclo de vida da assinatura. Ela não expõe captura do lado do
servidor, então `Payment` não é implementada. Chamar
`provider.as_payment()` retorna `None`. Assinaturas são criadas
indiretamente: chame `Checkout::start_session`, complete o widget da
Paddle, e o webhook `SubscriptionCreated` chega para confirmar o ID
da assinatura.

## A divisão de traits

`PaymentProvider` é uma trait guarda-chuva que reúne quatro traits
universais - `Checkout`, `Subscription`, `CustomerStore`,
`WebhookHandler` - que todo adaptador implementa. Duas traits
adicionais são opcionais: `Payment` (captura do lado do servidor só
faz sentido para gateways como a Stripe) e `Promotions` (emissão de
códigos de promoção). Adaptadores optam por elas sobrescrevendo
`PaymentProvider::as_payment()` / `PaymentProvider::as_promotions()`.

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// Retorna `Some` se este provedor também implementa `Payment` (server-capture).
    /// O padrão retorna `None`.
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// Retorna `Some` se este provedor também implementa `Promotions`
    /// (emissão de códigos de promoção). O padrão retorna `None`.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - universal, abre o widget do cliente

Todo provedor implementa `Checkout`. Chame `start_session` para
obter um `SessionPayload` com tag de fluxo que seu frontend renderiza.
`session_status` (padrão: `NotSupported`; sobrescrito por provedores
cujas sessões podem ser consultadas, ex.: Stripe) relata o estado
autoritativo do lado do provedor de uma sessão que você iniciou
antes.

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

Campos de `StartSessionRequest`:

| Campo | Tipo | Descrição |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` ou `Subscription` |
| `customer_ref` | `String` | ID do cliente no provedor, de `CustomerStore::create_customer` |
| `price_refs` | `Vec<String>` | IDs de preço/produto no provedor |
| `success_return_url` | `String` | Para onde enviar o usuário depois do pagamento |
| `cancel_return_url` | `String` | Para onde enviar o usuário se ele abandonar |
| `amount_hint` | `Option<Money>` | Override ou dica para valores pontuais |
| `idempotency_key` | `Option<String>` | Para retries seguros |

`session_status` é a primitiva de verificação do lado do servidor
para fluxos de redirecionamento. Quando o cliente volta para a sua
página de retorno, NÃO confie nos parâmetros de query que o
navegador dele carregou - passe o `provider_session_id` que você
registrou no momento do `start_session` e ramifique com base no
resultado:

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // Cumpra o pedido. `payment_ref` (ex.: o `pi_…` da Stripe) se
        // correlaciona com operações de `Payment` e o espelho payments_transactions.
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* liquidação pendente */ }
    CheckoutSessionState::Open => { /* cliente ainda não terminou de pagar */ }
    CheckoutSessionState::Expired => { /* sessão expirou - encerre o pedido */ }
}
```

A mesma chamada alimenta varreduras de reconciliação: consulte de
novo os pedidos ainda abertos no seu banco e realize aqueles cujas
sessões completaram depois que o cliente fechou a aba.

### `Payment` - opcional, captura do lado do servidor

Só provedores que expõem captura do lado do servidor implementam
`Payment`. Stripe sim; Paddle não. Para verificar em tempo de
execução:

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(payment) = provider.as_payment() {
    let result = payment.charge(ChargeRequest {
        customer_ref: "cus_...".into(),
        payment_method_ref: "pm_...".into(),
        amount: Money::from_minor_units(2999, Currency::USD),
        description: Some("Pro plan one-off".into()),
        idempotency_key: Some("charge_user42_order99".into()),
        metadata: None,
    }).await?;
}
```

Interface completa de `Payment`:

```rust,ignore
#[async_trait]
pub trait Payment: Send + Sync {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult>;
    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult>;
    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult>;
    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()>;
    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus>;
}
```

`ChargeResult` é um enum marcado com `kind` - veja a seção
[Money e ChargeResult](#chargeresult).

### `Promotions` - opcional, emite códigos de promoção

Provedores com uma superfície de códigos de promoção implementam
`Promotions`. O próprio objeto de desconto (um cupom de percentual
ou de valor fixo) é criado com antecedência - tipicamente uma vez,
no dashboard do provedor - e essa trait emite *códigos* a partir
dele, cada um restrito a um cliente e uma janela de resgate. É essa
a forma de que campanhas de win-back e upsell precisam: todo
destinatário recebe um código pessoal, inutilizável por qualquer
outra pessoa e morto depois que a janela se encerra.

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // cupom pré-criado
        customer_ref: "cus_...".into(),             // só este cliente pode resgatar
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // uso único
    }).await?;
    // Envie `minted.code` por email ao cliente; ele o digita no checkout
    // e o provedor aplica toda restrição.
}
```

O `MockPaymentProvider` implementa `Promotions` (códigos são
emitidos como `PROMO_MOCK_n`) e registra toda requisição - faça
assert em `recorded_promotion_requests()` nos testes.

### `Subscription` - assina, atualiza, cancela, busca

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

Cancelar no fim do período (mantém acesso até o fim do ciclo de
cobrança):

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// Cancelar imediatamente:
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

Nota: `Paddle::subscribe` retorna `PaymentError::NotSupported` - a
Paddle cria assinaturas através da conclusão do checkout, não de
chamadas diretas de API. Use `Checkout::start_session` e espere o
webhook `SubscriptionCreated`.

### `CustomerStore` - cria, atualiza, busca, remove

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` recebe `user_id`, `email`,
`name: Option<String>`, e `metadata: Option<Value>`. `CustomerRef`
volta com `provider_customer_id` - guarde isso junto ao registro do
seu usuário para usar em chamadas subsequentes.

### `WebhookHandler` - verifica, faz parse, e extrai

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// Extrai IDs de entidade do payload bruto para que o framework saiba
    /// quais linhas do espelho hidratar. O padrão retorna um `PayloadIds` vazio.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// Constrói um `PaymentSnapshot` a partir de um evento de pagamento /
    /// fatura. O padrão retorna `None`, o que pula o upsert de `payments_transactions`.
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// Constrói um `CustomerSnapshot` a partir de um evento de cliente. O
    /// padrão retorna `None`, o que pula a atualização de email / metadata na linha existente.
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

Na prática você nunca chama nada disso diretamente -
`webhook_routes` invoca esses métodos para todo webhook recebido.
Eles vivem na trait para que crates adaptadores possam implementar
verificação de assinatura, parsing de evento e extração de payload
específicos do provedor de forma testável. Os métodos `extract_*`
todos têm padrões sensatos; os adaptadores Stripe e Paddle já
inclusos os sobrescrevem com implementações cientes do formato do
provedor (Stripe acessa `data.object.*`, Paddle acessa `data.*`).

## O payload do Inertia com tag `flow`

`start_session` retorna um enum `SessionPayload` que serializa para
JSON com um campo discriminador `flow`. Seu frontend decide com base
em `flow` qual widget renderizar:

```rust,ignore
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum SessionPayload {
    StripeElements {
        client_secret: String,
        publishable_key: String,
        provider_session_id: String,
    },
    StripeCheckoutRedirect {
        url: String,
        provider_session_id: String,
    },
    PaddleInline {
        transaction_id: String,
        customer_token: Option<String>,
        client_token: String,
    },
    /// Fluxo Mobile Money - sem redirecionamento nem embed. O frontend
    /// exibe uma mensagem para o usuário pedindo que confirme no
    /// celular dele (prompt USSD ou app da operadora), e então faz
    /// polling do provedor via `provider_transaction_id` por atualizações
    /// de status.
    MobileMoneyPrompt {
        provider_transaction_id: String,
        message: String,
        operator: MobileMoneyOperator,
    },
    Redirect {
        url: String,
        provider_session_id: String,
    },
}
```

Forma serializada de um payload `StripeElements`:

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

Um payload `MobileMoneyPrompt` se parece com isto - não há URL
porque o cliente nunca deixa a sua página; o frontend renderiza
`message` e começa a fazer polling:

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

Retorne do seu controller a variante que o provedor produzir, como
props do Inertia. A integração de frontend é descrita em
[Pagamentos - integração Frontend](payments-frontend.md).

## Tabelas espelho

Seis tabelas são criadas pela migração do framework. Importe o
alias público e inclua-o no migrator do seu app:

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... suas outras migrações ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

O mesmo módulo também exporta um helper
`pub fn migrations() -> Vec<Box<dyn MigrationTrait>>` para o caso de
você preferir chamar isso e espalhar o resultado na sua própria
lista.

### Visão geral das tabelas

| Tabela | Finalidade |
|---|---|
| `payments_customers` | Uma linha por par `(provider, user_id)` |
| `payments_payment_methods` | Métodos de pagamento armazenados por cliente |
| `payments_subscriptions` | Estado do ciclo de vida da assinatura |
| `payments_subscription_items` | Itens de linha dentro de uma assinatura |
| `payments_transactions` | Cobranças pontuais e faturas de assinatura |
| `payments_webhook_events` | Log de auditoria e guarda de idempotência |

Toda tabela tem uma coluna JSON `provider_metadata`. Quando a
representação neutra do framework não cobre um campo específico do
provedor, leia-o de lá.

### Tabela de transações

`payments_transactions` divide os valores em `amount_total_minor` e
`amount_tax_minor`. Stripe relata valores exclusivos de imposto - o
imposto é zero na linha de transação, e qualquer dado de imposto vive
em `provider_metadata`. Paddle relata valores inclusivos de imposto e
define `amount_tax_minor` como o componente de imposto. As duas
representações funcionam; some `amount_total_minor - amount_tax_minor`
para o valor líquido.

### Tabela de eventos de webhook

`payments_webhook_events` tem um índice
`UNIQUE(provider, provider_event_id)`. Todo webhook recebido é
verificado contra isso antes do processamento - duplicatas retornam
200 OK sem reprocessar. Isso é estrutural: Stripe, Paddle, e a
maioria dos provedores refazem webhooks falhados de forma agressiva.

### Ressalvas

Código de domínio lê das tabelas espelho, não diretamente da API do
provedor. Mutações (criar assinatura, cancelar, etc.) vão para o
provedor; o webhook resultante sincroniza as tabelas espelho de
volta. Isso significa que há uma janela breve entre uma mutação e a
chegada do webhook em que suas tabelas espelho ficam atrasadas.
Projete sua UX levando isso em conta (mostre estados de
"processando", confie nas URLs de redirecionamento do provedor para
confirmação imediata).

## Tratamento de webhooks

Monte a rota de ingestão de webhook uma vez no bootstrap - veja o
exemplo de rotas em [Início rápido](#início-rápido) para o padrão de
composição. `webhook_routes(db)` retorna um `Router` carregando o
único handler `POST /webhooks/payments/{provider}` embutido no
framework. Você encadeia suas próprias rotas nele (ou chama as
primitivas subjacentes da rota diretamente dentro do seu próprio
bloco `routes!{}`).

O handler do framework faz isto a cada requisição:

1. Busca o provedor nomeado no `PaymentProviderRegistry`.
2. Chama `WebhookHandler::verify` para checar a assinatura. Retorna
   401 em caso de falha.
3. Chama `WebhookHandler::parse_event` para construir um
   `WebhookEvent`. Retorna 400 em caso de falha de parse.
4. Verifica `payments_webhook_events` por uma linha existente com o
   mesmo `(provider, provider_event_id)`. Se encontrada, retorna 200
   imediatamente - essa é a guarda de idempotência.
5. Insere a linha de auditoria.

### Estrutura de WebhookEvent

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // string bruta do provedor, ex.: "customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // mapeado para a taxonomia do framework, ou None para eventos específicos do provedor
    pub raw_payload: Value,                 // corpo JSON completo para fallthrough
}
```

`NeutralEventKind` cobre o caminho comum:

```rust,ignore
pub enum NeutralEventKind {
    PaymentSucceeded,
    PaymentFailed,
    PaymentRefunded,
    PaymentDisputed,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    InvoicePaid,
    InvoiceFailed,
    CustomerCreated,
    CustomerUpdated,
}
```

Quando `neutral` é `None`, o evento é específico do provedor. Leia
`provider_event_type` e `raw_payload` para os dados completos.

### Hidratação da tabela espelho

Depois que a linha de auditoria é persistida, o framework despacha
o evento para a tabela espelho relevante com base em `neutral`.
**Todas as escritas no espelho para um evento acontecem dentro de
uma única transação de banco junto com `mark_processed`** - estado
parcial do espelho nunca é observável. Ou tudo commita junto ou tudo
sofre rollback.

| `NeutralEventKind`               | Efeito no espelho                                                                                       |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated`    | Chama `Subscription::get(id)` no provedor, faz upsert de `payments_subscriptions`, sincroniza itens.       |
| `SubscriptionCanceled`           | Igual ao anterior; também define `canceled_at` e vira `status` para `canceled` na linha existente.        |
| `PaymentSucceeded / Failed / Refunded / Disputed` | Faz upsert de `payments_transactions` a partir do snapshot que o provedor produz de `raw_payload`.        |
| `InvoicePaid / InvoiceFailed`    | Faz upsert de `payments_transactions` com `provider_subscription_id` vinculado.                              |
| `CustomerCreated / CustomerUpdated` | Atualiza `email` / `provider_metadata` da linha existente de `payments_customers` a partir do `CustomerSnapshot` do provedor. **Nunca insere.**   |
| `None` (não mapeado)                | Só linha de auditoria - sem mudança no espelho.                                                                   |

O espelho de cliente é intencionalmente só-de-atualização no
caminho do webhook. `user_id` é `NOT NULL` e só o app sabe a qual
usuário um cliente do provedor pertence (o vínculo é criado pelo seu
código logo depois de `CustomerStore::create_customer`). Clientes
fora de banda - criados no dashboard da Stripe, por exemplo - são
registrados em log mas nunca sintetizados no espelho.

### Contrato de recuperação de falhas

O handler trata retries do provedor como o mecanismo de recuperação:

- **Hidratação com sucesso:** a transação commita, `processed_at` é
  definido, `process_error` é limpo. Resposta: `200 ok`.
- **Hidratação falha:** a transação sofre rollback (sem estado
  parcial do espelho), a linha de auditoria mantém
  `processed_at = NULL` e `process_error` registra a falha. Resposta:
  `503 hydration-failed` - o provedor vai fazer retry com backoff.
- **Provedor refaz o evento falhado:** a verificação de idempotência
  vê a linha de auditoria existente mas `processed_at IS NULL`,
  então a hidratação roda de novo. O retry substitui o
  `process_error` obsoleto pelo resultado da tentativa atual.
- **Provedor refaz um evento bem-sucedido:** a verificação de
  idempotência vê `processed_at IS NOT NULL`, retorna
  `200 duplicate` imediatamente. Sem rehidratação.

Um evento de assinatura/cliente com um `subscription_id` /
`customer_id` faltando no payload é tratado como um erro
`Validation` (também 503 + `process_error` registrado). Sucesso
silencioso em um payload malformado deixaria o espelho obsoleto sem
visibilidade para o operador.

Itens removidos de uma assinatura do lado do provedor (ex.: o
usuário abandonou um add-on de assento) são removidos de
`payments_subscription_items` quando o próximo webhook
`subscription.updated` chega. A resposta de
`Subscription::get(id)` do provedor é a fonte da verdade em toda
sincronização.

## Métodos de pagamento além de cartões

`PaymentMethod` é o enum que o framework usa para métodos
armazenados em `payments_payment_methods` e para qualquer provedor
que exponha metadados de método. Ele cobre os casos óbvios - cartões,
transferências bancárias, e-wallets - mais métodos regionais que são
de primeira classe em muitos mercados:

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// Pagador identificado por telefone + operadora + país.
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// Cripto pareada (pegged) - equivalente a dinheiro para a maioria dos provedores.
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// Criptomoeda não pareada (non-pegged).
    Crypto { network: String, address: String },
    /// Válvula de escape para métodos regionais / específicos do provedor ainda não modelados.
    Custom { kind: String, descriptor: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMoneyOperator {
    MtnMomo,
    Mpesa,
    AirtelMoney,
    OrangeMoney,
    Lipila,
    Custom { identifier: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StablecoinAsset {
    Usdc,
    Usdt,
    Dai,
    Custom { ticker: String },
}
```

Os operadores e ativos nomeados são os que já enumeramos. As
variantes `Custom { ... }` em cada um cobrem operadores regionais e
stablecoins que ainda não fixamos, então adicionar suporte para um
deles não força um release do framework.

`PhoneNumber` e `CountryCode` são DTOs validados em
`suprnova::payments` - eles rejeitam entrada malformada no momento
da construção, que é onde você quer a falha, em vez de na chamada ao
provedor.

## Money

Valores são representados como `Money` - uma contagem `i64` em
unidades menores mais uma `Currency`. Nenhum `f64` envolvido.

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// A partir de unidades menores (centavos, pence, yen, etc.)
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// A partir de uma string decimal
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// Moedas sem decimais - 1234 minor = 1234 JPY (sem conversão)
let yen = Money::from_minor_units(1234, Currency::JPY);

// Aritmética - sofre panic em caso de moedas incompatíveis
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// Valores negativos representam reembolsos ou créditos
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// Lê de volta
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` e `Sub` sofrem panic em caso de moedas incompatíveis e de
overflow de `i64`. Use a aritmética com panic para garantir
correção - soma silenciosa entre moedas diferentes é um bug, não um
recurso.

## ChargeResult

`Payment::charge` retorna um enum `ChargeResult`. Nem toda cobrança
completa imediatamente - step-up 3DS e cartões off-session podem
exigir um redirecionamento ou uma ação do lado do cliente:

```rust,ignore
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChargeResult {
    Completed {
        provider_transaction_id: String,
        amount: Money,
        status: PaymentStatus,
        provider_metadata: Value,
    },
    RedirectRequired {
        provider_transaction_id: String,
        url: String,
        return_to: Option<String>,
    },
    RequiresClientAction {
        provider_transaction_id: String,
        action_kind: String,
        client_secret: Option<String>,
        publishable_key: Option<String>,
    },
}
```

Trate `RequiresClientAction` retornando o payload para o seu
frontend. O frontend renderiza o desafio 3DS usando `client_secret` +
`publishable_key`. Veja
[Pagamentos - integração Frontend](payments-frontend.md) para o
código de despacho do frontend.

## Chaves de idempotência

Todo DTO de mutação tem um `idempotency_key: Option<String>`
opcional. Defina uma nas chamadas de rede que podem ser refeitas:

```rust,ignore
provider.start_session(StartSessionRequest {
    // ...
    idempotency_key: Some(format!("checkout_{}_{}", user_id, order_id)),
    // ...
}).await?;

provider.subscribe(SubscribeRequest {
    // ...
    idempotency_key: Some(format!("sub_{}_{}", user_id, plan_id)),
    // ...
}).await?;
```

Stripe honra chaves de idempotência via o header HTTP
`Idempotency-Key`. Paddle tem um mecanismo equivalente. Se uma
requisição falha no meio do caminho e você refaz com a mesma chave,
o provedor retorna a resposta original em vez de criar uma cobrança
ou assinatura duplicada.

## O padrão discriminador

Todo adaptador que afirma implementar `PaymentProvider` precisa
passar pelo mesmo fluxo E2E:

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

O `MockPaymentProvider` incluído no framework passa por isso:

```rust,ignore
use suprnova::payments::*;

#[tokio::test]
async fn discriminator_flow() {
    let provider = MockPaymentProvider::new();

    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "user_42".into(),
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        metadata: None,
    }).await.unwrap();

    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("idem_1".into()),
        metadata: None,
    }).await.unwrap();
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        trial_days: None,
        idempotency_key: Some("idem_2".into()),
        metadata: None,
    }).await.unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // Cancelar no fim do período
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // Cancelar imediatamente
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProvider deliberadamente omite Payment (opcional estilo Paddle)
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` não implementa `Payment` - isso exercita o
mesmo invariante que a Paddle. `StripeProvider` e `PaddleProvider`
ambos passam pelo mesmo fluxo contra a API real em testes de
integração.

## Apps com múltiplos provedores

Registre os dois adaptadores no boot e despache com base em onde o
registro de cada cliente foi criado:

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// Depois, por requisição:
let provider_name = user.payment_provider.as_str(); // "stripe" or "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

Usos comuns: rotear clientes da UE pela Paddle (para o tratamento de
imposto do MoR) e clientes dos EUA pela Stripe; fazer teste A/B de
conversão de checkout entre provedores; usar um provedor para
assinaturas e outro para cobranças pontuais.

## Migração do Laravel Cashier

Cashier é exclusivo da Stripe por design. Suprnova já vem com
múltiplos provedores prontos. Mapeamento rápido:

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await` (se o provedor suportar) |
| `$invoice->download()` | Não embutido; leia `provider_metadata["invoice_pdf_url"]` da tabela espelho de transações |

## Próximos passos

- [Pagamentos - adaptador Stripe](payments-stripe.md) - o fluxo de
  gateway em detalhe: PaymentIntents, formato de assinatura de
  webhook, mapeamento de tipo de evento
- [Pagamentos - adaptador Paddle](payments-paddle.md) - o fluxo de
  MoR em detalhe: criação de assinatura via checkout, tratamento de
  imposto, verificação de notificação
- [Pagamentos - integração Frontend](payments-frontend.md) - exemplos
  de despacho por fluxo em Svelte 5, React 19, e Vue 3.5
- [Escrevendo um adaptador de provedor de pagamento](payments-provider-guide.md) -
  construa seu próprio crate adaptador do início ao fim
- [Banco de dados](database.md) - a camada SeaORM sobre a qual as
  tabelas espelho ficam
