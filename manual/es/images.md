# Imágenes

Suprnova incluye un pipeline de imágenes con forma de Laravel:
constrúyelo en un handler, encadena las operaciones que quieras y
termínalo con un terminal que te entrega bytes, una respuesta o un
archivo almacenado.

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

Ese handler decodifica el JPEG, rellena una caja de 320x320, recorta el
sobrante desde el centro, codifica en WebP y devuelve un `200` con
`Content-Type: image/webp`.

El subsistema vive en `suprnova::media`, detrás de la feature `media`,
activa por defecto. Todo aquello a lo que sueles echar mano - `Image`,
`OutputFormat`, `ImageDriver`, `ImageConfig` - se reexporta plano en la
raíz del crate, así que `use suprnova::Image;` es el import que quieres.
El nombre del módulo es plural en espíritu a propósito: es donde vivirán
también las superficies de audio y vídeo respaldadas por OxideAV.

Si vienes de una versión anterior, ten en cuenta que el validador de
subidas que antes se llamaba `Image` ahora es `ImageFile`, lo que libera
el nombre a secas para este tipo del pipeline. Eso refleja a Laravel,
donde la regla de validación es `ImageFile` y el tipo de manipulación es
`Image`. Consulta [Solicitudes](requests.md) para el validador.

## El pipeline es perezoso

Construir un `Image` no lee nada ni decodifica nada. Las operaciones se
anotan a sí mismas; el origen solo se abre cuando se ejecuta un terminal.
Así que esto es gratis:

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

Todavía nada ha tocado el disco. `Image` es `Clone`, y un clon vuelve a
ejecutar el pipeline desde su origen en lugar de compartir un resultado.

Dos constructores tienen que ser anticipados, y así lo dicen en su
documentación: `from_upload` (el archivo temporal de una subida no
sobrevive a la solicitud) y `from_stream` (un stream solo se puede
consumir una vez).

## Construcción

| Constructor | Origen | ¿Anticipado? |
|---|---|---|
| `Image::from_bytes(bytes)` | cualquier cosa `Into<Bytes>` | no |
| `Image::from_path(path)` | el sistema de archivos | no |
| `Image::from_disk(disk, path)` | un disco de `Storage` | no |
| `Image::from_upload(&file).await?` | un `UploadedFile` | sí |
| `Image::from_stream(stream).await?` | un `Stream<Item = io::Result<Bytes>>` | sí |

`from_stream` hace cumplir `IMAGE_MAX_ALLOC_BYTES` *mientras* recopila,
así que un stream infinito se corta en lugar de descubrirse cuando ya ha
llenado la memoria.

## Operaciones

| Método | Efecto |
|---|---|
| `resize(w, h)` | Dimensiones exactas, ignorando la relación de aspecto |
| `resize_width(w)` / `resize_height(h)` | Una dimensión, la otra derivada de la relación de aspecto |
| `scale(w, h)` | Encaja dentro de la caja conservando la relación de aspecto. **Nunca amplía** |
| `scale_width(w)` / `scale_height(h)` | Reduce hasta como mucho una dimensión. Nunca amplía |
| `crop(w, h, x, y)` | Recorta un rectángulo. Da error si cae fuera de la imagen |
| `cover(w, h)` | Rellena la caja exactamente, recortando el sobrante desde el centro |
| `contain(w, h)` | Encaja dentro de la caja conservando la relación de aspecto. Sin relleno |
| `rotate(degrees)` | Rota en sentido horario cualquier ángulo, agrandando el lienzo para que quepa |
| `flip_vertically()` / `flip_horizontally()` | El `flip` y el `flop` de Laravel |
| `blur(amount)` | Desenfoque gaussiano, `0..=100`. `0` no hace nada |
| `sharpen(amount)` | Máscara de enfoque, `0..=100`. `0` no hace nada. `50` es la intensidad clásica |
| `grayscale()` | Desatura. Escrito a la manera de Laravel |
| `to_format(format)` | Elige el contenedor de salida |
| `quality(q)` | Calidad de codificación, acotada a `1..=100`, `70` por defecto |

