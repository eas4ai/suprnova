# OAuth, Apple e login sem senha

O Suprnova traz três métodos de login apoiados em torii atrás da facade
`Auth`: **OAuth genérico** (GitHub, Google, ou qualquer provedor OIDC/OAuth2),
**Sign in with Apple**, e **magic links sem senha**. Eles compartilham um
pré-requisito (`init_torii` mais a migração da cerimônia) e o mesmo formato
de facade - `Auth::oauth(provider)` / `Auth::magic_link()` - e nenhum deles
traz rotas: você adiciona um controlador fino (start + callback) e o
framework cuida do state CSRF, do PKCE, da troca de token, da verificação de
identidade, do upsert de usuário, e da cunhagem da sessão.

Toda a superfície vive em `framework/src/torii_integration/`. **Não** há
contrato de variável de ambiente do framework para nada disso - toda
credencial é passada programaticamente (busque a sua própria a partir do
ambiente); os exemplos deste capítulo usam `std::env::var(...)` apenas para
mostrar onde os seus segredos entram.

## Pré-requisitos

1. **Inicialize o torii uma vez na inicialização** - isso apoia o upsert de
   usuário e a criação de sessão:

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // em bootstrap::register(), depois de DB::init()
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **Execute a migração da cerimônia.** OAuth e Apple guardam uma cerimônia
   de curta duração (10 minutos) de `state` CSRF + PKCE na tabela
   `auth_ceremony_tokens`. Registre a migração
   `m20251209_000000_create_auth_ceremony_tokens_table` no seu `Migrator`
   (os starter kits já a incluem). Opcionalmente, agende
   `suprnova::torii_integration::ceremony::prune_expired()` para fazer GC das
   linhas obsoletas.

3. **`SessionMiddleware` na rota *start* do OAuth.** `begin()` escreve o
   `state` na sessão; uma chamada sem sessão falha com um 500.

Magic links só precisam do passo 1.

## OAuth genérico (GitHub, Google, customizado)

### Configure um provedor

Registre cada provedor uma vez na inicialização. O registro é global ao
processo e idempotente, então registrar o mesmo provedor de novo apenas
substitui a config:

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → a tabela well-known embutida
    apple_key_pair: None,       // Somente Apple; deixe None para GitHub/Google
    apple_team_id: None,        // Somente Apple
});
```

Os endpoints well-known de authorize/token/userinfo já vêm embutidos para
`github`, `google`, e `apple`. Para qualquer outro provedor - ou um servidor
self-hosted / de teste - forneça-os você mesmo:

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // fallback /emails estilo GitHub para um email primário privado
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### Inicie o fluxo (URL de autorização)

```rust
// GET /auth/oauth/github/start  (a rota PRECISA carregar SessionMiddleware)
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - redirecione o navegador para aqui
// kickoff.state - state CSRF, já armazenado na sessão para você
```

`begin()` cunha o `state` CSRF (UUID v4) e um verificador/desafio S256 PKCE
(RFC 7636), registra a cerimônia (TTL de 10 minutos), e retorna a URL de
autorização do provedor. Redirecione o usuário para `authorization_url`.

### Complete o fluxo - `verify` vs `complete`

No callback você tem dois pontos de entrada (divididos na 0.5.4). Escolha
conforme sua tabela `users` **ser** ou não o schema do torii:

| Método | Retorna | Efeitos colaterais | Use quando |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **Nenhum** - verifica a cerimônia, troca o code, busca o userinfo, extrai um email verificado + `subject` estável. Sem usuário, sem sessão. | Sua app é dona da própria tabela `users` e você quer buscar / criar o usuário você mesmo. |
| `complete(code, state)` | `(User, Session)` | Faz upsert do usuário no torii (`get_or_create_user`) e cunha uma sessão. | Sua tabela `users` é o schema do torii. |

```rust
// Tabela users customizada:
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject é o id estável do provedor; id.email é verificado-ou-None.
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// …ou, apoiado em torii:
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

Um `email` retornado por `verify` é sempre um endereço *verificado* (OIDC
`email_verified`, GitHub tratado como verificado, ou o fallback `/emails`);
um email não verificado ou ausente volta como `None`, e logins repetidos
resolvem pelo `subject`.

### Rotas que você adiciona

O framework não fornece rotas de OAuth - conecte dois handlers finos
(espelhando o formato dos controladores `auth_verify` / `auth_reset` já
existentes no starter kit):

