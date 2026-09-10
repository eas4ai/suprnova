# Operações do RenderCache

Um cache que você não consegue ver é um cache em que você não consegue
confiar. O RenderCache responde a duas perguntas de operador diretamente e
sem jamais imprimir uma página armazenada: **o que este nó está guardando
sob esta chave, e isso ainda está atual?** e **como eu faço tudo parar?**
Ele responde a uma terceira - "esta rota está sendo servida a partir de uma
cópia armazenada, afinal?" - por telemetria e pelo cabeçalho `Age` em vez de
por um comando, porque essa pergunta é sobre tráfego e não sobre uma
entrada. Há dois comandos de console, nove contadores de telemetria, uma
varredura de disco limitada e uma alavanca de emergência.

Este capítulo é a superfície operacional: os comandos, exatamente o que eles
imprimem e o que conseguem ver; os contadores e seus conjuntos fechados de
desfechos; como a camada de arquivo recupera disco; como testar uma rota em
cache de modo que o teste prove o cache em vez de meramente responder; o que
fazer quando algo está errado, incluindo o procedimento multinó de que uma
restauração de banco de dados precisa; e como o desempenho do próprio cache
é medido e quanto esses números honestamente valem. Os exemplos de comando
são os que
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` executa
pelo próprio ponto de entrada de console deste repositório, em
`app/tests/live_render_cache.rs`.

## Os dois comandos de console

Ambos são comandos ocultos, registrados pelo framework e alcançáveis pelo
binário `console` do seu projeto como qualquer outro. Nenhum dos dois jamais
imprime um corpo armazenado ou uma identidade de dependência crua.

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** relata a forma de uma entrada armazenada:
sua classe de representação, seus `body_bytes`, seus outros metadados e o
epoch de autoridade atual ao lado, para que você possa saber se a entrada
que está olhando ainda é autoridade viva ou se já expirou sem que você
percebesse. Ele imprime `no entry (current epoch: {epoch})` quando a chave
não nomeia nada que ele consiga ver, e falha - não relata sucesso - com uma
chave impossível de parsear ou sem runtime instalado.

**Ele lê a L0 em processo deste processo, e nada além disso.**
`RenderCache::inspect` procura a chave apenas na L0; ele nunca consulta a
camada L1. No perfil Database
ou Redis isso importa: uma entrada que está viva em
`suprnova_render_entries` ou no Redis, publicada por outro nó ou por este
antes de um reinício, imprime `no entry` aqui, a menos que este processo a
tenha servido desde que iniciou. Leia o relatório como "o que este nó tem em
memória", nunca como "o que a implantação tem armazenado". O mesmo vale para
`RenderCache::store_inspection`, que relata a ocupação da L0 e o epoch
atual.

A chave é o texto que a própria busca usa: `rk1.` mais 43 caracteres
base64url, que é o que o logging e a telemetria da sua aplicação podem
expor. Ela não é um segundo hash de nada, então uma chave que um operador
tem em mãos nomeia exatamente uma entrada.

Essa afirmação de ausência de corpo é verificada, e não meramente declarada.
O teste pega o documento que foi de fato servido, divide-o em linhas e exige
que **toda** linha não trivial dele esteja ausente do que o relatório de
inspeção imprimiu.

**`render-cache:epoch-advance`** é a invalidação de emergência. Ele avança o
epoch de autoridade e imprime `epoch advanced to {epoch}`. Como o epoch está
embutido em toda chave de busca, isso põe as entradas armazenadas fora de
alcance sem nada a enumerar e nada a excluir. O teste afirma a linha impressa
e depois a consequência que importa: depois do comando, a rota renderiza de
novo.

**No nó que o executa**, o efeito é imediato: o comando descarta o lease de
epoch daquele processo e limpa a sua camada em processo, então a sua próxima
requisição deriva chaves sob o novo epoch e não encontra nada. (Essa última
cláusula vale enquanto o epoch só avançar, que é o caso comum; depois de uma
restauração de banco de dados o valor avançado pode ser um que a implantação
já usou antes, então veja "Restaurando o banco de dados" abaixo.) **Em
qualquer outro nó**, o ledger se moveu mas aquele processo ainda segura o seu
epoch antigo em lease e a sua própria L0, e ele se atualiza na sua próxima
leitura de autoridade - imediatamente sob `CoherenceMode::Authority`, e até
`max_age_ms` depois sob `CoherenceMode::Lease`. Execute o comando em cada nó,
ou reinicie os outros. "Restaurando o banco de dados" abaixo tem o
procedimento completo e os testes por trás dele.

Recorra a ele quando algo estiver errado com o conteúdo em cache e você não
puder esperar que entradas individuais expirem, e depois de um job que mudou
o que as páginas em cache exibem (veja "lacunas conhecidas" em
[Gerações do RenderCache](render-cache-generations.md)).

## Mudanças de permissão

`RenderCache::bump_permission_version().await?` é a única chamada de
invalidação que uma aplicação faz à mão, e ela não é realmente um comando de
operações - o lugar dela é no caminho de código que muda o que um usuário
autenticado tem permissão para fazer. Ela avança uma geração persistida que
toda renderização com chave por principal observa, sobrevive a um reinício, e
se junta à transação em que a mudança de papel roda, quando há uma. Sem ela,
um usuário cujas permissões acabaram de mudar continua correspondendo ao que
estava em cache sob o seu conjunto de permissões anterior.

## Telemetria

Nove nomes fechados de contador, e nada em nenhum deles nomeia uma camada,
um provedor ou um backend:

| Contador | Atributo |
|---|---|
| `suprnova.render_cache.lookups` | `outcome`, e `reason` quando `outcome="declined"` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | nenhum |
| `suprnova.render_cache.rebuilds` | nenhum |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |
| `suprnova.render_cache.stitch.nested` | `outcome`, `cause` |
| `suprnova.render_cache.hints` | `outcome` |
| `suprnova.render_cache.epoch_rewinds` | nenhum |

`lookups` e `hits` carregam o mesmo conjunto fechado de oito desfechos:

- `l0`, `l1` - uma entrada válida servida da camada em processo ou da
  compartilhada.
- `conditional` - um hit válido cujo `If-None-Match` correspondeu,
  respondido com `304`.
- `stale` - uma entrada obsoleta servível servida imediatamente, ou o
  fallback de obsoleta em erro depois que uma reconstrução em primeiro plano
  falhou.
- `miss` - nada encontrado, uma reconstrução de obsoleta em erro em
  andamento, ou uma entrada morta.
- `bypass` - um parâmetro de consulta não declarado, uma dimensão de
  variância declarada que não pôde ser resolvida, ou uma lista de waiters
  esgotada.
- `moved` - a releitura depois de renderizar encontrou uma dependência ou o
  epoch alterados; o candidato foi descartado, nunca publicado.
- `declined` - a renderização não era armazenável, por uma de trinta e
  oito razões abaixo, carregada no atributo `reason` ao lado de `outcome`.
  `reason` só é emitido junto de `outcome="declined"`; qualquer outro
  desfecho não carrega nenhum. A razão é calculada a partir de um valor
  tipado no ramo exato que recusou, nunca reconstruída depois a partir da
  resposta, de modo que nomeia o contrato que de fato recusou a
  renderização:

  - Elegibilidade (`policy.eligibility`, espelhando o próprio
    `DeclineReason` do motor): `policy_uncacheable`, `method`, `status`,
    `streaming`, `sets_cookie`, `unsafe_header_name`.
  - Observação (o relatório do coletor e a leitura do ledger dentro da
    transação): `observation_overflowed`, `ledger_read_failed`,
    `handler_not_begun`.
  - Classificação reduzida a `Uncacheable`: `session_value_read`,
    `secret_context_read`, `undeclared_context`.
  - Fatos do documento Live: `identity_bound_without_stitching`,
    `invalid_stitch_capture`, `no_store_intent`,
    `unresolvable_seed_deadline`.
  - Invariantes sobre a chave (se as próprias observações da renderização
    concordam com os valores com que a chave de lookup já havia sido
    construída): `unreasoned_private_class`, `principal_undeclared`,
    `principal_divergent`, `tenant_undeclared`, `tenant_divergent`,
    `locale_undeclared`, `locale_divergent`.
  - Publicação: `seed_deadline_elapsed`, `unsafe_header_value`,
    `composite_capture_invalid`, `composite_slot_count_mismatch`,
    `composite_too_many_slots`, `composite_digest_mismatch`,
    `composite_empty_slot`, `composite_slot_not_found`,
    `composite_slot_ambiguous`, `composite_nested_unauthorizable`,
    `composite_nested_wider_class`, `composite_nested_longer_freshness`,
    `composite_nested_depth_exceeded`, `composite_nested_cycle`,
    `composite_nested_unresolvable`.

`hits` incrementa apenas para `l0`, `l1`, `conditional` e `stale`.
`publications` conta apenas um armazenamento respondendo "publicado", nunca
uma tentativa barrada por fence ou rejeitada. `rebuilds` conta uma por
reconstrução em segundo plano disparada.

Os dois contadores de costura de ilha carregam os seus próprios conjuntos:
`assembled` e `fail_document` para montagens; `rendered`, `omitted`,
`fallback` e `failed` para slots.

`suprnova.render_cache.stitch.nested` distingue o próprio desfecho de um
segmento interno em cache e nomeado do de um slot de ilha, com um
incremento por tentativa de resolução de um `Segment::Nested`. O seu
atributo `outcome` assume exatamente um de `resolved`, `omitted`,
`fallback` e `failed`; o seu atributo `cause` assume exatamente um de
`none` (usado apenas quando `outcome="resolved"`), `fetch_failed`,
`version_mismatch`, `length_mismatch`, `depth_exceeded`, `cycle` e
`unauthorized`. Nenhum dos dois atributos carrega alguma vez uma chave, um
nome de rota ou um digest de identidade. Um segmento falho ou degradado
sempre se resolve através da política que o grafo que o inclui declarou
para ele (`FailDocument`/`Omit`/`Fallback`), exatamente como acontece com a
própria falha de um slot de ilha; `outcome="failed"` (proveniente de uma
política `FailDocument`) abandona a montagem do documento inteiro e recai
no próprio handler sem cache da rota.

`epoch_rewinds` conta detecções, não entradas: um incremento a cada vez
que um nó encontra uma entrada ou um epoch em lease carimbado acima do
próprio epoch da autoridade, sobe o epoch do ledger para além desse
carimbo, e limpa a sua própria L0. Um valor diferente de zero depois de
uma restauração de banco de dados é o sinal de que a restauração foi
percebida. Um valor diferente de zero em qualquer outro momento significa
que uma autoridade andou para trás por um motivo que ninguém pretendia.

`hints` conta as dicas de geração credíveis que este nó tratou no canal
pub/sub da camada 2: um incremento por mensagem recebida, um por assinatura
encerrada e um por anúncio que uma fila de publicação cheia impediu este nó
de enviar. É o único contador aqui que uma implantação pode deixar
permanentemente em zero por escolha própria: as dicas ficam desligadas a
menos que o perfil Redis, ou `RENDER_CACHE_HINTS=redis`, as ligue, e um nó
com elas desligadas serve exatamente o que um nó com elas ligadas serve. O
seu atributo `outcome` assume exatamente um de `applied` (a mensagem nomeou
um digest que um lease de validação deste nó observa, e todos esses leases
foram encurtados), `ignored_unknown_key` (não nomeou nada contra o que este
nó tenha um lease, o que inclui uma mensagem que este nó não consegue ler de
jeito nenhum), `dropped_over_bound` (carregava mais de 64 digests e foi
descartada inteira em vez de truncada, porque uma dica truncada é uma dica
silenciosamente errada), `subscriber_dropped` (a assinatura deste nó
terminou, porque ficou para trás ou porque a conexão falhou, e está sendo
restabelecida) e `dropped_publish_queue_full` (este nó tinha um avanço a
anunciar e a sua própria fila de publicação estava cheia, de modo que a
mensagem foi descartada em vez de fazer esperar a escrita que a produziu, uma
contagem para cada mensagem que ficou sem anúncio). Nenhum atributo carrega
jamais uma rota, uma chave ou uma identidade de dependência.

Uma dica só pode encurtar um lease de validação que este nó já detém. Nunca
pode estender um, criar um, nem fazer as vezes do ledger de gerações, e todo
hit continua lendo esse ledger. Portanto um `subscriber_dropped` em alta
significa que este nó revalida mais tarde do que poderia - no pior caso tão
tarde quanto o próprio `max_age_ms` do lease, que é o limite de obsolescência
que a rota já declarou - e nunca que algo está sendo servido que a
verificação de coerência teria recusado.

**Uma taxa alta de `declined` é o sinal que vale um alerta.** Ela significa
que rotas que você incluiu estão renderizando e servindo corretamente sem
jamais serem armazenadas, e a resposta parece idêntica dos dois jeitos. A
verificação local mais rápida são duas requisições seguidas: se a segunda não
carrega cabeçalho `Age`, nada foi armazenado.

## Higiene de disco

Só a camada de arquivo precisa de varredura, e ela em grande parte se varre
sozinha.

O `FileRenderStore` armazena um arquivo por chave, plano sob
`RENDER_CACHE_L1_DIR`. Uma entrada está morta quando a sua idade desde a
publicação alcança a retenção com que ela foi publicada, ou quando o seu
epoch de fence é mais antigo que o epoch atual. A retenção vem da mesma
borda de morte ciente de classe que a verificação de validade viva usa, então
o arquivo de uma entrada privada é aposentado mais cedo que o de uma pública
e uma varredura nunca pode discordar de uma verificação de validade sobre se
uma entrada está de fato morta.

`sweep` remove no máximo 64 entradas por chamada, da publicação mais antiga
para a mais nova, e retorna se ainda restam mais. Ela roda automaticamente a
cada 256ª publicação, então um diretório saudável não precisa de atenção. O
`RenderCache::sweep()` a conduz explicitamente quando você quiser, e uma fila
maior que o limite de uma chamada escoa ao longo de disparos posteriores em
vez de travar em uma única varredura longa.

Duas coisas que a varredura não é:

- **Um avanço de epoch não toca na L1.** Ele limpa a L0 por completo, porque
  aquilo é memória em processo sem nada com que reconciliar, e deixa todo
  arquivo pré-epoch no disco até que uma varredura o recupere. Isso é
  higiene de disco, não uma questão de corretude - os arquivos já estão
  inalcançáveis por busca.
- **A camada de banco de dados não tem varredura automática**, e é
  recuperada apenas por meio de `RenderCache::sweep()`. A camada Redis não
  precisa de nenhuma: toda entrada que ela armazena carrega uma expiração e
  o Redis recupera os bytes sozinho.

A publicação é segura contra queda. Ela escreve um arquivo temporário, faz
fsync dele, renomeia-o sobre o alvo e faz fsync do diretório pai, então um
leitor só chega a ver o arquivo completo anterior ou o arquivo completo novo.
Na abertura, o armazenamento remove qualquer arquivo temporário remanescente
e qualquer arquivo que falhe na verificação de frame, tratando uma escrita
rasgada como algo autocurável em vez de uma entrada permanentemente
envenenada.

## Testando uma rota em cache

Um teste que afirma que uma rota em cache responde corretamente passa quer a
resposta tenha vindo do armazenamento, quer tenha vindo de uma renderização
nova. Toda afirmação precisa ser feita contra algo que apenas uma entrada
armazenada sendo de fato servida consegue produzir. Quatro padrões fazem
isso, e os próprios testes de dogfood deste repositório usam os quatro:
`app/tests/live_render_cache.rs` com o harness em
`app/tests/live_support/mod.rs`.

**1. Conte as renderizações do lado do handler do cache.** Registre um
middleware de contagem *depois* de `RenderCache::install`. O registro
acrescenta, então ele fica mais perto do handler que o
`RenderCacheMiddleware`, e uma requisição que o cache responde retorna antes
de chamá-lo:

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// Depois da instalação, para que ele só veja requisições que o cache encaminhou.
render_counter::register();
```

