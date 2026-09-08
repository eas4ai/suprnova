# Operações do RenderCache

Um cache que você não consegue ver é um cache em que você não consegue
confiar. O RenderCache responde a duas perguntas de operador diretamente e
sem jamais imprimir uma página armazenada: **o que este nó está guardando
sob esta chave, e isso ainda está atual?** e **como eu faço tudo parar?**
Ele responde a uma terceira - "esta rota está sendo servida a partir de uma
cópia armazenada, afinal?" - por telemetria e pelo cabeçalho `Age` em vez de
por um comando, porque essa pergunta é sobre tráfego e não sobre uma
entrada. Há dois comandos de console, seis contadores de telemetria, uma
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

Seis nomes fechados de contador, e nada em nenhum deles nomeia uma camada,
um provedor ou um backend:

| Contador | Atributo |
|---|---|
| `suprnova.render_cache.lookups` | `outcome` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | nenhum |
| `suprnova.render_cache.rebuilds` | nenhum |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |

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
- `declined` - a renderização não era armazenável: elegibilidade, um
  relatório de observação transbordado, uma classificação `Uncacheable`, uma
  regra de documento Live, ou um limite.

`hits` incrementa apenas para `l0`, `l1`, `conditional` e `stale`.
`publications` conta apenas um armazenamento respondendo "publicado", nunca
uma tentativa barrada por fence ou rejeitada. `rebuilds` conta uma por
reconstrução em segundo plano disparada.

Os dois contadores de costura carregam os seus próprios conjuntos:
`assembled` e `fail_document` para montagens; `rendered`, `omitted`,
`fallback` e `failed` para slots.

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

