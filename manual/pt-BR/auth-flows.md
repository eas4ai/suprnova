# Fluxos de autenticação

`suprnova::auth_flows` é a camada de ciclo de vida sobre a [autenticação
de sessão](authentication.md). Enquanto `auth::*` responde "quem é essa
solicitação", `auth_flows::*` responde tudo em torno dessa pergunta -
provar que o endereço de email é real, recuperá-lo quando a senha se
perde, defendê-lo contra credential stuffing, e protegê-lo com um
segundo fator. Cinco fluxos são distribuídos sob um único namespace:

- `EmailVerification` - cunha, verifica, e consome tokens de
  verificação de uso único; `send_link` / `resend` despacham o email de
  verificação através da facade [`Mail`](mail.md), e `verify` marca o
  usuário como verificado através do provedor de usuário configurado.
- `PasswordReset` - `send_link` anti-enumeração, `check` que não
  consome, e `complete`. `complete` rotaciona a senha através do
  provedor de usuário configurado, revoga toda sessão e linha de
  remember-me do usuário, e envia uma notificação de segurança
  `PasswordChangedMail`.
- `BruteForce` + `LoginThrottleMiddleware` - estado de bloqueio apoiado
  em torii, mais um middleware HTTP que faz short-circuit com `429 Too
  Many Requests` antes de o handler de login ser invocado.
- `TwoFactor` - cadastro TOTP, confirmação, verificação, códigos de
  recuperação, rotação de secret, o fluxo de desafio completo que
  bloqueia um login por senha até o segundo fator, e proteção contra
  replay na granularidade de timestep de 30 segundos.
- `remember_me` - reexportação de `crate::auth::remember` (cookies
  persistentes de linha-de-BD + bcrypt + rotação de uso único) por
  coesão de namespace.

Dois middlewares de bloqueio de rota são distribuídos no mesmo
namespace:

- `EnsureEmailVerifiedMiddleware` - se compõe depois de
  `AuthMiddleware` para bloquear rotas com base em
  `email_verified_at`.
- `TwoFactorChallengeMiddleware` - se compõe na frente de
  `AuthMiddleware` para redirecionar uma sessão com um desafio de 2FA
  pendente para o formulário de desafio, em vez da página de login.

Toda mensagem transacional é entregue através da facade
[`Mail`](mail.md). A feature opcional `mailer` do torii é
deliberadamente desabilitada em `framework/Cargo.toml`: rodar uma
segunda pilha de mail dentro do torii dividiria a telemetria, dobraria
a superfície de configuração de transporte, e forçaria as apps a
conectar dois endereços de "from".

### Onde o estado vive

Verificação de email e redefinição de senha são **agnósticas a
provedor**. Tokens de verificação e reset vivem na tabela própria do
framework `auth_flow_tokens` (uso único, hasheada com SHA-256), e a
busca + mutação do usuário passam por qualquer
[`UserProvider`](authentication.md) que a app tenha registrado - o
mesmo provedor contra o qual `Auth::user` resolve. Não há instância
global de auth para inicializar para esses dois fluxos: uma app recém
criada com scaffold já tem `EloquentUserProvider<User>` vinculado, e é
tudo que `EmailVerification` e `PasswordReset` precisam.

O torii ainda é dono do estado de segurança para os fluxos que
genuinamente dependem dele - o contador de bloqueio de força bruta por
conta, as cerimônias de OAuth / passkey / WebAuthn, e o pool de
sessão. O Suprnova é dono das preocupações transversais em todo fluxo -
mail de saída, dispatch de evento, a tabela TOTP de 2FA, cookies de
remember-me, e o middleware HTTP. O código da aplicação só toca
`suprnova::auth_flows::*`. O Laravel dobra a superfície equivalente
dentro do Fortify; o Suprnova mantém as model traits (`MustVerifyEmail`
/ `CanResetPassword`) e o token store no framework, para que os fluxos
funcionem contra qualquer backend de usuário.

## Semântica de falha entre fluxos

Toda facade segue uma regra de ordenação: a mudança de estado durável
faz commit primeiro, depois os efeitos colaterais de notificação
disparam. Um panic de listener, uma falha transitória de transporte de
mail, ou um erro de dispatcher depois da mutação não conseguem desfazer
a mutação.

