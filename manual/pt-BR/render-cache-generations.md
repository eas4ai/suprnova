# Gerações do RenderCache

A maioria dos caches expira. O RenderCache também expira, mas a expiração é
a rede de segurança, e não o mecanismo. O mecanismo é uma **geração**: cada
pedaço de dado que uma renderização leu tem um contador no banco de dados, a
renderização armazena os contadores que viu, e uma escrita avança o contador
daquilo que ela mudou. Uma representação armazenada está atual quando os
contadores que ela viu ainda batem com os contadores que o banco de dados
guarda agora. Você não escreve nenhuma regra de invalidação para os seus
próprios dados, porque um `model.save()` comum já é uma.

Este capítulo é sobre essa maquinaria vista de fora: de que uma renderização
é registrada como dependente, quão grosseiras essas dependências realmente
são, o que o framework não consegue ver e portanto não consegue invalidar,
como a verificação de coerência é paga em um hit, qual requisição reconstrói
quando várias querem a mesma entrada ao mesmo tempo, e o que um visitante
recebe na janela entre "não está mais atual" e "reconstruída". Toda
afirmação abaixo é sustentada por um teste nomeado ou por uma medição
versionada; os exemplos de dogfood são rotas em `app/src/live/mod.rs`
provadas por `app/tests/live_render_cache.rs`.

## De que uma renderização é registrada como dependente

Enquanto uma renderização roda, um coletor com escopo de requisição registra
cada dependência que consegue nomear: a leitura de uma tabela, a leitura de
um registro por chave primária, uma classe de consulta, uma relação, uma
identidade de configuração, uma feature, uma localidade, uma rota, e uma
identidade `Broad` sempre presente que toda representação observa. Leituras
através do ORM e do construtor de consultas se registram sozinhas; você não
escreve nada.

`/live/todos` é o padrão inteiro em um só handler:

```rust
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView { count: titles.len(), titles })
    }
    .await;
    result.map_err(failed)
}
```

`Todo::all()` registra a tabela `todos`. Nada mais no handler ou no template
lê a sessão, o visitante autenticado ou uma tradução, que é o que permite à
rota continuar sendo uma representação compartilhada.

## Uma escrita comum é a invalidação

`an_orm_write_invalidates_the_todos_document_through_generations` percorre o
ciclo inteiro através da aplicação em execução:

1. O primeiro `GET /live/todos` renderiza e publica.
2. O segundo é um hit: ele nunca chega ao handler, não carrega `Warning` e
   não agenda nada.
3. Um `POST /todos/random` escreve uma linha - pela própria rota da
   aplicação, com a sessão e o token CSRF que um navegador enviaria.
4. O `GET` seguinte é servido com `Warning: 110 - "Response is Stale"` e
   agenda exatamente uma reconstrução em segundo plano. Seus cinco minutos
   de validade mal começaram, então a geração avançada da tabela `todos` é a
   única coisa que pode explicar qualquer um dos dois.
5. Essa reconstrução realmente roda: o teste espera no contador de
   renderizações - uma barreira de estado, não uma espera cronometrada - até
   que uma renderização que o próprio teste não disparou tenha acontecido.
6. A linha escrita realmente está na listagem. Este é um passo **separado** e
   deliberadamente não é uma afirmação sobre a saída da própria reconstrução
   em segundo plano: o teste descarta a L0 primeiro e renderiza de novo,
   porque a publicação da reconstrução acontece em um momento que nada
   alcançável a partir da aplicação torna observável, então afirmar sobre
   qualquer requisição que por acaso a pegasse seria uma corrida.
7. E a rota volta a ser um hit simples contra a entrada republicada.

Nenhuma chave de cache foi nomeada em lugar algum daquela sequência. Uma
escrita do ORM dentro de um `DB::transaction` avança as suas gerações dentro
daquela mesma transação, então uma escrita revertida não avança nada.

## A invalidação hoje tem granularidade de tabela

Esta é a coisa mais importante a saber antes de você dimensionar uma rota em
cache.

Uma leitura pontual através do ORM registra a identidade da **tabela** além
da identidade da linha. `Model::find` chama `observe_table_read(Self::TABLE)`
antes de procurar qualquer coisa e `observe_record_read_json` depois de
hidratar uma linha, então uma entrada que leu uma linha depende da tabela
inteira. Qualquer escrita naquela tabela portanto invalida **toda** entrada
em cache que leu dela, e não apenas as entradas que leram a linha que mudou.

