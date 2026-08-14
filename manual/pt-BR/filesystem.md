# Sistema de arquivos e armazenamento

A facade de armazenamento do Suprnova te dá uma única API de disco nomeado
sobre sistemas de arquivos locais, backends em memória, e os principais
serviços de armazenamento de objetos (S3, Azure Blob, Google Cloud
Storage). Por baixo dos panos ela é construída sobre
[`opendal`](https://docs.rs/opendal) - mas a superfície voltada ao
consumidor é moldada para corresponder às chamadas `Storage::disk(...)` do
Laravel, então a memória muscular de PHP se traduz direto.

```rust,no_run
use suprnova::{DiskExt, Storage};

# async fn doc() -> Result<(), suprnova::FrameworkError> {
Storage::register_fs("local", "./storage")?;
let disk = Storage::disk("local")?;

disk.put("notes/hello.txt", b"hello world".to_vec()).await?;
let bytes = disk.get("notes/hello.txt").await?;
assert_eq!(bytes, b"hello world");
# Ok(())
# }
```

## Registrando discos

Todo disco é registrado uma vez na inicialização via `Storage::register_*`
e localizado por nome através de `Storage::disk(name)`. Não existe um
"backend padrão" para o qual os outros degradam - cada driver é um par.

| Construtor                           | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Sistema de arquivos local     | `filesystem`        |
| `Storage::register_memory(name)`     | Memória em processo (testes)  | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 ou compatível com S3 | `filesystem`       |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |

`filesystem` vem ativada por padrão; as features de Azure e GCS não vêm.
Ative uma no seu `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.2", features = ["filesystem-gcs"] }
```

Sem a feature, `register_azblob` / `register_gcs` e seus structs de
configuração não existem - você recebe um erro de compilação nomeando o
item ausente, não uma falha em tempo de execução.

Todo construtor tem uma variante `_with` que te entrega o
`suprnova::opendal::Operator` pouco antes de ele chegar ao registro, para
que você possa instalar camadas de retry/timeout/logging em torno dele:

```rust,ignore
use std::time::Duration;
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
use suprnova::Storage;

Storage::register_fs_with("local", "./storage", |op| {
    op.layer(RetryLayer::new().with_max_times(3))
      .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(30)))
      .layer(LoggingLayer::default())
})?;
```

Os construtores de nuvem (`register_s3`, `register_azblob`, `register_gcs`)
aplicam uma `RetryLayer` (3 tentativas) por padrão, já que throttling
transitório / erros 5xx são rotina em serviços de armazenamento de
objetos. Use as variantes `_with` quando precisar de controle total.

O conjunto completo de camadas opendal já ligadas pelo Suprnova é
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (faz a ponte
para o OTel via `tracing-opentelemetry` quando a feature `otel` do
framework está ativa), e `PrometheusClientLayer` (exporta histogramas e
contadores para um `prometheus_client::registry::Registry` que você
possui). A ordem das camadas importa - a camada mais externa envolve tudo
que está dentro dela - e a pilha idiomática é `RetryLayer → TimeoutLayer
→ LoggingLayer`, para que uma tentativa que sofreu timeout ainda seja
registrada em log e um retry cubra falhas de transporte.

Registrar de novo o mesmo nome substitui o operador anterior e emite um
log `warn!` - discos devem ser registrados uma única vez na
inicialização, e uma duplicata acidental poderia trocar um disco de
produção por um em memória. A substituição acontece mesmo assim; o aviso
apenas torna a troca audível.

### Por que Suprnova diverge

O `config/filesystems.php` do Laravel lista todo driver de disco e você
escolhe um em tempo de execução; nada é compilado fora. O Suprnova
condiciona Azure e GCS a features porque em Rust a escolha tem um custo
de dependência, e esse caso tem uma dimensão de segurança: ambos os
crates de serviço do opendal trazem `rsa`, que carrega
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) (o
ataque de timing Marvin) sem release corrigido a montante. Torná-los
opt-in significa que um app que guarda arquivos localmente ou no S3 nunca
carrega esse crate.

