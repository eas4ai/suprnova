# Sistema de archivos y almacenamiento

La fachada de almacenamiento de Suprnova te da una única API de disco
nombrado sobre sistemas de archivos locales, backends en memoria y los
principales almacenes de objetos (S3, Azure Blob, Google Cloud
Storage). Internamente está construida sobre
[`opendal`](https://docs.rs/opendal) - pero la superficie de cara al
consumidor está moldeada para igualar las llamadas
`Storage::disk(...)` de Laravel, de modo que la memoria muscular de
PHP se traslada directamente.

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

## Registrar discos

Cada disco se registra una vez en el arranque vía `Storage::register_*` y se
busca por nombre a través de `Storage::disk(name)`. No hay un "backend por
defecto" al que degraden los demás - cada driver está al mismo nivel.

| Constructor                          | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Sistema de archivos local     | `filesystem`        |
| `Storage::register_memory(name)`     | Memoria en proceso (tests)    | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 o compatible con S3 | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | Compuesto read-through | `filesystem` |

`filesystem` está activada por defecto; las features de Azure y GCS no.
Actívalas en tu `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.5", features = ["filesystem-gcs"] }
```

Sin la feature, `register_azblob` / `register_gcs` y sus structs de
configuración no existen - obtienes un error de compilación que nombra el
elemento ausente, no un fallo en tiempo de ejecución.

Cada constructor tiene una variante `_with` que te entrega el
`suprnova::opendal::Operator` justo antes de que aterrice en el registro,
para que puedas instalar capas de reintento/tiempo de espera/logging a su
alrededor:

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

Los constructores de nube (`register_s3`, `register_azblob`,
`register_gcs`) aplican un `RetryLayer` (3 intentos) por defecto, porque la
limitación transitoria y los errores 5xx son rutina en los almacenes de
objetos. Usa las variantes `_with` cuando necesites control total.

El conjunto completo de capas de opendal que Suprnova cablea es
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (puentea a
OTel vía `tracing-opentelemetry` cuando la feature `otel` del framework
está activada) y `PrometheusClientLayer` (exporta histogramas y contadores
a un `prometheus_client::registry::Registry` tuyo). El orden de las capas
importa - la capa más externa envuelve todo lo que hay dentro de ella - y
la pila idiomática es `RetryLayer → TimeoutLayer → LoggingLayer`, de modo
que un intento que agota su tiempo aún se registra y un reintento cubre
los fallos de transporte.

Volver a registrar el mismo nombre reemplaza el operator anterior y emite
un log `warn!` - los discos están pensados para registrarse una sola vez
en el arranque, y un duplicado accidental podría cambiar un disco de
producción por uno en memoria. El reemplazo se produce igualmente; el
aviso solo hace que el cambio no pase desapercibido.

### Por qué Suprnova diverge

El `config/filesystems.php` de Laravel lista todos los drivers de disco y
eliges uno en tiempo de ejecución; no se compila nada fuera. Suprnova pone
Azure y GCS tras features porque en Rust la elección tiene un coste de
dependencias, y esta además tiene una dimensión de seguridad: ambos crates
de servicio de opendal arrastran `rsa`, que carga con
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) (el
ataque de temporización Marvin) sin ninguna versión corregida upstream.
Hacerlas opcionales significa que una aplicación que guarda archivos en
local o en S3 nunca lleva ese crate.

S3 deliberadamente *no* está tras una feature - su firmador nunca dependió
de `rsa`, así que ponerlo tras una rompería el backend en la nube más
usado sin eliminar nada.

### Escrituras locales atómicas

En un disco local, toda operación que publica bytes en una ruta los
publica en un solo paso. `disk.write(...)`, `disk.writer(...)` y
`disk.copy(...)` aterrizan primero en `<root>/.suprnova-atomic/`, se
vuelcan y se sincronizan ahí, y luego se renombran sobre el destino;
`disk.rename(...)` ya es un solo paso. Un lector concurrente ve por tanto
o bien el objeto anterior o bien el nuevo ya terminado, nunca una longitud
parcial, y un proceso que muere a mitad de escritura deja el destino
intacto en lugar de truncado en la ruta activa.

