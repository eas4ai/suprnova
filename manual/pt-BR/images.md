# Imagens

O Suprnova traz um pipeline de imagens no formato do Laravel: monte-o em
um handler, encadeie as operações que quiser e termine com um terminal
que devolve bytes, uma resposta ou um arquivo armazenado.

```rust
use suprnova::{Image, OutputFormat, Response, handler};

#[handler]
pub async fn thumbnail() -> Response {
    Ok(Image::from_path("storage/photos/hero.jpg")
        .cover(320, 320)
        .to_format(OutputFormat::WebP)
        .quality(80)
        .to_response()
        .await?)
}
```

Esse handler decodifica o JPEG, preenche uma caixa de 320x320, corta o
excedente a partir do centro, codifica em WebP e devolve um `200` com
`Content-Type: image/webp`.

O subsistema vive em `suprnova::media`, atrás da feature `media`, que
vem ligada por padrão. Tudo o que você normalmente usa - `Image`,
`OutputFormat`, `ImageDriver`, `ImageConfig` - é reexportado de forma
plana na raiz do crate, então `use suprnova::Image;` é o import que
você quer. O nome do módulo é abrangente de propósito: é ali que as
superfícies de áudio e vídeo apoiadas no OxideAV também vão viver.

Se você está atualizando, note que o validador de upload que se
chamava `Image` agora é `ImageFile`, o que libera o nome simples para
este tipo de pipeline. Isso espelha o Laravel, onde a regra de
validação é `ImageFile` e o tipo de manipulação é `Image`. Veja
[Solicitações](requests.md) para o validador.

## O pipeline é lazy

Construir um `Image` não lê nada e não decodifica nada. As operações se
registram; a origem só é aberta quando um terminal roda. Então isto sai
de graça:

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

Nada tocou o disco ainda. `Image` é `Clone`, e um clone reexecuta o
pipeline a partir da sua origem em vez de compartilhar um resultado.

Dois construtores precisam ser eager, e dizem isso na documentação
deles: `from_upload` (o arquivo temporário de um upload não sobrevive à
solicitação) e `from_stream` (um stream só pode ser consumido uma vez).

## Construção

| Construtor | Origem | Eager? |
|---|---|---|
| `Image::from_bytes(bytes)` | qualquer coisa `Into<Bytes>` | não |
| `Image::from_path(path)` | o sistema de arquivos | não |
| `Image::from_disk(disk, path)` | um disco `Storage` | não |
| `Image::from_upload(&file).await?` | um `UploadedFile` | sim |
| `Image::from_stream(stream).await?` | um `Stream<Item = io::Result<Bytes>>` | sim |

O `from_stream` aplica `IMAGE_MAX_ALLOC_BYTES` *enquanto* coleta, então
um stream sem fim é interrompido em vez de descoberto depois de já ter
enchido a memória.

## Operações

| Método | Efeito |
|---|---|
| `resize(w, h)` | Dimensões exatas, proporção ignorada |
| `resize_width(w)` / `resize_height(h)` | Uma dimensão, a outra derivada da proporção |
| `scale(w, h)` | Cabe dentro da caixa, preservando a proporção. **Nunca amplia** |
| `scale_width(w)` / `scale_height(h)` | Reduz para no máximo uma dimensão. Nunca amplia |
| `crop(w, h, x, y)` | Recorta um retângulo. Dá erro se ele cair fora da imagem |
| `cover(w, h)` | Preenche a caixa exatamente, cortando o excedente a partir do centro |
| `contain(w, h)` | Cabe dentro da caixa, preservando a proporção. Sem preenchimento |
| `rotate(degrees)` | Gira no sentido horário em qualquer ângulo, aumentando o canvas para caber |
| `flip_vertically()` / `flip_horizontally()` | O `flip` e o `flop` do Laravel |
| `blur(amount)` | Desfoque gaussiano, `0..=100`. `0` é no-op |
| `sharpen(amount)` | Máscara de nitidez, `0..=100`. `0` é no-op. `50` é a intensidade clássica |
| `grayscale()` | Dessatura. Escrito à maneira do Laravel |
| `to_format(format)` | Escolhe o container de saída |
| `quality(q)` | Qualidade da codificação, limitada a `1..=100`, padrão `70` |