A diferença entre duas leituras de `render_counter::renders()` é então o
número de renderizações que o cache não evitou, e nada mais - ao contrário de
corpos idênticos ou de um cabeçalho `Age`, que têm ambos explicações honestas
sem cache. Toda afirmação de hit em
`an_orm_write_invalidates_the_todos_document_through_generations` se apoia
nisso. Espere por uma renderização que você não disparou (uma reconstrução em
segundo plano) com a barreira do próprio contador,
`wait_until_renders_at_least`, nunca com um sleep.

**A exceção é uma rota `PublicShellStitched`, e não é uma exceção pequena.**
Um hit costurado é deliberadamente encaminhado por toda a cadeia da rota - a
guarda de autorização dela precisa rodar de novo, e só o middleware de
conclusão do Live no fim daquela cadeia serve o hit. Um middleware de
contagem registrado globalmente depois da instalação fica fora da cadeia
própria da rota, então ele é alcançado em um hit costurado exatamente como em
um miss. Em uma rota dessas o contador não consegue sustentar a afirmação de
"nenhum handler rodou", de forma alguma.

Afirme, em vez disso, sobre o que o armazenamento guarda, sobre do que o
documento servido é feito e sobre que idade ele tem, que é o que
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` faz: a
entrada armazenada é um `EntryKind::Composite` com a contagem de slots
esperada (`inspect_route_for_test`), os documentos de dois principais diferem
nas suas tags de ilha e em nenhum outro lugar, e a resposta ao segundo
principal informa um `Age` dos segundos inteiros passados desde que o shell
foi publicado.

Esse último é a prova em que o teste se apoia, e é o comprovante local de
serviço a partir do armazenamento na sua forma exata. O cabeçalho `Age`
sozinho é o sinal fraco contra o qual este capítulo avisou acima, porque uma
renderização também põe um: em zero. O número não é fraco: uma renderização
publica a sua resposta e a sua entrada no mesmo instante, então uma resposta
renderizada informa zero por mais que o relógio tenha andado, enquanto uma
montagem informa a idade do shell a partir do qual foi montada. O teste move
um relógio ajustável, bem dentro da janela de validade da rota, para que o
que ele lê seja exato e não incidental.

A resposta também carrega `Cache-Control: private, no-store`, mas leia isso
pelo que é: a diretiva que uma rota com slots desta classe carrega, fixada
tanto na renderização que publica o shell quanto em cada montagem posterior,
porque ela segue o que os bytes guardam e não o caminho que os produziu. A
palavra operativa é "com slots": um `Composite` de zero slots mantém, em vez
disso, o `max-age` privado da classe, então a diretiva diz algo de uma rota
com ilhas nela e nada de uma sem elas. Aquele teste afirma
`renders() == before + 1` em um hit, e diz na sua própria nota por que essa é
a leitura honesta em vez de uma falha.

**2. Leia a entrada de volta.** Duas chamadas de facade são API pública
comum: `RenderCache::store_inspection()` relata a ocupação da L0, os bytes e
o epoch atual, e `RenderCache::inspect(key_text)` relata os metadados sem
corpo de uma entrada. Ao lado delas, o framework expõe costuras ocultas de
teste - `#[doc(hidden)]`, e nomeadas com `_for_test` para que nada as
confunda com API de aplicação:

| Costura | O que ela dá a um teste |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | o texto de chave que o middleware deriva **no epoch 1**, o valor que a migração semeia |
| `RenderCache::key_for_route_at_epoch_for_test(pattern, params, login, epoch)` | o mesmo, sob um epoch que você nomeia |
| `RenderCache::inspect_route_for_test(pattern)` | a entrada da L0 daquela chave de epoch 1: classe, tipo, status, `body_bytes`, slots |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | o mesmo, saído da camada L1 configurada |
| `RenderCache::clear_l0_for_test()` | esvazia a L0 e deixa a L1, o epoch e o coordenador em paz |

O epoch importa porque ele faz parte da chave. O
`key_for_route_for_test` fixa o epoch 1, então um teste que avançou o epoch -
neste nó ou, através do ledger, em outro - precisa nomear o novo com
`key_for_route_at_epoch_for_test`, ou vai buscar uma chave sob a qual nada
foi publicado.

`the_public_document_is_a_hit_whose_seed_still_promotes` usa
`store_inspection` e `inspect_route_for_test` para afirmar que a entrada
existe e está armazenada sob a classe declarada;
`the_database_profile_serves_a_hit_through_the_sql_stores` usa
`inspect_l1_for_test` e depois `clear_l0_for_test`, que é a única forma de
provar que uma requisição posterior saiu da L1 e não da memória.

