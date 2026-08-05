# Pagamentos - integração Frontend

O servidor retorna um `SessionPayload` como parte das suas props de
página Inertia. O payload carrega um campo `flow` que diz ao
frontend qual widget montar; seu frontend despacha com base em
`flow` e nunca nomeia um provedor específico. Este capítulo cobre os
loops de despacho em Svelte 5, React 19, e Vue 3.5, incluindo o
ciclo de confirm-card-payment do Stripe Elements e o handler de
step-up 3DS off-session.

Os cinco valores possíveis de `flow` e seus campos associados:

| `flow` | Campos | Widget |
|---|---|---|
| `stripe_elements` | `client_secret`, `publishable_key`, `provider_session_id` | Stripe Elements (formulário de cartão embutido) |
| `stripe_checkout_redirect` | `url`, `provider_session_id` | Redirecionamento para checkout hospedado pela Stripe |
| `paddle_inline` | `transaction_id`, `client_token`, `customer_token?` | Overlay inline do Paddle.js |
| `mobile_money_prompt` | `provider_transaction_id`, `message`, `operator` | Prompt USSD / app da operadora + polling |
| `redirect` | `url`, `provider_session_id` | Redirecionamento genérico (Mollie, mock, etc.) |

O controller do backend chama `Checkout::start_session` e retorna o
resultado como props do Inertia - da perspectiva do frontend a API é
a mesma independentemente de qual adaptador está rodando.

## Despache com base em `flow`, não no provedor

Sua página de checkout lê o campo `flow` uma vez e monta o widget
correspondente. Ela nunca nomeia "Stripe" ou "Paddle"; só o
bootstrap que escolheu o adaptador sabe. Esse é o contrato sobre o
qual o resto do capítulo se constrói.

### Por que Suprnova diverge

Laravel Cashier embute uma view Blade para o Stripe Checkout, um
caminho de partials para SCA, e uma convenção de SDK separada para a
Paddle. Os caminhos Stripe e Paddle não compartilham um contrato de
frontend - o widget de cada provedor está conectado a uma action de
controller diferente e uma árvore de template diferente.

Suprnova inverte isso: o backend sempre retorna o mesmo enum
`SessionPayload` e o frontend sempre decide com base em `flow`.
Adicionar um novo provedor significa adicionar uma variante do lado
do servidor e um `case` do lado do cliente; o resto da sua página de
checkout não se move. A variante Mobile Money é a prova - ela não
produz widget nenhum (o cliente confirma no celular dele), e o
dispatcher a absorve sem nenhum tratamento especial no componente
que chama.

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayload chega nas props de página do Inertia
  let session = $derived($page.props.session as SessionPayload);

  type MobileMoneyOperator =
    | { kind: "mtn_momo" }
    | { kind: "mpesa" }
    | { kind: "airtel_money" }
    | { kind: "orange_money" }
    | { kind: "lipila" }
    | { kind: "custom"; identifier: string };

  type SessionPayload =
    | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
    | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
    | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
    | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
    | { flow: "redirect"; url: string; provider_session_id: string };

  let mobileMessage = $state("");

  $effect(() => {
    if (!session) return;
    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        mobileMessage = session.message;
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  });

  async function mountStripeElements(s: Extract<SessionPayload, { flow: "stripe_elements" }>) {
    // O Stripe.js precisa estar carregado - adicione ao index.html:
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // Conecte o envio do formulário:
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // Mostre o erro ao usuário
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // Pagamento completo - navegue ou mostre confirmação
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // O Paddle.js precisa estar carregado - adicione ao index.html:
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // Faça polling do seu próprio backend, que lê a tabela espelho de transações.
    // O handler de webhook atualiza a linha quando o provedor nos notifica.
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }
</script>

