# Inicialização da aplicação

`bootstrap.rs` é o único lugar onde a sua aplicação monta a própria fiação
de inicialização. Vinculações do contêiner, event listeners, observers,
supervisors, middleware global - qualquer coisa que deva existir antes que a
primeira solicitação chegue ao servidor (ou o primeiro job saia da fila) é
registrada aqui. Não há scaffold de service-provider para montar.

Há dois hooks, não um. `register` vale para todo o processo: todo
subcomando o executa, incluindo `queue:work`, `schedule:work`,
`workflow:work` e seu binário de console, não apenas o servidor. Registre
a conexão de banco de dados, vinculações do contêiner, event listeners,
observers, supervisors e registro de jobs de worker ali.
`register_http_stack`, conectado por `.http_bootstrap`, executa apenas no
caminho do servidor (`serve` / `web:run`) - middleware global e
`Inertia::install` pertencem a ele. A seção "Onde a inicialização se
encaixa na ordem de boot" abaixo explica por que a divisão existe.

## A estrutura

O ponto de entrada de uma app com scaffold constrói uma [`Application`](lifecycle.md)
de forma fluente e a executa. A inicialização são dois métodos no builder:

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`, não `#[tokio::main]`

O atributo não é cosmético, e trocá-lo de volta quebra a inicialização com
uma mensagem explicando o motivo.

Carregar o `.env` escreve no ambiente do processo, e `set_var` é seguro
apenas enquanto o processo é single-threaded. O `#[tokio::main]` constrói o
runtime *em torno* de todo o `main`, então toda worker thread já
existe antes que sua primeira instrução seja executada - e qualquer uma delas pode chamar
`getenv` indiretamente através de resolução DNS, formatação de horário, ou uma
dependência C. A corrida é silenciosa quando dá errado, que é a pior
propriedade que uma corrida pode ter.

`#[suprnova::main]` mantém a mesma `async fn main` que você escreveria de
qualquer forma e simplesmente reordena duas coisas: carrega o ambiente,
depois constrói o runtime, depois executa seu corpo nele. Ele aceita os
mesmos argumentos `flavor` e `worker_threads` que `#[tokio::main]`.

Se `Application::run` descobrir que o ambiente nunca foi carregado a partir
de um contexto single-threaded, ele se recusa a inicializar em vez de emitir
um aviso - uma app que inicia "bem" sob `#[tokio::main]` é precisamente a
que corrompe uma leitura de ambiente não relacionada semanas depois.

O framework chama sua `bootstrap_fn` uma vez durante a sequência de boot,
depois que o ambiente é carregado e depois que os drivers de runtime
(Cache, Queue, RateLimit, Mail) estão de pé, mas antes que o router seja
construído. A mesma chamada executa para workers em background
(`queue:work`, `workflow:work`, `schedule:work`) para que um observer ou
listener registrado aqui dispare identicamente para um insert de um queue
job e um insert de um handler HTTP. `http_bootstrap_fn` executa
imediatamente depois de `bootstrap_fn`, mas apenas no caminho do servidor -
workers em background e o binário de console nunca o chamam.
[Ciclo de vida da solicitação](lifecycle.md) percorre a sequência completa.

As assinaturas das duas funções são fixadas por
`Application::bootstrap` e `Application::http_bootstrap`:

```rust
// src/bootstrap.rs
pub async fn register() {
    // banco de dados, vinculações, observers, listeners, supervisors,
    // registro de jobs de worker
}

pub fn register_http_stack() {
    // middleware global, Inertia::install
}
```

`register` retorna `()`; `register_http_stack` é síncrona, não `async` -
ambas são conectadas como closures assíncronas no call site
(`.http_bootstrap(|| async { bootstrap::register_http_stack() })`) porque
um ponteiro de função simples também pode servir como ponto de entrada de
harness de teste sem levar `async` para o teste. Setup falível usa
`.expect("…")` com uma mensagem que explica a remediação - a inicialização
é o momento certo para falhar de forma explícita. A chamada da app de
exemplo é `DB::init().await.expect("Failed to connect to database");`
então uma `DATABASE_URL` ausente aborta o processo na inicialização com o
erro real impresso, em vez de aparecer como um confuso "connection
refused" na primeira solicitação.

## O que vai na inicialização

