# Fluxos de autenticação

`suprnova::auth_flows` é a camada de ciclo de vida sobre a
[autenticação](authentication.md). Enquanto `auth::*` responde "quem é
esta solicitação", `auth_flows::*` cobre prova de caixa postal,
recuperação de senha, bloqueio de conta e desafios TOTP do framework.

Cinco superfícies são distribuídas no namespace:

- `EmailVerification` cria e consome `auth_flow_tokens` do framework,
  envia email pela facade [`Mail`](mail.md) e marca como verificado o
  dono autenticado do token pelo `UserProvider` configurado.
- `PasswordReset` delega a emissão e prova do token, mutação de senha,
  rotação de época de autenticação e revogação de sessão ao engine
  Magnetar instalado. O framework é dono do email e dos eventos de ciclo
  de vida.
- `BruteForce` e `LoginThrottleMiddleware` delegam o estado de bloqueio
  de conta ao engine Magnetar instalado.
- `TwoFactor` é a facade TOTP pertencente ao framework sobre
  `two_factor_credentials`. Ela fornece inscrição, confirmação,
  verificação, códigos de recuperação, rotação de segredo, promoção de
  desafio e proteção contra replay de timestep.
- `remember_me` reexporta o módulo de remember legado do framework para
  compatibilidade de namespace. Quando o Magnetar está instalado, os
  fluxos normais de remember em `Auth` e `SessionMiddleware` usam
  credenciais do Magnetar em vez disso.

Dois middleware de gate de rota são distribuídos no mesmo namespace:

- `EnsureEmailVerifiedMiddleware` compõe depois de `AuthMiddleware` e
  bloqueia rotas por `email_verified_at`.
- `TwoFactorChallengeMiddleware` compõe antes de `AuthMiddleware` e
  redireciona uma sessão com desafio TOTP do framework pendente ao
  formulário de desafio.

Mensagens transacionais sempre usam a facade [`Mail`](mail.md) do
framework. O Magnetar fornece engines de segurança e contratos de
armazenamento; não instala um segundo transporte de email da aplicação.

### Onde o estado vive

Tokens de verificação de email ficam na tabela `auth_flow_tokens` do
framework e o timestamp de verificação é gravado pelo `UserProvider`
configurado. A verificação é vinculada ao ator: o usuário autenticado
atual deve ser dono do token.

Tokens de redefinição de senha, credenciais de senha, linhas de lockout,
sessões opacas, credenciais de remember, cerimônias de passkey, cerimônias
OAuth e épocas de autenticação pertencem ao engine de host Magnetar
instalado. A redefinição de senha, o magic link e a conclusão de email
verificado por OAuth compartilham o limite atômico de primeira prova de
email do Magnetar para retomar contas não verificadas.

A facade pública `TwoFactor` deste capítulo mantém seu schema
`two_factor_credentials` pertencente ao framework. O Magnetar também
tem um engine de fator usado pelos fluxos integrados de senha, magic
link, passkey, OAuth e sessão. Não presuma que os dois armazenamentos são
intercambiáveis: use uma superfície de inscrição consistentemente para
uma determinada aplicação.

O Suprnova continua dono do middleware HTTP, cookies, email de saída,
eventos e da ponte `UserProvider`. O código da aplicação usa facades do
framework em vez de chamar engines de armazenamento diretamente.

## Semântica de falha entre fluxos

Toda facade segue uma regra de ordenação: a mudança de estado durável
confirma primeiro; depois, efeitos colaterais de notificação disparam. Um
panic de listener, uma falha transitória no transporte de email ou um
erro de dispatcher depois da mutação não pode revertê-la.

- `EmailVerification::verify` exige o dono autenticado do token, consome
  o token e marca o usuário verificado antes de disparar
  `EmailVerified`.
- `PasswordReset::complete` primeiro confirma a transação de redefinição
  de senha do Magnetar. A transação consome o token, aplica a política de
  primeira prova ou de conta verificada, avança a época de autenticação e
  revoga sessões e credenciais de remember. O email e os eventos do
  framework executam depois.
- `BruteForce::unlock_account` confirma o desbloqueio antes de disparar
  `AccountUnlocked`.
- `TwoFactor::confirm` grava `confirmed_at` antes de disparar
  `TwoFactorEnrolled`; `TwoFactor::disable` exclui a linha antes de
  disparar `TwoFactorDisabled`; `TwoFactor::complete_challenge` promove
  pendente → autenticado antes de despachar o par padrão
  `auth::Login` + `auth::Authenticated`, seguido por
  `TwoFactorChallenged`.

Um listener que precisa de durabilidade deve colocar seu trabalho em
buffer (enfileirar um job a partir do corpo do listener); a própria
facade nunca repete a tentativa.

## Inicialização

