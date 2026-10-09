# Images

Suprnova livre un pipeline d'images façon Laravel : construisez-le dans un
handler, enchaînez les opérations voulues et terminez par un terminal qui
vous rend des octets, une réponse ou un fichier stocké.

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

Ce handler décode le JPEG, remplit une boîte de 320x320, rogne le
débordement depuis le centre, encode en WebP et renvoie un `200` avec
`Content-Type: image/webp`.

Le sous-système vit dans `suprnova::media`, derrière la feature `media`
activée par défaut. Tout ce que vous manipulez d'ordinaire - `Image`,
`OutputFormat`, `ImageDriver`, `ImageConfig` - est réexporté à plat à la
racine du crate, si bien que `use suprnova::Image;` est l'import qu'il
vous faut. Le nom du module est pluriel dans l'esprit, et c'est
volontaire : c'est là que vivront aussi les surfaces audio et vidéo
adossées à OxideAV.

Si vous faites une montée de version, notez que le validateur de
téléversement qui s'appelait `Image` s'appelle désormais `ImageFile`, ce
qui libère le nom simple pour ce type de pipeline. Cela reflète Laravel, où
la règle de validation est `ImageFile` et le type de manipulation est
`Image`. Voir [Requêtes](requests.md) pour le validateur.

## Le pipeline est paresseux

Construire une `Image` ne lit rien et ne décode rien. Les opérations
s'enregistrent elles-mêmes ; la source n'est ouverte que lorsqu'un
terminal s'exécute. Ceci ne coûte donc rien :

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

Rien n'a encore touché le disque. `Image` est `Clone`, et un clone rejoue
le pipeline depuis sa source plutôt que de partager un résultat.