- `EmailVerification::verify` consome o token e marca o usuário como
  verificado através do provedor antes de disparar `EmailVerified`.
- `PasswordReset::complete` consome o token e rotaciona a senha através
  do provedor primeiro, depois revoga toda sessão e linha de
  remember-me do usuário (registrado em log na falha, não exposto),
  depois despacha `PasswordChangedMail` fire-and-forget, depois dispara
  `PasswordResetCompleted`.
- `BruteForce::unlock_account` faz commit do desbloqueio antes de
  disparar `AccountUnlocked`.
- `TwoFactor::confirm` estampa `confirmed_at` antes de disparar
  `TwoFactorEnrolled`; `TwoFactor::disable` deleta a linha antes de
  disparar `TwoFactorDisabled`; `TwoFactor::complete_challenge`
  promove pending → authed antes de despachar o par padrão
  `auth::Login` + `auth::Authenticated`, seguido por
  `TwoFactorChallenged`.

Um listener que precisa de durabilidade deveria bufferizar seu
trabalho (colocar um job na fila a partir do corpo do listener); a
própria facade nunca faz retry.

## Inicialização

Verificação de email e redefinição de senha são apoiadas em provedor e
**não precisam de torii**. Proteção contra força bruta e 2FA ainda
precisam de torii. Conecte o que os fluxos que você usa exigem - eles
são independentes.

### Verificação de email + redefinição de senha

Três coisas, todas as quais uma app com scaffold já tem:

1. **Um provedor de usuário que implementa a superfície de auth-flow.**
   Registre `EloquentUserProvider<User>` (o mesmo provedor contra o
   qual `Auth::user` resolve) como o binding `dyn UserProvider` em
   `bootstrap.rs::register()`. Ambas as facades resolvem o provedor
   ativo internamente; nenhuma instância é passada no call site.

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **As duas model traits no seu `User`.** `EloquentUserProvider<User>`
   só implementa os métodos de auth-flow (`retrieve_by_email` /
   `mark_email_verified` / `set_password` / `is_email_verified`)
   quando `User` implementa tanto `MustVerifyEmail` quanto
   `CanResetPassword` - os análogos do Suprnova aos contratos
   `MustVerifyEmail` / `CanResetPassword` do Laravel:

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // O valor chega já hasheado - armazene-o ao pé da letra.
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` tem um padrão que rastreia o timestamp
   (`email_verified_at().is_some()`), e `name()` tem `None` como
   padrão - sobrescreva-o para saudar os usuários pelo nome no mail.

3. **Duas colunas / tabelas no seu migrator.** A tabela `users` precisa
   de um timestamp `email_verified_at` anulável (o provedor o lê em
   `is_email_verified` e o estampa em `mark_email_verified`), e a
   tabela de uso único `auth_flow_tokens` do framework guarda os
   tokens de verificação / reset. O framework traz o `CREATE` da
   tabela de tokens; liste-a no seu migrator:

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   Adicione `email_verified_at` a `users` na sua própria migração de
   coluna (um `timestamp_with_time_zone` anulável); `NULL` significa
   não verificado, então as linhas existentes fazem backfill
   corretamente.

Tokens são de uso único e hasheados com SHA-256 em repouso - um dump
de banco de dados nunca produz um token em texto puro utilizável. Os
TTLs padrão são de **24 horas** para verificação de email e **15
minutos** para redefinição de senha.

### Força bruta + 2FA: conectando o torii

`BruteForce` / `LoginThrottleMiddleware` e `TwoFactor` são apoiados em
torii - eles precisam da instância global de torii inicializada em
`bootstrap.rs::register()`, depois de `DB::init`. (Cerimônias de
OAuth, passkeys, e WebAuthn passam pela mesma instância - veja
[Autenticação](authentication.md).)

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` é idempotente. A guarda `OnceLock` significa que a
segunda chamada é um no-op, então harnesses de teste que reentram em
`register()` por fixture não fazem migração em dobro. Para testes,
troque para `ToriiConfig::sqlite_in_memory()` - ela levanta um banco de
dados em memória com cache compartilhado que sobrevive entre runtimes:

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

