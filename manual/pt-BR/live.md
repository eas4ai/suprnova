# Live

Suprnova Live é o motor de interação orientado pelo servidor do framework. Um
componente Live é uma struct Rust cujo estado vive no servidor, cuja view é um
template Askama e cujas ações são executadas por um protocolo assinado a partir
de um pequeno runtime de navegador que transforma no lugar o HTML
re-renderizado. Não há um modelo de estado no cliente para manter sincronizado,
nenhuma ferramenta de build para instalar a fim de usar o runtime distribuído e
nenhum JavaScript inline nos seus documentos.

Este capítulo cobre a superfície voltada à aplicação: escrever um componente,
registrá-lo, servir documentos e ilhas, os limites de segurança que toda
requisição Live atravessa, uploads, atualizações assíncronas, assets, testes,
diagnóstico e recuperação. Tudo aqui usa apenas `suprnova::live` e
`suprnova::view`.

## Início rápido

Um projeto criado com `suprnova new` já está pronto para Live: ele traz
`src/live/mod.rs` com um registro de componentes vazio e uma função
`routes()`, seu bootstrap vincula o registro e `cmd/main.rs` instala as rotas.
Gere um componente e depois verifique-o:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` escreve `src/live/counter.rs` e `templates/live/counter.html`,
registra o componente em `src/live/mod.rs` e imprime os próximos passos.
`live:check` compila sua aplicação e prova cada view registrada contra o
verificador integrado.

## Escrevendo um componente

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` é o nome registrado do componente. Use um nome com pontos em
  kebab-case como `app.counter`; a CLI deriva `<package>.<kebab>`.
- `view` é a identidade do template, relativa à raiz de templates.
- Campos `#[public]` são renderizados e transportados no snapshot assinado.
  Campos `#[model]` também aceitam propostas do navegador por meio de
  `live:model`.
- Métodos `#[action]` são os únicos pontos de entrada que o navegador pode
  invocar. Eles recebem argumentos validados e podem devolver resultados
  tipados, como um redirecionamento ou um flash.

Todo tipo de campo precisa implementar `Default`; uma ilha nova parte desses
valores padrão, a menos que um hook de montagem diga o contrário.

## Views

Views são templates Askama. A raiz de templates é `templates/`, a menos que um
`askama.toml` nomeie outros diretórios, então `live/counter.html` fica em
`templates/live/counter.html`:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

As diretivas usam a gramática fechada `live:`: `live:click`, `live:submit`,
`live:model`, `live:upload`, `live:key`, `live:loading` e o restante do
conjunto documentado. O verificador prova cada diretiva contra o componente:
uma ação desconhecida, um campo de modelo desconhecido, um filtro `safe` bruto
ou uma violação de acessibilidade faz `live:check` falhar com o arquivo, a
linha e a coluna.

Documentos que posicionam ilhas são views comuns declaradas com
`#[suprnova::view]`; o único valor sem escape que elas aceitam é `TrustedHtml`
pelo filtro `trusted_html`.

## Registro e bootstrap

`src/live/mod.rs` é dono do registro e das rotas:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Vincule-o durante o bootstrap para que o servidor, os workers e os comandos
`suprnova live:*` vejam os mesmos componentes:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

O registro é imutável depois que o runtime é montado. Um nome de componente ou
uma view duplicados, ou um componente cujas ações precisam de validação sem um
port de validação, faz o registro falhar com um `RegistryError` tipado.

## Rotas

`Router::try_live()` instala o namespace reservado exatamente uma vez:
`/__live/v1/action`, `/__live/v1/upload`, as rotas de controle e o handshake
WebSocket de `/__live/v1/async/*`, e as rotas imutáveis de
`/__live/v1/assets/*`. A inicialização falha se uma rota da aplicação puder
reivindicar `/__live`.

As rotas de requisição reservadas carregam uma política estrita: toda
requisição precisa de fatos de sessão, origem, CSRF, principal, tenant e limite
de taxa. O framework registra a sessão e a prova CSRF; sua aplicação anexa o
restante com o guarda de rotas:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::new())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Instale as rotas a partir do ponto de entrada para que o runtime e o catálogo
de montagens estejam prontos antes da primeira requisição:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Documentos e ilhas

