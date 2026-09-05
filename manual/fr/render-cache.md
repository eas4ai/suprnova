# RenderCache

RenderCache stocke une copie prouvée sûre de la réponse d'une route GET ou
HEAD et sert la requête correspondante suivante à partir de cette copie sans
jamais exécuter votre handler. Vous activez explicitement des routes et des
groupes ; tout le reste continue de fonctionner exactement comme
aujourd'hui. Une route que vous n'activez jamais reste intacte. Une route
que vous activez continue de se rendre et de servir correctement même quand
rien, pour cette requête en particulier, ne s'avère sûr à mettre en cache -
elle n'est alors simplement jamais stockée, et vous pouvez savoir pourquoi.

Ce chapitre couvre l'activation du cache, l'activation des routes et des
groupes, la déclaration de la variance, la lecture des en-têtes de réponse
qu'il ajoute, les raisons pour lesquelles un rendu est refusé, le contrôle
opérationnel, et en quoi il diffère de `suprnova::Cache`.

## Activer le cache

Deux variables d'environnement comptent pour démarrer :

- `RENDER_CACHE_ENABLED` - `true` sauf si réglée sur `false` ou `0`. Une
  fois désactivée, chaque requête contourne entièrement RenderCache ; rien
  n'est recherché et rien n'est stocké.
- `RENDER_CACHE_L1_DIR` - non définie par défaut, ce qui signifie aucun
  palier sur disque. Réglez-la sur un répertoire que le processus peut
  créer et dans lequel il peut écrire, et les représentations stockées
  survivent à un redémarrage du processus dans un second palier adossé à
  des fichiers.

Une poignée d'autres variables ajustent les valeurs par défaut :
`RENDER_CACHE_L0_ENTRIES` (4 096) et `RENDER_CACHE_L0_BYTES` (128 Mio)
bornent le palier intra-processus ; `RENDER_CACHE_L1_BYTES` (1 Gio) borne le
palier fichier ; `RENDER_CACHE_FAILURE` (`open` par défaut, ou `closed`)
décide si un problème de magasin ou de base de données sert la route sans
cache ou refuse la requête ; `APP_BUILD_ID` (la propre version de votre
crate par défaut) cantonne chaque entrée mise en cache au build qui l'a
produite, si bien qu'un déploiement ne sert jamais les octets d'un ancien
build.

## Activer une route ou un groupe

Rien n'est mis en cache tant que vous ne l'avez pas décidé.
`Router::try_render_cache` active un pattern de route déjà enregistré ;
`Router::try_render_cache_group` active chaque route sous un préfixe de
chemin. Les deux prennent une politique construite avec
`RenderCachePolicy::builder` :

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)`
définit combien de temps une représentation est fraîche, combien de temps
supplémentaire elle peut encore être servie pendant qu'une reconstruction
en arrière-plan s'exécute, et combien de temps de plus encore elle peut
être servie si cette reconstruction échoue purement et simplement.
`RepresentationClass` va du partage le plus large au plus étroit :
`PublicShared` (une représentation pour tous ceux qui correspondent à la
variance déclarée), `PublicShellStitched` (réservé à une future
représentation à coque composée, pas encore utilisable), `PrivateCached`
(une représentation par visiteur connecté ou par tenant), et `Uncacheable`.

Un pattern de route doit déjà être enregistré avant que vous ne l'activiez,
et vous devez terminer d'activer les routes et les groupes **avant**
d'appeler `RenderCache::install` (ci-dessous) - l'étape d'installation lit
tout ce qui a été enregistré jusque-là.

Une politique au niveau d'une route peut aussi être un correctif qui la
restreint par rapport à son groupe englobant, en utilisant `PolicyPatch` au
lieu d'une `RenderCachePolicy` complète : elle hérite de tout ce que le
groupe a déclaré et ne peut que la restreindre (une fenêtre de fraîcheur
plus courte, une classe plus stricte), jamais l'élargir. Retirer
entièrement une route d'un groupe mis en cache se fait avec un
`PolicyPatch` qui règle la classe sur `Uncacheable`.

Terminez de câbler RenderCache en une ligne, après chaque enregistrement de
middleware qui établit la locale, la session ou l'identité à la portée de
la requête (RenderCache les lit pour construire sa clé de recherche, donc
il doit s'exécuter après ce qui les met en place) :

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## Déclarer la variance

Par défaut, une représentation mise en cache ne varie que par pattern de
route, par paramètres de chemin et par build de l'application. Tout le
reste dont dépend réellement la sortie de votre handler doit être déclaré,
avec deux mécanismes :

- **Paramètres de requête.** `.query(QueryPolicy::declared(["page", "sort"]))`
  nomme les paramètres de requête qui distinguent les représentations ;
  tout autre paramètre de requête présent sur une requête contourne le
  cache pour cette requête au lieu d'être silencieusement ignoré.
- **Dimensions de variance**, ajoutées une par une avec `.vary(dimension)` :
  - `VarianceDimension::Locale` partitionne par la locale négociée.
  - `VarianceDimension::Media` partitionne par le type de média négocié.
  - `VarianceDimension::Host` partitionne par l'hôte de la requête, là où
    votre déploiement rend plus d'un hôte significatif.
  - `VarianceDimension::Tenant` partitionne par le tenant courant utilisé
    comme matériau de clé opaque ; une route dont le handler lit le
    tenant, à quelque moment que ce soit, doit déclarer cette dimension.
  - `VarianceDimension::Principal` partitionne par le visiteur connecté
    utilisé comme matériau de clé opaque, lié à une version de permission
    (voir « Epoch, permissions et inspection » ci-dessous) ; une route
    `PrivateCached` doit déclarer `Principal` ou `Tenant` (ou les deux),
    sinon elle échoue purement et simplement à la construction.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion`, et
