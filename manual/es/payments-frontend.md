# Pagos - Integración de Frontend

El servidor devuelve un `SessionPayload` como parte de las props de tu
página de Inertia. El payload lleva un campo `flow` que le dice al
frontend qué widget montar; tu frontend despacha según `flow` y nunca
nombra un proveedor específico. Este capítulo cubre los bucles de
despacho de Svelte 5, React 19, y Vue 3.5, incluido el ciclo de
confirm-card-payment de Stripe Elements y el handler de step-up 3DS
fuera de sesión.

Los cinco valores posibles de `flow` y sus campos asociados:

| `flow` | Campos | Widget |
|---|---|---|
| `stripe_elements` | `client_secret`, `publishable_key`, `provider_session_id` | Stripe Elements (formulario de tarjeta incrustado) |
| `stripe_checkout_redirect` | `url`, `provider_session_id` | Redirección al checkout alojado por Stripe |
| `paddle_inline` | `transaction_id`, `client_token`, `customer_token?` | Overlay incrustado de Paddle.js |
| `mobile_money_prompt` | `provider_transaction_id`, `message`, `operator` | Aviso USSD / app del operador + sondeo |
| `redirect` | `url`, `provider_session_id` | Redirección genérica (Mollie, mock, etc.) |

El controlador del backend llama a `Checkout::start_session` y
devuelve el resultado como props de Inertia - desde la perspectiva del
frontend, la API es la misma sin importar qué adaptador esté
corriendo.

## Despacha según `flow`, no según el proveedor

Tu página de checkout lee el campo `flow` una vez y monta el widget
que corresponde. Nunca nombra "Stripe" ni "Paddle"; solo el bootstrap
que elige el adaptador lo sabe. Este es el contrato sobre el que se
construye el resto del capítulo.

### Por qué Suprnova diverge

Laravel Cashier distribuye una vista Blade para Stripe Checkout, una
ruta de partials para SCA, y una convención de SDK separada para
Paddle. Los caminos de Stripe y de Paddle no comparten un contrato de
frontend - el widget de cada proveedor está conectado a una acción de
controlador distinta y a un árbol de plantillas distinto.

Suprnova invierte eso: el backend siempre devuelve el mismo enum
`SessionPayload` y el frontend siempre conmuta según `flow`. Añadir un
proveedor nuevo significa añadir una variante del lado del servidor y
un `case` del lado del cliente; el resto de tu página de checkout no
se mueve. La variante de Mobile Money es la prueba - no produce ningún
widget en absoluto (el cliente confirma desde su teléfono), y el
despachador la absorbe sin ningún caso especial en el componente que
lo llama.

## Svelte 5

```svelte
<!-- src/pages/Billing/Checkout.svelte -->
<script lang="ts">
  import { page } from "@inertiajs/svelte";

  // SessionPayload llega en las props de la página de Inertia
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
    // Stripe.js debe estar cargado - añade esto a index.html:
    // <script src="https://js.stripe.com/v3/"></script>
    const stripe = (window as any).Stripe(s.publishable_key);
    const elements = stripe.elements({ clientSecret: s.client_secret });

    const card = elements.create("card");
    card.mount("#card-element");

    // Conecta el envío del formulario:
    const form = document.getElementById("payment-form") as HTMLFormElement;
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const { error, paymentIntent } = await stripe.confirmCardPayment(s.client_secret, {
        payment_method: { card },
      });
      if (error) {
        // Muestra el error al usuario
        console.error(error.message);
      } else if (paymentIntent?.status === "succeeded") {
        // Pago completo - navega o muestra la confirmación
        window.location.href = "/billing/success";
      }
    });
  }

  function mountPaddleInline(s: Extract<SessionPayload, { flow: "paddle_inline" }>) {
    // Paddle.js debe estar cargado - añade esto a index.html:
    // <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>
    const Paddle = (window as any).Paddle;
    Paddle.Initialize({ token: s.client_token });
    Paddle.Checkout.open({
      transactionId: s.transaction_id,
      customerToken: s.customer_token,
    });
  }

  async function pollMobileMoney(txId: string) {
    // Sondea tu propio backend, que lee la tabla de copia local de
    // transacciones. El handler de webhook actualiza la fila
    // cuando el proveedor nos notifica.
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
  <!-- Solo se renderiza para stripe_elements; oculto en cualquier otro caso -->
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

La salvaguarda `mountedRef` evita el doble montaje bajo el
doble-render de desarrollo del StrictMode de React 19.

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

## Carga de los SDKs de pago

Añade los scripts pertinentes a tu `index.html` (o el punto de entrada
equivalente). Incluye solo los que requiera tu selección de
proveedores:

```html
<!-- Stripe (añádelo si usas stripe_elements o stripe_checkout_redirect) -->
<script src="https://js.stripe.com/v3/" crossorigin="anonymous"></script>

