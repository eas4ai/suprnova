# Vector

Suprnova ofrece una fachada `Vector` con forma de Laravel, respaldada
por uno de cuatro drivers - Memory en proceso, Qdrant, Pinecone, o el
`VECTOR(N)` nativo de MariaDB - elegido explícitamente en el arranque
vía `Vector::register`. La fachada es una capa fina sobre un trait
`VectorDriver`, así que los backends personalizados se enchufan de la
misma forma que los integrados.

## Inicio rápido

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// Arranque (normalmente una sola vez al iniciar la app)
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// Úsalo
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

## El contrato

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` lleva un id `String` arbitrario, un `embedding: Vec<f32>`, y un `metadata: serde_json::Value` de forma libre (debe ser un objeto JSON o `null`). `VectorMatch` devuelve el id original, la puntuación de similitud del backend, y la misma forma de metadatos.

El trait es deliberadamente pequeño. Cuando necesites expresiones de filtro en la búsqueda, vectores dispersos, scroll/list, instantáneas, o parámetros de cuantización, baja al SDK subyacente del driver a través de su vía de escape pública `client()`.

### Por qué Suprnova diverge

Laravel ofrece vectores solo a través de `pgvector` en Postgres. Esa
es la respuesta con forma de PHP: elegir un backend de almacenamiento,
esconderlo detrás de un único driver, y darlo por terminado. Suprnova
trata la elección como un asunto de configuración. El mismo trait
cubre un `HashMap` en proceso para tests, una base de datos vectorial
dedicada (Qdrant, Pinecone) cuando la cantidad de embeddings justifica
el coste operativo, y un backend relacional (MariaDB 11.7+) cuando
prefieres mantener los vectores junto a las filas que los produjeron.
Weaviate, Milvus, LanceDB, pgvector, y LibSQL esperan su turno detrás
de demanda real de usuarios - ninguno está bloqueado por la forma del
trait.

Cuando el resto de tu app cabe en un solo motor, MariaDB 11.7+
mantiene los vectores junto a las tablas relacionales, los documentos
JSON, y los datos temporales con versionado de sistema - menos piezas
móviles que ejecutar Postgres + Redis + Qdrant por separado. Consulta
[Despliegue](deployment.md) para la recomendación en contexto.

## Drivers

### Memory - `MemoryVectorDriver`

Driver en proceso respaldado por `HashMap`. Similitud de coseno; los puntos con dimensión no coincidente se omiten en silencio en la consulta (así los datos de test con dimensiones mezcladas no explotan), y las consultas con vector cero fallan con un error claro.

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

Úsalo en tests y en desarrollo. Cada instancia de `MemoryVectorDriver::new()` es hermética - no hay estado compartido entre dos `new()`.

### Qdrant - `QdrantVectorDriver`

Habla con Qdrant por gRPC (puerto 6334 por defecto) a través del SDK oficial `qdrant-client`.

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // por defecto
    .with_auto_create(true);                // por defecto

Vector::register("docs", Arc::new(driver));
```

Para Qdrant Cloud:

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**Mapeo de IDs.** Qdrant exige que los IDs de punto sean `u64` o un UUID válido. El framework tiende un puente entre cadenas arbitrarias con tres reglas:

1. Si la cadena se analiza como `u64`, usa la variante `Num(u64)`.
2. Si la cadena es un UUID válido, usa la variante `Uuid(String)` tal cual.
3. En cualquier otro caso, deriva un UUID v5 determinista a partir de un namespace estable.

La cadena original de quien llama se guarda en el payload del punto bajo la clave reservada `__suprnova_id` (exportada como `SUPRNOVA_ID_PAYLOAD_KEY`) y se retira de `VectorMatch.metadata` al recuperarla. Los usuarios avanzados que consultan Qdrant directamente vía `driver.client()` pueden filtrar por `__suprnova_id` para conectar las escrituras del framework con las llamadas directas.

**Auto-creación.** En el primer `upsert` sobre una colección no vista, el driver la crea con la dimensión inferida del primer elemento y la métrica de distancia configurada (coseno por defecto). Seguro ante carreras - varios upserts concurrentes sobre la misma colección recién creada no fallan; quien la cree primero gana, y el resto continúa. Desactívalo con `.with_auto_create(false)` para exigir creación explícita.

