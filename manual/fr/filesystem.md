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

## Enregistrer des disques

Chaque disque est enregistré une fois au démarrage via
`Storage::register_*` et retrouvé par nom via `Storage::disk(name)`.
Il n'y a pas de « backend par défaut » vers lequel les autres se
dégradent - chaque driver est un pair.

| Constructeur                         | Backend                       | Feature             |
|--------------------------------------|-------------------------------|---------------------|
| `Storage::register_fs(name, root)`   | Système de fichiers local     | `filesystem`        |
| `Storage::register_memory(name)`     | Mémoire in-process (tests)    | `filesystem`        |
| `Storage::register_s3(name, cfg)`    | Amazon S3 ou compatible S3    | `filesystem`        |
| `Storage::register_azblob(name, cfg)`| Azure Blob Storage            | `filesystem-azure`  |
| `Storage::register_gcs(name, cfg)`   | Google Cloud Storage          | `filesystem-gcs`    |
| `Storage::register_read_through(name, cfg)` | Composite read-through | `filesystem` |

`filesystem` est activée par défaut ; les features Azure et GCS ne le
sont pas. Activez-en une dans votre `Cargo.toml` :

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.5", features = ["filesystem-gcs"] }
```

Sans la feature, `register_azblob` / `register_gcs` et leurs structs
de configuration n'existent pas - vous obtenez une erreur de
compilation qui nomme l'élément manquant, pas un échec à l'exécution.

Chaque constructeur a une variante `_with` qui vous remet le
`suprnova::opendal::Operator` juste avant qu'il n'atterrisse dans le
registre, afin que vous puissiez installer autour de lui des couches
de réessai/expiration/journalisation :

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
`register_gcs`) appliquent une `RetryLayer` (3 tentatives) par défaut,
puisque le bridage transitoire et les erreurs 5xx sont routiniers sur
les stockages d'objets. Utilisez les variantes `_with` quand vous avez
besoin d'un contrôle total.

L'ensemble complet des couches opendal câblées par Suprnova est
`RetryLayer`, `TimeoutLayer`, `LoggingLayer`, `TracingLayer` (fait le
pont vers OTel via `tracing-opentelemetry` quand la feature `otel` du
framework est activée) et `PrometheusClientLayer` (exporte des
histogrammes et des compteurs dans un
`prometheus_client::registry::Registry` qui vous appartient). L'ordre
des couches compte - la couche la plus externe enveloppe tout ce qui
se trouve à l'intérieur - et la pile idiomatique est `RetryLayer →
TimeoutLayer → LoggingLayer`, si bien qu'une tentative expirée est
tout de même journalisée et qu'un réessai couvre les défaillances de
transport.

Ré-enregistrer le même nom remplace l'opérateur précédent et émet un
journal `warn!` - les disques sont censés être enregistrés une fois au
démarrage, et un doublon accidentel pourrait échanger un disque de
production contre un disque mémoire. Le remplacement a tout de même
lieu ; l'avertissement rend simplement l'échange audible.

### Pourquoi Suprnova diverge

Le `config/filesystems.php` de Laravel liste chaque driver de disque
et vous en choisissez un à l'exécution ; rien n'est compilé hors de
l'image. Suprnova conditionne Azure et GCS à des features parce qu'en
Rust ce choix a un coût en dépendances, et celui-ci a une dimension de
sécurité : les deux crates de service opendal tirent `rsa`, qui porte
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)
(l'attaque temporelle Marvin) sans version corrigée en amont. Les
rendre optionnels signifie qu'une application qui stocke ses fichiers
localement ou sur S3 ne porte jamais cette crate.

S3 n'est délibérément *pas* conditionné - son signataire n'a jamais
dépendu de `rsa`, donc le conditionner casserait le backend cloud le
plus utilisé sans rien retirer.

### Écritures atomiques locales

Sur un disque local, toute opération qui publie des octets sur un chemin
les publie en une seule étape. `disk.write(...)`, `disk.writer(...)` et
`disk.copy(...)` écrivent d'abord dans `<root>/.suprnova-atomic/`, y
vident et synchronisent leurs octets, puis renomment le fichier sur la
cible ; `disk.rename(...)` est déjà une étape unique. Un lecteur
concurrent voit donc soit l'objet précédent, soit le nouvel objet
terminé, jamais une longueur partielle, et un processus qui meurt en
pleine écriture laisse la cible intacte plutôt que tronquée sur le
chemin en service.

`append` est la seule opération sur place, parce que préparer un ajout
imposerait de copier d'abord l'objet entier. Cela vaut pour l'ajout qui
*crée* l'objet autant que pour chacun des ajouts suivants, si bien que
deux writers qui ajoutent au même objet encore inexistant aboutissent
tous les deux. Ce fonctionnement sur place est aussi ce qu'un ajout vous
coûte : un ajout qui échoue ou qui est abandonné laisse l'objet derrière
lui, vide ou incomplet, exactement comme un ajout sur un objet existant
l'a toujours fait.

Une écriture conditionnelle est publiée avec `link(2)` plutôt qu'avec un
renommage, ce qui en fait une vraie création exclusive et non une
vérification suivie d'un écrasement :

```rust,ignore
// Un seul appelant parmi tous ceux en concurrence obtient Ok ici. Chacun des
// autres obtient une erreur `ErrorKind::ConditionNotMatch` et n'écrit rien.
disk.write_with("locks/import.json", body).if_not_exists(true).await?;
```

Cette publication exige un système de fichiers doté de liens physiques.
Sur FAT, exFAT et certains systèmes de fichiers réseau, `link(2)` n'est
pas pris en charge, et une écriture conditionnelle y échoue plutôt que
de se dégrader silencieusement en une vérification suivie d'un
écrasement - ce qui vous donnerait une garantie d'exclusivité qui ne
tient pas. Aucune autre opération n'est affectée.

Publier par renommage remplace l'inode de l'objet. Une réécriture ne
préserve donc ni le mode, ni le propriétaire, ni les liens physiques du
fichier précédent, et un lecteur qui détient un descripteur ouvert
continue de lire l'ancien contenu au lieu de voir les nouveaux octets.
C'est le compromis habituel de la publication atomique, mais c'est un
changement si vous comptiez sur l'un ou l'autre.

Un chemin qui atteint le disque à travers un lien symbolique que le
garde-fou ne peut pas résoudre - un lien orphelin, dont la cible
n'existe pas - est rejeté plutôt que traité comme un nom libre à créer.
Créer à travers un tel lien créerait la cible du lien, n'importe où sur
l'hôte : le garde-fou ne peut donc pas distinguer un lien orphelin
inoffensif d'une évasion, et refuse les deux.

Le nom `.suprnova-atomic` est réservé à la racine de chaque disque
local. Tout chemin dont le premier composant porte ce nom est rejeté par
une erreur de permission, et il en va de même pour tout chemin qui
*se résout* dans ce répertoire à travers un lien symbolique : vous ne
pouvez donc ni lire le fichier de préparation d'un autre writer, ni
écrire dans le répertoire, ni le supprimer. L'entrée est filtrée de
`files`, `directories`, `all_files` et `all_directories`, si bien
qu'elle n'apparaît jamais comme un objet. Le nom est exporté sous
`suprnova::ATOMIC_STAGING_DIR` parce que les outils de sauvegarde et de
synchronisation en ont besoin : excluez ce répertoire comme vous
excluriez un répertoire de verrous. Il contient les fichiers temporaires
en cours ainsi que ce qu'un processus mort en pleine publication a
laissé derrière lui, et rien ne les purge : un hôte en boucle de
plantage le fera donc grossir jusqu'à ce que quelqu'un le vide - ce que
l'on peut faire sans risque tant que rien n'écrit.

### Garde-fou contre la traversée de chemin

Les disques de système de fichiers local reçoivent une
`PathGuardLayer` appliquée avant toute couche fournie par
l'utilisateur. Une requête comme `disk.write("../escaped.txt", ..)`
est rejetée avant d'atteindre le système d'exploitation - aucun
composant `..` ni préfixe absolu ne peut s'échapper de la racine du
disque. Les stockages d'objets et le backend en mémoire ne reçoivent
pas le garde-fou (une clé comme `../foo` n'est qu'un caractère de clé
ordinaire sur ces backends).

Après avoir rejeté `..` et les composants absolus, le garde-fou
canonicalise la racine du disque local et la cible demandée sur
disque. Les cibles existantes résolvent chaque composant lien
symbolique ; pour un chemin qui n'existe pas encore, le garde-fou
remonte jusqu'à l'ancêtre existant le plus proche et le canonicalise.
L'opération est rejetée si ce chemin résolu se trouve hors de la
racine canonique, si bien qu'un lien symbolique interne à la racine
observé pendant la validation ne peut pas rediriger une lecture, une
écriture, un listage, une copie ou un renommage hors du disque.

C'est un garde-fou du type canonicaliser-puis-opérer, pas un
confinement du système de fichiers relatif aux descripteurs. Il
suppose que la racine du disque et son contenu sont de confiance face
aux mutations concurrentes : un attaquant capable de remplacer des
répertoires ou des liens symboliques après la validation mais avant
que le backend n'ouvre le chemin peut gagner une course entre le
moment de la vérification et le moment de l'utilisation. Utilisez une
isolation au niveau du système d'exploitation ou un système de
fichiers dédié quand d'autres entités peuvent muter l'arbre de
stockage en parallèle.

Les writers, listers et copiers en streaming effectuent cette
vérification de chemin résolu une seule fois, immédiatement avant leur
première E/S sur le backend. La validation est ensuite figée pour
cette session de flux, si bien que chaque morceau ou élément ne bloque
pas sur la canonicalisation du système de fichiers. Les abandons de
copier et de writer transmettent toujours le nettoyage à leurs
backends, même avant l'activation ou lorsque la validation ne peut
plus aboutir.

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

## Disques read-through

Un disque read-through associe un *primaire* rapide à un *repli* plus
lent et déplace les objets du second vers le premier à mesure qu'ils
sont lus. Pointez le primaire vers le magasin vers lequel vous migrez
et le repli vers celui depuis lequel vous migrez : l'ensemble actif
bascule sous trafic réel - pas de fenêtre de maintenance, pas de copie
en masse d'objets que personne ne demande.

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
// Lit `logo.png` depuis `legacy-store` et l'écrit sur `new-store` au
// passage. Chaque lecture ultérieure est servie par `new-store`.
let bytes = assets.read("logo.png").await?;
```