O S3 deliberadamente *não* é condicionado - seu signer nunca dependeu de
`rsa`, então condicioná-lo quebraria o backend de nuvem mais usado e não
removeria nada.

### Proteção contra path traversal

Discos de sistema de arquivos local têm uma `PathGuardLayer` aplicada
antes de qualquer camada fornecida pelo usuário. Uma solicitação como
`disk.write("../escaped.txt", ..)` é rejeitada antes de alcançar o SO -
nenhum componente `..` ou prefixo absoluto consegue escapar da raiz do
disco. Serviços de armazenamento de objetos e o backend em memória não
recebem a proteção (uma chave como `../foo` é apenas um caractere comum
de chave nesses backends).

Depois de rejeitar componentes `..` e absolutos, a proteção canonicaliza
a raiz do disco local e o destino solicitado no disco. Alvos existentes
resolvem cada componente de link simbólico; para um caminho que ainda não
existe, a proteção sobe até o ancestral existente mais próximo e o
canonicaliza. A operação é rejeitada se esse caminho resolvido estiver
fora da raiz canônica, então um link simbólico dentro da raiz observado
durante a validação não consegue redirecionar uma leitura, escrita,
listagem, cópia ou renomeação para fora do disco.

Essa é uma proteção do tipo canonicalizar-depois-operar, não confinamento
de sistema de arquivos relativo a descritor. Ela assume que a raiz do
disco e seu conteúdo são confiáveis contra mutação concorrente: um
atacante que consiga substituir diretórios ou links simbólicos depois da
validação mas antes de o backend abrir o caminho pode ganhar uma corrida
de tempo-de-verificação para tempo-de-uso. Use isolamento no nível do SO
ou um sistema de arquivos dedicado quando outros agentes puderem mutar a
árvore de armazenamento concorrentemente.

Escritores, listadores e copiadores em streaming executam essa
verificação de caminho resolvido uma única vez, imediatamente antes de
sua primeira I/O no backend. A validação então fica fixa para aquela
sessão de stream, para que cada chunk ou item não bloqueie na
canonicalização do sistema de arquivos. Abortos de copiador e de escritor
sempre repassam a limpeza para seus backends, mesmo antes da ativação ou
quando a validação não pode mais ser concluída.

## A superfície de disco no estilo Laravel

`Storage::disk(name)` retorna um `suprnova::opendal::Operator`
diretamente, para que você use sua superfície completa de streaming
(`writer`, `reader`, `presign_read`, `list`, `stat`, ...). Além disso, o
trait [`DiskExt`] - implementado de forma abrangente sobre `Operator` e
reexportado como `suprnova::DiskExt` - adiciona todo método de
conveniência do Laravel que você buscaria através de
`Storage::disk('local')->...`.

Traga-o para o escopo com `use suprnova::DiskExt;`.

### Verificações de existência

```rust,ignore
disk.exists("a.txt").await?;        // opendal bruto
disk.missing("a.txt").await?;       // negação
disk.file_exists("a.txt").await?;   // somente arquivo (não um diretório)
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### Leitura e escrita

| Nome no Laravel | Equivalente nativo em Rust | Nota |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` retorna `Vec<u8>`; `read` retorna o `Buffer` do opendal. |
| `put(path, contents)` | `write(path, contents)` | Ambos aceitam qualquer `Into<Bytes>`. |
| `json::<T>(path)` | - | Lê + desserializa via serde_json. |
| `put_json(path, &value)` | - | Formata com indentação via serde_json. |
| `prepend(path, data)` | - | Junta com `\n`. Use `prepend_with_separator` para uma junção customizada. |
| `append(path, data)`  | - | Junta com `\n`. Use `append_with_separator` para uma junção customizada. |

