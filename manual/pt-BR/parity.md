# Mapa de paridade do Laravel

O mapeamento honesto, feature a feature, entre o Laravel 13.x e o
Suprnova. Use isto quando estiver perguntando "o Suprnova tem X?" e
quiser uma resposta sim/não/onde em uma única linha.

As seções espelham o índice da documentação do Laravel para que um
desenvolvedor Laravel consiga escanear de cima a baixo. Dentro de cada
seção as colunas são sempre as mesmas:

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|

A coluna **Status** usa quatro valores:

| Símbolo | Significado |
|---|---|
| **disponível** | Mesma superfície, mesmo comportamento (muitas vezes os mesmos nomes de método) |
| **divergente** | Mesmo trabalho, forma diferente porque o Rust torna possível uma escolha melhor |
| **ainda não** | Genuinamente planejado, ainda não está no disco |
| **não, por design** | Não vai ser lançado - explicação na coluna Notas |

O capítulo relevante (onde existe) está linkado a partir da coluna
**Notas**.

Este é um mapa vivo. O Suprnova lança toda a superfície do Laravel
13.x nos 30 domínios documentados; as lacunas listadas abaixo são as
lacunas reais e atuais do framework tal como foi lançado.

## Conceitos de arquitetura

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Ciclo de vida da solicitação | Chain `Application` → `Server` → `handle_request` | disponível | [Ciclo de vida](lifecycle.md) |
| Contêiner de serviço | `Container` + facade `App`, três camadas (task / thread / global) | divergente | Task-local para por-solicitação, thread-local para testes - [Contêiner](container.md) |
| Provedores de serviço | Função `bootstrap()` + `#[service]`, `#[policy]`, `#[command]`, macros de observer | divergente | Sem classe de registro - o bootstrap é uma única função; as macros usam `inventory` para registro em tempo de compilação. [Inicialização](bootstrap.md) |
| Facades | `App::get`, `Cache::*`, `Mail::*`, `Auth::*`, `Storage::*`, `Queue::*`, `Bus::*`, `Event::*`, `Notification::*`, `Gate::*`, `Schedule::*`, `DB::*`, `Vector::*` estáticos | disponível | Mesmo formato de chamada; as facades são tipos reais, não aliases |
| Contracts | Traits - `Mailer`, `KeyValueStore`, `Hasher`, `Channel`, `VectorDriver`, `Evaluator`, `PaymentProvider`, etc. | disponível | Todas as costuras públicas vivem em traits; vincule pelo trait, troque implementações livremente |

## Primeiros passos

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Instalação | `cargo install --git …suprnova-cli` e depois `suprnova new <name>` | disponível | [Instalação](installation.md) |
| Configuração | Config tipada via `#[derive(Config)]` + `Config::register` | divergente | Tipada em tempo de compilação em vez de array bags. [Configuração](configuration.md) |
| Desenvolvimento agêntico (IA) | Nenhum SDK de IA de primeira classe no framework | não, por design | Use os crates que você já usaria de qualquer forma (`async-openai`, `anthropic-rs`, `tokenizers`, etc.) sob `App::bind(Arc<dyn YourLlm>)` |
| Estrutura de diretórios | `src/{actions,bootstrap,controllers,middleware,models,routes}` | disponível | Mesma intenção, layout Rust-idiomático. [Estrutura](structure.md) |
| Frontend | Inertia v3 sobre Svelte 5 / React 19 / Vue 3.5 | disponível | [Frontend](frontend.md), [Páginas](frontend-pages.md), [Tipos TS](frontend-typescript-types.md) |
| Kits iniciais | **Nebula** (auth) e **Pulsar** (site de produto completo), mais o scaffold plano do `suprnova new` | disponível | Dois kits estão disponíveis hoje - Nebula é o equivalente do Breeze; Pulsar adiciona docs, blog, comunidade e RBAC. [Kits iniciais](starter-kits.md) |
| Implantação | Binário único; receitas Docker / Railway / DO / Hetzner | divergente | Um artefato, não um runtime PHP + opcache + FPM. [Implantação](deployment.md) |