**3. Mova o relógio em vez de esperar.** O relógio que o runtime lê é
definível em uma `RenderCacheConfig` e nunca por `from_env`, então um teste
que precisa de uma faixa de validade instala o seu próprio:

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Ligado primeiro a um nome próprio: passar `Arc::clone(&clock)` embutido
// deixa o compilador inferindo o trait object como o tipo de retorno do clone.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... instale pela costura de configuração da própria aplicação, então:
clock.advance_ms(300_001);
```

O `AdjustableTestClock` vem de `suprnova::live::testing`, e o `unix_now_ms`
é a leitura de relógio de parede do próprio harness, de modo que um relógio
ajustável começa onde o do sistema está, e não em uma origem de tempo com a
qual o resto do processo discordaria. (Isso é um zero de relógio, não o epoch
de autoridade que este capítulo em geral quer dizer com essa palavra.) O
harness embrulha o par como `setup_app_with_clock` e `advance_clock_ms`, o
segundo dos quais entra em pânico em vez de silenciosamente não fazer nada
quando o boot pegou o relógio do sistema.
`stale_service_is_marked_and_rebuilt_in_the_background` é o teste.

**4. Conte instruções SQL.** Um cache que pulou o handler mas ainda
consultou o banco de dados a cada hit satisfaz todo contador do lado do
handler e ainda custa uma ida e volta. O
`DbConnection::observe_statements_for_test` aponta o callback de métrica do
SeaORM para um contador seu, e ele enxerga instruções no pool e em toda
transação iniciada a partir dele:

```rust
// Imediatamente depois de conectar, antes de a conexão ser clonada ou
// vinculada ao container: instalar exige posse exclusiva do pool, e a
// chamada relata `false` em vez de contar nada silenciosamente.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