`Storage::disk("assets")` retourne un `Operator` ordinaire : toutes ses
méthodes et toutes les commodités de `DiskExt` fonctionnent sans
changement.

### Quel disque répond à quelle opération

| Opération | Disque |
|---|---|
| `read` | Le primaire s'il détient l'objet, sinon le repli - et, sauf si `copy` vaut `false`, l'objet trouvé sur le repli est promu |
| `exists`, `size`, `last_modified`, `mime_type`, `stat` | Le primaire s'il détient l'objet, sinon le repli |
| `write`, `make_directory` | Le primaire seulement |
| `files`, `directories`, `list` | Le primaire seulement - les entrées du repli sont invisibles pour un listage |
| `delete` | Les deux, le repli d'abord |
| `copy`, `rename` / `move_to` | Le primaire s'il détient la source, sinon la source est transférée en flux depuis le repli ; un `rename` supprime en plus la source sur le repli |
| `temporary_url` | Le primaire s'il détient l'objet, sinon le repli |
| `temporary_upload_url` | Le primaire seulement - un upload doit atterrir là où atterrissent les écritures |

Le listage ne porte que sur le primaire, par conception. Un listage unifié
devrait réconcilier la pagination et l'ordre entre deux backends, et il
signalerait des objets qu'un listage ultérieur ne retourne plus une
fois qu'ils ont été promus. Utilisez directement
`Storage::disk("legacy-store")` quand vous avez besoin d'énumérer ce
qui reste sur le repli.