**Invalidación de caché.** Si una colección se elimina externamente (o Qdrant se reinicia antes de haber persistido), el driver detecta el error "not found" en el `upsert`, descarta la entrada de caché, vuelve a ejecutar `ensure_collection`, y reintenta una vez.

**Vía de escape.** `driver.client()` devuelve el `qdrant_client::Qdrant` subyacente - úsalo para expresiones de filtro en la búsqueda, scroll, instantáneas, u otras APIs que el trait no expone. `QdrantVectorDriver::resolve_point_id`, `build_point`, y `decode_match` te permiten mezclar llamadas directas y enrutadas por el trait sin perder la traducción de ids.

**Configuración local.** Ejecuta Qdrant con Docker:

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

Los tests de integración se ejecutan con:

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **Detrás de una feature - desactivado por defecto.** Actívalo con `cargo build --features vector-pinecone` (o añade `features = ["vector-pinecone"]` bajo la dependencia `suprnova` de tu `Cargo.toml`). La feature no cuesta dependencias extra - solo activa la compilación del driver, nada más - así que está desactivada simplemente porque la mayoría de las apps no usan Pinecone y no deberían pagar por compilarlo.

Habla con Pinecone a través de su API REST, usando el cliente HTTP que el framework ya trae consigo.

> **¿Por qué no el SDK oficial?** El driver solía envolver `pinecone-sdk`, que habla gRPC. La versión más reciente de ese crate (0.1.2, publicada el 2024-09-06) fija `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`, y `rustls-webpki 0.102` acarrea cuatro avisos de RustSec que ya están corregidos en upstream a partir de `>= 0.103.13`. Un crate abandonado retenía todo el árbol, sin ninguna versión de "esperar a upstream" que fuera a terminar. Pinecone expone por HTTPS cada operación que este driver necesita, así que la ruta REST eliminó cuatro avisos y dos dependencias de una sola vez.

```rust
use suprnova::PineconeVectorDriver;

// Clave de API directamente
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// O vía entorno: PINECONE_API_KEY, más opcionalmente PINECONE_CONTROLLER_HOST
// y PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// Vincular a un namespace distinto del predeterminado
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

El nombre de store que se pasa vía `Vector::store(name)` se mapea a un nombre de índice de Pinecone. El driver resuelve el host de ese índice de forma perezosa en el primer uso, vía `GET /indexes/{name}` del plano de control, y luego lo cachea. Evita el viaje de ida y vuelta fijando el host que ya conoces:

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

Un host aprendido del plano de control siempre se contacta por `https`, sea lo que sea que diga la respuesta. Un host fijado con `with_index_host` mantiene el esquema que le diste, así que un emulador local en `http://` funciona.

**Versión de la API.** Pinecone versiona su API REST por fecha y quiere esa versión fijada en un encabezado. El driver fija `2025-04` - la versión contra la que se escribieron y probaron sus formas de solicitud y respuesta - y expone `with_api_version` (o `PINECONE_API_VERSION`) para moverse de forma deliberada. No flota: la convención de clave de namespace en `describe_index_stats` es una de las cosas que ha cambiado entre versiones, y `count()` lee ese mapa.

**Sin auto-creación.** Crear un índice en Pinecone exige elegir nube (AWS/GCP/Azure), región, dimensión del vector, métrica de distancia, y protección contra borrado - demasiadas decisiones para tener un valor por defecto razonable. Crea los índices desde la consola de Pinecone, la CLI de Pinecone, o una llamada a `control_plane_post` antes de registrar, y luego apunta el framework al nombre ya existente.

Esta es la principal asimetría con el driver de Qdrant, que crea las colecciones automáticamente en el primer `upsert`.

