# Implantação do RenderCache

Um processo cacheando para si mesmo não precisa de nada além de memória.
Vários processos atrás de um balanceador de carga precisam concordar sobre o
que está armazenado, sobre quem tem permissão para reconstruir uma entrada e
sobre quando algo deixou de ser verdade - e precisam concordar sem que
nenhum deles seja capaz de convencer os outros de que conteúdo obsoleto está
atual. O RenderCache responde a isso com **perfis**: um perfil nomeia quais
provedores um processo constrói, e nada mais muda. Declarações de rota,
políticas, chaves, o coletor, o fluxo de middleware e a costura são
idênticos em todos os perfis, e nenhum tipo voltado para a aplicação difere
entre eles.

Este capítulo é sobre como escolher e conectar um. Os três perfis e o que
cada um oferece, as variáveis de ambiente que a configuração do framework de
fato lê, a migração que a sua aplicação precisa listar antes que um perfil
compartilhado suba, onde a instalação entra no seu bootstrap, e o que as
camadas prometem e não prometem. A aplicação de dogfood deste repositório
roda o perfil embedded por padrão e é inicializada sobre o perfil Database
por `the_database_profile_serves_a_hit_through_the_sql_stores` em
`app/tests/live_render_cache.rs`.

## Três perfis

| Perfil | Entradas na L1 | Liderança de reconstrução | Registros de instância do Live |
|---|---|---|---|
| `embedded` (padrão) | um arquivo por chave, ou nenhum | no processo | no processo |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`, `suprnova_live_promotions` |
| `redis` | um hash Redis por chave | um hash Redis por chave, mais um contador de tokens | um hash Redis por registro |

A coisa que **não** se move entre eles é a verdade das gerações. O ledger de
gerações apoiado no banco de dados é a autoridade em todos os perfis: seja
qual for a camada que entregou os bytes, a atualidade é provada contra o
banco de dados - relendo-o no hit sob `CoherenceMode::Authority`, ou por um
lease de validação concedido a partir de uma leitura anterior dele sob
`CoherenceMode::Lease`. É isso que mantém um acelerador sendo um acelerador:
o Redis pode perder tudo o que guarda sem que nada obsoleto seja provado
atual, porque nada do que o Redis guarda prova atualidade, para começo de
conversa.

Escolha pelo que você de fato precisa compartilhar:

- **`embedded`** para um único processo, e para vários processos que se
  contentam em manter cada um a sua própria cópia. Defina
  `RENDER_CACHE_L1_DIR` e cada processo ganha uma camada de arquivo que
  sobrevive ao seu próprio reinício.
- **`database`** quando vários nós devem compartilhar entradas armazenadas e
  eleger um líder de reconstrução por chave, e você preferiria não adicionar
  mais uma peça móvel à implantação.
- **`redis`** quando a latência da camada compartilhada importa mais do que
  a sua durabilidade, com o banco de dados ainda guardando por baixo a
  verdade das gerações.

## As variáveis de ambiente

`RenderCacheConfig::from_env` lê estas, em
`framework/src/render_cache/config.rs`:

| Variável | Padrão | Significado |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | qualquer coisa exceto `false` ou `0`; `false` torna `RenderCache::install` um no-op |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database` ou `redis`; define as duas linhas abaixo |
| `RENDER_CACHE_L1` | o do perfil | `disabled`, `file`, `database` ou `redis` |
| `RENDER_CACHE_COORDINATOR` | o do perfil | `local`, `database` ou `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4.096 | teto de entradas em processo |
| `RENDER_CACHE_L0_BYTES` | 128 MiB | teto de bytes em processo |
| `RENDER_CACHE_L1_DIR` | não definida | o diretório da camada de arquivo; sob `embedded`, defini-la é o que liga a L1 |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | o diretório inteiro para a camada de arquivo, uma entrada para as camadas de banco de dados e Redis |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, depois `redis://127.0.0.1:6379` | onde as duas camadas de cache Redis se conectam |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | o namespace de chaves sob o qual as duas camadas de cache Redis escrevem |
| `RENDER_CACHE_LEASE_MS` | 30.000 | tempo de vida do lease de reconstrução |
| `RENDER_CACHE_MAX_WAITERS` | 128 | teto de waiters em processo |
| `RENDER_CACHE_HINTS` | o do perfil | `disabled` ou `redis`; dicas de geração credíveis em `<prefix>hints`, sobre o endpoint acima |
| `RENDER_CACHE_FAILURE` | `open` | `open` serve a rota sem cache em uma falha de provedor, `closed` responde `503` |
| `APP_BUILD_ID` | a versão do pacote da aplicação (veja abaixo) | isola cada entrada no namespace do build que a produziu |

