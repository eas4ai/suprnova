# Déploiement de RenderCache

Un seul processus qui met en cache pour lui-même n'a besoin que de mémoire.
Plusieurs processus derrière un répartiteur de charge doivent s'accorder sur
ce qui est stocké, sur qui a le droit de reconstruire une entrée, et sur le
moment où quelque chose a cessé d'être vrai - et ils doivent s'accorder sans
qu'aucun d'eux ne puisse convaincre les autres qu'un contenu périmé est à
jour. RenderCache répond à cela avec des **profils** : un profil nomme les
fournisseurs qu'un processus construit, et rien d'autre ne change. Les
déclarations de routes, les politiques, les clés, le collecteur, le flux de
middleware et la couture sont identiques à chaque profil, et aucun type
exposé à l'application ne diffère entre eux.

Ce chapitre explique comment en choisir un et le câbler. Les trois profils
et ce que chacun fournit, les variables d'environnement que la configuration
du framework lit réellement, la migration que votre application doit lister
avant qu'un profil partagé ne démarre, l'endroit où l'installation prend
place dans votre amorçage, et ce que les paliers promettent et ne promettent
pas. L'application dogfood de ce dépôt exécute le profil embarqué par
défaut et est démarrée sur le profil Database par
`the_database_profile_serves_a_hit_through_the_sql_stores` dans
`app/tests/live_render_cache.rs`.

## Trois profils

| Profil | Entrées L1 | Leadership de reconstruction | Enregistrements d'instances Live |
|---|---|---|---|
| `embedded` (par défaut) | un fichier par clé, ou aucun | dans le processus | dans le processus |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`, `suprnova_live_promotions` |
| `redis` | un hash Redis par clé | un hash Redis par clé, plus un compteur de jetons | un hash Redis par enregistrement |

Ce qui ne bouge **pas** entre eux, c'est la vérité des générations. Le
registre des générations adossé à la base de données fait autorité à chaque
profil : quel que soit le palier qui a livré les octets, l'actualité est
prouvée contre la base de données - en la relisant sur le hit sous
`CoherenceMode::Authority`, ou par un bail de validation accordé à partir
d'une lecture antérieure de celle-ci sous `CoherenceMode::Lease`. C'est ce
qui garde un accélérateur dans son rôle d'accélérateur : Redis peut perdre
tout ce qu'il détient sans que rien de périmé ne soit prouvé à jour, parce
que rien de ce que Redis détient ne prouve l'actualité au départ.

Choisissez selon ce que vous avez réellement besoin de partager :

- **`embedded`** pour un seul processus, et pour plusieurs processus qui se
  contentent de garder chacun leur propre copie. Réglez
  `RENDER_CACHE_L1_DIR` et chaque processus gagne un palier fichier qui
  survit à son propre redémarrage.
- **`database`** quand plusieurs nœuds doivent partager les entrées stockées
  et élire un leader de reconstruction par clé, et que vous préférez ne pas
  ajouter une pièce mobile de plus au déploiement.
- **`redis`** quand la latence du palier partagé compte plus que sa
  durabilité, la base de données détenant toujours la vérité des générations
  en dessous.

## Les variables d'environnement

`RenderCacheConfig::from_env` lit celles-ci, dans
`framework/src/render_cache/config.rs` :

| Variable | Défaut | Signification |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | tout sauf `false` ou `0` ; `false` fait de `RenderCache::install` une opération sans effet |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database`, ou `redis` ; règle les deux lignes ci-dessous |
| `RENDER_CACHE_L1` | celui du profil | `disabled`, `file`, `database`, ou `redis` |
| `RENDER_CACHE_COORDINATOR` | celui du profil | `local`, `database`, ou `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4 096 | plafond d'entrées intra-processus |
| `RENDER_CACHE_L0_BYTES` | 128 Mio | plafond d'octets intra-processus |
| `RENDER_CACHE_L1_DIR` | non définie | le répertoire du palier fichier ; sous `embedded`, la régler est ce qui active L1 |
| `RENDER_CACHE_L1_BYTES` | 1 Gio | tout le répertoire pour le palier fichier, une seule entrée pour les paliers base de données et Redis |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, puis `redis://127.0.0.1:6379` | où se connectent les deux paliers de cache Redis |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | l'espace de noms de clés sous lequel écrivent les deux paliers de cache Redis |
| `RENDER_CACHE_LEASE_MS` | 30 000 | durée de vie d'un bail de reconstruction |
| `RENDER_CACHE_MAX_WAITERS` | 128 | plafond de requêtes en attente dans le processus |
| `RENDER_CACHE_FAILURE` | `open` | `open` sert la route sans cache en cas de défaillance d'un fournisseur, `closed` répond `503` |
| `APP_BUILD_ID` | la version du paquet de l'application (voir ci-dessous) | cantonne chaque entrée au build qui l'a produite |

