# Représentations de RenderCache

Une route mise en cache ne stocke pas « une page ». Elle stocke une
**représentation** : une réponse concrète, sous une clé de recherche, dans
un ou plusieurs paliers de stockage, avec assez de métadonnées à côté pour
répondre à une requête conditionnelle et pour prouver plus tard qu'elle est
toujours à jour. Deux visiteurs obtiennent les mêmes octets stockés
seulement quand la clé qu'ils dérivent est la même clé, et la clé est
dérivée de ce que la route a déclaré - jamais de ce que le handler a fait.

Ce chapitre porte sur cette chose stockée. Les formes qu'une représentation
peut prendre (`Complete` et `Composite`), ce qui entre dans sa clé, les
paliers dans lesquels elle est écrite, les `ETag`, `Cache-Control`, `Vary`,
`Age` et `Warning` que porte un hit servi, les quatre états de fraîcheur
dans lesquels elle peut être, la façon dont elle répond à `If-None-Match`
et à `HEAD`, et ce que `PrivateCached` et `PublicShellStitched` stockent
réellement. *Pourquoi* une représentation quitte la bande de fraîcheur - une
écriture, un avancement d'epoch - est le sujet du chapitre suivant ; ici, il
suffit que les bandes existent et qu'une représentation se trouve dans
l'une d'elles. Chaque exemple ci-dessous est une route de l'application
dogfood de ce dépôt
(`app/src/live/mod.rs`) et est prouvé par un test nommé dans
`app/tests/live_render_cache.rs`.

## Deux formes d'entrée

Une entrée stockée est de l'une des deux sortes suivantes.

- **`Complete`** est une réponse finie : un statut, un jeu d'en-têtes
  rejouables, et un tampon de corps. La servir ne copie rien et n'exécute
  rien. Chaque route `PublicShared` et `PrivateCached` stocke cette forme.
- **`Composite`** est une **coque** partagée dans laquelle sont découpés
  des trous typés, plus un graphe de segments qui dit ce qui revient dans
  chaque trou. Seul `RepresentationClass::PublicShellStitched` stocke cette
  forme, et seul un document Live en produit une.

La classe que vous déclarez dans la politique décide de la forme qui est
seulement atteignable. `/live/public` et `/live/todos` déclarent tous deux
`PublicShared` ; `the_database_profile_serves_a_hit_through_the_sql_stores`
relit l'entrée publiée de `/live/todos` depuis le magasin et asserte que
c'est une entrée `EntryKind::Complete`, et
`the_public_document_is_a_hit_whose_seed_still_promotes` relit celle de
`/live/public` via `RenderCache::inspect_route_for_test` et asserte la
classe sous laquelle elle a été stockée. Cela compte, parce que « elle a été
stockée » et « elle a été silencieusement refusée » produisent la même
réponse : l'affirmation doit porter sur l'entrée, pas sur ce que le visiteur
voit.

## La clé de recherche

La clé qu'une requête dérive est construite à partir du pattern de route, de
ses paramètres de chemin, des paramètres de requête que la politique a
déclarés, de la valeur résolue de chaque dimension de variance déclarée, de
l'identifiant de build de l'application (`APP_BUILD_ID`), et de l'epoch
d'autorité courant. Rien d'autre. Un paramètre de requête qui arrive sur la
requête mais que `QueryPolicy::declared` ne nomme pas contourne le cache
pour cette requête au lieu d'être discrètement retiré de la clé, car le
retirer servirait la mauvaise page à celui qui l'a envoyé.

