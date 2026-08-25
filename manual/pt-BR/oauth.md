# OAuth, Apple e login por link mágico

O Suprnova expõe OAuth, Iniciar sessão com a Apple e links mágicos sem senha por
meio da facade `Auth`, pertencente ao framework. O Magnetar fornece os mecanismos
de credencial, cerimônia, identidade, controle de fatores e sessão por trás dessa
facade.

Os pontos de entrada públicos são:

- `Auth::oauth(provider)` para OAuth e Apple.
- `Auth::magic_link()` para login por e-mail sem senha.

O Suprnova não instala rotas para esses fluxos. As aplicações fornecem pequenos
handlers de início e retorno de chamada e decidem como entregar o e-mail do
link mágico.

## Inicializar o Magnetar com OAuth

Configure OAuth na mesma `MagnetarConfig` que inicializa serviços de senha, chave de acesso, sessão, bloqueio e autenticação de dois fatores. O registro de provedores é publicado atomicamente com esses serviços: se qualquer serviço não puder ser criado, nenhum deles ficará visível.

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

O framework reexporta o contrato `OAuthProvider`, os cinco provedores próprios e tipos de configuração, e todos os tipos necessários para implementar um provedor personalizado. `ReqwestOAuthTransport` fornece E/S de produção para token, userinfo e revogação. `FrameworkAbuseLimiter` usa o `RateLimiterDriver` configurado pela aplicação. As aplicações não precisam nem de uma dependência direta de `suprnova-magnetar` nem de adaptadores de transporte e limitador escritos manualmente.

`MagnetarConfig` cria seu esquema quando `apply_migrations` está habilitado, que é o padrão. Use `.apply_migrations(false)` somente quando a implantação preparar o mesmo esquema separadamente. Uma segunda inicialização retorna um erro em vez de substituir qualquer mecanismo instalado.

### Requisitos do provedor GitHub

O endpoint REST de usuário do GitHub exige um `User-Agent`; um provedor da comunidade o adiciona, junto com qualquer valor `Accept` de tipo de mídia de que precise, por meio de `OAuthProvider::userinfo_headers`. O Suprnova adiciona separadamente o cabeçalho bearer `Authorization` e rejeita tentativas do provedor de substituí-lo.

A resposta `/user` do GitHub inclui um e-mail somente quando o usuário o tornou público. O endereço primário verificado exige uma segunda solicitação `/user/emails`, enquanto `resolve_identity` deliberadamente não realiza E/S e recebe uma resposta userinfo. Um provedor GitHub pode retornar `email: None` e usar a cerimônia de conclusão de e-mail do Suprnova, ou apontar `userinfo_endpoint` para um adaptador de host que combine `/user` com o e-mail primário verificado. Não trate um endereço não verificado ou meramente público como propriedade da conta.

## Associação à sessão

O início de OAuth requer `SessionMiddleware`. O Magnetar associa a cerimônia a um
digest da sessão de framework que a iniciou, de forma que o retorno de chamada não
possa ser movido para outra sessão de navegador.

Um login bem-sucedido por senha, link mágico, passkey e OAuth alterna o ID de
sessão do framework e o token CSRF, registra o ID do usuário da aplicação e
armazena uma associação web opaca do Magnetar. A hidratação de lembrar-me alterna
tanto a credencial do Magnetar quanto a associação de sessão do framework.

## Inicie um fluxo OAuth

Use `begin` no handler de início do provedor:

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// Retorne um redirecionamento HTTP para kickoff.authorization_url.
```

O `OAuthKickoff` retornado contém:

- `authorization_url`, a URL a enviar ao navegador.
- `state`, o seletor de uso único associado à sessão que iniciou o fluxo.

O Magnetar é responsável pela geração do estado, política PKCE, persistência da
cerimônia, troca com o provedor, verificação de identidade e limitação de abuso. O
controlador hospedeiro é responsável pelo redirecionamento HTTP e pela rota de
retorno de chamada.

## Verifique ou conclua o retorno de chamada

O retorno de chamada tem dois pontos de entrada:

| Método | Resultado | Efeitos colaterais |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | Verifica a prova do provedor e retorna o provedor, assunto, e-mail verificado e nome de exibição sem criar uma sessão de aplicação. |
| `complete(code, state)` | `(User, Session)` | Resolve a identidade pelo mecanismo hospedeiro instalado, aplica a política de vinculação de contas e o controle de fatores, alterna a sessão do framework e retorna os valores de usuário pertencente ao framework e de sessão do Magnetar. |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` está presente somente quando o provedor forneceu um e-mail
verificado. Persista o provedor e o assunto como a identidade externa estável. O
e-mail não é um identificador estável do provedor.

## Política de vinculação de contas

A conclusão do OAuth não trata a posse de uma cadeia de e-mail não verificada como
prova de que quem chama possui uma conta de aplicação existente.