un `VarianceDimension::Application(name)` personnalisé existent sur le
type mais n'ont pas de résolveur dans cette version : une route qui en
déclare un contourne le cache à chaque requête, silencieusement, au lieu
d'échouer à la construction. Ne les déclarez pas encore.

## Lire les en-têtes de réponse

Une réponse servie depuis le cache porte `ETag` (un validateur fort que
votre client peut renvoyer sous forme de `If-None-Match` pour obtenir un
`304`), `Cache-Control` (`private` sauf si la classe est `PublicShared` et
que vous avez réglé un `SharedCachePolicy::SMaxAge`, auquel cas elle porte
aussi `public` et `s-maxage`), `Vary` (à partir de toute dimension déclarée
qui en implique un - `Locale` implique `Accept-Language`, `Media` implique
`Accept`), et `Age` (secondes entières depuis la publication de la
représentation). Une réponse périmée mais encore servable porte en plus
`Warning: 110 - "Response is Stale"`.

## Pourquoi un rendu n'est jamais stocké

Être activé n'est pas une garantie. Deux vérifications indépendantes
s'exécutent après chaque rendu, et chacune peut refuser le stockage sans
faire échouer la requête - la réponse que vous récupérez est identique
dans les deux cas, elle ne devient simplement jamais une entrée de cache :

**L'éligibilité** refuse d'emblée une réponse qui n'est pas un `200` nu
pour un `GET` ou `HEAD`, qui diffuse son corps en streaming, qui pose un
cookie, ou qui porte un en-tête de saut à saut ou de traçage. Ce sont
presque toujours des accidents (une redirection, une page d'erreur, une
réponse qui touche par hasard à `Set-Cookie`) plutôt que quelque chose que
vous devez anticiper dans votre conception.

**La classification** refuse selon ce que votre handler a réellement fait
pendant son exécution, en des termes que vous reconnaîtrez :

- **Vous avez lu une valeur de session.** Toute lecture de la session
  courante (via `session()`, `session_mut`, ou un cookie de session) force
  le rendu à `Uncacheable`, de façon permanente, quelle que soit la
  variance déclarée par la route. Cela se déclenche aussi quand l'identité
  d'un visiteur anonyme se résout via le repli sur la session - une
  surprise fréquente, puisque le visiteur est réellement anonyme et que la
  clé résultante est correctement `Anonymous`, mais la lecture elle-même
  reste une lecture de session.
- **Vous avez lu une identité, sur une route qui ne déclare pas
  `Principal`.** Lire l'utilisateur connecté restreint la classe à
  `PrivateCached` ; si la variance déclarée par la route n'inclut pas
  `Principal`, il n'y a aucun moyen d'indexer l'entrée par visiteur, donc
  elle est refusée plutôt que partagée.