Le profil est un raccourci, pas un verrou. `RENDER_CACHE_L1` et
`RENDER_CACHE_COORDINATOR` remplacent chacune leur propre moitié, si bien
qu'un déploiement qui veut ses entrées dans la base de données mais ses
baux de reconstruction dans le processus le dit exactement plutôt que de
choisir le profil entier le plus proche.

Une variable dotée d'un ensemble fermé de valeurs acceptées et réglée sur
quelque chose en dehors de cet ensemble fait échouer le démarrage avec un
message qui nomme la variable. La valeur rejetée n'est jamais répétée dans
ce message, parce qu'une valeur d'environnement peut porter un secret.

**Réglez `APP_BUILD_ID` explicitement, une fois par déploiement.** Elle est
mêlée à chaque clé de recherche, donc la changer est ce qui empêche un
nouveau build de servir des entrées publiées par le précédent. En l'absence
de la variable, `RenderCacheConfig::from_env` se replie sur la propre
version du paquet de votre application : `#[suprnova::main]` enregistre
`CARGO_PKG_VERSION` à partir de la compilation propre de la crate de
l'application, au moment où elle charge l'environnement, et c'est cette
valeur enregistrée sur laquelle la valeur par défaut se replie ici. Seul un
binaire qui n'étend jamais `#[suprnova::main]` se replie plus loin encore,
sur la version de cette crate du **framework** elle-même - nommée ainsi
parce qu'elle est sinon facile à confondre avec celle de l'application.
Dans les deux cas la valeur ne bouge que quand quelqu'un incrémente un
numéro de version, et une version de paquet change rarement par
déploiement : un déploiement qui change un template, une traduction, ou un
handler sans incrémenter de version garde le même identifiant de build et
peut servir des entrées publiées par le build précédent. Réglez-la sur
quelque chose qui change chaque fois que vous livrez - un identifiant de
commit ou un identifiant de version :

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

Une installation qui ne lit jamais l'environnement règle la même valeur
dans le code avec `RenderCacheConfig::with_build_id`, qui l'emporte sur ce
qu'a choisi `from_env` - un `APP_BUILD_ID` explicite inclus - pour une
application qui dérive son propre identifiant par déploiement de façon
programmatique.

