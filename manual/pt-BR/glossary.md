# Glossário

Termos específicos do Suprnova, definidos uma vez. Se um capítulo usa
uma palavra sem explicá-la, a definição vive aqui. As entradas são
alfabéticas; siga o link cruzado até o capítulo que usa o termo em
contexto.

Algumas convenções para ter em mente ao ler o restante desta lista:

- **Trait** significa um trait de Rust - um contrato de comportamento
  que você implementa em um tipo. **Facade** significa uma struct de
  tamanho zero cujos métodos estáticos são o ponto de entrada de um
  subsistema (`Cache`, `Mail`, `Auth`, `Storage`, `Bus`, `Notify`,
  `Vector`, `DB`, `Schedule`, `App`).
- **Driver** significa um backend substituível por trás de uma facade
  ou registro - `CacheStore`, `QueueDriver`, `VectorDriver`,
  `RateLimiterDriver`, `MailDriver`. Os drivers são escolhidos na
  inicialização via variáveis de ambiente e vinculados através do
  contêiner.
- **Registro** significa uma tabela de consulta global ao processo,
  preenchida em tempo de compilação via `inventory` ou na
  inicialização via registro explícito - `ConnectionRegistry`,
  `MiddlewareRegistry`, `InertiaRegistry`, `ChannelRegistry`,
  `VectorRegistry`, `SupervisorRegistry`, `PaymentProviderRegistry`,
  `ScopeRegistry`.

## A

### Acessador