Isso é seguro - só é capaz de invalidar demais, nunca de menos - e é medido
em vez de presumido. A carga de trabalho de tempestade de invalidação em
`framework/benches/render_cache_workloads.rs` publica 64 chaves sobre 12
identidades de registro, executa 1.000 escritas e registra o fan-out que
observou em
`crates/suprnova-live/benchmarks/render-cache-workloads-v1.json`
(resumido; o objeto registrado também carrega os campos de burst, varredura,
hit, reconstrução, instrução e latência):

```json
"invalidation_storm": {
  "keys": 64,
  "identities": 12,
  "writes": 1000,
  "every_write_invalidates_every_key": true,
  "rebuilds_per_write": 1.28,
  "final_bodies_coherent": true
}
```

Projete levando isso em conta. Uma rota em cache apoiada em uma tabela na
qual sua aplicação escreve constantemente vai reconstruir constantemente,
diga o que disser a sua janela de validade. Uma rota em cache apoiada em uma
tabela que muda quando um editor publica algo vai ficar parada por horas. Se
você precisa de granularidade mais fina que a tabela, a resposta honesta
hoje é que você não a tem.

## O que o framework não consegue ver

Uma dependência que não pode ser nomeada não pode ser invalidada, e o
framework é deliberado sobre quais delas recusa armazenar e quais deixa
passar.

**Recusadas de imediato.** SQL bruto por meio de `DB::select`,
`DB::select_one`, `DB::scalar` ou `DB::select_on` não consegue nomear as
tabelas que a sua instrução leu, então a renderização é marcada como não
observável e nunca é armazenada. A resposta ainda é servida, corretamente,
toda vez. As próprias verificações de papel e de permissão RBAC do framework
leem desse jeito, então uma rota em cache que avalia uma delas nunca
armazena. Leituras por meio de `DB::table(..)` conhecem a sua tabela e
entram em cache normalmente.

**Invisíveis, e de sua responsabilidade.** Um cabeçalho de requisição lido
por meio de `Request::header`, uma chamada a `Config::get` e um escopo
global do Eloquent que filtra uma consulta a partir do seu próprio estado
por requisição, todos mudam o que uma renderização produz sem que o coletor
veja nada. Declare a dimensão de variância correspondente em uma rota
dessas; nada aqui consegue capturar essa omissão por você.

**Lacunas conhecidas.** Nada avança uma geração para um sinalizador de
recurso quando o sinalizador muda, e uma escrita feita por um worker de
fila, uma tarefa agendada ou um comando de console não avança absolutamente
nada, porque apenas o processo que executou `RenderCache::install` carrega a
instrumentação do lado da escrita. Uma página que depende de tal escrita
permanece atual apenas dentro da sua janela de validade; execute
`render-cache:epoch-advance` depois de um job que muda o que as páginas em
cache exibem. Veja
[Operações do RenderCache](render-cache-operations.md).

## Quanto custa um hit

A verificação de coerência é o que transforma "temos bytes" em "estes bytes
estão atuais", e é o único trabalho que um hit faz.

Um hit não roda **nenhum handler, nenhuma consulta do ORM, nenhum template e
nenhum serializador**, e não copia byte algum de corpo: os bytes que o
servidor escreve no socket são os bytes que o armazenamento guarda, provado
por endereço em vez de por valor em
`framework/tests/render_cache/bypass.rs`. O que sobra é a leitura de banco de
dados que prova a atualidade, e com que frequência você paga por ela é o
`CoherenceMode` da política:

| Modo | Instruções SQL por hit quente | No que ele confia |
|---|---|---|
| `Authority` (padrão) | exatamente 1 | no ledger, relido a cada hit |
| `Lease { max_age_ms }` | 0 | em um lease de validação concedido localmente, até ele expirar |

`an_authority_mode_hit_issues_exactly_one_statement` prende o modo authority
a uma ida e volta: as gerações observadas e o epoch de autoridade são lidos
juntos em um único `UNION ALL`, e não como duas leituras.
`a_lease_mode_hit_runs_nothing_and_issues_no_statement` prende o modo lease
a zero, porque o epoch sob o qual a chave foi derivada é concedido em lease
junto com as gerações em vez de ser lido por requisição.

O epoch em si é lido uma vez por processo, e não uma vez por requisição.
`the_epoch_is_read_once_at_first_use` mede o primeiro miss de um runtime
novo contra um segundo runtime idêntico em tudo o mais e descobre que o
primeiro paga exatamente uma instrução a mais - a única leitura de
autoridade que preenche o lease de epoch. Toda requisição depois dessa não
paga nada por ele.