Valores que não fariam sentido são limitados em vez de rejeitados:
`blur(500)` registra `100`, `quality(0)` registra `1`. Um recorte que
cai fora da imagem é um erro de verdade, não um limite, porque mover em
silêncio a caixa de corte de alguém é pior do que avisar.

O `rotate` aceita ângulos arbitrários. Um múltiplo de 90 graus segue um
caminho exato alinhado aos eixos, sem reamostragem; qualquer outro é
bilinear, e o canvas cresce para que nenhum pixel seja cortado. Os
cantos expostos ficam transparentes onde o formato de saída tem canal
alfa.

## Terminais

Todo terminal é `async`, consome o `Image` e roda o trabalho de
decodificação, transformação e codificação em uma thread bloqueante,
para que nunca trave o runtime. A I/O da origem acontece antes desse
salto, então um disco lento nunca ocupa um worker bloqueante.

| Terminal | Devolve |
|---|---|
| `to_bytes()` | `Vec<u8>` do arquivo codificado |
| `to_response()` | Um `HttpResponse` com o `Content-Type` correto |
| `save(path)` | Escreve no sistema de arquivos |
| `store(disk, path)` | Escreve em um disco `Storage` |
| `dimensions()` | `(width, height)` da imagem **processada** |
| `mime_type()` | O tipo de mídia da imagem **processada** |
| `dominant_color()` | A cor média, como `#rrggbb` |

`dimensions()`, `mime_type()` e `dominant_color()` descrevem todos a
imagem final, não a origem - o mesmo contrato que o Laravel tem. Pedir
o mime type ainda executa o pipeline, porque informar um tipo para uma
imagem que não pode de fato ser produzida é uma mentira que o chamador
só descobriria depois.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // Lê (1200, 400), não as dimensões da origem.
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## Formatos

Cinco formatos são lidos e escritos hoje: **PNG, JPEG, WebP, GIF e
BMP**.

| Formato | Lê | Escreve | Controle de qualidade |
|---|---|---|---|
| PNG | sim | sim | ignorado (sem perdas) |
| JPEG | sim | sim | respeitado |
| WebP | sim | sim (sem perdas) | sem efeito hoje |
| GIF | sim | sim | ignorado (paleta) |
| BMP | sim | sim | ignorado (sem perdas) |

AVIF ainda não é lido nem escrito. O encoder AV1 interno de que ele
depende não foi publicado, e lançar um `OutputFormat::Avif` que sempre
falhasse seria uma promessa que o framework não conseguiria cumprir.
Ele chega junto com essa publicação, como uma nova variante de enum e
nada mais.

A saída GIF é quantizada em paleta para no máximo 256 cores, com
dithering de Floyd-Steinberg antes da codificação, então uma origem
fotográfica converte sem problemas em vez de dar erro.

O WebP é escrito sem perdas, então hoje `quality()` não tem efeito
nenhum sobre a saída WebP. Use JPEG quando precisar de um controle de
tamanho e qualidade.

## Armazenamento

`from_disk` e `store` funcionam contra qualquer disco `Storage`
registrado, então uma ida e volta de redimensionar e regravar nunca
toca em caminhos locais:

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

Veja [Armazenamento de arquivos](filesystem.md) para registrar discos.

## Limites de decodificação

A decodificação é onde entrada hostil faz estrago: alguns kilobytes
podem declarar um canvas de 40000x40000 e pedir que o servidor aloque
seis gigabytes para ele. O Suprnova recusa isso antes de alocar
qualquer coisa.

| Var | Padrão | Propósito |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Limite de largura e altura em pixels |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | Limite da pegada RGBA decodificada e do tamanho do próprio arquivo de origem |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Teto de tempo de relógio para uma invocação do ImageMagick (somente driver `magick`) |

