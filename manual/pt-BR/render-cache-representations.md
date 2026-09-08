# Representações do RenderCache

Uma rota em cache não armazena "uma página". Ela armazena uma
**representação**: uma resposta concreta, sob uma chave de busca, em uma ou
mais camadas de armazenamento, com metadados suficientes ao lado para
responder a uma requisição condicional e para provar depois que ela continua
atual. Dois visitantes recebem os mesmos bytes armazenados apenas quando a
chave que derivam é a mesma chave, e a chave é derivada do que a rota
declarou - nunca do que o handler por acaso fez.

Este capítulo é sobre essa coisa armazenada. Que formas uma representação
pode assumir (`Complete` e `Composite`), o que entra na sua chave, em quais
camadas ela é escrita, o `ETag`, o `Cache-Control`, o `Vary`, o `Age` e o
`Warning` que um hit servido carrega, os quatro estados de validade em que
ela pode estar, como ela responde a `If-None-Match` e a `HEAD`, e o que
`PrivateCached` e `PublicShellStitched` de fato armazenam. *Por que* uma
representação sai da faixa de validade - uma escrita, um avanço de epoch - é
assunto do próximo capítulo; aqui basta que as faixas existam e que uma
representação esteja em uma delas. Todo exemplo abaixo é uma rota da
aplicação de dogfood
deste repositório (`app/src/live/mod.rs`) e é provado por um teste nomeado
em `app/tests/live_render_cache.rs`.

## Duas formas de entrada

Uma entrada armazenada é de um entre dois tipos.

- **`Complete`** é uma resposta pronta: um status, um conjunto de cabeçalhos
  reproduzíveis e um buffer de corpo. Servi-la não copia nada e não executa
  nada. Toda rota `PublicShared` e `PrivateCached` armazena essa forma.
- **`Composite`** é um **shell** compartilhado com buracos tipados
  recortados nele, mais um grafo de segmentos dizendo o que volta para cada
  buraco. Apenas `RepresentationClass::PublicShellStitched` armazena essa
  forma, e apenas um documento Live produz uma.

A classe que você declara na política decide qual forma é sequer alcançável.
`/live/public` e `/live/todos` declaram ambas `PublicShared`;
`the_database_profile_serves_a_hit_through_the_sql_stores` lê a entrada
publicada de `/live/todos` de volta do armazenamento e afirma que ela é uma
entrada `EntryKind::Complete`, e
`the_public_document_is_a_hit_whose_seed_still_promotes` lê a de
`/live/public` de volta através de `RenderCache::inspect_route_for_test` e
afirma a classe sob a qual ela foi armazenada. Isso importa, porque "ela foi
armazenada" e "ela foi silenciosamente recusada" produzem a mesma resposta:
a afirmação precisa ser feita contra a entrada, não contra o que o visitante
vê.

## A chave de busca

A chave que uma requisição deriva é construída a partir do padrão de rota,
dos seus parâmetros de caminho, dos parâmetros de consulta que a política
declarou, do valor resolvido de cada dimensão de variância declarada, do id
de build da aplicação (`APP_BUILD_ID`) e do epoch de autoridade atual. Nada
mais. Um parâmetro de consulta que chega na requisição mas não é nomeado por
`QueryPolicy::declared` contorna o cache para essa requisição em vez de ser
silenciosamente descartado da chave, porque descartá-lo serviria a página
errada a quem a enviou.