Nada é dito ao callback sobre a instrução - nenhum texto SQL, nenhum valor
vinculado - porque uma contagem é o objetivo inteiro. O
`framework/tests/render_cache/bypass.rs` é escrito inteiramente sobre esse
padrão: `a_lease_mode_hit_runs_nothing_and_issues_no_statement` prende um hit
em modo lease a zero instruções,
`an_authority_mode_hit_issues_exactly_one_statement` prende um hit em modo
authority a uma, e
`the_epoch_is_read_once_at_first_use` mede dois misses um contra o outro para
mostrar que o epoch custa uma leitura por runtime.

Dois hábitos que vale manter. Inicialize o harness pela costura de
configuração da sua própria aplicação em vez de por um router construído à
mão, para que o teste instale as mesmas rotas, políticas e ordenação de
middleware que o servidor instala. E nunca adicione uma espera cronometrada:
toda barreira acima é uma barreira de estado sobre um contador, que é o que
torna esses testes reproduzíveis em vez de instáveis.

## Quando algo está errado

- **Uma página está exibindo conteúdo que você sabe ser antigo.** Verifique
  se a rota está armazenando (duas requisições, procure o `Age`). Todo
  processo cuja configuração habilita o RenderCache e cujo banco de dados
  contém a migração do RenderCache avança gerações para suas próprias
  escritas, então um worker de fila, uma tarefa agendada ou um comando de
  console invalida as mesmas gerações que o processo que serve invalidaria;
  confirme que o processo que escreveu realmente tem o RenderCache
  habilitado e migrado, já que um que não tem não avança nada. Sob
  `CoherenceMode::Lease`, uma entrada obsoleta mas ainda armazenada se
  atualiza sozinha dentro de `max_age_ms`, em vez de imediatamente. Para
  tudo o mais, execute `render-cache:epoch-advance` (por nó - veja o
  último item).
