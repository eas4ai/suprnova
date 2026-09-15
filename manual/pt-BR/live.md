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
`/__live/action`, `/__live/upload`, as rotas de controle e o handshake
WebSocket de `/__live/async/*`, e as rotas imutáveis de
`/__live/assets/*`. A inicialização falha se uma rota da aplicação puder
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
            .middleware(AuthMiddleware::optional())
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
`Sec-Fetch-Site` do próprio navegador; ele não carrega token de sessão. O
middleware CSRF verifica essa prova por conta própria em toda requisição Live,
seja qual for a política de origem configurada: uma requisição Live de mesma
origem passa com a disposição CSRF sem estado, enquanto uma requisição entre
sites ou sem cabeçalho recorre à validação por token e é recusada. As rotas
comuns mantêm a validação por token sob a política padrão; usar Live não
afrouxa nada mais:

```rust
global_middleware!(CsrfMiddleware::new());
```

Visitantes anônimos renderizam sementes públicas e podem agir sobre elas
quando o guarda usa `AuthMiddleware::optional()`: um principal autenticado é
registrado, um visitante anônimo segue adiante e o tipo de montagem decide.
Uma semente pública é então promovida para a própria sessão do visitante na
primeira ação, enquanto uma ilha ligada à identidade continua recusando uma
requisição sem prova de principal. Com `AuthMiddleware::new()` o guarda
responde `401` a toda requisição anônima antes de qualquer trabalho do motor.
Ilhas ligadas à identidade exigem uma sessão e um principal; o tenant é
vinculado ao escopo da ilha sempre que o seu resolvedor nomear um, e um
resolvedor que não consiga determinar o tenant deve devolver um erro em vez de
`None`. Toda
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
runtime cria, transfere e conclui o upload por `/__live/upload`; o arquivo
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
comum: o estado da ilha se atualiza quando um transporte está indisponível,
mas as cargas de eventos publicadas nesse intervalo não são reenviadas aos
seus handlers, o que o runtime relata como um stream degradado em vez de
atual. Um componente que declara exatamente um stream tem a raiz da ilha
inscrita nele; um componente com vários streams inscreve cada um pelas
chamadas registradas do runtime.

Um stream termina com a sessão que o abriu. Quando uma sessão é destruída no
nó que mantém o stream, por um logout simples, invalidação, regeneração do id
ou um "sair de todos os lugares", toda associação que ela abriu ali é
encerrada de imediato e nenhum evento posterior a alcança. Uma sessão
destruída em outro nó é apanhada pela própria entrega: a sessão de cada
associação é conferida de novo no armazenamento de sessões no máximo uma vez a
cada dez segundos, então os eventos param dentro desse intervalo. O gate do
stream é consultado de novo antes de cada entrega de qualquer forma, de modo
que uma mudança de política encerra a entrega imediatamente em todos os nós.

## Assets e uso sem build

O framework serve os artefatos de runtime exatos e revisados em
`/__live/assets/<identity>/<file>` com cache imutável, validadores fortes e
atributos de integridade nas tags de bootstrap. Uma política estrita
`script-src 'self'` se sustenta porque os documentos não contêm script inline.
Para publicar os mesmos bytes em uma CDN ou em um diretório estático:

```bash
suprnova live:assets --out public/__live
```

A publicação é atômica e se recusa a substituir um diretório cujos bytes
diferem, a menos que você passe `--replace`.

## Biblioteca de componentes

O Suprnova traz as fundações de uma biblioteca de componentes para o Live: uma
folha de estilos de tokens com uma camada base e uma família de formulários de
componentes de apresentação construídos sobre controles nativos e o
vocabulário `live:model`, `live:error` e `live:loading`. A base é um artefato
do runtime. Se um documento optar por ela, chega como um único link de folha
de estilos sob o mesmo contrato de identidade, integridade e cache dos scripts
do runtime:

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