Inicialize o Magnetar depois de `DB::init` e depois que `APP_KEY` tiver
inicializado `Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` cria o schema de autenticação padrão, a menos que as
migrações sejam desabilitadas, e então instala adaptadores de
senha/sessão e passkey atomicamente. Chamá-lo uma segunda vez retorna um
erro. Testes que precisam de uma instalação global do processo devem usar
um binário de teste de integração dedicado porque um engine instalado não
pode ser substituído.

### Verificação de email

A verificação de email exige:

1. Um `UserProvider` registrado que possa recuperar usuários por email e
   marcar o timestamp de verificação.
2. `MustVerifyEmail` no tipo de usuário da aplicação.
3. Uma coluna anulável `email_verified_at`.
4. A tabela `auth_flow_tokens` do framework.

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

O handler de verificação deve executar dentro de um escopo de sessão
autenticado. Um token válido de outro usuário é rejeitado sem ser
consumido.

### Redefinição de senha e lockout

A redefinição de senha e `BruteForce` exigem o engine de senha Magnetar
instalado. `MagnetarConfig::lockout_config` aceita
`magnetar::password::lockout::LockoutConfig`. A política padrão habilita
lockout depois de cinco tentativas falhas por 15 minutos, retém linhas de
auditoria por sete dias e falha fechada quando o backend de lockout está
indisponível.

Uma redefinição de senha normaliza um endereço desconhecido para `Ok(())`
somente depois que as verificações do limitador de abuso, da configuração de
email, do engine e do armazenamento forem bem-sucedidas. Os caminhos de contas
conhecidas e desconhecidas ainda podem diferir em falhas e tempo de execução.
A conclusão usa o armazenamento atômico de primeira prova de email e retorna um
`PasswordResetOutcome` para chamadores que precisam de estado explícito da
revogação de sessão ou remember.

### Registrando as migrações de 2FA

O framework fornece o schema; sua aplicação opta por ele listando ambas
as migrações em seu próprio migrator:

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... suas próprias migrações ...

            // Cria `two_factor_credentials`.
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // Adiciona `last_used_timestep` para proteção contra replay de TOTP.
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

Ambas são idempotentes contra um banco de dados já migrado (a v1 usa
`CREATE TABLE IF NOT EXISTS`; a v2 adiciona uma coluna). Executar
`suprnova migrate` novamente contra um banco de produção que já tem o
schema é um no-op.

### Ambiente

Os mailables transacionais leem duas variáveis de ambiente no momento do
envio:

| Var | Padrão | Usada para |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Branding do assunto e o rótulo de emissor `otpauth://` que os apps autenticadores exibem. |
| `MAIL_FROM` | nenhum - **gera erro quando não definida** | `From` de envelope em toda mensagem de saída. Defina-o para um domínio de remetente verificado. |

`MAIL_FROM` deliberadamente não tem padrão. Usar por padrão um
placeholder como `noreply@example.com` quebraria silenciosamente DMARC /
SPF em produção e enviaria de um domínio que o operador não controla;
portanto, a facade falha fechada. `EmailVerification::send_link` e
`PasswordReset::send_link` expõem o erro como `Err`;
`PasswordReset::complete` registra por `tracing::warn!` e continua (a
mudança de senha já foi confirmada, portanto o caminho de notificação não
pode revertê-la).

As aplicações também definem `APP_URL` para que os controladores possam
derivar a URL base usada em chamadas `send_link`; a própria facade do
framework recebe a URL base como parâmetro.

O driver de email é configurado separadamente por `MAIL_DRIVER` - veja a
documentação de [Email](mail.md).

## Verificação de email

`EmailVerification` cunha, verifica, e consome tokens de verificação
contra a tabela `auth_flow_tokens`, e marca o usuário como verificado
através do provedor configurado. Quatro operações cobrem o ciclo de
vida:

| Método | Assinatura | Notas |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Cunha + envia, dado um usuário já em mãos. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Normaliza um resultado de provedor desconhecido para `Ok(())`; falhas no armazenamento de tokens e no email ainda retornam `Err`, e o tempo de execução não é equalizado. |
| `check` | `check(token: &str) -> Result<bool>` | Não consome - seguro chamar em uma landing page. |
| `verify` | `verify(token: &str) -> Result<String>` | Vinculado ao ator e de uso único: o usuário autenticado deve ser dono do token; em caso de sucesso, ele é consumido, marca esse usuário como verificado e retorna seu ID. |

```rust
use suprnova::auth_flows::EmailVerification;

// Depois de um signup recém-feito, com o usuário recém-criado em mãos:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Verificação opcional de landing-page - não consome, então um
// refresh de página não queima o token.
let valid: bool = EmailVerification::check(&token_str).await?;

// O handler de click-through executa protegido por autenticação. `verify`
// consome o token somente quando `Auth::id()` corresponde ao seu dono.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` dispara `EmailVerified` em caso de sucesso - listeners são o
lugar certo para desbloquear funcionalidade adicional (email de
boas-vindas, follows padrão, CTA de "complete seu perfil") sem
acoplá-los ao handler de verificação. O evento carrega o id de usuário
do provedor.

