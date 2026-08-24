# Autenticação

O Suprnova traz um sistema de autenticação no formato do Laravel: uma
facade estática `Auth`, guards nomeados resolvidos através de um
`AuthManager`, provedores de usuário plugáveis, uma trait
`Authenticatable` no seu model de User, e middleware para bloquear
rotas. Um projeto criado com scaffold inicializa com um guard de sessão
(`web`) e um guard de token (`api`) já conectados contra o seu `User`
tipado, então login, registro, e rotas protegidas funcionam no dia em
que você rodar `suprnova new`.

## As peças

| Tipo | Papel |
|---|---|
| `Auth` | Facade do framework para guards, além de operações de senha, magic link, passkey e OAuth apoiadas no Magnetar |
| `MagnetarConfig` / `init_magnetar` | Compõe e instala atomicamente os engines padrão de senha, sessão, lockout, passkey e fator |
| `Authenticatable` | Trait que o model da sua aplicação implementa; expõe `get_auth_identifier() -> String` e o hash da senha |
| `UserProvider` | Trait que busca usuários da aplicação; `EloquentUserProvider<M>` e `DatabaseUserProvider` já vêm embutidos |
| `AuthManager` | Mantém a `AuthConfig` e os provedores registrados; resolve guards nomeados sob demanda |
| `SessionGuard` / `TokenGuard` | Contratos de guard stateful e stateless do framework |
| `BearerTokenMiddleware` | Resolve sessões bearer do Magnetar no estado de autenticação da solicitação do framework |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | Guards de rota |
| `Credentials` | Mapa de credenciais em formato JSON, tipicamente `{ "email", "password" }` |

O código de guard/provider do framework fica em `framework/src/auth/`.
Os adaptadores de host e as facades do Magnetar ficam em
`framework/src/magnetar_integration/`; o crate do engine fica em
`crates/suprnova-magnetar/`. Os fluxos de verificação de email,
redefinição de senha, lockout e TOTP de nível mais alto ficam em
`framework/src/auth_flows/` e são cobertos em [Fluxos de
autenticação](auth-flows.md). O login OAuth, Apple e por magic link é
coberto em [OAuth e login sem senha](oauth.md).

## Modelo de identificador

O id do usuário autenticado flui através do Suprnova como uma `String`
de ponta a ponta - armazenamento de sessão,
[`UserProvider::retrieve_by_id`], a tabela de remember-me, todo evento
de auth. A superfície canônica é
`Authenticatable::get_auth_identifier() -> String` (o
`getAuthIdentifier` do Laravel). Chaves primárias numéricas se
convertem para string trivialmente; UUIDs, ULIDs, e ids opacos de
provedor OAuth fluem sem alteração.

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` é aquilo contra o que os provedores embutidos
verificam uma senha em texto puro via `hashing::verify_async`. Retorne
`None` para usuários que autenticam por outros meios (OAuth, passkey,
magic link). O método `auth_identifier_name() -> &'static str` (padrão
`"id"`) nomeia a coluna em que o id vive. O método de conveniência
`auth_identifier() -> i64` faz parse da string do id por padrão e cai
para `0` em ids não numéricos - o próprio Suprnova nunca o chama;
sobrescreva-o só para models indexados por inteiro que queiram pular o
parse.

### Por que Suprnova diverge

O `getAuthIdentifier()` do Laravel retorna `mixed`. O PHP não se
importa se o id é um int, uma string UUID, ou uma chave primária
stringly-typed de uma tabela legada. O Rust precisa de um único tipo
concreto sobre o qual a sessão, o provedor, e os eventos todos
concordem. `String` é a única escolha que acomoda todo formato de id
sem forçar o framework a saber qual deles a sua app usa. A conveniência
inteira de `auth_identifier()` existe para o caso comum em que a sua
coluna é um `BIGINT`, mas o framework nunca depende dela - troque seu
`User` para um ULID amanhã e nada na pilha de auth vai notar.

## Conectando o auth no boot

