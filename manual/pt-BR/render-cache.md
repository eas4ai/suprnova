# RenderCache

O RenderCache armazena uma cópia comprovadamente segura da resposta de uma
rota GET ou HEAD e atende à próxima requisição correspondente a partir dela,
sem sequer executar seu handler. Você inclui rotas e grupos explicitamente;
tudo o mais continua funcionando exatamente como funciona hoje. Uma rota que
você nunca inclui permanece intocada. Uma rota que você inclui ainda
renderiza e responde corretamente mesmo quando nada relacionado àquela
requisição em particular se revela seguro para armazenar em cache - ela
simplesmente nunca é armazenada, e você pode descobrir o motivo.

Este capítulo cobre habilitar o cache, incluir rotas e grupos, declarar
variância, ler os cabeçalhos de resposta que ele adiciona, os motivos pelos
quais uma renderização é recusada, o controle operacional e em que ele
difere de `suprnova::Cache`.

## Habilitando o cache

Duas variáveis de ambiente importam para começar:

- `RENDER_CACHE_ENABLED` - `true`, a menos que seja definida como `false`
  ou `0`. Com ela desabilitada, toda requisição contorna o RenderCache por
  completo; nada é buscado e nada é armazenado.
- `RENDER_CACHE_L1_DIR` - não definida por padrão, o que significa nenhuma
  camada em disco. Defina-a como um diretório que o processo possa criar e
  no qual possa escrever, e as representações armazenadas sobrevivem a um
  reinício do processo em uma segunda camada apoiada em arquivo.

Um punhado de outras variáveis ajusta os padrões: `RENDER_CACHE_L0_ENTRIES`
(4.096) e `RENDER_CACHE_L0_BYTES` (128 MiB) limitam a camada em processo;
`RENDER_CACHE_L1_BYTES` (1 GiB) limita a camada de arquivo;
`RENDER_CACHE_FAILURE` (`open` por padrão, ou `closed`) decide se um
problema de armazenamento ou de banco de dados serve a rota sem cache ou
recusa a requisição; `APP_BUILD_ID` (a versão do seu próprio crate, por
padrão) isola cada entrada em cache no namespace do build que a produziu,
de modo que um deploy nunca sirva os bytes de um build antigo.

## Incluindo uma rota ou um grupo

Nada é armazenado em cache até que você diga isso. `Router::try_render_cache`
inclui um padrão de rota já registrado; `Router::try_render_cache_group`
inclui toda rota sob um prefixo de caminho. Ambos recebem uma política
construída com `RenderCachePolicy::builder`:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)`
define por quanto tempo uma representação é válida, por quanto tempo a mais
ela ainda pode ser servida enquanto uma reconstrução em segundo plano é
executada, e por quanto tempo a mais ainda pode ser servida se essa
reconstrução falhar completamente. `RepresentationClass` vai da mais ampla
à mais restrita em compartilhamento: `PublicShared` (uma representação para
todos que correspondem à variância declarada), `PublicShellStitched`
(reservada para uma futura representação de shell composto, ainda não
utilizável), `PrivateCached` (uma representação por visitante autenticado ou
tenant), e `Uncacheable`.

Um padrão de rota precisa já estar registrado antes de você incluí-lo, e
você precisa terminar de incluir rotas e grupos **antes** de chamar
`RenderCache::install` (abaixo) - a etapa de instalação lê o que estiver
registrado até aquele ponto.

Uma política no nível de rota também pode ser um patch de restrição do seu
grupo envolvente, usando `PolicyPatch` em vez de uma `RenderCachePolicy`
completa: ela herda tudo o que o grupo declarou e só pode torná-lo mais
restrito (uma janela de validade mais curta, uma classe mais estrita), nunca
mais amplo. Retirar uma rota inteiramente de um grupo em cache é um
`PolicyPatch` que define a classe como `Uncacheable`.

Termine de conectar o RenderCache com uma única linha, depois de todo
registro de middleware que estabelece a localidade, a sessão ou a identidade
com escopo de requisição (o RenderCache os lê para construir sua chave de
busca, então ele precisa rodar depois de tudo o que os configura):

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

let router = add_render_cache(router)?;
let router = RenderCache::install(router, RenderCacheConfig::from_env()).await?;
```

## Declarando variância

Por padrão, uma representação em cache varia apenas por padrão de rota,
parâmetros de caminho e pelo build da aplicação. Qualquer outra coisa da
qual a saída do seu handler realmente dependa precisa ser declarada, com
dois mecanismos:

- **Parâmetros de consulta.** `.query(QueryPolicy::declared(["page", "sort"]))`
  nomeia os parâmetros de consulta que distinguem representações; qualquer
  outro parâmetro de consulta presente em uma requisição contorna o cache
  para essa requisição em vez de ser silenciosamente ignorado.