Toda regra nela fica dentro da camada de cascata `suprnova-ui`, de modo que
seus próprios estilos sem camada vencem sem disputa de especificidade. Todo
valor visual é uma propriedade personalizada `--sn-` para cor, fonte, espaço,
raio, sombra, movimento, densidade e estado, com valores claros e escuros:
sobrescreva um token em `:root` para mudar o tema, ou remova a camada e
mantenha todo comportamento, nome e atributo de estado, porque um componente
estiliza seus estados a partir dos atributos que o verificador prova
(`aria-invalid`, `aria-busy`, `aria-expanded`, `aria-pressed`, `aria-current`,
`aria-selected`, `:disabled`), nunca a partir de uma classe. Um preset
`@theme` do Tailwind CSS 4 mapeia os tokens para os namespaces do Tailwind; o
Tailwind nunca é obrigatório. Os componentes são instalados com `live:add`, um
diretório cada sob a raiz reservada `templates/suprnova-ui/`: a view com
macros do Askama, a folha de estilos, o JavaScript quando o componente o tem e
o manifesto que os nomeia:

```bash
suprnova live:add field
suprnova live:add password-input
```

Um arquivo que você editou é mantido em uma execução posterior; `--force` o
substitui. Um componente de terceiros é instalado a partir do seu próprio
manifesto com `--manifest`, sob a sua própria raiz. Chame as macros nas suas
views, sirva a folha de estilos e o script com `try_live_ui_assets()` e faça o
link a partir do documento:

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

O verificador expande as macros, então o `live:check` prova uma view da
biblioteca como qualquer outra. A família de formulários hoje: campo, rótulo,
entrada, área de texto, entrada numérica, controle deslizante, entrada de
busca, entrada de senha com revelação, caixa de seleção e grupo de caixas,
grupo de rádios, interruptor, seletor, botão e botão de link, grupo de botões,
fieldset, ações do formulário, resumo de validação e entrada de arquivo. Os
componentes da biblioteca chamam-se `suprnova.*` e o registro recusa esse
prefixo vindo de qualquer outro crate; os elementos personalizados são light
DOM e levam o prefixo `sn-`.

A família de overlays vem sobre as mesmas fundações: tooltip, collapsible e
accordion, popover, um menu suspenso de um único nível, dialog, sheet e drawer.
Cada um mantém seu estado aberto pela primitiva do próprio navegador antes de
qualquer script rodar: `details` para os disclosures, o atributo `popover` para
popovers e menus, e `dialog` para os três modais, que os elementos vendidos
`sn-dialog`, `sn-sheet` e `sn-drawer` abrem com `showModal()` e fecham
devolvendo o foco ao gatilho. Abrir e fechar nunca faz uma requisição Live; só
uma ação que você coloca dentro de um overlay faz. Toda raiz de overlay carrega
uma chave estável e `live:preserve.self`, então um overlay aberto sobrevive a
um morph que não substituiu sua região:

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

O atributo `popover` fixa a base suportada em Chrome e Edge 114, Firefox 128 e
Safari 17. Onde existe o posicionamento por âncora do CSS, o popover e o menu
ficam sob seu gatilho; caso contrário o navegador os centraliza. O modo de
abertura única do accordion depende de `details name`, que versões suportadas
mais antigas tratam como disclosures independentes.

Seguem a família de feedback e a família de navegação. Feedback: alert,
skeleton, spinner, progress, empty state e uma região de toasts com uma região
de flash ao lado. Cada um apresenta um estado que o servidor ou o runtime já
têm. Um alert escolhe seu papel pela variante e marca cada variante com um
glifo e um rótulo oculto, nunca só com cor. Um spinner ou skeleton é vinculado
por `live:loading.show` a uma ação registrada e entregue oculto, de modo que o
runtime o revela após seu próprio atraso e o mantém além do mínimo, e uma
ação rápida nunca o faz piscar. Progress é o elemento nativo `progress` com
rótulo e leitura em texto, e só carrega um valor para trabalho determinado. O
empty state toma seu motivo (vazio, sem resultados, sem permissão,
desconectado) do estado renderizado no servidor e só oferece uma próxima ação
onde quem chama a renderiza. Um toast anuncia uma vez a partir de uma região
de status polida e nunca toma o foco; o elemento vendorizado `sn-toast-region`
expira os toasts, pausa durante hover ou foco, limita quantos aparecem de uma
vez e responde ao botão de fechar, com cada toast com chave e preservado para
que um toast fechado continue fechado após um morph. Um erro crítico também pertence a um alert; um toast nunca é sua única superfície. Os toasts são renderizados dentro de um loop, então suas chaves passam pelo filtro `live_key`, e a island que os monta o expõe com `pub mod filters { pub use suprnova::view::filters::live_key; }`. A região de flash
renderiza uma única vez o que a requisição anterior deixou na sessão:

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

