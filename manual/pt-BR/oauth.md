# OAuth, Apple e login por link mágico

O Suprnova expõe OAuth, Iniciar sessão com a Apple e links mágicos sem senha por
meio da facade `Auth`, pertencente ao framework. O Magnetar fornece os mecanismos
de credencial, cerimônia, identidade, controle de fatores e sessão por trás dessa
facade.

Os pontos de entrada públicos são:

- `Auth::oauth(provider)` para OAuth e Apple.
- `Auth::magic_link()` para login por e-mail sem senha.

O Suprnova não instala rotas para esses fluxos. As aplicações fornecem pequenos
manipuladores de início e retorno de chamada e decidem como entregar o e-mail do
link mágico.

## Inicialize o Magnetar

Inicialize os mecanismos padrão de senha, passkey, sessão, bloqueio e dois fatores
depois de `DB::init` e depois que `APP_KEY` inicializar `Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`MagnetarConfig` usa a conexão SeaORM da aplicação. O mecanismo padrão cria seu
esquema quando `apply_migrations` está habilitado, que é o padrão. Defina
`.apply_migrations(false)` somente quando a implantação executar a mesma
configuração de esquema separadamente.

`init_magnetar` instala adaptadores de senha/sessão e passkey atomicamente. Uma
segunda instalação retorna um erro em vez de substituir o mecanismo e dividir o
estado de autenticação.

## Instalação do mecanismo OAuth

O suporte a OAuth é compilado pelo recurso padrão `magnetar-oauth` do framework,
mas o registro de provedores é sempre uma etapa explícita em runtime. Em um build
com `--no-default-features`, habilite `magnetar-oauth` explicitamente.
`init_magnetar` não retorna nem expõe seu engine de host concreto interno, então
o exemplo abaixo aplica-se somente a uma aplicação que constrói e mantém seu
próprio `MagnetarHostEngine`; ele não pode ser anexado ao exemplo de inicialização
padrão anterior. A API pública atual não tem um método de conveniência para
adicionar um registro OAuth a um engine já instalado por `MagnetarConfig`.

```rust,ignore
use std::sync::Arc;
use suprnova::magnetar_integration::install_magnetar_oauth_engine;

let oauth = host_engine.oauth_service(oauth_host_config)?;
install_magnetar_oauth_engine(Arc::new(oauth))?;
```

`MagnetarOAuthHostConfig` recebe uma lista explícita de valores
`MagnetarOAuthProviderConfig`, um transporte HTTP, um limitador de abuso, uma
política de autorização e uma política de vinculação automática. O registro de
provedores se torna autoritativo quando instalado. Um provedor desconhecido falha
de forma fechada em vez de recorrer a outra implementação de autenticação.

As implementações de provedores e seus dossiês de autenticação de cliente vêm do
crate `suprnova-magnetar`. As aplicações que constroem o mecanismo OAuth devem
adicionar esse crate como dependência direta com os recursos dos provedores que
usam. O framework não infere IDs ou segredos de cliente OAuth de variáveis de
ambiente. Leia-os pela configuração da aplicação ou por um gerenciador de
segredos e construa o registro de provedores durante o bootstrap.

## Associação à sessão

O início de OAuth requer `SessionMiddleware`. O Magnetar associa a cerimônia a um
digest da sessão de framework que a iniciou, de forma que o retorno de chamada não
possa ser movido para outra sessão de navegador.

Um login bem-sucedido por senha, link mágico, passkey e OAuth alterna o ID de
sessão do framework e o token CSRF, registra o ID do usuário da aplicação e
armazena uma associação web opaca do Magnetar. A hidratação de lembrar-me alterna
tanto a credencial do Magnetar quanto a associação de sessão do framework.

## Inicie um fluxo OAuth

Use `begin` no manipulador de início do provedor:

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
- Instalação OAuth:
  `suprnova::magnetar_integration::install_magnetar_oauth_engine` e os tipos de
  configuração em `suprnova::magnetar_integration::engine`.
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
