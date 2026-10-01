# Vindo do Rust web

Você despachou serviços Rust em Axum, Actix, Rocket, ou hyper feito à mão.
Você conhece a linguagem e o runtime. O que o Suprnova realmente oferece
a você?

**A camada de produtividade.** Roteamento, controladores, uma ORM, migrações,
filas, agendamento, autenticação, mail, notificações, transmissão, cache,
armazenamento, validação e uma ponte frontend tipada - tudo integrado,
usando as mesmas convenções, tudo pronto para produção. Você escreve
controladores e modelos; você não escolhe o layout.

Se você já construiu um ou dois aplicativos reais em Axum, você sabe quanto
daquele esforço foi integração em vez de funcionalidades. Suprnova é a integração,
feita uma vez, opinada onde opinião importa, plugável onde não importa.

## Resumo de 30 segundos

```bash
suprnova new myapp --frontend svelte    # cria scaffold backend + SPA + Vite
cd myapp
suprnova db:sync                        # executa migrações, regenera entidades
suprnova serve                          # backend + servidor dev Vite
```

Você agora tem:

- Um servidor hyper com HTTP/1.1 e HTTP/2, upgrade de WebSocket, encerramento elegante
- Uma camada Eloquent suportada por SeaORM com relacionamentos, carregamento eager, soft deletes
- Inertia.js conectando Rust → Svelte 5 com `#[derive(InertiaProps)]` tipado
- Autenticação com guardas e middleware do framework, além de engines
  Magnetar para senha, passkey, magic link, OAuth, sessão bearer,
  bloqueio e remember
- Uma fila com drivers memory/sync/redis/database/null
- Um agendador cron conduzido pela trait `Task`
- Um binário console por projeto para `cargo run --bin console <cmd>`
- Cache, armazenamento (fs/s3/azblob/gcs), mail (SMTP + 5 provedores: SES, Mailgun, Postmark, SendGrid, Resend), web push
- Transmissão em um hub plugável (sea-streamer por padrão)
- Validação, CSRF, CORS, limitação de taxa, idempotência, timeouts de solicitação, erros estruturados

E um binário estaticamente vinculado no final de `cargo build --release`.

## O que está por baixo

| Preocupação | Crate |
|---|---|
| Servidor HTTP | `hyper` + middleware tipo tower (implementação própria) |
| Runtime assíncrono | `tokio` |
| Roteador | `matchit` |
| ORM | `sea-orm` (re-exportada como `suprnova::sea_orm`) |
| Migrações | `sea-orm-migration` |
| Drivers de banco de dados | `sqlx` (postgres / mysql / mariadb / sqlite) |
| Serialização | `serde` / `serde_json` |
| Validação | `validator` |
| Sessões de navegador | `SessionMiddleware` do framework e stores de sessão plugáveis |
| Engines de autenticação | `suprnova-magnetar` por trás de facades pertencentes ao framework |
| Templating | `tera` (para corpos de mail; frontend é Inertia) |
| Criptografia | `aes-gcm`, `argon2`, `bcrypt` |
| WebSockets | `hyper-tungstenite` |
| Streaming | `sea-streamer` (backend fanout de transmissão) |
| OAuth | Registro de provedores e engine de cerimônias do Magnetar |
| Rastreamento | `tracing` + `tracing-subscriber` |

Você normalmente não vai alcançar nenhum desses diretamente - Suprnova
re-exporta o que você precisa. SeaORM é a passagem mais profunda: `Entity`,
`Column`, `ActiveModel`, `ConnectionTrait`, o construtor de consultas, o
prelude de migração. A válvula de escape é `use suprnova::sea_orm;` se você
precisar de algo que a superfície curada não cobre.

## O que Suprnova adiciona ao Axum puro

Axum é excelente. Actix também é. Rocket também é. A razão pela qual Suprnova
existe não é que esses frameworks sejam ruins - é que todo time construindo um
produto real neles acaba re-implementando a mesma camada de produtividade.
Suprnova fornece essa camada:

| Capacidade | Fazer manualmente em Axum | Em Suprnova |
|---|---|---|
| Macros de roteamento que escalam para centenas de rotas | Builder API, pode ficar ruidosa | Macro `routes!` com agrupamento, prefixos, middleware, nomeação |
| Vinculação de modelo de rota (id de caminho → modelo carregado) | Extrator customizado por tipo | `#[handler]` resolve `post::Model` de `{id}` automaticamente |
| Construtor de consultas encadeável tipo Eloquent | Use SeaORM diretamente | `Post::query().db_where(...).order_by(...).get().await?` |
| Soft deletes, observers, eventos de ciclo de vida | Construir por modelo | `#[model(soft_deletes)] + impl Observer<Post>` |
| Migrações + geração de entidades | Integre sea-orm-cli + scripts | `suprnova db:sync` executa migrações e regenera entidades |
| Autenticação (sessões, provedores, guards) | Costure tower-sessions + lógica própria | `Auth::attempt`, `Auth::user`, `.middleware(AuthMiddleware)` por rota |
| Verificação de email, redefinição de senha, 2FA, força bruta | Construa todos os quatro | Todos integrados, configuráveis, idempotentes |
| Fila de background | Escolha um driver, escreva workers | `Queue::push` + `cargo run -- queue:work` |
| Agendamento cron | Escreva uma tarefa tokio com `tokio_cron_scheduler` | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Ponte Inertia | Construa extractors + um adaptador JS | `inertia_response!(&req, "Page", props)` |
| Props frontend tipadas (Rust → TS) | Escreva um gerador | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| Transmissão (canais públicos / privados / presença) | Integre um backend de streaming + autenticação | Traits `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel` |
| Mail com múltiplos provedores | Escolha um, escreva sua própria abstração | `Mail::driver("ses")` etc., API `Mailable` uniforme |
| WebPush | Leia a spec, construa um notificador | `WebPushChannel` é fornecido, VAPID integrado |
| Validação + form requests | Use `validator` + extrator customizado | Form requests `#[derive(Data, Validate)]`, validação async |
| Recursos JSON:API | Formate respostas manualmente | `#[derive(Resource)]` |
| Limitação de taxa com política fail-open/closed | Construa | `RateLimiter` + `BackendErrorPolicy` |
| Chaves de idempotência | Construa | `Idempotency::remember(key, ttl, body)` com replay estilo Stripe |
| CSRF (com exclusões glob estilo Laravel) | Construa | `CsrfMiddleware` com `except` + `except_method` |
| Erros estruturados com 5xx sanitizados | Construa | `FrameworkError` / trait `HttpError`, recuperação de pânico |
| Contêiner com escopos task-local → thread-local → global | Escreva o seu | `App::bind` / `singleton` / `factory` com isolamento apropriado |
| Health endpoint, request id, logging estruturado | Costure junto | Todos ativados por padrão |

O trade-off é opiniões: Suprnova escolhe um layout, escolhe um driver padrão,
escolhe uma convenção de nomenclatura. Você pode divergir (drivers são plugáveis,
config é sobrescritível, o contêiner permite trocar serviços), mas os padrões
são projetados para serem a escolha certa para "construir um produto rapidamente".

## Padrões Rust familiares

Você vai reconhecer as formas:

```rust
// Um handler retorna `Result<HttpResponse, HttpResponse>` (aliasado como Response).
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// Middleware é uma trait, não uma closure:
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// Trabalho de background é a trait `Job` - `handle(self)` executa o trabalho:
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

Se você está acostumado com middleware Tower: middleware Suprnova é
conceitualmente o mesmo (um wrapper em torno de `next`), mas usa sua própria
trait (não a `Service` de Tower) porque os tipos combinadores de tower ficam
desagradáveis quando você começa a aninhar extractors específicos da aplicação.
A forma é mais simples; o modelo mental é o mesmo.

Se você usou o padrão de extrator do Axum: a macro `#[handler]` do Suprnova
faz o mesmo papel, mas resolve através do contêiner de serviço em vez de via
traits, o que permite injetar serviços da aplicação assim como dados de
solicitação. Vinculação de modelo de rota (`Post` de `{id}`) é integrada.

Se você usou `sqlx` diretamente: a ORM do Suprnova fica em cima de SeaORM,
que fica em cima de sqlx. Você pode cair para SQL puro via `DB::select(...)` /
`DB::select_one(...)` ou usar `DB::table("name")` para consultas dinâmicas
encadeáveis; você pode cair direto para SeaORM para coisas que a superfície
Eloquent não cobre (ex: queries `Statement` puras com mapeamento de resultado
customizado). O [capítulo Eloquent](eloquent.md) cobre as válvulas de escape.

## Qual é o delta de produtividade?

Escolha uma funcionalidade que você já construiu antes em Axum puro. Suprnova
a fornece como um capítulo:

- **"Construí um sistema de autenticação uma vez e levou duas semanas."** →
  [Autenticação](authentication.md) + [Fluxos de autenticação](auth-flows.md). Defina
  a migração, configure o guard, você terminou.
