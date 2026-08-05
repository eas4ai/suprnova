# Sinalizadores de recursos

O sistema de sinalizadores de recursos do Suprnova combina
declarações `Feature` em tempo de compilação com
sobrescritas em runtime persistidas em uma tabela
`features`. O valor de uma flag no momento da avaliação é
determinado por, em ordem:

1. Uma linha com escopo na tabela `features` - `user:42` ou
   `team:staff`.
2. A linha global na tabela `features` (escopo `""`).
3. O `default` em tempo de compilação embutido na
   declaração `Feature`.

Toggles via o CRUD de admin se propagam para avaliadores
vivos antes que a chamada de mutação retorne. Flags de
kill-switch de fato desabilitam em tempo real, não "dentro
da próxima janela de TTL".

## Início rápido

```rust
// app/src/features.rs - toda flag que seu app referencia vive aqui.
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - conecte a cadeia uma vez durante o boot.
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ... DB::init, sessão, etc.

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// qualquer handler - Feature::is_enabled() resolve contra o contexto por solicitação.
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// alterne a flag a partir de uma rota de admin ou da CLI:
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - None para mudanças iniciadas pelo sistema
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ audit: quem fez toggle nisso
//                                  |   └ enabled
//                                  └ scope_key: "" = global, "user:42" = sobrescrita com escopo
```

A próxima chamada de `NEW_CHECKOUT_FLOW.is_enabled()` observa
`true` - incluindo qualquer entrada de avaliador em cache,
que foi invalidada de forma síncrona dentro de
`admin::upsert`.

## As peças

### `Feature<'a>`

A declaração em tempo de compilação. Carrega o nome da flag
e um valor padrão para quando ela estiver ausente.

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ default: true (pagamentos habilitados até serem desabilitados)
```

Centralizar toda declaração em `app/src/features.rs` te dá:

- um único lugar para fazer grep quando um operador pergunta
  "quais flags existem?"
- unicidade em tempo de compilação para o nome da flag - um
  erro de digitação no call site não compila
- o lugar óbvio para colocar um doc comment explicando o que
  a flag controla

Chame `flag.is_enabled()` para ler contra o contexto ambiente
(montado por [`FeatureMiddleware`](#featuremiddleware)) ou
`flag.is_enabled_in(Some(&ctx))` para passar um
[`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)
específico.

As macros `feature!` e `is_enabled!` também são reexportadas
de `suprnova::*` para call sites que não querem importar a
constante:

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

