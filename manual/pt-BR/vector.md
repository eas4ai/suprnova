# Vetor

O Suprnova vem com uma facade `Vector` no formato Laravel,
apoiada por um de quatro drivers - Memory em processo,
Qdrant, Pinecone, ou o `VECTOR(N)` nativo do MariaDB -
escolhido explicitamente no boot via `Vector::register`. A
facade é uma camada fina sobre uma trait `VectorDriver`,
então backends customizados se encaixam do mesmo jeito que
os embutidos.

## Início rápido

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// Inicialização (tipicamente uma vez no início do app)
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// Uso
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## O contrato

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` carrega um id `String` arbitrário, um
`embedding: Vec<f32>`, e um `metadata: serde_json::Value` de
forma livre (precisa ser um objeto JSON ou `null`).
`VectorMatch` retorna o id original, o score de similaridade
do backend, e a mesma forma de metadados.

A trait é pequena de propósito. Quando você precisar de
expressões de filtro na busca, vetores esparsos, scroll/list,
snapshots, ou controles de quantização, desça até o SDK
subjacente do driver via sua válvula de escape `client()`
pública.

### Por que Suprnova diverge

O Laravel só distribui vetores através do `pgvector` do
Postgres. Essa é a resposta no formato PHP: escolha um
backend de armazenamento, esconda-o atrás de um único
driver, e dê como resolvido. O Suprnova trata a escolha como
uma questão de configuração. A mesma trait cobre um
`HashMap` em processo para testes, um banco de dados de
vetor dedicado (Qdrant, Pinecone) quando a contagem de
embeddings justifica o custo operacional, e um backend
relacional (MariaDB 11.7+) quando você prefere manter os
vetores junto das linhas que os produziram. Weaviate,
Milvus, LanceDB, pgvector e LibSQL aguardam demanda real de
usuários antes de entrar - nenhum é bloqueado pela forma da
trait.

Quando o resto do seu app cabe em um único engine, o MariaDB
11.7+ mantém vetores junto de tabelas relacionais, documentos
JSON, e dados temporais com versionamento de sistema - menos
peças móveis do que rodar Postgres + Redis + Qdrant
separadamente. Veja [Implantação](deployment.md) para a
recomendação em contexto.

## Drivers

### Memory - `MemoryVectorDriver`

Driver em processo, apoiado por `HashMap`. Similaridade de
cosseno; pontos com dimensão incompatível são pulados
silenciosamente na consulta (para que dados de teste com
dimensões mistas não explodam), consultas com vetor zero dão
erro de forma clara.

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

Use em testes e no dev. Cada instância de
`MemoryVectorDriver::new()` é hermética - não há estado
compartilhado entre dois `new()`s.

### Qdrant - `QdrantVectorDriver`

Conversa com o Qdrant via gRPC (porta padrão 6334) através do
SDK oficial `qdrant-client`.

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // padrão
    .with_auto_create(true);                // padrão

Vector::register("docs", Arc::new(driver));
```

Para o Qdrant Cloud:

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**Mapeamento de ID.** O Qdrant exige que IDs de ponto sejam
`u64` ou um UUID válido. O framework faz a ponte de strings
arbitrárias com três regras:

1. Se a string faz parse como `u64`, a variante `Num(u64)` é
   usada.
2. Se a string é um UUID válido, a variante `Uuid(String)` é
   usada ao pé da letra.
3. Caso contrário, um UUID v5 determinístico é derivado a
   partir de um namespace estável.

A string original do chamador é guardada no payload do ponto
sob a chave reservada `__suprnova_id` (exportada como
`SUPRNOVA_ID_PAYLOAD_KEY`) e removida de
`VectorMatch.metadata` na recuperação. Usuários avançados que
consultam o Qdrant diretamente via `driver.client()` podem
filtrar por `__suprnova_id` para conectar escritas do
framework com chamadas diretas.

**Criação automática.** No primeiro `upsert` para uma coleção
nunca vista, o driver a cria com a dimensão inferida a partir
do primeiro item e a métrica de distância configurada
(Cosseno por padrão). Seguro contra corrida - chamadas de
upsert concorrentes na mesma coleção nova não falham: quem
cria primeiro vence, o outro segue adiante. Desative via
`.with_auto_create(false)` para exigir criação explícita.