O framework parseia o cabeçalho da própria entrada - algumas dezenas de
bytes, sem alocação -, lê as dimensões declaradas e rejeita entradas
grandes demais antes de construir um decodificador. Os mesmos limites
valem para os alvos de redimensionamento, porque `resize(50_000,
50_000)` aloca exatamente a mesma coisa quer os números venham de um
atacante, quer venham de um erro de digitação.

Bater no limite vira um `FrameworkError::param`, no formato 4xx, porque
entrada grande demais é problema do cliente, não falha do servidor.

Configuração fora do intervalo é limitada com um aviso em vez de falhar
o boot: `IMAGE_MAX_DIMENSION=0` rejeitaria toda imagem da aplicação, o
que não é o que ninguém quis configurar.

### Um limite não é configurável

Um WebP declara seu tamanho decodificado real no chunk mais interno do
bitstream, e não no cabeçalho do canvas, então o framework percorre o
container para encontrá-lo. Essa varredura para depois de **4096 chunks
por nível** e segue aninhamento até **dois níveis de profundidade**, e
um arquivo que exceda qualquer um dos dois é recusado de imediato em
vez de medido.

Ele é recusado em vez de medido de propósito. Informar um número
vindo de uma varredura que não chegou ao fim do arquivo seria um gate
que uma pilha grande o bastante de chunks de enchimento poderia
contornar, então uma varredura que não termina não tem resposta a dar.

Nenhum dos dois números é ajustável, e nenhuma variável `IMAGE_MAX_*`
afeta os dois - o erro diz isso, em vez de dizer "configurado",
justamente para que ninguém passe uma tarde aumentando
`IMAGE_MAX_ALLOC_BYTES` e vendo nada mudar. Na prática, só um arquivo
deliberadamente hostil chega perto disso: uma animação de 300 quadros
passa com folga, e uma de 4100 quadros não.

## Backends

Como no Laravel, a superfície de imagens são dois drivers, escolhidos
com `IMAGE_DRIVER`.

| Driver | Valor | Precisa de | Lê |
|---|---|---|---|
| OxideAV | `oxideav` (padrão) | nada | PNG, JPEG, WebP, GIF, BMP |
| ImageMagick | `magick` | ImageMagick 7 no host | o que os delegates do host oferecerem |

### `IMAGE_DRIVER=oxideav`