- **Dimensões de variância**, adicionadas uma de cada vez com
  `.vary(dimension)`:
  - `VarianceDimension::Locale` particiona pela localidade negociada.
  - `VarianceDimension::Media` particiona pelo tipo de mídia negociado.
  - `VarianceDimension::Host` particiona pelo host da requisição, quando
    sua implantação torna mais de um host significativo.
  - `VarianceDimension::Tenant` particiona pelo tenant atual como material
    opaco de chave; uma rota cujo handler leia o tenant em algum momento
    precisa declará-lo.
  - `VarianceDimension::Principal` particiona pelo visitante autenticado
    como material opaco de chave, vinculado a uma versão de permissão (veja
    "Epoch, permissões e inspeção" abaixo); uma rota `PrivateCached`
    precisa declarar `Principal` ou `Tenant` (ou ambos), ou ela simplesmente
    falha ao ser construída.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` e
uma `VarianceDimension::Application(name)` personalizada existem no tipo,
mas não têm resolvedor nesta versão: uma rota que declara uma delas contorna
o cache em toda requisição, silenciosamente, em vez de falhar ao ser
construída. Não as declare ainda.

## Lendo os cabeçalhos de resposta

Um hit servido carrega `ETag` (um validador forte que seu cliente pode
enviar de volta como `If-None-Match` para obter um `304`), `Cache-Control`
(`private`, a menos que a classe seja `PublicShared` e você defina uma
`SharedCachePolicy::SMaxAge`, caso em que também carrega `public` e
`s-maxage`), `Vary` (a partir de quaisquer dimensões declaradas que
impliquem um - `Locale` implica `Accept-Language`, `Media` implica
`Accept`), e `Age` (segundos inteiros desde que a representação foi
publicada). Uma resposta obsoleta mas servível carrega adicionalmente
`Warning: 110 - "Response is Stale"`.

## Por que uma renderização nunca é armazenada

Estar incluída não é garantia. Duas verificações independentes rodam depois
de cada renderização, e qualquer uma delas pode recusar o armazenamento sem
falhar a requisição - a resposta que você recebe de volta é idêntica de
qualquer forma, ela simplesmente nunca se torna uma entrada de cache:

**Elegibilidade** recusa de imediato uma resposta que não seja um `200`
simples para um `GET` ou `HEAD`, que faça streaming do corpo, que defina um
cookie, ou que carregue um cabeçalho hop-by-hop ou de rastreamento. Isso é
quase sempre acidental (um redirecionamento, uma página de erro, uma
resposta que acaba tocando em `Set-Cookie`) e não algo que você precise
projetar para evitar.

**Classificação** recusa com base no que seu handler realmente fez enquanto
rodava, em termos que você vai reconhecer:

- **Você leu um valor de sessão.** Qualquer leitura da sessão atual (por
  meio de `session()`, `session_mut`, ou de um cookie de sessão) força a
  renderização para `Uncacheable`, permanentemente, não importa qual
  variância a rota declare. Isso também acontece quando a identidade de um
  visitante anônimo é resolvida por meio do fallback de sessão - uma
  surpresa comum, já que o visitante é genuinamente anônimo e a chave
  resultante é corretamente `Anonymous`, mas a leitura em si ainda é uma
  leitura de sessão.
- **Você leu uma identidade, em uma rota que não declara `Principal`.**
  Ler o usuário autenticado restringe a classe para `PrivateCached`; se a
  variância declarada da rota não incluir `Principal`, não há como
  estabelecer uma chave por visitante para a entrada, portanto ela é
  recusada em vez de compartilhada.
- **Você traduziu (ou seu motor de views traduziu) sem declarar `Locale`.**
  Qualquer leitura da localidade negociada precisa de uma dimensão `Locale`
  declarada, ou a renderização é recusada. O shell de documento de toda
  página Inertia lê a localidade para definir `<html lang>`,
  independentemente de os próprios dados da página terem algo a ver com
  idioma ou não - então uma rota Inertia precisa declarar `Locale` para
  conseguir entrar em cache, mesmo uma sem nenhum conteúdo traduzido
  próprio.
- **Você verificou autorização.** O `Gate` sempre trata uma decisão como
  por visitante, então ele precisa que `Principal` seja declarado mesmo em
  uma rota cuja chave é apenas `Tenant`, até que a própria verificação do
  gate seja comprovadamente por tenant. O RenderCache não consegue
  distinguir isso sozinho.
- **Um model por trás da página carrega um escopo global delimitado por
  tenant.** Um escopo global que lê o tenant atual a partir do seu próprio
  estado local de requisição para filtrar uma consulta - o padrão que a
  própria documentação do `GlobalScope` do Suprnova mostra - muda o que a
  consulta retorna sem que o RenderCache jamais veja essa leitura. Declare
  a variância `Tenant` em qualquer rota apoiada por um model desse tipo;
  nada aqui consegue capturar essa omissão por você.
- **Você leu um valor de configuração secreto, ou um contexto de
  requisição não declarado.** Ambos forçam `Uncacheable`. A dependência de
  uma resposta em relação a um cabeçalho de requisição comum, ou a
  `Config::get`, é completamente invisível para o RenderCache - ele não
  pode recusar o que não consegue ver, então declarar a variância
  correspondente é responsabilidade sua.

Nada disso precisa de ferramentas especiais para ser visto na prática: o
comando oculto `render-cache:inspect` (abaixo) mostra se a entrada de uma
rota existe ou não, ou você pode simplesmente tentar duas requisições
seguidas e verificar se a segunda carrega um cabeçalho `Age`.

## Uma rota que entra em cache

Uma página de listagem pública sem conteúdo por visitante:

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

registrada e incluída:

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` nunca toca na sessão, no visitante autenticado ou na localidade,
então a primeira requisição renderiza e publica; toda requisição pelos
próximos cinco minutos é servida a partir dessa cópia armazenada com um
cabeçalho `Age`, um `304` para um cliente que já a possui, e
`Cache-Control: public, max-age=300, s-maxage=300` para qualquer CDN à sua
frente.