O resultado da conclusão pode exigir mais trabalho em vez de emitir uma sessão:

- **Conclusão de e-mail necessária** retorna HTTP 409 quando a identidade do
  provedor precisa de uma cerimônia de e-mail verificado separada.
- **Vinculação explícita necessária** retorna HTTP 409 quando uma conta verificada
  existente deve autorizar o vínculo.
- **Fator necessário** retorna HTTP 401 quando a política da conta exige um
  segundo fator antes da emissão da sessão.

Uma conclusão de e-mail verificado que vence a fronteira da primeira prova de
e-mail recupera atomicamente uma conta não verificada ocupada. A transação avança
a época de autenticação, remove credenciais provisórias, revoga sessões e
credenciais de lembrar-me antigas e anexa a conta de provedor verificada. Uma
conta verificada nunca é vinculada automaticamente apenas pelo e-mail.

## Iniciar sessão com a Apple

A Apple usa a mesma facade `Auth::oauth("apple")`, mas seu retorno de chamada
comumente usa `response_mode=form_post`. Registre o retorno de chamada como uma
rota `POST` e passe o campo de formulário opcional `user` da Apple pelos métodos
específicos para Apple:

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` inclui o assunto estável, e-mail verificado opcional,
`email_verified` e `is_private_email`. Persista o assunto como a chave estável. A
Apple pode fornecer o nome de exibição somente durante a primeira autorização,
portanto o adaptador de provedor deve preservar esse primeiro valor de `form_post`.

A verificação de token e identidade Apple pertence à implementação de provedor
instalada. Os provedores Magnetar atuais exigem verificações de assinatura,
emissor, audiência, expiração e nonce em vez de confiar no JSON decodificado de um
token de ID.

## Login por link mágico

O login por link mágico usa o mecanismo de senha/sessão Magnetar instalado. O
framework retorna o token de uso único em texto simples, enquanto a aplicação é
responsável pela composição do e-mail e pelo formato da URL:

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` aplica o orçamento de abuso de autenticação antes da emissão do token.
`consume` é de uso único, aplica o controle de fatores, associa a sessão
resultante à sessão de solicitação do framework e retorna o usuário e a sessão do
Magnetar.

Para uma conta pré-existente não verificada, o consumo bem-sucedido do link mágico
é uma primeira prova de e-mail. A transação recupera a conta e remove o estado
provisório de senha, passkey, conta vinculada, dois fatores, sessão e lembrar-me
para que um ocupante anterior não possa reter acesso.

## Rotas a adicionar

Uma aplicação típica adiciona estas rotas:

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

Aplique `SessionMiddleware` a todas as rotas de início/retorno de chamada de OAuth
e passkey. A sessão carrega o seletor da cerimônia e associa a ida e volta ao
navegador que a iniciou.

## Migração de autenticação

O crate `suprnova-magnetar` inclui um mecanismo de migração sensível ao formato
para esquemas Torii, Suprnova web, Suprnova API e Magnetar existentes. É uma
superfície de biblioteca e um exemplo, não um subcomando da CLI `suprnova`.

Habilite o recurso `migration` mais o driver de banco de dados de origem e execute
um plano a seco antes de aplicar. Para PostgreSQL:

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

Use `seaorm-mysql` ou `seaorm-sqlite` quando esse for o driver de banco de dados
da origem e da aplicação.

Adicione `--apply` para aplicar o plano revisado. O executor verifica novamente as
impressões digitais da origem e do esquema antes da importação, registra o estado
de repetição, recusa colisões de identidade e usa importações transacionais. As
migrações MySQL no mesmo banco de dados usam uma troca de sombra protegida por
barreira de escrita com caminhos de restauração e aborto retomáveis.

Mantenha o plano e o relatório gerados nos registros de implantação. Não aplique
um plano cuja impressão digital da origem tenha mudado após a revisão.

## Referência

- Inicialização padrão: `MagnetarConfig`, `PasskeyConfig` e `init_magnetar`.
- Facades: `Auth::oauth(provider)` e `Auth::magic_link()`.
- Instalação OAuth: `MagnetarConfig::oauth`, `ReqwestOAuthTransport` e `FrameworkAbuseLimiter`.
- Biblioteca de migração: `magnetar::migration` do crate `suprnova-magnetar`.
- Autenticação bearer: `BearerTokenMiddleware`.

## Próximos passos

- [Autenticação](authentication.md) aborda senha, passkey, guardas, sessões de
  framework e inicialização de mecanismos.
- [Fluxos de autenticação](auth-flows.md) aborda verificação de e-mail,
  redefinição de senha, bloqueio e autenticação de dois fatores.
- [Correio](mail.md) aborda a entrega de link mágico pertencente à aplicação.
- [Sessão](session.md) aborda a sessão de navegador que associa cerimônias OAuth e
  passkey.