`append` es la única operación en el sitio, porque preparar un append
significaría copiar antes el objeto entero. Eso vale tanto para el append
que *crea* el objeto como para todos los que vienen después, así que dos
escritores que añaden al mismo objeto nuevo aterrizan ambos. Estar en el
sitio es también lo que te cuesta un append: uno que falla o se aborta
deja el objeto atrás, vacío o corto, exactamente igual que ha ocurrido
siempre con un append sobre un objeto existente.

Una escritura condicional se publica con `link(2)` en lugar de con un
rename, lo que la mantiene como una creación exclusiva de verdad y no como
una comprobación seguida de una sobrescritura:

```rust,ignore
// Exactamente uno de cualquier número de llamadores en carrera obtiene Ok
// aquí. Todos los demás obtienen un error `ErrorKind::ConditionNotMatch` y
// no escriben nada.
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

Esa publicación necesita un sistema de archivos con enlaces duros. En FAT,
exFAT y algunos sistemas de archivos de red `link(2)` no está soportado, y
ahí una escritura condicional falla en lugar de degradar en silencio a una
comprobación seguida de una sobrescritura - lo que te entregaría una
garantía de exclusividad que no se sostiene. Ninguna otra operación se ve
afectada.

Publicar mediante rename reemplaza el inodo del objeto. Una reescritura no
preserva por tanto el modo, el propietario ni los enlaces duros del
archivo anterior, y un lector que mantiene un descriptor abierto sigue
leyendo el contenido antiguo en lugar de ver los bytes nuevos. Esa es la
contrapartida habitual de la publicación atómica, pero es un cambio si
dependías de cualquiera de las dos cosas.

Una ruta que llega al disco a través de un symlink que la salvaguarda no
puede resolver - uno roto, cuyo destino no existe - se rechaza en lugar de
tratarse como un nombre libre que crear. Crear a través de ese enlace
crearía el destino del enlace, en cualquier punto del host, así que la
salvaguarda no puede distinguir un enlace roto inofensivo de un intento de
fuga y rechaza ambos.

El nombre `.suprnova-atomic` está reservado en la raíz de todo disco
local. Toda ruta cuyo primer componente sea ese nombre se rechaza con un
error de permisos, y también toda ruta que se *resuelva* dentro del
directorio a través de un symlink, así que no puedes leer el archivo de
preparación de otro escritor, ni escribir dentro del directorio, ni
borrarlo. La entrada se filtra fuera de `files`, `directories`,
`all_files` y `all_directories`, así que nunca aparece como un objeto. El
nombre se exporta como `suprnova::ATOMIC_STAGING_DIR` porque las
herramientas de copia de seguridad y de sincronización lo necesitan:
excluye el directorio igual que excluirías un directorio de bloqueos.
Contiene archivos temporales en vuelo más lo que haya dejado atrás un
proceso que murió a mitad de una publicación, y nada barre eso, así que un
host en un bucle de caídas lo hará crecer hasta que alguien lo vacíe -
algo que es seguro hacer mientras nada esté escribiendo.

### Salvaguarda contra path traversal

A los discos de sistema de archivos local se les aplica un
`PathGuardLayer` antes que cualquier capa aportada por el usuario. Una
solicitud como `disk.write("../escaped.txt", ..)` se rechaza antes de
llegar al sistema operativo - ningún componente `..` ni prefijo absoluto
puede escapar de la raíz del disco. Los almacenes de objetos y el backend
en memoria no reciben la salvaguarda (en esos backends, una clave como
`../foo` no son más que caracteres de clave corrientes).

Tras rechazar los componentes `..` y los absolutos, la salvaguarda
canonicaliza la raíz del disco local y el destino solicitado en disco. Los
destinos existentes resuelven todos los componentes de symlink; para una
ruta que todavía no existe, la salvaguarda sube hasta el ancestro
existente más cercano y lo canonicaliza. La operación se rechaza si esa
ruta resuelta queda fuera de la raíz canónica, de modo que un symlink
dentro de la raíz observado durante la validación no puede redirigir una
lectura, escritura, listado, copia o renombrado fuera del disco.

Esta es una salvaguarda de canonicalizar-y-luego-operar, no un
confinamiento del sistema de archivos relativo a descriptores. Asume que
la raíz del disco y su contenido son de confianza frente a la mutación
concurrente: un atacante capaz de reemplazar directorios o symlinks
después de la validación pero antes de que el backend abra la ruta puede
ganar una carrera entre el momento de la comprobación y el del uso. Usa
aislamiento a nivel de sistema operativo o un sistema de archivos dedicado
cuando otros principales puedan mutar el árbol de almacenamiento de forma
concurrente.

Los escritores, listadores y copiadores en streaming hacen esta
comprobación de ruta resuelta una sola vez, justo antes de su primer I/O
contra el backend. La validación queda entonces fijada para esa sesión de
stream, de modo que cada chunk o ítem no se bloquea en la canonicalización
del sistema de archivos. Los abortos de copiador y de escritor siempre
reenvían la limpieza a sus backends, incluso antes de la activación o
cuando la validación ya no puede completarse.

## La superficie de disco al estilo Laravel

`Storage::disk(name)` devuelve un `suprnova::opendal::Operator`
directamente, así que puedes usar su superficie completa de streaming
(`writer`, `reader`, `presign_read`, `list`, `stat`, ...). Además de
eso, el trait [`DiskExt`] - con impl general (blanket impl) sobre
`Operator` y reexportado como `suprnova::DiskExt` - añade cada método
de conveniencia de Laravel al que echarías mano a través de
`Storage::disk('local')->...`.

Tráelo al alcance con `use suprnova::DiskExt;`.

### Comprobaciones de existencia

```rust,ignore
disk.exists("a.txt").await?;        // opendal en crudo
disk.missing("a.txt").await?;       // negación
disk.file_exists("a.txt").await?;   // solo archivo (no un directorio)
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### Lectura y escritura