- **"Escrevi meu próprio worker de fila com retry/backoff."** →
  [Filas](queues.md). `Queue::push` + `cargo run -- queue:work`.
- **"Integrei WebSockets com hyper-tungstenite uma vez."** →
  [WebSockets](websockets.md). A macro `ws!()` tipa o handler;
  o upgrade, heartbeat ping/pong, handshake de close-frame e
  back-pressure são cuidados.
- **"Construí um adaptador Inertia do zero."** →
  [Inertia](frontend.md). `inertia_response!(&req, "Page", props)`, com
  `InertiaProps` gerando os tipos TS.
- **"Construí um limitador de taxa por tenant."** →
  [Limitação de taxa](rate-limiting.md). Chave configurável, política
  fail-open vs fail-closed configurável, fail-closed retorna 503.
- **"Implementei verificação de assinatura de webhook Stripe + proteção contra replay."** →
  [Pagamentos: Stripe](payments-stripe.md). Integrado no adaptador,
  webhooks vão para uma tabela espelho com idempotência UNIQUE.

O que você construiria manualmente em duas semanas, você importa em uma linha.

## O que você ainda vai reconhecer como "seu"

Algumas coisas permanecem perto de Rust puro porque a linguagem oferece algo
melhor que uma abstração de framework:

- **Primitivas de concorrência.** `tokio::spawn`, `Arc`, `Mutex`, channels -
  use-as. O framework não as encapsula.
- **Tipos de erro.** Você define seus erros de domínio. Implemente a
  trait `HttpError` neles para obter um status code e mensagem apropriados
  na resposta no fio. O `FrameworkError` e `AppError` do framework
  são válvulas de escape para erros transversais e ad-hoc respectivamente.
- **Drivers customizados.** Cache, fila, mail, transmissão, vector, pagamentos -
  todo subsistema de "registro de driver" aceita drivers customizados. Implemente
  a trait, registre em `bootstrap.rs`, pronto.
- **SQL puro quando você quiser.** `DB::select(...)`, `DB::table(...).get()`
  para linhas dinâmicas, ou caia totalmente para SeaORM. A ORM sai do caminho.
- **Seu próprio middleware tower?** Suprnova não fornece um adaptador
  Tower - middleware aqui é `impl Middleware`, não `tower::Service`.
  Se você precisar trazer um crate apenas Tower, você o adaptaria manualmente.
  Na prática, o sistema de middleware integrado cobre quase tudo que
  você buscaria. Veja [Middleware](middleware.md).

## O que você abre mão

Honestidade importa mais que marketing:

- **Convenções.** Modelos vivem aqui, controladores ali, migrações
  ali, observers ali. O scaffolder escolhe. Você pode lutar; você
  provavelmente não deveria. As convenções são do Laravel, auditadas e
  testadas em batalha.
- **Alguma flexibilidade em como a solicitação flui.** A cadeia de
  middleware tem uma ordem mais externa fixa (request-id → globals → middleware
  de rota → handler). Você pode inserir middleware em qualquer lugar nisso,
  mas você não pode mover as camadas request-id ou panic-recovery - elas
  são invariantes.
- **Os cantos em forma de PHP.** Onde Laravel faz algo porque PHP,
  Suprnova faz a coisa em forma de Rust em vez disso - mas informamos
  quando. Procure por callouts **"Por que Suprnova diverge"** em capítulos.

## Por que "inspirado em Laravel" deveria importar para você mesmo que nunca tenha escrito PHP

O ecossistema web Rust está aproximadamente onde o PHP estava por volta de 2009.
Os crates existem; os padrões não. Suprnova porta um conjunto extremamente refinado
de padrões de um framework que teve 10+ anos de pressão de produção moldando-o.
Você obtém padrões que já sobreviveram ao contato com a realidade.

O custo é que Suprnova *é opinada*. Se você quer um framework minimal
"escolha-tudo-por-si-mesmo", Axum está bem ali e é excelente. Se você quer
um "framework que decide coisas para que você possa focar no produto", esse é
o Suprnova.

## Próximos passos

- [Instalação](installation.md) - `suprnova new`, o que é criado com scaffold
- [Início rápido](quickstart.md) - construa um aplicativo pequeno em 5 minutos
- [Ciclo de vida da solicitação](lifecycle.md) - como uma solicitação flui, o que executa onde
- [Contêiner de serviços](container.md) - como serviços são vinculados e resolvidos
- [Eloquent](eloquent.md) - o capítulo mais longo; a superfície é ampla

Ou salte para qualquer lugar via [`documentation.md`](documentation.md).