```rust
// start - redireciona para o provedor
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// callback - GitHub/Google usam GET ?code&state
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

Coloque a rota `/start` (pelo menos) atrás de `SessionMiddleware`.

## Sign in with Apple

A Apple é a mesma facade - `Auth::oauth("apple")` - com algumas regras
específicas da Apple já embutidas:

- **O callback é um `POST`.** A Apple usa `response_mode=form_post`, então o
  redirect entrega `code` + `state` em um corpo de formulário, não em query
  params. Registre o callback da Apple como uma rota `post!` e leia os
  campos a partir do formulário.
- **Sem PKCE.** A Apple rejeita `code_challenge`, então a URL de autorização
  o omite (o client secret é, em vez disso, um JWT assinado).
- **`client_secret` não é usado** - deixe-o como `String::new()`. O Suprnova
  cunha o client secret JWT de curta duração a partir da sua chave `.p8` em
  cada troca de token.
- **Os ID tokens são verificados contra o JWKS da Apple (RS256)** desde a
  0.5.6, e não apenas confiados estruturalmente.

### Forneça sua chave Apple - `AppleKeyPair`

`AppleKeyPair` é o único tipo da Apple reexportado para apps (assim você não
precisa de uma dependência direta em `apple`). Construa-o a partir da sua
chave de assinatura `.p8`:

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // *Key ID* da Apple (não o Team ID)
    &std::env::var("APPLE_P8_PATH")?,  // caminho para AuthKey_XXXXXX.p8
)?;
// ou: AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### Configure a Apple

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // seu Services ID
    client_secret: String::new(),                  // não usado - cunhado a partir da chave
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // Team ID de 10 caracteres
});
```

### Complete o fluxo da Apple

Mesma divisão do OAuth genérico. `complete` faz upsert + sessão; o caminho
de verify retorna um `AppleIdentity` para uma tabela users customizada:

```rust
// POST /auth/apple/callback - leia code + state a partir do corpo do FORM
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// …ou tabela users customizada:
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id: AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` é `Some(_)` somente quando a Apple garante que ele foi
verificado; um email não verificado é recusado (401) antes de a identidade
ser construída. `is_private_email` é definido quando o usuário escolheu o
endereço de relay privado da Apple - persista o `subject` como a chave
estável, já que o endereço de relay é o único email que você vai conseguir.

## Login por Magic Link

Login por email sem senha, apoiado em torii, via `Auth::magic_link()`. O
framework emite e verifica o token; **você** envia o link por email (ele
nunca envia mail por conta própria), o que se compõe de forma limpa com o
capítulo [Correio](mail.md).

```rust
use suprnova::Auth;

// POST /auth/magic - solicita um link
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// Construa o link e envie-o por email você mesmo:
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - consome-o (uso único; uma segunda chamada falha)
let (user, session) = Auth::magic_link().consume(&token).await?;
```

O usuário é criado automaticamente no primeiro uso. `send` retorna o token
em **texto puro** para que você controle o formato da URL e a entrega.

> **Nota - `TokenPurpose::MagicLink`.** O enum `TokenPurpose` de
> `auth_flows` tem uma variante `MagicLink` (adicionada na 0.5.5), mas ela é
> um *discriminador reservado* para o `TokenStore` genérico - nenhum fluxo
> embutido a consome. O caminho de magic-link funcional e suportado é o
> `Auth::magic_link()` acima. Só recorra a `TokenPurpose::MagicLink` se você
> estiver construindo à mão seu próprio fluxo sobre a tabela
> `auth_flow_tokens`.

## Uma nota sobre configuração

Nenhum desses métodos lê variáveis de ambiente do framework - IDs de
provedor, segredos, URLs de redirect, e chaves da Apple são todos passados
para `configure(...)` programaticamente. Carregue-os do jeito que preferir
(`std::env::var`, uma struct de config tipada, um secret manager) e registre
os provedores uma vez durante o `bootstrap`. Isso mantém setups de provedor
multi-tenant / por-deploy como cidadãos de primeira classe, em vez de forçar
um esquema fixo de nomenclatura de variável de ambiente.

## Referência

- Pontos de entrada da facade: `Auth::oauth(provider)`, `Auth::magic_link()`
  (`suprnova::Auth`)
- Config: `suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- Resultados de OAuth: `OAuthKickoff { authorization_url, state }`,
  `OAuthIdentity { provider, subject, email, name }`,
  `AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- Inicialização: `suprnova::{init_torii, ToriiConfig}`
- Armazenamento da cerimônia: tabela `auth_ceremony_tokens` +
  `suprnova::torii_integration::ceremony::prune_expired()`

## Próximos passos

- [Autenticação](authentication.md) - guards, provedores, e o modelo de
  usuário `Authenticatable` para o qual esses fluxos criam sessões
- [Fluxos de autenticação](auth-flows.md) - verificação de email,
  redefinição de senha, e 2FA
- [Correio](mail.md) - enviando o email do magic link (e a config de
  remetente `MAIL_FROM` / `MAIL_FROM_NAME`)
- [Sessões](session.md) - o que é o `Session` retornado e como ele é
  persistido