### O endpoint de resend (anti-enumeração)

`resend` recebe apenas o email e procura o usuário pelo provedor ativo. Um
resultado de provedor desconhecido é normalizado para `Ok(())`. Para uma conta
conhecida, a facade cunha um token e envia o email.
`EmailVerification::resend` também normaliza um resultado de provedor
desconhecido para `Ok(())`; ela não garante tempo idêntico nem comportamento
idêntico quando o armazenamento do token ou a entrega do email falha. Um handler
ainda pode retornar uma mensagem neutra depois de qualquer resultado bem-sucedido:

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` faz a busca e normaliza um endereço desconhecido para `Ok(())`.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` e `resend` constroem a URL como
`{base_url}?token={plaintext_token}`. Uma barra final em `base_url` é
removida antes de a query string ser anexada, então
`https://app.example.com/verify/` e `https://app.example.com/verify`
produzem, ambas, uma URL limpa.

O handler de click-through deve executar protegido por `AuthMiddleware`.
Ele extrai o token da query string e chama `verify`:

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;

    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` verifica `Auth::id()` contra o dono do token antes do consumo.
Um token que pertence a outra conta retorna a mesma resposta de token
inválido e permanece sem uso. Em caso de sucesso, o provedor marca o
dono autenticado como verificado e a facade dispara `EmailVerified`.

### Rotas só-para-verificados: `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` bloqueia rotas com base no
`email_verified_at` do usuário autenticado. Componha-o depois de
`AuthMiddleware`, e a chain bloqueia qualquer solicitação cujo usuário
ainda não tenha completado a etapa de verify.

A escolha entre **403 JSON** e **302 redirect HTML** é feita no
momento do registro da rota, via o construtor - não há sniffing de
conteúdo de solicitação, seguindo o mesmo padrão de
`AuthMiddleware::new` / `AuthMiddleware::redirect_to`:

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// Superfície de API - 403 com um corpo JSON.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Superfície web - 302 (ou 409 + X-Inertia-Location para visitas Inertia).
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

Se nenhum usuário está autenticado, o middleware cai no mesmo ramo de
resposta que "autenticado mas não verificado" - correspondendo ao
formato `! $request->user() || ! hasVerifiedEmail()` do Laravel.
Componha `AuthMiddleware` primeiro quando você quiser um `401`
separado para solicitações não autenticadas.

Para ramificação dentro do handler (por exemplo, renderizar
condicionalmente um CTA de "por favor verifique" sem redirecionar),
carregue o usuário tipado através do guard de sessão e leia o método
da trait:

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // ramifique com base nisso
}
```

## Redefinição de senha

`PasswordReset` tem quatro operações:

| Método | Assinatura | Notas |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Retorna `Ok(())` para um endereço desconhecido depois que as verificações do limitador de abuso, da configuração de email, do engine e do armazenamento forem bem-sucedidas; outras falhas ainda retornam `Err`. |
| `check` | `check(token: &str) -> Result<bool>` | Validação não consumidora pelo engine Magnetar instalado. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Consome atomicamente o token, aplica a política de primeira prova, rotaciona credenciais, revoga sessões e estado de remember e retorna o ID do usuário. |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | Executa a mesma transação e retorna as contagens de revogação confirmadas. |

```rust
use suprnova::auth_flows::PasswordReset;

// A partir do formulário "esqueci a senha". Um endereço desconhecido retorna `Ok(())`
// depois que as verificações de pré-requisito forem bem-sucedidas; erros de configuração e de backend ainda surgem.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Verificação opcional de landing-page antes de renderizar o formulário de senha nova.
let valid: bool = PasswordReset::check(&token).await?;

// O handler de click-through, depois de o usuário enviar uma senha
// nova: consome o token + rotaciona a senha, retornando o id do usuário.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` passa a senha em texto puro por `SecretString`; o Magnetar
faz o hash dentro do engine de credenciais. Não faça hash previamente.
Uma senha vazia ou somente com espaços retorna HTTP 400 antes de o engine
ser chamado.

### Comportamento de anti-enumeração limitado

`PasswordReset::send_link` retorna `Ok(())` para um endereço desconhecido
somente depois que as verificações do limitador de abuso, da configuração de
email, do engine e do armazenamento forem bem-sucedidas. Falhas de configuração,
limitador, armazenamento e email ainda retornam `Err`. O controlador de dogfood
fornece às solicitações bem-sucedidas de contas conhecidas e desconhecidas o
mesmo status HTTP e corpo, mas a implementação não equaliza o tempo de
execução.