Afirme, em vez disso, sobre o que o armazenamento guarda e sobre do que o
documento servido é feito, que é o que
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` faz: a
entrada armazenada é um `EntryKind::Composite` com a contagem de slots
esperada (`inspect_route_for_test`) e os documentos de dois principais
diferem nas suas tags de ilha e em nenhum outro lugar. A resposta também
carrega `Cache-Control: private, no-store`, mas leia isso pelo que é: a
diretiva da classe inteira, fixada tanto na renderização que publica o shell
quanto em cada montagem posterior, porque ela segue o que os bytes guardam e
não o caminho que os produziu. A palavra operativa ali é "com slots": um `Composite` de
zero slots mantém, em vez disso, o `max-age` privado da classe, então aquela
afirmação serve a uma rota com ilhas nela e não a uma sem elas. Aquele teste
afirma `renders() == before + 1` em um hit, e diz na sua própria nota por que
essa é a leitura honesta em vez de uma falha.

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
  se a rota está armazenando (duas requisições, procure o `Age`). Se estiver,
  e a escrita que deveria tê-la invalidado veio de um worker de fila, de uma
  tarefa agendada ou de um comando de console, essa escrita não avançou nada:
  execute `render-cache:epoch-advance` (por nó - veja o último item).
- **Uma página que você esperava que entrasse em cache nunca carrega um
  cabeçalho `Age`.** Ela está sendo recusada, não falhando. Percorra a lista
  de classificação em [RenderCache](render-cache.md): uma leitura de sessão,
  uma leitura de identidade em uma rota sem variância `Principal`, uma
  leitura de localidade sem variância `Locale`, uma verificação de
  autorização, ou uma leitura de SQL bruto.
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
armazenada. Três fatos do código decidem o que uma entrada armazenada faz em
seguida, e nenhum deles é "ela é silenciosamente descartada".

**Uma entrada movida não é automaticamente retida.** A comparação de
coerência (`CoherenceCheck::compare`) é uma desigualdade em *qualquer* das
direções, então uma entrada armazenada cujas gerações observadas diferem das
do ledger restaurado é um movimento para qualquer lado que os números tenham
ido. Mas um movimento não é uma recusa de servir: o middleware avalia uma
entrada movida com uma idade efetiva de pelo menos o seu intervalo de
validade (`freshness_state` em
`framework/src/render_cache/middleware.rs`) e, em uma rota que declara uma
janela de obsolescência servível, isso a coloca na faixa de obsoleta
servível. **O visitante recebe a cópia pré-restauração uma vez, sob
`Warning`, enquanto a reconstrução roda atrás da requisição.** Essa é a mesma
passagem de bastão que
[Gerações do RenderCache](render-cache-generations.md) descreve, e o passo 4
de `an_orm_write_invalidates_the_todos_document_through_generations` a
afirma. Uma rota `PrivateCached` nunca faz isso - a sua borda de morte é a
sua borda de validade - e uma rota que não declarou janela de obsolescência
servível também não; ambas reconstroem em primeiro plano.

**Um avanço de epoch é por processo.** O `RenderCache::advance_epoch` avança
o epoch do ledger, depois descarta o lease de epoch *deste* processo e limpa
a L0 *deste* processo. Os nós irmãos mantêm os dois: as suas entradas na L0 e
o epoch pré-restauração que eles têm em lease. Cada um fica sabendo na sua
próxima leitura de autoridade - imediatamente no seu próximo hit sob
`CoherenceMode::Authority`, e até `max_age_ms` depois sob
`CoherenceMode::Lease` - que é exatamente o que os três testes
`an_epoch_advanced_by_another_node_*` em
`framework/tests/render_cache/middleware.rs` medem. Até lá um irmão pode
servir uma entrada pré-restauração e, em uma rota obsoleta servível, pode
servi-la sob `Warning` como acima.

**A camada compartilhada não é varrida só por uma mudança de epoch.** A
varredura da camada de arquivo remove uma entrada quando a sua retenção
passou *ou* o seu epoch de fence é `<` que o atual. Se restaurar o backup
baixou o epoch do ledger para abaixo de valores sob os quais a implantação já
havia publicado entradas, essas entradas carregam um epoch de fence que agora
é *maior* que o atual, então essa cláusula não as recupera; elas esperam a
sua retenção acabar em vez disso. A camada de banco de dados é varrida
apenas por um `RenderCache::sweep()` explícito. A camada Redis se recupera
sozinha, mas no ritmo do próprio Redis: cada hash de
entrada é armazenado sob `<RENDER_CACHE_REDIS_PREFIX>entry:<key>` (prefixo
padrão `suprnova_render:`) com um `PEXPIRE` definido a partir da retenção da
entrada, então esperar acabar a maior retenção que você declarou é a opção
passiva.

Então o procedimento, em ordem:

1. **Execute `render-cache:epoch-advance` uma vez**, antes que a implantação
   restaurada sirva. Ele falha ruidosamente em vez de relatar sucesso quando
   o singleton de epoch está ausente, que é também como você descobre que a
   migração não voltou junto com os dados.
2. **Esvazie a camada L1 compartilhada.** Apague o conteúdo do diretório da
   camada de arquivo, `DELETE FROM suprnova_render_entries`, ou apague as
   chaves Redis que casam com `<prefix>entry:*` - o que quer que o perfil
   configure. Faça isso em vez de esperar por uma varredura, pelo motivo
   acima.
3. **Cubra a L0 de cada nó, com o tráfego ainda desligado.** O avanço só
   limpou o nó que o executou, então até este passo estar feito um irmão não
   coberto ainda pode servir uma entrada pré-restauração uma vez - e é por
   isso que o tráfego fica desligado até aqui, e não até o passo 2. Ou
   reinicie os outros nós - um processo novo tem uma L0 vazia e nenhum epoch
   em lease, então a sua primeira requisição lê a autoridade restaurada - ou
   execute `render-cache:epoch-advance` em cada um deles, o que limpa a L0 de
   cada um conforme roda. A segunda opção incrementa o epoch do ledger uma
   vez por nó, o que não custa nada: o epoch só avança dali em diante, e todo
   nó acaba lendo o último valor. As duas são seguras; o reinício é
   a mais simples de raciocinar, e é a única que não precisa de aritmética de
   modo `Lease`.

Os passos 2 e 3 são o que torna o passo 1 completo em vez de parcial. Pule-os
e, em uma rota com uma janela de obsolescência servível, uma representação
pré-restauração ainda pode ser servida uma vez - corretamente marcada com
`Warning`, e reconstruída logo em seguida, mas servida.

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
conjunto fechado de seis contadores com conjuntos fechados de atributos, que
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
