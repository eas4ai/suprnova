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

Todo disco é registrado uma vez no boot via `Storage::register_*` e
localizado pelo nome através de `Storage::disk(name)`. Não existe um
"backend padrão" no qual os outros degradam - cada driver é um par dos
demais.

| Construtor                           | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Sistema de arquivos local     | `filesystem`        |
| `Storage::register_memory(name)`     | Memória no processo (testes)  | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 ou compatível com S3| `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | Composto read-through | `filesystem` |

`filesystem` vem ligada por padrão; as features de Azure e GCS não.
Ligue uma no seu `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.6", features = ["filesystem-gcs"] }
```

Sem a feature, `register_azblob` / `register_gcs` e suas structs de
config não existem - você recebe um erro de compilação nomeando o item
ausente, não uma falha em runtime.

Todo construtor tem uma variante `_with` que te entrega o `suprnova::opendal::Operator`
logo antes de ele chegar ao registro, para que você possa instalar
camadas de retry/timeout/logging ao redor dele:

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

Os construtores de nuvem (`register_s3`, `register_azblob`,
`register_gcs`) aplicam um `RetryLayer` (3 tentativas) por padrão, já
que throttling transitório / erros 5xx são rotina em object stores. Use
as variantes `_with` quando você precisar de controle total.

O conjunto completo de camadas do opendal conectadas pelo Suprnova é
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (faz ponte
para o OTel via `tracing-opentelemetry` quando a feature `otel` do
framework está ligada), e `PrometheusClientLayer` (exporta histogramas
e contadores para um `prometheus_client::registry::Registry` que é
seu). A ordem das camadas importa - a mais externa envolve tudo o que
está dentro dela - e a pilha idiomática é
`RetryLayer → TimeoutLayer → LoggingLayer`, para que uma tentativa que
estourou o timeout ainda registre log e um retry cubra falhas de
transporte.

Registrar o mesmo nome de novo substitui o operator anterior e emite um
log `warn!` - discos devem ser registrados uma vez no boot, e uma
duplicata acidental poderia trocar um disco de produção por um de
memória. A substituição acontece de qualquer forma; o aviso apenas
torna a troca perceptível.

### Por que Suprnova diverge

O `config/filesystems.php` do Laravel lista todo driver de disco e você
escolhe um em runtime; nada é removido da compilação. O Suprnova
condiciona Azure e GCS a features porque em Rust a escolha tem um custo
de dependência, e esta tem uma dimensão de segurança: os dois crates de
serviço do opendal puxam `rsa`, que carrega o
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)
(o ataque de timing Marvin) sem release corrigido upstream. Torná-los
opt-in significa que um app que armazena arquivos localmente ou no S3
nunca carrega esse crate.

O S3 deliberadamente *não* é condicionado - seu signer nunca dependeu
de `rsa`, então condicioná-lo quebraria o backend de nuvem mais usado e
não removeria nada.

### Escritas locais atômicas

Em um disco local, toda operação que publica bytes em um caminho os
publica em um único passo. `disk.write(...)`, `disk.writer(...)` e
`disk.copy(...)` aterrissam primeiro em `<root>/.suprnova-atomic/`, recebem
flush e sync ali, e só então são renomeados para o alvo;
`disk.rename(...)` já é um único passo. Um leitor concorrente, portanto,
vê ou o objeto anterior ou o novo já pronto, e nunca um tamanho parcial, e
um processo que morre no meio de uma escrita deixa o alvo intacto em vez
de truncado no caminho ativo.

O `append` é a única operação feita no lugar, porque preparar um append
significaria copiar o objeto inteiro antes. Isso vale para o append que
*cria* o objeto tanto quanto para todo append depois dele, então dois
writers dando append no mesmo objeto novo aterrissam os dois. Ser feito no
lugar é também o que um append custa a você: um append que falha ou é
abortado deixa o objeto para trás, vazio ou curto, exatamente como um
append sobre um objeto existente sempre deixou.

Uma escrita condicional é publicada com `link(2)` em vez de um rename, o
que a mantém sendo uma criação exclusiva de verdade em vez de uma
checagem seguida de uma sobrescrita:

```rust,ignore
// Exatamente um entre qualquer número de chamadores concorrentes recebe
// Ok aqui. Todos os outros recebem um erro `ErrorKind::ConditionNotMatch`
// e não escrevem nada.
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