| Nombre en Laravel | Equivalente nativo de Rust | Nota |
|--------------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` devuelve `Vec<u8>`; `read` devuelve el `Buffer` de opendal. |
| `put(path, contents)` | `write(path, contents)` | Ambos aceptan cualquier `Into<Bytes>`. |
| `json::<T>(path)` | - | Lee + deserializa vía serde_json. |
| `put_json(path, &value)` | - | Imprime con formato vía serde_json. |
| `prepend(path, data)` | - | Une con `\n`. Usa `prepend_with_separator` para una unión personalizada. |
| `append(path, data)`  | - | Une con `\n`. Usa `append_with_separator` para una unión personalizada. |

`prepend` y `append` crean el archivo si todavía no existe, así que son
seguros como primera escritura en un archivo de log.

### Metadatos

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` primero le pregunta al backend - S3, Azure y GCS
propagan el `Content-Type` almacenado. Si el backend no tiene uno,
inspecciona los primeros 16 KiB vía el crate `infer`. `Ok(None)` se
reserva para blobs binarios no reconocidos.

`checksum` admite `Md5`, `Sha1` y `Sha256` vía [`ChecksumAlgorithm`].
MD5 y SHA-1 se incluyen por paridad con Laravel y con los ETags de los
almacenes de objetos; elige SHA-256 para cualquier comprobación de
integridad nueva.

### Listados

```rust,ignore
let files = disk.files("docs", false).await?;     // archivos de nivel superior
let all   = disk.all_files("docs").await?;        // recursivo
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

Los cuatro devuelven `Vec<String>` ordenado, así que quien llama puede
confiar en un orden estable entre backends. Los directorios se filtran
fuera de `files`, y viceversa. Las rutas de directorio se devuelven
**sin** una barra final (`"docs/sub"`) para igualar la salida de
`Storage::directories()` de Laravel - el `list` subyacente de opendal
reporta `"docs/sub/"`, pero quitamos la barra por paridad.

### Mutar directorios y archivos

| Nombre en Laravel      | Nativo de opendal      |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to` recurre a `copy + delete` si el backend no admite `rename`,
y a `read + write + delete` si tampoco admite `copy` - así que
funciona tanto contra el driver en memoria usado en los tests como
contra los backends de producción.