Uma transformação do lado da leitura declarada em um model Eloquent
com a macro `#[accessor]`. Executa toda vez que a propriedade é
lida, retornando um valor computado derivado de uma ou mais colunas
subjacentes (`full_name` a partir de `first_name + last_name`, por
exemplo). O dual de um [Mutador](#mutador). Veja
[Eloquent - Acessadores e mutadores](eloquent.md#accessors-and-mutators).

### Ação

Uma classe de serviço injetável que encapsula uma peça de lógica de
negócio - um único método público, com dependências injetadas via a
macro `#[injectable]`. O análogo Suprnova dos invocáveis de ação
única do Laravel. Ações são vinculadas como singletons no contêiner
automaticamente e resolvidas por handlers, jobs e outras ações. Veja
[Ações](actions.md).

### Aplicação

O builder fluente em `Application::new()` que registra suas funções
de config, bootstrap, rotas e migrações, e então chama `.run()` para
despachar o subcomando de CLI do binário (`serve`, `migrate`,
`queue:work`, etc.). Um por binário, vive em `src/app.rs`. Veja
[Ciclo de vida da solicitação](lifecycle.md).

### Contador atômico

Uma operação de cache (`Cache::increment`, `Cache::decrement`) que
muta um valor numérico em um único round-trip, sem races de
read-modify-write. Apoiada em `INCR`/`DECR` do Redis no store Redis,
por uma guarda mantida no store em memória. Veja
[Cache - Contadores atômicos](cache.md#atomic-counters).

### Authenticatable

O trait que um tipo de usuário autenticado implementa
(`get_auth_identifier() -> String`, `get_auth_password()`, etc.)
para que guards e middleware possam falar com ele sem conhecer a
struct de usuário concreta. Veja [Autenticação](authentication.md).

### Authorizable

O trait que dá a um tipo de usuário os pontos de entrada de política
(`can`, `can_any`, `cannot`) usados pelo [Gate](#gate). Veja
[Autorização](authorization.md).

## B

### Padrão de backoff

A sequência de atrasos que um worker de fila espera entre retries de
um job falhando. `BackoffSchedule::linear`, `BackoffSchedule::exponential`,
ou um `Vec<Duration>` customizado. Veja
[Filas - Padrões de backoff](queues.md#backoff-schedules).

### Batch (fila)

Um grupo de jobs despachados juntos e rastreados como uma unidade -
`PendingBatch::new().add(job).add(other).dispatch()` retorna o id
do batch persistido. Útil quando você quer espalhar trabalho e
rodar um callback quando o batch inteiro completa. Veja
[Filas - Batches enfileirados](queues.md#queued-batches).

### `BelongsTo`

A relação inversa de `HasOne`/`HasMany` - o filho guarda a chave
estrangeira, o pai fica do outro lado. Uma das onze relações
Eloquent. Veja [Eloquent - Relacionamentos](eloquent.md#relationships).

### `BelongsToMany`

Uma relação muitos-para-muitos que passa por um terceiro model de
primeira classe: [Pivot](#pivot). `BelongsToMany<Local, Related,
Pivot>` - o pivot é nomeado no tipo, não sintetizado por convenção
de string. Veja [Eloquent - Relacionamentos](eloquent.md#relationships).

### Inicialização

A `bootstrap_fn` que você registra no builder `Application` e que
roda uma vez na inicialização (depois da config, antes de servir).
Onde você vincula serviços no [Contêiner](#contêiner), registra
observers e event listeners, configura headers padrão, e assim por
diante. O análogo Suprnova dos service providers do Laravel,
colapsado em uma única função. Veja
[Inicialização da aplicação](bootstrap.md).

### Broadcastable

O trait que um [Evento](#evento) implementa quando deve ser
empurrado para assinantes WebSocket em vez de (ou além de) listeners
locais em processo. A ponte entre o dispatcher de eventos e o
[Broadcast Hub](#broadcasthub). Veja [Transmissão](broadcasting.md).

### `BroadcastHub`

O trait que nomeia "a coisa que faz fan-out de uma mensagem para
todo assinante WebSocket de um canal" - a implementação em memória
(`InMemoryBroadcastHub`) é o padrão; a implementação sea-streamer
(`SeaStreamerBroadcastHub`) é o deployment multi-processo de
produção. Veja
[Transmissão - Fanout multi-processo](broadcasting.md#multi-process-fanout).

### Construtor (Eloquent)

O objeto de consulta fluente retornado por `Model::query()` - a
superfície encadeável onde você constrói `where`, `order_by`, `with`,
`limit`, etc. antes de `.get()`, `.first()`, ou `.paginate(...)`.
Nomeado em dobro: todo método de filtro existe tanto sob seu nome
Laravel (`db_where`, `db_or_where`) quanto seu sinônimo Rust-nativo
(`filter`, `or_filter`). Veja
[Eloquent - Construtor de consultas](eloquent.md#query-builder--dual-api).

### Comando de barramento

Uma struct serializável despachada através de `Bus::dispatch(cmd)`
que roteia para um único `Handler<C>` registrado. Comandos de
barramento são para trabalho em processo cujo resultado deve
propagar de volta para quem chamou - [Job](#job)s de fila são para
trabalho que deve ser persistido e reexecutado em segundo plano.
Veja [Barramento de comandos](bus.md).

## C

### Driver de cache

O backend selecionado (`memory` ou `redis`) por trás da facade
`Cache`. Escolhido na inicialização via `CACHE_DRIVER` e exposto
através do trait [CacheStore](#cachestore). Veja [Cache](cache.md).

### `CacheStore`

O trait que define a SPI do driver de cache - `get`, `put`, `forget`,
`increment`, etc. `InMemoryCache` e `RedisCache` são as
implementações que já vêm prontas. Veja
[Cache - Configuração](cache.md#configuration).

### Cast (Eloquent)

Uma transformação bidirecional declarada com `casts!` em um model
Eloquent - tipo da coluna do BD ↔ tipo Rust. 22 casts embutidos vêm
prontos (`AsBool`, `AsDateTime`, `AsJson`, `AsEncrypted`, `AsArray`,
etc.); um trait `Cast` implementado pelo usuário cobre qualquer outra
coisa. Veja [Eloquent - Casts](eloquent.md#casts).

### Chain (fila)

Uma sequência de [Job](#job)s encadeados de modo que cada um só roda
se o anterior tiver sucesso. Construída com `PendingChain::dispatch`
/ `Queue::chain`. Veja [Filas - Chains enfileiradas](queues.md#queued-chains).

### Canal (transmissão)

O trait para o qual um evento transmite - `PublicChannel`,
`PrivateChannel`, ou `PresenceChannel`. A struct do canal nomeia a
si mesma (`fn name() -> String`) e autoriza a conexão
(`fn authorize(...)`); canais privados e de presença adicionam
trait bounds mais fortes. Veja
[Transmissão - Canais](broadcasting.md#channels).

### Canal (notificação)

O trait que roteia uma [Notification](#notification) para um
mecanismo de entrega - mail, database, broadcast, web push. Uma
notificação nomeia seus canais em `fn via(...)`; cada canal resolve
o destino e envia. Distinto do trait de transmissão de mesmo nome.
Veja [Notificações - Canais](notifications.md#channels).

### Contêiner

O registro de três camadas (task-local → thread-local → global) onde
serviços são vinculados e resolvidos através da facade `App`. O
análogo Suprnova do service container do Laravel, com camadas extras
para isolamento por solicitação e por teste. Veja
[Contêiner de serviços](container.md).

### Contexto (por solicitação)

O saco de valores tipados por solicitação, alcançável a partir de
qualquer código na mesma task async - `Context::set::<T>(value)`,
`Context::get::<T>()`. Sobrevive a spawns de task quando você o
propaga explicitamente. Distinto do contexto de feature flag que
compartilha o nome. Veja [Contexto](context.md).

### CORS

Cross-Origin Resource Sharing. A regra de segurança do navegador que
condiciona um fetch JavaScript da origem A para a origem B; o
Suprnova traz `CorsMiddleware` para emitir os headers de resposta
que sinalizam quais solicitações cross-origin são permitidas. Veja
[CORS](cors.md).

### CSRF

Cross-Site Request Forgery. O ataque contra o qual uma sessão
stateful precisa se defender; o Suprnova traz `CsrfMiddleware` para
exigir um token correspondente em toda solicitação que muda estado.
Veja [CSRF Protection](csrf.md).

## D

### `DB` facade

O ponto de entrada sem model para o banco de dados - `DB::table(...)`,
`DB::transaction(...)`, `DB::raw(...)`. Para consultas que não se
encaixam na forma Eloquent (colunas dinâmicas, agregados com join,
SQL bruto). Veja
[Eloquent - Facade DB](eloquent.md#db-facade--model-less-queries).

### Disco

Um backend de armazenamento nomeado, registrado através da facade
`Storage` - `Storage::disk("s3")`, `Storage::disk("local")`. Cada
disco implementa [DiskExt](#diskext) e é chaveado pelo seu nome de
registro. Veja [Armazenamento de arquivos](filesystem.md).

### `DiskExt`

O trait que todo backend de armazenamento implementa - `put`, `get`,
`delete`, `list`, `signed_url`, etc. Apoiado em `opendal` por baixo
dos panos; traz adaptadores para fs local, em memória, S3, Azure
Blob e GCS. Veja [Armazenamento de arquivos](filesystem.md).

## E

### Eloquent

A camada inteira de ORM - trait `Model`, `Builder<M>`, relações,
casts, scopes, observers, eventos, soft deletes, prunable,
factories. O nome Laravel para o que outros ecossistemas chamam de
ORM; no Suprnova ele fica em cima do SeaORM (que o usuário não deve
ver). Veja [Eloquent](eloquent.md).

### Envelope (fila)

A struct wrapper (`Envelope { payload, attempts, max_attempts,
delay, ... }`) que um driver de fila de fato serializa e armazena.
Isola o payload do [Job](#job) da plumbing da fila. Veja
[Filas](queues.md).

### Evento

Uma struct clonável despachada através de
`EventDispatcher::dispatch(evt)` e entregue a todo `Listener<E>`
registrado. O Suprnova traz o trait, a facade (`EventFacade`), o
agregador `Subscriber`, e hooks para [Listener enfileirado](#listener-enfileirado)s.
Veja [Eventos](events.md).

### Listener de evento

Veja [Listener](#listener).

## F

### Facade

A convenção de nomenclatura para uma struct de tamanho zero cujo
bloco `impl` guarda a API pública de um subsistema - `Cache`, `Mail`,
`Auth`, `Storage`, `Bus`, `Notify`, `Vector`, `DB`, `Schedule`,
`App`. Herdada do Laravel; no Suprnova a implementação subjacente é
resolvida através do [Contêiner](#contêiner) em vez do magic-call do
PHP. Veja [Contêiner de serviços](container.md).

### Factory (Eloquent)

A macro `#[derive(Factory)]` e o trait `Factory` que produzem linhas
de teste realistas com padrões orientados a `fake` -
`UserFactory::times(5).create_many().await?`. A contraparte Rust das
model factories do Laravel. Veja [Macros - Factories](macros.md#factories).

### Fail-closed

Uma política de falha de driver em que uma queda de backend faz a
solicitação rejeitar com um 5xx - usada por rate limit, sessão e
idempotência quando "melhor recusar do que vazar". O oposto de
[Fail-open](#fail-open). Configurada via
`BackendErrorPolicy::FailClosed`. Veja [Limitação de taxa](rate-limiting.md).

### Fail-open

Uma política de falha de driver em que uma queda de backend deixa a
solicitação passar (com um warning logado) em vez de rejeitá-la -
usada quando disponibilidade importa mais que o limite. Configurada
via `BackendErrorPolicy::FailOpen`. Veja
[Limitação de taxa](rate-limiting.md).

### Sinalizador de recurso

Um booleano (ou valor tipado) chaveado por nome e avaliado contra o
usuário/contexto atual - `feature!(MyFeature)`. Apoiado no trait
`Evaluator`; traz um evaluator de banco de dados e um evaluator
cacheado por TTL em cima. Veja
[Sinalizadores de recursos](feature-flags.md).

### Fillable

A allowlist de tempo de compilação que diz quais colunas do model
podem receber mass-assignment a partir de um hash de attributes não
confiáveis - declarada na struct do model via o attribute
`#[fillable]` ou o trait `Fillable`. O dual de `#[guarded]`. Veja
[Eloquent - Atribuição em massa](eloquent.md#mass-assignment).

### Sistema de arquivos

O subsistema de armazenamento inteiro - a facade `Storage`, [Disco](#disco)s
registrados, o trait [DiskExt](#diskext), cópia de streaming entre
discos. Veja [Armazenamento de arquivos](filesystem.md).

### Solicitação de formulário

Uma struct que implementa `FormRequest` (ou derivada via
`#[request]`) que extrai e valida um corpo de solicitação antes do
handler rodar. O análogo composável e type-safe das classes de
form-request do Laravel. Veja [Validação](validation.md).

### `FrameworkError`

O enum único para o qual toda falha interna do framework converte.
Carrega sua própria projeção `HttpResponse`
(`From<FrameworkError> for HttpResponse`) que sanitiza corpos 5xx e
carimba um request id. Veja [Modelo de erros](error-model.md).

## G

### Gate

O ponto de entrada de autorização - `Gate::allows("update-post", user,
post)`. Resolve contra políticas registradas (declaradas via a macro
`#[policy]`) e faz short-circuit em allow/deny. Retorna uma
`GateResponse` (reexportada como o `Response` de autorização). Veja
[Autorização](authorization.md).

### Scope global

Uma restrição de consulta aplicada a toda chamada `Model::query()`
até ser explicitamente removida (`Builder::without_global_scope`).
Implementada via o trait `GlobalScope` e registrada na
inicialização. Veja [Eloquent - Scopes](eloquent.md#scopes).

### Guard (autenticação)

A estratégia de autenticação nomeada, anexada a uma solicitação -
`session` (stateful, apoiada em cookie), `token` (stateless, bearer
token). Múltiplos guards coexistem; `Auth::guard("api")` escolhe um.
Veja [Autenticação](authentication.md).

### Guarded

A blocklist de tempo de compilação que diz quais colunas do model
*não podem* receber mass-assignment. O dual de [Fillable](#fillable).
Veja [Eloquent - Atribuição em massa](eloquent.md#mass-assignment).

## H

### `HasMany`

Uma relação um-para-muitos - o pai guarda a chave local, os filhos
guardam a chave estrangeira. Uma das onze relações Eloquent. Veja
[Eloquent - Relacionamentos](eloquent.md#relationships).

### `HasManyThrough`

Uma relação que alcança o model relacionado saltando por um terceiro
model intermediário - `Country -> User -> Post`. Veja
[Eloquent - Relacionamentos](eloquent.md#relationships).

### `HasOne`

O irmão de linha única de [HasMany](#hasmany) - o pai guarda a chave
local, o filho tem a chave estrangeira, retorna no máximo uma linha.
Veja [Eloquent - Relacionamentos](eloquent.md#relationships).

### Hash facade

O ponto de entrada de hashing de senha - `hash(password)`,
`verify(password, hash)`. Escolhe bcrypt ou argon2 via
`HASH_DRIVER`; `needs_rehash` deixa você migrar usuários entre
algoritmos no login. Veja [Hashing](hashing.md).

### Handler

A função async que retorna uma `Response` para uma rota casada -
transformada na forma de handler tipado do framework pela macro
`#[handler]`. Composta na borda mais interna da chain de middleware.
Veja [Roteamento](routing.md), [Controladores](controllers.md).

### `HttpError`

O trait que um tipo de erro definido pelo usuário implementa para
especificar como deve se renderizar como uma resposta HTTP - status,
corpo, headers. Espelha as exceptions `Renderable` do Laravel. Veja
[Tratamento de erros](errors.md).

### `HttpResponse`

O tipo concreto de resposta HTTP produzido por handlers e
middleware. Envolve um status code, headers, e um corpo - a coisa
que de fato é escrita na rede. Veja [Respostas](responses.md).

## I

### Chave de idempotência

Um header fornecido pelo cliente (`Idempotency-Key`) que diz "se
você já processou uma solicitação com essa chave, reproduza a mesma
resposta em vez de rodar o handler de novo". Obrigatória para
POST/PUT/PATCH/DELETE seguros para retry; o Suprnova traz
`Idempotency`, `Idempotent`, e `Replay` para envolver handlers. Veja
[Idempotência](idempotency.md).

### Resposta Inertia

Uma resposta que retorna um nome de componente tipado mais props
serializadas em vez de HTML - a ponte entre um handler Rust e uma
página Svelte / React / Vue. Construída com `Inertia::render(...)`
ou a macro `#[derive(InertiaProps)]` mais `inertia_response!`. Veja
[Frontend](frontend.md), [Respostas Inertia](frontend-inertia-responses.md).

### `InertiaProps`

A derive macro que gera o impl `Serialize` mais metadados de tipo
TypeScript para uma struct usada como props de uma página Inertia.
Aciona o comando `suprnova generate-types`. Veja
[Tipos TypeScript](frontend-typescript-types.md).

## J

### Job

Uma struct serializável que implementa o trait `Job` - tem um método
`handle(self)`, enfileirada através de `Queue::push(job)` (ou
`Queue::push_later(job, when)` para um despacho atrasado). Persistida
no armazenamento do driver de fila e executada por um worker. Veja
[Filas](queues.md).

### Middleware de job

Os wrappers composáveis (`WithoutOverlapping`, `RateLimited`,
`ThrottlesExceptions`, `Skip`, `FailOnException`,
`SkipIfBatchCancelled`) que rodam ao redor da chamada `handle` de um
job. O equivalente de fila do middleware HTTP. Veja
[Filas - Middleware de job](queues.md#job-middleware).

### `JobOutcome`

O enum discriminado que a liquidação de um job produz - `Completed`,
`Failed`, `Released`, `Deleted`, `Skipped` - reportado através de
eventos de ciclo de vida do job e do contador de métricas da fila.
Veja [Filas](queues.md).

## L

### Coleção lazy

A contraparte em streaming da [Coleção](#collection-eloquent) -
`Model::query().lazy().await` retorna uma `LazyCollection<M>` que
puxa linhas do banco de dados em chunks em vez de carregar toda
linha na memória. Veja
[Eloquent - Chunking e iteração lazy](eloquent.md#chunking-and-lazy-iteration).

### Paginador length-aware

O paginador clássico de páginas numeradas (`Builder::paginate(per_page)`)
que roda a consulta mais um `COUNT(*)` - conhece a contagem total de
linhas. Veja [Eloquent - Paginação](eloquent.md#pagination).

### Listener

O trait que um handler de evento implementa - `Listener<E>::handle(evt)`.
Registrado com `EventDispatcher::listen::<E, _>(arc_listener)` ou via
o agregador `Subscriber`. Veja [Eventos](events.md).

### Guarda de bloqueio (cache)

O handle retornado por `Cache::lock(key, ttl).acquire()`,
representando exclusão mútua entre processos - `LockGuard`. Liberar
a guarda libera o lock; deixá-la cair no chão depende do TTL. Veja
[Cache](cache.md).

### Política de bloqueio

A política do projeto inteiro para lidar com o poisoning de
`std::sync::Mutex` / `std::sync::RwLock` em um processo de vida
longa - dois padrões sancionados (mapear para erro ou recuperar in
place); nunca `.lock().unwrap()` puro. Veja
[Política de bloqueio](lock-policy.md).

## M

### `Mailable`

O trait que uma mensagem de mail implementa - `subject`, `to`, `cc`,
`bcc`, `view`, anexos. Ou escrita à mão ou derivada via a macro
`#[derive(NotificationMailable)]`; enviada através de
`Mail::to(...).send(MyMail).await`. Veja [Correio](mail.md).

### Modo de manutenção

Uma flip de tempo de solicitação que tira a aplicação do ar para
todo mundo, exceto uma allowlist - `maintenance_mode().set(payload)`.
Apoiado em `FileMaintenanceMode` (padrão, um arquivo sentinela) ou
`CacheMaintenanceMode` (apoiado em cache, para deployments
multi-instância); servido por `MaintenanceMiddleware`. Reexportado
na raiz do crate.

### Middleware

Um wrapper composável ao redor de um handler - vê a solicitação
antes, a resposta depois, e pode fazer short-circuit retornando
`Err(resp)`. Registrado globalmente, por rota, ou por grupo; roda em
uma ordem fixa de fora para dentro. Veja [Middleware](middleware.md).

### Model

Uma struct anotada com `#[suprnova::model]` que nomeia uma tabela do
banco de dados. A struct *é* o `Model` do SeaORM depois que a macro
expande - o Suprnova não a envolve. Carrega CRUD via o trait
`Model`, construção de consulta via `Model::query()`, factories,
casts, scopes, relações, observers. Veja [Eloquent](eloquent.md).

### Morph

Abreviação de "polymorphic". Uma relação morph deixa uma única
relação apontar para um de vários tipos de model - `MorphTo`
(dono único de vários tipos possíveis), `MorphMany`/`MorphOne` (o
inverso, coletando filhos morphed), `MorphToMany`/`MorphedByMany`
(muitos-para-muitos entre tipos morphed). O framework mantém um
[Registro](#registro) em runtime de mapeamentos `MorphTypeEntry`
entre strings discriminadoras e tipos Rust. Veja
[Eloquent - Relacionamentos](eloquent.md#relationships).

### Mutador

Uma transformação do lado da escrita declarada com a macro
`#[mutator]` - executa toda vez que a propriedade é definida, antes
do valor ser armazenado no model. O dual de um [Acessador](#acessador).
Veja [Eloquent - Acessadores e mutadores](eloquent.md#accessors-and-mutators).

## N

### Notifiable

O trait que um usuário (ou qualquer objeto que possa receber
notificações) implementa - `route_for(channel)` retorna o endereço
para o canal nomeado (endereço de mail, push subscription, id de
usuário de broadcast, etc.) ou `None` para pular. Veja
[Notificações - A trait Notifiable](notifications.md#the-notifiable-trait).

### Notification

O trait que uma mensagem de notificação implementa - `channels()`
retorna a lista de nomes de canal para os quais deve fazer fan-out;
cada canal chama de volta para a notificação (via traits por canal
como métodos de payload `MailRendering` / `DatabaseChannel`) para o
payload específico do canal. Despachada através de
`Notify::send(&user, &notif).await`. Veja [Notificações](notifications.md).

## O

### Observer

Uma struct que implementa `Observer<M>` e escuta os eventos de ciclo
de vida de um model Eloquent - `creating`, `created`, `updating`,
`updated`, `deleting`, `deleted`, `saving`, `saved`, `retrieved`,
`replicating`, etc. Registrada via a macro `#[suprnova::observer(M)]`;
drenada do inventory na inicialização. Veja
[Eloquent - Observers e eventos de ciclo de vida](eloquent.md#observers-and-lifecycle-events).

### `OriginPolicy`

A escolha de enforcement do middleware CSRF para o header `Origin`
em solicitações que mudam estado - `Strict` (deve casar com o host),
`AllowList`, ou `None`. Veja [CSRF Protection](csrf.md).

## P

### Paginador

O resultado de uma chamada `.paginate(...)` - um de três sabores.
`LengthAwarePaginator` (páginas numeradas com um `COUNT(*)`),
`Paginator` (próximo/anterior, sem total), `CursorPaginator` (cursor
opaco para iteração estável sobre um result set em movimento). Os
três serializam para um payload JSON na forma Laravel. Veja
[Eloquent - Paginação](eloquent.md#pagination).

### Limite de panic

O wrapper `AssertUnwindSafe(...).catch_unwind()` ao redor da chain
de middleware (e ao redor de cada handler de background-worker) que
converte um panic não tratado em um 500 sanitizado mais um evento
`ErrorOccurred` logado. Uma rede de segurança, não um contrato -
APIs públicas ainda devem retornar `Result`. Veja
[Ciclo de vida da solicitação - Limite de panic](lifecycle.md#5-panic-boundary--execute_chain_safely).

### Provedor de pagamento

Um tipo que implementa o super-trait `PaymentProvider` (= `Checkout` + `Subscription` + `CustomerStore` + `WebhookHandler`).
Adaptadores de referência: `suprnova-payments-stripe` (gateway,
impl `Payment` completo) e `suprnova-payments-paddle`
(merchant-of-record, sem `Payment`). Veja [Pagamentos](payments.md),
[Guia do provedor](payments-provider-guide.md).

### Pivot

O model intermediário em uma relação [BelongsToMany](#belongstomany) -
um `#[suprnova::model]` de primeira classe, com sua própria struct,
casts e timestamps, nomeado explicitamente como o terceiro parâmetro
de tipo (`BelongsToMany<L, R, P>`). O Suprnova não sintetiza um
pivot implícito a partir de um nome de tabela. Veja
[Eloquent - Relacionamentos](eloquent.md#relationships).

### Canal de presença

Uma variante de [Canal](#canal-transmissão) em que o servidor
rastreia quem está atualmente inscrito e emite eventos de
join/leave com os metadados de cada membro. Útil para indicadores
de "quem está online". Veja
[Transmissão - Canais de presença](broadcasting.md#presence-channels).

### Canal privado

Uma variante de [Canal](#canal-transmissão) que exige autorização no
subscribe - `authorize(...)` deve retornar true para o usuário se
inscrevendo. Útil para streams de notificação por usuário. Veja
[Transmissão - Canais](broadcasting.md#channels).

### Prunable

O trait que marca um model soft-deleted (ou consultável) como
elegível para limpeza por `model:prune` -
`Prunable::prunable_query()` retorna o builder para as linhas que
devem ir embora. `MassPrunable` deleta em um único `DELETE WHERE`; o
padrão emite deletes linha a linha para que observers disparem.
Marcada para o registro via a macro `#[prunable]`. Veja
[Eloquent - Prunable](eloquent.md#prunable).

## Q

### Fila

O subsistema de trabalho em segundo plano inteiro - facade `Queue`,
trait [Job](#job), [Envelope](#envelope-fila), drivers (memory,
sync, redis, database, null), worker, batches, chains. Veja
[Filas](queues.md).

### Driver de fila

Um tipo que implementa `QueueDriver` (push, pop, release, etc.) -
traz `MemoryQueueDriver`, `SyncQueueDriver` (roda inline),
`RedisQueueDriver`, `DatabaseQueueDriver`, `NullQueueDriver`.
Escolhido na inicialização via `QUEUE_DRIVER`. Veja
[Filas - Drivers](queues.md#drivers).

### Worker de fila

O loop de vida longa que puxa envelopes do driver de fila, roda
middleware de job ao redor do handler, e reporta o resultado. Sobe
pelo mesmo ciclo de vida que o servidor HTTP, então observers e
listeners disparam identicamente. Iniciado por
`cargo run -- queue:work`. Veja [Filas](queues.md).

### Listener enfileirado

Um `Listener<E>` que, quando invocado, persiste o payload do evento
na fila e roda `handle` em um worker de background em vez de em
processo. Útil quando um listener de evento faz I/O que não deveria
bloquear o caminho de dispatch. Envolvido via o adapter
`QueuedListener`. Veja [Eventos](events.md).

## R

### Limitador de taxa

O subsistema de rate limiting inteiro - `RateLimiter` (a facade
apoiada em cache), builder `Limit`, `SlidingWindowConfig` (driver de
sliding window), `RateLimitMiddleware` (montado na rota),
`ThrottleRequestsMiddleware` (alias com nome Laravel),
`BackendErrorPolicy` (fail-open vs fail-closed). Veja
[Limitação de taxa](rate-limiting.md).

### Redirecionamento

Uma [HttpResponse](#httpresponse) especializada envolvendo um header
`Location` - construída via `Redirect::to(...)`, `Redirect::route(...)`,
`Redirect::back()`, com chains `.with(...)`/`.with_input(...)` para
dados flash. Veja [Geração de URLs](urls.md), [Respostas](responses.md).

### Registro

Uma tabela de consulta global ao processo, preenchida ou em tempo de
compilação pelo `inventory` (`ModelEntry`, `RelationEntry`,
`MorphTypeEntry`, `ObserverEntry`, `PrunerEntry`, `TaskEntry`,
`PaymentProviderEntry`, `CommandEntry`) ou na inicialização por
registro explícito (`ConnectionRegistry`, `MiddlewareRegistry`,
`InertiaRegistry`, `ChannelRegistry`, `VectorRegistry`,
`SupervisorRegistry`). Todos são drenados ou consultados durante a
sequência de inicialização.

### Relação

O trait que toda relação implementa - `BelongsTo`, `HasOne`,
`HasMany`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`,
`MorphTo`, `MorphOne`, `MorphMany`, `MorphToMany`, `MorphedByMany`.
Um model declara suas relações como métodos que retornam uma struct
de relação; o framework guia eager loading, `with(...)`, consultas
de existência de relação, e touches em cascata a partir do trait.
Veja [Eloquent - Relacionamentos](eloquent.md#relationships).

### Solicitação

A struct de solicitação tipada do framework - envolve a solicitação
hyper subjacente e expõe `req.param("id")`, `req.json::<T>()`,
`req.form_data()`, `req.flash()`, etc. Reexportada como
`suprnova::Request`. Veja [Solicitações](requests.md).

### `Response`

O Suprnova vincula `http::Response` a `Result<HttpResponse,
HttpResponse>` - ambos os braços carregam uma `HttpResponse`. Corpos
de handler retornam `Response`, propagam trabalho falível com `?`,
e o runtime colapsa ambos os braços com `result.unwrap_or_else(|e| e)`.
O tipo de decisão de autorização é reexportado como `GateResponse`
para evitar a colisão. Veja [Respostas](responses.md),
[Ciclo de vida da solicitação](lifecycle.md#the-response-contract).

### Recurso

Duas coisas não relacionadas compartilham o nome; ambas vêm prontas.

1. **Recurso JSON:API** - uma struct `#[derive(Resource)]` que
   serializa um model na forma JSON:API com fieldsets esparsos e
   includes. Veja [Recursos JSON:API](eloquent-resources.md).
2. **Roteamento de recurso** - um helper de rota que monta um
   conjunto CRUD `index`/`show`/`store`/`update`/`destroy` contra um
   impl `ResourceController`. Veja [Roteamento](routing.md).

### `routes!` macro

A macro de tempo de compilação que expande uma DSL de roteamento
(`get!("/users", users::index)`, `group!`, `middleware!(Auth)`) em
uma função factory `Router`. A fonte única da verdade de rota para
uma aplicação. Veja [Roteamento](routing.md), [Macros](macros.md).

## S

### Scope (local)

Um fragmento de consulta reutilizável declarado em um model Eloquent
com a macro `#[scopes(Model)]` -
`Post::query().published().recent().get()`. Scopes locais são
desligados por padrão; só rodam quando invocados. A contraparte do
[Scope global](#scope-global). Veja [Eloquent - Scopes](eloquent.md#scopes).

### Seeder

Um tipo que implementa o trait `Seeder` e preenche o banco de dados
com dados iniciais - registrado através de `suprnova db:seed`.
Muitas vezes apoiado em uma [Factory](#factory-eloquent). Veja
[Eloquent](eloquent.md).

### URL assinada

Uma URL cuja query string carrega uma assinatura HMAC
(`?signature=...&expires=...`) provando que foi produzida pela
aplicação e não foi adulterada. Construída via `sign_url(...)` /
`sign_route(...)`; verificada por middleware ou via
`verify_signature(...)`. Veja
[Geração de URLs - URLs assinadas](urls.md#signed-urls).

### Soft deletes

O padrão em que deletar uma linha de model define um timestamp
`deleted_at` em vez de emitir `DELETE`. Opt-in por model via
`soft_deletes = true` no attribute `#[suprnova::model]`;
`Model::query()` filtra automaticamente linhas trashed;
`with_trashed()` e `only_trashed()` voltam a incluí-las. Veja
[Eloquent - Excluindo e soft deletes](eloquent.md#deleting-and-soft-deletes).

### `Storage` facade

O ponto de entrada para o subsistema de sistema de arquivos -
`Storage::disk("s3")`, `Storage::disk("local")` - retornando uma
implementação de [DiskExt](#diskext). Veja
[Armazenamento de arquivos](filesystem.md).

### Subscriber

Um agregador que registra muitos listeners em uma única chamada -
implementa `Subscriber::subscribe(dispatcher)` e é registrado via
`EventDispatcher::subscribe(subscriber)`. Veja [Eventos](events.md).

### Supervisor

O trait que um ator de background de vida longa implementa
(`Supervisor::run`) para viver sob o `SupervisorRegistry`. O
registro captura panics no loop de execução, aplica uma
`RestartPolicy`, e reinicia. O equivalente Rust do padrão de
supervisor `gen_server` do Erlang. Veja
[Supervisores](supervisors.md).

## T

### Tarefa

Uma struct que implementa o trait `Task` - declara uma expressão
cron ou uma frequência de nível mais alto (`daily()`,
`every_minute()`) e roda no scheduler. Descoberta em tempo de
compilação via o inventory `TaskEntry`. Veja
[Agendamento de tarefas](scheduling.md).

### Middleware terminável

Middleware que registra um hook para rodar *depois* que a resposta
foi escrita para o cliente - implementado via o trait `Terminable`,
capturado em um `TerminationSnapshot`, e despachado por
`dispatch_termination`. Útil para logging, flush de métricas,
auditoria pós-voo. Veja
[Middleware - Middleware terminável](middleware.md#terminable-middleware-post-response-hooks).

### Through (relação)

Uma relação que salta por um terceiro model intermediário -
[HasManyThrough](#hasmanythrough) e `HasOneThrough`. Veja
[Eloquent - Relacionamentos](eloquent.md#relationships).

### Tempo limite

O middleware que limita o tempo de relógio de uma única solicitação
e retorna 504 quando o limite é excedido - `TimeoutMiddleware`.
Distinto dos timeouts de worker de fila (`TimeoutExceeded` do lado
da fila) e dos timeouts de cliente HTTP. Veja [Timeout](timeout.md).

### `TypedCommand`

O trait do lado do console - implementado por structs
`#[derive(Command)]` - que dá a um comando de console argumentos
tipados (via `clap`) e um método async `handle(self)`. Registrado no
inventory `CommandEntry` em tempo de compilação. Veja
[Console](console.md).

## U

### `UserId`

O identificador de string opaco retornado por `Auth::id()`. Os caminhos
de guard/provider do framework carregam qualquer chave estável que o
`UserProvider` configurado use; com `EloquentUserProvider<User>`, em
geral é a chave primária stringificada. As facades do Magnetar expõem um
newtype `UserId`, mas vinculam seu valor de volta ao ID canônico de
usuário da aplicação antes de gravar o estado de sessão do framework.
Manter o limite da solicitação no formato de string permite que IDs
numéricos, UUIDs e IDs opacos independentes de provedor usem os mesmos
contratos de middleware e eventos. Veja [Autenticação](authentication.md).

## V

### VAPID

Voluntary Application Server Identification - a especificação IETF
para identificar um remetente de web push. O Suprnova traz
`VapidKey`, `VapidSigner`, `VapidClaims`, e o `WebPushClient` que
assina cada push request. Veja [Web Push](web-push.md).

### `Vector` facade

O ponto de entrada para o subsistema de busca vetorial -
`Vector::driver("qdrant").await?.upsert(...)`. Apoiado em
implementações de `VectorDriver`: em memória, Qdrant, Pinecone (por
trás de feature), MariaDB nativo. Veja [Busca vetorial](vector.md).

### `VectorDriver`

O trait que todo backend vetorial implementa - `upsert`, `search`,
`delete`, `count`. Permite ao framework suportar múltiplos bancos
vetoriais sem forçar um só. Veja [Busca vetorial](vector.md).

## W

### Web push

O protocolo de push notification da plataforma web - payloads
criptografados entregues através do push service do user agent. O
Suprnova traz `WebPushClient` (signer VAPID, parsing de
retry-after, cap de rejeição de 8 KiB) e `WebPushChannel` para
entrega de [Notification](#notification). Veja [Web Push](web-push.md).

### Webhook

Uma solicitação HTTP enviada por um terceiro (provedor de pagamento,
provedor de identidade, …) para dentro da sua aplicação, para
reportar um evento. O Suprnova trata todo webhook como idempotente
por padrão - adaptadores de provider implementam
`WebhookHandler::verify(...)` e armazenam o event id do provider em
uma constraint `UNIQUE` que rejeita replays. Veja
[Pagamentos - Tratamento de webhooks](payments.md#webhook-handling),
[Idempotência](idempotency.md).

### Fluxo de trabalho

Uma peça de trabalho em segundo plano, stateful e de longa duração,
composta de steps tipados - macros `#[workflow]` e
`#[workflow_step]`. O valor de retorno de cada step é persistido,
então um restart de worker no meio do fluxo de trabalho retoma a
partir do último step completado. A resposta do Suprnova para
processos de background com múltiplos passos que não cabem em um
único [Job](#job). Veja [Fluxos de trabalho](workflows.md).

### `WsConfig`

A configuração de WebSocket por rota - caps de tamanho de payload
(padrão 1 MiB texto / 64 KiB binário), tamanho máximo de frame,
intervalo de ping, timeout de ociosidade, política de origem. Usada
por rotas `ws!()`. Veja [WebSockets](websockets.md).

### `WsSocket`

O handle de WebSocket tipado do framework, entregue a um handler
`ws!()`. Dividido em uma metade `Sink` (envio) e uma `Stream`
(recepção) via `WsSocket::split()`; pings/pongs são gerenciados por
uma task de heartbeat com um `AbortHandle`, para que um handler
derrubado sempre encerre de forma limpa. Veja
[WebSockets](websockets.md).

## Próximos passos

- [Mapa de paridade do Laravel](parity.md) - comparação
  feature a feature com o Laravel 13
- [Variáveis de ambiente](env-vars.md) - todo `env!` que o
  framework lê
- [Índice da documentação](documentation.md) - o mapa de capítulos