- **Uma página que você esperava que entrasse em cache nunca carrega um
  cabeçalho `Age`.** Ela está sendo recusada, não falhando. Leia primeiro o
  rótulo `reason` do lookup `declined` - ele nomeia o contrato exato que
  recusou a renderização, a partir do conjunto fechado em "Telemetria"
  acima - depois, para uma das razões reduzidas por classificação, percorra
  a lista de classificação em [RenderCache](render-cache.md): uma leitura
  de sessão, uma leitura de identidade em uma rota sem variância
  `Principal`, uma leitura de localidade sem variância `Locale`, uma
  verificação de autorização, ou uma leitura de SQL bruto.
- **Um backend está inalcançável.** O `RENDER_CACHE_FAILURE` decide: `open`
  (o padrão) serve a rota sem cache, `closed` responde um `503` puro. Um
  backend ausente no boot interrompe o boot, com uma frase nomeando a
  migração ou a variável a corrigir.
- **O Redis foi esvaziado ou reiniciado.** As entradas dão miss e são
  rerrenderizadas. Nada obsoleto pode ser provado atual: a atualidade é
  provada contra o ledger de gerações do banco de dados, nunca contra a
  camada que guardava os bytes.
- **Um líder de reconstrução morreu no meio da reconstrução.** O lease dele é
  assumido assim que o tempo do armazenamento passa da expiração, e a
  publicação do antigo líder é barrada pelo fence em vez de disputar com a
  nova. Ele não publica nada; a resposta da requisição dele ainda é servida.