### URLs firmadas temporalmente

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url` y `temporary_upload_url` devuelven la URL como
`String` por paridad con Laravel. Se apoyan en `Operator::presign_read`
/ `presign_write`, así que fallan con un mensaje `Unsupported` en los
backends que no implementan firma temporal (los drivers en memoria y
de sistema de archivos local caen en este grupo; S3, Azure Blob y GCS
sí la admiten).

## Copia en streaming entre discos

`copy_between_disks(src, src_path, dest, dest_path)` transmite el
objeto de origen hacia el destino en trozos de 64 KiB,
independientemente del par de backends. El origen y el destino pueden
estar respaldados por *cualquier* driver de opendal - de sistema de
archivos local a S3, de S3 a Azure Blob, de memoria a GCS, y así
sucesivamente.

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

Si algún paso falla a mitad de la copia, el objeto de destino parcial
se aborta y se elimina antes de que el error original se propague -
una copia fallida nunca es observable como un destino truncado.

## Discos read-through

Un disco read-through empareja un *primario* rápido con un *fallback* más
lento y va moviendo los objetos del segundo al primero a medida que se
leen. Apunta el primario al store al que estás migrando y el fallback a
aquel del que migras, y el conjunto de trabajo cruzará bajo tráfico
real - sin ventana de mantenimiento, sin copia masiva de objetos que
nadie pide.

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
// Lee `logo.png` de `legacy-store` y lo escribe en `new-store` de camino a
// la salida. Cada lectura posterior la sirve `new-store`.
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` devuelve un `Operator` corriente, así que todos
sus métodos y todas las comodidades de `DiskExt` funcionan sin cambios.

### Qué disco responde a cada operación

| Operación | Disco |
|---|---|
| `read` | El primario si tiene el objeto; si no, el fallback - y, salvo que `copy` sea `false`, el acierto en el fallback se promueve |
| `exists`, `size`, `last_modified`, `mime_type`, `stat` | El primario si tiene el objeto; si no, el fallback |
| `write`, `make_directory` | Solo el primario |
| `files`, `directories`, `list` | Solo el primario - las entradas del fallback son invisibles para un listado |
| `delete` | Ambos, el fallback primero |
| `copy`, `rename` / `move_to` | El primario si tiene el origen; si no, se transmite en streaming desde el fallback; un `rename` borra además el origen del fallback |
| `temporary_url` | El primario si tiene el objeto; si no, el fallback |
| `temporary_upload_url` | Solo el primario - una subida tiene que aterrizar donde aterrizan las escrituras |

El listado es solo del primario por diseño. Un listado unido tendría que
reconciliar la paginación y el orden entre dos backends, e informaría de
objetos que un listado posterior ya no devuelve una vez promovidos. Usa
`Storage::disk("legacy-store")` directamente cuando necesites enumerar lo
que queda en el fallback.

El borrado elimina el objeto de los dos discos. Si solo eliminara la copia
del primario, la siguiente lectura promovería la copia del fallback de
vuelta al instante. La consecuencia es que un disco read-through sobre un
fallback de solo lectura no puede borrar: el borrado en el fallback falla
y el error te llega.

### Cuando una promoción falla

Por defecto, un fallo de promoción se registra a nivel `warn` y se
absorbe. Sigues recibiendo los bytes que pediste; el disco simplemente se
degrada a leer el fallback cada vez, hasta que el primario vuelva a ser
escribible. Pon `throw_on_promotion_failure: true` cuando una pérdida
silenciosa de la promoción te escondería un fallo que necesitas ver - una
migración que estás intentando terminar, por ejemplo:

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

El registro rechaza una configuración que no puede funcionar: un `primary`
o un `fallback` vacíos, un par que nombra dos veces el mismo disco, un
disco que se nombra a sí mismo, o un nombre que no está registrado. Cada
caso devuelve un `FrameworkError` que nombra el problema, y no se registra
ningún disco.

### Leer sin promover

Pon `copy: false` para servir los aciertos del fallback sin escribirlos a
través:

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

### Copiar y mover a través del fallback