Uma função `bootstrap` real faz um pequeno número de coisas distintas.
Cada subseção abaixo é uma delas. O `app/src/bootstrap.rs` da app de exemplo
exercita todas elas e é a referência funcional.

### Conexão com o banco de dados

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` lê `DatabaseConfig` (registrado pelo seu `config_fn`) e
abre o pool. A conexão é armazenada no [contêiner](container.md)
como um singleton - `DB::connection()` / `DB::get()` a resolve em
qualquer lugar. `DB::init_with(config)` é a válvula de escape de
teste e ferramentas quando você quer apontar para algo diferente da
URL derivada do ambiente.

### Engine de autenticação Magnetar

Aplicações que usam as facades integradas de senha, passkey, magic link,
bearer, bloqueio, remember ou OAuth inicializam o Magnetar depois que o
banco de dados e a `APP_KEY` estiverem prontos:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

O `MagnetarConfig` padrão vincula identidades de aplicação à tabela canônica
`app_users`. O scaffold full-stack gerado usa um model `users` e não inicializa
o Magnetar, portanto não adicione o inicializador padrão sem modificá-lo a esse
scaffold. Use o model `app_users` do scaffold de API ou construa um
`MagnetarHostEngine` e um binding `AuthSchema` personalizados para sua tabela
`users` existente. Mantenha o `UserProvider` do framework e o binding de host
do Magnetar na mesma identidade de aplicação. O scaffold de API, não
`app/src/bootstrap.rs`, é a referência de trabalho atual para a inicialização
padrão de `MagnetarConfig`.

O Magnetar vale para todo o processo porque workers de fila, schedulers,
handlers HTTP e middleware de sessão usam os mesmos stores de credenciais e
sessão. Coloque `init_magnetar` em `register`, não em
`register_http_stack`. O instalador é de uso único e falha se outro engine
já estiver instalado.

O scaffold de API lê `PASSKEY_RP_ID` e `PASSKEY_RP_ORIGIN` no bootstrap da
aplicação. Esses nomes são convenções do scaffold, não variáveis de
ambiente pertencentes ao framework.

### Middleware global

O middleware global é somente HTTP, então pertence a
`register_http_stack`, não a `register`:

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` registra uma camada que executa em toda solicitação,
incluindo as não roteadas (404s, preflight OPTIONS). A ordem em que você
registra é a ordem em que a chain executa - de fora para dentro. O framework
encaixa o próprio `RequestIdMiddleware` na posição mais externa; tudo que você
adiciona fica dentro dele. [Middleware](middleware.md) explica a forma completa
da chain, incluindo a camada por rota.

### Vinculações do contêiner

O contêiner aceita o que quer que você coloque nele; as macros são açúcar
sintático sobre a fachada [`App`](container.md).

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // Trait → singleton (envolve em Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Singleton concreto:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Factory (construída a cada resolve):
    factory!(|| RequestLogger::new());

    // Ou chame a fachada diretamente para controle mais fino:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

Vinculações de trait object são a forma mais comum - vincule uma interface,
deixe handlers e testes substituírem a implementação. O capítulo
[Contêiner de serviços](container.md) tem a API de vinculação completa
incluindo `bind_factory!`, as variantes `_if_absent`, e o modelo de lookup
de três camadas.

### Event listeners e observers

O dispatcher está ativo assim que a inicialização executa - listeners
registrados aqui veem todo dispatch subsequente.

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Observers Eloquent (`#[suprnova::observer(M)]`) se coletam via
`inventory::submit!` em tempo de compilação. Uma chamada drena o inventário
para dentro do dispatcher:

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

A chamada é idempotente - executar a inicialização novamente (um worker que
inicializa uma segunda vez) não registra os adaptadores de listener em
duplicidade. [Eventos](events.md) cobre dispatch e autoria de listener;
[API Eloquent](eloquent.md) cobre observers.

### Supervisores

Tarefas de longa duração em background declaradas via o trait `Supervisor` e
`inventory::submit!` iniciam através de uma chamada:

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Cada supervisor executa em sua própria task de restart-loop com um limite
de panic; um supervisor que sofre panic é registrado em log e reiniciado, sem
ter permissão para derrubar o processo. Veja [Supervisores](supervisors.md)
para o trait e a política de restart.

### Registro de job de worker