Deux constructeurs sont forcément immédiats, et leur documentation le dit :
`from_upload` (le fichier temporaire d'un téléversement ne survit pas à la
requête) et `from_stream` (un flux ne peut être consommé qu'une fois).

## Construction

| Constructeur | Source | Immédiat ? |
|---|---|---|
| `Image::from_bytes(bytes)` | tout ce qui est `Into<Bytes>` | non |
| `Image::from_path(path)` | le système de fichiers | non |
| `Image::from_disk(disk, path)` | un disque `Storage` | non |
| `Image::from_upload(&file).await?` | un `UploadedFile` | oui |
| `Image::from_stream(stream).await?` | un `Stream<Item = io::Result<Bytes>>` | oui |

`from_stream` applique `IMAGE_MAX_ALLOC_BYTES` *pendant* la collecte, si
bien qu'un flux sans fin est coupé au lieu d'être découvert une fois la
mémoire déjà remplie.

## Opérations

| Méthode | Effet |
|---|---|
| `resize(w, h)` | Dimensions exactes, rapport d'aspect ignoré |
| `resize_width(w)` / `resize_height(h)` | Une dimension, l'autre déduite du rapport d'aspect |
| `scale(w, h)` | Tient dans la boîte en préservant le rapport d'aspect. **N'agrandit jamais** |
| `scale_width(w)` / `scale_height(h)` | Réduit à au plus une dimension. N'agrandit jamais |
| `crop(w, h, x, y)` | Découpe un rectangle. Erreur s'il tombe hors de l'image |
| `cover(w, h)` | Remplit exactement la boîte en rognant le débordement depuis le centre |
| `contain(w, h)` | Tient dans la boîte en préservant le rapport d'aspect. Sans remplissage |
| `rotate(degrees)` | Rotation horaire d'un angle quelconque, la toile s'agrandissant pour tout contenir |
| `flip_vertically()` / `flip_horizontally()` | Les `flip` et `flop` de Laravel |
| `blur(amount)` | Flou gaussien, `0..=100`. `0` est sans effet |
| `sharpen(amount)` | Masque flou, `0..=100`. `0` est sans effet. `50` est l'intensité classique |
| `grayscale()` | Désature. Orthographié à la manière de Laravel |
| `to_format(format)` | Choisit le conteneur de sortie |
| `quality(q)` | Qualité d'encodage, bornée à `1..=100`, `70` par défaut |

Les valeurs qui n'auraient aucun sens sont bornées plutôt que rejetées :
`blur(500)` enregistre `100`, `quality(0)` enregistre `1`. Un rognage qui
tombe hors de l'image est une vraie erreur, pas un bornage, car déplacer
silencieusement la zone de rognage de quelqu'un est pire que de le lui
dire.

`rotate` accepte des angles quelconques. Un multiple de 90 degrés emprunte
un chemin exact aligné sur les axes, sans rééchantillonnage ; tout le
reste est bilinéaire, et la toile s'agrandit pour qu'aucun pixel ne soit
coupé. Les coins ainsi dégagés sont transparents lorsque le format de
sortie possède un canal alpha.

## Terminaux

Chaque terminal est `async`, consomme l'`Image` et exécute le décodage, la
transformation et l'encodage sur un thread bloquant, de sorte qu'il ne
bloque jamais le runtime. Les E/S sur la source ont lieu avant ce saut,
si bien qu'un disque lent n'occupe jamais un worker bloquant.

| Terminal | Renvoie |
|---|---|
| `to_bytes()` | Le `Vec<u8>` du fichier encodé |
| `to_response()` | Une `HttpResponse` avec le bon `Content-Type` |
| `save(path)` | Écrit sur le système de fichiers |
| `store(disk, path)` | Écrit sur un disque `Storage` |
| `dimensions()` | Le `(width, height)` de l'image **traitée** |
| `mime_type()` | Le type de média de l'image **traitée** |
| `dominant_color()` | La couleur moyenne, sous la forme `#rrggbb` |

`dimensions()`, `mime_type()` et `dominant_color()` décrivent tous les
trois l'image finie, pas la source - le même contrat que celui de
Laravel. Demander le type MIME exécute quand même le pipeline, car
annoncer un type pour une image qui ne peut pas réellement être produite
est un mensonge que l'appelant ne découvrirait que plus tard.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // Lit (1200, 400), pas les dimensions de la source.
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## Formats

Cinq formats sont lus et écrits aujourd'hui : **PNG, JPEG, WebP, GIF et
BMP**.

| Format | Lecture | Écriture | Réglage de qualité |
|---|---|---|---|
| PNG | oui | oui | ignoré (sans perte) |
| JPEG | oui | oui | respecté |
| WebP | oui | oui (sans perte) | sans effet aujourd'hui |
| GIF | oui | oui | ignoré (palette) |
| BMP | oui | oui | ignoré (sans perte) |

AVIF n'est encore ni lu ni écrit. L'encodeur AV1 maison dont il dépend
n'a pas été publié, et livrer un `OutputFormat::Avif` qui échouerait
toujours serait une promesse que le framework ne pourrait pas tenir. Il
arrive avec cette publication, sous la forme d'une nouvelle variante
d'enum et de rien d'autre.

La sortie GIF est quantifiée sur une palette d'au plus 256 couleurs avec
un tramage Floyd-Steinberg avant l'encodage, si bien qu'une source
photographique se convertit proprement au lieu d'échouer.

WebP est écrit sans perte, donc `quality()` n'a actuellement aucun effet
sur la sortie WebP. Utilisez JPEG lorsqu'il vous faut un curseur
taille/qualité.

## Stockage

`from_disk` et `store` fonctionnent avec n'importe quel disque `Storage`
enregistré, si bien qu'un aller-retour redimensionner-puis-restocker ne
touche jamais de chemin local :

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

Voir [Système de fichiers et stockage](filesystem.md) pour enregistrer des
disques.

## Limites de décodage

Le décodage est l'endroit où une entrée hostile fait des dégâts :
quelques kilo-octets peuvent déclarer une toile de 40000x40000 et
demander à un serveur d'allouer six gigaoctets pour elle. Suprnova refuse
cela avant toute allocation.

| Variable | Défaut | Rôle |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Plafond de la largeur et de la hauteur en pixels |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 Mio) | Plafond de l'empreinte RGBA décodée, et de la taille du fichier source lui-même |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Plafond en temps réel d'une invocation d'ImageMagick (driver `magick` uniquement) |