La clé est un texte qu'un opérateur peut tenir en main :
`RenderCache::key_for_route_for_test` dans
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` asserte
qu'elle commence par `rk1.`, et `render-cache:inspect` prend exactement ce
texte.

Parce que l'epoch fait partie de la clé, un avancement d'epoch n'a rien à
trouver ni à supprimer. Chaque entrée précédemment stockée cesse simplement
d'être atteignable par une recherche ordinaire à la requête suivante. C'est
le mécanisme sur lequel repose l'invalidation d'urgence du chapitre
[Exploitation](render-cache-operations.md).

## Dans quels paliers une politique écrit

Il y a deux paliers de stockage. **L0** est la mémoire intra-processus,
bornée par `RENDER_CACHE_L0_ENTRIES` et `RENDER_CACHE_L0_BYTES`. **L1** est
ce que le profil de déploiement configure - un répertoire de fichiers, une
table de base de données, ou Redis - et il est partagé par chaque processus
qui pointe dessus.

Le constructeur de politique stocke dans **L0 seulement** sauf indication
contraire : `StorageLayers::l0_only()` est la valeur par défaut. Une route
qui vaut la peine d'être mise dans le palier partagé le déclare :

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

C'est la déclaration de `/live/todos` dans `app/src/live/mod.rs`. C'est le
seul document de cette application dont chaque nœud peut partager les
octets, donc c'est le seul qui déclare `l0_and_l1()`. Sous le profil
embarqué, où L1 est désactivé sauf si `RENDER_CACHE_L1_DIR` nomme un
répertoire, déclarer le palier ne change rien ; sous le profil Database
l'entrée atterrit dans `suprnova_render_entries` et un second processus l'y
trouve.

`the_database_profile_serves_a_hit_through_the_sql_stores` en est la preuve.
Il démarre l'application sur les fournisseurs du profil Database, lit
l'entrée publiée directement depuis L1 sous la clé même que le middleware a
dérivée, puis vide L0 et redemande - et la seconde requête reçoit encore une
réponse sans que le handler ne s'exécute. Un hit en mémoire aurait l'air
identique du côté du client, et c'est pourquoi le test va chercher dans le
magasin.

Choisissez les paliers route par route plutôt que globalement. L1 coûte un
aller-retour sur un miss que L0 seul ne coûte pas, et une entrée qu'un seul
nœud demandera jamais ne vaut pas la peine d'être mise là où chaque nœud
peut la voir.

## Les métadonnées que porte un hit servi

Cinq champs de réponse décrivent une représentation servie, et c'est ici
qu'ils sont définis ; les autres chapitres les utilisent sans les redéfinir.

| Champ | Ce qu'il dit |
|---|---|
| `ETag` | Un validateur fort sur exactement les octets envoyés. Un client peut le renvoyer sous forme de `If-None-Match`. |
| `Cache-Control` | `private` pour chaque classe par défaut. Une route `PublicShared` qui règle `SharedCachePolicy::SMaxAge` obtient aussi `public` et `s-maxage`, ce qui est la seule façon d'inviter un proxy partagé à garder les octets. Un document `Composite` comportant au moins un îlot reçoit `private, no-store`, qu'il ait été assemblé sur un hit ou produit par le rendu qui a publié la coque. |
| `Vary` | Dérivé des dimensions de variance déclarées qui impliquent un en-tête de requête : `Locale` implique `Accept-Language`, `Media` implique `Accept`, `Encoding` implique `Accept-Encoding`. Une dimension qui n'en implique aucun n'ajoute rien. Les noms sont émis triés par nom d'en-tête, pas dans l'ordre où vous avez déclaré les dimensions. |
| `Age` | Secondes entières depuis la publication de la représentation. Sa présence est la preuve locale la plus simple qu'une réponse est sortie du magasin. |
| `Warning` | `110 - "Response is Stale"`, et seulement sur une réponse servie au-delà de son intervalle de fraîcheur. |

La correspondance entre dimension et en-tête est
`VarianceDimension::vary_header` dans
`crates/suprnova-live/src/render_cache/variance.rs`. Deux tests du moteur
prouvent les moitiés `Locale` et `Encoding` de cette correspondance ainsi
que la valeur d'en-tête jointe :
`a_descriptor_orders_dimensions_and_bounds_values`
(`crates/suprnova-live/tests/render_cache_variance.rs`) asserte qu'un
descripteur portant les deux rapporte `["Accept-Encoding", "Accept-Language"]`,
et `cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
(`crates/suprnova-live/tests/render_cache_coherence.rs`) asserte que la même
paire émet `Accept-Encoding, Accept-Language` et qu'un descripteur sans
aucune dimension impliquant un en-tête n'émet aucun `Vary` du tout. Le fait
que `Media` implique `Accept` est documenté depuis le code ; aucun test ici
ne couvre cette paire.