Queue jobs e mailables que workers precisam despachar por nome se
registram na inicialização:

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

Sem isso, o worker não tem como mapear um envelope enfileirado de volta para
o tipo que o trata.

## O hook pós-inicialização: `booted()`

A inicialização *registra*; `booted()` *resolve*. O builder recebe um
segundo callback que dispara depois que o servidor termina seu próprio
boot de serviços mas antes que comece a aceitar conexões. Use-o quando
você precisa ler algo que o próprio framework vinculou durante o boot:

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` é síncrono e executa depois de `Server::from_config` - os drivers
estão de pé, as chaves de criptografia são carregadas, suas vinculações existem.
A maioria das apps não precisa deste hook; recorra a ele quando um efeito
colateral pós-boot de execução única precisa ver um contêiner totalmente construído.

## Um `bootstrap.rs` completo

Esta composição representativa não é um trecho literal do app de exemplo. Ela
mantém o registro de todo o processo em `register` e a configuração somente HTTP
em `register_http_stack`. A inicialização do Magnetar é mostrada separadamente
acima porque seu schema de usuário da aplicação deve corresponder ao provedor de
usuário do framework.

```rust
//! Inicialização da aplicação - registra serviços, listeners, middleware
//! global e a camada Inertia.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── Banco de dados
    DB::init().await.expect("Failed to connect to database");

    // ── Provedor de autenticação
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());


    // ── Hub de transmissão + registro de canais
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Event listeners + pontes
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Discos de armazenamento (S3 habilitado por env em produção)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Registro de job de worker
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observers + supervisores
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Sinalizadores de recursos
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── Middleware global (de fora para dentro na ordem de registro)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Camada de protocolo Inertia (sem fixar versão: o padrão calcula
    // hash do manifesto de build do Vite, então um build do frontend
    // atualiza a versão do asset por conta própria - veja "Detecção da
    // versão" em frontend-inertia-responses.md)
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

Note o ritmo: cada bloco faz uma coisa, chama uma ou duas APIs, e ou tem
sucesso ou falha com uma mensagem clara. Nada aqui é sofisticado; as funções
são longas porque a app tem muitas partes móveis, não porque o padrão de
inicialização é complicado.

## Quando usar a inicialização vs. `#[injectable]`

`#[injectable]` é uma macro que auto-registra um singleton no
`inventory` do contêiner em tempo de compilação. É a escolha certa para
serviços que não precisam de nada além de suas dependências `#[inject]` para
serem construídos:

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

Eles se resolvem por conta própria; a inicialização não precisa tocá-los.

A inicialização é o lugar certo quando a construção precisa de qualquer outra
coisa - uma variável de ambiente, uma struct de config construída, uma
vinculação `dyn Trait`, uma decisão em runtime, uma chamada de setup
assíncrona, ou o registro de algo que não é em si um serviço (um listener,
um observer, um mapeamento de queue job, uma camada de middleware global).

| Use `#[injectable]` para | Use `bootstrap` para |
|---|---|
| Singletons concretos sem config em runtime | Qualquer `dyn Trait` |
| Serviços construídos a partir de outros injectables | Qualquer coisa async na inicialização |
| Grafo de DI padrão | Valores derivados do ambiente |
| | Event listeners, observers, supervisores |
| | Middleware global |
| | Registro de job de worker + mailable |

Você pode misturar livremente. Serviços `#[injectable]` já estão visíveis no
contêiner no momento em que a inicialização executa, então uma vinculação na
inicialização pode lê-los.

## Onde a inicialização se encaixa na ordem de boot

A sequência completa (extraída de [Ciclo de vida da solicitação](lifecycle.md)):

1. `Config::init(".")` - carrega o `.env`, detecta o ambiente
2. `init_policies()` - drena o inventário `#[policy]`
3. Seu `config_fn` executa (registro de config tipado)
4. Migrations executam (auto-migrate em `serve`)
5. **Sua `bootstrap_fn` executa** ← `bootstrap::register`
6. **Sua `http_bootstrap_fn` executa, apenas no caminho do servidor** ← `bootstrap::register_http_stack`
7. Rotas montadas a partir do seu `routes_fn`
8. `Server::from_config` inicializa drivers + contêiner
9. Seus `booted_fn`s disparam
10. O servidor começa a aceitar conexões

Workers em background (`queue:work`, `workflow:work`, `schedule:work`) e o
binário de console compartilham as etapas 1–5 e 8 - executam
`bootstrap_fn`, mas nunca a etapa 6, pois apenas `serve` / `web:run`
executa `http_bootstrap_fn`. Isso permite que um listener ou observer
registrado em `register` alcance os caminhos de código de worker exatamente
como alcança handlers HTTP, enquanto o middleware global e
`Inertia::install` de `register_http_stack` ficam fora de processos que
nunca servem HTTP.

### Por que Suprnova diverge

O Laravel executa `register()` e `boot()` de cada service provider também
para comandos `artisan` e workers de fila, não apenas para solicitações HTTP -
e consegue fazer isso porque sua integração Vite resolve URLs de asset
preguiçosamente, no momento da renderização, a partir da diretiva `@vite`
Blade que for solicitada. Um worker que nunca renderiza uma view nunca toca
no manifesto, então um build ausente simplesmente não aparece.

O `Inertia::install` do Suprnova resolve o manifesto uma vez, no boot, e
falha de forma fechada em produção quando ele está ausente - por design,
para que um deploy mal configurado não possa servir URLs de asset apontando
para um servidor Vite que ninguém executa. Essa escolha de design é
exatamente o que quebra uma imagem de worker ou console que (corretamente)
não envia `public/assets`: a falha que o Laravel adia para o momento da
solicitação, o Suprnova atingiria no início do processo, em todo subcomando.
Dividir a superfície de boot entre `bootstrap` e `http_bootstrap` mantém a
verificação de falha fechada, mas apenas onde ela pertence - no caminho do
servidor que realmente renderizará uma página Inertia.

O Laravel também divide o próprio boot entre vários service providers:
cada provider implementa `register()` e `boot()`, eles são coletados em
`config/app.php`, e o Laravel os percorre em duas passadas (todos os
`register`, depois todos os `boot`) para que um serviço possa depender das
vinculações de outro provider sem cerimônia de ordenação no código do
usuário. A classe provider oferece uma unidade de organização quando uma app
acumula dezenas de subsistemas distintos.

O Suprnova reduz isso a duas funções - `register` e
`register_http_stack` - em vez de um par `register`/`boot` por provider. As
razões:

- **A divisão em duas passadas `register`/`boot` resolve um problema de
  ordenação que o Rust não tem.** `#[injectable]` e o
  `bootstrap_singletons` do contêiner já resolvem grafos de dependência
  sem ordenação visível ao usuário. Vinculações se registram inline; a
  maquinaria de lookup cuida do resto.
- **Duas funções são mais fáceis de ler do que dez.** Um novo contribuidor
  abre `bootstrap.rs` e vê cada vinculação, cada listener, cada observer e
  cada camada de middleware em um dos dois lugares. A fragmentação estilo
  provider esconde o que a app realmente faz.
- **Auto-registro estilo inventory cobre o resto.** Observers,
  supervisores, tarefas agendadas, políticas e queue handlers todos se
  coletam em tempo de compilação via `inventory::submit!`. A inicialização
  drena os inventários com chamadas únicas (`bootstrap_observers`,
  `SupervisorRegistry::start_all`) em vez de enumerar cada um.

Onde o Laravel ganha com a divisão em providers é na distribuição de
bibliotecas: um crate que traz suas próprias vinculações ia querer um ponto
de entrada de registro no qual uma app pudesse optar por entrar sem editar
sua própria inicialização. O análogo do Suprnova é uma
`pub async fn register()` pública na raiz do crate e uma chamada de uma
linha a partir da `bootstrap` da app. O custo ergonômico é uma linha; o
ganho em legibilidade é tudo estar em um só lugar.

## Próximos passos

- [Ciclo de vida da solicitação](lifecycle.md) - a ordem de boot completa
  e onde a `bootstrap_fn` dispara
- [Contêiner de serviços](container.md) - `App::bind` / `App::singleton` /
  `App::factory` e o lookup de três camadas
- [Configuração](configuration.md) - registro de config tipado que
  executa antes da inicialização
- [Middleware](middleware.md) - composição de chain para camadas
  registradas com `global_middleware!`
- [Eventos](events.md) - o dispatcher no qual listeners e observers
  se conectam