Los valores que serían un disparate se acotan en lugar de rechazarse:
`blur(500)` anota `100`, `quality(0)` anota `1`. Un recorte que cae fuera
de la imagen sí es un error, no un acotado, porque mover en silencio la
caja de recorte de alguien es peor que decírselo.

`rotate` acepta ángulos arbitrarios. Un múltiplo de 90 grados toma una
ruta exacta alineada con los ejes y sin remuestreo; cualquier otro es
bilineal, y el lienzo crece para que no se recorte ningún píxel. Las
esquinas que quedan al descubierto son transparentes cuando el formato de
salida tiene canal alfa.

## Terminales

Todo terminal es `async`, consume el `Image` y ejecuta el trabajo de
decodificación, transformación y codificación en un hilo bloqueante, de
modo que nunca atasca el runtime. La E/S del origen ocurre antes de ese
salto, así que un disco lento nunca ocupa un worker bloqueante.

| Terminal | Devuelve |
|---|---|
| `to_bytes()` | `Vec<u8>` del archivo codificado |
| `to_response()` | Un `HttpResponse` con el `Content-Type` correcto |
| `save(path)` | Escribe en el sistema de archivos |
| `store(disk, path)` | Escribe en un disco de `Storage` |
| `dimensions()` | `(width, height)` de la imagen **procesada** |
| `mime_type()` | El tipo de medio de la imagen **procesada** |
| `dominant_color()` | El color medio, como `#rrggbb` |

`dimensions()`, `mime_type()` y `dominant_color()` describen todos la
imagen terminada, no el origen: el mismo contrato que tiene Laravel.
Pedir el tipo mime sigue ejecutando el pipeline, porque informar de un
tipo para una imagen que en realidad no se puede producir es una mentira
que quien llama solo descubriría más tarde.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // Lee (1200, 400), no las dimensiones del origen.
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## Formatos

Hoy se leen y se escriben cinco formatos: **PNG, JPEG, WebP, GIF y BMP**.

| Formato | Lee | Escribe | Control de calidad |
|---|---|---|---|
| PNG | sí | sí | se ignora (sin pérdida) |
| JPEG | sí | sí | se respeta |
| WebP | sí | sí (sin pérdida) | hoy no tiene efecto |
| GIF | sí | sí | se ignora (paleta) |
| BMP | sí | sí | se ignora (sin pérdida) |

AVIF todavía no se lee ni se escribe. El codificador AV1 propio del que
depende no se ha publicado, y distribuir un `OutputFormat::Avif` que
siempre fallara sería una promesa que el framework no podría cumplir.
Llegará con esa publicación, como una nueva variante del enum y nada más.

La salida GIF se cuantiza a una paleta de como mucho 256 colores con
difuminado de Floyd-Steinberg antes de codificar, así que un origen
fotográfico se convierte limpiamente en lugar de dar error.

WebP se escribe sin pérdida, así que `quality()` no tiene actualmente
ningún efecto sobre la salida WebP. Usa JPEG cuando necesites un dial de
tamaño y calidad.

## Almacenamiento

`from_disk` y `store` funcionan contra cualquier disco de `Storage`
registrado, así que un viaje de ida y vuelta de redimensionar y volver a
guardar nunca toca rutas locales:

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

Consulta [Sistema de archivos y almacenamiento](filesystem.md) para
registrar discos.

## Límites de decodificación

La decodificación es donde la entrada hostil hace daño: unos pocos
kilobytes pueden declarar un lienzo de 40000x40000 y pedirle a un
servidor que reserve seis gigabytes para él. Suprnova rechaza eso antes
de reservar nada.

| Var | Por defecto | Propósito |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Tope del ancho y el alto en píxeles |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | Tope de la huella RGBA decodificada, y del tamaño del propio archivo de origen |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Techo de reloj de pared para una invocación de ImageMagick (solo para el driver `magick`) |

El framework parsea la propia cabecera de la entrada - unas pocas decenas
de bytes, sin reservar memoria -, lee las dimensiones declaradas y
rechaza la entrada de tamaño excesivo antes de construir un
decodificador. Los mismos topes se aplican a los objetivos de
redimensionado, porque `resize(50_000, 50_000)` reserva exactamente lo
mismo tanto si los números vienen de un atacante como de una errata.