Trois des valeurs de réponse sont assertées contre l'application en cours
d'exécution : `the_public_document_is_a_hit_whose_seed_still_promotes` lit
`private, max-age=300` sur `/live/public` et exige un en-tête `Age` sur la
seconde requête ;
`the_private_document_is_cached_per_principal_and_never_crosses` lit
`private, max-age=60` sur `/live/me` ;
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` lit
`private, no-store` sur le tableau de bord, aussi bien sur le rendu qui
publie sa coque que sur le hit assemblé qui suit, car cette valeur suit ce
que contiennent les octets et non le chemin de code qui les a produits.

## Les quatre états de fraîcheur

Chaque hit se résout à exactement un état parmi quatre avant que quoi que ce
soit ne soit servi.
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` les
règle. **Les deux fenêtres de péremption sont toutes deux mesurées depuis la
fin de l'intervalle de fraîcheur, et non empilées l'une après l'autre** -
c'est le détail sur lequel on trébuche :

| État | Âge depuis la publication | Ce que reçoit le visiteur |
|---|---|---|
| Frais | en dessous de `fresh_ms` | les octets stockés, sans `Warning` |
| Périmé-servable | au-delà de `fresh_ms` de moins de `stale_servable_ms` | les octets stockés immédiatement, sous `Warning`, avec une reconstruction bornée lancée derrière la requête |
| Périmé-sur-erreur | au-delà de `fresh_ms` d'au moins `stale_servable_ms`, et de moins de `stale_on_error_ms` | une reconstruction au premier plan ; les octets stockés sous `Warning` seulement si cette reconstruction échoue elle-même |
| Mort | au-delà de `fresh_ms` de la plus grande des deux fenêtres ou plus | rien ; la requête se rend |

`/live/todos` déclare `FreshnessPolicy::new(300_000, 60_000, 300_000)`, donc
il est frais pendant cinq minutes, périmé-servable pendant la sixième,
périmé-sur-erreur jusqu'à dix minutes, et mort ensuite.

Deux règles priment sur les bandes. Une représentation `PrivateCached`
n'est **jamais** servie périmée : au-delà de son intervalle de fraîcheur
elle est Morte, et c'est pourquoi `/live/me` déclare
`FreshnessPolicy::new(60_000, 0, 0)` - une bande de péremption s'y lirait
comme une promesse que le cache ne tient pas. Et un document à graine
publique stocké dont l'échéance de promotion est passée est Mort quels que
soient ses intervalles, parce qu'une graine passé son échéance ne peut plus
jamais être promue.

`stale_service_is_marked_and_rebuilt_in_the_background` pousse `/live/todos`
au-delà de la première frontière sur une horloge contrôlée et asserte le
corps servi, `Warning: 110 - "Response is Stale"`, et `Age: 300`. Ce qui
*fait* qu'une représentation quitte la bande de fraîcheur plus tôt - une
écriture, un avancement d'epoch - est le sujet de
[Générations de RenderCache](render-cache-generations.md).

## Requêtes conditionnelles et HEAD