Quelle que soit la valeur que vous réglez, le binaire de production qui la
lit est censé être construit dans la [forme de build de
production](deployment.md#production-build-shape) de Suprnova - les
fonctionnalités par défaut éteintes, `testing` réservé à `cargo test` seul.

Le propre registre d'instances de Live se configure séparément, parce qu'il
relève de l'autorité de Live plutôt que du stockage du cache :
`LIVE_LEDGER_DRIVER` (`memory`, `database`, ou `redis`), `LIVE_REDIS_URL`,
et `LIVE_REDIS_PREFIX`. Un déploiement peut exécuter le cache sur un palier
et le registre sur un autre.

## La migration que votre application doit lister

Le schéma de RenderCache est détenu par le framework et appliqué par
l'application. Votre `Migrator` le liste, si bien que `suprnova migrate`
provisionne les tables aux côtés des vôtres :

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

C'est `app/src/migrations/mod.rs` mot pour mot, et les deux ne sont pas
interchangeables :

- **`Migration`** crée les trois tables `suprnova_render_*` qui détiennent
  la vérité durable des générations : les générations courantes, un journal
  de changements en ajout seul, et l'epoch d'autorité. Chaque profil en a
  besoin, y compris `embedded`, parce que la vérité des générations ne passe
  jamais dans un palier de cache.
- **`TierMigration`** crée les quatre tables que lisent le magasin L1 base
  de données et le coordinateur de reconstruction base de données. Seul un
  profil qui les atteint en a besoin - mais `RenderCache::install` refuse de
  démarrer le profil Database sans elles, si bien que la lister est ce qui
  fait de `RENDER_CACHE_PROFILE=database` un choix de configuration que
  votre application peut réellement faire.

Une application qui règle `RENDER_CACHE_ENABLED=false` n'a besoin d'aucune
des deux : l'installation retourne le routeur intact, ne sonde rien,
n'assemble aucun runtime, n'enregistre aucun middleware, et laisse le côté
écriture non instrumenté, si bien que rien ne paie pour un cache qui est
éteint.

## L'installer

`RenderCache::install` est asynchrone, parce qu'elle sonde la présence des
tables et envoie un ping à chaque point de terminaison Redis distinct que la
configuration utiliserait avant d'assembler quoi que ce soit.
`Application::try_routes_async` est le point d'accroche qui l'héberge. Ceci
est `app/src/live/mod.rs`, et la séparation en deux fonctions vaut la peine
d'être copiée :

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` est la moitié interne synchrone : elle enregistre les routes Live
réservées, les routes de documents, et chaque politique de cache, et
n'installe aucun middleware. `cmd/main.rs` atteint
`routes_with_render_cache` via `Application::try_routes_async`, et le
serveur du scénario navigateur dans `app/examples/live_dogfood_host.rs`
l'attend directement.

Le point d'accroche de configuration en dessous n'est pas décoratif. Un test
qui a besoin d'un profil différent ou d'une horloge qu'il peut déplacer n'a
aucun autre moyen d'entrer, et il importe qu'il installe *les mêmes* routes,
politiques et ordre de middleware que le serveur, en ne différant que par la
configuration passée. Les deux démarrages dogfood passent par lui :
`the_database_profile_serves_a_hit_through_the_sql_stores` passe une
configuration de profil Database, et
`stale_service_is_marked_and_rebuilt_in_the_background` en passe une portant
une horloge réglable. Voir « Tester une route mise en cache » dans
[Exploitation de RenderCache](render-cache-operations.md).

Deux règles d'ordre, toutes deux à la charge de l'appelant :

1. Chaque route et chaque groupe doivent être activés **avant** `install`,
   qui lit tout ce qui a été enregistré jusque-là.
2. `install` ajoute à la chaîne de middleware globale, elle doit donc
   s'exécuter **après** les middlewares de session, de locale et d'identité
   dont le middleware de cache lit l'état à la portée de la requête pendant
   qu'il dérive une clé de recherche.

L'installation échoue en mode fermé, via deux sondes. Elle vérifie que les
tables que la configuration atteindrait existent, et elle envoie un ping à
chaque point de terminaison Redis distinct que la configuration utiliserait,
une fois par point de terminaison. L'échec de l'une ou de l'autre arrête le
démarrage avec une seule phrase actionnable nommant la migration ou la
variable à corriger, si bien que rien n'est jamais servi contre une table
absente ou un point de terminaison auquel rien ne répond. (Le propre
registre d'instances de Live est sondé séparément, par `Server::run`, avant
qu'aucune requête ne soit servie.)

## Choisir où vivent les entrées d'une route

Le profil décide de ce que L1 *est* ; la politique décide quelles routes
l'utilisent. Le constructeur stocke dans L0 seulement sauf si une route en
déclare autrement, si bien qu'un palier partagé est peuplé à dessein :

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` prouve
l'aller-retour de bout en bout sur le profil Database : l'entrée publiée est
relue depuis le magasin SQL sous la clé que le middleware a dérivée, puis L0
est vidé et la requête suivante reçoit encore une réponse sans rendu. Voir
[Représentations de RenderCache](render-cache-representations.md) pour
décider route par route.

## Une seule suite de conformité, tous les fournisseurs

Chaque magasin répond à la même suite.
`framework/tests/render_cache/store_conformance.rs` exécute les scénarios de
fournisseur du moteur - écrits contre le seul trait `RenderStore` - sur le
L1 adossé à des fichiers, le L1 SQL sur SQLite, PostgreSQL et MySQL, et le
L1 Redis, et le magasin intra-processus répond à la même suite dans la crate
du moteur. PostgreSQL, MySQL et Redis passent par des tests ignorés que
`scripts/check-postgres.sh`, `scripts/check-mysql.sh` et
`scripts/check-redis.sh` sélectionnent par leur nom contre de vrais
serveurs. Un fournisseur n'est pas « supporté » ici parce qu'il existe ; il
est supporté parce qu'il passe les mêmes mots que tous les autres.

## Ce que les paliers promettent, et ce qu'ils ne promettent pas

- **Aucune attente inter-nœuds.** Une clé qu'un autre nœud reconstruit déjà
  est un contournement : ce nœud rend et ne publie rien. Un calcul dupliqué
  borné entre nœuds est accepté ; deux publications acceptées ne le sont
  pas, et c'est la barrière de publication propre au magasin qui interdit la
  seconde.
- **Un backend perdu est un miss, jamais une mauvaise réponse.** Une
  éviction, une expiration ou un redémarrage de Redis fait manquer les
  entrées et disparaître les instances. La vérification de cohérence contre
  le registre des générations en base de données s'exécute à chaque hit,
  quel que soit ce qui a servi les octets.
- **Des octets altérés sont un miss.** Les octets d'une entrée dans une
  ligne ou dans un hash forment une trame de codec signée, si bien qu'une
  valeur partielle, tronquée ou altérée échoue à sa vérification d'intégrité
  et est traitée comme un miss plutôt que servie.
- **Les paliers base de données et Redis n'évincent pas pour faire de la
  place.** `RENDER_CACHE_L1_BYTES` y borne une seule entrée, pas la table ni
  l'espace de clés ; la croissance est bornée par la rétention à la place.
  Seul le palier fichier borne un répertoire entier, parce qu'il est seul à
  posséder ce répertoire.
- **Les adaptateurs Redis visent une instance Redis 7 ou plus récente
  unique.** Redis Cluster est refusé : les scripts touchent des clés qu'ils
  ne déclarent pas, et ils lisent l'horloge du magasin avec `TIME` à
  l'intérieur d'un script.
- **MySQL exige 8.0.19 ou plus récent** pour une classification précise des
  clés dupliquées. Les MySQL plus anciens et MariaDB rapportent une
  collision que ce build n'attribuera pas à une table, si bien qu'il se
  dégrade en une erreur de fournisseur indisponible - la direction sûre, et
  rien n'est accordé deux fois dans un cas comme dans l'autre.
- **Chaque décision d'expiration inter-nœuds est prise sur l'horloge du
  backend**, lue à l'intérieur de l'opération qui agit dessus. Un nœud dont
  l'horloge avance ne peut ni prolonger un bail, ni cacher une entrée vivante
  à ses pairs, ni déclarer écoulé l'enregistrement d'un pair.

### Pourquoi Suprnova diverge

Les paquets de mise en cache de réponses de Laravel héritent du magasin de
cache que vous avez déjà configuré, si bien que « déployer le cache sur
plusieurs nœuds » revient à pointer `CACHE_STORE` sur Redis et à faire
confiance à ce qui s'y trouve. Il n'y a aucune notion séparée de qui a le
droit de reconstruire une entrée, aucune barrière qui empêche deux workers
de publier des octets contradictoires pour la même clé, et - le plus lourd
de conséquences - aucune autorité sous le magasin. Si Redis détient une
page, la page est servie ; si Redis est vidé, tout est recalculé. Le magasin
*est* la vérité.

Suprnova sépare délibérément les deux. Le palier partagé détient des octets
et rien d'autre ; la base de données détient la vérité des générations à
chaque profil, et c'est contre elle qu'un hit est vérifié. C'est pourquoi
perdre Redis coûte ici de la latence plutôt que de la correction, pourquoi
une reconstruction est louée et une publication barrée plutôt que courue, et
pourquoi les mêmes déclarations de routes s'exécutent sans changement depuis
`cargo run` sur un portable jusqu'à une flotte coordonnée par la base de
données. Le coût est une migration que votre application doit lister et une
lecture en base sur le chemin du hit qu'un simple cache clé-valeur ne paie
pas - une instruction, ou zéro sous un bail de validation. Voir
[Générations de RenderCache](render-cache-generations.md) pour ce que cette
lecture apporte.

## Suivant

- [Exploitation de RenderCache](render-cache-operations.md) - les commandes
  console, la télémétrie, l'hygiène disque, et quoi faire quand quelque
  chose ne va pas
- [Déploiement](deployment.md) - la checklist de production environnante
- [Migrations](migrations.md) - comment la liste du `Migrator` ci-dessus est
  appliquée