<!-- Paddle (añádelo si usas paddle_inline) -->
<script src="https://cdn.paddle.com/paddle/v2/paddle.js" crossorigin="anonymous"></script>
```

El navegador carga ambos scripts de forma asíncrona. Si usas Vite con
code-splitting, cárgalos vía `import()` dinámico, o inclúyelos como
externals en tu `vite.config.ts` para evitar empaquetar tú mismo los
SDKs del proveedor.

Tanto Stripe como Paddle exigen que cargues el SDK desde su propio
CDN - Stripe hace de esto una condición de cumplimiento PCI, y Paddle
depende de eso para la reescritura de URL en vivo. La Integridad de
Subrecursos (`integrity="sha384-..."`) no es usable en ninguno de los
dos scripts porque ambos proveedores distribuyen de forma continua y
no publican hashes estables; el límite de confianza es la conexión
HTTPS más el CDN del proveedor. Si tu modelo de amenazas exige SRI
para todo lo que incrustas, esa es una señal para mantener toda la UI
de pago en un checkout alojado por el proveedor
(`stripe_checkout_redirect`, o el overlay alojado de Paddle invocado
desde una redirección emitida por el servidor) en lugar de en tu
propia página.

## Tipos de TypeScript

El tipo `SessionPayload` que se muestra en cada ejemplo de arriba es
una unión discriminada que coincide con la forma serializada del enum
de Rust. Puedes generarlo automáticamente con
`suprnova generate-types` si tu `SessionPayload` se expone mediante un
envoltorio `#[derive(InertiaProps)]`, o definirlo a mano como se
muestra.

## Sondeo de Mobile Money

`mobile_money_prompt` es el único flujo en el que el cliente nunca
vuelve a tocar tu página después de que llega el aviso. Confirma desde
su teléfono (menú USSD o notificación push de la app del operador), el
proveedor notifica a tu handler de webhook, y tu frontend tiene que
descubrir que la transacción se liquidó.

Conecta un pequeño endpoint de estado que lea la tabla de copia local
`payments_transactions` por `provider_transaction_id`. El handler de
webhook instalado por `webhook_routes(db)` mantiene al día la columna
de estado de la fila; tu endpoint solo la refleja de vuelta:

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

El ayudante `pollMobileMoney` del frontend que se muestra en cada
ejemplo de arriba llama a ese endpoint cada tres segundos, con un
techo de cinco minutos. Las cadenas de estado vienen del enum
`PaymentStatus` y se serializan en snake_case: `created`,
`requires_action`, `pending`, `processing`, `authorized`, `expired`,
`succeeded`, `failed`, `canceled`, `refunded`, `partially_refunded`,
`disputed`.

## Manejo de errores - `RequiresClientAction`

Cuando `Payment::charge` (captura del lado del servidor) devuelve
`ChargeResult::RequiresClientAction`, el backend serializa el
resultado a JSON y lo devuelve al frontend. Esto ocurre en los flujos
de step-up 3DS fuera de sesión, donde el emisor de la tarjeta exige
autenticación adicional.

El JSON luce así:

```json
{
  "kind": "requires_client_action",
  "provider_transaction_id": "pi_...",
  "action_kind": "stripe_3ds",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_..."
}
```

`client_secret` y `publishable_key` son `Option<String>` del lado de
Rust y estarán ausentes del JSON cuando una acción no los necesite.
Comprueba siempre que ninguno de los dos sea null antes de pasarlos a
un SDK de proveedor, y deja que `action_kind` impulse el despacho -
ese campo siempre está presente.

Tu controlador de backend debería detectar esto y devolverlo como una
prop de Inertia distinta, o como una respuesta HTTP que el frontend
lea. Patrón de controlador de ejemplo:

```rust,ignore
use suprnova::payments::ChargeResult;

let result = payment.charge(req).await?;
match result {
    ChargeResult::Completed { .. } => {
        // Redirige a la página de éxito
    }
    ChargeResult::RequiresClientAction { action_kind, client_secret, publishable_key, .. } => {
        return inertia.render("Billing/ThreeDSChallenge", json!({
            "action_kind": action_kind,
            "client_secret": client_secret,
            "publishable_key": publishable_key,
        }));
    }
    ChargeResult::RedirectRequired { url, .. } => {
        // Redirige el navegador
    }
}
```

En el frontend, despacha según `action_kind`:

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
      // Muestra el mensaje de fallo del 3DS
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

El campo `action_kind` es una cadena específica del proveedor.
Actualmente `"stripe_3ds"` es el único valor que produce el adaptador
de Stripe que se distribuye. Cuando otros adaptadores requieran
acciones del cliente, añadirán sus propios valores de `action_kind`
siguiendo el mismo patrón - escribe una rama por defecto
(`console.warn("Unknown action_kind:", k)`) para que un valor no
reconocido falle de forma estrepitosa en lugar de descartar el pago en
silencio.

## Siguiente

- [Pagos](payments.md) - la superficie de cinco traits, el registro, y
  el patrón de bootstrap que produce el `SessionPayload`.
- [Pagos - Stripe](payments-stripe.md) - la configuración del lado del
  servidor para los flujos `stripe_elements`,
  `stripe_checkout_redirect`, y `stripe_3ds`.
- [Pagos - Paddle](payments-paddle.md) - la configuración del lado del
  servidor para el flujo `paddle_inline` y el reparto de
  responsabilidad de Merchant of Record.
- [Pagos - Guía del proveedor](payments-provider-guide.md) - añade una
  variante nueva de `SessionPayload` cuando escribas un adaptador para
  una pasarela que Suprnova no distribuye.
- [Frontend](frontend.md) - la configuración de páginas de Inertia, el
  tipado de props, y cómo `usePage` se conecta a tu starter de
  Svelte / React / Vue.