<div id="payment-form">
  <div id="card-element"></div>
  <!-- Só renderizado para stripe_elements; oculto no resto -->
  {#if session?.flow === "stripe_elements"}
    <button type="submit">Pay now</button>
  {/if}
  {#if session?.flow === "mobile_money_prompt"}
    <p>{mobileMessage}</p>
    <p>Waiting for confirmation…</p>
  {/if}
</div>
```

## React 19

```tsx
// src/pages/Billing/Checkout.tsx
import { useEffect, useRef, useState } from "react";
import { usePage } from "@inertiajs/react";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

export default function Checkout() {
  const { session } = usePage<{ session: SessionPayload }>().props;
  const mountedRef = useRef(false);
  const [mobileMessage, setMobileMessage] = useState("");

  useEffect(() => {
    if (!session || mountedRef.current) return;
    mountedRef.current = true;

    switch (session.flow) {
      case "stripe_elements":
        mountStripeElements(session);
        break;
      case "stripe_checkout_redirect":
        window.location.href = session.url;
        break;
      case "paddle_inline":
        mountPaddleInline(session);
        break;
      case "mobile_money_prompt":
        setMobileMessage(session.message);
        pollMobileMoney(session.provider_transaction_id);
        break;
      case "redirect":
        window.location.href = session.url;
        break;
    }
  }, [session]);

  async function mountStripeElements(
    s: Extract<SessionPayload, { flow: "stripe_elements" }>
  ) {
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });
    const card = elements.create("card");
    card.mount("#card-element");

    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(
    s: Extract<SessionPayload, { flow: "paddle_inline" }>
  ) {
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    const deadline = Date.now() + 5 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000));
      const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
      const { status } = await res.json();
      if (status === "succeeded") {
        window.location.href = "/billing/success";
        return;
      }
      if (status === "failed" || status === "canceled" || status === "expired") {
        window.location.href = "/billing/failed";
        return;
      }
    }
  }

  return (
    <form id="payment-form">
      <div id="card-element" />
      {session?.flow === "stripe_elements" && (
        <button type="submit">Pay now</button>
      )}
      {session?.flow === "mobile_money_prompt" && (
        <div>
          <p>{mobileMessage}</p>
          <p>Waiting for confirmation…</p>
        </div>
      )}
    </form>
  );
}
```

A guarda `mountedRef` previne dupla-montagem sob o duplo-render de
desenvolvimento do StrictMode do React 19.

## Vue 3.5

```vue
<!-- src/pages/Billing/Checkout.vue -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { usePage } from "@inertiajs/vue3";

type MobileMoneyOperator =
  | { kind: "mtn_momo" }
  | { kind: "mpesa" }
  | { kind: "airtel_money" }
  | { kind: "orange_money" }
  | { kind: "lipila" }
  | { kind: "custom"; identifier: string };

type SessionPayload =
  | { flow: "stripe_elements"; client_secret: string; publishable_key: string; provider_session_id: string }
  | { flow: "stripe_checkout_redirect"; url: string; provider_session_id: string }
  | { flow: "paddle_inline"; transaction_id: string; client_token: string; customer_token?: string }
  | { flow: "mobile_money_prompt"; provider_transaction_id: string; message: string; operator: MobileMoneyOperator }
  | { flow: "redirect"; url: string; provider_session_id: string };

const page = usePage<{ session: SessionPayload }>();
const session = page.props.session;
const isStripeElements = ref(session?.flow === "stripe_elements");
const isMobileMoney = ref(session?.flow === "mobile_money_prompt");
const mobileMessage = ref(
  session?.flow === "mobile_money_prompt" ? session.message : ""
);

onMounted(() => {
  if (!session) return;
  switch (session.flow) {
    case "stripe_elements":
      mountStripeElements(session);
      break;
    case "stripe_checkout_redirect":
      window.location.href = session.url;
      break;
    case "paddle_inline":
      mountPaddleInline(session);
      break;
    case "mobile_money_prompt":
      pollMobileMoney(session.provider_transaction_id);
      break;
    case "redirect":
      window.location.href = session.url;
      break;
  }
});

async function mountStripeElements(
  s: Extract<SessionPayload, { flow: "stripe_elements" }>
) {
  const stripe = (window as any).Stripe(s.publishable_key);
  const elements = stripe.elements({ clientSecret: s.client_secret });
  const card = elements.create("card");
  card.mount("#card-element");

  const form = document.getElementById("payment-form") as HTMLFormElement;
  form?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
      payment_method: { card },
    });
    if (error) {
      console.error(error.message);
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  });
}

function mountPaddleInline(
  s: Extract<SessionPayload, { flow: "paddle_inline" }>
) {
  const Paddle = (window as any).Paddle;
  Paddle.Initialize({ token: s.client_token });
  Paddle.Checkout.open({
    transactionId: s.transaction_id,
    customerToken: s.customer_token,
  });
}