Alcanzar un límite es un `FrameworkError::param` con forma de 4xx, porque
una entrada de tamaño excesivo es un problema del cliente, no un fallo
del servidor.

La configuración fuera de rango se recorta con un aviso en lugar de hacer
fallar el arranque: `IMAGE_MAX_DIMENSION=0` rechazaría todas las imágenes
de la aplicación, que no es lo que nadie pretendía configurar.

### Una cota no es configurable

Un WebP declara su tamaño decodificado real en su fragmento de bitstream
más interno, no en la cabecera del lienzo, así que el framework recorre el
contenedor para encontrarlo. Ese recorrido se detiene tras **4096
fragmentos por nivel** y sigue el anidamiento **hasta dos niveles de
profundidad**, y un archivo que supere cualquiera de los dos se rechaza de
plano en lugar de medirse.

Se rechaza en lugar de medirse a propósito. Informar de un número a partir
de un recorrido que no llegó al final del archivo sería una compuerta que
un montón suficientemente grande de fragmentos de relleno podría esquivar,
así que un recorrido que no se puede terminar no tiene respuesta que dar.

Ninguno de los dos números se puede ajustar, y ninguna variable
`IMAGE_MAX_*` los afecta - el error lo dice, en vez de decir
"configurado", precisamente para que nadie se pase una tarde subiendo
`IMAGE_MAX_ALLOC_BYTES` y viendo que no cambia nada. En la práctica solo
un archivo deliberadamente hostil se acerca a esa cota: una animación de
300 fotogramas pasa holgadamente, y una de 4100 no.

## Backends

Como en Laravel, la superficie de imágenes son dos drivers, elegidos con
`IMAGE_DRIVER`.

| Driver | Valor | Necesita | Lee |
|---|---|---|---|
| OxideAV | `oxideav` (por defecto) | nada | PNG, JPEG, WebP, GIF, BMP |
| ImageMagick | `magick` | ImageMagick 7 en el anfitrión | lo que provean los delegados del anfitrión |

### `IMAGE_DRIVER=oxideav`