- **Um arquivo da L1 foi rasgado por uma queda ou por um disco cheio.** Nada
  o serve. Cada arquivo carrega um digest sobre o seu próprio frame, então um
  arquivo truncado ou alterado falha nessa verificação e é um miss; o
  armazenamento o remove, junto com qualquer arquivo temporário remanescente,
  da próxima vez que abrir. Uma escrita rasgada aqui é autocurável em vez de
  uma entrada permanentemente envenenada.
- **O banco de dados foi restaurado de um backup.** Este tem um procedimento
  em vez de uma frase; veja "Restaurando o banco de dados" abaixo.
- **Você precisa que tudo suma, agora.** `render-cache:epoch-advance`. Em
  mais de um nó, execute-o em cada um, ou reinicie aqueles em que você não o
  executou: o avanço move o epoch do ledger para a implantação inteira, mas
  limpa a L0 e descarta o epoch em lease apenas no processo que o executou. O
  procedimento de restauração abaixo explica por quê.

## Restaurando o banco de dados

O ledger de gerações é a autoridade contra a qual todo hit é provado, então
restaurar o banco de dados muda o que "atual" significa para toda entrada já
armazenada. Duas coisas decidem o que uma entrada armazenada faz em
seguida, e nenhuma delas é "ela é silenciosamente descartada".

**Isso é tratado para você.** A primeira leitura de autoridade depois da
restauração que encontra um epoch ou uma entrada carimbados acima do valor
restaurado recusa essa entrada de imediato - nem servida uma vez sob
`Warning`, em nenhuma idade, seja lá o que a política de validade da rota
diga -, reconstrói-a, sobe o epoch do ledger para um a mais que o carimbo
mais alto que viu, substitui o lease de epoch daquele nó pelo valor
elevado, e limpa a L0 daquele nó. Todo outro nó vê o epoch elevado na sua
própria próxima leitura de autoridade: imediatamente sob
`CoherenceMode::Authority`, e dentro de `max_age_ms` sob
`CoherenceMode::Lease`. `suprnova.render_cache.epoch_rewinds` conta cada
detecção.

É isso, e é a mesma convergência que um `render-cache:epoch-advance` de
operador produz, alcançada sem o operador. Os três testes
`an_epoch_advanced_by_another_node_*` em
`framework/tests/render_cache/middleware.rs` medem o limite de propagação,
e `a_rewound_epoch_refuses_the_entry_rebuilds_and_lifts` mede a recusa.

**Um passo opcional resta.** Esvazie a camada L1 compartilhada se uma rota
com uma janela de obsolescência servível não puder servir nem uma vez uma
representação pré-restauração antes da sua reconstrução. A subida de epoch
é o que torna isso alcançável: uma entrada de L1 carimbada *abaixo* do
epoch elevado volta a ser uma entrada movida comum, e uma entrada movida em
uma rota assim é servida uma vez sob `Warning` enquanto a reconstrução roda
atrás da requisição. Apague o conteúdo do diretório da camada de arquivo,
`DELETE FROM suprnova_render_entries`, ou apague as chaves Redis que casam
com `<prefix>entry:*` - o que quer que o perfil configure. Pule esse passo
e o pior caso é um corpo pré-restauração marcado `Warning` por cada chave
assim.