Uma rota de documento declara suas ilhas uma vez, renderiza-as por meio de
`LiveDocument` e emite as tags de bootstrap:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` declara uma ilha que qualquer visitante pode
  renderizar; seu estado é uma semente reutilizável promovida a instância na
  primeira ação.
- `LiveMount::identity_bound` declara uma ilha que pertence à sessão e ao
  principal atuais; a rota de documento precisa autenticar.
- Monte toda ilha antes de `bootstrap`, e chame `bootstrap` uma única vez. O
  bootstrap emite o elemento de configuração inerte e as tags script para a
  estratégia ESM ou clássica, adicionando os papéis de upload e assíncrono
  quando um componente montado precisa deles e a ponte Stimulus sob demanda.
- O template do documento coloca `{{ bootstrap|trusted_html }}` em `<head>` e
  cada ilha onde ela pertence.

## Limites de segurança

Live nunca contorna o middleware do framework. O que cada requisição precisa:

| Fato | Registrado por |
|---|---|
| Sessão | `SessionMiddleware` |
| Origem e CSRF | `CsrfMiddleware` com a verificação de origem ativada |
| Principal | `AuthMiddleware` em seu ramo autenticado |
| Tenant | `LiveTenantMiddleware` com o seu resolvedor |
| Limite de taxa | `RateLimitMiddleware` em seu ramo permitido |

O runtime distribuído envia o tipo de mídia Live e o cabeçalho
`Sec-Fetch-Site` do próprio navegador; ele não carrega token de sessão.
Configure o middleware CSRF para verificar origens: uma requisição Live de
mesma origem passa com a disposição CSRF sem estado, enquanto uma requisição
entre sites ou sem cabeçalho recorre à validação por token e é recusada:

```rust
global_middleware!(CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly));
```

Visitantes anônimos podem renderizar sementes públicas, mas não executar
ações: a `AuthMiddleware` do guarda responde `401` antes de qualquer trabalho
do motor. Ilhas ligadas à identidade exigem uma sessão e um principal; o tenant
é vinculado ao escopo da ilha sempre que o seu resolvedor nomear um. Toda
recusa é fechada: um `409` para um snapshot obsoleto ou adulterado não carrega
corpo, e mensagens de produção nunca incluem snapshots, tokens, cookies ou HTML
renderizado.

## Uploads

Declare uma política de upload em um campo de modelo:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

A view vincula o campo com `<input type="file" live:upload="avatar">`. O
runtime cria, transfere e conclui o upload por `/__live/v1/upload`; o arquivo
aguarda em quarentena até a ação de finalização declarada ser executada,
quando o framework o entrega ao seu `UploadFinalizer`. Vincule o finalizador,
e qualquer scanner ou validador, antes que o runtime seja montado:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Uploads são autorizados por campo e controle através do gate. Defina as
habilidades `live:<component>.upload.<field>.<Control>` para `Create`,
`Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
`Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`
e `Fail`.

Um navegador que perdeu sua concessão de transferência a readquire por uma rota
que sua aplicação possui fora do namespace reservado:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

A rota exige os mesmos fatos de uma ação, responde apenas à sessão e ao
principal que criaram o upload, e devolve uma concessão nova com o estado
atual da transferência.

## Atualizações assíncronas

Um componente declara os streams que escuta; o runtime do navegador se inscreve
por SSE ou WebSocket e recorre ao polling como alternativa:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Defina a habilidade `live:<component>.stream.<name>` para os inscritos e então
publique de qualquer lugar da aplicação:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

Um refresh diz às ilhas inscritas para renderizarem do zero; um evento é
entregue aos handlers registrados da ilha. O polling é a renderização nova
comum, então nada se perde quando um transporte está indisponível.

## Assets e uso sem build

O framework serve os artefatos de runtime exatos e revisados em
`/__live/v1/assets/<identity>/<file>` com cache imutável, validadores fortes e
atributos de integridade nas tags de bootstrap. Uma política estrita
`script-src 'self'` se sustenta porque os documentos não contêm script inline.
Para publicar os mesmos bytes em uma CDN ou em um diretório estático:

```bash
suprnova live:assets --out public/__live
```

A publicação é atômica e se recusa a substituir um diretório cujos bytes
diferem, a menos que você passe `--replace`.

## Testes

`suprnova::live::testing` prepara o runtime e o catálogo de montagens de um
router para testes em processo. Os testes da aplicação em
`app/tests/live_*.rs` mostram o padrão completo: um banco de dados em memória,
um cookie de sessão semeado, a pilha real de middleware global e requisições
por `handle_request`:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Decodifique o snapshot de uma ilha a partir do atributo
`data-suprnova-live-snapshot`, envie uma ação com o cookie de sessão e
`Sec-Fetch-Site: same-origin`, e verifique a renderização aceita. Um snapshot
obsoleto responde `409` com corpo vazio; um principal ausente responde `401`.

## Diagnóstico e operação

- `suprnova live:check` prova cada view registrada; `--allow-unproved` aceita
  estruturas dinâmicas sobre as quais o verificador deliberadamente não se
  pronuncia.
- `suprnova live:inspect` relata o registro vinculado, os limites de
  configuração, as capacidades de upload instaladas, os serviços de runtime
  montados e a identidade dos assets sem expor estado nem segredos.
- `LiveConfig` limita os bytes de requisição e resposta e a vida útil do
  contexto confiável; vincule um personalizado antes que o runtime seja
  montado.
- Erros carregam tipos fechados como `live_document_context_rejected` e
  `invalid_live_bootstrap`; rótulos de telemetria são enumerações fechadas.

## Recuperação

- Um `409` diz ao runtime para renderizar a ilha do zero; a operação não é
  repetida.
- Um transporte assíncrono fechado é aposentado e o runtime se reconecta com
  uma nova geração de transporte; uma geração obsoleta é recusada.
- Uma sessão que expira ou rotaciona invalida o trabalho ligado à identidade;
  a aplicação expõe seu caminho de login e o visitante continua a partir de um
  documento novo.

Live funciona por completo sem RenderCache; o cache de documentos Live é uma
funcionalidade separada, com capítulo próprio quando chegar.

## Referência da CLI

| Comando | Finalidade |
|---|---|
| `suprnova live:make <name>` | Gerar um componente e sua view e registrá-lo |
| `suprnova live:check` | Provar cada view registrada com o verificador integrado |
| `suprnova live:inspect` | Relatar o estado seguro de runtime, registro, provedores e artefatos |
| `suprnova live:assets --out <dir>` | Publicar atomicamente os artefatos de runtime revisados |