- **Vous avez traduit (ou votre moteur de vues l'a fait) sans déclarer
  `Locale`.** Toute lecture de la locale négociée exige une dimension
  `Locale` déclarée, sinon le rendu est refusé. La coque de document de
  chaque page Inertia lit la locale pour définir `<html lang>`, que les
  données propres de la page aient ou non un rapport avec la langue - donc
  une route Inertia a besoin que `Locale` soit déclarée pour pouvoir un
  jour mettre en cache, même une route sans aucun contenu traduit qui lui
  soit propre.
- **Vous avez vérifié une autorisation.** `Gate` traite toujours une
  décision comme propre à chaque visiteur, donc il a besoin que `Principal`
  soit déclarée même sur une route dont la clé ne repose que sur `Tenant`,
  tant que la vérification du gate elle-même n'est pas démontrablement
  propre à chaque tenant. RenderCache ne peut pas faire la différence de
  lui-même.
- **Un modèle derrière la page porte une portée globale bornée par
  tenant.** Une portée globale qui lit le tenant courant depuis son propre
  état local à la requête pour filtrer une requête - le pattern que montre
  la documentation de `GlobalScope` de Suprnova lui-même - change ce que
  retourne la requête sans que RenderCache ne voie jamais cette lecture.
  Déclarez la variance `Tenant` sur toute route adossée à un tel modèle ;
  rien ici ne peut rattraper cet oubli à votre place.
- **Vous avez lu une valeur de configuration secrète, ou un contexte de
  requête non déclaré.** Les deux forcent `Uncacheable`. La dépendance
  d'une réponse à un en-tête de requête ordinaire, ou à `Config::get`, est
  totalement invisible pour RenderCache - il ne peut pas refuser ce qu'il
  ne peut pas voir, donc déclarer la variance correspondante vous revient.

Rien de tout cela n'a besoin d'un outillage particulier pour être observé
en pratique : la commande masquée `render-cache:inspect` (ci-dessous)
montre si l'entrée d'une route existe ou non, ou vous pouvez simplement
essayer deux requêtes de suite et vérifier si la seconde porte un en-tête
`Age`.

## Une route qui met en cache

Une page de listing publique sans contenu propre à chaque visiteur :

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

enregistrée et activée :

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` ne touche jamais à la session, au visiteur connecté ni à la
locale, donc la première requête se rend et publie ; chaque requête
pendant les cinq minutes suivantes est servie depuis cette copie stockée
avec un en-tête `Age`, un `304` pour un client qui l'a déjà, et
`Cache-Control: public, max-age=300, s-maxage=300` pour tout CDN placé
devant.

## Une route qui est refusée

La même forme de page, mais le handler lit la session pour afficher un
message flash :

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

activée exactement de la même façon que ci-dessus. Chaque requête continue
de se rendre et de servir la bonne page - message flash compris - mais
rien n'est jamais stocké : la lecture de session restreint la classe à
`Uncacheable` avant même que RenderCache n'atteigne la vérification
d'éligibilité, si bien qu'une seconde requête pour la même URL se rend à
nouveau depuis zéro au lieu de revenir avec un en-tête `Age`. Le correctif,
si cette page est censée être mise en cache, consiste à arrêter de lire la
session dans le chemin mis en cache (rendez plutôt le flash à partir d'un
paramètre de requête ou d'une petite réponse séparée) - aucune déclaration
de variance ne rend une lecture de session compatible avec la mise en
cache, car une lecture de session signifie que la réponse dépend de
quelque chose qu'aucune clé ne pourrait partitionner en toute sécurité.

## Epoch, permissions et inspection

- **`RenderCache::bump_permission_version()`** - appelez ceci chaque fois
  qu'une action applicative change ce qu'un utilisateur connecté est
  autorisé à faire (un changement de rôle, l'octroi ou la révocation d'une
  permission). Sans cela, un utilisateur dont les permissions viennent de
  changer continue de correspondre à ce qui était mis en cache sous son
  précédent jeu de permissions.
- **`RenderCache::advance_epoch()`**, ou la commande masquée
  `render-cache:epoch-advance` - une invalidation d'urgence. Chaque entrée
  actuellement stockée devient inatteignable par une recherche ordinaire
  dès sa toute prochaine requête, immédiatement, parce que l'epoch est
  intégré directement à la clé de recherche elle-même. Le palier
  intra-processus est aussi entièrement vidé au même instant ; un palier
  adossé à des fichiers conserve ses anciens fichiers sur le disque
  jusqu'à ce que le balayage périodique ou manuel les récupère, ce qui
  relève de l'hygiène disque plutôt que d'un problème de correction.
  Recourez à ceci quand quelque chose ne va pas avec le contenu mis en
  cache et que vous ne pouvez pas attendre l'expiration individuelle des
  entrées.
- **La commande masquée `render-cache:inspect <key>`** rapporte les
  métadonnées d'une entrée stockée (jamais son corps) via le texte de clé
  que les logs ou la télémétrie de votre application peuvent faire
  apparaître, ainsi que l'epoch courant, afin que vous puissiez déterminer
  si ce que vous regardez fait encore autorité en direct ou a déjà expiré
  entre-temps.

## RenderCache face à `suprnova::Cache`

`suprnova::Cache` est un magasin clé-valeur que vous appelez explicitement :
vous choisissez la clé, vous choisissez ce qu'il faut stocker, vous
choisissez quand l'invalider (`Cache::put`, `Cache::get`,
`Cache::remember`, `Cache::forget`). Il fonctionne pour toute donnée que
votre code juge digne d'être mise en cache, sur tout backend que vous
configurez (mémoire ou Redis).

RenderCache n'est pas un magasin à usage général, et vous ne l'appelez
jamais depuis votre handler. Il met en cache des réponses HTTP entières,
la clé est dérivée automatiquement de la route et de sa variance déclarée,
et l'invalidation est fondée sur des générations : une écriture ordinaire
en base de données via l'ORM ou le générateur de requêtes avance les
générations dont dépendait le rendu, et l'entrée est recalculée la
prochaine fois qu'elle est demandée plutôt que supprimée à la main.
Tournez-vous vers `suprnova::Cache` quand vous avez une valeur précise que
vous voulez calculer une fois et réutiliser ; tournez-vous vers RenderCache
quand vous avez une route entière dont la réponse est coûteuse à rendre et
sûre à partager.