`copy` y `rename` resuelven el origen contra el primario primero. Cuando
solo lo tiene el fallback, el objeto se transmite en streaming en
fragmentos de 64 KiB y el destino aterriza en el primario:

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png` vive solo en `legacy-store`. La copia lo transmite y escribe
// `branding/logo.png` en `new-store`; el objeto heredado se queda donde
// está.
assets.copy("logo.png", "branding/logo.png").await?;

// Un movimiento hace lo mismo y después borra el origen heredado.
assets.rename("logo.png", "branding/logo.png").await?;
```

Un movimiento borra el origen del fallback por los dos caminos - tanto si
el primario tenía el origen como si no. Sin eso, la siguiente lectura
promovería de vuelta la copia del fallback y desharía el movimiento.

Los dos caminos se diferencian en cuándo lo borran, y esa diferencia es lo
que deja atrás un movimiento fallido:

- El primario tenía el origen. La copia del fallback cae primero, antes del
  rename. Mientras el primario tenga la ruta, la copia del fallback es
  inalcanzable a través de este disco, así que quitarla primero no cambia
  nada observable - y si el borrado falla, todavía no se ha movido nada.
  Reintenta el movimiento. Si en cambio el borrado tuvo éxito y luego falló
  el rename, el fallback no tiene nada para esa ruta, el destino está sin
  escribir y el primario sigue teniendo el origen, así que un reintento
  toma este mismo camino y vuelve a hacer el rename. El fallo cuesta la
  copia fría y nada más.
- Solo lo tenía el fallback. El borrado solo puede llegar después de que el
  destino esté en su sitio, así que un movimiento que falla en el borrado
  deja el destino escrito y el origen todavía en el fallback. Reintenta el
  movimiento; el origen ya está en el primario, así que el reintento toma
  el primer camino.

De un modo u otro, un movimiento fallido es seguro de reintentar, y el
destino con el que acabas es el objeto del que partió el movimiento.

Las condiciones viajan con la operación también por el camino de streaming.
`if_not_exists` se convierte en una escritura condicional, así que una copia
o un movimiento con guarda sigue rechazando un destino existente en lugar
de sobrescribirlo, y una copia que nombra una versión del origen obtiene
esa versión del fallback. El `if_match` de una copia es la única excepción:
es una condición que el backend aplica dentro de su propia copia, que es
justo la llamada que este camino no puede hacer, así que se rechaza con un
error `Unsupported` que nombra la condición, en lugar de ignorarse en
silencio.

Eso convierte a las condiciones en el único sitio donde se deja ver qué
disco tiene el origen. Un directorio local anuncia `copy` y `rename` pero
ninguna de sus formas condicionales, así que
`copy_with(a, b).if_not_exists(true)` funciona cuando solo el fallback
tiene `a` (se convierte en una escritura condicional) y se rechaza con
`Unsupported` cuando lo tiene el primario. Comprueba la condición que
necesitas contra el driver primario, en lugar de suponer que se cumple para
todos los objetos del disco.

Un movimiento que el primario rechazaría se rechaza antes de borrar nada.
Un primario sin `rename` en absoluto, un movimiento con guarda hacia un
primario sin `rename` condicional, y un movimiento con guarda hacia un
destino que ya existe fallan todos con el origen del fallback todavía en su
sitio - un movimiento que nunca ocurre no debe costarte la copia fría.

Si el stream falla a medias, se aborta el escritor y se borra el destino
que la transferencia creó antes de que el error te llegue, así que una
transferencia fallida no es observable como un objeto truncado. Un destino
que ya estaba ahí se deja en paz - una copia fallida no debe ser lo que
destruya un objeto que nunca escribió. Un primario de sistema de archivos
local también respeta eso, porque prepara la transferencia bajo
`.suprnova-atomic/` y solo renombra cuando tiene éxito; abortar el
escritor elimina el archivo preparado, así que una transferencia fallida
no deja ni un destino parcial ni un archivo temporal residual.

### Lecturas versionadas y condicionales