**Invalidação de cache.** Se uma coleção for descartada
externamente (ou o Qdrant reiniciar antes que a persistência
seja gravada), o driver detecta o erro "not found" no upsert,
descarta a entrada de cache, roda `ensure_collection` de novo,
e tenta de novo uma vez.

**Válvula de escape.** `driver.client()` retorna o
`qdrant_client::Qdrant` subjacente - use-o para expressões de
filtro na busca, scroll, snapshots, ou outras APIs não
expostas pela trait. `QdrantVectorDriver::resolve_point_id`,
`build_point`, e `decode_match` deixam você misturar chamadas
diretas e roteadas pela trait sem perder a tradução de id.

**Configuração local.** Rode o Qdrant via Docker:

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

Testes de integração rodam via:

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **Atrás de uma feature Cargo - desativado por padrão.**
> Ative com `cargo build --features vector-pinecone` (ou
> adicione `features = ["vector-pinecone"]` sob a
> dependência `suprnova` no seu `Cargo.toml`). A feature não
> custa dependências extras - ela só bloqueia a compilação do
> driver, nada mais - então está desativada simplesmente
> porque a maioria dos apps não usa o Pinecone e não deveria
> pagar para compilá-lo.

Conversa com o Pinecone via sua API REST, usando o cliente
HTTP que o framework já carrega.

> **Por que não o SDK oficial?** O driver costumava envolver
> o `pinecone-sdk`, que fala gRPC. O release mais recente
> daquele crate (0.1.2, publicado em 2024-09-06) fixa `tonic
> 0.11 → rustls 0.22 → rustls-webpki 0.102`, e o
> `rustls-webpki 0.102` carrega quatro avisos do RustSec que
> já foram todos corrigidos upstream em `>= 0.103.13`. Um
> crate abandonado travava a árvore inteira, sem nenhuma
> versão de "esperar pelo upstream" que fosse terminar algum
> dia. O Pinecone expõe toda operação que este driver
> precisa via HTTPS, então a rota REST removeu quatro avisos
> e duas dependências de uma vez.

```rust
use suprnova::PineconeVectorDriver;

// Chave de API direto
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// Ou via env: PINECONE_API_KEY, mais opcionalmente
// PINECONE_CONTROLLER_HOST e PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// Vincula a um namespace não padrão
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

O nome de armazenamento passado via `Vector::store(name)`
mapeia para um nome de índice do Pinecone. O driver resolve o
host daquele índice de forma lazy no primeiro uso, via o `GET
/indexes/{name}` do control plane, e então o guarda em cache.
Pule a viagem de ida e volta fixando o host que você já
conhece:

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

Um host aprendido a partir do control plane é sempre
contatado via `https`, seja o que for que a resposta diga. Um
host fixado via `with_index_host` mantém o esquema que você
passou, então um emulador local em `http://` funciona.

**Versão da API.** O Pinecone versiona sua API REST por data
e quer aquela versão fixada em um header. O driver fixa
`2025-04` - a versão contra a qual suas formas de solicitação
e resposta foram escritas e testadas - e expõe
`with_api_version` (ou `PINECONE_API_VERSION`) para mudar de
forma deliberada. Ela não flutua: a convenção de
namespace-key em `describe_index_stats` é uma das coisas que
mudou entre versões, e `count()` lê esse mapa.

**Sem criação automática.** Criar um índice no Pinecone exige
escolher cloud (AWS/GCP/Azure), região, dimensão do vetor,
métrica de distância, e proteção contra exclusão -
trade-offs demais para ter um padrão bom. Crie índices pelo
console do Pinecone, pela CLI do Pinecone, ou por uma chamada
`control_plane_post` antes de registrar, depois aponte o
framework para o nome existente.

Essa é a principal assimetria com o driver do Qdrant, que
cria coleções automaticamente no primeiro upsert.

