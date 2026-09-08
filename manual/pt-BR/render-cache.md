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

## Os capítulos

Este é o primeiro de cinco. Leia-os em ordem na primeira vez; depois disso,
cada um responde sozinho a uma pergunta.

| Capítulo | Responde |
|---|---|
| RenderCache (este) | Como eu ligo isso e incluo uma rota? |
| [Representações](render-cache-representations.md) | O que é de fato armazenado, e sob qual chave? |
| [Gerações](render-cache-generations.md) | Quando uma cópia armazenada deixa de estar atual? |
| [Implantação](render-cache-deployment.md) | Como vários nós compartilham um único cache? |
| [Operações](render-cache-operations.md) | Como eu inspeciono, testo, meço e desligo isso? |

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
recusa a requisição; `APP_BUILD_ID` isola cada entrada em cache no namespace
do build que a produziu. Defina-a explicitamente como algo que muda a cada
deploy: seu padrão é uma versão de crate compilada junto, que não muda. Veja
[Implantação do RenderCache](render-cache-deployment.md).

`RENDER_CACHE_PROFILE` (`embedded` por padrão, ou `database` ou `redis`)
escolhe se a segunda camada e o coordenador de reconstrução ficam neste
processo ou são compartilhados com todos os outros nós. Um perfil
compartilhado também exige uma migração que sua aplicação liste. Ambos são
assunto do capítulo [Implantação](render-cache-deployment.md), junto com a
tabela completa de variáveis.

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
define por quanto tempo uma representação é válida e, em seguida, duas
janelas medidas a partir dessa borda de validade: quão além dela a cópia
armazenada ainda pode ser servida enquanto uma reconstrução em segundo
plano é executada, e quão além dela a cópia armazenada pode ser servida se
uma reconstrução em primeiro plano falhar completamente. As duas janelas não
são empilhadas; veja
[Representações do RenderCache](render-cache-representations.md).

`RepresentationClass` vai da mais ampla à mais restrita em compartilhamento:
`PublicShared` (uma representação para todos que correspondem à variância
declarada), `PublicShellStitched` (um documento Live cujo shell
compartilhado é armazenado uma única vez e cujas ilhas são remontadas para
quem estiver pedindo; veja
[Representações](render-cache-representations.md)),
`PrivateCached` (uma representação por visitante autenticado ou tenant), e
`Uncacheable`.

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

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
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
  - `VarianceDimension::Media` particiona pelo tipo de mídia negociado e
    adiciona `Accept` ao `Vary`.
  - `VarianceDimension::Encoding` particiona pela codificação de conteúdo
    negociada e adiciona `Accept-Encoding` ao `Vary`.
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