O análogo Rust de `config/auth.php` é uma `AuthConfig` registrada como
um singleton de `AuthManager` no contêiner, mais um `UserProvider`
registrado sob um nome. `bootstrap.rs` normalmente faz as duas coisas
em duas linhas:

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ... DB::init, instalação do SessionMiddleware, etc.

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` lê o guard padrão a partir de `AUTH_GUARD`
(padrão `"web"`) e já vem com dois guards nomeados prontos para uso: um
guard de sessão `web` e um guard de token `api`, ambos apoiados pelo
provedor `"users"`. Apps que precisam de mais guards (um provedor
`admins` separado, guards stateful e stateless distintos) constroem a
config explicitamente:

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## Inicialize o engine Magnetar

O starter de API inicializa o Magnetar depois que o banco de dados e
`APP_KEY` estão prontos:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

O engine padrão compartilha a conexão SeaORM da aplicação e cria seu
schema, a menos que `.apply_migrations(false)` seja selecionado. Ele
instala os adaptadores de senha/sessão e passkey atomicamente. Uma nova
inicialização retorna um erro em vez de substituir um adaptador enquanto
outra solicitação ainda usa o armazenamento antigo.

`MagnetarConfig` também aceita valores de política para sessão, lockout e
dois fatores:

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

O binding de host padrão usa a tabela canônica `app_users` com IDs de
aplicação `i64`. O `UserId` público do Magnetar permanece opaco no
limite da facade; o binding padrão só faz parse do identificador
armazenado onde ele cruza para a tabela da aplicação.

### Métodos de facade apoiados no Magnetar

O engine instalado alimenta estes métodos pertencentes ao framework:

- `Auth::password().register(...)`.
- `Auth::password().authenticate(...)`.
- `Auth::magic_link().send(...)` e `.consume(...)`.
- `Auth::passkey().begin_registration(...)` e `.finish_registration(...)`.
- `Auth::passkey().begin_authentication(...)` e
  `.finish_authentication(...)`.
- `Auth::oauth(provider)` quando um delegado OAuth está instalado.
- Emissão, rotação e revogação de remember-me.
- Busca de sessão bearer por `BearerTokenMiddleware`.
- `list_sessions`, `revoke_session` e `revoke_all_sessions` em
  `suprnova::magnetar_integration`.

Um login bem-sucedido rotaciona o ID de sessão e o token CSRF do
framework, armazena o ID de usuário da aplicação e registra um binding
web opaco do Magnetar. O framework continua a ser dono de middleware
HTTP, cookies, email, eventos e seus contratos de guard/provider.

### Autenticação por senha

Use a facade de senha do Magnetar quando a aplicação precisar do caminho
integrado de credencial, lockout, gate de fator e sessão:

```rust,ignore
let user = Auth::password()
    .register("alice@example.com", password)
    .await?;

let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`authenticate` retorna erros HTTP 401 para credenciais inválidas,
lockout ou um segundo fator obrigatório. Falhas de armazenamento e do
engine continuam a ser erros do servidor. O método nunca retorna
material de senha.

### Passkeys

As chamadas de início e fim de passkey exigem `SessionMiddleware` porque
o seletor de cerimônia de uso único fica armazenado na sessão do
framework:

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

O registro segue o par correspondente
`begin_registration` e `finish_registration`. A inscrição em uma conta
existente exige um ator de solicitação verificado e reautenticação recente
pelo caminho do plugin; um ID de usuário isolado em uma sessão legada não
é promovido a ator de credencial.

### Primeira prova de email e épocas de autenticação

O Magnetar trata a primeira prova de caixa postal bem-sucedida em uma
conta não verificada como um limite de credencial atômico. A redefinição
de senha, o consumo de magic link e a conclusão de email verificado por
OAuth podem vencer esse limite.

A transação avança a época de autenticação da conta, revoga sessões e
credenciais de remember antigas e remove credenciais provisórias que um
ocupante indevido poderia ter registrado antes da chegada do dono da
caixa postal. Escritas de senha, passkey, conta vinculada e dois fatores
carregam um snapshot de ator e falham se a época da conta tiver mudado
enquanto a operação estava em andamento.