Essa publicação precisa de um sistema de arquivos com hard links. Em FAT,
exFAT e alguns sistemas de arquivos de rede o `link(2)` não é suportado, e
uma escrita condicional falha ali em vez de degradar silenciosamente para
uma checagem seguida de uma sobrescrita - o que te entregaria uma garantia
de exclusividade que não se sustenta. Toda outra operação fica inalterada.

Publicar por rename substitui o inode do objeto. Uma reescrita, portanto,
não preserva o modo, o dono nem os hard links do arquivo anterior, e um
leitor que segura um descritor aberto continua lendo o conteúdo antigo em
vez de ver os bytes novos. Esse é o custo de sempre da publicação
atômica, mas é uma mudança se você contava com qualquer um dos dois.

Um caminho que chega ao disco por um symlink que a guarda não consegue
resolver - um symlink quebrado, cujo alvo não existe - é recusado, em vez
de tratado como um nome livre para criar. Criar através de um link desses
criaria o alvo do link, em qualquer lugar do host, então a guarda não tem
como distinguir um symlink quebrado inofensivo de um escape e recusa os
dois.

O nome `.suprnova-atomic` é reservado na raiz de todo disco local.
Qualquer caminho cujo primeiro componente seja esse nome é recusado com um
erro de permissão, e o mesmo vale para qualquer caminho que *resolva* para
dentro do diretório por um symlink, então você não consegue ler o arquivo
de preparo de outro writer, escrever dentro do diretório nem excluí-lo. A
entrada é filtrada para fora de `files`, `directories`, `all_files` e
`all_directories`, então ela nunca aparece como um objeto. O nome é
exportado como `suprnova::ATOMIC_STAGING_DIR` porque ferramentas de backup
e de sincronização precisam dele: deixe o diretório de fora do mesmo jeito
que você deixaria de fora um diretório de lock. Ele guarda arquivos
temporários em voo mais o que um processo que morreu no meio de uma
publicação deixou para trás, e nada varre esses restos, então um host em
crash loop vai fazê-lo crescer até alguém esvaziá-lo - o que é seguro
fazer enquanto nada está escrevendo.

### Guarda de path traversal

Discos de sistema de arquivos local têm um `PathGuardLayer` aplicado
antes de quaisquer camadas fornecidas por você. Uma solicitação como
`disk.write("../escaped.txt", ..)` é rejeitada antes de chegar ao OS -
nenhum componente `..` ou prefixo absoluto consegue escapar da raiz do
disco. Object stores e o backend em memória não recebem a guarda (uma
chave como `../foo` é apenas um caractere de chave comum nesses
backends).

Depois de rejeitar componentes `..` e absolutos, a guarda canonicaliza
a raiz do disco local e o alvo em disco solicitado. Alvos existentes
resolvem todo componente de symlink; para um path que ainda não existe,
a guarda caminha até o ancestral existente mais próximo e o
canonicaliza. A operação é rejeitada se esse path resolvido ficar fora
da raiz canônica, então um symlink dentro da raiz observado durante a
validação não consegue redirecionar uma leitura, escrita, listagem,
cópia, ou renomeação para fora do disco.

Esta é uma guarda de canonicalize-então-opere, não um confinamento de
filesystem relativo a descritor. Ela assume que a raiz do disco e seu
conteúdo são confiáveis contra mutação concorrente: um atacante que
consiga substituir diretórios ou symlinks depois da validação, mas
antes de o backend abrir o path, pode vencer uma corrida TOCTOU
(time-of-check para time-of-use). Use isolamento no nível do OS ou um
filesystem dedicado quando outros principais podem mutar a árvore de
armazenamento concorrentemente.

Writers, listers e copiers de streaming realizam essa checagem de path
resolvido uma vez, imediatamente antes do seu primeiro I/O de backend.
A validação então fica fixa para aquela sessão de stream, de modo que
cada chunk ou item não bloqueia em canonicalização de filesystem.
Aborts de copier e de writer sempre encaminham a limpeza para seus
backends, mesmo antes da ativação ou quando a validação não pode mais
ser completada.

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

## Discos read-through

Um disco read-through combina um *primário* rápido com um *fallback* mais
lento e move objetos do segundo para o primeiro conforme eles são lidos.
Aponte o primário para o store para o qual você está migrando e o fallback
para aquele do qual você está migrando, e o conjunto de trabalho atravessa
sob tráfego real - sem janela de manutenção, sem cópia em massa de objetos
que ninguém pede.