El de por defecto. Rust puro, construido sobre la familia de códecs
[OxideAV](https://github.com/OxideAV): sin biblioteca nativa, sin nada que
instalar, sin nada que configurar. Es la elección correcta para casi
cualquier aplicación, y es lo que recibe una aplicación con andamiaje.

### `IMAGE_DRIVER=magick`

Opcional. Ejecuta un binario de ImageMagick 7 instalado en el anfitrión,
enviándole la imagen por stdin y leyendo el resultado por stdout, sin
archivos temporales. El nombre del binario viene de `IMAGE_MAGICK_BINARY`
y por defecto es `magick`; un binario ausente da un error claro en el
primer uso, no un respaldo silencioso.

Elígelo cuando necesites formatos de entrada que el driver en Rust puro no
lleva, siendo HEIC el habitual. El coste es una dependencia del anfitrión:
quien opera instala ImageMagick y sus delegados, y se hace cargo de sus
licencias. El framework no enlaza nada ni compila nada nativo en ninguno
de los dos casos.

Los argumentos son siempre un array fijo que se entrega directamente al
proceso, nunca una cadena de shell, y todo argumento numérico se formatea
a partir de un campo ya validado. No hay ninguna posición de argumento a
la que pueda llegar la entrada del usuario.

Cuando el framework reconoce la entrada, el decodificador se nombra en la
línea de comandos: `png:-` en lugar de un `-` a secas. Eso importa: ante
un `-` a secas, ImageMagick elige un codificador a partir de los bytes que
recibe, así que un archivo cuyos bytes mágicos dicen MVG o MSL se lee como
un *script*, sin importar lo que tu aplicación creyera estar aceptando.
Fijar el codificador hace que un archivo mal etiquetado falle en lugar de
convertirse en otra cosa.

**La entrada que el framework no puede nombrar sigue dependiendo de tu
`policy.xml`.** Leer esos formatos es la razón de ser de este driver, así
que esa ruta no puede fijar un codificador. Endurece la política de
ImageMagick del anfitrión - como mínimo desactivando los codificadores
`MVG`, `MSL`, `URL`, `HTTPS`, `EPHEMERAL` y `TEXT` - si aceptas subidas
arbitrarias con `IMAGE_DRIVER=magick`.

Los límites de decodificación se hacen cumplir dos veces bajo este driver.
Para los cinco formatos que el framework sabe parsear, la comprobación de
cabecera de arriba se ejecuta antes de lanzar el proceso. Para todo lo
demás un preanálisis es imposible, así que cada invocación lleva los
propios flags `-limit` de ImageMagick derivados de la misma configuración,
incluido un `-limit time` de reloj de pared.

Ese flag no es toda la historia, porque ImageMagick lo hace cumplir con su
propio monitor de recursos, y un proceso atascado dentro de un delegado
antes de que ese monitor entre en juego nunca lo activa. Así que Suprnova
mantiene además su propio plazo: pasados los `IMAGE_MAGICK_TIMEOUT_SECS`
(más un par de segundos de gracia para que el límite del propio IM se
dispare primero) mata el grupo de procesos - delegados incluidos, no solo
el proceso que lanzó - y deja de esperar en las tuberías. Un delegado
estancado no puede, por tanto, retener un hilo de worker. Los delegados que
se quedan en el grupo de procesos mueren con él; uno que abandone el
grupo, o un anfitrión sin binario `kill`, puede sobrevivir a la solicitud:
ese residuo es para lo que sirve la supervisión de procesos del anfitrión.

Un kill emerge como un `FrameworkError::internal` 5xx, no como un 4xx,
aunque lo haya desencadenado una solicitud. Algo atascó la ruta de
imágenes lo bastante mal como para necesitar que se le matara, y eso
pertenece a la monitorización de errores de servidor, donde quien opera lo
verá; clasificarlo como error de cliente archivaría la única condición de
aquí por la que merece la pena avisar de madrugada.

## Drivers propios

`ImageDriver` es el punto de extensión: entra `&[u8]`, sale `Vec<u8>`, sin
que ningún tipo de códec cruce la frontera.

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // Decodifica `contents`, reproduce `pipeline.transformations` y luego
        // codifica a `pipeline.format` con `pipeline.quality`.
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

Instálalo durante `bootstrap()`, antes de procesar la primera imagen:

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

Un driver conforme hace cumplir los límites de `ImageConfig` configurados
antes de reservar memoria para una decodificación. El framework no puede
hacerlo en nombre de un driver, porque nunca ve el búfer decodificado.

### Llegar a más formatos

Si los cinco integrados no bastan, hay tres vías, más o menos ordenadas
por cuánto asumes tú:

1. **El driver `magick` integrado.** Pon `IMAGE_DRIVER=magick`. La amplitud
   de formatos viene de los delegados de ImageMagick del anfitrión, y no
   hay ninguna dependencia de compilación que gestionar.
2. **Un driver propio alrededor de libvips**, por ejemplo mediante el
   crate
   [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings)
   (MIT). libvips es el motor detrás de `sharp` de Node, con un abanico de
   formatos muy amplio - JPEG, JPEG XL, TIFF, PNG, WebP, HEIC, AVIF, PDF,
   SVG, GIF y más, además de delegación en ImageMagick - y un buen
   rendimiento en streaming. Enlaza la biblioteca C de libvips, así que tu
   aplicación instala libvips en tiempo de compilación y de ejecución y se
   hace cargo de esa dependencia, que es exactamente por lo que pertenece
   detrás del trait y no dentro del framework. Una nota práctica: el
   `VipsImage` del binding no es seguro entre hilos, cosa que la forma del
   driver, con una imagen por llamada a `process()`, ya contempla.
3. **Cualquier herramienta de CLI**, envuelta como lo está el driver
   `magick`: un array fijo de argumentos entregado a
   `std::process::Command`, los bytes de la imagen por stdin y de vuelta
   por stdout, nunca una cadena de shell.

Suprnova respalda la frontera del trait, no ninguna dependencia concreta
detrás de ella. Lo que haya ahí atrás es cosa tuya, y su licencia también.

## Pruebas

El subsistema no necesita fixtures en disco: es su propia fábrica de
fixtures en cuanto la decodificación y la codificación hacen el viaje de
ida y vuelta:

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// Agranda un fixture de 1x1 escrito como literal de bytes hasta el
/// tamaño que necesite una prueba.
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

Las pruebas que estrechan los límites de decodificación tienen que
serializarse: los límites son globales al proceso, así que una prueba
hermana en paralelo decodificaría bajo el tope estrechado.

### Por qué Suprnova diverge

**No hay HEIC en el driver por defecto, y la razón son las patentes.**
HEVC, el códec que hay dentro de HEIC, está sujeto a patentes: el pool de
Access Advance, entre otros. Suprnova no instala bibliotecas nativas, así
que un decodificador integrado tendría que ser Rust puro y cargaría con
esa exposición directamente, y el único decodificador creíble en Rust puro
tiene licencia dual AGPL-3.0/comercial, que es una obligación legal por
aplicación y no algo a lo que un framework MIT pueda arrastrar a nadie por
defecto.

Los dos frameworks convierten HEIC en un asunto de aprovisionamiento del
anfitrión; la versión de Suprnova sencillamente tiene una pieza móvil
menos. El driver por defecto de Laravel, GD, no puede leer HEIC en
absoluto, y su ruta Imagick necesita el delegado libheif compilado en
**ambos**: el binario de ImageMagick del sistema y la extensión `imagick`
de PHP. En Suprnova el driver por defecto no lee HEIC, e
`IMAGE_DRIVER=magick` lo lee siempre que el ImageMagick del anfitrión
lleve el delegado libheif, sin una capa de extensión de por medio. Así que
la ingesta de HEIC funciona hoy: instala ImageMagick con libheif desde tu
gestor de paquetes y cambia la variable de entorno. La licencia queda
donde le corresponde, con el anfitrión.

Cuando el driver `oxideav` se topa con un archivo HEIC lo dice por su
nombre, señala este capítulo y nombra las dos vías hacia delante, en lugar
de devolver un genérico "formato no soportado".

**AVIF está pendiente, no descartado.** Está libre de regalías y es la
respuesta que queremos en formatos modernos; sencillamente, el codificador
AV1 propio todavía no se ha publicado. Mientras tanto, WebP es la vía de
formato moderno.

**No hay constructores desde base64 ni desde URL.** El `ImageManager` de
Laravel tiene `->read($base64)` y `->read($url)`. `from_bytes` compone con
lo que sea que produjo los bytes, incluido el [cliente
HTTP](http-client.md), y mantener una descarga por URL fuera del
subsistema de imágenes deja sus tiempos de espera, sus reintentos y su
política de SSRF en un solo sitio en lugar de en dos.

**`from_stream` es anticipado, con un tope.** Los contenidos de Laravel
son un closure perezoso. Un stream no se puede reproducir, así que este se
drena en el momento de la construcción, contando los bytes contra
`IMAGE_MAX_ALLOC_BYTES` sobre la marcha.

**`contain` no rellena.** Encaja la imagen dentro de la caja y ahí se
queda; no la coloca sobre un fondo con bandas. Compónlo tú con un fondo si
necesitas uno.

**El redimensionado usa remuestreo bilineal.** El conjunto de filtros del
backend incluye vecino más cercano y bilineal; bilineal es su valor por
defecto documentado para imágenes naturales.

**Las imágenes nunca son serializables.** Laravel lanza una excepción en
`__serialize` y Suprnova sencillamente no lo implementa. Guarda la ruta o
la clave del disco y reconstruye el pipeline.

## Siguiente

- [Sistema de archivos y almacenamiento](filesystem.md) para los discos
  que `from_disk` y `store` leen y escriben.
- [Respuestas HTTP](responses.md) para lo que devuelve `to_response()`.
- [Variables de entorno](env-vars.md) para la lista completa de ajustes
  de imagen.