**IDs e metadados.** O Pinecone aceita ids `String`
arbitrários nativamente, então `VectorItem::id` passa direto.
Metadados são carregados como JSON de ponta a ponta -
`PineconeVectorDriver::metadata_from_json` /
`metadata_to_json` só aplicam a própria regra do framework de
que metadados são um objeto ou null. O próprio Pinecone
restringe *valores* de metadados a strings, números,
booleanos e listas de strings, e rejeita objetos aninhados no
lado do servidor; o driver não reimplementa essa verificação,
porque as regras do Pinecone são versionadas e uma cópia
local ficaria desatualizada.

**Limites de batch.** O Pinecone documenta um máximo de 1000
vetores por upsert e 1000 ids por delete. O driver envia o
que você passar em uma única solicitação em vez de dividir em
chunks silenciosamente - uma escrita com sucesso parcial é
mais difícil de raciocinar sobre do que uma rejeitada. Divida
em batches do seu lado se você exceder esses limites.

**Namespaces.** Uma instância de driver se vincula a um
namespace. Para usar múltiplos namespaces do mesmo índice,
registre um driver por namespace sob nomes de armazenamento
diferentes:

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**Throughput.** Nada serializa. O driver guarda em cache uma
string de host por índice, não um handle de conexão, e as
solicitações compartilham o pool de conexões do `reqwest` -
então chamadas concorrentes ao mesmo índice seguem
concorrentemente. (O driver gRPC que este substitui mantinha
um `Index` por nome atrás de um `tokio::Mutex`, porque o
`pinecone-sdk` expunha `Index` só atrás de `&mut self`.)

**Válvula de escape.** `control_plane_get`,
`control_plane_post` e `data_plane_post` alcançam qualquer
endpoint que o Pinecone distribui, com seus próprios tipos de
solicitação e resposta, sobre o transporte autenticado e com
host resolvido do driver - expressões de filtro, vetores
esparsos, fetch-by-id, `/vectors/list`, gerenciamento de
índice:

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**Testes.** Testes de contrato de rede rodam por padrão sob a
feature: eles rodam o driver contra um fake local e verificam
o método, path, headers e corpo JSON exatos que ele coloca na
rede. Esses fixam o driver ao contrato *documentado* do
Pinecone. Confirmar que a documentação corresponde ao serviço
em produção precisa dos testes de integração marcados com
`#[ignore]`, que exigem as duas env vars:

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

Conversa com o MariaDB 11.7+ via `sqlx::MySqlPool` direto,
usando o tipo de coluna nativo `VECTOR(N)` do MariaDB e
indexação HNSW. Na primeira vez que você chama um método do
driver, ele roda `SELECT VERSION()` e rejeita qualquer coisa
abaixo de 11.7 - servidores mais antigos não têm as funções
de vetor.

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // padrão

