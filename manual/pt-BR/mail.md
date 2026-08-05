# Correio

O subsistema de mail do Suprnova espelha a API
`Mail::to(...)->send(...)` do Laravel sobre o Tokio. Uma
facade `Mail`, oito transportes (log e in-memory para
dev/testes, SMTP, e cinco provedores HTTP - Postmark, SES,
SendGrid, Mailgun, Resend), templates renderizados com Tera
usando os campos serializados do Mailable como contexto, fila +
entrega adiada sobre o envelope durável de ao-menos-uma-vez,
e um guarda de teste `Mail::fake()` do mesmo tipo que
`Bus::fake()` e `Cache::fake()`.

## Início rápido

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

O Mailable serializa para JSON, que se torna o contexto Tera
para o template; todo campo `pub` é alcançável como `{{
field_name }}`.

## Configuração

`Server::serve` chama
`suprnova::mail::boot::bootstrap_from_env()` uma vez na
inicialização. Ele lê `MAIL_DRIVER` e vincula o transporte
correspondente. Tem como default o driver `log` quando não
definido.

| `MAIL_DRIVER` | Comportamento |
|---------------|----------|
| `log`         | Emite um `tracing::info!` por envio - envelope e corpos completos, como o Laravel faz - e descarta. Default fora de produção. |
| `memory`      | Captura toda mensagem em processo. Veja `suprnova::mail::boot::captured_in_memory()`. |
| `smtp`        | Conecta a um servidor SMTP (STARTTLS quando credenciais estão definidas, TCP puro caso contrário). |
| `postmark`    | Faz POST de JSON para o endpoint `/email` do Postmark. |
| `ses`         | Faz POST de solicitações assinadas com SigV4 para o `SendEmail` do Amazon SES. |
| `sendgrid`    | Faz POST de JSON para `/v3/mail/send` do SendGrid. |
| `mailgun`     | Faz POST de `application/x-www-form-urlencoded` (ou `multipart/form-data` quando anexos estão presentes) para `/v3/{domain}/messages` do Mailgun. |
| `resend`      | Faz POST de JSON para `/emails` do Resend. |

### Produção falha de forma fechada em um driver que descarta mail

`log` e `memory` renderizam uma mensagem e a descartam. Sob
`APP_ENV=production`, o boot **se recusa** a iniciar em
qualquer um dos dois - e igualmente em um `MAIL_DRIVER` não
definido ou um valor que o build não reconhece, porque ambos
caem no mesmo transporte `log`:

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

A falha que isso previne é silenciosa: com o default antigo,
um deploy que esqueceu `MAIL_DRIVER` - ou escreveu
`MAIL_DRIVER=SMTP` com a caixa errada - reportava toda
redefinição de senha como enviada enquanto nada nunca saía do
processo, e ninguém descobria até que um usuário ficasse
trancado de fora.

Se uma implantação de produção genuinamente quer nenhum mail
de saída (um espelho somente-leitura, um dark launch),
reconheça isso explicitamente:

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

Só `1`, `true`, `yes`, ou `on` contam como consentimento -
`=false` ou um erro de digitação deixa a salvaguarda armada.
Com a sobrescrita definida, todo boot avisa que mail de saída
não será entregue.

Nada muda fora de produção: `local`, `development`,
`testing`, e `staging` mantêm o default `log` e mantêm o
comportamento de avisar-e-recair para drivers desconhecidos.

### Produção falha de forma fechada em uma conexão SMTP não criptografada

A mesma regra, aplicada a como a conexão é protegida, em vez
de a se ela entrega. `MAIL_DRIVER=smtp` em produção precisa
resolver para um transporte criptografado, ou o boot falha.

`MAIL_SMTP_ENCRYPTION` aceita `starttls`, `tls`, ou `none`
(`ssl` e `null` são aceitos como aliases compatíveis com o
Laravel). Se não definida, ela deriva das credenciais:

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | Resolve para | Porque |
|---|---|---|
| ambos definidos | `starttls` | Credenciais implicam um relay de verdade na porta de submission. |
| nenhum definido | `none` | O caminho do catcher local. Mailpit, MailHog e maildev escutam sem autenticação na 1025 e não falam TLS. |

