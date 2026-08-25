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

`filesystem` está activada por defecto; las features de Azure y GCS no.
Actívalas en tu `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.2", features = ["filesystem-gcs"] }
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