### Registrando as migrações de 2FA

O framework traz o schema; a sua app opta por participar listando as
duas migrações no seu próprio migrator:

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

Ambas são idempotentes contra um banco de dados já aplicado (a v1 usa
`CREATE TABLE IF NOT EXISTS`; a v2 é uma adição de coluna). Rodar
`suprnova migrate` de novo contra um banco de produção que já tem o
schema é um no-op.

### Ambiente

Os mailables transacionais leem duas variáveis de ambiente no momento
do envio:

| Var | Padrão | Usado para |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Branding do subject e o label de issuer `otpauth://` que apps autenticadores exibem. |
| `MAIL_FROM` | nenhum - **dá erro quando não definido** | `From` do envelope em toda mensagem de saída. Defina para um domínio de remetente verificado. |

`MAIL_FROM` deliberadamente não tem padrão. Cair para um placeholder
como `noreply@example.com` quebraria silenciosamente o DMARC / SPF em
produção e enviaria a partir de um domínio que o operador não
controla, então a facade falha de forma fechada em vez disso.
`EmailVerification::send_link` e `PasswordReset::send_link` expõem o
erro como `Err`; `PasswordReset::complete` registra em log via
`tracing::warn!` e continua (a mudança de senha já fez commit, então o
caminho de notificação não consegue desfazê-la).

As apps também definem `APP_URL` para que os controladores possam
derivar a URL base usada nas chamadas a `send_link`; a própria facade
do framework recebe a URL base como um parâmetro.

O driver de mail é configurado separadamente via `MAIL_DRIVER` - veja
a documentação de [Correio](mail.md).

## Verificação de email

`EmailVerification` cunha, verifica, e consome tokens de verificação
contra a tabela `auth_flow_tokens`, e marca o usuário como verificado
através do provedor configurado. Quatro operações cobrem o ciclo de
vida:

| Método | Assinatura | Notas |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Cunha + envia, dado um usuário já em mãos. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Anti-enumeração: busca o usuário pelo email; um endereço desconhecido é um `Ok(())` silencioso. |
| `check` | `check(token: &str) -> Result<bool>` | Não consome - seguro chamar em uma landing page. |
| `verify` | `verify(token: &str) -> Result<String>` | Uso único: consome o token, marca o usuário como verificado, retorna o id do usuário. |

```rust
use suprnova::auth_flows::EmailVerification;

// Depois de um signup recém-feito, com o usuário recém-criado em mãos:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Verificação opcional de landing-page - não consome, então um
// refresh de página não queima o token.
let valid: bool = EmailVerification::check(&token_str).await?;

// O handler de click-through consome o token e estampa o usuário,
// retornando o id do usuário verificado.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` dispara `EmailVerified` em caso de sucesso - listeners são o
lugar certo para desbloquear funcionalidade adicional (email de
boas-vindas, follows padrão, CTA de "complete seu perfil") sem
acoplá-los ao handler de verificação. O evento carrega o id de usuário
do provedor.

### O endpoint de resend (anti-enumeração)

`resend` recebe só o email - a facade busca o usuário através do
provedor ativo e, quando uma conta está registrada, cunha um token e
envia o mail; um email desconhecido é um no-op silencioso que ainda
retorna `Ok(())`. O handler nunca ramifica com base na existência em
si, então um chamador sondando não consegue distinguir "enviado" de
"conta não existe":

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
    // `resend` faz a busca + anti-enumeração internamente.
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

O handler de click-through extrai o token da query string e chama
`verify`:

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

O handler não precisa buscar o usuário - `verify` consome o token,
marca o usuário como verificado através do provedor, retorna o id do
usuário, e dispara `EmailVerified`. Uso único: um segundo `verify` no
mesmo token retorna um erro.

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

`PasswordReset` tem três operações:

| Método | Assinatura | Notas |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Anti-enumeração: busca o usuário pelo email; um endereço desconhecido é um `Ok(())` silencioso. |
| `check` | `check(token: &str) -> Result<bool>` | Não consome - confirme o token antes de renderizar o formulário de senha nova. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Uso único: consome o token, rotaciona a senha, revoga sessões + remember-me, envia a notificação de mudança, retorna o id do usuário. |