La suppression retire l'objet des deux disques. Si elle ne retirait que
la copie du primaire, la lecture suivante repromouvrait aussitôt la
copie du repli. La conséquence est qu'un disque read-through par-dessus
un repli en lecture seule ne peut pas supprimer : la suppression sur le
repli échoue et l'erreur vous parvient.

### Quand une promotion échoue

Par défaut, un échec de promotion est journalisé en `warn` et avalé.
Vous recevez quand même les octets que vous avez demandés ; le disque
se dégrade simplement en lisant le repli à chaque fois, jusqu'à ce que
le primaire redevienne inscriptible. Positionnez
`throw_on_promotion_failure: true` quand une perte silencieuse de
promotion masquerait une panne que vous devez voir - une migration que
vous essayez de terminer, par exemple :

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

L'enregistrement rejette une configuration qui ne peut pas fonctionner :
un `primary` ou un `fallback` vide, une paire qui nomme deux fois le
même disque, un disque qui se nomme lui-même, ou un nom qui n'est pas
enregistré. Chacun retourne une `FrameworkError` nommant le problème,
et aucun disque n'est enregistré.

### Lire sans promouvoir

Positionnez `copy: false` pour servir ce qui est trouvé sur le repli
sans l'écrire au passage :

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

Le disque se lit alors comme une surcouche transparente : le primaire
répond pour ce qu'il détient, le repli répond pour tout le reste, et
rien ne bouge entre les deux. Employez-le quand le primaire est un
petit cache que vous ne voulez pas voir rempli par une lecture unique,
ou quand le repli fait autorité et que le primaire ne détient jamais
que les objets que vous y avez délibérément déposés.