Una lectura que lleva una versión o una condición `If-Match`,
`If-None-Match`, `If-Modified-Since` o `If-Unmodified-Since` se transmite
con esa condición intacta, así que la respuesta significa lo que pediste
que significara. Una lectura así se sirve pero nunca se promueve: escribir
una versión antigua o un cuerpo que casa con el validador en el primario lo
publicaría como el objeto vivo, y toda lectura simple posterior lo
recibiría.

Qué disco responde a una de ellas se decide de la forma habitual. El primer
sondeo es una comprobación de existencia corriente, así que un disco
read-through delega una lectura versionada o condicional en el primario
siempre que el primario tenga la ruta; solo llega al fallback cuando el
primario no la tiene.

El primario decide además cuáles de estas acepta siquiera un disco
read-through, porque el lector del primario se abre primero. Una lectura
versionada contra un disco read-through cuyo primario es un directorio
local se rechaza antes de llegar al fallback, ya que un directorio local no
tiene versiones.

### Por qué Suprnova diverge

Laravel construye un disco read-through a partir de una entrada de
`config/filesystems.php` cuyas claves `primary` y `fallback` aceptan o bien
un nombre de disco o bien una configuración de driver en línea. Suprnova
toma solo nombres de disco, porque aquí los discos se registran mediante
constructores tipados en lugar de describirse con arrays: registra primero
el disco interior y después nómbralo.

La promoción de Laravel vuelve a comprobar el primario después de leer el
fallback, con lo que gana un escritor concurrente. Suprnova mantiene esa
comprobación y publica la promoción de forma atómica, cosa que Laravel no
hace. Sobre un primario de sistema de archivos local, los bytes se preparan
en un archivo temporal hermano y se renombran a su sitio; escribirlos
directamente al destino dejaría un archivo a medio escribir y creciendo,
visible durante toda la escritura, y un disco read-through enruta a los
lectores por exactamente esa comprobación de existencia. Sobre un primario
sin rename - en memoria, S3, Azure Blob, GCS - una escritura ya es una
publicación única e indivisible, así que la promoción escribe el destino
directamente, condicionada a que el objeto no exista ya, para que dos
lectores concurrentes no promuevan los dos.

Esa condición es la parte que una promoción preparada no puede tener: la
ruta de preparación es única, así que una condición de no sobrescribir
sobre ella sería vacua, y el destino se publica mediante un rename que
sobrescribe. Un disco read-through sobre un primario de sistema de archivos
local renuncia por tanto a ella - una escritura que aterriza en el primario
en el instante que va entre la última comprobación de existencia de la
promoción y su rename queda sobrescrita por la copia promovida. Sobre un
primario sin rename la condición se cumple y no existe tal ventana.

El objeto de preparación es una entrada real del primario mientras dura,
así que un listado tomado a mitad de una promoción puede mostrar un
hermano `.suprnova-promote-<id>.tmp`. Una lectura que se completa, falla o
se rinde intenta quitar su propio hermano, y registra una advertencia si
ese borrado falla, en lugar de hacer fallar la lectura. Nada barre un
hermano dejado por un borrado fallido, por un proceso que se cayó o por un
futuro de lectura cancelado a mitad de la promoción: esos hay que quitarlos
a mano.