## O básico

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Definições de rota | macro `routes!` + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | disponível | [Roteamento](routing.md) |
| Parâmetros de rota | params de path `{id}` + `req.param("id")` | disponível | Params opcionais via `{id?}`; constraints via `where!()` |
| Nomes de rota | `.name("posts.show")` na rota + `url("posts.show", &[("id", "42")])` | disponível | [Geração de URL](urls.md) |
| Grupos de rota | macro `group!` com `.prefix()` / `.middleware()` / `.name()` / `.controller()` | disponível | O middleware de grupo é achatado em cada rota no momento do registro |
| Rotas de recurso | `resource!("posts", PostController)` registra as 7 rotas padrão | disponível | `apiResource!`, `only(...)`, `except(...)` são todos suportados |
| URLs assinadas | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | disponível | HMAC-SHA256 com `APP_KEY` |
| Vinculação de model de rota | `#[handler]` extrai `Post` de `{post}` via impl `RouteBinding` | disponível | O derive `AutoRouteBinding` auto-implementa para tipos `#[suprnova::model]` |
| Limitação de taxa | middleware `throttle:60,1` + `RateLimiter::for_signature` | disponível | [Limitação de taxa](rate-limiting.md) |
| Middleware | trait `impl Middleware`; registre globalmente ou por rota | disponível | [Middleware](middleware.md) |
| Grupos de middleware + aliases | `register_middleware_group`, `register_middleware_alias` | disponível | Busca por nome de string nas rotas |
| Proteção CSRF | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | disponível | A política de origem exige POST same-origin. [CSRF](csrf.md) |
| Controladores | `#[handler] pub async fn show(req: Request) -> Response` | disponível | Controladores são módulos de funções livres, não classes. [Controladores](controllers.md) |
| Controladores de ação única | Um handler já é uma única função; agrupe em módulos | disponível | A convenção Rust - sem a cerimônia do `__invoke` |
| Solicitações | struct `Request` com `.input()`, `.param()`, `.query()`, `.header()`, `.cookie()`, `.json()`, `.file()`, etc. | disponível | [Solicitações](requests.md) |
| Solicitações de formulário | `#[derive(Data, Validate, FormRequest)]` | disponível | A validação roda enquanto você extrai |
| Uploads de arquivo | `req.file("avatar")?` retorna `UploadedFile`; multipart em streaming com caps de tamanho + parte | disponível | Spill automático para arquivo temporário acima do limite |
| Respostas | builders `HttpResponse` + `json!()` / `text!()` / `Redirect::to` / `view` | disponível | [Respostas](responses.md) |
| Views (Blade) | Páginas Inertia renderizadas no servidor (Svelte/React/Vue) - sem equivalente a Blade | divergente | Inertia é a camada de view. Use [Páginas](frontend-pages.md) em vez de Blade |
| Empacotamento de assets (Vite) | Vite 8 vem em todo scaffold; `suprnova serve` roda Vite + backend juntos | disponível | Leitura de manifest + HMR já conectados |
| Assets estáticos (`public/`, servidos pelo servidor web no Laravel) | Handler de fallback `StaticFiles::public()` em processo, servindo `public/` na raiz web | disponível | `StaticFiles::from_dir(...)` + `cache_control(...)`; nenhum servidor web separado é necessário |
| Geração de URL | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | disponível | [Geração de URL](urls.md) |
| Sessão | `session()`, `session_mut()`, flash bag via `req.flash()` | disponível | Apoiada em BD via `DatabaseSessionDriver`; apoiada em cookie por padrão. [Sessão](session.md) |
| Validação | `#[derive(Validate)]` + 17 regras embutidas + traits `Rule`/`AsyncRule` | disponível | Regras async (ex.: `Unique`) atingem o BD. [Validação](validation.md) |
| Tratamento de erros | `FrameworkError`, `AppError`, trait `HttpError`, limite de panic em `execute_chain_safely` | disponível | [Tratamento de erros](errors.md), [Modelo de erros](error-model.md) |
| Logs | Subscriber `tracing` com campos estruturados, `LogFormat` (json / pretty / compact) | divergente | Uma linha de log é um documento JSON; `request_id` sempre presente. [Logs](logging.md) |
| Helpers de abort | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | disponível | Mesmo formato da família `abort_if` do Laravel |