Navegação: barra de cabeçalho, footer, sidebar com grupos recolhíveis,
breadcrumbs, tabs, paginação e load more. Cada destino é uma âncora com uma
URL de rota real e cada ação é um botão; o item atual carrega `aria-current`
a partir do valor que você vincula, nunca da localização do navegador. Os
grupos da sidebar são `details` nativos, com chave e preservados. As tabs
exigem um modo: `local`, painéis com semântica de tablist, teclas de seta pelo
elemento vendorizado `sn-tabs` e nenhuma requisição na troca, ou `route`, tabs
como âncoras. A paginação também exige um modo: páginas de rota são links
canônicos, e páginas Live são botões sobre suas ações cujo resultado reflete a
nova query na entrada de histórico atual por `url_intent`, sem entrada por
página. Load more é um botão sobre uma ação registrada que acrescenta a uma
lista com chaves, de modo que o morph mantém cada linha já presente, e o controle sai da view quando você o renderiza esgotado. Uma reflexão de URL é um resultado do protocolo 2, então uma island que pagina por `url_intent` declara `minimum_protocol_version = 2`; suas linhas com chaves passam por `live_key` como um toast:

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

A família de exibição de dados fecha o conjunto embutido. Apresentacionais:
separator, scroll area, aspect image, card, badge, avatar e grupo de avatares,
list group, description list e stat card. Cada um preserva a ordem do
documento e a semântica nativa: o separator é um `hr` ou um papel separator
rotulado, a scroll area é uma região rotulada focalizável que rola
nativamente, a aspect image é o próprio `img` com uma proporção nomeada, o
card é um article ou uma section rotulada pelo próprio título com as ações em
um grupo rotulado, e a description list é um `dl`. Um badge sempre carrega
seu texto, um avatar nomeia sua pessoa no `alt` ou no rótulo das iniciais, e
a tendência de um stat card diz "Up", "Down" ou "Flat" em texto antes do
delta, de modo que nenhum status depende só da cor. O list group dá chave a
cada item por `live_key`, então uma reordenação mantém cada nó. O chart é
renderizado no servidor: a island chama `render_chart` de
`suprnova::live::charts`, que desenha marcas de barras ou linhas por
`charts-rs` a partir de séries tipadas limitadas e devolve marcação
confiável, e a macro renderiza o SVG ao lado de um resumo em texto e de uma
tabela de dados em um disclosure, de modo que o documento canônico se lê sem
a imagem e nenhum script de gráficos chega ao navegador:

```rust
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};

pub fn chart_svg(&self) -> TrustedHtml {
    render_chart(
        ChartKind::Bar,
        &["Apr", "May", "Jun"],
        &[ChartSeries::new("Revenue", vec![42.0, 47.0, 51.0])],
    )
    .expect("a bounded fixed series renders")
}
```

A datatable é o último componente, com uma island por tabela. É uma `table`
nativa com um caption que nomeia a contagem de resultados, cabeçalhos de
coluna com `scope` e `aria-sort` na coluna ordenada. Ordenar e filtrar são
submits Live nos campos model da island, mudanças de página são botões Live,
e a island declara a ordenação aplicada, a direção, o filtro e a página como
campos `#[url]` e os reflete por `url_intent` após cada ação, de modo que a
barra de endereço sempre contém uma URL compartilhável e o documento monta a
mesma view a partir dela:

```rust
#[live(name = "app.invoices", view = "live/invoices.html", minimum_protocol_version = 2)]
pub struct Invoices {
    #[model]
    pub sort: String,
    #[url(key = "sort")]
    pub sorted_by: String,
    #[url(key = "dir")]
    pub direction: String,
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    #[url(key = "page")]
    pub page: u64,
    pub rows: Vec<Invoice>,
}
```