Una lectura que se resuelve desde el fallback mantiene el objeto en memoria
hasta que se completa la escritura de la promoción, porque la promoción
necesita el objeto entero. Eso encaja con el caso de almacenamiento por
niveles para el que existe un disco read-through. Para objetos fríos muy grandes,
lee el disco de fallback directamente o usa
[`copy_between_disks`](#copia-en-streaming-entre-discos) en su lugar.

Laravel devuelve el propio stream del fallback cuando `copy` es `false` y
almacena en búfer a través de `php://temp` cuando es `true`. Suprnova, en
cambio, estrecha la obtención del fallback al rango pedido cuando `copy` es
`false`, y solo almacena en búfer en el camino que promueve, donde de todas
formas hace falta el objeto entero.

El `copy` y el `move` entre fallbacks de Laravel también almacenan el
origen en búfer a través de `php://temp`. Suprnova lo transmite en
fragmentos de 64 KiB en su lugar, porque el fallback es donde viven los
objetos grandes y poco tocados, y borra un destino a medio escribir antes
de devolver el error. De OpenDAL se siguen dos diferencias más. Borrar una
ruta que no está ahí cuenta como éxito, así que un movimiento limpia el
origen del fallback sin comprobar antes que exista. Y OpenDAL lleva
condiciones sobre `copy` y `rename` para las que Flysystem no tiene
equivalente, así que Suprnova tiene que decidir qué significa cada una
cuando el origen está solo en el fallback: `if_not_exists` y la versión de
origen de una copia se respetan, y el `if_match` de una copia se rechaza en
lugar de descartarse.

Laravel borra el origen del fallback después del movimiento por los dos
caminos. Suprnova lo borra primero cuando el primario tiene el origen,
porque los dos órdenes se diferencian bajo un reintento: el origen es
inalcanzable a través del disco en cualquier caso, pero borrar al final
hace que un movimiento que perdió su borrado por un fallo transitorio
vuelva convertido en un movimiento cuyo origen ahora está solo en el
fallback, y transmita la copia obsoleta del fallback por encima del destino
que el primer intento ya escribió correctamente.

## Higiene del registro

```rust,ignore
let removed = Storage::forget("local");  // bool: ¿estaba presente?
Storage::purge();                        // elimina cada disco
let names = Storage::disks();            // Vec<String>, ordenado
```

Estos reflejan `FilesystemManager::forgetDisk` / `purge` de Laravel y
son útiles para las recargas de configuración y los paneles de
administración. No son solo para tests: el código de producción
ocasionalmente necesita eliminar y volver a registrar un disco en
tiempo de ejecución (por ejemplo, tras una rotación de secretos).

## Pruebas

`Storage::fake()` devuelve una guarda que:

1. Adquiere un mutex global de proceso para que los casos
   `#[tokio::test]` concurrentes no compitan por el registro
   compartido, y
2. Reinicia el registro en su construcción y en su drop, dejando la
   suite en un estado limpio para el test que se ejecute a
   continuación.

Un disco de memoria `"default"` viene preregistrado por comodidad.

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

Los cinco ayudantes de aserción - `assert_exists`, `assert_contents`,
`assert_missing`, `assert_count`, `assert_directory_empty` - se
exponen a través del trait [`DiskAssertExt`], detrás de
`#[cfg(any(test, feature = "testing"))]` para que el código de
producción no pueda usarlos.

## Referencia rápida de paridad

| `Storage::disk(...)->...` en Laravel  | Suprnova                                                 |
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
| `move($from, $to)`                    | `disk.move_to(from, to)` (o el `rename` nativo de opendal) |
| `copy($from, $to)`                    | `disk.copy(from, to)` (nativo de opendal)                |
| `delete($path)`                       | `disk.delete(path)` (nativo de opendal)                  |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (o el `presign_read` nativo de opendal) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (o el `presign_write` nativo de opendal) |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## Configuración

La configuración de almacenamiento vive por completo en código Rust,
no en `.env`. Los discos se registran por nombre en `bootstrap()` vía
`Storage::register_*` y se referencian por nombre en el sitio de
llamada (`Storage::disk("public")`). No hay ninguna variable de
entorno `FILESYSTEM_DISK` que el framework lea, ni ningún disco por
defecto implícito - cada driver es un igual. Las apps deciden a qué
nombre de disco apunta una subida o descarga dada, y pasan cualquier
URL / clave / credencial que el driver elegido necesite como sus
propias variables de entorno.

Consulta [Configuración](configuration.md) para la regla más amplia
sobre dónde lee el framework del entorno frente a dónde espera un
registro del lado del código.

## Siguiente

- [Configuración](configuration.md) - qué lee el framework de `.env`
  (y por qué el almacenamiento no está en esa lista)
- [Solicitudes](requests.md) - las subidas de archivos aterrizan en un
  disco vía `UploadedFile::store_as`
- [Respuestas](responses.md) - transmitir bytes de vuelta desde un
  disco
- [Caché](cache.md) - el otro registro de driver nombrado, con la
  misma forma
- [Pruebas](testing.md) - la superficie de pruebas más amplia, con
  fakes para todo

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html