Então um scaffold novo continua funcionando com zero
configuração, e um deploy de produção que nunca conectou as
credenciais para em vez de enviar silenciosamente em texto
claro. Defina `MAIL_SMTP_ENCRYPTION=tls` para um relay que
espera TLS implícito na 465 - um modo que o transporte sempre
suportou, mas que nenhuma combinação de variáveis de ambiente
conseguia alcançar antes.

Um valor não reconhecido falha o boot em *todo* ambiente, não
só em produção. `MAIL_SMTP_ENCRYPTION=tsl` é uma transposição
de um modo que criptografa, então tratá-lo silenciosamente
como "sem criptografia" seria exatamente a falha que a
variável existe para prevenir - melhor falhar na máquina do
desenvolvedor do que no deploy.

A válvula de escape espelha a de cima:

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

Só defensável quando o relay é alcançável somente através de
uma rede privada - um sidecar, ou um Postfix dentro da VPC. Em
qualquer outra coisa, SMTP em texto claro coloca as
credenciais e todo link de redefinição de senha na rede, e
isso fica lá para quem estiver escutando no caminho.

### O driver `log` loga a mensagem inteira

O mesmo que o mailer `log` do Laravel: envelope *e* corpos
renderizados.

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

Esse link é o ponto principal. No development, o console é
onde você lê o link de verificação ou redefinição de senha
que o app acabou de "enviar", e um driver que o esconde é um
driver que ninguém consegue usar.

Isso é seguro aqui porque o driver não consegue alcançar
produção - o boot se recusa a iniciar em `MAIL_DRIVER=log` sob
`APP_ENV=production` (veja acima). Os corpos só existem na
máquina de um desenvolvedor.

Se você define `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`
para rodar o driver `log` em um ambiente implantado, você está
escolhendo colocar links bearer de uso único nos seus logs.
Qualquer um que consiga ler esses arquivos - operadores, o log
shipper, o bucket de retenção, o agregador - pode usá-los, e a
expiração do link não ajuda porque o log shipping é mais
rápido que uma pessoa lendo sua caixa de entrada. Dimensione
sua política de retenção e acesso para isso, ou use um driver
que não imprime:

```env
# Captura em processo - suprnova::mail::boot::captured_in_memory(), ou Mail::fake() em testes
MAIL_DRIVER=memory

# Ou um catcher local (mailpit / maildev / mailhog), que renderiza o mail real em uma UI
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### Ambiente por driver

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # ou `tls` para TLS implícito na 465, ou `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

Cada provedor HTTP também honra uma sobrescrita
`MAIL_<PROVIDER>_ENDPOINT` correspondente que aponta para uma
URL regional ou um servidor mock (útil para testes de
integração contra `wiremock`).

### Remetente de auth-flow: `MAIL_FROM` e `MAIL_FROM_NAME`

Os mailables de auth-flow embutidos - verificação de email,
redefinição de senha, e o aviso de senha-alterada - resolvem
seu `From` de envelope a partir do ambiente, em vez de um
`from()` fixo no código:

```env
MAIL_FROM=no-reply@example.com        # endereço puro (exigido pelos fluxos de auth; falha de forma fechada se não definido)
MAIL_FROM_NAME=Acme Support           # nome de exibição opcional (desde a 0.5.9)
```

- `MAIL_FROM` **precisa ser um endereço puro.** Ele é levado
  direto para o `From` da mensagem, então um valor `"Name
  <addr>"` seria tratado como o endereço inteiro e rejeitado
  pelo transporte.
- `MAIL_FROM_NAME` (opcional, adicionado na **0.5.9**) anexa
  um nome de exibição, então o header renderiza como `Acme
  Support <no-reply@example.com>`. Não definido ou em branco
  mantém o comportamento anterior de endereço puro. É lido no
  momento do envio, então também se aplica a mail de
  auth-flow enfileirado.

Essas duas variáveis só afetam os próprios mailables de
auth-flow do framework. Seus próprios `Mailable`s definem seu
remetente através de `from()` (ou o default global
`always_from`) - veja abaixo.

## A trait Mailable