Vector::register("documents", Arc::new(driver));
```

`from_url` é lazy - ele valida a sintaxe da URL mas NÃO abre
uma conexão até o primeiro uso, então chamá-lo na
inicialização do app é seguro mesmo antes do banco de dados
estar alcançável. Envolva um pool existente com
`MariaDbVectorDriver::from_pool(pool)` quando você precisar
de opções de pool customizadas.

**O schema é seu.** O driver não cria tabelas
automaticamente - schema é uma questão de migração. O
caminho recomendado é `driver.ensure_table_sql_for(name,
dim)`, que herda a distância configurada do driver, então a
cláusula `DISTANCE=` da migração e a função de consulta que
`similar` usa são garantidas de corresponder:

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// Resultado:
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

Para geradores de migração que não têm um driver no escopo
(ferramentas de CLI, build scripts), use o
`MariaDbVectorDriver::ensure_table_sql(name, dim, distance)`
estático e passe a mesma `MariaDbDistance` que você vai
configurar depois no driver.

**A distância precisa corresponder nas duas pontas.** O
MariaDB recai silenciosamente para um full table scan quando
a função usada no momento da consulta não corresponde à
cláusula `DISTANCE=` do índice. O driver se protege contra
isso em duas camadas:

1. **`ensure_table_sql_for(name, dim)`** lê `self.distance`
   tanto para o SQL de migração emitido quanto para a função
   de runtime em `similar` - os dois não conseguem se
   desalinhar por construção.
2. **Uma verificação em runtime na primeira chamada de
   `similar`** roda um `SHOW CREATE TABLE` por armazenamento,
   faz parse da cláusula `DISTANCE=` real a partir do schema
   em produção, e dá erro de forma clara se ela discordar de
   `with_distance(...)`. O resultado é armazenado em cache,
   então chamadas subsequentes são de custo zero. Isso pega
   migrações escritas à mão ou configurações de `from_pool`
   que contornam `ensure_table_sql_for`.

**Segurança do nome de armazenamento.** Nomes de
armazenamento são interpolados no SQL emitido (o MySQL não
parametriza identificadores). Nomes são validados como
`[A-Za-z_][A-Za-z0-9_]*` com tamanho ≤ 64; o nome validado é
então citado com backtick em toda statement. Nomes inválidos
dão erro com `FrameworkError::param` na fronteira de
`register`/`upsert`/`similar`/`delete`/`count`.

**IDs e metadados.** `VARCHAR(255)` aceita ids `String`
arbitrários - sem derivação de UUID, sem chaves de payload
reservadas. Metadados fazem o round-trip através do tipo de
coluna `JSON` do MariaDB; metadados `null` são armazenados
como `NULL` do SQL. Metadados que não são objeto (arrays,
primitivos) são rejeitados com `FrameworkError::param`, para
paridade com Qdrant e Pinecone.

**Normalização de score.** O MariaDB retorna a *distância*
bruta (menor = mais próximo). O contrato da trait é *score*
(maior = mais similar) - o driver converte por métrica:

| Métrica    | MariaDB retorna       | `score` exposto              |
| ---------- | --------------------- | ----------------------------- |
| Cosseno    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`   |
| Euclidiana | `[0, ∞)` norma L2     | `1.0 / (1.0 + d)` → `(0, 1]` |

Nos dois casos, o ranking é preservado (melhor resultado
primeiro), mas os valores absolutos de score NÃO são
comparáveis entre drivers - só a ordenação é. Cada backend
chega a uma convenção `higher = better`, mas as faixas
diferem: o cosseno do Memory retorna `[-1, 1]`, o cosseno
normalizado do MariaDB retorna `[0, 1]`, o Qdrant emite sua
similaridade de cosseno nativa em `[-1, 1]`, e o Pinecone
retorna a similaridade bruta para qualquer métrica com a qual
o índice foi criado. Use `score` para ordenar dentro do
conjunto de resultados de um único driver; não compare scores
numéricos entre drivers sem renormalizar você mesmo.

**Válvula de escape.** `driver.pool()` retorna o
`sqlx::MySqlPool` subjacente para consultas brutas que a
trait não cobre. `MariaDbVectorDriver::embedding_to_vec_text`,
`score_from_distance`, e `ensure_table_sql` são funções puras
que você pode chamar independentemente ao misturar SQL
direto com chamadas roteadas pela trait.

**Comportamento de upsert em massa.** `upsert` emite uma
statement `INSERT ... VALUES (...), (...), ...` multi-linha
por chunk de 500 linhas, tudo dentro de uma única transação.
As viagens de ida e volta na rede caem ~500x comparado a
inserts linha a linha ao carregar um corpus novo; a chamada
permanece atômica através do batch inteiro. O tamanho do
chunk é interno - chame `upsert` uma vez com todos os seus
itens e o driver cuida da divisão em chunks.

**Índices HNSW são reconstruídos no momento do commit.** O
MariaDB atualiza o grafo HNSW conforme as linhas entram, mas
o trabalho de indexação se concentra no commit. Um `upsert`
de 1 milhão de linhas vai manter a transação aberta durante
toda a construção do índice, o que pode levar minutos. Para
cargas iniciais muito grandes, divida o corpus em batches de
10 mil a 100 mil linhas e chame `upsert` repetidamente, para
que cada batch faça commit e libere o lock entre as rodadas.
(Chamadas de `upsert` menores não são mais lentas por linha -
elas só espalham o trabalho de indexação em mais pontos de
commit.)

**A dimensão é fixada na criação da tabela.** `VECTOR(N)`
fixa a dimensão; trocar de modelo de embedding de um modelo
de 768 dimensões para um de 1536 dimensões significa uma
migração de tabela completa (tabela nova, re-embed, e troca).
Planeje upgrades de modelo do mesmo jeito que você planejaria
uma migração de schema - não existe um caminho "ALTER COLUMN
VECTOR(768) → VECTOR(1536)".