Le flag gouverne la promotion à la lecture et rien d'autre. Les
écritures, les suppressions, les métadonnées, les listages et les
destinations de `copy` et `rename` se comportent exactement comme avec
la promotion activée - un disque en `copy: false` fait donc quand même
atterrir sur le primaire un objet copié ou déplacé. Comme rien n'est
réécrit, une lecture avec `copy: false` ne récupère que la plage
demandée plutôt que l'objet entier.

### Copier et déplacer à travers le repli

`copy` et `rename` résolvent la source contre le primaire d'abord.
Quand seul le repli la détient, l'objet est transféré en flux par blocs
de 64 Kio et la destination atterrit sur le primaire :

```rust,ignore
let assets = Storage::disk("assets")?;

// `logo.png` ne vit que sur `legacy-store`. La copie le transfère en
// flux et écrit `branding/logo.png` sur `new-store` ; l'objet d'origine
// reste en place.
assets.copy("logo.png", "branding/logo.png").await?;

// Un déplacement fait la même chose puis supprime la source d'origine.
assets.rename("logo.png", "branding/logo.png").await?;
```

Un déplacement supprime la source du repli sur les deux chemins -
que le primaire ait détenu la source ou non. Sans cela, la lecture
suivante repromouvrait la copie du repli et annulerait le déplacement.

Les deux chemins diffèrent par le moment où ils la suppriment, et cette
différence est ce qu'un déplacement échoué laisse derrière lui :

- Le primaire détenait la source. La copie du repli part en premier,
  avant le `rename`. Tant que le primaire détient le chemin, la copie
  du repli est inatteignable à travers ce disque : la retirer d'abord
  ne change donc rien d'observable - et si la suppression échoue, rien
  n'a encore bougé. Refaites le déplacement. Si au contraire la
  suppression a réussi et que le `rename` a ensuite échoué, le repli ne
  détient plus rien pour ce chemin, la destination n'est pas écrite et
  le primaire détient toujours la source : une nouvelle tentative
  reprend donc ce même chemin et renomme à nouveau. L'échec coûte la
  copie froide et rien de plus.
- Seul le repli la détenait. La suppression ne peut venir qu'après la
  mise en place de la destination : un déplacement qui échoue sur la
  suppression laisse donc la destination écrite et la source toujours
  sur le repli. Refaites le déplacement ; la source est désormais sur
  le primaire, si bien que la nouvelle tentative prend le premier
  chemin.