Mailables são structs serializáveis que sabem como se
renderizar. Os defaults da trait renderizam com
`tera::Tera::one_off` contra os campos serializados do
mailable:

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| Método | Obrigatório? | Propósito |
|--------|-----------|---------|
| `mailable_name()` | sim | Nome estável persistido no envelope da fila - renomear quebra mail enfileirado em trânsito. |
| `subject(&self)` | sim | Assunto computado. Usado ao pé da letra quando `subject_template_source` retorna `None`. |
| `subject_template_source(&self)` | opcional | Template Tera para o assunto - quando `Some`, tem precedência sobre `subject()` e renderiza com `self` como contexto. Mesma semântica dos template sources de corpo. |
| `html_template_source(&self)` | opcional | Template Tera do corpo HTML. Retorne `None` para pular HTML. |
| `text_template_source(&self)` | opcional | Template Tera do corpo em texto puro. Retorne `None` para pular texto. |
| `from(&self)` | opcional | Sobrescreve o default global `noreply@localhost`. |
| `attachments(&self)` | opcional | Arquivos para anexar. Cada um é `name + bytes + mime`. |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | opcional | Sobrescreva se você quer contornar o Tera (Markdown → HTML, conteúdo pré-renderizado, lógica de assunto customizada, etc.). |

Pelo menos um de `html_template_source` ou
`text_template_source` precisa retornar `Some` (ou
`render_html`/`render_text` precisam produzir conteúdo). Um
mailable de corpo vazio é recusado tanto no dispatch
(`Mail::send`) quanto no enqueue (`Mail::queue`).

### Autoescape do Tera

O autoescape está **DESLIGADO** porque corpos de mail são
tipicamente HTML escrito à mão, onde o escaping `<>&` do Tera
faria escaping demais. Se seu corpo literal contém `{{` por
razões que não são de template (por exemplo, texto de
marketing citando sintaxe Mustache), escape isso: `{% raw
%}{{ literal }}{% endraw %}`.

## Construindo mensagens

O builder `Mail::to(...)` costura destinatários, CC/BCC,
reply-to, e uma sobrescrita de remetente por mensagem, no
dispatch:

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // (nome de exibição, email)
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` aceita `&str`, `String`, e tuplas `(name, email)`;
`Mail::to(...)` aceita qualquer coisa que seja
`Into<Address>`.

## Anexos

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

Anexos passam através do método `Mailable::attachments`. Os
cinco provedores HTTP lidam com eles - Postmark/SendGrid/Resend
via JSON (codificado em base64), SES via Raw MIME (já que
`Content.Simple` não suporta anexos), e Mailgun via
`multipart/form-data` (o caminho form-encoded é usado quando
não há anexos).

## Enfileiramento

`Mail::queue(...)` constrói um `SendMailJob` e o empurra para
a fila do framework. O worker reconstrói o mailable a partir
da factory registrada e despacha através do transporte
vinculado:

```rust
// Uma vez só: registre todo tipo Mailable que o worker vai ver.
suprnova::mail::register_mailable_factory::<Welcome>()?;

// No momento do envio:
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// Adiado:
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

A mesma salvaguarda de corpo-vazio roda no caminho da fila,
então um Mailable mal configurado é rejeitado no momento do
push antes que qualquer envelope seja criado.

## Telemetria

Todo envio roteia através de
`suprnova::mail::dispatch_with_telemetry`, que abre um
`tracing::info_span!` de `mail.send` carregando:

- `transport` - nome do driver (`"postmark"`, `"smtp"`,
  `"in-memory"`, …)
- `to_count`, `cc_count`, `bcc_count` - contagens de
  destinatário
- `has_html`, `has_text` - forma do corpo
- `attachment_count` - número de anexos
- `tag_count`, `metadata_count` - contagens de dica-de-provedor
- `priority` - `1..=5`, ou `0` quando não definido

Na conclusão, o span emite `mail sent` (info) ou `mail send
failed` (warn) com `duration_ms`. O mesmo wrapper cobre
`Mail::send`, o worker de fila `SendMailJob`, e o
`MailChannel` de notificação, então o schema do span é
idêntico independente de como a mensagem foi produzida.

## Testando com `Mail::fake()`