`prepend` e `append` criam o arquivo se ele ainda não existir, então são
seguros como a primeira escrita em um arquivo de log.

### Metadados

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` primeiro pergunta ao backend - S3, Azure e GCS repassam o
`Content-Type` armazenado. Se o backend não tiver um, ele fareja os
primeiros 16 KiB via o crate `infer`. `Ok(None)` é reservado para blobs
binários não reconhecidos.

`checksum` suporta `Md5`, `Sha1` e `Sha256` via [`ChecksumAlgorithm`]. MD5
e SHA-1 estão incluídos por paridade com o Laravel e com ETags de
armazenamento de objetos; escolha SHA-256 para qualquer nova verificação
de integridade.

### Listagem

```rust,ignore
let files = disk.files("docs", false).await?;     // arquivos de nível superior
let all   = disk.all_files("docs").await?;        // recursivo
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

As quatro retornam `Vec<String>` ordenados, para que quem chama possa
contar com ordenação estável entre backends. Diretórios são filtrados de
fora de `files`, e vice-versa. Caminhos de diretório são retornados
**sem** uma barra final (`"docs/sub"`) para corresponder à saída de
`Storage::directories()` do Laravel - o `list` subjacente do opendal
relata `"docs/sub/"`, mas removemos a barra por paridade.

### Modificando diretórios e arquivos

| Nome no Laravel        | opendal nativo         |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to` recorre a `copy + delete` se o backend não suportar rename, e a
`read + write + delete` se também não suportar copy - então funciona
contra o driver em memória usado em testes tanto quanto contra backends
de produção.

### URLs pré-assinadas

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url` e `temporary_upload_url` retornam a URL como `String` por
paridade com o Laravel. Elas são respaldadas por `Operator::presign_read`
/ `presign_write`, então erram com uma mensagem `Unsupported` em backends
que não implementam presigning (os drivers em memória e de sistema de
arquivos local caem nesse balde; S3, Azure Blob e GCS o suportam).

## Cópia via streaming entre discos

`copy_between_disks(src, src_path, dest, dest_path)` transmite o objeto
de origem para o destino em chunks de 64 KiB, independentemente do par de
backends. Origem e destino podem ser respaldados por *qualquer* driver
opendal - sistema de arquivos local para S3, S3 para Azure Blob, memória
para GCS, e assim por diante.

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

Se qualquer etapa falhar no meio da cópia, o objeto de destino parcial é
abortado e apagado antes que o erro original se propague - uma cópia com
falha nunca é observável como um destino truncado.

## Higiene do registro

```rust,ignore
let removed = Storage::forget("local");  // bool: estava presente?
Storage::purge();                        // descarta todo disco
let names = Storage::disks();            // Vec<String>, ordenado
```

Estas espelham `FilesystemManager::forgetDisk` / `purge` do Laravel e são
úteis para recargas de configuração e painéis administrativos. Não são
exclusivas de teste: código de produção ocasionalmente precisa descartar
e registrar de novo um disco em tempo de execução (por exemplo, depois de
uma rotação de segredos).

## Testes

`Storage::fake()` retorna uma guarda que:

1. Adquire um mutex global de processo para que casos `#[tokio::test]`
   concorrentes não corram entre si sobre o registro compartilhado, e
2. Reseta o registro na construção e no drop, deixando a suíte em um
   estado limpo para qualquer teste que rode a seguir.

Um disco de memória `"default"` é pré-registrado por conveniência.

```rust,ignore
use suprnova::filesystem::testing::DiskAssertExt;
use suprnova::{DiskExt, Storage};

#[tokio::test]
async fn stores_and_asserts() {
    let _guard = Storage::fake();
    Storage::register_memory("uploads");
    let disk = Storage::disk("uploads").unwrap();

    disk.put("a.txt", b"hello".to_vec()).await.unwrap();

    disk.assert_exists("a.txt").await;
    disk.assert_contents("a.txt", b"hello").await;
    disk.assert_missing("not-here.txt").await;
    disk.assert_count("", 1, false).await;
    disk.assert_directory_empty("docs/").await;
}
```