Dans les deux cas, un déplacement échoué peut être refait sans danger,
et la destination que vous obtenez au bout est l'objet dont le
déplacement est parti.

Les conditions voyagent aussi avec l'opération sur le chemin en flux.
`if_not_exists` devient une écriture conditionnelle : une copie ou un
déplacement gardé refuse donc toujours une destination existante au
lieu de l'écraser, et une copie qui nomme une version de la source
obtient cette version depuis le repli. Le `if_match` d'une copie est la
seule exception : c'est une condition que le backend applique à
l'intérieur de sa propre copie, c'est-à-dire l'appel que ce chemin ne
peut justement pas faire ; elle est donc refusée avec une erreur
`Unsupported` nommant la condition, plutôt qu'ignorée en silence.

Cela fait des conditions le seul endroit où le disque qui détient la
source transparaît. Un répertoire local annonce `copy` et `rename` mais
aucune de leurs formes conditionnelles :
`copy_with(a, b).if_not_exists(true)` réussit donc quand seul le repli
détient `a` (cela devient une écriture conditionnelle) et est refusé
avec `Unsupported` quand le primaire le détient. Vérifiez la condition
dont vous avez besoin contre le driver primaire plutôt que de supposer
qu'elle vaut pour tous les objets du disque.

Un déplacement que le primaire refuserait est refusé avant que quoi que
ce soit ne soit supprimé. Un primaire sans `rename` du tout, un
déplacement gardé vers un primaire sans `rename` conditionnel, et un
déplacement gardé vers une destination qui existe déjà échouent tous
avec la source du repli toujours en place - un déplacement qui n'a
jamais lieu ne doit pas vous coûter la copie froide.

Si le flux échoue en cours de route, le writer est avorté et une
destination créée par le transfert est supprimée avant que l'erreur ne
vous parvienne : un transfert échoué n'est donc pas observable sous
forme d'objet tronqué. Une destination qui était déjà là est laissée
intacte - une copie échouée ne doit pas être ce qui détruit un objet
qu'elle n'a jamais écrit. Un primaire de type système de fichiers
local respecte cela lui aussi, parce qu'il prépare le transfert sous
`.suprnova-atomic/` et ne renomme qu'en cas de succès ; avorter le
writer supprime le fichier de préparation, si bien qu'un transfert
échoué ne laisse ni destination partielle ni fichier temporaire
résiduel.

### Lectures versionnées et conditionnelles

Une lecture qui porte une version ou une condition `If-Match`,
`If-None-Match`, `If-Modified-Since` ou `If-Unmodified-Since` est
transmise avec cette condition intacte : la réponse veut donc dire ce
que vous lui avez demandé de vouloir dire. Une telle lecture est servie
mais jamais promue : écrire sur le primaire une ancienne version, ou un
corps retenu par un validateur, la publierait comme l'objet courant, et
toute lecture simple ultérieure l'obtiendrait.

Le disque qui répond à une telle lecture est décidé de la façon
habituelle. La première sonde est une vérification d'existence
ordinaire : un disque read-through délègue donc une lecture versionnée
ou conditionnelle au primaire dès lors que le primaire détient le
chemin ; il n'atteint le repli que quand le primaire ne le détient pas.

Le primaire décide aussi lesquelles de ces lectures un disque
read-through accepte tout court, parce que le lecteur du primaire est
ouvert en premier. Une lecture versionnée contre un disque read-through
dont le primaire est un répertoire local est rejetée avant d'atteindre
le repli, puisqu'un répertoire local n'a pas de versions.

### Pourquoi Suprnova diverge

Laravel construit un disque read-through à partir d'une entrée de
`config/filesystems.php` dont les clés `primary` et `fallback`
acceptent soit un nom de disque, soit une config de driver en ligne.
Suprnova ne prend que des noms de disques, parce qu'ici les disques
sont enregistrés par des constructeurs typés plutôt que décrits par des
tableaux - enregistrez d'abord le disque interne, puis nommez-le.