```rust
use suprnova::auth_flows::PasswordReset;

// A partir do formulário "esqueci a senha". Sempre Ok(()) - a facade
// busca o usuário e só envia quando uma conta está registrada.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Verificação opcional de landing-page antes de renderizar o formulário de senha nova.
let valid: bool = PasswordReset::check(&token).await?;

// O handler de click-through, depois de o usuário enviar uma senha
// nova: consome o token + rotaciona a senha, retornando o id do usuário.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` faz hash de `new_password` antes de entregá-la ao provedor -
passe o texto puro, não um valor pré-hasheado. Uma senha vazia / só com
espaços é rejeitada de antemão com um `400`.

### Anti-enumeração

`send_link` é estruturado de forma que o formato da resposta nunca
vaza se um endereço de email tem uma conta:

- Ele sempre retorna `Ok(())`. Quando o email está ausente, nenhum
  token é cunhado, nenhum mail é despachado, e nenhum evento
  `PasswordResetLinkSent` dispara - mas a ausência também não é
  exposta através do tipo de retorno, então um chamador (e um
  observador de rede) não consegue distinguir "conta não existe" de
  "link enviado".
- O controlador de dogfood combina `send_link` com um corpo de
  resposta 200 fixo, então um chamador sondando não consegue
  distinguir através de status code, corpo de resposta, ou timing de
  resposta.

### Efeitos colaterais de `complete`

`complete` executa quatro passos em ordem:

1. Consome o token (uso único) e rotaciona o hash da senha através do
   provedor configurado (o único passo que pode falhar a chamada).
2. Revoga toda linha de sessão do usuário via
   `crate::session::destroy_all_for_user` (best-effort: falhas geram
   `tracing::warn!`).
3. Revoga toda linha de remember-me via
   `crate::auth::remember::revoke_all_for_user` (best-effort).
4. Despacha `PasswordChangedMail` fire-and-forget, depois dispara
   `PasswordResetCompleted`.

Uma sessão roubada e um cookie de remember-me capturado não podem
sobreviver à credencial de que dependiam. As revogações acontecem em
todo reset bem-sucedido, não só nos iniciados pelo usuário, então um
reset forçado por um time de segurança também expulsa um atacante
ativo.

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
  Cai para `900` (15 minutos - o período de bloqueio padrão do torii)
  se o timestamp estiver, de algum jeito, ausente.
- Corpo: `"Account locked due to too many failed login attempts. Try
  again later."`

### Fail-open em erros de backend

Se `get_lockout_status` retornar um `Err` (um engasgo transitório de
banco de dados), o middleware deixa a solicitação passar. O handler de
login downstream então fará a chamada por conta própria e pode decidir
se falha de forma fechada ou aberta. O middleware erra a favor da
disponibilidade: derrubar o endpoint de login sempre que o banco de
dados de auth tiver um problema momentâneo é peor do que deixar o
handler fazer a chamada diretamente.

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

O `BruteForceProtectionConfig` do torii tem como padrão **5 tentativas
falhas antes do bloqueio** e um **período de bloqueio de 15 minutos**.
É isso que `init_torii` conecta hoje; configurar valores por app exige
acessar a própria superfície de configuração do torii e não é exposto
através do builder `ToriiConfig` do Suprnova. Os padrões são
deliberadamente conservadores - escolha "cinco erros de digitação me
bloqueiam por 15 minutos" antes de decidir relaxá-los.

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

`user_id` é a chave de armazenamento opaca - tipicamente
`torii::UserId.as_str()`, mas qualquer identificador estável por
usuário funciona. A tabela de 2FA indexa por ela; não há FK para a sua
tabela de usuário.

`email` é embutido no segmento `account_name` da URL `otpauth://`
para que o app autenticador renderize a linha com um label legível
por humanos (por exemplo, "MyCorp (alice@example.com)").

Um padrão comum é um newtype pequeno que envolve o seu model de
usuário:

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
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
Listeners observando "usuário tentou 2FA e falhou" se inscrevem no
evento novo; listeners observando "senha não autenticou" permanecem em
`auth::Failed`. As duas superfícies são mantidas separadas para que
um erro de digitação no 2FA não pareça uma falha de senha para
pipelines de auditoria.