`Mail::fake()` instala um transporte de captura em memória
pela duração da guarda RAII retornada. Espelha `Bus::fake()` /
`Queue::fake()` / `Cache::fake()`:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));
}
```

Quando a guarda dropa, o transporte previamente vinculado (se
houver) é restaurado. Testes que misturam `Mail::fake()` com
vinculação explícita de transporte não vazam estado.

`Mail::fake()` é `Send + Sync`; compartilhe-o através de
awaits ou threads conforme necessário.

## Transportes customizados

A trait `MailTransport` é o ponto de integração:

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// No boot:
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

Transportes rodam no runtime do Tokio - IO assíncrono, pooling
de conexão, e envio concorrente são de primeira classe. Não há
penalidade de fork por solicitação.

### Por que Suprnova diverge

A camada Mailable do Laravel é construída sobre o Symfony
Mailer, que roda de forma síncrona dentro do ciclo de vida da
solicitação. O `MailTransport` do Suprnova é `async fn
send(&self, msg: &OutgoingMessage)` de ponta a ponta: os
provedores HTTP usam `reqwest`, o caminho SMTP usa um
adaptador lettre assíncrono, e `dispatch_with_telemetry`
envolve todo envio em um span de `tracing` do Tokio.
Provedores de longa distância não bloqueiam a thread do
handler, pools de conexão sobrevivem entre solicitações, e
envios concorrentes em um handler são triviais -
`tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))`
faz o que você esperaria.

A outra divergência é o cancelamento de evento. O Laravel
modela um listener de `MessageSending` que pode retornar
`false` e suprimir o envio (`events->until()`). O dispatcher
do Suprnova não expõe um canal de retorno de short-circuit -
`MessageSending` é somente-observação. Para bloquear um envio,
recuse na camada Mailable (sobrescreva `render_html` /
`render_text` para retornar um erro) ou envolva a chamada de
`MailBuilder::send` com sua própria salvaguarda. A troca é
real: perdemos um hook do Laravel para manter o contrato do
dispatcher simples.

Uma divergência menor é hardening deliberado. O Laravel se
contenta em deixar `MAIL_MAILER=log` rodando em produção; o
Suprnova se recusa a dar boot lá sem um reconhecimento
explícito, porque um subsistema de mail que reporta sucesso e
não entrega nada é o tipo de queda que ninguém percebe por
semanas. O próprio driver `log` se comporta exatamente como o
do Laravel - mensagem completa, corpos e links incluídos - o
que é o que o torna útil no development, e a recusa em
produção é o que mantém isso seguro (veja [O driver `log` loga
a mensagem inteira](#o-driver-log-loga-a-mensagem-inteira)).

## Boas práticas

### Registre factories no boot, não por solicitação

`Mail::queue` e `Mail::later` empurram um `SendMailJob`
carregando o nome do mailable e o payload JSON - o worker
reconstrói o tipo concreto via `mailable_registry`. Registre
todo `Mailable` enfileirável uma vez, no momento do
`Server::serve`:

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

Um `Mail::queue` para um mailable não registrado cai na fila,
roda uma vez, bate em "unknown mailable", tenta de novo
conforme a política de backoff do envelope, e vai para
dead-letter - custando tempo de observabilidade que você não
teria gastado se a factory estivesse vinculada no boot.

### Enfileire mail para qualquer render lento ou não confiável

Enviar mail em um handler de solicitação acopla a latência de
resposta do usuário ao seu servidor SMTP (ou à API HTTP de
qualquer provedor). Use `Mail::queue` para qualquer coisa além
de um render síncrono local-dev, e `Mail::later` quando você
quer o dispatch adiado - follow-ups de onboarding, emails de
lembrete, digests agendados.

```rust
// Ruim: amarra o tempo de resposta ao provedor de mail
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// Bom: o 200 OK retorna imediatamente; o worker entrega o mail.
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Sempre defina `from` em um Mailable

O remetente default do framework é `noreply@localhost` - útil
para capturar remetentes ausentes no development, não um
remetente que qualquer provedor vai aceitar em produção.
Sobrescreva `Mailable::from(&self)` (ou defina `from = "..."`
no atributo `#[mail(...)]` em um `NotificationMailable`) para
que toda mensagem despachada tenha uma identidade de remetente
real:

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

A sobrescrita por mensagem no `MailBuilder`
(`.from(("Operations", "ops@example.com"))`) tem precedência
sobre o default do mailable - útil para envios transacionais
pontuais.

### Use a fila para entrega ao-menos-uma-vez, não o caminho direto

`MailBuilder::send` é no-máximo-uma-vez: se o transporte
falhar no meio do despacho para dois provedores, você não
consegue tentar de novo sem arriscar envio duplicado.
`MailBuilder::queue` viaja sobre o envelope durável da fila,
que suporta chaves de idempotência e retry em nível de worker.
Para qualquer mail que você não pode perder E não pode enviar
duas vezes, enfileire com uma chave de idempotência estável
vinculada ao evento de origem.