## Indo mais fundo

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Console Artisan | Binário `console` por app, construído a partir de `#[command]` + `#[derive(Command)]` | disponível | [Console](console.md). `cargo run --bin console <subcommand>` |
| Tinker (REPL) | Sem REPL | não, por design | Escreva um script avulso `cargo run --bin xxx` ou um `#[suprnova_test]` |
| Transmissão | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | disponível | Fanout via sea-streamer para multi-node. [Transmissão](broadcasting.md) |
| Cache | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | disponível | Operações atômicas + cache com tags + locks de cache (`LockGuard`). [Cache](cache.md) |
| Coleções | `eloquent::Collection<M>` com métodos na forma Laravel | disponível | `Deref<Target = Vec<M>>` então os idiomas existentes de Vec continuam funcionando. [Coleções](eloquent-collections.md) |
| Concorrência | Tokio em todo lugar - `tokio::spawn`, `tokio::join!`, `tokio::select!` | disponível | O framework inteiro é async. A facade `Concurrency::run([...])` do Laravel não está disponível; Tokio é a resposta |
| Contexto | `Context::put` / `Context::get` / `ContextStore` + auto-injeção em fila / mail / eventos | disponível | [Contexto](context.md) |
| Contracts | Todas as costuras públicas são traits | disponível | Veja a linha "Arquitetura / Contracts" acima |
| Eventos | `EventFacade::dispatch(e).await?`, `#[derive(Event)]`, `EventDispatcher`, listeners enfileirados, subscribers | disponível | [Eventos](events.md) |
| Armazenamento de arquivos | `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` sobre OpenDAL | disponível | Mesma superfície `put/get/delete/copy/move/exists/url`. Proteção contra path traversal embutida. [Sistema de arquivos](filesystem.md) |
| Helpers | Os equivalentes estão em seus módulos de origem (sem um `helpers.md` pia-de-cozinha) | divergente | Ex.: helpers de URL vivem em [urls.md](urls.md), helpers de string em `std`/`heck`, helpers de array em `std::collections` - o Rust faz isso com crates, não um namespace global |
| Cliente HTTP | builder `Http::get/post/...` + `Http::fake(...)` para testes | disponível | Grava solicitações automaticamente; `assert_sent` / `assert_not_sent`. [Cliente HTTP](http-client.md) |
| Localização | `Lang::get` / `get_with` / `try_get` / `has` + a macro `__!("key", name: value)` sobre catálogos Fluent `.ftl` em `lang/<locale>/`, detecção via `LocaleMiddleware`, mensagens de validação traduzidas, formatação ICU4X | disponível | O mesmo catálogo é servido ao navegador em `/_suprnova/lang/<locale>.ftl` e tipado por `generate-types`. [Localização](localization.md) |
| Correio | `Mail::to(...).send(MyMail { ... }).await?` + drivers `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` | disponível | Trait `Mailable` + corpos HTML/text renderizados com Tera. [Correio](mail.md) |
| Notificações | `Notify::send(&user, notif).await?` + canais `mail/database/broadcast/webpush` | disponível | Trait `Notifiable` + `Notification` por canal. [Notificações](notifications.md), [Web Push](web-push.md) |
| Desenvolvimento de pacotes | Crates de adaptador em workspace (ex.: `suprnova-payments-stripe`) | disponível | Mesmo formato dos pacotes Laravel: depende do framework, vincula no contêiner, expõe macros se necessário |
| Processos (rodando comandos de shell) | `tokio::process::Command` da stdlib | não, por design | Sem facade - a API do Tokio já é o formato certo |
| Filas | `Queue::push(job).await?` + drivers `sync/memory/database/redis/null`, batches, chains, `JobMiddleware`, `FailedJobStore` | disponível | [Filas](queues.md) |
| Limitação de taxa | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | disponível | Sliding window via `SlidingWindowConfig`. [Limitação de taxa](rate-limiting.md) |
| Busca (Scout) | Nenhum adaptador de busca full-text de primeira parte | ainda não | A busca vetorial está disponível hoje via [Vetor](vector.md); o equivalente Scout de busca por palavra-chave está planejado |
| Strings (helpers) | crate `heck` (conversões de case), `std::str`, `regex` | divergente | Os mesmos crates que o resto do ecossistema Rust usa; sem um global `Str::camel($x)` |
| Agendamento de tarefas | `Schedule::call/command/task` + `#[derive(Task)]` + sintaxe cron + worker `schedule:run` | disponível | [Agendamento](scheduling.md) |
| Chaves de idempotência | `Idempotency::remember(key, ttl, body)` - proteção contra replay estilo Stripe | disponível | Quem chama namespacea a chave com a rota + identidade de usuário / negócio. [Idempotência](idempotency.md) |
| Tempo limite de solicitação | `TimeoutMiddleware` configurável por rota | disponível | Rust-nativo - aborta a future em voo, libera o worker. [Timeout](timeout.md) |
| Sinalizadores de recursos (Pennant) | `Feature` + `Evaluator` + `FeatureMiddleware` + CRUD de admin | disponível | Propagação em sub-segundo via trait `FeatureSync`. [Sinalizadores de recursos](feature-flags.md) |
| Observabilidade (Pulse) | OpenTelemetry via `init_telemetry`, `Metrics`, `tracing` em todo lugar | divergente | OTel é a língua franca da observabilidade em Rust - aponte seu coletor para o binário. [Observabilidade](observability.md) |
| Telescope (dashboard de debug) | Nenhum equivalente ainda | ainda não | Adiado para v2+; o output de tracing + OTel do framework cobre a maioria das necessidades de diagnóstico |
| Pulse (dashboard de performance) | Nenhum equivalente ainda | ainda não | Mesmo caso do Telescope - exponha métricas com sua stack de observabilidade existente até que um dashboard esteja disponível |
| Busca vetorial | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | disponível | Sem gatekeeping de "só Postgres pgvector". [Busca vetorial](vector.md) |

### Exclusivo do Suprnova (sem equivalente no Laravel)

| Suprnova | O que é | Notas / link |
|---|---|---|
| Macro `ws!()` + handlers WebSocket | Rotas WS tipadas que compartilham o router + a stack de middleware | [WebSockets](websockets.md) |
| Eventos enviados pelo servidor | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| Fluxos de trabalho | Trabalho stateful de longa duração com retries, sleep, fronteiras de step | [Fluxos de trabalho](workflows.md) |
| Supervisores | Trait `Supervisor` com auto-restart via panic-catch para tasks tokio de vida longa | [Supervisores](supervisors.md) |
| Web Push (VAPID) | Notificações push de navegador como canal de primeira classe | [Web Push](web-push.md) |
| Split de leitura/escrita multi-conexão | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [Banco de dados](database.md) |
| HTTP/2 + WebSocket no mesmo socket | `hyper.with_upgrades()` em `Server::run` | [Ciclo de vida](lifecycle.md) |
| Conteúdo Markdown + pipeline de docs | `MarkdownRenderer` (comrak sanitizado → syntect → ammonia) + `build_docs(DocsBuildConfig)` → `DocsCatalog` pesquisável de `DocsChapter`s | Extração de heading + `slugify_heading`; move docs / blog em Markdown sem um gerador de site estático separado |