**IDs y metadatos.** Pinecone acepta IDs `String` arbitrarios de forma nativa, así que `VectorItem::id` pasa directo. Los metadatos se transportan como JSON de punta a punta - `PineconeVectorDriver::metadata_from_json` / `metadata_to_json` solo hacen cumplir la propia regla del framework de que los metadatos son un objeto o `null`. Pinecone en sí restringe los *valores* de metadatos a cadenas, números, booleanos y listas de cadenas, y rechaza objetos anidados en el lado del servidor; el driver no reimplementa esa comprobación, porque las reglas de Pinecone están versionadas y una copia local se desviaría con el tiempo.

**Límites de lote.** Pinecone documenta un máximo de 1000 vectores por `upsert` y 1000 ids por `delete`. El driver envía lo que le das en una sola solicitud en lugar de dividirlo en silencio - una escritura con éxito parcial es más difícil de razonar que una rechazada. Divide tú mismo en lotes si superas esos límites.

**Namespaces.** Una instancia de driver se vincula a un solo namespace. Para usar varios namespaces del mismo índice, registra un driver por namespace bajo nombres de store distintos:

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**Throughput.** Nada se serializa. El driver cachea una cadena de host por índice, no un handle de conexión, y las solicitudes comparten el pool de conexiones de `reqwest` - así que las llamadas concurrentes al mismo índice avanzan de forma concurrente. (El driver por gRPC que este reemplaza mantenía un `Index` por nombre detrás de un `tokio::Mutex`, porque `pinecone-sdk` solo exponía `Index` detrás de `&mut self`.)

**Vía de escape.** `control_plane_get`, `control_plane_post` y `data_plane_post` alcanzan cualquier endpoint que Pinecone ofrezca, con tus propios tipos de solicitud y respuesta, sobre el transporte autenticado y con el host ya resuelto del driver - expresiones de filtro, vectores dispersos, fetch por id, `/vectors/list`, gestión de índices:

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**Tests.** Los tests de contrato de red se ejecutan por defecto bajo la feature: dirigen el driver contra un fake local y verifican el método, la ruta, los encabezados y el cuerpo JSON exactos que envía por la red. Esos fijan el driver al contrato *documentado* de Pinecone. Confirmar que la documentación coincide con el servicio real necesita los tests de integración marcados `#[ignore]`, que exigen ambas variables de entorno:

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

Habla con MariaDB 11.7+ vía `sqlx::MySqlPool` directo, usando el tipo de columna nativo `VECTOR(N)` de MariaDB e indexación HNSW. La primera vez que llamas a un método del driver, este ejecuta `SELECT VERSION()` y rechaza cualquier versión por debajo de 11.7 - los servidores más antiguos no tienen las funciones de vector.

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // por defecto