`RENDER_CACHE_HINTS` é o único botão aqui que não muda resposta nenhuma. Sob
o perfil `redis` ele vale `redis` por padrão, e sob os outros dois
`disabled`, e ele não tem endpoint próprio: pega carona no
`RENDER_CACHE_REDIS_URL` e no `RENDER_CACHE_REDIS_PREFIX` acima. Uma dica só
pode encurtar um lease de validação que um nó já detém, então uma
implantação com elas desligadas, uma cujo canal está morto e uma construída
sem o recurso servem as mesmas entradas e admitem as mesmas reconstruções;
só muda o momento da revalidação. É por isso também que um endpoint de dicas
inalcançável não impede o boot, como impede uma camada de cache
inalcançável.

O perfil é um atalho, não uma trava. `RENDER_CACHE_L1` e
`RENDER_CACHE_COORDINATOR` sobrescrevem cada um a sua metade, de modo que
uma implantação que quer as suas entradas no banco de dados mas os seus
leases de reconstrução no processo diz exatamente isso, em vez de escolher o
perfil inteiro mais próximo.

Uma variável com um conjunto fechado de valores aceitos que seja definida
como algo fora dele faz o boot falhar com uma mensagem nomeando a variável.
O valor rejeitado nunca é repetido nessa mensagem, porque um valor de
ambiente pode carregar um segredo.

**Defina `APP_BUILD_ID` explicitamente, uma vez por deploy.** Ele é
misturado em toda chave de busca, então mudá-lo é o que impede um build novo
de servir entradas que o anterior publicou. Na ausência da variável,
`RenderCacheConfig::from_env` recai para a própria versão do pacote da sua
aplicação: `#[suprnova::main]` registra `CARGO_PKG_VERSION` a partir da
compilação do próprio crate da aplicação, no momento em que carrega o
ambiente, e é esse valor registrado para o qual o padrão recai aqui.
Somente um binário que nunca expande `#[suprnova::main]` recai ainda mais
longe, para a versão do próprio crate do **framework** - nomeada assim
porque é fácil confundi-la com a da aplicação de outro modo. De todo modo
o valor só se move quando alguém incrementa um número de versão, e uma
versão de pacote raramente muda por deploy: um deploy que muda um
template, uma tradução ou um handler sem incrementar a versão mantém o
mesmo id de build e pode servir entradas que o build anterior publicou.
Defina-o como algo que muda toda vez que você entrega - um id de commit ou
um identificador de release:

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

Uma instalação que nunca lê o ambiente define o mesmo valor em código com
`RenderCacheConfig::with_build_id`, que sobrescreve o que quer que
`from_env` tenha escolhido - um `APP_BUILD_ID` explícito incluído - para
uma aplicação que deriva seu próprio identificador por deploy de forma
programática.