Lê a tabela `features` para um snapshot em memória no boot e
em todo [`reload()`](#controle-de-fluxo-propagação-de-flag).
O hot path (`is_enabled`) é totalmente síncrono - nenhuma
consulta de BD por solicitação, nenhum `block_on` dentro do
avaliador.

Ordem de resolução no lookup, do mais específico primeiro:

1. `user:{id}` - quando o contexto da solicitação carrega um
   `UserIdField`.
2. `team:{name}` - quando o contexto carrega um `TeamField`.
3. `""` - a flag global.
4. `None` - a linha não existe, o default em tempo de
   compilação assume o controle.

### `CachedEvaluator`

Faz memoization de lookups `(feature, user, team)` atrás de
um `DashMap` com um TTL que você escolhe. O hot path
permanece sync; entradas são descartadas de forma síncrona
quando [`admin::upsert`](#admin-crud) escreve uma flag.

Um TTL de zero degenera para "sem cache" - toda chamada passa
direto para o avaliador interno. Útil para apps com poucas
flags que querem a estrutura de propagação sem o cache.

### `FeatureMiddleware`

Abre um contexto de featureflag por solicitação, populado por
extractors definidos pelo usuário. Padrões:

- `user_id` - de `Auth::id()`.
- `team` - nenhum.

Sobrescreva qualquer um via o builder:

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // Customizado: extrai de um header em vez da sessão.
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// ou: .with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### Admin CRUD

`suprnova::features::admin` é a camada de persistência para a
tabela `features`. Use-o a partir de handlers de admin,
ferramentas de CLI, scripts de implantação - em qualquer
lugar onde uma flag precise ser alternada:

```rust
use suprnova::features::admin;

// Cria ou atualiza uma flag global.
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// argumentos: name, scope_key, enabled, description, actor_id

// Sobrescrita com escopo de usuário (vence a global).
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// Remove uma linha por completo - a flag recai para o default em tempo de compilação.
admin::delete("kill-switch.payments", "", actor_id).await?;

// Lê para uma tabela de UI de admin.
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

Toda mutação dispara o [evento](#eventos) correspondente e
chama
[`features::sync::notify`](#controle-de-fluxo-propagação-de-flag)
para que qualquer avaliador vivo vinculado no contêiner App
seja atualizado antes que a chamada retorne.

`actor_id: Option<String>` é o ponteiro de auditoria. Passe o
user id do operador (o mesmo que sua camada de auth emite);
deixe `None` para mudanças iniciadas pelo sistema (CLI,
migração de deploy, etc.).

## Controle de fluxo: propagação de flag

A trait que faz o toggle do admin aparecer imediatamente:

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

Implementadores reagem a mutações:

- `DatabaseEvaluator::on_flag_changed` chama `self.reload()` -
  puxa o snapshot completo.
- `CachedEvaluator::on_flag_changed` chama
  `self.invalidate(feature)` - descarta toda entrada em cache
  para aquele nome.

A cadeia canônica é uma `CompositeFeatureSync`, que **ordena
fontes de dados antes de caches** - caches precisam invalidar
*depois* que a fonte de dados atualiza, ou um leitor
concorrente pode encontrar o cache vazio, passar direto para
a fonte de dados stale, e repopular o cache com o valor
antigo.

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // fontes de dados primeiro
    vec![cached.clone() as Arc<dyn FeatureSync>],   // caches depois
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` resolve
`Arc<dyn FeatureSync>` a partir do contêiner e aguarda
`on_flag_changed`. No-op quando nenhum sync está vinculado -
o comportamento certo para ferramentas de admin fora do
processo que só escrevem no BD e não têm avaliador vivo para
atualizar.

## Helper de inicialização

`bootstrap_database_cached(ttl)` conecta tudo em uma única
chamada:

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// Opcional: guarde features.database para agendar
// recarregamentos periódicos ou expor views de diff de
// admin. A maioria dos apps descarta o handle e deixa o
// refresh guiado por notify fazer o trabalho.
```

O que ele faz:

1. Ela constrói o `DatabaseEvaluator` contra a conexão de BD
   primária.
2. Envolve-o em um `CachedEvaluator` com o TTL solicitado.
3. Chama `install_evaluator(cached)` - define o default
   global do featureflag *e* vira um tracker "installed" de
   propriedade do framework, para que o middleware não logue
   o warning de "no evaluator".
4. Constrói um `CompositeFeatureSync` com a ordem de slots
   correta e o vincula no contêiner App.

Ela retorna `BootstrappedFeatures { database, cached }` para
chamadores que querem handles diretos para qualquer uma das
camadas.

Se sua topologia não é `Cached(Database)` - um cache apoiado
em Redis, uma fonte de sync remota, uma cadeia multi-tier -
conecte a cadeia manualmente usando as mesmas primitivas.
`bootstrap_database_cached` é conveniência, não um contrato.

## Migrações

O framework é dono do schema da tabela `features`:

```rust
// app/src/migrations/mod.rs
vec![
    // ... as migrações do seu app ...
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

Schema:

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` carrega o tipo de escopo inline (`"user:42"`,
`"team:staff"`, `""` para global), então o read path
permanece um único lookup de string contra um índice único.

## IDs de usuário e de equipe

`UserIdField` e `TeamField` são extensões tipadas guardadas
em `Context::extensions` do featureflag. Ambas são tipadas
como string, então os ids de usuário opacos (UUID / ULID) do
torii e colunas numéricas `users.id` coexistem atrás da mesma
forma.

Construindo um contexto manualmente (fora do middleware):

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // ids de usuário em string - UUIDs, ULIDs, qualquer coisa opaca.
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// ids numéricos ainda funcionam - o framework converte i64 → String no momento de on_new_context.
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## Eventos

Dois eventos disparam a partir do caminho de CRUD de admin:

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

Escute por eles via o dispatcher de eventos do framework,
para alimentar um log de auditoria, alerta do Slack, ou
qualquer pipeline downstream que você precise:

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` não dispara um evento de read path.** Toda
solicitação que verifica uma flag multiplicaria o volume de
eventos pelo número de flags verificadas - bom para uma
história de auditoria-de-mutações, proibitivo para tracing de
read path. Se sua implantação precisa de auditoria de read
path amostrada, adicione uma camada de avaliador customizado
que grava em um canal de log limitado (um stream do Redis ou
uma fila fan-out, dependendo da escala).

## Detecção de avaliador ausente

Se `FeatureMiddleware` está instalado mas nenhum avaliador
foi registrado via `install_evaluator` /
`bootstrap_database_cached`, toda flag retorna
silenciosamente seu default em tempo de compilação - uma má
configuração séria para pegar em QA. O middleware emite
exatamente um `tracing::warn!` por processo, na primeira
solicitação que observa esse estado:

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

A troca usa um `AtomicBool::swap`, então uma tempestade de
solicitações concorrentes no boot serializa para uma única
emissão de warning, não uma por worker.

## Testes

Dois padrões, dependendo do que você está verificando.

### Teste unitário de uma Feature isolada

Use `featureflag::evaluator::with_default` para dar escopo a
um avaliador substituto dentro de uma closure sync:

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` é um helper somente para
teste que inicializa seu próprio SQLite + roda
`CreateFeaturesTable` para que o teste permaneça hermético.
Não o use em caminhos de produção.

### Teste de integração da propagação de ponta a ponta

Use `TestDatabase::fresh::<TestMigrator>()` para o BD e
`TestContainer::bind` (NÃO `App::bind`) para o FeatureSync -
caso contrário, testes paralelos no mesmo processo
sobrescreveriam a vinculação um do outro via o contêiner
global:

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // propaga instantaneamente
}
```

Veja `framework/tests/features.rs` para o conjunto completo
de testes de composição.

### Por que Suprnova diverge

O Laravel Pennant resolve toda flag contra o banco de dados
sob demanda (com memoization opcional em nível de driver por
solicitação). O modelo de um-processo-por-solicitação do PHP
faz com que um hit de BD por solicitação seja barato, porque
a conexão é dedicada e morre com a solicitação.

O modelo de processo do Suprnova é o oposto - um único
binário de longa duração servindo milhares de solicitações
concorrentes. Um hit de BD por solicitação em toda verificação
de flag multiplicaria a carga do pool de conexões pela
contagem de verificações de flag. A cadeia de duas camadas
(snapshot do `DatabaseEvaluator` + TTL do `CachedEvaluator`)
é a resposta nativa do Rust: o hot path é totalmente síncrono
contra dados em memória, e a trait `FeatureSync` dá a
mudanças iniciadas pelo operador uma propagação de
sub-segundo sem um reload por polling. A forma é a mesma do
Pennant - defina uma flag, verifique-a em um handler,
sobrescreva-a a partir de uma rota de admin. O mecanismo
interno é diferente porque o runtime é diferente.

## Notas de design

- **Por que um avaliador sync em vez de async?** O
  `is_enabled` do featureflag é o hot path. Um avaliador
  async forçaria um `block_on` (propenso a deadlock) ou
  empurraria todo handler para fazer `.await` em leituras de
  flag (desastre de ergonomia). O framework faz a ponte entre
  sync e async via um snapshot em memória atualizado de forma
  assíncrona pela `FeatureSync`.

- **Por que uma trait `FeatureSync` separada em vez de
  estender `Evaluator`?** O `Evaluator` do featureflag é de
  propriedade de um crate upstream; não podemos adicionar
  métodos a ele. `FeatureSync` é uma trait irmã que apps
  implementam nos mesmos tipos concretos. O trait object é
  vinculado separadamente no contêiner App, para que um
  processo possa colocar múltiplos avaliadores em camadas
  enquanto ainda roteia notificações corretamente.

- **Por que `set_flag` é `pub` em `DatabaseEvaluator`?**
  Conveniência de teste. O caminho de escrita de produção é
  `admin::upsert`; `set_flag` existe para que testes possam
  preencher flags sem configurar um listener de
  `EventFacade`. Os dois caminhos chamam
  `features::sync::notify`, então o contrato de propagação se
  mantém dos dois jeitos.

- **Por que não existe um evento `FeatureRetrieved`?**
  Volume. Um handler que verifica dez flags por solicitação
  dispara dez eventos por solicitação - para um serviço de 1k
  req/s isso são 36M eventos/hora, bem acima da relação
  sinal-ruído de qualquer pipeline de auditoria. A auditoria
  do caminho de mutação (`FeatureUpdated` / `FeatureDeleted`)
  é o que é distribuído; a amostragem de read path, se
  necessária, se sobrepõe via um wrapper de avaliador
  customizado.

## Próximos passos

- [Middleware](middleware.md) - `FeatureMiddleware` pertence
  depois de `SessionMiddleware`; este capítulo cobre a
  ordenação e a pilha global
- [Eventos](events.md) - escute `FeatureUpdated` /
  `FeatureDeleted` para alimentar logs de auditoria, alertas
  do Slack, ou pipelines downstream
- [Contêiner de serviços](container.md) - como a vinculação
  `dyn FeatureSync` é resolvida, e por que
  `TestContainer::bind` existe para testes paralelos
- [Testes](testing.md) - os padrões
  `TestDatabase::fresh::<M>()` e `TestContainer::fake` nos
  quais este capítulo se apoia
- [Autenticação](authentication.md) - `Auth::id()` é o
  extractor padrão de user id, e alimenta `actor_id` para
  mutações de admin

Externo: a [documentação do crate featureflag](https://docs.rs/featureflag)
cobre as primitivas upstream `Evaluator`, `Context`, e
`Feature`. `suprnova::features::admin` é a facade de CRUD
completa - `cargo doc --open -p suprnova` para navegar.