### Efeitos colaterais de `complete`

O Magnetar confirma a redefinição de senha em uma transação:

1. Consome o token de redefinição de uso único.
2. Aplica a política de primeira prova de email quando a conta ainda não
   está verificada.
3. Faz hash e substitui a senha.
4. Avança a época de autenticação.
5. Revoga sessões opacas e credenciais de remember antigas.
6. Remove credenciais provisórias quando esta redefinição é a primeira
   prova de caixa postal da conta.

Depois do commit, o framework envia `PasswordChangedMail` e despacha
`PasswordResetCompleted`. Uma falha de email ou listener não pode
reverter a redefinição.

Em uma conta já verificada, a redefinição preserva passkeys, contas
vinculadas e inscrição confirmada de dois fatores legítimas. Em uma conta
não verificada ocupada indevidamente, a primeira prova remove credenciais
provisórias para que o registrante anterior não possa reter acesso.

## Proteção contra força bruta

A camada de força bruta tem duas partes: a facade `BruteForce`, que
registra e consulta o estado de bloqueio, e o `LoginThrottleMiddleware`,
que faz short-circuit na camada HTTP antes de o handler ser invocado.

### A facade `BruteForce`

Chame `record_failed_attempt` a partir do ramo de auth-falhou do seu
handler de login, e `reset_attempts` a partir do ramo de sucesso:

```rust
use suprnova::auth_flows::BruteForce;

// No caminho de auth falhou:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Opcionalmente exponha uma resposta customizada. O middleware vai
    // fazer isso para você na *próxima* solicitação - veja abaixo.
}

// No caminho de sucesso:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` retorna o `LockoutStatus` atualizado
(`is_locked`, `failed_attempts`, e `locked_until` quando bloqueado).
Passe o `ip` opcional para logs de auditoria; passe `None` se o seu
transporte não expõe um IP de cliente de forma limpa.

Duas operações adicionais:

```rust
// Somente leitura - seguro em emails sem histórico.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Desbloqueio de admin / forçado. Dispara `AccountUnlocked` só em uma
// transição de estado real (um unlock no-op em uma conta já
// desbloqueada não dispara).
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` retorna `true` quando a conta estava bloqueada no
momento da chamada, `false` caso contrário. O evento `AccountUnlocked`
dispara só em `true` - um retorno `false` é o no-op que ele é, não um
evento de auditoria.

### `LoginThrottleMiddleware`

O middleware lê o estado de bloqueio para o email que uma solicitação
está visando, e faz short-circuit com `429 Too Many Requests` quando a
conta está bloqueada. O handler de login nunca é invocado, então uma
conta bloqueada nem chega a tentar uma verificação de credenciais:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// O extrator de email é uma closure síncrona sobre `&Request`. Ler
// corpo JSON/form é assíncrono e consome `Request`, então a closure
// não consegue ler o corpo - puxe de um header, query string, ou
// route param em vez disso.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Superfícies práticas de extração:

- Um header (`X-Login-Email`), definido por um pré-processador
  anterior - o padrão usado na app de dogfood.
- Um parâmetro de query string (`?email=…`).
- Um parâmetro de rota (`/login/{email}`).