Em uma conta já verificada, a redefinição de senha preserva passkeys,
contas vinculadas e inscrição de dois fatores legítimas, enquanto ainda
rotaciona a senha e invalida sessões. O OAuth nunca vincula
automaticamente uma conta existente não verificada apenas pelo email; ele
exige a conclusão de email verificado ou vinculação explícita conforme a
política do host.

### Superfície direta do crate Magnetar

A maioria das aplicações fica nas facades do framework. Aplicações que
criam um host de identidade customizado podem depender diretamente de
`suprnova-magnetar` para:

- Rotas de plugin e handlers de efeito neutros para o framework.
- Plugins de senha e gerenciamento de senha.
- Engines de passkey e dois fatores.
- Autorização OAuth, grants, plugins de provider, autorização de
  dispositivo e serviços de token broker.
- Engines de sessão opaca, JWT, remember e grant.
- Bindings de armazenamento customizados e o schema SeaORM padrão.
- Migração de dados de autenticação orientada por formato.

O uso direto não transfere a propriedade de HTTP ou do usuário da
aplicação para o Magnetar. O host ainda mapeia solicitações do wire,
efeitos de email, IDs de aplicação, drivers de limite de taxa e bindings
de sessão para seu próprio framework.

## A facade `Auth`

A facade estática `Auth` é a superfície no formato do Laravel que você
chama a partir de controladores e middleware. Os métodos baseados em
credencial e em usuário delegam para o **guard padrão** (o que quer que
`AuthConfig::default_guard` aponte, padrão `"web"`); as leituras
síncronas `check`/`guest`/`id` são o caminho rápido apoiado em sessão e
não precisam de manager.

```rust
use suprnova::{Auth, Credentials};

// Valida as credenciais e loga o usuário. Dispara Attempting → (Login +
// Authenticated), honra o remember-me. Retorna o usuário resolvido, ou
// None em credenciais inválidas.
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// Loga um usuário já conhecido diretamente.
Auth::login(user, remember).await?;

// Loga por id sem reverificar credenciais (por exemplo, um registro recém-concluído).
Auth::login_using_id(&id, remember).await?;

// Valida credenciais sem persistir uma sessão (diálogos de confirmação de senha).
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// Autentica só para esta solicitação - sem escrita de sessão. O `once` do Laravel.
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// Caminho rápido apoiado em sessão (não exige AuthManager).
if Auth::check()    { /* autenticado */ }
if Auth::guest()    { /* não autenticado */ }
if let Some(id) = Auth::id() { /* id em string */ }

// Se o usuário atual foi autenticado pelo cookie de remember-me nesta
// solicitação. O `viaRemember()` do Laravel.
if Auth::via_remember() { /* … */ }

// Resolve o usuário atual (via o provedor registrado).
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// Desmonta o auth + revoga o remember-me + rotaciona o CSRF + dispara Logout.
Auth::logout().await?;

// Destruição completa da sessão (regenera o id + flush + revoga remember-me + dispara Logout).
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` retorna o usuário resolvido em caso de sucesso, em vez
de um `bool` nu - mais rico que a API do Laravel, e economiza a chamada
de acompanhamento a `Auth::user()`. `Ok(None)` significa que as
credenciais não resolveram um usuário; `Err` significa uma falha de
banco de dados / hashing / configuração que precisa se propagar.

Se você já verificou a identidade de um usuário por conta própria e só
quer estabelecer a sessão - digamos, depois que um callback de OAuth se
completa - recorra à primitiva síncrona:

```rust
// Síncrono, sem provedor, sem AuthManager, sem eventos. Retorna Err
// quando chamado fora de um escopo de solicitação (sem SessionMiddleware
// instalado), para que um login descartado silenciosamente nunca possa
// parecer um sucesso.
Auth::login_id(user.id.to_string())?;
```

`login_id` regenera o id da sessão (evitando fixação de sessão) e
rotaciona o token CSRF, e então escreve o id na sessão. Isso é
deliberado: versões anteriores faziam no-op silenciosamente fora de um
escopo de sessão em vez de falhar de forma explícita, e a auditoria
corrigiu isso - um "login bem-sucedido" que nunca se efetivou é o tipo
de bug que nada mais pega.