## Mensagens pontuais: `Mail::raw` e `Mail::html`

Quando o mail é um único ping transacional que não justifica
uma struct `Mailable` completa, dois atalhos pulam o
boilerplate:

```rust
use suprnova::mail::Mail;

// Texto puro
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("Hi")
        .from("hello@example.com")
}).await?;
```

A closure recebe um [`MailBuilder`] pré-carregado com o corpo
e deixa você colocar em camadas destinatários, assunto,
remetente, tags, metadados, prioridade, e qualquer outro
método fluente de [`MailBuilder`] por cima. Esses caminhos
contornam a trait `Mailable` inteiramente - útil para pings de
teste de tiro único e notas transacionais curtas.

## Defaults globais: `always_from`, `always_reply_to`, `always_to`, `always_return_path`

Espelhando `Mailer::alwaysFrom` / `alwaysReplyTo` / `alwaysTo`
/ `alwaysReturnPath` do Laravel, a facade Mail expõe quatro
setters globais:

```rust
use suprnova::mail::{Address, Mail};

// No boot:
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// "Caixa de entrada única" para local-dev - roteia TODO
// mail para um endereço, descarta CC/BCC:
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// Desfaz tudo (testes tipicamente chamam isso no
// teardown):
Mail::forget_always()?;
```

A precedência é conservadora - defaults só se aplicam quando a
mensagem despachada não tem um valor explícito:

| Campo | O default se aplica quando |
|-------|---------------------|
| `always_from` | O `from` da mensagem é o default `noreply@localhost` do framework |
| `always_reply_to` | A mensagem não tem `reply_to` explícito |
| `always_to` | Sempre - roteia toda mensagem para esse endereço, limpa CC/BCC |
| `always_return_path` | A mensagem não tem `return_path` explícito |

A mesma precedência se aplica no caminho da fila: mailables
enfileirados passam por `apply_always_defaults` no momento do
dispatch do worker, então envios diretos e envios enfileirados
convergem em formas de envelope idênticas.

## Tags, Metadados, Prioridade, Headers, Return-Path

Toda mensagem despachada pode carregar dicas de provedor no
estilo Laravel - tags, pares de chave/valor de metadados,
prioridade RFC-2076, headers MIME customizados, e um endereço
Sender / bounce-to. Eles são encaminhados para os campos
nativos dos provedores HTTP (Postmark `Tag` / `Metadata` /
`Headers`, SES `EmailTags`, SendGrid `categories` /
`custom_args` / `headers`, Mailgun `o:tag` / `v:` / `h:`,
Resend `tags` / `headers`) e para o SMTP como headers RFC
5322.

Duas formas de anexá-los - no nível do Mailable para defaults
por tipo, ou por mensagem no builder:

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// Por mensagem, no builder. O builder vence em colisões de
// chave de metadados; tags + headers fazem união.
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

Constantes para os cinco níveis de prioridade vivem em
`suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH,
PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}` - a mesma
escala de inteiros `1..=5` que o Laravel usa.

## Inspecionando mensagens capturadas

`OutgoingMessage` carrega helpers de inspeção no estilo
Laravel - úteis tanto para assertions de teste quanto para
logging de auditoria em runtime:

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Welcome") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

Verificações de destinatário não diferenciam
maiúsculas/minúsculas no email; verificações de metadados,
tag, assunto, e nome-de-arquivo-de-anexo são exatas.

## Fake de teste: superfície expandida