Le framework analyse l'en-tête de l'entrée elle-même - quelques dizaines
d'octets, aucune allocation -, lit les dimensions déclarées et rejette
une entrée surdimensionnée avant qu'un décodeur ne soit construit. Les
mêmes plafonds s'appliquent aux cibles de redimensionnement, car
`resize(50_000, 50_000)` alloue tout autant, que les nombres viennent d'un
attaquant ou d'une faute de frappe.

Un plafond atteint donne un `FrameworkError::param` de forme 4xx, car une
entrée surdimensionnée est un problème du client, pas une faute du
serveur.

Une configuration hors plage est bornée avec un avertissement plutôt que
de faire échouer l'amorçage : `IMAGE_MAX_DIMENSION=0` rejetterait toutes
les images de l'application, ce que personne n'a voulu configurer.

### Une borne n'est pas configurable

Un WebP déclare sa vraie taille décodée dans son chunk de flux binaire le
plus interne, pas dans l'en-tête de la toile ; le framework parcourt donc
le conteneur pour la trouver. Ce parcours s'arrête après **4096 chunks par
niveau** et suit l'imbrication sur **deux niveaux**, et un fichier qui
dépasse l'un ou l'autre est refusé d'emblée plutôt que mesuré.

Il est refusé plutôt que mesuré, et c'est volontaire. Annoncer un nombre
issu d'un parcours qui n'a pas atteint la fin du fichier donnerait une
gate qu'un tas suffisamment gros de chunks de remplissage pourrait
contourner ; un parcours qu'on ne peut pas terminer n'a donc aucune
réponse à donner.

Aucun de ces deux nombres n'est réglable, et aucune variable
`IMAGE_MAX_*` ne les affecte - l'erreur le dit, au lieu de dire
« configuré », précisément pour que personne ne passe un après-midi à
augmenter `IMAGE_MAX_ALLOC_BYTES` en constatant que rien ne change. En
pratique, seul un fichier délibérément hostile s'en approche : une
animation de 300 images passe confortablement, une de 4100 non.

## Backends

Comme chez Laravel, la surface image se résume à deux drivers, choisis
avec `IMAGE_DRIVER`.

| Driver | Valeur | Requiert | Lit |
|---|---|---|---|
| OxideAV | `oxideav` (par défaut) | rien | PNG, JPEG, WebP, GIF, BMP |
| ImageMagick | `magick` | ImageMagick 7 sur l'hôte | ce que fournissent les délégués de l'hôte |

### `IMAGE_DRIVER=oxideav`