### Por que Suprnova diverge

O `user_id` de 2FA é intencionalmente uma `String`. Se fosse tipado
como `i64`, `Uuid`, ou `torii::UserId`, a tabela de 2FA ficaria
permanentemente amarrada a qualquer formato que o framework escolhesse
primeiro - apps que armazenam usuários com um formato diferente (UUIDs
versus inteiros auto-incrementados, ou apps que não usam torii de modo
algum, mas querem o módulo de 2FA) ficariam de fora. Um `user_id` em
string deixa cada app escolher qualquer identificador estável por
usuário que preferir; a contrapartida é um `.to_string()` no call
site. O Fortify do Laravel amarra a coluna equivalente ao `User::id`
do Eloquent - o Suprnova a desacopla, para que `TwoFactor` seja uma
primitiva de ciclo de vida reutilizável, não um acessório no formato
de User.

## Remember-me

`suprnova::auth_flows::remember_me` reexporta `suprnova::auth::remember` -
o módulo de cookie persistente que já era distribuído junto com o auth
de sessão. A reexportação é puramente organizacional: tudo que tem o
formato de auth-flow vive sob `auth_flows::*`, mesmo quando a
implementação antecede esse namespace.

O design que é distribuído:

- **Linha-de-BD + hash bcrypt** - todo token emitido tem uma linha na
  tabela `remember_tokens` armazenando só o hash bcrypt, nunca o texto
  puro. Um dump de banco de dados não pode produzir credenciais que
  reautentiquem.
- **Rotação de uso único** - uma verificação bem-sucedida faz DELETE
  na linha correspondente e emite uma nova. Um cookie capturado não
  pode ser reusado; se atacante e vítima correrem para usá-lo, o
  perdedor vê a linha desaparecer e falha ao autenticar.
- **Revogação** - `revoke_all_for_user` apaga toda linha de um usuário
  em um único DELETE. `Auth::logout` encadeia isso para que um logout
  de verdade limpe o estado persistente, e `PasswordReset::complete`
  faz o mesmo para que uma redefinição de senha invalide todo cookie
  persistente existente.
- **Prune** - `prune_expired` limpa linhas expiradas em uma agenda.

Na prática, o middleware de sessão do framework faz o trabalho pesado;
a app típica não chama o módulo `remember_me` diretamente. O documento
de [Autenticação](authentication.md) cobre a superfície voltada ao
usuário - a flag `remember` em `Auth::login`, o nome do cookie, e os
controles de lifetime.

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

O fake registra eventos despachados sem invocar listeners, então um
listener que fala com um serviço externo não vai disparar durante o
teste. O companheiro `assert_not_dispatched::<E>(pred)` afirma o
negativo; `dispatched_count::<E>(pred)` retorna a contagem bruta para
asserções mais refinadas.

### Testes de integração para verificação de email + redefinição de senha