```rust,ignore
use suprnova::{ReadThroughConfig, S3Config, Storage};

Storage::register_s3("new-store", S3Config { bucket: "assets-2".into(), ..Default::default() })?;
Storage::register_s3("legacy-store", S3Config { bucket: "assets-1".into(), ..Default::default() })?;

Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        ..Default::default()
    },
)?;

let assets = Storage::disk("assets")?;
// Lê `logo.png` do `legacy-store` e o escreve no `new-store` no caminho de
// saída. Toda leitura posterior é servida pelo `new-store`.
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` retorna um `Operator` comum, então todo método
dele e toda conveniência de `DiskExt` funcionam sem mudança.

### Qual disco responde a qual operação

| Operação | Disco |
|---|---|
| `read` | O primário, se ele tiver o objeto; caso contrário o fallback - e, a menos que `copy` seja `false`, o acerto no fallback é promovido |
| `exists`, `size`, `last_modified`, `mime_type`, `stat` | O primário, se ele tiver o objeto; caso contrário o fallback |
| `write`, `make_directory` | Somente o primário |
| `files`, `directories`, `list` | Somente o primário - entradas do fallback são invisíveis para uma listagem |
| `delete` | Os dois, o fallback primeiro |
| `copy`, `rename` / `move_to` | O primário, se ele tiver a origem; caso contrário o objeto é transmitido a partir do fallback; um `rename` também exclui a origem no fallback |
| `temporary_url` | O primário, se ele tiver o objeto; caso contrário o fallback |
| `temporary_upload_url` | Somente o primário - um upload tem que aterrissar onde as escritas aterrissam |

A listagem é somente do primário por design. Uma listagem unificada teria
de reconciliar paginação e ordenação entre dois backends, e reportaria
objetos que uma listagem posterior não devolve mais, uma vez promovidos.
Use `Storage::disk("legacy-store")` diretamente quando você precisar
enumerar o que sobrou no fallback.

O delete remove o objeto dos dois discos. Se ele removesse apenas a cópia
do primário, a próxima leitura promoveria a cópia do fallback de volta na
hora. A consequência é que um disco read-through sobre um fallback somente
leitura não consegue excluir: o delete no fallback falha e o erro chega
até você.

### Quando uma promoção falha

Por padrão, uma falha de promoção é registrada em `warn` e engolida. Você
ainda recebe os bytes que pediu; o disco simplesmente degrada para ler o
fallback toda vez, até que o primário volte a aceitar escrita. Defina
`throw_on_promotion_failure: true` quando uma perda silenciosa de promoção
esconderia uma falha que você precisa enxergar - uma migração que você
está tentando terminar, por exemplo:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "new-store".into(),
        fallback: "legacy-store".into(),
        throw_on_promotion_failure: true,
        ..Default::default()
    },
)?;
```

O registro rejeita uma configuração que não pode funcionar: um `primary`
ou `fallback` vazio, um par que nomeia o mesmo disco duas vezes, um disco
que nomeia a si mesmo, ou um nome que não está registrado. Cada caso
retorna um `FrameworkError` nomeando o problema, e nenhum disco é
registrado.

### Lendo sem promover

Defina `copy: false` para servir acertos do fallback sem escrevê-los de
volta:

```rust,ignore
Storage::register_read_through(
    "assets",
    ReadThroughConfig {
        primary: "cache-store".into(),
        fallback: "origin-store".into(),
        copy: false,
        ..Default::default()
    },
)?;
```

O disco então se lê como uma sobreposição transparente: o primário
responde pelo que ele tem, o fallback responde por todo o resto, e nada se
move entre eles. Use isso quando o primário é um cache pequeno que você
não quer que uma leitura avulsa encha, ou quando o fallback é a fonte da
verdade e o primário só guarda objetos que você colocou lá
deliberadamente.

A flag governa a promoção no momento da leitura e nada mais. Escritas,
deletes, metadados, listagens e os destinos de `copy` e `rename` se
comportam exatamente como se comportam com a promoção ligada - então um
disco com `copy: false` ainda aterrissa no primário um objeto copiado ou
movido. Como nada é escrito de volta, uma leitura com `copy: false` busca
apenas o range que você pediu, e não o objeto inteiro.

### Copiando e movendo através do fallback

`copy` e `rename` resolvem a origem contra o primário primeiro. Quando só
o fallback a tem, o objeto é transmitido em chunks de 64 KiB e o destino
aterrissa no primário:

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png` vive apenas no `legacy-store`. A cópia o transmite e escreve
// `branding/logo.png` no `new-store`; o objeto legado fica onde está.
assets.copy("logo.png", "branding/logo.png").await?;