A família live-native é a última: os componentes que só fazem sentido sobre o runtime em execução. O widget de upload apresenta o protocolo de upload incluído: seu campo de arquivo carrega `live:upload` para o campo de upload da ilha, seu elemento `progress` é a raiz de progresso do runtime, e cancelar, tentar de novo e remover agem sobre a referência temporária por meio de `live:upload.cancel` e seus irmãos. Cada estado que o domínio conhece é renderizado como texto e exibido a partir do `data-live-upload-state` da raiz de progresso, e "ready" se lê como verificado mas não salvo, porque nada é durável até a ação finalizadora rodar:

```html
{% call upload::upload("attachment", "Attachment", accept="image/png") %}{% endcall %}
<button type="submit" live:loading.disabled="save_attachment">Save attachment</button>
```

O feed ao vivo e o sino de notificações ficam em uma ilha apoiada por um stream. O runtime escreve `data-live-stream-state` na raiz da ilha e anuncia cada mudança no elemento `[data-live-stream-status]` que as macros renderizam (Updates disconnected, Connecting to updates, Updates current, Updates degraded, Reconnecting to updates, Updates closed), de modo que um stream degradado, em reconexão ou fechado diz isso e só o estado current se lê como atual. Os itens do feed passam por `live_key`. O menu de conta é um `details` expansível com âncoras e um formulário de saída que envia com o token CSRF da sessão; ele é um slot de stitch sob o RenderCache, então uma aplicação o monta como sua própria ilha ligada à identidade e o shell compartilhado nunca contém o nome do principal.

A camada de elementos personalizados aprimora controles nativos que nunca substitui. Cada elemento é uma subclasse de `HTMLElement` em light DOM definida apenas pelo seu próprio arquivo vendorizado, carrega o prefixo `sn-` e não guarda valor de formulário, porque a entrada nativa dentro dele é o controle: bloqueie o script e o formulário ainda envia o mesmo valor. A entrada OTP é uma única entrada nativa (`inputmode="numeric"`, `autocomplete="one-time-code"`, um padrão de comprimento) sobre um modelo transitório, e `sn-input-otp` espelha os caracteres digitados em células `aria-hidden`. O seletor de data é uma entrada `type="date"`, e suas faixas de ano, mês e dia são fieldsets de radios nativos dentro de contêineres CSS scroll-snap, então tocar, clicar e as setas selecionam sem script; `sn-date-picker` compõe uma seleção completa na entrada. O combobox é o padrão acessível de combobox (`role="combobox"`, `aria-expanded`, `aria-activedescendant`, um `role="listbox"` de opções) sobre uma entrada nativa com uma `datalist` para o caso sem script; `sn-combobox` filtra, move a opção ativa e seleciona, e recusa um listbox cujo `data-sn-query` não é o texto atual da entrada, de modo que um resultado obsoleto nunca substitui os resultados de uma consulta mais nova:

```html
{% call otp::input_otp("code", "One-time code") %}{% for index in cells %}{% call otp::otp_cell(index) %}{% endcall %}{% endfor %}{% endcall %}
{% call date::date_picker("when", "Renewal date", years, months, days, min="2026-01-01", max="2028-12-31") %}{% endcall %}
{% call combo::combobox("country", "Country", countries, query=country, placeholder="Type a country") %}{% endcall %}
```

### Por que o Suprnova diverge

O Laravel traz componentes Blade e a marcação de um kit inicial; o Suprnova
entrega a biblioteca pelo próprio framework, no vocabulário do Live, sem que
nenhuma aplicação cliente seja dona da página. A aparência vem ligada por
padrão e pode ser removida sem que nada quebre, que é o que headless significa
aqui.

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

Live funciona por completo sem RenderCache. Colocar documentos Live em cache
é tarefa do RenderCache; veja [RenderCache](render-cache.md).

## Referência da CLI

| Comando | Finalidade |
|---|---|
| `suprnova live:make <name>` | Gerar um componente e sua view e registrá-lo |
| `suprnova live:check` | Provar cada view registrada com o verificador integrado |
| `suprnova live:inspect` | Relatar o estado seguro de runtime, registro, provedores e artefatos |
| `suprnova live:assets --out <dir>` | Publicar atomicamente os artefatos de runtime revisados |