`Mail::fake()` cobre AMBAS as trilhas, enviada e enfileirada.
Mail enviado (via `MailBuilder::send`) cai no transporte em
memória; mail enfileirado (via `.queue` / `.later`) cai no
buffer de fila do fake.

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // Lado enviado
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Welcome"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Password reset"));

    // Lado enfileirado (para mails adiados)
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // Composto
    fake.assert_outgoing_count(2);   // enviado + enfileirado
    fake.assert_not_outgoing("PasswordReset");
}
```

Helpers adicionais:

| Helper | Propósito |
|--------|---------|
| `fake.captured()` | Todas as mensagens enviadas |
| `fake.count()` | Contagem de enviadas |
| `fake.queued()` | Todos os `QueuedSnapshot`s enfileirados |
| `fake.queued_count()` | Contagem de enfileiradas |
| `fake.outgoing_count()` | Enviadas + enfileiradas |
| `fake.sent(predicate)` | Filtra enviadas por predicado |
| `fake.sent_to(email)` | Filtra enviadas por destinatário |
| `fake.queued_named(name)` | Mailables enfileirados de um dado nome |
| `fake.queued_to(email)` | Mailables enfileirados para destinatário |
| `fake.assert_sent_count(n)` | Contagem exata de enviadas |
| `fake.assert_queued_count(n)` | Contagem exata de enfileiradas |
| `fake.assert_outgoing_count(n)` | Total exato |
| `fake.assert_nothing_sent()` | Buffer de enviadas vazio |
| `fake.assert_nothing_queued()` | Buffer de enfileiradas vazio |
| `fake.assert_nothing_outgoing()` | Ambos vazios |
| `fake.assert_sent_to(email)` | Pelo menos uma enviada para destinatário |
| `fake.assert_not_sent_to(email)` | Nenhuma enviada para destinatário |
| `fake.assert_queued(name)` | Pelo menos uma enfileirada de nome |
| `fake.assert_queued_with(name, fn)` | Pelo menos uma enfileirada de nome que corresponde ao predicado |
| `fake.assert_queued_to(email)` | Pelo menos uma enfileirada para destinatário |
| `fake.assert_not_queued(name)` | Nenhuma enfileirada de nome |

`QueuedSnapshot::decode::<M>()` deserializa o payload de volta
para o `M` concreto, então predicados verificados por tipo
funcionam sem boilerplate de decode sob medida.

## Eventos: `MessageSending` e `MessageSent`

Todo dispatch bem-sucedido dispara dois eventos do framework:

- `MessageSending` - imediatamente ANTES da chamada de
  transporte. Listeners observam a forma da mensagem
  (destinatários, assunto, tags, flags de forma de corpo).
- `MessageSent` - imediatamente DEPOIS de uma chamada de
  transporte bem-sucedida. Listeners observam a mesma forma;
  envios que falham não emitem esse evento.

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

Os dois eventos são somente-observação - o dispatcher não
modela um canal de cancelamento no estilo Laravel. Veja [Por
que Suprnova diverge](#por-que-suprnova-diverge) acima para o
workaround de bloqueio.

## Conveniência multi-destinatário: `Mail::cc` e `Mail::bcc`

A facade Mail expõe três pontos de entrada - `to`, `cc`,
`bcc` - que todos retornam um `MailBuilder` novo. Use qualquer
um que combine com a intenção de roteamento dominante:

```rust
// Comece com um cc / bcc quando a mensagem é
// principalmente uma cópia de auditoria.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

A mesma superfície fluente se aplica independente de com qual
ponto de entrada você começa.

### Teste contra `Mail::fake()`, não contra o transporte vinculado

`Mail::fake()` instala um transporte de captura local ao
processo pela duração da guarda RAII, e restaura o que quer
que estivesse vinculado antes. Testes que o usam não precisam
limpar globais em toda entrada/saída - a semântica de drop
cuida disso. Combine `#[serial_test::serial]` com
`Mail::fake()` para testes que mutam o transporte global;
testes concorrentes se atropelariam um ao outro do contrário.

## Próximos passos

- [Notificações](notifications.md) - `Notify::send` se
  espalha via fan-out pelos canais mail, database, e webpush;
  `#[derive(NotificationMailable)]` é o atalho guiado por
  macro sobre a trait `Mailable`
- [Filas](queues.md) - o envelope durável sobre o qual
  `Mail::queue` e `Mail::later` viajam
- [Eventos](events.md) - escutando `MessageSending` /
  `MessageSent` mais o modelo mais amplo de dispatcher
- [Testes](testing.md) - `Mail::fake()` ao lado das outras
  guardas `*::fake()`
- [Configuração](configuration.md) - registro de config
  tipada para credenciais de serviço

## Referência

- Trait: `suprnova::mail::Mailable`
- Facade: `suprnova::mail::Mail`
- Bootstrap: `suprnova::mail::boot::bootstrap_from_env()`
- Transportes: `LogMailTransport`, `InMemoryMailTransport`, `SmtpMailTransport`, `PostmarkMailTransport`, `SesMailTransport`, `SendGridMailTransport`, `MailgunMailTransport`, `ResendMailTransport`
- Job de fila: `suprnova::mail::SendMailJob`
- Guarda de teste: `suprnova::mail::MailFake`
- Helper de telemetria: `suprnova::mail::dispatch_with_telemetry`