`Media` e `Encoding` são declaráveis e entram na chave, mas esta versão
resolve cada uma delas para uma constante: toda requisição é `text/html` e
`identity`, respectivamente. Declará-las é, portanto, um movimento de
compatibilidade futura - elas ampliam o `Vary` corretamente e reservam o
espaço de chaves, de modo que uma camada posterior de negociação de conteúdo
ou de compressão não possa colidir com entradas publicadas antes de ela
existir - em vez de algo que particione o tráfego hoje.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` e
uma `VarianceDimension::Application(name)` personalizada existem no tipo,
mas não têm resolvedor nesta versão: uma rota que declara uma delas contorna
o cache em toda requisição, silenciosamente, em vez de falhar ao ser
construída. Não as declare ainda.

## Lendo os cabeçalhos de resposta

Um hit servido carrega `ETag` (um validador forte que seu cliente pode
enviar de volta como `If-None-Match` para obter um `304`), `Cache-Control`,
`Vary` e `Age` (segundos inteiros desde que a representação foi publicada, e
o sinal local mais rápido de que uma resposta saiu do armazenamento em vez
de sair do seu handler). Uma resposta servida além do seu intervalo de
validade carrega adicionalmente `Warning: 110 - "Response is Stale"`. Cada
um dos cinco é definido, com os valores que os testes exigem que as rotas de
dogfood enviem, em
[Representações do RenderCache](render-cache-representations.md).

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
  variância a rota declare. A única coisa que isso *não* cobre é a
  identidade do próprio visitante autenticado. `Auth::id()` a lê da sessão
  quando nada antes na requisição a resolveu, e essa leitura é classificada
  como leitura de identidade, não como leitura de sessão - então um login
  comum apoiado em cookie é exatamente aquilo para que serve uma rota
  `PrivateCached` que declara variância `Principal`, e buscar o id do
  visitante não torna a página silenciosamente não cacheável. Todo outro
  valor da sessão ainda torna a página não cacheável. Duas consequências que
  vale conhecer: uma requisição anônima a uma rota dessas entra em cache sob
  a chave `Anonymous`, porque a renderização não resolveu identidade
  nenhuma, não observou material de principal, e a chave diz isso - um
  visitante autenticado deriva uma chave `Private` que nunca alcança aquela
  entrada; e o identificador de um guard nomeado é material de principal
  exatamente da mesma forma que o do guard padrão.
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
- **Você executou SQL bruto por meio de `DB::select`, `DB::select_one`,
  `DB::scalar`, ou `DB::select_on`.** O framework não consegue nomear as
  tabelas que uma instrução bruta leu, então a renderização nunca é
  armazenada; ela ainda é servida. Leituras por meio de `DB::table(..)`
  conhecem sua tabela e entram em cache normalmente, assim como
  `Auth::user()`, que resolve por esse caminho. Verificações de papel e
  permissão RBAC também leem por meio dessas instruções brutas, então uma
  rota em cache que avalia uma delas nunca é armazenada.
- **A escrita foi feita por um worker de fila, uma tarefa agendada, ou um
  comando de console.** Apenas um processo que executou
  `RenderCache::install` (o servidor) avança gerações; escritas de outros
  processos não o fazem, e `RenderCache::bump_permission_version()` ali não
  faz nada. Uma página que depende de tal escrita permanece atual apenas
  dentro de sua janela de validade; execute `render-cache:epoch-advance`
  depois de um job que muda o que as páginas em cache exibem.

No PostgreSQL, a renderização roda em uma transação `REPEATABLE READ` para
que o que foi lido e as gerações registradas estejam de acordo; o handler de
uma rota em cache que atualiza uma linha que outra transação mudou depois
que a renderização começou sofre uma falha de serialização. Projete rotas em
cache como caminhos de leitura. Um handler que grava dentro da transação de
renderização ainda avança gerações, mas compete com escritores concorrentes
pelas mesmas linhas e pode sofrer a falha de serialização acima.

Uma escrita feita fora de qualquer transação (`model.save()` sozinho)
confirma primeiro e avança suas gerações em uma transação imediatamente
seguinte, então o momento entre as duas é "dado novo, geração antiga": uma
reconstrução extra, nunca conteúdo obsoleto.

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

- **`RenderCache::bump_permission_version().await?`** - chame isso sempre
  que uma ação da aplicação mudar o que um usuário autenticado tem permissão
  para fazer (uma mudança de papel, uma concessão ou revogação de
  permissão). Isso avança uma geração persistida que toda renderização com
  chave por visitante autenticado observa. A geração sobrevive a um
  reinício, e a chamada se junta à transação em que a mudança de papel roda,
  quando há uma. Sem essa chamada, um usuário cujas permissões acabaram de
  mudar continua correspondendo ao que estava armazenado em cache sob seu
  conjunto de permissões anterior.
- **`RenderCache::advance_epoch()`**, ou o comando oculto
  `render-cache:epoch-advance` - uma invalidação de emergência. O epoch está
  embutido na própria chave de busca, então avançá-lo põe as entradas
  armazenadas fora de alcance sem nada a enumerar e nada a excluir. No
  processo que o executa o efeito é imediato: ele descarta o lease de epoch
  daquele processo e limpa sua camada em processo no mesmo instante. Outro
  nó se atualiza na sua próxima leitura da autoridade, e sua camada apoiada
  em arquivo mantém os arquivos antigos até que uma varredura os recupere -
  a automática a cada 256ª publicação, ou um `RenderCache::sweep()`
  explícito - o que é uma questão de higiene de disco, e não de corretude.
  Recorra a isso quando algo estiver errado com o conteúdo em cache e você
  não puder esperar que entradas individuais expirem; em mais de um nó,
  veja [Operações do RenderCache](render-cache-operations.md).
- **O comando oculto `render-cache:inspect <key>`** relata os metadados de
  uma entrada armazenada (nunca seu corpo) pelo texto da chave que os logs
  ou a telemetria da sua aplicação podem expor, junto com o epoch atual,
  para que você possa saber se o que está vendo ainda é autoridade viva ou
  se já expirou sem que você percebesse. Ele procura a chave apenas na
  camada em processo do processo em execução, nunca na compartilhada, então
  em um perfil `database` ou `redis` ele não relata entrada alguma para uma
  chave que este nó não tenha servido ele mesmo.

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
próxima vez que for solicitada, em vez de apagada manualmente; uma
renderização que leu SQL bruto nunca chega a ser armazenada, então não há
nada para recalcular. Recorra a
`suprnova::Cache` quando você tiver um valor específico que quer computar
uma vez e reutilizar; recorra ao RenderCache quando você tiver uma rota
inteira cuja resposta é cara de renderizar e segura de compartilhar.

### Por que Suprnova diverge

O Laravel não tem equivalente no próprio framework. Cache de resposta é um
pacote que você adiciona, ele envolve a rota em um middleware que armazena a
resposta renderizada sob uma chave que você compõe, e tudo depois disso é
com você: quais rotas são seguras para cachear, o que torna dois visitantes
diferentes, e quando uma página armazenada deixa de ser verdadeira. O
framework não sabe que uma página foi cacheada, então não consegue te dizer
quando cachear uma delas foi um erro.

O RenderCache faz parte do framework exatamente por esse motivo. Ele vê a
renderização acontecer, então consegue registrar o que o handler leu,
comparar isso com o que a rota declarou, e recusar-se a armazenar uma
resposta cuja segurança não consegue justificar - silenciosamente, sem mudar
o que é servido ao visitante. Incluir uma rota é uma declaração pela qual o
framework passa a te cobrar, e não uma promessa que você faz a si mesmo. O
custo é que algumas rotas que você gostaria de cachear são recusadas e você
tem que descobrir por quê; o benefício é que as que são armazenadas foram
comprovadamente seguras de armazenar, uma vez, pelo processo que as
renderizou.

## Próximos passos

- [Representações do RenderCache](render-cache-representations.md) - o que é
  de fato armazenado, sob qual chave e em quais camadas
- [Gerações do RenderCache](render-cache-generations.md) - como uma cópia
  armazenada deixa de estar atual
- [Cache](cache.md) - o armazenamento chave-valor explícito com o qual este
  capítulo contrasta
- [Live](live.md) - os documentos dos quais uma representação costurada é
  recortada