Retornar `None` do extrator é o sinal explícito de "não tenho nada
para verificar" - o middleware deixa a solicitação passar sem
alteração. Isso torna o middleware seguro para instalar em rotas que
ocasionalmente veem tráfego anônimo (por exemplo, o mesmo endpoint
`POST /login` que também trata uma sub-ação sem email de "solicitar
redefinição de senha").

No bloqueio, o middleware retorna:

- Status `429 Too Many Requests`.
- Header `Retry-After` - segundos, computado a partir do
  `locked_until` do bloqueio via `LockoutStatus::retry_after_seconds`.
  Cai para `900` (15 minutos, período padrão de lockout do Magnetar)
  se o timestamp estiver, de algum jeito, ausente.
- Corpo: `"Account locked due to too many failed login attempts. Try
  again later."`

### Erros de backend (falha fechada por padrão)

Se `get_lockout_status` retornar um erro, `LoginThrottleMiddleware` registra a
falha e, por padrão, retorna HTTP `503 Service Unavailable` com `Retry-After: 1`
sem invocar o handler de login. Para manter o login disponível durante uma
indisponibilidade do backend de lockout, opte explicitamente por
`.on_backend_error(BackendErrorPolicy::FailOpen)`; somente essa política encaminha
a solicitação ao handler.

### Combinando com `RateLimitMiddleware`

`LoginThrottleMiddleware` é por conta - ele bloqueia um único email
quando o limiar é cruzado. Para cotas por IP, combine-o com
[`RateLimitMiddleware`](rate-limiting.md). Os dois se compõem
naturalmente:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Juntos, eles cobrem os formatos realistas de credential stuffing:
distribuído (um email × muitos IPs) é trabalho do rate limit;
concentrado (muitas tentativas × um email) é trabalho do middleware de
throttle.

### Configuração

`MagnetarConfig` aceita uma `LockoutConfig`. O padrão é cinco tentativas
falhas, um período de contagem e lockout de 15 minutos, retenção de
tentativas por sete dias e `BackendErrorPolicy::FailClosed`:

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

Use `LockoutConfig::disabled()` somente quando outro controle de
identidade fail-closed substituir o lockout de conta.

## Dois fatores (TOTP)

`TwoFactor` cobre 2FA baseado em TOTP - o tipo que pareia com qualquer
app autenticador compatível com o padrão (Google Authenticator,
1Password, Bitwarden, Authy). O fluxo é cadastro → confirmação →
verificação contínua, mais códigos de recuperação de uso único para
quando o usuário perde o dispositivo, mais o fluxo de desafio que
costura tudo isso no ciclo de vida do login.

### A trait `TwoFactorUser`

O framework não consegue alcançar o armazenamento de usuário da sua
aplicação, então os chamadores implementam uma pequena trait para
fazer a ponte do model de usuário deles até a facade de 2FA:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` é uma chave de armazenamento opaca. Pode ser um ID numérico da
aplicação renderizado como texto, um UUID ou um `UserId` do Magnetar. A
tabela TOTP do framework não tem chave estrangeira para a tabela de
usuário da aplicação.

`email` é incorporado ao segmento `account_name` da URL `otpauth://`
para que o app autenticador exiba um rótulo de conta reconhecível.

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### Armazenamento

O estado de 2FA vive na tabela `two_factor_credentials`, de
propriedade do framework. Secrets e códigos de recuperação são
criptografados em repouso com `crate::crypto::Crypt::encrypt_string`,
que exige uma `EncryptionKey` global ao processo. Apps optam pelo
schema listando as duas migrações no `Migrator::migrations()` delas -
veja [Inicialização](#inicialização).

### Cadastre, confirme, verifique

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Cadastro: gera um secret novo + 10 códigos de recuperação,
//    persiste-os criptografados, retorna tudo o que é necessário para
//    renderizar o QR code.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - deep link `otpauth://totp/...`
// response.qr_code_svg - <svg> envolvendo um PNG em base64, incorpore inline
// response.recovery_codes - Vec<String>, 10 códigos em texto puro - mostre UMA VEZ

// 2. Confirme: o usuário abre o app autenticador e digita o código
//    de 6 dígitos. `confirm` o valida e estampa `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// dispara `TwoFactorEnrolled`

// 3. Em logins subsequentes, bloqueie a sessão via `verify`:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` retorna os códigos de recuperação em texto puro
**exatamente uma vez**. Não há API para recuperá-los depois - a coluna
criptografada é unidirecional a partir deste ponto. Mostre-os na
página de sucesso do cadastro, encoraje o usuário a salvá-los, e não
armazene o texto puro em nenhum outro lugar.

`enroll` se recusa a sobrescrever um cadastro **confirmado** - ele
retorna um `409` para empurrar o chamador em direção a `re_enroll`, que
exige prova de posse. Recadastrar em uma linha não confirmada
(pendente) é permitido: o cadastro anterior nunca se tornou
autoritativo.

### Proteção contra replay

`verify` escreve o timestep TOTP atual em `last_used_timestep` em caso
de sucesso. Verifies subsequentes em que `current_timestep <=
last_used_timestep` são rejeitados mesmo quando o código em si é
estruturalmente válido, derrotando um replay de código roubado dentro
da janela de 30 segundos.

A reivindicação do timestep é atômica. O stamp acontece via um
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep <
:current` condicional, e o verify só tem sucesso quando o statement
afeta exatamente uma linha. Dois verifies concorrentes no mesmo
timestep não podem os dois vencer: o primeiro vira a coluna, o
predicado do segundo não corresponde mais, e o segundo é tratado como
um replay. Um read-modify-write simples seria uma corrida TOCTOU -
ambos os verifies leem a linha pré-stamp, ambos validam o mesmo
código, ambos estampam, ambos têm sucesso. Corredores concorrentes
também são contados como tentativas falhas, para que o contador de
força bruta os registre.

### Códigos de recuperação

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

Uso único: um código correspondente é removido da linha antes de a
chamada retornar, então uma segunda tentativa contra o mesmo código
retorna `false`. Os códigos têm 12 dígitos decimais no formato
`NNNNNN-NNNNNN` (~40 bits de entropia cada, correspondendo ao formato
do Fortify do Laravel).

`consume_recovery_code` só aceita códigos quando o 2FA está totalmente
confirmado - ele faz short-circuit para `Ok(false)` enquanto
`confirmed_at` é NULL. Sem esse gate, um atacante que tivesse disparado
o cadastro em uma conta vítima (ou qualquer fluxo que crie a linha sem
confirmar) poderia se autenticar usando só um código de recuperação
novo, contornando o TOTP inteiramente. O contrato é simétrico com a
salvaguarda de `verify` de "só cadastro confirmado".

### Rotacionando códigos de recuperação e secrets

Quando um usuário esgota os códigos de recuperação, ou quer
rotacioná-los depois de uma suspeita de comprometimento:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` precisa validar como um código TOTP atual ou um código de
recuperação não usado. Sem a verificação de proof, um atacante que
sequestrou a sessão poderia destruir silenciosamente os códigos de
recuperação do usuário legítimo (negação de serviço contra a
recuperação de conta). Os códigos novos substituem o conjunto
persistido; o secret existente e `confirmed_at` são preservados,
então o app autenticador do usuário continua funcionando sem
repareamento. Erros:

- `400` - não existe cadastro confirmado; chame `enroll`/`confirm`
  primeiro.
- `401` - `proof` não valida como um código TOTP nem como um código
  de recuperação não usado.
- `429` - a conta está bloqueada por throttling de força bruta.

Para rotacionar o **secret** (reparear para um dispositivo novo) sem
desabilitar o 2FA primeiro:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

Mesmo modelo de proof de `regenerate_recovery_codes`. A linha é
reescrita com um secret novo + 10 códigos de recuperação novos;
`confirmed_at` reseta para NULL, então o usuário precisa fazer
`confirm` com um código do novo autenticador antes de o 2FA estar
ativo de novo.

### Desativar

```rust
TwoFactor::disable(&user_2fa).await?;
// dispara `TwoFactorDisabled` só se uma linha foi removida
```

Idempotente: um disable em um usuário que nunca se cadastrou não é um
erro. O evento `TwoFactorDisabled` dispara só em uma transição de
estado real, então listeners de auditoria veem uma entrada por disable
real, em vez de uma por clique em um botão no-op.

### Fluxo de desafio (bloqueando o login até o segundo fator)

As primitivas enroll / confirm / verify são os blocos de construção;
o **fluxo de desafio** costura tudo isso no ciclo de vida do login,
para que um usuário com 2FA habilitado não consiga alcançar páginas
protegidas só com a senha.

O fluxo:

1. O login por senha resolve um usuário.
2. Se `TwoFactor::is_enabled_by_id(&user_id)` retornar `true`, o
   handler de login chama `TwoFactor::start_challenge(user_id,
   remember)` - isso guarda o user-id como **pending** na sessão,
   limpa o slot totalmente autenticado, revoga qualquer cookie de
   remember-me emitido por `Auth::attempt`, e lembra se o usuário
   optou por remember-me, para que o cookie possa ser reemitido
   depois de o desafio se completar. `Auth::id()` retorna `None` a
   partir deste ponto até o desafio se completar.
3. O handler redireciona para uma rota `/two-factor-challenge` que
   mostra o formulário de código.
4. O handler POST do desafio chama
   `TwoFactor::complete_challenge(code)` - verifica o código (TOTP
   **ou** um código de recuperação não usado, correspondendo ao
   controlador de desafio do Fortify), promove pending → authed,
   rotaciona o id de sessão (derrotando a fixação de
   sessão) e o token CSRF, reemite o cookie de remember-me quando o
   usuário optou por ele, e despacha os eventos de ciclo de vida
   padrão `auth::Login` + `auth::Authenticated`, mais o
   `TwoFactorChallenged` específico de 2FA.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Rebaixa para "pending": slot de auth limpo, pending definido,
                // cookie de remember-me revogado. Passa adiante a flag remember
                // do formulário para que `complete_challenge` possa reemitir
                // o cookie em caso de sucesso.
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // O id de sessão + CSRF rotacionaram; o remember-me foi reemitido
    // se o formulário de login original o definiu. Listeners
    // enganchados em `auth::Login` / `auth::Authenticated` viram um
    // login normal.
    redirect!("/dashboard").into()
}
```

`complete_challenge` rotaciona o id de sessão e o token CSRF como
parte da promoção para authed. Isso fecha o ataque clássico de
fixação de sessão, em que um atacante planta um id de sessão conhecido
em uma vítima antes de ela logar - depois da rotação, o id plantado
está morto e só o id recém-gerado carrega o estado autenticado. O
contrato corresponde a `Auth::login_id` / `Auth::login_using_id`,
então logins com 2FA são indistinguíveis de logins sem 2FA em termos
de estado de sessão e observabilidade de listener.

Bloqueie todo grupo de rota protegido com `TwoFactorChallengeMiddleware`
**antes** de `AuthMiddleware`, para que uma sessão pending seja
redirecionada para a página de desafio, em vez da página de login:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

A própria página de desafio (o GET que renderiza o formulário, o POST
que chama `complete_challenge`) NÃO deve instalar
`TwoFactorChallengeMiddleware` - ela é o destino. O handler POST
tipicamente também verifica `TwoFactor::pending_user_id().is_some()`
de antemão, para que um link obsoleto não alcance a lógica de verify
com uma sessão vazia.

`TwoFactor::cancel_challenge()` limpa os dois slots pending sem
autenticar ninguém - conecte-o a um link de "voltar para o login" na
página de desafio.

**Fallback de código de recuperação.** `complete_challenge(code)`
tenta o caminho TOTP primeiro e cai para consumir um código de
recuperação, então um usuário que perdeu o autenticador ainda
consegue entrar. Cada código de recuperação é de uso único.

**Ligação com força bruta.** Códigos de desafio falhos alimentam o
contador de força bruta por conta através de
`BruteForce::record_failed_attempt`, do mesmo jeito que o
`TwoFactor::verify` nu faz. Um atacante forçando o formulário de
desafio vai disparar `AccountLocked` depois do limiar configurado. Uma
única submissão errada conta como **uma** tentativa falha, mesmo que
`complete_challenge` tente os caminhos de TOTP e de código de
recuperação internamente - os núcleos de validação silenciosa pulam o
contador de força bruta, para que a camada externa registre a
tentativa canônica exatamente uma vez.

**Gate de bloqueio.** `complete_challenge` verifica
`BruteForce::is_locked` de antemão e retorna `429 Too Many Requests`
se a conta já estiver bloqueada - mesmo quando o código enviado está
correto. Sem esse gate embutido no método, um atacante que disparou o
bloqueio ainda poderia entrar submetendo o código certo na próxima
solicitação: o contador de força bruta é indexado pelo email do
usuário, mas o próprio `verify` não o consulta. O `LoginThrottleMiddleware`
do caminho de senha impõe a mesma restrição na camada de rota;
compô-lo na frente da rota POST de desafio é adequado - os dois gates
são idempotentes.

**Evento de falha.** `complete_challenge` despacha
`TwoFactorChallengeFailed { user_id }` em um código errado (ou uma
conta bloqueada), distinto do `auth::Failed` do caminho de senha.

Listeners que observam “usuário tentou 2FA e falhou” assinam o novo
evento; listeners que observam “a senha não autenticou” permanecem em
`auth::Failed`. As duas superfícies são mantidas separadas para que um
erro de digitação no 2FA não pareça uma falha de senha nos pipelines de
auditoria.

### Por que Suprnova diverge
O `user_id` TOTP do framework é uma `String`. Um tipo fixo `i64`, UUID
ou identificador do Magnetar vincularia a facade reutilizável a um único
schema de aplicação. O limite de string permite que uma aplicação escolha
qualquer identificador estável ao custo de uma conversão no local da
chamada.

O gate de fator integrado do Magnetar é separado desta facade mantida. A
separação preserva a compatibilidade de aplicações que usam
`two_factor_credentials`, mas aplicações não devem inscrever a mesma
conta nos dois armazenamentos.

## Remember-me

`suprnova::auth_flows::remember_me` reexporta o módulo legado
`suprnova::auth::remember` para compatibilidade.

Quando o Magnetar está instalado, `Auth::attempt(..., true)`,
`Auth::issue_remember_cookie` e a hidratação por `SessionMiddleware` usam
credenciais de remember associadas à finalidade do Magnetar. O Magnetar
armazena digests de verificadores, verifica a época de autenticação,
rotaciona credenciais em uso bem-sucedido, revoga-as com a sessão do
usuário e informa anomalias de replay ou de credencial malformada sem
expor o segredo.

O cookie voltado ao navegador continua sendo do framework. Ele é
criptografado com o nome lógico `remember_me`, segue
`SESSION_COOKIE_PREFIX` e é limpo antes da revogação do backend para que
uma falha de armazenamento não deixe o navegador enviar a credencial
antiga.

A implementação legada por linha de banco permanece disponível quando
nenhum engine Magnetar está instalado. Novas aplicações devem inicializar
o Magnetar e tratar a reexportação legada como uma superfície de
transição.

## Eventos

Nove eventos disparam entre os fluxos, um por transição de estado de
segurança:

| Evento | Disparado por | Carrega |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` em caso de sucesso | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` em caso de sucesso - anti-enumeração silenciosa para emails ausentes | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` em caso de sucesso | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` na transição desbloqueada → bloqueada | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` quando um desbloqueio real ocorreu | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` em caso de sucesso | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` promoveu pending → authed | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` rejeitou um código errado ou recusou uma conta bloqueada | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` quando uma linha foi de fato removida | `user_id: String` |

Todo evento é `Debug + Clone + 'static`, não carrega dados sensíveis
(sem tokens em texto puro, sem IPs), e usa identificadores em string,
para que listeners possam serializá-los através de fronteiras de task
sem vazar informação de tipo do backend de armazenamento de usuário.

### Escutando

Inscreva-se via a API de evento padrão - a mesma superfície de
qualquer outro evento in-process:

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ... notificação no Slack, append na tabela de auditoria, etc.
        Ok(())
    }
}