## `Auth::user()` e `user_as<T>`

`Auth::user()` retorna o usuário por trás da trait:

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

Esse trait object cobre qualquer um que implemente `Authenticatable`.
Para obter de volta o seu `User` concreto, faça downcast através de
`user_as::<T>()`:

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // Acesso direto a campo no model.
    println!("Welcome, {}!", user.name);
}
```

`user_as` retorna `Ok(None)` tanto quando nenhum usuário está
autenticado *quanto* quando o usuário resolvido não é um `T` (por
exemplo, um `Auth::set_user(...)` de um tipo diferente em algum outro
lugar da pilha). Dentro de uma solicitação, o usuário é cacheado por
solicitação, então chamar `Auth::user()` repetidamente só bate no
provedor uma vez.

## Guards nomeados

Os métodos `Auth::*` nus falam com o guard padrão. Para agir contra um
guard específico, resolva-o pelo nome:

```rust
use suprnova::Auth;

// Operações somente leitura funcionam em todo driver.
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attempt precisam de um guard stateful. Guards de token falham de forma explícita aqui.
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` retorna `Arc<dyn Guard>` (o contrato de leitura) e
`Auth::stateful_guard("name")` retorna `Arc<dyn StatefulGuard>`
(adiciona `attempt`/`login`/`logout`). Pedir o contrato stateful em um
guard de token retorna um erro com uma mensagem de remediação, em vez
de limitar silenciosamente a API.

## Provedores de usuário

Um `UserProvider` diz à pilha de auth como buscar e validar usuários.
Dois provedores já vêm embutidos, então o caso comum não precisa de
implementação customizada:

- **`EloquentUserProvider<M>`** - resolve através de um `User` tipado
  com `#[suprnova::model]` que também é `Authenticatable`. Busca por
  chave primária para ids, por `email` (padrão) para credenciais.
- **`DatabaseUserProvider`** - resolve uma tabela bruta pelo nome em um
  `GenericUser` (id + mapa de atributos). Use-o quando você não tem, ou
  não quer, um model tipado.

Ambos filtram buscas de credencial contra uma allowlist (padrão
`["email"]`) - um mapa de credenciais hostil não consegue injetar
predicados `WHERE` extras. Customize a allowlist com
`.credential_columns([...])`, a coluna de busca com
`.identifier_column("uuid")`, ou a estratégia de binding de id com
`.with_id_parser(...)`.

Para plugar uma fonte customizada (LDAP, uma API externa), implemente
`UserProvider` diretamente. `retrieve_by_id` recebe o identificador
como um `&str`:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … busca no LDAP, retorna como Arc<dyn Authenticatable>
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials têm padrões de trait
    // que retornam None / false. Sobrescreva-os para suportar
    // `Auth::attempt` e `Auth::validate` contra a sua fonte.
}
```

Registre-o no manager:

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## Protegendo rotas

### `AuthMiddleware`

Bloqueia rotas só-para-autenticados. Solicitações não autenticadas são
redirecionadas para uma página de login ou recebem `401`:

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` retorna `401 Unauthorized` em vez disso -
melhor para APIs JSON. `AuthMiddleware::redirect_to("/login")` emite um
`302` para solicitações normais e um `409 X-Inertia-Location` para
solicitações Inertia (que o cliente Inertia transforma em uma visita de
página completa). Para bloquear em um guard específico, encadeie
`for_guard`:

```rust
// 401 a menos que o guard api esteja autenticado.
.middleware(AuthMiddleware::new().for_guard("api"))
```

Um guard de token (`for_guard("api")`) depende de qualquer middleware
de bearer-token que execute mais cedo na chain para popular o id de
auth da solicitação; sem ele, o guard sempre relata não autenticado.

### `GuestMiddleware`

O inverso - para páginas de login e registro que usuários autenticados
não deveriam ver:

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` funciona da mesma forma que
`AuthMiddleware::for_guard`.

### `BasicAuthMiddleware`

Auth HTTP Basic a partir do header `Authorization: Basic` contra o
provedor de um guard:

```rust
use suprnova::BasicAuthMiddleware;