async function pollMobileMoney(txId: string) {
  const deadline = Date.now() + 5 * 60_000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 3000));
    const res = await fetch(`/billing/status?transaction_id=${encodeURIComponent(txId)}`);
    const { status } = await res.json();
    if (status === "succeeded") {
      window.location.href = "/billing/success";
      return;
    }
    if (status === "failed" || status === "canceled" || status === "expired") {
      window.location.href = "/billing/failed";
      return;
    }
  }
}
</script>

<template>
  <form id="payment-form">
    <div id="card-element" />
    <button v-if="isStripeElements" type="submit">Pay now</button>
    <div v-if="isMobileMoney">
      <p>{{ mobileMessage }}</p>
      <p>Waiting for confirmation…</p>
    </div>
  </form>
</template>
```

## Carregando os SDKs de pagamento

Adicione os scripts relevantes ao seu `index.html` (ou ponto de
entrada equivalente). Inclua só os que a seleção do seu provedor
exige:

```html
<!-- Stripe (adicione se usar stripe_elements ou stripe_checkout_redirect) -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle (adicione se usar paddle_inline) -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

Os dois scripts são carregados de forma assíncrona pelo navegador.
Se você usa Vite com code-splitting, carregue-os via `import()`
dinâmico ou os inclua como externals no seu `vite.config.ts` para
evitar empacotar os SDKs do provedor você mesmo.

Stripe e Paddle ambos exigem que você carregue o SDK a partir do CDN
deles - a Stripe faz disso uma condição de conformidade PCI, e a
Paddle depende disso para reescrita de URL em tempo real.
Subresource Integrity (`integrity="sha384-..."`) não é usável em
nenhum dos dois scripts porque ambos os fornecedores fazem deploy
continuamente e não publicam hashes estáveis; a fronteira de
confiança é a conexão HTTPS mais o CDN do fornecedor. Se seu modelo
de ameaça exige SRI para tudo que você embute, isso é um sinal para
manter toda UI de pagamento em um checkout hospedado pelo fornecedor
(`stripe_checkout_redirect`, ou o overlay hospedado da Paddle
invocado a partir de um redirecionamento emitido pelo servidor) em
vez de na sua própria página.

## Tipos TypeScript

O tipo `SessionPayload` mostrado em cada exemplo acima é uma união
discriminada que corresponde à forma serializada do enum Rust. Você
pode gerá-lo automaticamente com `suprnova generate-types` se seu
`SessionPayload` for exposto via um wrapper
`#[derive(InertiaProps)]`, ou defini-lo manualmente como mostrado.

## Polling do Mobile Money

`mobile_money_prompt` é o único fluxo em que o cliente nunca toca na
sua página depois que o prompt chega. Ele confirma no celular dele
(menu USSD ou push do app da operadora), o provedor notifica seu
handler de webhook, e seu frontend precisa descobrir que a
transação liquidou.

Conecte um pequeno endpoint de status que lê a tabela espelho
`payments_transactions` por `provider_transaction_id`. O handler de
webhook instalado por `webhook_routes(db)` mantém a coluna de status
da linha atualizada; seu endpoint simplesmente a reflete de volta:

```rust,ignore
use suprnova::{Json, Query, json_response};
use suprnova::payments::entities::transaction;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    pub transaction_id: String,
}

pub async fn status(Query(q): Query<StatusQuery>) -> Json<serde_json::Value> {
    let db = suprnova::db().await;
    let row = transaction::Entity::find()
        .filter(transaction::Column::ProviderTransactionId.eq(q.transaction_id))
        .one(&db)
        .await
        .unwrap();
    let status = row.map(|r| r.status).unwrap_or_else(|| "pending".into());
    Json(serde_json::json!({ "status": status }))
}
```

O helper `pollMobileMoney` do frontend mostrado em cada exemplo
acima acessa esse endpoint a cada três segundos com um teto de cinco
minutos. As strings de status vêm do enum `PaymentStatus` e
serializam em snake_case: `created`, `requires_action`, `pending`,
`processing`, `authorized`, `expired`, `succeeded`, `failed`,
`canceled`, `refunded`, `partially_refunded`, `disputed`.

## Tratamento de erros - `RequiresClientAction`

Quando `Payment::charge` (captura do lado do servidor) retorna
`ChargeResult::RequiresClientAction`, o backend serializa o
resultado para JSON e o retorna ao frontend. Isso acontece em fluxos
de step-up 3DS off-session onde a emissora do cartão exige
autenticação adicional.