## Segurança

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Autenticação | `Auth::user/check/login/logout/attempt`, trait `Authenticatable`, `Guard` por nome | disponível | [Autenticação](authentication.md) |
| Múltiplos guards | `Guard` registrado por nome (`web`, `api`, …) via `AuthManager` | disponível | `SessionGuard`, `TokenGuard`, impls customizadas |
| Provedores de usuário | `EloquentUserProvider<U>`, `DatabaseUserProvider`, customizado via trait `UserProvider` | disponível | [Fluxos de autenticação](auth-flows.md) |
| Verificação de email | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`; contract `MustVerifyEmail` no model de usuário | disponível | Apoiado em provider (sem torii) - [Fluxos de autenticação](auth-flows.md) |
| Redefinição de senha | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail`; contract `CanResetPassword` no model de usuário | disponível | Apoiado em provider (sem torii) - [Fluxos de autenticação](auth-flows.md) |
| Limitação de força bruta | `BruteForce` + `LoginThrottleMiddleware` | disponível | Contabilização por IP + por usuário |
| Dois fatores (TOTP) | `TwoFactor` + `TwoFactorChallengeMiddleware` + trait `TwoFactorUser` | disponível | Códigos de recuperação + proteção contra replay |
| Lembrar-me | Cookie assinado de vida longa via `SessionGuard` | disponível | `auth::remember` de propriedade do framework: linha de BD + bcrypt + rotação de uso único |
| OAuth (Socialite) | Via o fork vendorizado `torii_integration` (Google / GitHub / Apple etc.) | disponível | [Autenticação](authentication.md) |
| Sanctum (tokens de API) | `TokenGuard` + tokens apoiados em BD via torii | divergente | O modelo de token + middleware bearer estão disponíveis; sem uma superfície de API Sanctum separada |
| Passport (servidor OAuth) | Ainda não | ainda não | Se você precisa de um provedor OAuth, rode um serviço de identidade dedicado (Keycloak, Hydra) atrás do Suprnova |
| Fortify (backend de auth) | Substituído pelo módulo `auth_flows` + tipos `auth_flows::*` | disponível | Mesmo trabalho; sem necessidade de split headless-vs-headed porque o frontend é Inertia |
| Autorização (Policies / Gates) | `Gate::allows/denies` + `#[policy] impl PostPolicy` + trait `Authorizable` + registro via macro | disponível | [Autorização](authorization.md) |
| Papéis e permissões (spatie/laravel-permission) | Trait `HasRoles` + tabelas `roles` / `permissions` / `role_has_permissions` (`CreateRbacTables`) + `RoleMiddleware` / `PermissionMiddleware` (fail-closed) | disponível | Primeira parte, não um pacote de comunidade. Helpers `create_role` / `give_permission_to_role` / `assign_role_to_model`; se apoia em cima de Gate/Policy. [Autorização](authorization.md) |
| Criptografia | `Crypt::encrypt/decrypt` + AAD binding `CryptPurpose` | disponível | AES-256-GCM, rotação de chave via `APP_KEY_PREVIOUS`. [Criptografia](encryption.md) |
| Hashing | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | disponível | Bcrypt por padrão; argon2id disponível. [Hashing](hashing.md) |

## Banco de dados

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | disponível | [Banco de dados](database.md), [Consultas](queries.md) |
| Múltiplas conexões | `DB::on("read")` + `ConnectionRegistry` | disponível | Split de leitura/escrita de primeira classe |
| Transações | `DB::transaction(\|tx\| async move { ... }).await?` | disponível | Savepoints + retry em deadlock |
| Eventos de consulta | `QueryListener` + evento `QueryExecuted` | disponível | `DB::listen(\|q\| { ... })` |
| Expressões raw | `DB::raw("...")`, `DB::select("...", &[...])` | disponível | Binding de parâmetro obrigatório (sem interpolação de string) |
| Postgres / MySQL / SQLite | Todos os três de primeira classe via SeaORM | disponível | Detecção de URL em `database::config::database_type()` |
| MariaDB | De primeira classe como sua própria opção (vetor + JSON + temporal) | divergente | Tratado separadamente por causa das features multi-paradigma que o Laravel só lança como Postgres |
| Redis | Usado pelos drivers (cache/fila/rate-limit) - sem uma facade `Redis::*` separada | divergente | Use o crate `redis` diretamente quando precisar de comandos ad-hoc; cache/fila/rate-limit cobrem 95% do uso típico |
| MongoDB | Nenhum adaptador de primeira parte ainda | ainda não | Use o crate `mongodb` diretamente via `App::bind` |
| Construtor de consultas | `Builder<M>` com `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` / etc. | disponível | [Consultas](queries.md) |
| Paginação | `LengthAwarePaginator`, `Paginator` (simples), `CursorPaginator` | disponível | Os três serializam para JSON na forma Laravel. [Paginação](pagination.md) |
| Migrações | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | disponível | Rode via `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh`. [Migrações](migrations.md), [Migrações da CLI](cli-migrations.md) |
| Seeders | Trait `Seeder` + subcomando `db:seed` | disponível | Factories por model. [Preenchimento de dados](seeding.md) |