// Stateful - loga o usuário na sessão em caso de sucesso (o `basic` do Laravel).
.middleware(BasicAuthMiddleware::new())

// Stateless - autentica só para esta solicitação (o `onceBasic` do Laravel).
.middleware(BasicAuthMiddleware::once())
```

O username decodificado é comparado contra a credencial `field`
(padrão `"email"`); um header ausente, malformado, ou inválido retorna
`401` com um desafio `WWW-Authenticate: Basic realm="..."`. Configure
com `.field(...)`, `.realm(...)`, e `.for_guard(...)`.

## Eventos de ciclo de vida

Os guards despacham cinco eventos de ciclo de vida. Escute-os via a
[`EventFacade`](events.md):

| Evento | Quando |
|---|---|
| `Attempting` | uma tentativa de credencial começa (`attempt`/`once`) |
| `Authenticated` | um usuário é ativamente autenticado nesta solicitação (`login`/`once`/`once_using_id`) |
| `Login` | um usuário é persistido na sessão (`login`/`attempt` bem-sucedido) |
| `Logout` | um usuário é deslogado |
| `Failed` | uma tentativa de credencial falha (senha errada ou id desconhecido) |

Todo evento carrega o nome do guard e um id de usuário em string -
nunca a senha em texto puro e nunca o mapa de credenciais bruto.
`Authenticated` dispara só quando um usuário é ativamente estabelecido,
não em uma resolução passiva de `Auth::user()` a partir de uma sessão
existente, então listeners não recebem um fluxo de duplicatas em toda
solicitação autenticada.

## O fluxo de login com scaffold

`suprnova new` gera um controlador de autenticação que usa
`Auth::attempt` contra o provedor registrado. `FormRequest` e `Validate`
produzem o envelope de validação `{ message, errors }`. Para uma solicitação
Inertia, o middleware de redirecionamento de validação instalado transforma essa
falha em um redirecionamento HTTP `303 See Other` de volta e armazena os erros em
flash para a página de origem. Um cliente não Inertia recebe o envelope JSON HTTP
`422 Unprocessable Entity`:

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

O registro segue o mesmo formato: valide o formulário, crie o usuário,
então `Auth::login(Arc::new(user), false).await?` loga o usuário
recém-criado na sessão e dispara o evento `Login`.

## O model `User` com scaffold

O `User` gerado é um `#[suprnova::model]` que implementa
`Authenticatable`. Ele também contém `email_verified_at: Option<DateTime<Utc>>`
e implementa `MustVerifyEmail` e `CanResetPassword`. Essas pontes permitem que
`EloquentUserProvider<User>` marque a verificação de email e forneça dados de
identidade para a redefinição de senha. O trecho abaixo mostra apenas os campos e
helpers de login do guard; use o template de model gerado para a implementação
completa de fluxos de autenticação. Seus helpers de senha usam o módulo
[`hashing`](hashing.md):

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

O atributo `hidden = ["password", "remember_token"]` faz o model pular
essas colunas ao serializar para JSON para o wire - elas existem na
struct, mas nunca vazam através de uma resposta Inertia.

## Remember-me

Quando um engine Magnetar está instalado,
`Auth::attempt(credentials, true)` e `Auth::issue_remember_cookie`
emitem credenciais de remember do Magnetar associadas à finalidade. O
navegador ainda recebe o cookie `remember_me` criptografado do framework,
enquanto o Magnetar é dono do armazenamento de verificadores, das
verificações de época de autenticação, rotação de uso único, tratamento
de anomalias e revogação.

Em uma solicitação sem login ativo do framework, `SessionMiddleware`
consome o cookie pelo engine instalado, rotaciona a credencial de
remember, emite uma sessão Magnetar nova e vincula ambas as camadas de
sessão. Uma época de autenticação obsoleta, sessão de conta revogada,
credencial malformada ou replay não autentica a solicitação.

