# Système de fichiers et stockage

La façade de stockage de Suprnova offre une API à disque nommé unique
par-dessus les systèmes de fichiers locaux, les backends en mémoire et
les principaux services de stockage d'objets (S3, Azure Blob, Google
Cloud Storage). Sous le capot, elle repose sur
[`opendal`](https://docs.rs/opendal) - mais la surface consommateur est
façonnée pour correspondre aux appels `Storage::disk(...)` de Laravel,
si bien que les réflexes PHP se transposent directement.

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

## Enregistrement des disques

Chaque disque est enregistré une seule fois à l'amorçage via
`Storage::register_*` et retrouvé par son nom via
`Storage::disk(name)`. Il n'existe pas de « backend par défaut » vers
lequel les autres se dégraderaient - chaque driver est un pair.

| Constructeur | Backend | Feature |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Système de fichiers local     | `filesystem`        |
| `Storage::register_memory(name)`     | Mémoire en process (tests)    | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 ou compatible S3    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |

`filesystem` est activée par défaut ; les features Azure et GCS ne le
sont pas. Activez-en une dans votre `Cargo.toml` :

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4", features = ["filesystem-gcs"] }
```

Sans la feature, `register_azblob` / `register_gcs` et leurs structs de
config n'existent pas - vous obtenez une erreur de compilation nommant
l'élément manquant, pas un échec à l'exécution.

Chaque constructeur a une variante `_with` qui vous remet le
`suprnova::opendal::Operator` juste avant qu'il n'atterrisse dans le
registre, pour que vous puissiez poser des couches
retry/timeout/logging autour de lui :

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

Les constructeurs cloud (`register_s3`, `register_azblob`,
`register_gcs`) appliquent par défaut une `RetryLayer` (3 tentatives),
car les erreurs de throttling transitoire ou 5xx sont courantes sur les
services de stockage d'objets. Utilisez les variantes `_with` quand
vous avez besoin d'un contrôle complet.

L'ensemble complet des couches opendal câblées par Suprnova est
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (relie à
OTel via `tracing-opentelemetry` quand la feature `otel` du framework
est activée), et `PrometheusClientLayer` (exporte des histogrammes et
des compteurs dans un `prometheus_client::registry::Registry` que vous
possédez). L'ordre des couches compte - la couche la plus externe
enveloppe tout ce qu'elle contient - et la pile idiomatique est
`RetryLayer → TimeoutLayer → LoggingLayer`, si bien qu'une tentative
qui expire journalise quand même et qu'une relance couvre les échecs
de transport.

Ré-enregistrer le même nom remplace l'opérateur précédent et émet un
journal `warn!` - les disques sont censés être enregistrés une seule
fois à l'amorçage, et un doublon accidentel pourrait échanger un disque
de production contre un disque en mémoire. Le remplacement a bien lieu
quand même ; l'avertissement rend juste l'échange audible.

### Pourquoi Suprnova diverge

Le `config/filesystems.php` de Laravel liste chaque driver de disque et
vous en choisissez un à l'exécution ; rien n'est retiré à la
compilation. Suprnova place Azure et GCS derrière des features car, en
Rust, le choix a un coût de dépendance, et celui-ci a en plus une
dimension sécurité : les deux crates de service opendal tirent `rsa`,
qui porte
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)
(l'attaque temporelle de Marvin) sans version corrigée en amont. Les
rendre opt-in signifie qu'une application qui stocke ses fichiers en
local ou sur S3 ne porte jamais cette crate.

S3 n'est délibérément *pas* filtré derrière une feature - son
signataire n'a jamais dépendu de `rsa`, donc le filtrer casserait le
backend cloud le plus utilisé sans rien retirer.

### Garde-fou contre la traversée de chemin

Les disques de système de fichiers local ont une `PathGuardLayer`
appliquée avant toute couche fournie par l'utilisateur. Une requête
comme `disk.write("../escaped.txt", ..)` est rejetée avant d'atteindre
l'OS - aucun composant `..` ni préfixe absolu ne peut s'échapper de la
racine du disque. Les services de stockage d'objets et le backend en
mémoire n'ont pas ce garde-fou (une clé comme `../foo` n'est qu'un
caractère de clé ordinaire sur ces backends).

Après avoir rejeté les composants `..` et absolus, le garde-fou
canonicalise la racine du disque local et la cible visée sur le
disque. Pour une cible existante, chaque composant lien symbolique est
résolu ; pour un chemin qui n'existe pas encore, le garde-fou remonte
jusqu'au plus proche ancêtre existant et le canonicalise. L'opération
est rejetée si le chemin résolu se trouve hors de la racine canonique,
si bien qu'un lien symbolique dans la racine observé pendant la
validation ne peut pas rediriger une lecture, une écriture, un
listage, une copie ou un renommage hors du disque.

C'est un garde-fou de type canonicaliser-puis-opérer, pas un
confinement de système de fichiers relatif à un descripteur. Il
suppose que la racine du disque et son contenu sont sûrs face à une
mutation concurrente : un attaquant capable de remplacer des
répertoires ou des liens symboliques après la validation mais avant
que le backend n'ouvre le chemin peut gagner une course
time-of-check-to-time-of-use. Utilisez une isolation au niveau de l'OS
ou un système de fichiers dédié quand d'autres principaux peuvent
muter l'arbre de stockage en concurrence.

Les writers, listers et copiers en streaming effectuent cette
vérification du chemin résolu une seule fois, juste avant leur premier
E/S sur le backend. La validation est alors fixée pour cette session
de flux, si bien que chaque bloc ou élément ne bloque pas sur la
canonicalisation du système de fichiers. Les abandons de copier et de
writer transmettent toujours le nettoyage à leurs backends, même avant
l'activation ou quand la validation ne peut plus se terminer.

## La surface de disque à la Laravel

`Storage::disk(name)` retourne directement un
`suprnova::opendal::Operator`, pour que vous puissiez utiliser toute sa
surface de streaming (`writer`, `reader`, `presign_read`, `list`,
`stat`, ...). Par-dessus, le trait [`DiskExt`] - implémenté en bloc sur
`Operator` et ré-exporté comme `suprnova::DiskExt` - ajoute chaque
méthode de confort Laravel que vous iriez chercher via
`Storage::disk('local')->...`.

Ramenez-le dans la portée avec `use suprnova::DiskExt;`.

### Vérifications d'existence

```rust,ignore
disk.exists("a.txt").await?;        // opendal brut
disk.missing("a.txt").await?;       // négation
disk.file_exists("a.txt").await?;   // fichier seulement (pas un répertoire)
disk.file_missing("a.txt").await?;
disk.directory_exists("dir/").await?;
disk.directory_missing("dir/").await?;
```

### Lecture et écriture

| Nom Laravel | Équivalent Rust natif | Notes |
|--------------|------------------------|------|
| `get(path)`  | `read(path)`           | `get` retourne un `Vec<u8>` ; `read` retourne le `Buffer` d'opendal. |
| `put(path, contents)` | `write(path, contents)` | Les deux acceptent tout `Into<Bytes>`. |
| `json::<T>(path)` | - | Lit et désérialise via serde_json. |
| `put_json(path, &value)` | - | Écrit du JSON indenté via serde_json. |
| `prepend(path, data)` | - | Concatène avec `\n`. Utilisez `prepend_with_separator` pour un séparateur personnalisé. |
| `append(path, data)`  | - | Concatène avec `\n`. Utilisez `append_with_separator` pour un séparateur personnalisé. |

`prepend` et `append` créent le fichier s'il n'existe pas encore, ils
sont donc sûrs comme première écriture dans un fichier de log.

### Métadonnées

```rust,ignore
let bytes  = disk.size("a.bin").await?;          // u64
let when   = disk.last_modified("a.bin").await?; // Option<DateTime<Utc>>
let mime   = disk.mime_type("a.bin").await?;     // Option<String>
let digest = disk.checksum("a.bin", ChecksumAlgorithm::Sha256).await?;
```

`mime_type` interroge d'abord le backend - S3, Azure et GCS
transmettent le `Content-Type` stocké. Si le backend n'en a pas, elle
renifle les 16 premiers Kio via la crate `infer`. `Ok(None)` est
réservé aux blobs binaires non reconnus.

`checksum` prend en charge `Md5`, `Sha1` et `Sha256` via
[`ChecksumAlgorithm`]. MD5 et SHA-1 sont inclus pour la parité avec
Laravel et les ETags des services de stockage d'objets ; choisissez
SHA-256 pour toute nouvelle vérification d'intégrité.

### Listage

```rust,ignore
let files = disk.files("docs", false).await?;     // fichiers de premier niveau
let all   = disk.all_files("docs").await?;        // récursif
let dirs  = disk.directories("docs", false).await?;
let all   = disk.all_directories("docs").await?;
```

Les quatre retournent un `Vec<String>` trié, pour que les appelants
puissent compter sur un ordre stable entre les backends. Les
répertoires sont filtrés hors de `files`, et vice-versa. Les chemins
de répertoire sont retournés **sans** slash final (`"docs/sub"`) pour
correspondre à la sortie de `Storage::directories()` de Laravel - le
`list` sous-jacent d'opendal rapporte `"docs/sub/"`, mais nous retirons
le slash pour la parité.

### Modifier les répertoires et fichiers

| Nom Laravel           | Natif opendal        |
|------------------------|-----------------------|
| `make_directory(path)` | `create_dir(path)`    |
| `delete_directory(p)`  | `delete_with(p).recursive(true)` |
| `move_to(from, to)`    | `rename(from, to)`    |

`move_to` retombe sur `copy + delete` si le backend ne prend pas en
charge `rename`, et sur `read + write + delete` s'il ne prend pas non
plus en charge `copy` - ainsi il fonctionne aussi bien contre le driver
en mémoire utilisé dans les tests que contre les backends de
production.

### URL pré-signées

```rust,ignore
let read_url   = disk.temporary_url("uploads/a.pdf", Duration::from_secs(900)).await?;
let upload_url = disk.temporary_upload_url("uploads/new.pdf", Duration::from_secs(900)).await?;
```

`temporary_url` et `temporary_upload_url` retournent l'URL comme une
`String`, pour la parité avec Laravel. Elles s'appuient sur
`Operator::presign_read` / `presign_write`, elles échouent donc avec un
message `Unsupported` sur les backends qui n'implémentent pas la
pré-signature (les drivers en mémoire et système de fichiers local en
font partie ; S3, Azure Blob et GCS la prennent en charge).

## Copie en streaming entre disques

`copy_between_disks(src, src_path, dest, dest_path)` diffuse l'objet
source vers la destination par blocs de 64 Kio, quelle que soit la
paire de backends. La source et la destination peuvent être adossées à
*n'importe quel* driver opendal - système de fichiers local vers S3, S3
vers Azure Blob, mémoire vers GCS, et ainsi de suite.

```rust,ignore
use suprnova::filesystem::streaming::copy_between_disks;

Storage::register_fs("local", "./storage")?;
Storage::register_memory("scratch");
let bytes = copy_between_disks("local", "uploads/big.bin", "scratch", "big.bin").await?;
```

Si une étape échoue en cours de copie, l'objet de destination partiel
est abandonné et supprimé avant que l'erreur d'origine ne se propage -
une copie échouée n'est jamais observable comme une destination
tronquée.

## Hygiène du registre

```rust,ignore
let removed = Storage::forget("local");  // bool : était-il présent ?
Storage::purge();                        // supprime tous les disques
let names = Storage::disks();            // Vec<String>, trié
```

Ils reflètent les `FilesystemManager::forgetDisk` / `purge` de Laravel
et sont utiles pour les rechargements de configuration et les tableaux
de bord d'administration. Ils ne sont pas réservés aux tests : le code
de production a parfois besoin de supprimer puis de ré-enregistrer un
disque à l'exécution (par exemple après une rotation de secrets).

## Tests

`Storage::fake()` retourne une garde qui :

1. Acquiert un mutex global au process, pour que les cas
   `#[tokio::test]` concurrents ne se disputent pas le registre
   partagé, et
2. Réinitialise le registre à la construction et au drop, laissant la
   suite dans un état propre pour le prochain test qui s'exécute.

Un disque en mémoire `"default"` est pré-enregistré par commodité.

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

Les cinq aides d'assertion - `assert_exists`, `assert_contents`,
`assert_missing`, `assert_count`, `assert_directory_empty` - sont
exposées via le trait [`DiskAssertExt`], filtré par
`#[cfg(any(test, feature = "testing"))]` pour que le code de production
ne puisse pas s'en servir.

## Référence rapide de parité

| Laravel `Storage::disk(...)->...`     | Suprnova                                                 |
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
| `move($from, $to)`                    | `disk.move_to(from, to)` (ou `rename` natif d'opendal)   |
| `copy($from, $to)`                    | `disk.copy(from, to)` (natif opendal)                    |
| `delete($path)`                       | `disk.delete(path)` (natif opendal)                      |
| `temporaryUrl($path, $expiry)`        | `disk.temporary_url(path, expire)` (ou `presign_read` natif d'opendal) |
| `temporaryUploadUrl($path, $expiry)`  | `disk.temporary_upload_url(path, expire)` (ou `presign_write` natif d'opendal) |
| `Storage::fake()`                     | `Storage::fake()`                                        |
| `Storage::disk()->assertExists()`     | `disk.assert_exists(path).await`                         |
| `FilesystemManager::forgetDisk($n)`   | `Storage::forget(name)`                                  |
| `FilesystemManager::purge()`          | `Storage::purge()`                                       |

## Configuration

La configuration du stockage vit entièrement dans le code Rust, pas
dans `.env`. Les disques sont enregistrés par nom dans `bootstrap()`
via `Storage::register_*` et adressés par leur nom au site d'appel
(`Storage::disk("public")`). Il n'y a pas de variable d'env
`FILESYSTEM_DISK` que le framework lirait, ni de disque par défaut
implicite - chaque driver est un pair. Les applications décident quel
nom de disque cible un téléversement ou un téléchargement donné, et
transmettent les URL / clés / identifiants dont le driver choisi a
besoin comme leurs propres variables d'env.

Voir [Configuration](configuration.md) pour la règle plus large sur où
le framework lit depuis l'environnement, par opposition à où il attend
un enregistrement côté code.

## Suivant

- [Configuration](configuration.md) - ce que le framework lit depuis
  `.env` (et pourquoi le stockage n'est pas sur cette liste)
- [Requêtes](requests.md) - les téléversements de fichiers atterrissent
  sur un disque via `UploadedFile::store_as`
- [Réponses](responses.md) - diffuser des octets en streaming depuis
  un disque
- [Cache](cache.md) - l'autre registre à driver nommé, même forme
- [Tests](testing.md) - la surface de test plus large qui fake tout

[`DiskExt`]: https://docs.rs/suprnova/latest/suprnova/trait.DiskExt.html
[`DiskAssertExt`]: https://docs.rs/suprnova/latest/suprnova/filesystem/testing/trait.DiskAssertExt.html
[`ChecksumAlgorithm`]: https://docs.rs/suprnova/latest/suprnova/enum.ChecksumAlgorithm.html