Vector::register("documents", Arc::new(driver));
```

`from_url` es perezoso - valida la sintaxis de la URL, pero NO abre una conexión hasta el primer uso, así que llamarlo en el arranque de la app es seguro incluso antes de que la base de datos sea alcanzable. Envuelve un pool ya existente con `MariaDbVectorDriver::from_pool(pool)` cuando necesites opciones de pool personalizadas.

**El esquema es tuyo.** El driver no crea tablas automáticamente - el esquema es un asunto de migraciones. La ruta recomendada es `driver.ensure_table_sql_for(name, dim)`, que hereda la distancia configurada del driver, de modo que la cláusula `DISTANCE=` de la migración y la función de consulta que usa `similar` están garantizadas a coincidir:

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

Para generadores de migraciones que no tienen un driver a mano (herramientas de CLI, scripts de build), usa el método estático `MariaDbVectorDriver::ensure_table_sql(name, dim, distance)` y pásale la misma `MariaDbDistance` que luego configurarás en el driver.

**La distancia debe coincidir en ambos extremos.** MariaDB recurre en silencio a un escaneo completo de tabla cuando la función usada en el momento de la consulta no coincide con la cláusula `DISTANCE=` del índice. El driver se protege contra esto en dos capas:

1. **`ensure_table_sql_for(name, dim)`** lee `self.distance` tanto para el SQL de migración emitido como para la función en tiempo de ejecución usada en `similar` - por construcción, no pueden desalinearse.
2. **Una comprobación en tiempo de ejecución en la primera llamada a `similar`** ejecuta un `SHOW CREATE TABLE` por store, analiza la cláusula `DISTANCE=` real del esquema en vivo, y falla con un error claro si no coincide con `with_distance(...)`. El resultado se cachea, así que las llamadas siguientes son gratis. Esto detecta migraciones escritas a mano o configuraciones con `from_pool` que se saltan `ensure_table_sql_for`.

**Seguridad del nombre de store.** Los nombres de store se interpolan en el SQL emitido (MySQL no parametriza identificadores). Los nombres se validan como `[A-Za-z_][A-Za-z0-9_]*` de longitud ≤ 64; el nombre validado se entrecomilla luego con backtick en cada instrucción. Los nombres inválidos fallan con `FrameworkError::param` en la frontera de `register`/`upsert`/`similar`/`delete`/`count`.

**IDs y metadatos.** `VARCHAR(255)` acepta IDs `String` arbitrarios - sin derivación de UUID, sin claves de payload reservadas. Los metadatos hacen el viaje de ida y vuelta a través del tipo de columna `JSON` de MariaDB; los metadatos `null` se guardan como `NULL` de SQL. Los metadatos que no son un objeto (arrays, primitivos) se rechazan con `FrameworkError::param`, en paridad con Qdrant y Pinecone.

**Normalización de la puntuación.** MariaDB devuelve una *distancia* en crudo (menor = más cercano). El contrato del trait es una *puntuación* (mayor = más similar) - el driver convierte según la métrica:

| Métrica    | MariaDB devuelve       | `score` expuesta              |
| --------- | --------------------- | ---------------------------- |
| Coseno    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`   |
| Euclidiana | `[0, ∞)` norma L2      | `1.0 / (1.0 + d)` → `(0, 1]` |

En ambos casos se conserva el orden (el mejor resultado primero), pero los valores absolutos de puntuación NO son comparables entre drivers - solo el orden lo es. Cada backend adopta la convención `mayor = mejor`, pero los rangos difieren: la similitud de coseno de Memory devuelve `[-1, 1]`, la de coseno normalizada de MariaDB devuelve `[0, 1]`, Qdrant emite su similitud de coseno nativa en `[-1, 1]`, y Pinecone devuelve la similitud en crudo, según la métrica con la que se creó el índice. Usa `score` para ordenar dentro del conjunto de resultados de un solo driver; no compares puntuaciones numéricas entre drivers sin renormalizarlas tú mismo.

**Vía de escape.** `driver.pool()` devuelve el `sqlx::MySqlPool` subyacente para las consultas en crudo que el trait no cubre. `MariaDbVectorDriver::embedding_to_vec_text`, `score_from_distance`, y `ensure_table_sql` son funciones puras que puedes llamar de forma independiente al mezclar SQL directo con llamadas enrutadas por el trait.

**Comportamiento del `upsert` masivo.** `upsert` emite una sola instrucción multifila `INSERT ... VALUES (...), (...), ...` por bloque de 500 filas, todo envuelto en una única transacción. Los viajes de ida y vuelta por red caen ~500 veces frente a inserciones fila por fila al cargar un corpus nuevo; la llamada sigue siendo atómica en todo el lote. El tamaño del bloque es interno - llama a `upsert` una vez con todos tus elementos y el driver se encarga de dividirlo.

**Los índices HNSW se reconstruyen en el momento del commit.** MariaDB actualiza el grafo HNSW a medida que entran filas, pero el trabajo de indexación se concentra en el commit. Un `upsert` de 1M de filas mantendrá la transacción abierta durante toda la construcción del índice, lo que puede tomar minutos. Para cargas iniciales muy grandes, divide el corpus en lotes de 10k-100k filas y llama a `upsert` repetidamente, para que cada lote haga commit y libere el bloqueo entre rondas. (Las llamadas a `upsert` más pequeñas no son más lentas por fila - solo reparten el trabajo de indexación en más puntos de commit.)

**La dimensión queda fijada en la creación de la tabla.** `VECTOR(N)` fija la dimensión; cambiar de modelo de embeddings, de uno de 768 dimensiones a uno de 1536, exige una migración completa de tabla (tabla nueva, reembeber, y hacer el cambio). Planifica las actualizaciones de modelo igual que planificarías una migración de esquema - no existe una ruta "ALTER COLUMN VECTOR(768) → VECTOR(1536)".