Testes de verify / reset não precisam de torii - provisione a tabela
`auth_flow_tokens` em um banco de dados em memória, registre um
provedor, defina `MAIL_FROM`, e dirija a facade sob `Mail::fake()`. Os
próprios testes do framework cunham a tabela diretamente a partir de
`create_auth_flow_tokens_table()`:

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // As facades leem MAIL_FROM (fail-closed); defina-a para o teste.
    // SAFETY: serializado por `#[serial]` - sem observador paralelo.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // ... dirija EmailVerification::send_link(&user, base) ...
    fake.assert_sent_to("ada@example.com");
}
```

Os caminhos apoiados em provedor (`resend` / `verify` / `complete`)
adicionalmente registram um binding `dyn UserProvider` para que a
busca + mutação resolvam - veja `framework/tests/email_verify.rs` e
`framework/tests/password_reset.rs`.

### `ToriiConfig::sqlite_in_memory()` para testes de força bruta + 2FA

Testes de força bruta e 2FA levantam um torii novo em um banco de
dados SQLite em memória. Os arquivos de teste de exemplo em
`framework/tests/` usam um padrão de runtime compartilhado +
`once_cell::sync::Lazy<()>` para amortizar o custo entre testes, mais
`#[serial]` para manter o transporte de mail global ao processo
estável entre testes que intercalam `Mail::fake()`:

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // ... use Mail::fake() / EventFacade::fake() aqui ...
    });
}
```

Exemplos canônicos - copie a partir deles ao escrever os seus
próprios:

- `framework/tests/email_verify.rs` - round-trip de token de verify,
  remoção de barra final em `send_link`, asserções de `Mail::fake()`
  sobre subject/HTML.
- `framework/tests/password_reset.rs` - round-trip de reset com
  autenticação por senha nova, anti-enumeração em emails desconhecidos,
  `complete` rejeita tokens reusados.
- `framework/tests/brute_force.rs` - ciclo de vida completo de
  bloqueio, `AccountLocked` dispara uma vez por transição,
  `unlock_account` retorna `was_locked`.
- `framework/tests/two_factor.rs` - enroll → confirm → verify
  completo, com um código TOTP real computado a partir da URL
  otpauth, uso único de código de recuperação, recadastro sobrescreve
  o secret, rejeição de replay entre dois verifies concorrentes.
- `framework/tests/two_factor_challenge_flow.rs` - o fluxo de desafio
  de ponta a ponta, com rotação de sessão, reemissão de remember-me, e
  dispatch de evento.
- `framework/tests/email_verified_middleware.rs` e
  `two_factor_challenge_middleware.rs` - formatos de resposta de
  middleware (403 JSON vs 302 vs 409 + X-Inertia-Location).

## Referência

| Símbolo | Propósito |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, `verify` - apoiado em provedor; `verify` retorna o id do usuário. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` para 403 JSON, `redirect_to(path)` para 302 / 409 + X-Inertia-Location. Verifica o `is_email_verified` do provedor configurado (fail-closed). |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` - apoiado em provedor; `complete` retorna o id do usuário. |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | Model traits que um usuário por trás de `EloquentUserProvider` implementa, para que as facades de verify / reset possam ler seu email + escrever seu timestamp de verificação / hash de senha. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | `CREATE TABLE` do SeaORM para `auth_flow_tokens` - liste no seu migrator. |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`, `reset_attempts`, `get_lockout_status`, `is_locked`, `unlock_account`. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP que responde 429 antes do handler quando a conta visada está bloqueada. |
| `suprnova::auth_flows::TwoFactor` | `enroll`, `re_enroll`, `confirm`, `verify`, `consume_recovery_code`, `regenerate_recovery_codes`, `is_enabled`, `is_enabled_by_id`, `start_challenge`, `pending_user_id`, `cancel_challenge`, `complete_challenge`, `disable`. |
| `suprnova::auth_flows::TwoFactorUser` | Trait que faz a ponte do model de usuário da app até a facade de 2FA. |
| `suprnova::auth_flows::EnrollmentResponse` | Valor de retorno de `TwoFactor::enroll` - `otpauth_url`, `qr_code_svg`, `recovery_codes`. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | `new()` para 403 JSON, `redirect_to(path)` para 302 / 409 + X-Inertia-Location. Componha na frente de `AuthMiddleware`. |
| `suprnova::auth_flows::two_factor::migration::Migration` | Migração SeaORM para `two_factor_credentials`. Liste no seu `Migrator::migrations()`. |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | Adição de coluna para `last_used_timestep` (proteção contra replay de TOTP). Liste depois da migração de create-table. |
| `suprnova::auth_flows::remember_me` | Reexportação de `suprnova::auth::remember`. |
| `suprnova::auth_flows::events::*` | Nove eventos - veja [Eventos](#eventos). |
| `suprnova::auth_flows::EmailVerificationMail` | Mailable transacional. Subject `"Verify your email for {APP_NAME}"`. |
| `suprnova::auth_flows::PasswordResetMail` | Mailable transacional. Subject `"Reset your {APP_NAME} password"`. |
| `suprnova::auth_flows::PasswordChangedMail` | Mailable de notificação de segurança. Subject `"Your {APP_NAME} password was changed"`. |
| `suprnova::torii_integration::ToriiConfig` | Config de bootstrap do torii. `from_sea_orm(conn)` para produção, `sqlite_in_memory()` para testes. |
| `suprnova::torii_integration::init_torii` | Init global idempotente. Chame uma vez a partir de `bootstrap.rs::register()`. |

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