A chave é um texto que um operador consegue segurar:
`RenderCache::key_for_route_for_test`, em
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch`, afirma
que ela começa com `rk1.`, e `render-cache:inspect` recebe exatamente esse
texto.

Como o epoch faz parte da chave, um avanço de epoch não precisa encontrar e
excluir nada. Toda entrada armazenada anteriormente simplesmente deixa de
ser alcançável por busca comum na requisição seguinte. Esse é o mecanismo do
qual depende a invalidação de emergência do capítulo
[Operações](render-cache-operations.md).

## Em quais camadas uma política escreve

Existem duas camadas de armazenamento. A **L0** é memória em processo,
limitada por `RENDER_CACHE_L0_ENTRIES` e `RENDER_CACHE_L0_BYTES`. A **L1** é
o que o perfil de implantação configurar - um diretório de arquivos, uma
tabela de banco de dados ou o Redis - e é compartilhada por todo processo
que apontar para ela.

O builder de política armazena **apenas na L0**, a menos que você diga o
contrário: `StorageLayers::l0_only()` é o padrão. Uma rota que vale a pena
colocar na camada compartilhada declara isso:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

Essa é a declaração de `/live/todos` em `app/src/live/mod.rs`. É o único
documento daquela aplicação cujos bytes todo nó pode compartilhar, então é o
único que declara `l0_and_l1()`. Sob o perfil embedded, em que a L1 fica
desabilitada a menos que `RENDER_CACHE_L1_DIR` nomeie um diretório, declarar
a camada não muda nada; sob o perfil Database a entrada vai parar em
`suprnova_render_entries` e um segundo processo a encontra lá.

`the_database_profile_serves_a_hit_through_the_sql_stores` é a prova.
Ele inicializa a aplicação sobre os provedores do perfil Database, lê a
entrada publicada diretamente da L1 sob exatamente a chave que o middleware
derivou, depois esvazia a L0 e pergunta de novo - e a segunda requisição
ainda é respondida sem que o handler rode. Um hit em memória pareceria
idêntico do lado do cliente, e é por isso que o teste vai até o
armazenamento.

Escolha as camadas por rota, e não globalmente. A L1 custa uma ida e volta
em um miss que a L0 sozinha não custa, e uma entrada que só um nó vai pedir
não vale a pena ser posta onde todos os nós podem vê-la.

## Os metadados que um hit servido carrega

Cinco campos de resposta descrevem uma representação servida, e é aqui que
eles são definidos; os outros capítulos os usam sem repeti-los.

| Campo | O que ele diz |
|---|---|
| `ETag` | Um validador forte sobre exatamente os bytes enviados. Um cliente pode devolvê-lo como `If-None-Match`. |
| `Cache-Control` | `private` para toda classe, por padrão. Uma rota `PublicShared` que define `SharedCachePolicy::SMaxAge` também recebe `public` e `s-maxage`, que é a única forma de um proxy compartilhado ser algum dia convidado a guardar os bytes. Um documento `Composite` montado com pelo menos uma ilha é `private, no-store`. |
| `Vary` | Derivado das dimensões de variância declaradas que implicam um cabeçalho de requisição: `Locale` implica `Accept-Language`, `Media` implica `Accept`, `Encoding` implica `Accept-Encoding`. Uma dimensão que não implica nenhum não adiciona nada. Os nomes são emitidos ordenados por nome de cabeçalho, e não na ordem em que você declarou as dimensões. |
| `Age` | Segundos inteiros desde que a representação foi publicada. Sua presença é a prova local mais simples de que uma resposta saiu do armazenamento. |
| `Warning` | `110 - "Response is Stale"`, e apenas em uma resposta servida além do seu intervalo de validade. |

O mapeamento de dimensão para cabeçalho é `VarianceDimension::vary_header`
em `crates/suprnova-live/src/render_cache/variance.rs`. Dois testes de
engine provam as metades `Locale` e `Encoding` dele e o valor combinado do
cabeçalho: `a_descriptor_orders_dimensions_and_bounds_values`
(`crates/suprnova-live/tests/render_cache_variance.rs`) afirma que um
descritor que carrega as duas relata `["Accept-Encoding", "Accept-Language"]`,
e `cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
(`crates/suprnova-live/tests/render_cache_coherence.rs`) afirma que o mesmo
par emite `Accept-Encoding, Accept-Language` e que um descritor sem nenhuma
dimensão que implique cabeçalho não emite `Vary` algum. `Media` implicando
`Accept` está documentado a partir do código; nenhum teste aqui faz esse
par.

Três dos valores de resposta são afirmados contra a aplicação em execução:
`the_public_document_is_a_hit_whose_seed_still_promotes` lê
`private, max-age=300` de `/live/public` e exige um cabeçalho `Age` na
segunda requisição;
`the_private_document_is_cached_per_principal_and_never_crosses` lê
`private, max-age=60` de `/live/me`;
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` lê
`private, no-store` de um painel montado, que é o valor que nada além do
respondedor composto escreve.

## Os quatro estados de validade

Todo hit resolve para exatamente um de quatro estados antes de qualquer
coisa ser servida.
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` os
define. **As duas janelas de obsolescência são ambas medidas a partir do fim
do intervalo de validade, e não empilhadas uma após a outra** - esse é o
detalhe em que as pessoas tropeçam:

| Estado | Idade desde a publicação | O que o visitante recebe |
|---|---|---|
| Válida | abaixo de `fresh_ms` | os bytes armazenados, sem `Warning` |
| Obsoleta servível | além de `fresh_ms` por menos de `stale_servable_ms` | os bytes armazenados imediatamente, sob `Warning`, com uma reconstrução limitada disparada atrás da requisição |
| Obsoleta em erro | além de `fresh_ms` por pelo menos `stale_servable_ms`, e por menos de `stale_on_error_ms` | uma reconstrução em primeiro plano; os bytes armazenados sob `Warning` apenas se essa própria reconstrução falhar |
| Morta | além de `fresh_ms` pela maior das duas janelas ou mais | nada; a requisição renderiza |