**Tamaño del pool.** `from_url` usa el `MySqlPoolOptions` por defecto de sqlx - `max_connections = 10` al momento de escribir esto. Para cargas de alto QPS (cientos de llamadas a `similar` por segundo), construye el pool tú mismo con `MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` y pásalo a `from_pool`. El driver no impone su propio tope de conexiones.

**Configuración local.** Ejecuta MariaDB 11.7+ con Docker:

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

Los tests de integración se ejecutan con:

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## Comparación de drivers

| Aspecto | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| Store de respaldo | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| Persistencia | Ninguna | Sí | Sí | Sí |
| Auto-creación | n/a | Sí (configurable) | No (el usuario crea el índice) | No (la migración es tuya) |
| IDs de cadena | Nativos | Con hash a UUID-5 | Nativos | Nativos |
| Clave de metadatos reservada | Ninguna | `__suprnova_id` | Ninguna | Ninguna |
| Throughput | Por proceso | Concurrente | Concurrente (acotado por pool) | Concurrente (acotado por pool) |
| Métrica de distancia | Coseno | Configurable | Fijada en la creación del índice | Coseno / Euclidiana |
| Requisito de versión | - | Cualquiera | Cualquiera | **11.7+** |

## Notas operativas

**Convenciones de nombre de store.** El nombre de store que se pasa a `Vector::register` y `Vector::store` es una etiqueta - puede ser cualquier cadena. Para Qdrant el framework lo usa como nombre de colección; para Pinecone, como nombre de índice. Haz coincidir la etiqueta con el esquema de nombres que ya use el backend.

**Volver a registrar** un nombre con una nueva instancia de driver es, por diseño, una operación en la que gana la última escritura - útil para intercambiar drivers en harnesses de test sin reiniciar el proceso.

**Aislamiento de tests.** Tanto los tests de Memory como los de drivers respaldados por un registro usan nombres de store únicos marcados con timestamp para evitar colisiones bajo ejecuciones de test en paralelo.

**Semántica de error.** `Vector::store(name)` devuelve `FrameworkError::not_found` para nombres no registrados. Los fallos a nivel de driver (red, autenticación, dimensión no coincidente) vuelven como `FrameworkError::internal` o `FrameworkError::param`, con la cadena de causa en el mensaje que se muestra.

## Extendiendo

Para añadir un quinto backend (Weaviate, Milvus, LanceDB, pgvector, LibSQL, ...):

1. Añade un nuevo `framework/src/vector/<backend>.rs` que implemente `VectorDriver`.
2. Reexporta el tipo del driver desde `framework/src/vector/mod.rs` y desde la raíz del crate.
3. Refleja la división de tests de Pinecone: los tests de funciones puras y los tests de contrato de red (contra un fake local de `wiremock`) siempre se ejecutan; los tests de integración están marcados `#[ignore]` detrás de variables de entorno para las credenciales. La capa intermedia es la que se gana su lugar - un backend al que nadie puede llegar desde CI aun así tiene un formato de red que un error de tipeo puede romper.

El trait es deliberadamente pequeño para que el listón para lanzar un nuevo driver siga siendo bajo. Si un backend necesita una superficie que no encaja (expresiones de filtro, vectores dispersos, búsqueda híbrida), expónla a través de una vía de escape en el driver - no infles el trait.

## Siguiente

- [Despliegue](deployment.md) - la recomendación de MariaDB como
  opción por defecto en producción, en contexto
- [Base de datos](database.md) - configuración multi-driver de
  SeaORM, incluida MariaDB como backend relacional junto a los
  vectores
- [Variables de entorno](env-vars.md) - `QDRANT_URL`,
  `PINECONE_API_KEY`, `MARIADB_URL` y otros contratos de entorno de
  drivers
- [Caché](cache.md) - fachada hermana con la misma forma de
  trait-driver
- [Mapa de paridad con Laravel](parity.md) - dónde se ubica la
  búsqueda vectorial respecto a Scout