## Quando o epoch se move

`render-cache:epoch-advance` é a invalidação de emergência, e o epoch está
embutido em toda chave de busca, então o que acontece em seguida depende de
onde você está:

- **No nó que executou o comando**, a requisição seguinte já vê o novo
  epoch. A L0 é limpa por completo no mesmo instante, e o cache é invalidado
  imediatamente.
- **Em outro nó**, uma rota em modo `Authority` fica sabendo no seu próximo
  hit. Uma rota em modo `Lease` fica sabendo na sua próxima releitura de
  autoridade, que é no máximo `max_age_ms` depois.

Uma rota com uma janela de obsolescência servível serve a entrada movida uma
vez sob `Warning` enquanto a reconstrução roda atrás da requisição; uma rota
sem essa janela reconstrói em primeiro plano e quem pediu espera. Essa
diferença é a razão inteira para declarar uma janela de obsolescência
servível, e ela vale para qualquer movimento, não apenas para um avanço de
epoch.

Três testes em `framework/tests/render_cache/middleware.rs` prendem esses
caminhos pelo nome:
`an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`,
`an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`,
e
`an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.

## Uma reconstrução por chave: singleflight e waiters

Quando uma entrada está ausente ou não está mais atual, as requisições que
chegam por ela não renderizam todas. Elas são admitidas através de um
**coordenador de reconstrução**, que escolhe exatamente uma delas:

- O **líder** é a única requisição que renderiza e que pode publicar. Ele
  segura um lease sobre aquela chave pela duração da sua renderização.
- Os **waiters** são as requisições que chegam pela mesma chave enquanto o
  líder a segura. Elas esperam dentro do processo e, quando o líder libera,
  reavaliam o que está armazenado agora e servem aquilo. Um waiter nunca
  confia na espera: se o ciclo do líder deixou de publicar, ou publicou algo
  que a própria verificação de validade do waiter considera morto, o waiter
  também renderiza, em vez de servir o que encontrou.
  `a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh` em
  `framework/tests/render_cache/middleware.rs` é essa regra.
- Uma requisição que chega quando `RENDER_CACHE_MAX_WAITERS` (128 por
  padrão) já estão esperando faz **bypass**: ela renderiza e não publica
  nada, em vez de fazer crescer uma fila ilimitada.

`concurrent_misses_render_once_and_waiters_reuse_the_publication` prova o
caso comum de ponta a ponta - dois misses concorrentes, uma renderização,
corpos idênticos - e `one_leader_per_key_and_fence_with_bounded_waiters` em
`crates/suprnova-live/tests/render_cache_singleflight.rs` prova o limite
diretamente contra o coordenador: passado o seu limite de waiters, a
admissão responde `Bypass`.

Duas publicações para uma chave nunca podem ser ambas aceitas, seja lá o que
o coordenador tenha decidido. Um líder cunha um token de publicação sob o
seu lease, e o armazenamento compara esse fence antes de escrever: um epoch
mais antigo, ou um epoch igual com um token menor, perde. É isso que torna a
*renderização* duplicada segura de aceitar enquanto a *publicação*
duplicada não é, e é por isso que não existe espera entre nós: uma chave que
outro nó está reconstruindo é um bypass aqui. Veja
[Implantação do RenderCache](render-cache-deployment.md).

## Servir alguma coisa enquanto ela é reconstruída

Os quatro estados de validade, as faixas que a `FreshnessPolicy` define, e o
`Warning` e o `Age` que uma resposta obsoleta carrega estão definidos em
[Representações do RenderCache](render-cache-representations.md). O que
importa aqui é que um movimento de geração põe uma entrada nessas faixas
mais cedo: uma entrada movida é avaliada com uma idade efetiva de **pelo
menos** o seu intervalo de validade, qualquer que seja a sua idade real. A
idade real ainda decide em qual faixa isso a coloca:

- Idade real abaixo de `fresh_ms + stale_servable_ms`, em uma rota que
  declara uma janela de obsolescência servível: obsoleta servível. A cópia
  armazenada é servida uma vez sob `Warning` e a reconstrução roda atrás da
  requisição. Esse é o passo 4 do teste de escrita acima, em uma entrada
  cujos cinco minutos de validade mal tinham começado.
- Idade real além disso, mas ainda não na borda da morte: obsoleta em erro.
  A requisição espera por uma reconstrução em primeiro plano e vê a cópia
  armazenada apenas se essa reconstrução falhar.
- Em uma rota sem nenhuma janela de obsolescência servível, e em toda rota
  `PrivateCached` (cuja borda de morte *é* a sua borda de validade), um
  movimento cai em Morta: a requisição reconstrói em primeiro plano e espera.

`stale_service_is_marked_and_rebuilt_in_the_background` mostra a mesma
passagem de bastão conduzida pelo relógio em vez de por uma escrita: passados
os 300.000 milissegundos de validade de `/live/todos` e dentro dos seus
60.000 de obsolescência servível, o visitante recebe a cópia disponível sob
`Warning: 110 - "Response is Stale"` e `Age: 300`, exatamente uma
reconstrução é agendada, e essa reconstrução realmente roda.

O fallback de obsoleta em erro cobre a requisição que lidera uma
reconstrução **e** um waiter atrás de um líder cuja reconstrução falhou.
Ambos são respondidos da mesma forma: com os bytes obsoletos sob `Warning`,
em vez da falha. `framework/tests/render_cache/races.rs` prova cada braço
separadamente -
`a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on`
e `a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it` -
e o segundo por reversão: remover o fallback do braço que espera transforma
as suas afirmações finais de `200` em `500`.

Rotas costuradas são a exceção, e é uma exceção deliberada. Uma entrada
`Composite` nunca é servida pelo fallback de obsoleta em erro e nunca dispara
uma reconstrução em segundo plano: servir um shell armazenado depois de uma
reconstrução falha responderia a uma requisição que a própria cadeia de
autorização da rota nunca chegou a filtrar, e uma reconstrução em segundo
plano não carrega nada do estado de autorização da requisição, então o seu
shell seria o que a página renderiza para ninguém. Em uma rota costurada, o
resultado da própria reconstrução falha é o que o cliente vê.

## Rotas em cache são caminhos de leitura

A renderização do líder roda dentro de uma transação de banco de dados,
aberta em `REPEATABLE READ` no PostgreSQL e no MySQL, para que as gerações
que ela registra e os dados que ela leu compartilhem um único snapshot. Duas
consequências decorrem disso.

O handler de uma rota em cache que **escreve** compete com escritores
concorrentes pelas mesmas linhas e, no PostgreSQL, um handler que atualiza
uma linha que outra transação mudou depois que a renderização começou sofre
uma falha de serialização. Projete rotas em cache como caminhos de leitura.

Uma escrita feita fora de qualquer transação - `model.save()` sozinho -
confirma a sua linha primeiro e avança as suas gerações em uma transação
imediatamente seguinte. O momento entre as duas é "dado novo, geração
antiga": custa uma reconstrução extra e nunca serve conteúdo obsoleto.

Por fim, depois que a renderização termina, as dependências observadas e o
epoch são lidos de novo, fora da visão transacional da própria renderização.
Qualquer coisa que se moveu durante a renderização descarta o candidato em
vez de publicá-lo. É por isso que uma escrita que cai no meio de uma
renderização custa uma reconstrução em vez de uma página errada.

### Por que Suprnova diverge

O cache do Laravel é um armazenamento chave-valor e os seus pacotes de cache
de resposta são construídos em cima dele, então a invalidação é algo que
você escreve. Você chama `Cache::forget`, ou marca entradas com tags e faz
flush de uma tag, ou registra um observer de model que limpa as chaves que
você acredita que aquele model alimenta. Cada uma dessas é um mapeamento que
você mantém à mão, e o modo de falha é silencioso: a página que ninguém
lembrou de esquecer continua sendo servida até o seu TTL acabar.

O Suprnova inverte a direção. A renderização registra o que leu, a escrita
avança o que mudou, e os dois se encontram em um ledger no banco de dados em
vez de na sua cabeça. Não existe chamada de `forget` para esquecer. O preço
é que a dependência registrada é uma tabela em vez de uma linha, então uma
tabela movimentada reconstrói os seus dependentes com frequência, e que
leituras por SQL bruto são recusadas para armazenamento em vez de cacheadas
com uma dependência que ninguém consegue nomear. Ambas as coisas são
visíveis e medidas - o fan-out na carga de tempestade versionada, a recusa
no seu próprio cabeçalho `Age` ausente - em vez de uma página obsoleta
sobre a qual você fica sabendo por um cliente.

## Próximos passos

- [Implantação do RenderCache](render-cache-deployment.md) - perfis,
  provedores e a migração que torna durável a verdade das gerações
- [Representações do RenderCache](render-cache-representations.md) - o que é
  de fato armazenado, e sob qual chave
- [Banco de dados](database.md) - transações e isolamento, dentro dos quais
  renderizações em cache rodam