Le choix par défaut. Rust pur, bâti sur la famille de codecs
[OxideAV](https://github.com/OxideAV) : aucune bibliothèque native, rien à
installer, rien à configurer. C'est le bon choix pour presque toutes les
applications, et c'est ce que reçoit une application scaffoldée.

### `IMAGE_DRIVER=magick`

À activer explicitement. Exécute un binaire ImageMagick 7 installé sur
l'hôte, en lui envoyant l'image par stdin et en relisant le résultat par
stdout - sans fichiers temporaires. Le nom du binaire vient de
`IMAGE_MAGICK_BINARY` et vaut `magick` par défaut ; un binaire manquant
est une erreur claire dès la première utilisation, pas un repli
silencieux.

Choisissez-le lorsque vous avez besoin de formats d'entrée que le driver
en Rust pur ne porte pas - HEIC étant le cas courant. Le prix est une
dépendance à l'hôte : l'opérateur installe ImageMagick et ses délégués,
et assume leur licence. Le framework ne lie rien et ne compile rien de
natif dans un cas comme dans l'autre.

Les arguments forment toujours un tableau fixe passé directement au
processus, jamais une chaîne de shell, et chaque argument numérique est
formaté à partir d'un champ déjà validé. Aucune position d'argument n'est
accessible à une entrée utilisateur.

Lorsque le framework reconnaît l'entrée, le décodeur est nommé sur la
ligne de commande - `png:-` plutôt qu'un simple `-`. Cela compte : face à
un simple `-`, ImageMagick choisit un coder d'après les octets qu'on lui
tend, si bien qu'un fichier dont les octets magiques disent MVG ou MSL
est lu comme un *script*, quoi que votre application ait cru accepter.
Épingler le coder fait échouer un fichier mal étiqueté au lieu de le
laisser devenir autre chose.

**Une entrée que le framework ne sait pas nommer dépend encore de votre
`policy.xml`.** Lire ces formats est toute la raison d'être de ce driver,
donc ce chemin ne peut pas épingler de coder. Durcissez la politique
ImageMagick de l'hôte - au minimum en désactivant les coders `MVG`,
`MSL`, `URL`, `HTTPS`, `EPHEMERAL` et `TEXT` - si vous acceptez des
téléversements arbitraires sous `IMAGE_DRIVER=magick`.

Les limites de décodage sont appliquées deux fois sous ce driver. Pour les
cinq formats que le framework sait analyser, la vérification d'en-tête
ci-dessus s'exécute avant que le processus ne soit lancé. Pour tout le
reste, une pré-analyse est impossible : chaque invocation porte donc les
flags `-limit` propres à ImageMagick, dérivés de la même configuration,
dont un `-limit time` en temps réel.

Ce flag ne dit pas tout, car ImageMagick l'applique avec son propre
moniteur de ressources, et un processus coincé à l'intérieur d'un délégué
avant que ce moniteur n'entre en jeu ne le déclenche jamais. Suprnova
tient donc aussi sa propre échéance : passé `IMAGE_MAGICK_TIMEOUT_SECS`
(plus quelques secondes de grâce pour laisser la limite d'IM se déclencher
en premier), il tue le groupe de processus - délégués compris, pas
seulement le processus qu'il a lancé - et cesse d'attendre sur les tubes.
Un délégué bloqué ne peut donc pas immobiliser un thread de travail. Les
délégués qui restent dans le groupe de processus meurent avec lui ; un
délégué qui quitte le groupe, ou un hôte sans binaire `kill`, peut
survivre à la requête - ce résidu est précisément ce à quoi sert la
supervision de processus de l'hôte.

Une mise à mort remonte en `FrameworkError::internal` 5xx, pas en 4xx,
même si c'est une requête qui l'a déclenchée. Quelque chose a coincé le
chemin image au point qu'il fallait le tuer, ce qui relève de la
surveillance des erreurs serveur, là où un opérateur le verra - le classer
comme erreur client rangerait de côté la seule condition ici qui mérite de
déclencher une astreinte.

## Drivers personnalisés

`ImageDriver` est le point d'extension : `&[u8]` en entrée, `Vec<u8>` en
sortie, aucun type de codec ne franchit la frontière.

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // Décode `contents`, rejoue `pipeline.transformations`, puis encode
        // vers `pipeline.format` à `pipeline.quality`.
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

Installez-le pendant `bootstrap()`, avant que la première image ne soit
traitée :

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

Un driver conforme applique les limites `ImageConfig` configurées avant
d'allouer pour un décodage. Le framework ne peut pas le faire à la place
d'un driver, car il ne voit jamais le tampon décodé.

### Couvrir davantage de formats

Si les cinq formats intégrés ne suffisent pas, il y a trois voies, à peu
près classées par ce que vous prenez en charge :

1. **Le driver `magick` intégré.** Réglez `IMAGE_DRIVER=magick`.
   L'étendue des formats vient des délégués ImageMagick de l'hôte, et il
   n'y a aucune dépendance de build à gérer.
2. **Un driver personnalisé autour de libvips**, par exemple via le crate
   [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings)
   (MIT). libvips est le moteur derrière le `sharp` de Node, avec une
   gamme de formats très large - JPEG, JPEG XL, TIFF, PNG, WebP, HEIC,
   AVIF, PDF, SVG, GIF et d'autres, plus la délégation à ImageMagick - et
   de solides performances en streaming. Il lie la bibliothèque C libvips,
   donc votre application installe libvips au build et à l'exécution et
   assume cette dépendance, ce qui est exactement pourquoi elle a sa place
   derrière le trait plutôt que dans le framework. Une note pratique : le
   `VipsImage` de ce binding n'est pas thread-safe, ce que la forme du
   driver - une image par appel à `process()` - accommode déjà.
3. **N'importe quel outil CLI**, enveloppé comme l'est le driver
   `magick` : un tableau d'arguments fixe passé à `std::process::Command`,
   les octets de l'image par stdin et la sortie par stdout, jamais une
   chaîne de shell.

Suprnova cautionne la frontière du trait, pas une dépendance particulière
derrière elle. Ce qui se trouve là est votre affaire, et sa licence aussi.

## Tests

Le sous-système n'a besoin d'aucune fixture sur disque - il est sa propre
fabrique de fixtures dès lors que le décodage et l'encodage font
l'aller-retour :

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// Fait grandir une fixture littérale de 1x1 jusqu'à la taille qu'un test demande.
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

Les tests qui resserrent les limites de décodage doivent être sérialisés :
les limites sont globales au processus, donc un test voisin exécuté en
parallèle décoderait sous le plafond resserré.

### Pourquoi Suprnova diverge

**Pas de HEIC dans le driver par défaut, et la raison tient aux brevets.**
HEVC, le codec à l'intérieur de HEIC, est grevé de brevets - le pool
Access Advance entre autres. Suprnova n'installe aucune bibliothèque
native : un décodeur intégré devrait donc être en Rust pur et porterait
cette exposition directement, et le seul décodeur crédible en Rust pur est
sous double licence AGPL-3.0/commerciale, ce qui est une obligation
juridique par application plutôt que quelque chose dans quoi un framework
MIT peut embarquer qui que ce soit par défaut.

Les deux frameworks font de HEIC une affaire de provisionnement de
l'hôte ; la version de Suprnova a simplement une pièce mobile de moins. Le
driver par défaut de Laravel, GD, ne lit pas HEIC du tout, et son chemin
Imagick exige que le délégué libheif soit compilé **à la fois** dans le
binaire ImageMagick du système et dans l'extension PHP `imagick`. Dans
Suprnova, le driver par défaut ne lit pas HEIC, et `IMAGE_DRIVER=magick`
le lit dès que l'ImageMagick de l'hôte porte le délégué libheif - sans
couche d'extension entre les deux. L'ingestion de HEIC fonctionne donc
aujourd'hui : installez ImageMagick avec libheif via votre gestionnaire de
paquets et basculez la variable d'environnement. La licence est là où elle
doit être, chez l'hôte.

Lorsque le driver `oxideav` rencontre un fichier HEIC, il le dit par son
nom, renvoie vers ce chapitre et nomme les deux voies possibles, plutôt
que de retourner un « format non pris en charge » générique.

**AVIF est en attente, pas écarté.** Il est libre de redevances et c'est
la réponse « format moderne » que nous voulons ; l'encodeur AV1 maison n'a
simplement pas encore été publié. WebP est la voie du format moderne en
attendant.

**Pas de constructeurs base64 ni URL.** L'`ImageManager` de Laravel a
`->read($base64)` et `->read($url)`. `from_bytes` se compose avec ce qui a
produit les octets, y compris le [client HTTP](http-client.md), et garder
une récupération par URL hors du sous-système image maintient ses délais
d'attente, ses réessais et sa politique SSRF en un seul endroit plutôt que
deux.

**`from_stream` est immédiat, avec un plafond.** Le contenu chez Laravel
est une closure paresseuse. Un flux ne peut pas être rejoué : celui-ci est
donc vidé à la construction, en comptant les octets face à
`IMAGE_MAX_ALLOC_BYTES` au fur et à mesure.

**`contain` ne complète pas.** Il fait tenir l'image dans la boîte et
s'arrête là ; il n'ajoute pas de bandes sur un fond. Composez vous-même
avec un fond si vous en avez besoin.

**Le redimensionnement utilise un rééchantillonnage bilinéaire.** Le jeu
de filtres du backend fournit le plus proche voisin et le bilinéaire ; le
bilinéaire est son choix par défaut documenté pour les images naturelles.

**Les images ne sont jamais sérialisables.** Laravel lève une exception
sur `__serialize` et Suprnova ne l'implémente tout simplement pas. Stockez
le chemin ou la clé du disque et reconstruisez le pipeline.

## Suivant

- [Système de fichiers et stockage](filesystem.md) pour les disques que `from_disk` et `store` lisent et écrivent.
- [Réponses](responses.md) pour ce que `to_response()` rend.
- [Variables d'environnement](env-vars.md) pour la liste complète des réglages d'image.