Un client qui renvoie un `ETag` servi sous forme de `If-None-Match` obtient
un `304` sans corps, et un `HEAD` obtient les en-têtes sans corps. Ni l'un
ni l'autre n'atteint votre handler :

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` asserte
les trois contre l'application en cours d'exécution, y compris le fait que
le compteur de rendus ne bouge pas sur les deux derniers.

Une exception, et elle est délibérée : une réponse `Composite` ne répond
jamais `304`. Chaque assemblage est une représentation distincte - identités
d'îlots fraîches, nonce d'amorçage frais là où le document en a un - si bien
qu'un `304` dirait au client d'apparier le corps qu'il détient déjà avec des
en-têtes forgés pour cette requête. L'`ETag` d'une réponse assemblée reste
fort sur exactement les octets qui ont été envoyés ; il ne correspond
simplement jamais à une requête ultérieure. L'étape 7 de
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` renvoie
directement un validateur servi et asserte un `200` avec un `ETag`
différent.

## Une représentation qui appartient à une seule personne

`RepresentationClass::PrivateCached` stocke une représentation par visiteur
connecté. Elle est refusée à la construction sauf si la politique déclare
aussi la variance `Principal` ou `Tenant`, si bien que l'appariement ne peut
pas se défaire par accident :

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

Le handler derrière elle est un handler ordinaire. Il résout le visiteur
connecté et rend son nom :

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

Rien de plus n'est câblé pour que cela se mette en cache. La route porte le
même `AuthMiddleware::redirect_to("/login")` que le tableau de bord, donc un
visiteur anonyme est redirigé avant que le handler ne s'exécute, et le
principal lui-même est résolu à l'intérieur du rendu. Lire l'identité du
visiteur connecté dans la session est classé comme une **lecture
d'identité**, non comme une lecture de session, si bien que le rendu se
restreint à `PrivateCached`, que la clé porte un matériau opaque propre à
chaque principal, et que les deux concordent.

`the_private_document_is_cached_per_principal_and_never_crosses` connecte
deux visiteurs, frappe deux fois pour chacun sans aucun rendu, et asserte
que chaque corps nomme sa propre personne et pas celle de l'autre ; un
troisième visiteur provoque un rendu, parce qu'il ne partage rien avec ni
l'un ni l'autre. Le `Cache-Control` servi est `private, max-age=60`, donc
aucun proxy partagé ne se voit jamais proposer les octets. Le même test
montre l'autre moitié du marché : le rendu résout son principal via le
fournisseur qui lit la table `users`, donc ensemencer un troisième visiteur
invalide chaque entrée `/live/me` stockée, et la requête suivante de chacun
reconstruit. C'est l'invalidation à la granularité de la table faisant
exactement ce que décrit [Générations](render-cache-generations.md).

Deux conséquences de cette classification méritent d'être connues avant de
déclarer la classe :

- Une requête **anonyme** vers une route `PrivateCached` avec variance
  `Principal` se met en cache sous la clé `Anonymous`. Le rendu n'a résolu
  aucune identité, donc aucun matériau de principal n'a été observé, la clé
  dit `Anonymous`, et les deux concordent. Un visiteur connecté dérive une
  clé `Private` qui ne peut jamais atteindre cette entrée. Cela s'applique
  quand une telle requête rend réellement un `200`, ce que `/live/me` ne
  fait jamais - sa redirection de connexion répond `302`, et un `302` est
  refusé par l'éligibilité avant que rien de tout ceci ne soit consulté. Le
  test du framework qui atteint bien ce cas est
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  dans `framework/tests/render_cache/middleware.rs`.
- L'identifiant d'un **guard nommé** est un matériau de principal
  exactement de la même façon que celui du guard par défaut. Le lire
  enregistre une lecture de principal et, quand il y a un id, la valeur.

Et une règle qui n'a pas bougé : une route qui lit le principal *sans*
déclarer la variance `Principal` est refusée au stockage. Il n'y a aucun
moyen d'indexer une telle entrée par visiteur, donc elle n'est jamais
stockée plutôt que partagée. Toute autre valeur de session force encore
`Uncacheable` ; voir la liste de classification dans
[RenderCache](render-cache.md).

## Une coque percée de trous

