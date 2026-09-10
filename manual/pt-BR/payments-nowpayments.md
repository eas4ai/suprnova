# Pagamentos - NOWPayments

O adaptador `suprnova-payments-nowpayments` cria faturas hospedadas, verifica
notificações de pagamento e lê o status do pagamento usando a chave de API do
lojista. Ele se registra como `nowpayments` no registro normal de provedores
de pagamento.

## Instalação e configuração

Use uma revisão do Suprnova que contenha este adaptador para as duas
dependências. Para uma aplicação local ao lado de um checkout do código-fonte
do Suprnova:

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

Configure estes valores no ambiente protegido da aplicação:

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

O ambiente aceita `sandbox` ou `production` e usa `sandbox` por padrão. Um
ambiente desconhecido ou vazio faz a configuração falhar. Credenciais vazias
falham antes das requisições HTTP. Use credenciais separadas para cada
ambiente.

Registre o provedor durante o boot, propagando um erro de configuração:

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

Adicione as migrations de pagamento e componha `webhook_routes(db)` no router
da aplicação como mostrado na [visão geral de pagamentos](payments.md). O
endpoint é `POST /webhooks/payments/nowpayments`. Mantenha essa rota
autenticada pelo provedor fora do middleware de CSRF de sessões de navegador.
Configure o callback com a URL HTTPS pública exata; os corpos das requisições
precisam chegar ao adaptador sem alteração.

## Iniciar uma fatura hospedada

Persista uma tentativa de checkout com uma referência de pedido do lojista
única antes de fazer a requisição. O valor vem do pedido confiável da
aplicação, nunca de um total fornecido pelo navegador.

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` aceita a mesma requisição e retorna o genérico
`SessionPayload::Redirect`. Guarde o `provider_session_id` dele como um **ID
de fatura** e então redirecione para a URL validada. O método concreto
`create_invoice` também retorna a referência de pedido do lojista e distingue
erros de criação incertos.

O adaptador de faturas exige modo avulso, referências de cliente e de preço
vazias, e um valor `Money` positivo com um expoente de moeda fiduciária
definido. Ele envia o valor decimal exato na unidade principal e o código de
moeda em minúsculas. Os clientes escolhem uma criptomoeda na página
hospedada, a menos que `pay_currency` seja fornecido.

Os metadados são um objeto estrito com estes campos:

| Campo | Significado |
| --- | --- |
| `order_id` | Referência do lojista obrigatória e não vazia, no máximo 128 bytes |
| `order_description` | Descrição opcional, no máximo 500 bytes |
| `pay_currency` | Ticker do provedor opcional em minúsculas, como `btc` |
| `is_fixed_rate` | Ajuste opcional de taxa de câmbio do provedor |
| `is_fee_paid_by_user` | Ajuste opcional de tarifas do provedor |

Chaves de metadados desconhecidas são rejeitadas. Não coloque dados
arbitrários da aplicação ou do cliente neste objeto. As URLs de retorno
precisam usar a mesma origem HTTPS do callback, sem credenciais nem
fragmentos. Os redirecionamentos de fatura precisam apontar para o ambiente
NOWPayments selecionado e conter o ID de fatura retornado.

## Falhas de criação e novas tentativas

O endpoint de faturas da NOWPayments não documenta nenhuma garantia de chave
de idempotência. O adaptador rejeita um `idempotency_key` diferente de
`None`; `order_id` é dado de correlação, não deduplicação do lado do
provedor. Nem redirecionamentos HTTP nem novas tentativas automáticas estão
habilitados. As requisições têm prazo de 30 segundos e limite de resposta de
64 KiB.

`InvoiceCreationError::Rejected` representa validação de entrada ou uma
rejeição definitiva da API. `Unknown` significa que uma fatura pode já
existir: por exemplo, a requisição estourou o prazo, o provedor retornou um
erro de servidor, ou a resposta de sucesso dele era inválida. Marque essa
tentativa como incerta e reconcilie o pedido no painel do provedor antes de
criar outra fatura. Não coloque a criação de faturas dentro de um laço
genérico de novas tentativas. Através de `Checkout::start_session`, um
resultado desconhecido é um `PaymentError::Provider` com esta instrução de
recuperação.

## Verificar e reconciliar pagamentos

Uma fatura pode produzir um ID de pagamento separado.
`payment_status(payment_id)` chama o endpoint autenticado de pagamento e
rejeita uma resposta com um ID diferente.
`Checkout::session_status(invoice_id)` retorna `NotSupported`; um ID de
fatura não pode ser substituído no endpoint de consulta de pagamento. Este
adaptador não usa as credenciais de e-mail e senha do painel para listar
pagamentos por fatura.

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

O ID de pagamento precisa vir de um IPN verificado ou de outro registro
confiável do lado do servidor. Um retorno do navegador não é prova de
pagamento. Uma consulta bem-sucedida também não é uma verificação de
autorização: case-a com o pedido, a fatura, o valor e a moeda armazenados
antes de alterar o acesso. `price` é o preço fiduciário solicitado, não a
quantidade de cripto recebida. Revise as configurações do lojista para
aceitação de pagamento parcial; este adaptador nunca converte
`partially_paid` em sucesso.

| Estado do provedor | Evento neutro |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`, desconhecido | Sem classificação neutra |