Os cinco helpers de asserção - `assert_exists`, `assert_contents`,
`assert_missing`, `assert_count`, `assert_directory_empty` - são
expostos via o trait [`DiskAssertExt`], condicionado a
`#[cfg(any(test, feature = "testing"))]`, para que código de produção não
consiga alcançá-los.

## Referência rápida de paridade

| `Storage::disk(...)->...` no Laravel  | Suprnova                                                 |
|---------------------------------------|----------------------------------------------------------|
| `exists($path)`                       | `disk.exists(path)`                                      |
| `missing($path)`                      | `disk.missing(path)`                                     |
| `fileExists($path)` / `fileMissing`   | `disk.file_exists(path)` / `file_missing(path)`          |
| `directoryExists($p)` / `directoryMissing` | `disk.directory_exists(p)` / `directory_missing(p)` |
| `get($path)`                          | `disk.get(path)` (`Vec<u8>`)                             |
| `json($path)`                         | `disk.json::<T>(path)`                                   |
| `put($path, $contents)`               | `disk.put(path, bytes)`                                  |
| `prepend($path, $data)`               | `disk.prepend(path, data)`                               |
| `append($path, $data)`                | `disk.append(path, data)`                                |
| `size($path)`                         | `disk.size(path)`                                        |
| `lastModified($path)`                 | `disk.last_modified(path)`                               |
| `mimeType($path)`                     | `disk.mime_type(path)`                                   |
| `checksum($path, ['checksum_algo' => 'sha256'])` | `disk.checksum(path, ChecksumAlgorithm::Sha256)` |
| `files($dir, $recursive)`             | `disk.files(dir, recursive)`                             |
| `allFiles($dir)`                      | `disk.all_files(dir)`                                    |
| `directories($dir, $recursive)`       | `disk.directories(dir, recursive)`                       |
| `allDirectories($dir)`                | `disk.all_directories(dir)`                              |
| `makeDirectory($path)`                | `disk.make_directory(path)`                              |
| `deleteDirectory($path)`              | `disk.delete_directory(path)`                            |
| `move($from, $to)`                    | `disk.move_to(from, to)` (ou `rename` nativo do opendal) |
| `copy($from, $to)`                    | `disk.copy(from, to)` (nativo do opendal)                |
| `delete($path)`                       | `disk.delete(path)` (nativo do opendal)                  |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (ou `presign_read` nativo do opendal) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (ou `presign_write` nativo do opendal) |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## Configuração

A configuração de armazenamento vive inteiramente em código Rust, não no
`.env`. Discos são registrados por nome em `bootstrap()` via
`Storage::register_*` e endereçados por nome no ponto de chamada
(`Storage::disk("public")`). Não existe uma env var `FILESYSTEM_DISK` que
o framework leia, nem um disco padrão implícito - cada driver é um par.
Os apps decidem qual nome de disco um dado upload ou download tem como
alvo, e passam quaisquer URLs / chaves / credenciais que o driver
escolhido precisar como suas próprias env vars.

Veja [Configuração](configuration.md) para a regra mais ampla sobre onde
o framework lê a partir do ambiente versus onde ele espera registro do
lado do código.

## Próximos passos

- [Configuração](configuration.md) - o que o framework lê do `.env`
  (e por que o armazenamento não está nessa lista)
- [Solicitações](requests.md) - uploads de arquivo chegam a um disco via
  `UploadedFile::store_as`
- [Respostas](responses.md) - transmitindo bytes de volta a partir de um
  disco
- [Cache](cache.md) - o outro registro de driver nomeado, mesma forma
- [Testes](testing.md) - a superfície mais ampla de testes com fakes

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html