Qualquer que seja o valor definido, espera-se que o binário de produção
que o lê esteja construído na [forma de build de
produção](deployment.md#production-build-shape) da Suprnova: com as
features padrão desligadas e `testing` reservado só para `cargo test`.

O próprio ledger de instâncias do Live é configurado à parte, porque ele é
autoridade do Live e não armazenamento do cache: `LIVE_LEDGER_DRIVER`
(`memory`, `database` ou `redis`), `LIVE_REDIS_URL` e `LIVE_REDIS_PREFIX`.
Uma implantação pode rodar o cache em uma camada e o ledger em outra.

## A migração que a sua aplicação precisa listar

O schema do RenderCache pertence ao framework e é aplicado pela aplicação.
Seu `Migrator` o lista, para que `suprnova migrate` provisione as tabelas ao
lado das suas:

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

Isso é `app/src/migrations/mod.rs` literalmente, e as duas não são
intercambiáveis:

- **`Migration`** cria as três tabelas `suprnova_render_*` que guardam a
  verdade durável das gerações: as gerações atuais, um log de mudanças
  apenas de acréscimo, e o epoch de autoridade. Todo perfil precisa dela,
  inclusive o `embedded`, porque a verdade das gerações nunca se muda para
  uma camada de cache.
- **`TierMigration`** cria as quatro tabelas que o armazenamento L1 de banco
  de dados e o coordenador de reconstrução de banco de dados leem. Apenas um
  perfil que as alcance precisa dela - mas `RenderCache::install` se recusa
  a iniciar o perfil Database sem elas, então listá-la é o que torna
  `RENDER_CACHE_PROFILE=database` uma escolha de configuração que a sua
  aplicação de fato pode fazer.

Uma aplicação que define `RENDER_CACHE_ENABLED=false` não precisa carregar
nenhuma das duas: a instalação devolve o router intocado, não sonda nada,
não monta runtime algum, não registra middleware algum e deixa o lado da
escrita sem instrumentação, então nada paga por um cache que está desligado.

## Instalando

`RenderCache::install` é assíncrono, porque ele sonda pelas tabelas e faz
ping em cada endpoint Redis distinto que a configuração usaria antes de
montar qualquer coisa. `Application::try_routes_async` é o gancho que o
hospeda. Isto é `app/src/live/mod.rs`, e a divisão em duas funções vale a
pena ser copiada:

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` é a metade interna síncrona: ela registra as rotas Live reservadas,
as rotas de documento e todas as políticas de cache, e não instala middleware
algum. `cmd/main.rs` alcança `routes_with_render_cache` através de
`Application::try_routes_async`, e o servidor do cenário de navegador em
`app/examples/live_dogfood_host.rs` faz o await dele diretamente.

A costura de configuração por baixo disso não é decoração. Um teste que
precisa de um perfil diferente ou de um relógio que ele possa mover não tem
outra entrada, e importa que ele instale *as mesmas* rotas, políticas e
ordenação de middleware que o servidor instala, diferindo apenas na
configuração que passou. Os dois boots de dogfood passam por ela:
`the_database_profile_serves_a_hit_through_the_sql_stores` passa uma
configuração do perfil Database, e
`stale_service_is_marked_and_rebuilt_in_the_background` passa uma que carrega
um relógio ajustável. Veja "Testando uma rota em cache" em
[Operações do RenderCache](render-cache-operations.md).

Duas regras de ordenação, ambas de responsabilidade de quem chama:

1. Toda rota e todo grupo precisam ser incluídos **antes** do `install`, que
   lê o que estiver registrado até aquele ponto.
2. O `install` acrescenta à cadeia global de middleware, então ele precisa
   rodar **depois** do middleware de sessão, localidade e identidade cujo
   estado com escopo de requisição o middleware de cache lê enquanto deriva
   uma chave de busca.

A instalação falha de forma fechada, por meio de duas sondagens. Ela verifica
que as tabelas que a configuração alcançaria existem, e faz ping em cada
endpoint Redis distinto que a configuração usaria, uma vez por endpoint.
Qualquer uma das duas falhando interrompe o boot com uma única frase
acionável nomeando a migração ou a variável a corrigir, para que nada seja
jamais servido contra uma tabela ausente ou um endpoint que ninguém
responde. (O próprio ledger de instâncias do Live é sondado à parte, por
`Server::run`, antes que qualquer requisição seja servida.)

## Escolhendo onde vivem as entradas de uma rota

O perfil decide o que a L1 *é*; a política decide quais rotas a usam. O
builder armazena apenas na L0 a menos que uma rota declare o contrário,
então uma camada compartilhada é povoada de propósito:

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` prova a ida e
volta de ponta a ponta no perfil Database: a entrada publicada é lida de
volta do armazenamento SQL sob a chave que o middleware derivou, depois a L0
é esvaziada e a requisição seguinte ainda é respondida sem uma renderização.
Veja [Representações do RenderCache](render-cache-representations.md) para
como decidir por rota.

## Uma suíte de conformidade, todos os provedores

Todo armazenamento responde à mesma suíte.
`framework/tests/render_cache/store_conformance.rs` roda os cenários de
provedor da engine - escritos apenas contra a trait
`RenderStore` - sobre a L1 apoiada em arquivo, a L1 SQL no SQLite, no
PostgreSQL e no MySQL, e a L1 Redis, e o armazenamento em processo responde
à mesma suíte no crate da engine. PostgreSQL, MySQL e Redis rodam por meio de
testes ignorados que `scripts/check-postgres.sh`, `scripts/check-mysql.sh` e
`scripts/check-redis.sh` selecionam pelo nome contra servidores reais. Um
provedor não é "suportado" aqui porque existe; ele é suportado porque passa
nas mesmas palavras que todos os outros.

## O que as camadas prometem, e o que não prometem

- **Nenhuma espera entre nós.** Uma chave que outro nó já está reconstruindo
  é um bypass: este nó renderiza e não publica nada. Computação duplicada e
  limitada entre nós é aceita; duas publicações aceitas não são, e o próprio
  fence de publicação do armazenamento é o que proíbe a segunda.
- **Um backend perdido é um miss, nunca uma resposta errada.** Despejo,
  expiração ou um reinício do Redis fazem entradas darem miss e instâncias
  ficarem ausentes. A verificação de coerência contra o ledger de gerações do
  banco de dados roda em todo hit, seja lá o que tenha servido os bytes.
- **Bytes adulterados são um miss.** Os bytes de uma entrada em uma linha ou
  em um hash são um frame de codec assinado, então um valor rasgado,
  truncado ou alterado falha na sua verificação de integridade e é tratado
  como um miss em vez de ser servido.
- **As camadas de banco de dados e Redis não fazem despejo para abrir
  espaço.** O `RENDER_CACHE_L1_BYTES` limita uma entrada ali, não a tabela
  nem o keyspace; o crescimento é limitado por retenção. Só a camada de
  arquivo limita um diretório inteiro, porque ela é dona daquele diretório
  sozinha.
- **Os adaptadores Redis miram uma única instância Redis 7 ou mais nova.** O
  Redis Cluster é recusado: os scripts tocam chaves que não declaram, e leem
  o relógio do armazenamento com `TIME` dentro de um script.
- **O MySQL precisa da versão 8.0.19 ou mais nova** para a classificação
  precisa de chave duplicada. MySQL e MariaDB mais antigos relatam uma
  colisão que este build não vai atribuir a uma tabela, então ele degrada
  para um erro de provedor indisponível - a direção segura, e nada é
  concedido duas vezes de qualquer forma.
- **Toda decisão de expiração entre nós é tomada no relógio do backend**,
  lido dentro da operação que age sobre ela. Um nó cujo relógio adianta não
  consegue nem estender um lease, nem esconder uma entrada viva dos seus
  pares, nem declarar o registro de um par como vencido.

### Por que Suprnova diverge

Os pacotes de cache de resposta do Laravel herdam o armazenamento de cache
que você já configurou, então "implantar o cache em vários nós" significa
apontar `CACHE_STORE` para o Redis e confiar que o que estiver lá dentro
está certo. Não existe noção separada de quem pode reconstruir uma entrada,
nenhum fence que impeça dois workers de publicarem bytes conflitantes para a
mesma chave e - o mais consequente - nenhuma autoridade por baixo do
armazenamento. Se o Redis tem uma página, a página é servida; se o Redis é
esvaziado, tudo é recomputado. O armazenamento *é* a verdade.

O Suprnova separa os dois deliberadamente. A camada compartilhada guarda
bytes e nada mais; o banco de dados guarda a verdade das gerações em todos
os perfis, e é contra ele que um hit é verificado. É por isso que perder o
Redis aqui custa latência em vez de corretude, por que uma reconstrução é
concedida por lease e uma publicação é protegida por fence em vez de
disputada, e por que as mesmas declarações de rota rodam sem mudança desde
um `cargo run` em um laptop até uma frota coordenada por banco de dados. O
custo é uma migração que a sua aplicação precisa listar e uma leitura de
banco de dados no caminho do hit que um cache chave-valor simples não paga -
uma instrução, ou zero sob um lease de validação. Veja
[Gerações do RenderCache](render-cache-generations.md) para o que essa
leitura compra.

## Próximos passos

- [Operações do RenderCache](render-cache-operations.md) - os comandos de
  console, a telemetria, a higiene de disco e o que fazer quando algo está
  errado
- [Implantação](deployment.md) - o checklist de produção ao redor
- [Migrações](migrations.md) - como a lista do `Migrator` acima é aplicada