`RepresentationClass::PublicShellStitched` est faite pour un document Live
dont le cadre est le même pour tout le monde et dont les îlots ne le sont
pas. L'entrée stockée ne détient que la coque. Aucun balisage d'îlot lié à
une identité et aucun instantané signé n'est jamais à l'intérieur des octets
stockés ; chaque hit remonte chaque îlot pour celui qui demande, sous une
autorité dérivée pour cette requête.

Le tableau de bord de ce dépôt est cette route :

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` asserte ce
que cela apporte et ce que cela coûte. L'entrée stockée est un
`EntryKind::Composite` à trois emplacements, un par îlot lié à une identité.
Un second principal reçoit sa réponse depuis cette coque, et les deux
documents ne diffèrent **que** par leurs balises d'îlot : le test retire les
trois balises d'îlot de chacun et compare ce qui reste, octet par octet. La
propre redirection de connexion de la route s'exécute encore à chaque hit :
un hit cousu est transmis à travers toute la chaîne de middleware de la
route avant que quoi que ce soit ne soit servi, si bien qu'un visiteur
anonyme obtient la redirection, jamais un document assemblé.

Un document cousu comportant au moins un emplacement est envoyé avec
`Cache-Control: private, no-store`, sur le rendu qui publie la coque autant
que sur chaque assemblage qui suit. Il détient les îlots d'un principal sous
une autorité redérivée pour une seule requête, et un `max-age` laisserait un
profil de navigateur partagé les rejouer pour celui qui s'assied ensuite ; le
chemin qui a produit les octets ne change rien à ce qu'ils contiennent. Un
`Composite` à zéro emplacement ne porte aucun octet propre à un principal,
seulement un nonce propre à la requête, si bien qu'il garde le `max-age`
privé de la classe comme toute autre représentation privée ;
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` dans
`framework/tests/render_cache/stitch.rs` l'asserte. Dans les deux cas, la
classe refuse `SharedCachePolicy::SMaxAge` à la construction de la
politique, si bien qu'aucun proxy partagé ne se voit jamais proposer les
octets.

Deux limites à connaître : la classe n'a de sens que sur une route dont la
chaîne se termine par le middleware de complétion Live, donc utilisez-la
avec `LiveDocument::render` et rien d'autre, et une entrée cousue n'est
jamais servie par le repli périmé-sur-erreur et ne déclenche jamais de
reconstruction en arrière-plan. Le chapitre
[Générations](render-cache-generations.md) dit ce que ce second point
signifie en pratique.

### Pourquoi Suprnova diverge

Laravel n'a aucun modèle de représentation côté serveur. Ses paquets de mise
en cache de réponses stockent la sortie rendue d'une route sous une clé que
vous composez vous-même - typiquement l'URL, parfois l'URL plus un suffixe
écrit à la main pour l'utilisateur connecté - et la rendent à la requête
suivante. Il y a une seule forme de chose stockée, c'est toujours un corps
fini, et le fait que deux visiteurs la partagent est une propriété de la
chaîne de caractères que vous avez construite.

Suprnova fait de la clé une déclaration et de la forme une conséquence. Vous
nommez la classe et les dimensions de variance ; le framework dérive la clé,
refuse `PrivateCached` sans dimension de partitionnement, compare ce que le
rendu a réellement observé à ce que la clé a réellement dit, et refuse de
stocker le rendu quand les deux ne concordent pas. Et parce que
`PublicShellStitched` existe, une page partagée à 95 pour cent et privée à
5 pour cent n'a pas à choisir entre ne rien mettre en cache et mettre en
cache quelque chose qu'elle ne devrait pas : la partie partagée est stockée
une fois et la partie privée est rendue à nouveau à chaque requête, les
octets privés n'entrant jamais dans le magasin.

## Suivant

- [Générations de RenderCache](render-cache-generations.md) - comment une
  représentation stockée cesse d'être à jour, et ce qui se passe ensuite
- [RenderCache](render-cache.md) - déclarer les politiques, la variance, et
  les raisons pour lesquelles un rendu n'est jamais stocké
- [Live](live.md) - les îlots pour lesquels une coque cousue a des trous