## Uma rota que é recusada

A mesma forma de página, mas o handler lê a sessão para exibir uma mensagem
flash:

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

incluída exatamente da mesma forma que acima. Toda requisição ainda
renderiza e serve a página correta - mensagem flash incluída - mas nada é
jamais armazenado: a leitura da sessão restringe a classe para
`Uncacheable` antes mesmo de o RenderCache chegar à verificação de
elegibilidade, então uma segunda requisição para a mesma URL renderiza de
novo do zero em vez de voltar com um cabeçalho `Age`. A correção, se essa
página deve entrar em cache, é parar de ler a sessão no caminho em cache
(renderize a flash a partir de um parâmetro de consulta ou de uma resposta
pequena separada) - não existe declaração de variância que torne uma
leitura de sessão cacheável, porque uma leitura de sessão significa que a
resposta depende de algo que nenhuma chave poderia particionar com
segurança.

## Epoch, permissões e inspeção

- **`RenderCache::bump_permission_version()`** - chame isso sempre que uma
  ação da aplicação mudar o que um usuário autenticado tem permissão para
  fazer (uma mudança de papel, uma concessão ou revogação de permissão).
  Sem isso, um usuário cujas permissões acabaram de mudar continua
  correspondendo ao que estava armazenado em cache sob seu conjunto de
  permissões anterior.
- **`RenderCache::advance_epoch()`**, ou o comando oculto
  `render-cache:epoch-advance` - uma invalidação de emergência. Toda
  entrada atualmente armazenada se torna inacessível por busca comum já na
  sua próxima requisição, imediatamente, porque o epoch está embutido na
  própria chave de busca. A camada em processo também é limpa por completo
  no mesmo instante; uma camada apoiada em arquivo mantém seus arquivos
  antigos em disco até que a varredura periódica ou manual os recupere, o
  que é uma questão de higiene de disco, e não de corretude. Recorra a
  isso quando algo estiver errado com o conteúdo em cache e você não puder
  esperar que entradas individuais expirem.
- **O comando oculto `render-cache:inspect <key>`** relata os metadados de
  uma entrada armazenada (nunca seu corpo) pelo texto da chave que os logs
  ou a telemetria da sua aplicação podem expor, junto com o epoch atual,
  para que você possa saber se o que está vendo ainda é autoridade viva ou
  se já expirou sem que você percebesse.

## RenderCache versus `suprnova::Cache`

`suprnova::Cache` é um armazenamento chave-valor que você chama
explicitamente: você escolhe a chave, escolhe o que armazenar, escolhe
quando invalidá-lo (`Cache::put`, `Cache::get`, `Cache::remember`,
`Cache::forget`). Ele funciona para qualquer dado que seu código decida que
vale a pena cachear, em qualquer backend que você configure (memória ou
Redis).

O RenderCache não é um armazenamento de propósito geral, e você nunca o
chama a partir do seu handler. Ele armazena em cache respostas HTTP
inteiras, a chave é derivada automaticamente a partir da rota e de sua
variância declarada, e a invalidação é baseada em geração: uma escrita
comum no banco de dados através do ORM ou do construtor de consultas avança
as gerações das quais a renderização dependia, e a entrada é recalculada na
próxima vez que for solicitada, em vez de apagada manualmente. Recorra a
`suprnova::Cache` quando você tiver um valor específico que quer computar
uma vez e reutilizar; recorra ao RenderCache quando você tiver uma rota
inteira cuja resposta é cara de renderizar e segura de compartilhar.