**Dimensionamento do pool.** `from_url` usa o
`MySqlPoolOptions` padrão do sqlx - `max_connections = 10` no
momento em que este texto foi escrito. Para workloads de QPS
alto (centenas de chamadas de `similar` por segundo),
construa o pool você mesmo com
`MySqlPoolOptions::new().max_connections(N).connect_lazy(url)`
e passe para `from_pool`. O driver não impõe seu próprio
limite de conexões.

**Configuração local.** Rode o MariaDB 11.7+ via Docker:

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

Testes de integração rodam via:

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## Comparação de drivers

| Aspecto | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| Armazenamento subjacente | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| Persistência | Nenhuma | Sim | Sim | Sim |
| Criação automática | N/A | Sim (configurável) | Não (usuário cria o índice) | Não (a migração é sua) |
| IDs de string | Nativos | Hash para UUID-5 | Nativos | Nativos |
| Chave de metadados reservada | Nenhuma | `__suprnova_id` | Nenhuma | Nenhuma |
| Throughput | Por processo | Concorrente | Concorrente (limitado por pool) | Concorrente (limitado por pool) |
| Métrica de distância | Cosseno | Configurável | Definida na criação do índice | Cosseno / Euclidiana |
| Requisito de versão | - | Qualquer | Qualquer | **11.7+** |

## Notas operacionais

**Convenções de nome de armazenamento.** O nome de
armazenamento passado para `Vector::register` e
`Vector::store` é um rótulo - pode ser qualquer string. Para
o Qdrant, o framework o usa como o nome da coleção; para o
Pinecone, como o nome do índice. Combine o rótulo com o
esquema de nomenclatura existente do backend.

**Re-registrar** um nome com uma nova instância de driver é
uma operação onde a última escrita vence, por design - útil
para trocar drivers em harnesses de teste sem reiniciar o
processo.

**Isolamento de teste.** Tanto os testes do Memory quanto os
testes de driver apoiados em registry usam nomes de
armazenamento únicos marcados com timestamp, para evitar
colisões em execuções de teste paralelas.

**Semântica de erro.** `Vector::store(name)` retorna
`FrameworkError::not_found` para nomes não registrados.
Falhas no nível do driver (rede, auth, incompatibilidade de
dimensão) voltam como `FrameworkError::internal` ou
`FrameworkError::param`, com a string de causa na mensagem
exibida.

## Estendendo

Para adicionar um quinto backend (Weaviate, Milvus, LanceDB,
pgvector, LibSQL, ...):

1. Adicione um novo `framework/src/vector/<backend>.rs`
   implementando `VectorDriver`.
2. Reexporte o tipo do driver a partir de
   `framework/src/vector/mod.rs` e da raiz do crate.
3. Espelhe a divisão de testes do Pinecone: testes de função
   pura e testes de contrato de rede (contra um fake local de
   `wiremock`) sempre rodam; testes de integração são
   bloqueados por `#[ignore]`, condicionados a env vars de
   credenciais. A camada do meio é a que vale a pena manter -
   um backend que ninguém consegue alcançar a partir do CI
   ainda tem um formato de rede que um erro de digitação pode
   quebrar.

A trait é pequena de propósito, para que a barra para lançar
um novo driver continue baixa. Se um backend precisar de uma
superfície que não encaixa (expressões de filtro, vetores
esparsos, busca híbrida), exponha isso através de uma válvula
de escape no driver - não infle a trait.

## Próximos passos

- [Implantação](deployment.md) - a recomendação de MariaDB
  como padrão de produção, em contexto
- [Banco de dados](database.md) - configuração multi-driver
  do SeaORM, incluindo o MariaDB como backend relacional ao
  lado de vetores
- [Variáveis de ambiente](env-vars.md) - `QDRANT_URL`,
  `PINECONE_API_KEY`, `MARIADB_URL` e outros contratos de env
  var de driver
- [Cache](cache.md) - facade irmã com a mesma forma de
  trait-driver
- [Mapa de paridade do Laravel](parity.md) - onde a busca
  vetorial se posiciona em relação ao Scout