// Um move faz o mesmo e depois exclui a origem legada.
assets.rename("logo.png", "branding/logo.png").await?;
```

Um move exclui a origem no fallback nos dois caminhos - tendo o primário a
origem ou não. Sem isso, a próxima leitura promoveria a cópia do fallback
de volta e desfaria o move.

Os dois caminhos diferem em quando fazem essa exclusão, e a diferença está
no que um move que falha deixa para trás:

- O primário tinha a origem. A cópia do fallback vai primeiro, antes do
  rename. Enquanto o primário tem o path, a cópia do fallback é
  inalcançável através deste disco, então removê-la primeiro não muda nada
  que você possa observar - e, se a exclusão falhar, nada foi movido
  ainda. Tente o move de novo. Se, em vez disso, a exclusão teve sucesso e
  o rename então falhou, o fallback não tem nada para aquele path, o
  destino não foi escrito, e o primário ainda tem a origem - então uma nova
  tentativa toma esse mesmo caminho e renomeia de novo. A falha custa a
  cópia fria e nada mais.
- Só o fallback a tinha. A exclusão só pode vir depois que o destino está
  no lugar, então um move que falha na exclusão deixa o destino escrito e a
  origem ainda no fallback. Tente o move de novo; a origem agora está no
  primário, então a nova tentativa toma o primeiro caminho.

De um jeito ou de outro, um move que falhou é seguro de repetir, e o
destino com que você acaba é o objeto do qual o move partiu.

As condições também viajam com a operação no caminho de streaming.
`if_not_exists` vira uma escrita condicional, então uma cópia ou um move
protegidos ainda recusam um destino existente em vez de sobrescrevê-lo, e
uma cópia que nomeia uma versão de origem recebe aquela versão do
fallback. O `if_match` de uma cópia é a única exceção: ele é uma condição
que o backend aplica dentro da própria cópia dele, que é justamente a
chamada que este caminho não pode fazer, então ele é recusado com um erro
`Unsupported` nomeando a condição, em vez de ser ignorado em silêncio.

Isso faz das condições o único lugar em que qual disco tem a origem
aparece para fora. Um diretório local anuncia `copy` e `rename`, mas
nenhuma das formas condicionais deles, então
`copy_with(a, b).if_not_exists(true)` tem sucesso quando só o fallback tem
`a` (vira uma escrita condicional) e é recusado com `Unsupported` quando o
primário a tem. Confira a condição de que você precisa contra o driver
primário, em vez de assumir que ela vale para todo objeto do disco.

Um move que o primário recusaria é recusado antes de qualquer coisa ser
excluída. Um primário sem `rename` nenhum, um move protegido sobre um
primário sem `rename` condicional, e um move protegido sobre um destino
que já existe falham todos com a origem do fallback ainda no lugar - um
move que nunca acontece não pode te custar a cópia fria.

Se o stream falhar no meio, o writer é abortado e um destino que a
transferência criou é excluído antes de o erro chegar até você, então uma
transferência que falhou não é observável como um objeto truncado. Um
destino que já estava lá é deixado em paz - uma cópia que falha não pode
ser a coisa que destrói um objeto que ela nunca escreveu. Um primário de
sistema de arquivos local também honra isso, porque ele prepara a
transferência em `.suprnova-atomic/` e só renomeia em caso de sucesso;
abortar o writer remove o arquivo preparado, então uma transferência que
falhou não deixa nem um destino parcial nem um arquivo temporário
sobrando.

### Leituras versionadas e condicionais

Uma leitura que carrega uma versão ou uma condição `If-Match`,
`If-None-Match`, `If-Modified-Since` ou `If-Unmodified-Since` é repassada
com aquela condição intacta, para que a resposta signifique o que você
pediu que ela significasse. Uma leitura dessas é servida, mas nunca
promovida: escrever uma versão antiga ou um corpo que casou com um
validador no primário o publicaria como o objeto vigente, e toda leitura
simples posterior receberia esse.

Qual disco responde a uma dessas é decidido do jeito de sempre. A primeira
sondagem é uma checagem de existência comum, então um disco read-through
delega uma leitura versionada ou condicional ao primário sempre que o
primário tem aquele path; ele só alcança o fallback quando o primário não
tem.

O primário também decide quais dessas um disco read-through aceita, porque
o reader do primário é aberto primeiro. Uma leitura versionada contra um
disco read-through cujo primário é um diretório local é rejeitada antes de
alcançar o fallback, já que um diretório local não tem versões.

### Por que Suprnova diverge

O Laravel monta um disco read-through a partir de uma entrada de
`config/filesystems.php` cujas chaves `primary` e `fallback` aceitam ou um
nome de disco ou uma config de driver inline. O Suprnova aceita apenas
nomes de disco, porque aqui os discos são registrados por construtores
tipados em vez de descritos por arrays - registre o disco interno
primeiro, depois nomeie-o.

A promoção do Laravel reconfere o primário depois de ler o fallback, o que
faz um escritor concorrente vencer. O Suprnova mantém essa checagem e
publica a promoção atomicamente, coisa que o Laravel não faz. Em um
primário de sistema de arquivos local, os bytes são preparados em um irmão
temporário e renomeados para o lugar; escrevê-los direto no alvo deixaria
um arquivo crescente e escrito pela metade visível pela duração da
escrita, e um disco read-through roteia leitores por exatamente essa
checagem de existência. Em um primário sem rename - em memória, S3, Azure
Blob, GCS - uma escrita já é uma publicação única e indivisível, então a
promoção escreve o alvo diretamente, condicionada a o objeto ainda não
existir, para que dois leitores concorrentes não promovam os dois.

Essa condição é a parte que uma promoção preparada não pode ter: o path de
preparo é único, então uma condição de não sobrescrever sobre ele seria
vazia, e o alvo é publicado por um rename que sobrescreve. Um disco
read-through sobre um primário de sistema de arquivos local, portanto,
abre mão dela - uma escrita que aterrissa no primário no instante entre a
última checagem de existência da promoção e o rename dela é sobrescrita
pela cópia promovida. Em um primário sem rename a condição vale e não
existe essa janela.

O objeto de preparo é uma entrada real no primário enquanto dura, então
uma listagem tirada no meio de uma promoção pode mostrar um irmão
`.suprnova-promote-<id>.tmp`. Uma leitura que termina, falha ou desiste
tenta remover o próprio irmão, e registra um aviso se essa exclusão falhar
em vez de fazer a leitura falhar. Nada varre um irmão deixado por uma
exclusão que falhou, por um processo que sofreu crash ou por uma future de
leitura cancelada no meio da promoção: esses têm de ser removidos à mão.

Uma leitura que resolve a partir do fallback segura o objeto em memória
até a escrita da promoção terminar, porque a promoção precisa do objeto
inteiro. Isso serve ao caso de tiering para o qual um disco read-through
existe. Para objetos frios muito grandes, leia o disco de fallback
diretamente ou use
[`copy_between_disks`](#cópia-via-streaming-entre-discos).

O Laravel devolve o próprio stream do fallback quando `copy` é `false` e
faz buffer através de `php://temp` quando é `true`. O Suprnova, em vez
disso, estreita a busca no fallback para o range solicitado quando `copy`
é `false`, e só faz buffer no caminho que promove, onde o objeto inteiro é
necessário de qualquer forma.