O JSON se parece com isto:

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` e `publishable_key` são `Option<String>` do lado do
Rust e vão estar ausentes do JSON quando uma ação não precisar
deles. Sempre verifique se ambos são nulos antes de passá-los para
um SDK do provedor, e deixe `action_kind` conduzir o despacho - esse
campo está sempre presente.

Seu controller de backend deve detectar isso e retorná-lo como uma
prop Inertia distinta ou como uma resposta HTTP que o frontend lê.
Exemplo de padrão de controller:

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // Redirecione para a página de sucesso
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // Redirecione o navegador
    }
}
```

No frontend, despache com base em `action_kind`:

**Svelte 5:**

```svelte
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  let props = $derived($page.props as {
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  });

  $effect(() => {
    if (!props.action_kind) return;
    switch (props.action_kind) {
      case "stripe_3ds":
        handleStripe3DS(props.client_secret!, props.publishable_key!);
        break;
      default:
        console.warn("Unknown action_kind:", props.action_kind);
    }
  });

  async function handleStripe3DS(clientSecret: string, publishableKey: string) {
    const stripe = (window as any).Stripe(publishableKey);
    const { error, paymentIntent } = await stripe.handleNextAction({ clientSecret });
    if (error) {
      // Mostre mensagem de falha do 3DS
    } else if (paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
</script>
```

**React 19:**

```tsx
import { usePage } from "@inertiajs/react";
import { useEffect } from "react";

export default function ThreeDSChallenge() {
  const { action_kind, client_secret, publishable_key } = usePage<{
    action_kind: string;
    client_secret?: string;
    publishable_key?: string;
  }>().props;

  useEffect(() => {
    if (!action_kind) return;
    if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
      const stripe = (window as any).Stripe(publishable_key);
      stripe.handleNextAction({ clientSecret: client_secret }).then(
        ({ error, paymentIntent }: any) => {
          if (!error && paymentIntent?.status === "succeeded") {
            window.location.href = "/billing/success";
          }
        }
      );
    }
  }, [action_kind]);

  return <div>Completing payment authentication...</div>;
}
```

**Vue 3.5:**

```vue
<script setup lang="ts">
import { onMounted } from "vue";
import { usePage } from "@inertiajs/vue3";

const { action_kind, client_secret, publishable_key } = usePage<{
  action_kind: string;
  client_secret?: string;
  publishable_key?: string;
}>().props;

onMounted(async () => {
  if (action_kind === "stripe_3ds" && client_secret && publishable_key) {
    const stripe = (window as any).Stripe(publishable_key);
    const { error, paymentIntent } = await stripe.handleNextAction({
      clientSecret: client_secret,
    });
    if (!error && paymentIntent?.status === "succeeded") {
      window.location.href = "/billing/success";
    }
  }
});
</script>

<template>
  <p>Completing payment authentication...</p>
</template>
```

O campo `action_kind` é uma string específica do provedor.
Atualmente `"stripe_3ds"` é o único valor produzido pelo adaptador
Stripe já distribuído. Quando adaptadores adicionais exigirem ações
do cliente, eles vão adicionar seus próprios valores de
`action_kind` seguindo o mesmo padrão - escreva um branch padrão
(`console.warn("Unknown action_kind:", k)`) para que um valor não
reconhecido falhe de forma explícita em vez de descartar o pagamento
silenciosamente.

## Próximos passos

- [Pagamentos](payments.md) - a superfície de cinco traits, o
  registry, e o padrão de bootstrap que produz o `SessionPayload`.
- [Pagamentos - Stripe](payments-stripe.md) - configuração do lado
  do servidor para os fluxos `stripe_elements`,
  `stripe_checkout_redirect`, e `stripe_3ds`.
- [Pagamentos - Paddle](payments-paddle.md) - configuração do lado
  do servidor para o fluxo `paddle_inline` e a divisão de
  responsabilidade de Merchant-of-Record.
- [Pagamentos - guia do provedor](payments-provider-guide.md) -
  adicione uma nova variante de `SessionPayload` quando você
  escrever um adaptador para um gateway que o Suprnova não
  distribui.
- [Frontend](frontend.md) - configuração de página Inertia,
  tipagem de props, e como `usePage` se conecta ao seu starter de
  Svelte / React / Vue.