La promotion de Laravel revérifie le primaire après avoir lu le repli,
ce qui fait gagner un writer concurrent. Suprnova garde cette
vérification et publie la promotion de façon atomique, ce que Laravel
ne fait pas. Sur un primaire de type système de fichiers local, les
octets sont déposés dans un voisin temporaire puis renommés en place ;
les écrire droit dans la cible laisserait un fichier grandissant, à
moitié écrit, visible pendant toute la durée de l'écriture - et un
disque read-through route les lecteurs précisément par cette
vérification d'existence. Sur un primaire sans `rename` - en mémoire,
S3, Azure Blob, GCS - une écriture est déjà une publication unique et
indivisible : la promotion écrit donc directement la cible, à condition
que l'objet n'existe pas déjà, pour que deux lecteurs concurrents ne
promeuvent pas tous les deux.

Cette condition est justement ce qu'une promotion par dépôt temporaire
ne peut pas avoir : le chemin de dépôt est unique, une condition de
non-écrasement sur lui serait donc vide de sens, et la cible est
publiée par un `rename` qui écrase. Un disque read-through sur un
primaire de type système de fichiers local y renonce donc - une
écriture qui atterrit sur le primaire dans l'instant qui sépare la
dernière vérification d'existence de la promotion de son `rename` est
écrasée par la copie promue. Sur un primaire sans `rename`, la
condition tient et une telle fenêtre n'existe pas.

L'objet de dépôt est une véritable entrée du primaire tant qu'il dure :
un listage pris en pleine promotion peut donc montrer un voisin
`.suprnova-promote-<id>.tmp`. Une lecture qui se termine, échoue ou
abandonne essaie de retirer son propre voisin, et journalise un
avertissement si cette suppression échoue plutôt que de faire échouer
la lecture. Rien ne balaie un voisin laissé par une suppression
échouée, par un processus qui a crashé ou par une future de lecture
annulée en pleine promotion : ceux-là doivent être retirés à la main.

Une lecture qui se résout depuis le repli garde l'objet en mémoire
jusqu'à ce que l'écriture de promotion se termine, parce que la
promotion a besoin de l'objet entier. Cela convient au cas du stockage
hiérarchisé auquel un disque read-through est destiné. Pour des objets
froids très volumineux, lisez directement le disque de repli ou
utilisez plutôt
[`copy_between_disks`](#copie-en-streaming-entre-disques).

Laravel rend le flux du repli lui-même quand `copy` vaut `false` et
tamponne via `php://temp` quand il vaut `true`. Suprnova, à la place,
restreint la récupération sur le repli à la plage demandée quand `copy`
vaut `false`, et ne tamponne que sur le chemin de promotion, où l'objet
entier est de toute façon nécessaire.

Les `copy` et `move` de Laravel à travers le repli tamponnent eux aussi
la source via `php://temp`. Suprnova la transfère en flux par blocs de
64 Kio à la place, parce que c'est sur le repli que vivent les objets
volumineux et rarement touchés, et supprime une destination à moitié
écrite avant de retourner l'erreur. Deux autres différences découlent
d'OpenDAL. Supprimer un chemin qui n'est pas là compte comme un succès :
un déplacement efface donc la source sur le repli sans vérifier d'abord
qu'elle existe. Et OpenDAL porte sur `copy` et `rename` des conditions
pour lesquelles Flysystem n'a pas d'équivalent : Suprnova doit donc
décider de ce que chacune signifie quand la source n'est que sur le
repli - `if_not_exists` et la version de source d'une copie sont
honorées, et le `if_match` d'une copie est refusé plutôt qu'abandonné.

Laravel supprime la source du repli après le déplacement sur les deux
chemins. Suprnova la supprime en premier quand le primaire détient la
source, parce que les deux ordres diffèrent lors d'une nouvelle
tentative : la source est inatteignable à travers le disque dans les
deux cas, mais supprimer en dernier veut dire qu'un déplacement qui a
perdu sa suppression sur une panne transitoire revient comme un
déplacement dont la source n'est plus que sur le repli, et déverse la
copie périmée du repli sur la destination que la première tentative
avait déjà écrite correctement.

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