O padrão. Rust puro, construído sobre a família de codecs
[OxideAV](https://github.com/OxideAV): nenhuma biblioteca nativa, nada
a instalar, nada a configurar. É a escolha certa para quase toda
aplicação, e é o que um app criado com scaffold recebe.

### `IMAGE_DRIVER=magick`

Opcional. Executa um binário do ImageMagick 7 instalado no host,
mandando a imagem pela stdin e lendo o resultado de volta pela stdout -
sem arquivos temporários. O nome do binário vem de
`IMAGE_MAGICK_BINARY` e assume `magick` por padrão; um binário ausente
é um erro claro no primeiro uso, não um fallback silencioso.

Escolha esse driver quando você precisar de formatos de entrada que o
driver em Rust puro não carrega - HEIC sendo o mais comum. O custo é
uma dependência do host: o operador instala o ImageMagick e seus
delegates, e assume o licenciamento deles. O framework não linka nem
compila nada nativo em nenhum dos casos.

Os argumentos são sempre um array fixo entregue direto ao processo,
nunca uma string de shell, e todo argumento numérico é formatado a
partir de um campo já validado. Não existe posição de argumento que a
entrada do usuário alcance.

Quando o framework reconhece a entrada, o decodificador é nomeado na
linha de comando - `png:-` em vez de um `-` solto. Isso importa: diante
de um `-` solto, o ImageMagick escolhe um coder a partir dos bytes que
recebe, então um arquivo cujos bytes mágicos digam MVG ou MSL é lido
como um *script*, independentemente do que sua aplicação acreditava
estar aceitando. Fixar o coder faz um arquivo mal rotulado falhar em
vez de virar outra coisa.

**Entrada que o framework não consegue nomear ainda depende do seu
`policy.xml`.** Ler esses formatos é toda a razão de este driver
existir, então esse caminho não pode fixar um coder. Endureça a
política do ImageMagick do host - no mínimo desativando os coders
`MVG`, `MSL`, `URL`, `HTTPS`, `EPHEMERAL` e `TEXT` - se você aceita
uploads arbitrários sob `IMAGE_DRIVER=magick`.

Os limites de decodificação são aplicados duas vezes sob esse driver.
Para os cinco formatos que o framework consegue parsear, a verificação
de cabeçalho acima roda antes de o processo ser criado. Para todo o
resto, uma pré-análise é impossível, então cada invocação carrega as
próprias flags `-limit` do ImageMagick derivadas da mesma configuração,
incluindo um `-limit time` de tempo de relógio.

Essa flag não é a história toda, porque o ImageMagick a aplica com o
próprio monitor de recursos, e um processo travado dentro de um
delegate antes de esse monitor entrar em ação nunca a dispara. Então o
Suprnova mantém também o próprio prazo: passado
`IMAGE_MAGICK_TIMEOUT_SECS` (mais alguns segundos de folga para o
limite do próprio IM disparar primeiro), ele mata o grupo de
processos - delegates incluídos, não só o processo que iniciou - e
para de esperar nos pipes. Um delegate travado, portanto, não consegue prender
uma thread de worker. Delegates que ficam no grupo de processos morrem
com ele; um que sai do grupo, ou um host sem binário `kill`, pode
sobreviver à solicitação - esse resíduo é para o que serve a supervisão
de processos do host.

Uma morte desse tipo aparece como um `FrameworkError::internal` 5xx, e
não um 4xx, mesmo que uma solicitação a tenha provocado. Alguma coisa
travou o caminho de imagem a ponto de precisar ser morta, o que
pertence ao monitoramento de erro de servidor, onde um operador vai
ver - classificar isso como erro de cliente arquivaria a única condição
aqui que vale um alerta de plantão.

## Drivers personalizados

`ImageDriver` é o ponto de extensão: `&[u8]` entra, `Vec<u8>` sai,
nenhum tipo de codec atravessando a fronteira.

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // Decodifique `contents`, repita `pipeline.transformations` e então
        // codifique para `pipeline.format` com `pipeline.quality`.
        todo!()
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        todo!()
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        todo!()
    }

    fn name(&self) -> &'static str {
        "mine"
    }
}
```

Instale-o durante o `bootstrap()`, antes de a primeira imagem ser
processada:

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

Um driver em conformidade aplica os limites de `ImageConfig`
configurados antes de alocar para uma decodificação. O framework não
consegue fazer isso em nome de um driver, porque nunca vê o buffer
decodificado.

### Alcançando mais formatos

Se os cinco embutidos não bastam, há três caminhos, em ordem
aproximada de quanto você assume:

1. **O driver `magick` embutido.** Defina `IMAGE_DRIVER=magick`. A
   amplitude de formatos vem dos delegates do ImageMagick do host, e
   não há dependência de build para gerenciar.
2. **Um driver personalizado em volta do libvips**, por exemplo via o
   crate
   [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings)
   (MIT). O libvips é o motor por trás do `sharp` do Node, com uma
   faixa de formatos muito ampla - JPEG, JPEG XL, TIFF, PNG, WebP,
   HEIC, AVIF, PDF, SVG, GIF e mais, além de delegação ao ImageMagick -
   e forte desempenho em streaming. Ele se liga à biblioteca C do
   libvips, então seu app instala o libvips em tempo de build e de
   execução e assume essa dependência, que é exatamente por que ela
   pertence atrás da trait em vez de dentro do framework. Uma nota
   prática: o `VipsImage` do binding não é thread safe, o que o formato
   de driver de uma imagem por chamada de `process()` já acomoda.
3. **Qualquer ferramenta de linha de comando**, embrulhada do jeito que
   o driver `magick` é: um array fixo de argumentos entregue a
   `std::process::Command`, bytes da imagem pela stdin e de volta pela
   stdout, nunca uma string de shell.

O Suprnova endossa a fronteira da trait, não uma dependência
específica atrás dela. O que fica lá atrás é decisão sua, e o
licenciamento também.

## Testes

O subsistema não precisa de fixtures em disco - ele é a própria fábrica
de fixtures, uma vez que decodificação e codificação façam ida e volta:

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// Cresce uma fixture 1x1 de literal de bytes até o tamanho que um teste
/// precisar.
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

Testes que apertam os limites de decodificação precisam ser
serializados: os limites são globais ao processo, então um irmão em
paralelo decodificaria sob o limite apertado.

### Por que Suprnova diverge

**Nada de HEIC no driver padrão, e a razão são patentes.** O HEVC, o
codec dentro do HEIC, é onerado por patentes - o pool da Access Advance
entre outros. O Suprnova não instala nenhuma biblioteca nativa, então
um decodificador embutido teria de ser Rust puro e carregaria essa
exposição diretamente, e o único decodificador em Rust puro com
credibilidade é duplo AGPL-3.0/comercial, o que é uma obrigação legal
por aplicação, e não algo em que um framework MIT possa jogar alguém
por padrão.

Os dois frameworks fazem do HEIC uma questão de provisionamento do
host; a versão do Suprnova só tem uma peça móvel a menos. O driver
padrão do Laravel, o GD, não lê HEIC de jeito nenhum, e o caminho do
Imagick precisa do delegate libheif compilado **tanto** no binário do
ImageMagick do sistema **quanto** na extensão `imagick` do PHP. No
Suprnova o driver padrão não lê HEIC, e `IMAGE_DRIVER=magick` lê sempre
que o ImageMagick do host carregar o delegate libheif - sem camada de
extensão no meio. Então a ingestão de HEIC funciona hoje: instale o
ImageMagick com libheif pelo seu gerenciador de pacotes e vire a
variável de ambiente. O licenciamento fica onde deve, com o host.

Quando o driver `oxideav` encontra um arquivo HEIC, ele diz isso pelo
nome, aponta para este capítulo e nomeia os dois caminhos adiante, em
vez de devolver um genérico "formato não suportado".

**AVIF está pendente, não foi descartado.** Ele é livre de royalties e
é a resposta de formato moderno que queremos; o encoder AV1 interno
simplesmente ainda não foi publicado. O WebP é o caminho de formato
moderno enquanto isso.

**Nada de construtores de base64 ou URL.** O `ImageManager` do Laravel
tem `->read($base64)` e `->read($url)`. O `from_bytes` compõe com o que
quer que tenha produzido os bytes, incluindo o
[Cliente HTTP](http-client.md), e manter a busca de URL fora do
subsistema de imagens mantém os timeouts, os retries e a política de
SSRF dele em um lugar só, em vez de dois.

**O `from_stream` é eager, com um limite.** O conteúdo no Laravel é uma
closure lazy. Um stream não pode ser reproduzido de novo, então este
aqui é drenado na construção, contando bytes contra
`IMAGE_MAX_ALLOC_BYTES` conforme avança.

**O `contain` não preenche.** Ele encaixa a imagem dentro da caixa e
para por aí; não faz letterbox sobre um fundo. Componha você mesmo com
um fundo se precisar de um.

**O redimensionamento usa reamostragem bilinear.** O conjunto de
filtros do backend traz vizinho mais próximo e bilinear; bilinear é o
padrão documentado dele para imagens naturais.

**Imagens nunca são serializáveis.** O Laravel lança exceção em
`__serialize` e o Suprnova simplesmente não implementa isso. Guarde o
caminho ou a chave do disco e reconstrua o pipeline.

## Próximos passos

- [Armazenamento de arquivos](filesystem.md) para os discos que `from_disk` e `store` leem e escrevem.
- [Respostas HTTP](responses.md) para o que `to_response()` devolve.
- [Variáveis de ambiente](env-vars.md) para a lista completa de configurações de imagem.