`/live/todos` declara `FreshnessPolicy::new(300_000, 60_000, 300_000)`,
então ela é válida por cinco minutos, obsoleta servível durante o sexto,
obsoleta em erro até dez minutos, e morta depois disso.

Duas regras se sobrepõem às faixas. Uma representação `PrivateCached`
**nunca** é servida obsoleta: passado o seu intervalo de validade ela está
Morta, e é por isso que `/live/me` declara
`FreshnessPolicy::new(60_000, 0, 0)` - uma faixa de obsolescência ali seria
lida como uma promessa que o cache não cumpre. E um documento armazenado de
semente pública cujo prazo de promoção já passou está Morto quaisquer que
sejam seus intervalos, porque uma semente além do seu prazo nunca mais pode
ser promovida.

`stale_service_is_marked_and_rebuilt_in_the_background` conduz
`/live/todos` através da primeira fronteira em um relógio controlado e
afirma o corpo servido, o `Warning: 110 - "Response is Stale"` e o
`Age: 300`. O que *faz* uma representação sair da faixa de validade mais
cedo - uma escrita, um avanço de epoch - é assunto de
[Gerações do RenderCache](render-cache-generations.md).

## Requisições condicionais e HEAD

Um cliente que devolve um `ETag` servido como `If-None-Match` recebe um
`304` sem corpo, e um `HEAD` recebe os cabeçalhos sem corpo. Nenhum dos dois
chega ao seu handler:

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` afirma os
três contra a aplicação em execução, inclusive que a contagem de
renderizações não se move ao longo dos dois últimos.

Uma exceção, e ela é deliberada: uma resposta `Composite` nunca responde
`304`. Toda montagem é uma representação distinta - identidades de ilha
novas, um nonce de bootstrap novo quando o documento tem um - então um `304`
diria ao cliente para casar o corpo que ele já tem com cabeçalhos cunhados
para esta requisição. O `ETag` de uma resposta montada continua sendo forte
sobre exatamente os bytes que foram enviados; ele simplesmente nunca
corresponde a uma requisição posterior. O passo 7 de
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` devolve um
validador servido diretamente e afirma um `200` com um `ETag` diferente.

## Uma representação que pertence a uma pessoa

`RepresentationClass::PrivateCached` armazena uma representação por
visitante autenticado. Ela é recusada em tempo de construção a menos que a
política também declare variância `Principal` ou `Tenant`, de modo que o par
não possa se separar por acidente:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

O handler por trás dela é um handler comum. Ele resolve o visitante
autenticado e renderiza o nome dele:

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

Nada de extra é conectado para fazer aquilo entrar em cache. A rota carrega
o mesmo `AuthMiddleware::redirect_to("/login")` que o painel carrega, então
um visitante anônimo é redirecionado antes de o handler rodar, e o próprio
principal é resolvido dentro da renderização. Ler a identidade do visitante
autenticado a partir da sessão é classificado como uma **leitura de
identidade**, e não como uma leitura de sessão, então a renderização se
restringe a `PrivateCached`, a chave carrega material opaco por principal, e
as duas coisas concordam.

`the_private_document_is_cached_per_principal_and_never_crosses` autentica
dois visitantes, acerta duas vezes cada um sem renderizar, e afirma que cada
corpo nomeia a sua própria pessoa e não a do outro; um terceiro visitante
renderiza, porque não compartilha nada com nenhum dos dois. O
`Cache-Control` servido é `private, max-age=60`, então nenhum proxy
compartilhado jamais recebe a oferta dos bytes. O mesmo teste mostra a outra
metade do acordo: a renderização resolve o seu principal através do provedor
que lê a tabela `users`, então semear um terceiro visitante invalida toda
entrada `/live/me` armazenada, e a próxima requisição de cada uma
reconstrói. Isso é invalidação com granularidade de tabela fazendo
exatamente o que [Gerações](render-cache-generations.md) descreve.

Duas consequências dessa classificação valem ser conhecidas antes de você
declarar a classe:

- Uma requisição **anônima** a uma rota `PrivateCached` com variância
  `Principal` entra em cache sob a chave `Anonymous`. A renderização não
  resolveu identidade nenhuma, então nenhum material de principal foi
  observado, a chave diz `Anonymous`, e as duas coisas concordam. Um
  visitante autenticado deriva uma chave `Private` que nunca consegue
  alcançar aquela entrada. Isso vale quando uma requisição dessas de fato
  renderiza um `200`, o que `/live/me` nunca faz - o seu redirecionamento de
  login responde um `302`, e um `302` é recusado pela elegibilidade antes de
  qualquer coisa disso ser consultada. O teste de framework que de fato
  chega lá é
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  em `framework/tests/render_cache/middleware.rs`.
- O identificador de um **guard nomeado** é material de principal exatamente
  da mesma forma que o do guard padrão. Lê-lo registra uma leitura de
  principal e, quando há um id, o valor.

E uma regra que não mudou: uma rota que lê o principal *sem* declarar
variância `Principal` é recusada para armazenamento. Não há como estabelecer
uma chave por visitante para uma entrada dessas, então ela nunca é
armazenada em vez de ser compartilhada. Todo outro valor de sessão ainda
força `Uncacheable`; veja a lista de classificação em
[RenderCache](render-cache.md).

## Um shell com buracos nele

`RepresentationClass::PublicShellStitched` serve para um documento Live cuja
moldura é a mesma para todo mundo e cujas ilhas não são. A entrada
armazenada guarda apenas o shell. Nenhuma marcação de ilha ligada à
identidade e nenhum snapshot assinado jamais fica dentro dos bytes
armazenados; todo hit remonta toda ilha para quem estiver pedindo, sob
autoridade derivada para aquela requisição.

O painel deste repositório é essa rota:

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` afirma o que
isso compra e o que isso custa. A entrada armazenada é um
`EntryKind::Composite` com três slots, um por ilha ligada à identidade. Um
segundo principal é respondido a partir daquele shell, e os dois documentos
diferem **apenas** nas suas tags de ilha - o teste remove as três tags de
ilha de cada um e compara o que sobra, byte a byte. O próprio
redirecionamento de login da rota ainda roda em todo hit: um hit costurado é
encaminhado por toda a cadeia de middleware da rota antes de qualquer coisa
ser servida, então um visitante anônimo recebe o redirecionamento, nunca um
documento montado.

Um documento montado com pelo menos um slot é enviado com
`Cache-Control: private, no-store`. Ele guarda as ilhas de um principal sob
autoridade rederivada para uma requisição, e um `max-age` deixaria um perfil
de navegador compartilhado reproduzi-las para quem sentasse ali em seguida.
Um `Composite` de zero slots não carrega byte algum por principal, apenas um
nonce por requisição, então ele mantém o `max-age` privado da classe como
qualquer outra representação privada;
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` em
`framework/tests/render_cache/stitch.rs` afirma isso. De todo modo, a classe
recusa `SharedCachePolicy::SMaxAge` em tempo de construção da política, de
forma que nenhum proxy compartilhado jamais recebe a oferta dos bytes.

Dois limites a conhecer: a classe só faz sentido em uma rota cuja cadeia
termina no middleware de conclusão do Live, então use-a com
`LiveDocument::render` e com nada mais, e uma entrada costurada nunca é
servida pelo fallback de obsoleta em erro e nunca dispara uma reconstrução
em segundo plano. O capítulo
[Gerações](render-cache-generations.md) diz o que esse segundo ponto
significa na prática.

### Por que Suprnova diverge

O Laravel não tem modelo de representação no lado do servidor, de forma
alguma. Seus pacotes de cache de resposta armazenam a saída renderizada de
uma rota sob uma chave que você mesmo compõe - normalmente a URL, às vezes a
URL mais um sufixo escrito à mão para o usuário autenticado - e a devolvem
na requisição seguinte. Há uma única forma de coisa armazenada, ela é sempre
um corpo pronto, e se dois visitantes a compartilham é uma propriedade da
string que você construiu.

O Suprnova faz da chave uma declaração e da forma uma consequência. Você
nomeia a classe e as dimensões de variância; o framework deriva a chave,
recusa `PrivateCached` sem uma dimensão que particione, compara o que a
renderização de fato observou com o que a chave de fato disse, e se recusa a
armazenar a renderização quando os dois discordam. E porque
`PublicShellStitched` existe, uma página que é 95 por cento compartilhada e
5 por cento privada não precisa escolher entre não cachear nada e cachear
algo que não deveria: a parte compartilhada é armazenada uma vez e a parte
privada é rerrenderizada por requisição, com os bytes privados nunca
entrando no armazenamento.

## Próximos passos

- [Gerações do RenderCache](render-cache-generations.md) - como uma
  representação armazenada deixa de estar atual, e o que acontece em seguida
- [RenderCache](render-cache.md) - declarar políticas, variância, e os
  motivos pelos quais uma renderização nunca é armazenada
- [Live](live.md) - as ilhas para as quais um shell costurado tem buracos