As assinaturas de IPN usam JSON ordenado recursivamente e HMAC SHA-512. A
verificação usa uma comparação de MAC em tempo constante. Assinaturas
ausentes, duplicadas ou inválidas, chaves JSON duplicadas, payloads grandes
demais e campos obrigatórios malformados são rejeitados. O provedor não
assina nenhum timestamp de entrega independente, então não existe uma janela
inventada de replay por timestamp. A proteção contra replay usa recibos
persistidos.

Os IPNs da NOWPayments não têm um ID de evento independente. O adaptador
identifica um recibo pelo ID de pagamento e pelo status, então uma entrega
repetida com um `updated_at` diferente não repete o mesmo evento terminal. O
framework guarda os eventos verificados em `payments_webhook_events` e refaz
o processamento que falhou. Um evento pendente atrasado não modifica uma
tabela espelho de transação já liquidada.

### Faturas sem cliente e o estado da aplicação

A API de faturas não fornece a identidade de cliente que as tabelas espelho
de transações do Suprnova exigem. Este adaptador retorna `false` em
`WebhookHandler::mirrors_payment_transactions`: todos os eventos verificados,
inclusive estornos, permanecem no log de auditoria, mas ele não cria nenhuma
tabela espelho de cliente ou de transação. Stripe e Paddle mantêm o
comportamento de espelho padrão.

A rota genérica de webhooks registra e processa os eventos do provedor; ela
não realiza o fulfillment dos pedidos da aplicação. Um job de reconciliação
da aplicação deve consumir os recibos verificados, ler o status atual do
pagamento e atualizar seu pedido em uma transação idempotente. Guarde uma
chave de fulfillment única por provedor e pagamento, ou por pedido, para que
novas tentativas e entregas fora de ordem não possam conceder acesso duas
vezes. Acompanhe o estado durável de conclusão do próprio job separadamente
do `processed_at` do framework. Use o estado autenticado atual para eventos
atrasados e trate os estornos conforme a política da aplicação.

## Limites de capacidade

As operações de `CustomerStore` e `Subscription` retornam `NotSupported`.
`as_payment()` e `as_promotions()` retornam `None`. Este adaptador não
implementa saldos em custódia, cobrança recorrente, checkout direto por
endereço de depósito, repasses, início de estorno, nem gestão de clientes no
provedor. A NOWPayments oferece produtos separados para algumas dessas
operações; este adaptador de faturas não finge implementá-las.

### Por que Suprnova diverge

As traits comuns de provedor continuam disponíveis, mas operações não
suportadas falham de forma explícita. Faturas sem clientes no provedor usam
notificações auditadas e pedidos de propriedade da aplicação, em vez de
registros de cliente ou assinaturas fabricados.

## Verificação

Rode os testes do adaptador e as regressões de pagamento compartilhadas a
partir do checkout do código-fonte:

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

A suíte local usa respostas HTTP falsas, uma fixture de assinatura em
JavaScript gerada de forma independente, e ingresso real do framework com
SQLite. Ela cobre checkout bem-sucedido, falhas de autenticação, respostas
malformadas ou grandes, timeouts, redirecionamentos não confiáveis,
assinaturas inválidas, mapeamento de status, entregas duplicadas, e
recuperação após uma falha de banco de dados. As fixtures locais não
estabelecem interoperabilidade com o provedor.

Antes de habilitar produção, use uma conta sandbox da NOWPayments e um
callback HTTPS acessível. Crie uma fatura, exercite os casos de sucesso,
parciais e de falha disponíveis no provedor, confirme que a assinatura dele é
aceita, e reconcilie o ID de pagamento com a fatura e o pedido armazenados.
Repita uma entrega e confirme que a chave de fulfillment da aplicação impede
uma segunda concessão. Nenhuma execução em sandbox ou produção com conta real
é implicada pelos testes locais.

Referências do provedor: [endpoints da API](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description),
[autenticação de IPN](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)
e o [SDK oficial](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs).

## Próximos passos

Veja [Integração de frontend](payments-frontend.md) para renderizar payloads
de redirecionamento, ou o [Guia do provedor](payments-provider-guide.md) para
os contratos compartilhados.