O `copy` e o `move` do Laravel através do fallback também fazem buffer da
origem através de `php://temp`. O Suprnova em vez disso a transmite em
chunks de 64 KiB, porque o fallback é onde vivem os objetos grandes e
raramente tocados, e exclui um destino escrito pela metade antes de
retornar o erro. Mais duas diferenças decorrem do OpenDAL. Excluir um path
que não está lá conta como sucesso, então um move limpa a origem no
fallback sem antes conferir que ela existe. E o OpenDAL carrega em `copy`
e `rename` condições para as quais o Flysystem não tem equivalente, então
o Suprnova tem de decidir o que cada uma significa quando a origem está só
no fallback: `if_not_exists` e a versão de origem de uma cópia são
honradas, e o `if_match` de uma cópia é recusado em vez de descartado.

O Laravel exclui a origem do fallback depois do move nos dois caminhos. O
Suprnova a exclui primeiro quando o primário tem a origem, porque as duas
ordens diferem sob uma nova tentativa: a origem é inalcançável através do
disco de qualquer forma, mas excluir por último significa que um move que
perdeu sua exclusão para uma falha transitória volta como um move cuja
origem agora está só no fallback, e transmite a cópia stale do fallback
por cima do destino que a primeira tentativa já escreveu corretamente.

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