## API Eloquent

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | disponível | A struct É o `Model` do SeaORM. [Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | disponível | Tudo async |
| Create / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | disponível | macro `attrs! { name: "...", email: "..." }` para attrs parciais |
| Guards de atribuição em massa | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + scope `unguarded \|\| { ... }` | disponível | `prevent_silently_discarding_attributes()` para modo estrito |
| Soft deletes | `#[model(soft_deletes)]` auto-injeta `deleted_at` + trait `SoftDeletes` | disponível | `with_trashed()`, `only_trashed()`, `restore()`, `force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + worker `model:prune` | disponível | Pinado em cascata às relações |
| Timestamps | Auto `created_at`/`updated_at` se as colunas estiverem presentes | disponível | Desative via `#[model(timestamps = false)]` |
| Tipos de chave primária | i64 por padrão; UUID / ULID via `#[model(unique_id = "uuid")]` ou `unique_id = "ulid"` | disponível | Auto-gera o id na inserção |
| Scopes locais | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | disponível | Method dispatch em `Builder<M>` |
| Scopes globais | `impl GlobalScope for ActiveOnly { ... }` + registro | disponível | Removido via `Builder::without_global_scope` |
| Relacionamentos (11 tipos) | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | disponível | Enum morph por família. [Relacionamentos](eloquent-relationships.md) |
| Eager loading | `User::query().with(&["posts", "posts.comments"]).get()` | disponível | `EagerLoadDispatch` é sealed; só relações geradas por macro podem implementá-lo |
| Prevenção de lazy loading | `prevent_silently_discarding_attributes(true)` | disponível | Mesmo formato do `preventLazyLoading` do Laravel |
| Agregados em relações | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | disponível | Uma subquery por agregado |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | disponível | Engine EXISTS correlacionado |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | disponível | Opera na coleção inteira |
| Clonando um registro | `user.replicate()` / `user.replicate_into::<OtherType>()` | disponível | Dispara o evento `Replicating` |
| Touch nos timestamps do pai | `#[model(touches = ["post"])]` | disponível | `without_touching \|\| { ... }` para pular |
| Observers | `impl Observer<User>` + `#[suprnova::observer(User)]` | disponível | 16 eventos de ciclo de vida |
| 16 eventos de ciclo de vida | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | disponível | Submódulo `events::*` por model. `EventResult::cancel(_)` faz short-circuit com um 400 |
| Mutadores / Acessadores | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | disponível | [Mutadores](eloquent-mutators.md) |
| Casts (22 embutidos) | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | disponível | Implemente `Cast` para customizados |
| Coleções | `Collection<M>` com `pluck`, `filter`, `map`, `each`, `chunk`, `groupBy`, `keyBy`, `sort_by`, `where_`, `first`, `last`, `count`, `is_empty`, `to_array` e amigos do Laravel; `Deref<Target = Vec<M>>` então todos os idiomas de `Vec` continuam funcionando | disponível | [Coleções](eloquent-collections.md) |
| Recursos de API | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + fieldsets + includes | disponível | Formato JSON:API + formato de recurso estilo Laravel, ambos disponíveis. [Recursos de API](eloquent-resources.md) |
| Serialização | `#[model(hidden = [...], visible = [...], appends = [...])]` | disponível | Mesmo controle sobre quais attributes serializam. [Serialização](eloquent-serialization.md) |
| Factories | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?` (ou `UserFactory::times(5).create_many().await?`) | disponível | `Sequence` para ciclar valores. [Factories](eloquent-factories.md) |
| Ciclo de vida: chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | disponível | Iteração com memória limitada sobre tabelas grandes |
| Bloqueio pessimista | `Builder::lock_for_update()`, `shared_lock()` | disponível | Dentro de uma transação |
| Família `whereJsonContains` | Disponível via as column expressions do SeaORM (ciente do driver) | disponível | A grafia exata difere por backend; helpers estão disponíveis para os casos comuns |

## Paginação

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator` (page + total + per_page + last_page) | disponível | `Builder::paginate(n).await?` |
| `Paginator` (simples) | `Paginator` (page + per_page + has_more, sem count) | disponível | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator` (token de cursor opaco + direção) | disponível | `Builder::cursor_paginate(n).await?`; determinístico para infinite scroll |
| Integração Inertia | trait `IntoInertiaScroll` + `ScrollMetadata` | disponível | Conecta direto no `WhenVisible` / `merge` do Inertia |

## IA (o Laravel lança nativo hoje; nós não fazemos gatekeeping)

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| SDK de IA | Nenhum SDK de IA de primeira parte | não, por design | Traga o crate que você já usa (`async-openai`, `anthropic-sdk`, `ollama-rs`, `tokenizers`, etc.) e vincule sob `App` |
| MCP (Model Context Protocol) | Nenhum adaptador de servidor MCP de primeira parte | não, por design | Os crates MCP do Rust (`mcp-rs`, `mcp-sdk-rust`) se encaixam de forma limpa sob a superfície existente de roteamento / supervisor |
| Boost (agente de código do Laravel) | n/a | não, por design | Fora do escopo do framework |

## Testes

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| `php artisan test` | `cargo test` | disponível | [Testes](testing.md) |
| Estilo Pest / PHPUnit | `#[suprnova_test]` (ciente de async) + assertions `expect!()` estilo Jest + macros BDD `describe!()` / `test!()` | disponível | Os três funcionam de forma intercambiável |
| Testes de feature (HTTP) | Dirige `handle_request(router, registry, req)` em processo - nenhum socket é aberto | disponível | [Testes HTTP](http-tests.md) |
| Testes de console | Rode `dispatch_argv(["console", "..."])` e faça assert | disponível | Mesmo formato dos testes HTTP para o binário console |
| Testes de navegador (Dusk) | n/a no framework - use Playwright / WebdriverIO / o agente de navegador `gstack` | não, por design | Ferramental cross-language já existe; nós não reinventamos isso |
| Testes de banco de dados | `TestDatabase::fresh::<Migrator>()` + rollback por teste | disponível | [Testes de banco de dados](database-testing.md) |
| Mocking e fakes | Fakes por facade: `MailFake`, `NotifyFakeGuard`, `EventFakeGuard`, `Queue::fake`, `Bus::fake`, `Http::fake`, `Storage::fake` | disponível | Chamadas gravadas + helpers de assertion. [Mocking](mocking.md) |
| Viagem no tempo | `tokio::time::{pause, advance, resume}` do runtime da stdlib | disponível | Não lançamos o nosso próprio - a API do Tokio já faz isso |
| Isolamento de contêiner | `TestContainer::fake(\|tc\| tc.bind(...))` - thread-local | divergente | Seguro para paralelismo por construção. [Contêiner](container.md) |

## Pagamentos (Cashier do Laravel; o nosso é genérico por provedor)

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Cashier (Stripe) | Crate adaptador `suprnova-payments-stripe` atrás de traits genéricas `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` | divergente | Superfície genérica, adaptador concreto. [Pagamentos](payments.md), [Adaptador Stripe](payments-stripe.md) |
| Cashier (Paddle) | Adaptador `suprnova-payments-paddle` | divergente | Fluxo merchant-of-record + nenhum impl `Payment` direto (o Paddle é dono do gateway). [Adaptador Paddle](payments-paddle.md) |
| Provedor customizado | Implemente `PaymentProvider` + `SessionPayload` + `WebhookHandler` | disponível | [Guia do provedor](payments-provider-guide.md) |
| Componentes de checkout Inertia | Loops de dispatch documentados para Svelte / React / Vue contra `SessionPayload.flow` | disponível | [Pagamentos - integração Frontend](payments-frontend.md). Páginas de cobrança prontas são uma adição planejada aos kits iniciais ([Kits iniciais](starter-kits.md)) |
| Ciclos de vida de assinatura | `Subscription::subscribe / update / cancel / get` (onde o provedor suporta) | disponível | `NotSupported` é retornado onde o provedor não suporta (ex.: `subscribe` do Paddle e substituição de conjunto de preços) |
| Idempotência de webhook | Tabela espelho `payments_webhook_events` com `UNIQUE(provider, provider_event_id)` | disponível | Proteção contra replay estilo Stripe |
| Tabelas espelho | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | disponível | Coluna JSONB `provider_metadata` em cada uma para campos específicos do adaptador |

## Frontend (Laravel tem Blade + kits iniciais; nós temos Inertia)

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Blade | n/a - Inertia é a camada de view | divergente | [Frontend](frontend.md) |
| Inertia.js | De primeira classe: v3 sobre Svelte 5 / React 19 / Vue 3.5 | disponível | [Respostas Inertia](frontend-inertia-responses.md), [Páginas](frontend-pages.md) |
| Recargas parciais | `#[derive(Data)]` + `req.includes("subset")` + protocolo de recarga parcial do Inertia | disponível | Conjuntos de include type-safe |
| Props adiadas | `Prop::deferred(...)` + `DeferConfig` | disponível | Protocolo de props adiadas do Inertia v3 |
| Merge de props | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | disponível | Protocolo de merge do Inertia v3 |
| Histórico criptografado | `EncryptHistoryMiddleware` | disponível | Histórico criptografado em repouso no cliente |
| Posição de scroll | `ScrollConfig` + `ScrollMetadata` | disponível | Auto-restauração na navegação |
| Tipos TypeScript | `suprnova generate-types` lê `#[derive(InertiaProps)]` e emite `.d.ts` | disponível | [Tipos TypeScript](frontend-typescript-types.md) |
| Leitura de manifest do Vite | Conectado automaticamente via `Inertia::root_view` | disponível | HMR em dev, assets com hash em prod |

## CLI

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| `php artisan` | Binário `console` por app, construído a partir de macros `#[command]` | disponível | [Console](console.md), [Visão geral da CLI](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | disponível | [Geradores](cli-generators.md) |
| `serve` | `suprnova serve` (backend + servidor de dev Vite juntos) | disponível | [Serve](cli-serve.md) |
| Família `migrate` | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | disponível | [Migrações da CLI](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed` (via console por app) | disponível | Seeders registrados via trait `Seeder` |
| `schedule:run` / `schedule:work` / `schedule:list` | Mesmos nomes via binário console por app | disponível | [Agendamento CLI](cli-scheduling.md) |
| `queue:work` | Mesmo nome via binário console por app | disponível | Shutdown gracioso em SIGTERM/SIGINT |
| `tinker` | Sem REPL | não, por design | Veja a linha em "Indo mais fundo" |

## Implantação

| Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | divergente | Um binário, sem etapa de opcache |
| `php artisan config:cache` | A config tipada já é verificada em tempo de compilação | divergente | Nenhum cache de runtime para invalidar |
| `php artisan route:cache` | As rotas são expandidas por macro em tempo de compilação | divergente | O router é construído no boot a partir de rotas já tipadas |
| Envoy (deploys via SSH) | Use qualquer orquestrador - Docker, systemd, Kubernetes, fly.io, Railway | não, por design | O binário é o artefato de deploy |
| Forge / Vapor | Não é algo que lançamos - mas as receitas para Railway, DO e Hetzner cobrem o mesmo trabalho | divergente | [Implantação](deployment.md), [Railway](deployment-railway.md), [Digital Ocean](deployment-digital-ocean.md), [Hetzner](deployment-hetzner.md) |
| Horizon (dashboard de fila) | Nenhum dashboard ainda | ainda não | Inspeção de jobs falhados via `cargo run --bin console queue:failed` até lá |

## Pacotes (pacotes oficiais do Laravel - os nossos ou vêm no core, ou vêm como adaptadores, ou são lacunas deliberadas)

| Pacote Laravel | Suprnova | Status | Notas / link |
|---|---|---|---|
| Cashier (Stripe) | `suprnova-payments-stripe` | disponível | Genérico + adaptador. [Pagamentos](payments.md) |
| Cashier (Paddle) | `suprnova-payments-paddle` | disponível | Fluxo MoR. [Pagamentos](payments.md) |
| Dusk | n/a | não, por design | Ferramental de navegador cross-language já existe (Playwright, etc.) |
| Envoy | n/a | não, por design | Containers / systemd / orquestradores fazem o trabalho |
| Fortify | Substituído por `auth_flows` | disponível | Mesmo trabalho, integrado. [Fluxos de autenticação](auth-flows.md) |
| Folio | n/a - roteamento baseado em página não é Rust idiomático | não, por design | Use `routes!` para roteamento explícito |
| Homestead | n/a - use Docker / DevContainers | não, por design | [Receita Docker](cli-docker.md) |
| Horizon | n/a ainda | ainda não | Jobs falhados aparecem através do console por app |
| Mix | Substituído por Vite | divergente | Vite está disponível em todo scaffold |
| Octane | n/a - já somos Tokio de vida longa | não, por design | Binário único, sempre quente, sem FPM para trocar |
| Passport | n/a ainda | ainda não | Rode um IdP dedicado atrás do Suprnova até que esteja disponível |
| Pennant (sinalizadores de recursos) | Reimplementado como `features::*` | disponível | [Sinalizadores de recursos](feature-flags.md) |
| Pint (code style do PHP) | `cargo fmt` + `cargo clippy` | divergente | Toolchain padrão do Rust |
| Precognition | Solicitações precognitivas do Inertia via recargas parciais + os mesmos tipos `#[derive(Data, Validate, FormRequest)]` | disponível | As duas metades do Precog (validação antecipada + recarga leve) saem de graça do Inertia v3 + form requests |
| Prompts (UI de CLI) | Use o crate `dialoguer` / `inquire` quando necessário | não, por design | O ecossistema Rust já cobre isso |
| Pulse | n/a ainda | ainda não | OTel hoje, dashboard depois |
| Reverb (servidor WebSocket) | Embutido no Suprnova (`ws!()` + `BroadcastHub`) | divergente | Nenhum servidor separado é necessário - é o mesmo processo |
| Sail (dev com Docker) | `suprnova-cli` lança receitas Docker embutidas | disponível | [CLI Docker](cli-docker.md) |
| Sanctum | `TokenGuard` + middleware bearer | divergente | O modelo de token está disponível; sem superfície de pacote separada |
| Scout (busca full-text) | n/a ainda | ainda não | A busca vetorial está disponível ([Vetor](vector.md)); o equivalente Scout de busca por palavra-chave vem depois |
| Socialite | Via o fork torii vendorizado | disponível | [Autenticação](authentication.md) |
| Telescope | n/a ainda | ainda não | Tracing + OTel cobrem a lacuna de diagnóstico até que um dashboard esteja disponível |
| Valet | n/a - apps Rust rodam diretamente | não, por design | `suprnova serve` é o runner de dev |

## Macros (superfície específica do Rust; os análogos Laravel mais próximos, para contexto)

O Suprnova lança um conjunto amplo de proc-macros que não têm um
análogo no Laravel porque o Laravel não tem macros - ele tem
reflection em runtime. Incluindo-as aqui para que você não as perca.

| Macro | Ideia Laravel mais próxima | O que faz |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | Gera a entity SeaORM + implementa o trait `Model` |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | Registra um impl `Observer<M>` via `inventory` |
| `#[scopes(M)]` | Scopes locais em um model | Adiciona métodos a `Builder<M>` |
| `#[accessor]` / `#[mutator]` | Acessadores / mutadores Eloquent | Hooks de get/set no nível do campo |
| `#[handler]` | `__invoke` de controller | Auto-extrai params tipados de `Request` |
| `#[command]` / `#[derive(Command)]` | Classe de comando Artisan | Registra um subcomando de console |
| `#[policy]` | Classe de policy | Registra um impl `Policy` via `inventory` |
| `#[service(T)]` | `register` de service provider | Vincula `T` no contêiner |
| `#[injectable]` | Injeção via construtor | Gera um construtor apoiado em `App::make` |
| `#[derive(InertiaProps)]` | Props Inertia | Codegen TypeScript + serialização Inertia |
| `#[derive(Data)]` | DTO de request | Extraível de `Request` com suporte a conjunto de include |
| `#[derive(FormRequest)]` | Classe `FormRequest` | Validação + gate de auth + transformação |
| `#[derive(Factory)]` | Model factory | Geração de dados de teste apoiada em Faker |
| `#[derive(Resource)]` | Recurso de API | Serialização JSON:API + formato Laravel |
| `#[workflow]` / `#[workflow_step]` | n/a no Laravel | Trabalho stateful de longa duração |
| `routes!` + `get!` / `post!` / `ws!` etc. | `Route::get` / `Route::post` | Registro de rota em tempo de compilação |
| `casts!` | `protected $casts = [...]` | Declaração de cast por model |
| `attrs!` | Array de mass-assignment | Builder de attributes parcial |
| `json_response!` / `text_response!` | `response()->json(...)` | `Ok(HttpResponse::...)` rápido |

Veja [Macros](macros.md) para a referência completa.

## Funções helper (helpers globais do Laravel; os nossos são tipados)

O Laravel lança centenas de globais pequenos (`str_replace_first`,
`array_flatten`, `now()`, `tap()`, `optional()` …). A maioria deles
tem um equivalente Rust direto em `std` ou em um pequeno crate
padrão, então o Suprnova não os reintroduz como um único namespace.
Os que *são* úteis de ter como alias são lançados sob seu módulo de
origem.

| Helper Laravel | Equivalente Suprnova / Rust | Onde |
|---|---|---|
| `auth()` | `Auth::user().await?` | [Autenticação](authentication.md) |
| `cache()` | `Cache::get/put/...` | [Cache](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [Configuração](configuration.md) |
| `csrf_token()` | `csrf_token()` (mesmo nome) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()` (dump-and-die de consulta Eloquent) / `dbg!()` da stdlib | `Builder::dump()` / `Builder::dd()` existem para inspeção de consulta; use `dbg!()` para valores gerais |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [Configuração](configuration.md), [Variáveis de ambiente](env-vars.md) |
| `now()` | `chrono::Utc::now()` (reexportado como `suprnova::chrono`) | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | O Rust lida com isso diretamente com `Option<T>` |
| `redirect('/')` | `redirect("/")` (mesmo nome) | [Roteamento](routing.md) |
| `request()` | `Request` é passada para o seu handler | [Solicitações](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [Respostas](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [Geração de URL](urls.md) |
| `session('key')` | `session().get("key")` | [Sessão](session.md) |
| `str()` / `Str::camel($x)` | métodos do crate `heck` (`ToUpperCamelCase`, etc.) | - |
| `tap($x, fn) → $x` | `tap` do crate `tap`, ou `dbg!` para inspeção rápida | Use o crate `tap` idiomaticamente |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | Só chame a closure: `x()` | n/a - closures do Rust não precisam de helper |
| `view('home', $data)` | Resposta Inertia: `Inertia::render("Home", data)` | [Respostas Inertia](frontend-inertia-responses.md) |

## O que genuinamente ainda não temos

Uma lista consolidada de todo **ainda não** acima, para que você veja
o formato da lacuna em um só lugar:

| Área | O que falta | Alternativa até ser lançado |
|---|---|---|
| Busca (Scout - palavra-chave) | Adaptador Algolia / Meilisearch / Elastic | Construa o seu próprio com `meilisearch-sdk` / `elasticsearch` até ser lançado; [Vetor](vector.md) já cobre busca semântica hoje |
| Passport (servidor OAuth) | Provedor de identidade OAuth de primeira parte | Rode Hydra / Keycloak atrás do Suprnova |
| Telescope (dashboard de debug) | UI web para solicitações / consultas / eventos / hits de cache | Use o output de OTel + tracing ([Observabilidade](observability.md)) |
| Pulse (dashboard de performance) | UI web para consultas lentas / erros / rotas quentes | Mesmo caso: superfície OTel hoje, dashboard depois |
| Horizon (dashboard de fila) | UI web para profundidade de fila / jobs falhados / throughput | `cargo run --bin console queue:failed` e métricas OTel |

## O que não vamos lançar (e por quê)

| Feature do Laravel | Por que o Suprnova não tem |
|---|---|
| Tinker (REPL) | O Rust não tem uma história produtiva de REPL para binários compilados. Um `#[suprnova_test]` curto ou um script avulso `cargo run --bin <thing>` faz o trabalho |
| Templates Blade | Inertia é a camada de view; não lançamos um engine de template renderizado no servidor em paralelo |
| `helpers.md` pia-de-cozinha | O Rust lança `std` + crates pequenos e focados (`heck`, `chrono`, `regex`); não reintroduzimos um único namespace global |
| Mix | Vite cobre isso e está disponível em todo scaffold |
| Octane | O Suprnova já é Tokio de vida longa; não há modo FPM para otimizar |
| Dusk (testes de navegador) | Ferramental cross-language (Playwright, WebdriverIO, o agente de navegador `gstack`) já resolve isso |
| Sail (dev com Docker) | Receitas Docker estão disponíveis embutidas ([CLI Docker](cli-docker.md)); nenhum pacote separado é necessário |
| Valet | `suprnova serve` é o servidor de dev |
| Envoy (deploys via SSH) | Containers / systemd / orquestradores fazem o trabalho; não precisamos de uma DSL de SSH sob medida |
| Facade Concurrency (`Concurrency::run`) | Tokio (`tokio::join!` / `tokio::spawn` / `tokio::select!`) é a resposta; nenhuma facade é necessária |
| Facade Processes | `tokio::process::Command` já é o formato certo |
| SDK de IA / MCP / Boost de primeira parte | Escolha os crates Rust que você já usa; não fazemos gatekeeping |
| Facade Redis dedicada | Cache/fila/rate-limit cobrem 95% do uso típico; use o crate `redis` quando precisar de comandos ad-hoc |
| Facade Strings | `heck`, `regex`, `std::str` cobrem isso; nenhum global `Str::camel($x)` |
| Biblioteca de UI de prompts (CLI) | `dialoguer` / `inquire` já existem; não reinventamos |
| Arquivos de tradução estilo PHP/JSON do Laravel | A localização está disponível, mas o formato do catálogo é Fluent `.ftl` - um único formato que o servidor e o navegador entendem. `trans_choice` também não tem equivalente: o Fluent seleciona categorias de plural CLDR dentro da mensagem. [Localização](localization.md) |

## Como esta lista se mantém honesta

Toda linha na coluna **disponível** é verificável por:

1. Grepar `framework/src/lib.rs` em busca do export nomeado
2. Rodar a suíte de testes do framework (`cargo test --workspace`)
3. Ler o capítulo linkado

Toda linha na coluna **ainda não** é trabalho pretendido, não uma
recusa. Toda linha na coluna **não, por design** tem uma razão de uma
frase na coluna Notas; essas razões são os princípios de design em
[Introdução](introduction.md) aplicados a uma feature específica.

Se você encontrar uma feature do Laravel que você usa e que não está
neste mapa, abra uma issue - ou ela tem uma resposta Suprnova que só
está faltando uma linha, ou é uma lacuna real e queremos saber.

## Próximos passos

- [Vindo do Laravel](from-laravel.md) - o mesmo mapa, narrado lado a
  lado
- [Introdução](introduction.md) - os princípios de design que este
  trabalho de paridade segue
- [`documentation.md`](documentation.md) - o TOC mestre por todo
  capítulo