## Medindo

O RenderCache traz dois benchmarks, e eles são **ferramentas sob demanda,
nunca passos do gate**:

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

Isso roda o bench da engine (`render_cache_budget`, as medições de hit quente
e de montagem composta com um alocador que conta), depois o bench de carga do
framework (`render_cache_workloads`, a mesma rota através de todo o
middleware), depois o teste de contrato sobre os resultados versionados. Os
dois são fixados em `SUPRNOVA_LIVE_S1_CPUSET`.

Uma execução completa precisa de um PostgreSQL descartável (`PG_TEST_URL`) e
de um Redis descartável (`REDIS_TEST_URL`), porque o contrato de resultados
versionados exige os três perfis registrados. **Uma execução parcial precisa
redirecionar os dois arquivos de resultado** com
`SUPRNOVA_LIVE_BENCH_RESULT` e `SUPRNOVA_LIVE_WORKLOADS_RESULT` para dentro
de `benchmarks/local/`; sem isso ela sobrescreve os resultados versionados
com um arquivo mais curto e depois falha no seu próprio contrato.

Os números versionados, de
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` e
`render-cache-workloads-v1.json`:

| Medição | Valor |
|---|---|
| Trabalho da engine para um hit `Complete` válido na L0, p95 | 0,76 microssegundos |
| Alocações de heap, hit válido | 3 |
| Alocações de heap, hit condicional `304` | 3 |
| Alocações de heap, hit limitado por um prazo de semente | 4 |
| Cópias de corpo em qualquer um deles | nenhuma; o buffer é compartilhado |
| A mesma rota através do middleware, lado servidor, p95 | 14,4 microssegundos |
| A mesma requisição em uma ida e volta HTTP por loopback, p95 | 109 microssegundos |
| Instruções SQL por hit quente (modo lease) | 0 |

Os números do middleware são para um corpo de 65.536 bytes cuja renderização
leu 12 linhas, registradas como 14 identidades de dependência observadas.

**Leia isso como exploratório, e não como evidência qualificada.** Todo
resultado versionado carrega `"classification": "local_exploratory"` e
`"s1_requirements_met": false`: eles foram produzidos em uma estação de
trabalho de desenvolvimento, com CPU compartilhada, governador `powersave` e
provedores em loopback. Eles servem para pegar uma regressão de uma ordem de
grandeza inteira, e para nada mais fino que isso. Um número é evidência
qualificada apenas quando foi produzido na máquina dedicada com a sua
atestação definida, e estes não foram.

### Por que Suprnova diverge

Os pacotes de cache de resposta do Laravel deixam as operações a cargo do
armazenamento de cache por baixo. Inspecionar uma entrada significa achar a
chave dela à mão e ler o valor - que é a página renderizada, então olhar para
ele significa imprimir o HTML de alguém em um terminal - e invalidar tudo
significa fazer flush de um armazenamento que também guarda as suas sessões,
os seus limites de taxa e a sua fila. A observabilidade é o que o driver do
armazenamento por acaso emitir.

O Suprnova dá ao cache a sua própria superfície operacional, deliberadamente
estreita. A inspeção é sem corpo por construção, então um operador pode
confirmar que uma entrada existe, sob que classe ela está armazenada e qual é
o tamanho dela, sem jamais ver o seu conteúdo. A invalidação é um incremento
de epoch que não custa nada para aplicar e toca apenas neste cache - as suas
sessões e a sua fila não estão no raio da explosão. A telemetria é um
conjunto fechado de nove contadores com conjuntos fechados de atributos, que
é o que torna um dashboard sobre eles estável entre releases em vez de um
punhado de strings que derivam. A troca é que não existe comando de "apague
esta chave": as alavancas são por entrada e somente de leitura, ou de epoch
inteiro.

## Próximos passos

- [RenderCache](render-cache.md) - as declarações sobre as quais estes
  comandos operam
- [Observabilidade](observability.md) - onde os contadores acima são
  exportados
- [Testes](testing.md) - as convenções de teste ao redor dentro das quais os
  padrões acima se encaixam
- [Implantação](deployment.md) - o checklist de produção ao redor deles