`Auth::revoke_remember_tokens()` invalida toda credencial de remember do
usuário atual. O cookie de limpeza é enfileirado antes da revogação do
backend, portanto o navegador descarta sua credencial mesmo quando a
operação de armazenamento falha.

Quando nenhum engine Magnetar está instalado, o framework mantém o
fallback legado `remember_tokens` para compatibilidade. Novas aplicações
devem inicializar o Magnetar em vez de depender desse fallback.

## Garantias de segurança

Uma lista curta de invariantes que a pilha de auth estabelece:

- **`Auth::login_id` falha de forma explícita fora de um escopo de
  solicitação.** Versões anteriores descartavam silenciosamente a
  escrita de sessão; um "login bem-sucedido" que nunca se efetivou é o
  tipo de bug que nada mais pega.
- **O id de sessão e o token CSRF regeneram em todo login.** Tanto
  `login_id` quanto o `login`/`attempt` apoiado em guard os rotacionam
  para prevenir fixação de sessão.
- **O logout limpa o estado de auth antes de revogar o remember-me.**
  Se a revogação no BD falhar, a sessão já está em um estado deslogado,
  então um slot de auth obsoleto não pode sobreviver a um logout
  parcial. O cookie de limpeza do remember-me é enfileirado *antes* do
  delete no BD, então o navegador descarta o cookie mesmo quando o
  delete da linha falha (a limpeza de prune resolve depois).
- **Allowlists de credencial bloqueiam injeção.** Ambos os provedores
  embutidos filtram `retrieve_by_credentials` contra
  `credential_columns`, então chaves extras em um mapa de credenciais
  influenciado por um atacante não podem se tornar predicados `WHERE`
  extras.
- **Eventos de auth nunca carregam texto puro.** Nome do guard + id de
  usuário em string, nada mais. O rastreamento de tentativas falhas
  (bloqueios indexados por email) pertence ao `BruteForce` em
  [Fluxos de autenticação](auth-flows.md), não aos eventos de ciclo de
  vida.
- **Escritas de credencial são delimitadas pelo ator.** Mutações de
  senha, passkey, conta vinculada, dois fatores, sessão e remember
  carregam o ID de usuário e a época de autenticação estabelecidos pela
  autenticação verificada. Uma revogação ou mudança de época da primeira
  prova faz uma escrita obsoleta em andamento falhar.
- **A primeira prova de caixa postal é atômica.** Em uma conta não
  verificada, a redefinição de senha, o consumo de magic link ou a
  conclusão de email verificado por OAuth avançam a época de autenticação
  e removem credenciais provisórias na mesma transação. Uma escrita
  concorrente de ocupante indevido não pode restaurar acesso depois do
  commit.
- **A verificação de email é vinculada ao ator.** A facade de
  verificação do framework exige um usuário autenticado cujo ID
  corresponda ao dono do token. Um token de outra conta é rejeitado sem
  ser consumido.
- **O email OAuth não é propriedade da conta.** Uma conta existente não
  verificada nunca é vinculada automaticamente apenas a partir de um
  email de provider. Contas verificadas exigem vinculação explícita;
  contas não verificadas exigem o caminho de conclusão da primeira prova
  de email.

O capítulo [Sessões](session.md) cobre a configuração de cookie
(`SESSION_LIFETIME`, `SESSION_COOKIE`, `SESSION_SECURE`,
`SESSION_SAME_SITE` e `SESSION_COOKIE_PREFIX`) que os guards apoiados em
sessão herdam.

## Próximos passos

- [Fluxos de autenticação](auth-flows.md) - verificação de email,
  redefinição de senha, lockout de conta apoiado no Magnetar, TOTP 2FA do
  framework e eventos de fluxo de autenticação
- [OAuth e login sem senha](oauth.md) - OAuth do Magnetar, Apple, magic
  links, política de provider e migração de dados de autenticação
- [Autorização](authorization.md) - `Gate`, políticas e `Authorizable`
- [Sessões](session.md) - a camada de sessão e cookies do navegador
- [Proteção CSRF](csrf.md) - proteção de solicitações que alteram estado
- [Hashing](hashing.md) - helpers de bcrypt e Argon2