// Em bootstrap.rs:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Listeners rodam no runtime do Tokio e são despachados na ordem de
registro. Veja o capítulo [Eventos](events.md) para a superfície
completa.

## Testes

Três fakes cobrem a superfície de auth-flows, e eles se compõem.

### `Mail::fake()`

Instala um transporte de captura local ao processo. Todo send durante
o tempo de vida da guarda cai em um buffer em memória, em vez de
sair:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... dirija o fluxo ...
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` expõe `assert_sent`, `assert_not_sent`, `assert_sent_count`,
mais os acessores brutos `captured()` e `count()`. Quando a guarda sai
de escopo, o transporte previamente vinculado é restaurado - testes
que intercalam fakes com binding explícito de transporte não vazam
estado.

### `EventFacade::fake()`

O mesmo formato, mas para eventos:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... dirija o fluxo ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```
O fake registra os eventos despachados sem invocar listeners, portanto um listener que converse com um serviço externo não será executado durante o teste. O método complementar `assert_not_dispatched::<E>(pred)` verifica a condição negativa; `dispatched_count::<E>(pred)` retorna a contagem bruta para asserções mais granulares.

### Testes de integração para verificação de email e redefinição de senha

Testes de verificação de email criam `auth_flow_tokens`, registram um
`UserProvider`, estabelecem o dono autenticado do token, definem
`MAIL_FROM` e exercitam a facade sob `Mail::fake()`.

Testes de redefinição de senha instalam um adaptador de teste
`MagnetarPasswordAuthEngine` e verificam emissão, check não consumidor,
conclusão atômica, revogação de sessão e comportamento de uso único.

Exemplos de fonte canônicos são:

- `framework/tests/email_verify.rs` para verificação vinculada ao ator e
  tokens de uso único.
- `framework/tests/password_reset.rs` para delegação ao Magnetar e
  resultados de conclusão.
- `framework/tests/magnetar_default_engine.rs` para configuração do
  engine padrão real.
- `framework/tests/brute_force.rs` para o ciclo de vida de lockout.
- `framework/tests/two_factor_challenge_flow.rs` para o fluxo de desafio
  TOTP mantido pelo framework.
- `framework/tests/magnetar_remember_middleware.rs` para rotação de
  remember e vinculação de sessão dupla.

A instalação global de Magnetar para o processo é intencionalmente de uma
só vez. Coloque testes que precisam de engines diferentes em binários de
teste de integração separados ou instale um adaptador de teste uma vez
para todo o binário.

## Referência

| Símbolo | Propósito |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check` e `verify` vinculado ao ator; `verify` retorna o ID do usuário. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` para 403 JSON e `redirect_to(path)` para redirecionamentos de navegador ou Inertia. |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` e `complete_with_outcome` apoiados no Magnetar. |
| `suprnova::MustVerifyEmail` | Contrato de usuário da aplicação para a facade de verificação do framework. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | Definição de tabela SeaORM para tokens de verificação do framework. |
| `suprnova::auth_flows::BruteForce` | Facade de lockout de conta apoiada no Magnetar. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP que retorna 429 antes do handler quando a conta está bloqueada. |
| `suprnova::auth_flows::TwoFactor` | Facade TOTP de inscrição, verificação, recuperação e desafio mantida pelo framework. |
| `suprnova::auth_flows::TwoFactorUser` | Ponte de usuário da aplicação para a facade TOTP do framework. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | Gate para sessões esperando o desafio TOTP do framework. |
| `suprnova::auth_flows::remember_me` | Reexportação de compatibilidade do módulo de remember legado do framework. |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | Configuração padrão do engine Magnetar e instalação única. |
| `suprnova::auth_flows::events::*` | Eventos do ciclo de vida da autenticação. |

## Próximos passos

- [Autenticação](authentication.md) - guards, provedores, a facade
  `Auth`, `AuthMiddleware`.
- [Correio](mail.md) - a camada de transporte através da qual as
  chamadas de `send_link` despacham.
- [Eventos](events.md) - registrando listeners para os nove eventos de
  auth-flow.
- [Limitação de taxa](rate-limiting.md) - combine
  `RateLimitMiddleware::ip_based` com `LoginThrottleMiddleware` para
  uma defesa em camadas.
- [Sessões](session.md) - o que `start_challenge` /
  `complete_challenge` tocam quando rotacionam o id de sessão.
