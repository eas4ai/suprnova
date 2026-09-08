# Exploitation de RenderCache

Un cache que vous ne pouvez pas voir est un cache auquel vous ne pouvez pas
vous fier. RenderCache répond directement à deux questions d'opérateur, et
sans jamais imprimer une page stockée : **que détient ce nœud sous cette
clé, et est-ce encore à jour ?** et **comment fait-on tout arrêter ?** Il
répond à une troisième - « cette route est-elle seulement servie depuis une
copie stockée ? » - par la télémétrie et par l'en-tête `Age` plutôt que par
une commande, parce que cette question porte sur le trafic plutôt que sur
une entrée. Il y a deux commandes console, six compteurs de télémétrie, un
balayage disque borné, et un levier d'urgence.

Ce chapitre est la surface d'exploitation : les commandes, exactement ce
qu'elles impriment et ce qu'elles peuvent voir ; les compteurs et leurs
ensembles fermés de résultats ; comment le palier fichier récupère du
disque ; comment tester une route mise en cache pour que le test prouve la
mise en cache plutôt qu'une simple réponse ; quoi faire quand quelque chose
ne va pas, y compris la procédure multi-nœuds qu'exige une restauration de
base de données ; et comment la performance propre du cache est mesurée et
ce que ces chiffres valent honnêtement. Les exemples de commandes sont ceux
que `the_operator_commands_inspect_without_a_body_and_advance_the_epoch`
exécute via le propre point d'entrée console de ce dépôt dans
`app/tests/live_render_cache.rs`.

## Les deux commandes console

Les deux sont des commandes masquées, enregistrées par le framework et
atteignables via le binaire `console` de votre projet comme n'importe quelle
autre. Ni l'une ni l'autre n'imprime jamais un corps stocké ni une identité
de dépendance brute.

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** rapporte la forme d'une entrée stockée : sa
classe de représentation, ses `body_bytes`, ses autres métadonnées, et
l'epoch d'autorité courant à côté, afin que vous puissiez déterminer si
l'entrée que vous regardez fait encore autorité en direct ou a déjà expiré
entre-temps. Elle imprime `no entry (current epoch: {epoch})` quand la clé
ne nomme rien qu'elle puisse voir, et elle échoue - elle ne rapporte pas un
succès - sur une clé non analysable ou sans runtime installé.

**Elle lit le L0 intra-processus de ce processus et rien d'autre.**
`RenderCache::inspect` cherche la clé dans L0 seulement ; elle ne consulte
jamais le palier L1. Sur le profil Database ou Redis, cela compte : une
entrée qui est vivante dans `suprnova_render_entries` ou dans Redis, publiée
par un autre nœud ou par celui-ci avant un redémarrage, imprime ici
`no entry` sauf si ce processus l'a servie depuis son démarrage. Lisez le
rapport comme « ce que ce nœud a en mémoire », jamais comme « ce que le
déploiement a stocké ». Il en va de même de `RenderCache::store_inspection`,
qui rapporte l'occupation de L0 et l'epoch courant.

La clé est le texte que la recherche elle-même utilise : `rk1.` plus 43
caractères base64url, ce que la journalisation et la télémétrie de votre
application peuvent faire apparaître. Ce n'est pas un second hash de quoi
que ce soit, si bien qu'une clé qu'un opérateur détient nomme exactement une
entrée.

Cette affirmation d'absence de corps est vérifiée, pas seulement énoncée. Le
test prend le document qui a réellement été servi, le découpe en lignes, et
exige que **chaque** ligne non triviale soit absente de ce que le rapport
d'inspection a imprimé.

**`render-cache:epoch-advance`** est l'invalidation d'urgence. Elle fait
avancer l'epoch d'autorité et imprime `epoch advanced to {epoch}`. Parce que
l'epoch est intégré à chaque clé de recherche, cela met les entrées stockées
hors d'atteinte sans rien à énumérer ni rien à supprimer. Le test asserte la
ligne imprimée puis la conséquence qui compte : après la commande, la route
se rend à nouveau.

**Sur le nœud qui l'exécute**, l'effet est immédiat : la commande abandonne
le bail d'epoch de ce processus et vide son palier intra-processus, si bien
que sa toute prochaine requête dérive des clés sous le nouvel epoch et ne
trouve rien. (Cette dernière clause vaut tant que l'epoch ne bouge que vers
l'avant, ce qui est le cas ordinaire ; après une restauration de base de
données, la valeur avancée peut être une valeur que le déploiement a déjà
utilisée, donc voir « Restaurer la base de données » ci-dessous.) **Sur tout
autre nœud**, le registre a bougé mais ce processus détient encore son
ancien epoch loué et son propre L0, et il se met à jour à sa prochaine
lecture d'autorité - immédiatement sous `CoherenceMode::Authority`, et
jusqu'à `max_age_ms` plus tard sous `CoherenceMode::Lease`. Exécutez la
commande sur chaque nœud, ou redémarrez les autres. « Restaurer la base de
données » ci-dessous donne la procédure complète et les tests qui la
soutiennent.

Recourez-y quand quelque chose ne va pas avec le contenu mis en cache et que
vous ne pouvez pas attendre l'expiration individuelle des entrées, et après
un job qui a changé ce que montrent les pages mises en cache (voir
« lacunes connues » dans
[Générations de RenderCache](render-cache-generations.md)).

## Changements de permissions

`RenderCache::bump_permission_version().await?` est le seul appel
d'invalidation qu'une application fait à la main, et ce n'est pas vraiment
une commande d'exploitation - il a sa place dans le chemin de code qui
change ce qu'un utilisateur connecté est autorisé à faire. Il fait avancer
une génération persistée que chaque rendu indexé par visiteur observe, il
survit à un redémarrage, et il rejoint la transaction dans laquelle
s'exécute le changement de rôle quand il y en a une. Sans lui, un
utilisateur dont les permissions viennent de changer continue de
correspondre à ce qui était mis en cache sous son précédent jeu de
permissions.

## Télémétrie

Six noms de compteurs fermés, et rien dans aucun d'eux ne nomme un palier,
un fournisseur, ou un backend :

| Compteur | Attribut |
|---|---|
| `suprnova.render_cache.lookups` | `outcome` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | aucun |
| `suprnova.render_cache.rebuilds` | aucun |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |

`lookups` et `hits` portent le même ensemble fermé de huit résultats :

- `l0`, `l1` - une entrée fraîche servie depuis le palier intra-processus ou
  depuis le palier partagé.
- `conditional` - un hit frais dont le `If-None-Match` a correspondu,
  répondu par un `304`.
- `stale` - une entrée périmée-servable servie immédiatement, ou le repli
  périmé-sur-erreur après l'échec d'une reconstruction au premier plan.
- `miss` - rien trouvé, une reconstruction périmée-sur-erreur en cours, ou
  une entrée morte.
- `bypass` - un paramètre de requête non déclaré, une dimension de variance
  déclarée impossible à résoudre, ou une liste d'attente épuisée.
- `moved` - la relecture après le rendu a trouvé qu'une dépendance ou
  l'epoch avait changé ; le candidat a été écarté, jamais publié.
- `declined` - le rendu n'était pas stockable : éligibilité, rapport
  d'observation débordé, classification `Uncacheable`, règle de document
  Live, ou une borne.

`hits` ne s'incrémente que pour `l0`, `l1`, `conditional` et `stale`.
`publications` ne compte qu'un magasin répondant « publié », jamais une
tentative barrée ou rejetée. `rebuilds` compte une unité par reconstruction
en arrière-plan lancée.

Les deux compteurs de couture portent leurs propres ensembles : `assembled`
et `fail_document` pour les assemblages ; `rendered`, `omitted`, `fallback`
et `failed` pour les emplacements.

**Un taux de `declined` élevé est le signal sur lequel il vaut la peine
d'alerter.** Il signifie que des routes que vous avez activées se rendent et
servent correctement sans jamais être stockées, et la réponse a l'air
identique dans les deux cas. La vérification locale la plus rapide, ce sont
deux requêtes de suite : si la seconde ne porte pas d'en-tête `Age`, rien
n'a été stocké.

## Hygiène disque

Seul le palier fichier a besoin d'être balayé, et il se balaie surtout
lui-même.

`FileRenderStore` stocke un fichier par clé, à plat sous
`RENDER_CACHE_L1_DIR`. Une entrée est morte quand son âge depuis la
publication atteint la rétention avec laquelle elle a été publiée, ou quand
son epoch de barrière est plus ancien que l'epoch courant. La rétention vient
du même bord de mort conscient de la classe qu'utilise la vérification de
fraîcheur en direct, si bien que le fichier d'une entrée privée est retiré
plus tôt que celui d'une entrée publique et qu'un balayage ne peut jamais
être en désaccord avec une vérification de fraîcheur sur le fait qu'une
entrée est vraiment morte.

`sweep` retire au plus 64 entrées par appel, publication la plus ancienne
d'abord, et retourne s'il en reste. Il s'exécute automatiquement à chaque
256e publication, si bien qu'un répertoire en bonne santé n'a besoin
d'aucune attention. `RenderCache::sweep()` le déclenche explicitement quand
vous le souhaitez, et un arriéré plus grand que la limite d'un appel
s'écoule sur les déclenchements suivants au lieu de bloquer sur un long
parcours.

Deux choses que le balayage n'est pas :

- **Un avancement d'epoch ne touche pas L1.** Il vide L0 purement et
  simplement, parce que c'est de la mémoire intra-processus sans rien à
  réconcilier, et il laisse sur le disque chaque fichier antérieur à l'epoch
  jusqu'à ce qu'un balayage le récupère. C'est de l'hygiène disque, pas un
  problème de correction - les fichiers sont déjà inatteignables par une
  recherche.
- **Le palier base de données n'a aucun balayage automatique**, et n'est
  récupéré que par `RenderCache::sweep()`. Le palier Redis n'en a besoin
  d'aucun : chaque entrée qu'il stocke porte une expiration et Redis
  récupère les octets lui-même.

La publication est sûre en cas de crash. Elle écrit un fichier temporaire,
le fsync, le renomme par-dessus la cible, et fsync le répertoire parent, si
bien qu'un lecteur ne voit jamais que le fichier complet précédent ou le
nouveau fichier complet. À l'ouverture, le magasin retire tout fichier
temporaire résiduel et tout fichier qui échoue à sa vérification de trame,
traitant une écriture partielle comme auto-réparatrice plutôt que comme une
entrée définitivement empoisonnée.

## Tester une route mise en cache

Un test qui asserte qu'une route mise en cache répond correctement passe que
la réponse vienne du magasin ou d'un rendu frais. Chaque affirmation doit
porter sur quelque chose que seule une entrée stockée réellement servie peut
produire. Quatre motifs font cela, et les propres tests dogfood de ce dépôt
les utilisent tous les quatre : `app/tests/live_render_cache.rs` avec le
harnais dans `app/tests/live_support/mod.rs`.

**1. Comptez les rendus du côté handler du cache.** Enregistrez un
middleware compteur *après* `RenderCache::install`. L'enregistrement ajoute
à la fin, il atterrit donc plus près du handler que `RenderCacheMiddleware`,
et une requête à laquelle le cache répond revient avant de l'appeler :

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// After the install, so it only sees requests the cache forwarded.
render_counter::register();
```

La différence entre deux lectures de `render_counter::renders()` est alors
le nombre de rendus que le cache n'a pas évités, et rien d'autre - à la
différence de corps identiques ou d'un en-tête `Age`, qui ont tous deux des
explications honnêtes hors cache. Chaque assertion de hit dans
`an_orm_write_invalidates_the_todos_document_through_generations` repose
dessus. Attendez un rendu que vous n'avez pas déclenché (une reconstruction
en arrière-plan) avec la propre barrière du compteur,
`wait_until_renders_at_least`, jamais avec une temporisation.

**L'exception est une route `PublicShellStitched`, et elle n'est pas
mince.** Un hit cousu est délibérément transmis à travers toute la chaîne de
la route - son garde d'autorisation doit s'exécuter à nouveau, et seul le
middleware de complétion Live à la fin de cette chaîne sert le hit. Un
middleware compteur enregistré globalement après l'installation se trouve en
dehors de la chaîne propre à la route, il est donc atteint sur un hit cousu
exactement comme sur un miss. Sur une telle route, le compteur ne peut pas
du tout porter l'affirmation « aucun handler ne s'est exécuté ».

Assertez plutôt sur quelque chose que seul un document assemblé peut
produire, ce que fait
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` : l'entrée
stockée est un `EntryKind::Composite` avec le nombre d'emplacements attendu
(`inspect_route_for_test`), la réponse porte
`Cache-Control: private, no-store` - la valeur que le répondeur composite
épingle sur un assemblage doté d'emplacements, et seulement après qu'un
document entier a été assemblé - et les documents de deux principaux
diffèrent par leurs balises d'îlot et nulle part ailleurs. « Doté
d'emplacements » est le mot qui compte : un `Composite` à zéro emplacement
garde à la place le `max-age` privé de la classe, si bien que cette
assertion convient à une route qui contient des îlots et non à une route
qui n'en contient pas. Ce test asserte `renders() == before + 1` sur un
hit, et dit dans sa propre note pourquoi c'est la lecture honnête plutôt
qu'un échec.

**2. Relisez l'entrée.** Deux appels de façade sont de l'API publique
ordinaire : `RenderCache::store_inspection()` rapporte l'occupation de L0,
les octets, et l'epoch courant, et `RenderCache::inspect(key_text)` rapporte
les métadonnées sans corps d'une entrée. À côté d'eux, le framework expose
des points d'accroche de test masqués - `#[doc(hidden)]`, et nommés
`_for_test` pour que rien ne les prenne pour de l'API applicative :

| Point d'accroche | Ce qu'il donne à un test |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | le texte de clé que le middleware dérive **à l'epoch 1**, la valeur que la migration ensemence |
| `RenderCache::key_for_route_at_epoch_for_test(pattern, params, login, epoch)` | la même, sous un epoch que vous nommez |
| `RenderCache::inspect_route_for_test(pattern)` | l'entrée L0 de cette clé d'epoch 1 : classe, sorte, statut, `body_bytes`, emplacements |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | la même, depuis le palier L1 configuré |
| `RenderCache::clear_l0_for_test()` | vide L0 et laisse tranquilles L1, l'epoch et le coordinateur |

L'epoch compte parce qu'il fait partie de la clé.
`key_for_route_for_test` code en dur l'epoch 1, si bien qu'un test qui a
avancé l'epoch - sur ce nœud ou, via le registre, sur un autre - doit nommer
le nouveau avec `key_for_route_at_epoch_for_test` ou il cherchera une clé
sous laquelle rien n'a été publié.

`the_public_document_is_a_hit_whose_seed_still_promotes` utilise
`store_inspection` et `inspect_route_for_test` pour asserter que l'entrée
existe et est stockée sous la classe déclarée ;
`the_database_profile_serves_a_hit_through_the_sql_stores` utilise
`inspect_l1_for_test` puis `clear_l0_for_test`, qui est le seul moyen de
prouver qu'une requête ultérieure est venue de L1 plutôt que de la mémoire.

**3. Déplacez l'horloge au lieu d'attendre.** L'horloge que lit le runtime
est réglable sur un `RenderCacheConfig` et jamais par `from_env`, si bien
qu'un test qui a besoin d'une bande de fraîcheur installe la sienne :

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Bound to its own name first: passing `Arc::clone(&clock)` inline leaves
// the compiler inferring the trait object as the clone's return type.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... install through the application's own configuration seam, then:
clock.advance_ms(300_001);
```

`AdjustableTestClock` vient de `suprnova::live::testing`, et `unix_now_ms`
est la propre lecture d'horloge murale du harnais, si bien qu'une horloge
réglable démarre là où est celle du système plutôt qu'à une origine
temporelle que le reste du processus contesterait. (Il s'agit d'un zéro
d'horloge, pas de l'epoch d'autorité que ce chapitre désigne par ailleurs
avec ce mot.) Le harnais enveloppe la paire sous
`setup_app_with_clock` et `advance_clock_ms`, la seconde paniquant plutôt
que de ne rien faire silencieusement quand le démarrage a pris l'horloge
système. `stale_service_is_marked_and_rebuilt_in_the_background` est le
test.

**4. Comptez les instructions SQL.** Un cache qui a sauté le handler mais
consulte quand même la base à chaque hit satisfait tous les compteurs du
côté handler et coûte tout de même un aller-retour.
`DbConnection::observe_statements_for_test` pointe le callback de métrique
de SeaORM sur un compteur à vous, et il voit les instructions sur le pool et
sur chaque transaction démarrée depuis lui :

```rust
// Immediately after connecting, before the connection is cloned or bound
// into the container: installing needs sole ownership of the pool, and the
// call reports `false` rather than counting nothing silently.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

Rien n'est dit au callback sur l'instruction - aucun texte SQL, aucune
valeur liée - parce qu'un décompte est tout l'enjeu.
`framework/tests/render_cache/bypass.rs` est écrit entièrement sur ce
motif : `a_lease_mode_hit_runs_nothing_and_issues_no_statement` tient un hit
en mode bail à zéro instruction,
`an_authority_mode_hit_issues_exactly_one_statement` tient un hit en mode
autorité à une, et `the_epoch_is_read_once_at_first_use` mesure deux miss
l'un contre l'autre pour montrer que l'epoch coûte une lecture par runtime.

Deux habitudes à garder. Démarrez le harnais via le point d'accroche de
configuration de votre propre application plutôt que via un routeur
construit à la main, afin que le test installe les mêmes routes, politiques
et ordre de middleware que le serveur. Et n'ajoutez jamais d'attente
temporisée : chaque barrière ci-dessus est une barrière d'état sur un
compteur, et c'est ce qui rend ces tests reproductibles plutôt
qu'instables.

## Quand quelque chose ne va pas

- **Une page montre un contenu que vous savez ancien.** Vérifiez si la route
  stocke tout court (deux requêtes, cherchez `Age`). Si oui, et que
  l'écriture qui aurait dû l'invalider venait d'un worker de file d'attente,
  d'une tâche planifiée, ou d'une commande console, cette écriture n'a rien
  fait avancer : exécutez `render-cache:epoch-advance` (par nœud - voir le
  dernier point).
- **Une page que vous attendiez en cache ne porte jamais d'en-tête `Age`.**
  Elle est refusée, elle n'échoue pas. Parcourez la liste de classification
  dans [RenderCache](render-cache.md) : une lecture de session, une lecture
  d'identité sur une route sans variance `Principal`, une lecture de locale
  sans variance `Locale`, une vérification d'autorisation, ou une lecture
  SQL brute.
- **Un backend est injoignable.** `RENDER_CACHE_FAILURE` décide : `open` (le
  défaut) sert la route sans cache, `closed` répond un `503` nu. Un backend
  absent au démarrage arrête plutôt le démarrage, avec une phrase nommant la
  migration ou la variable à corriger.
- **Redis a été vidé ou redémarré.** Les entrées manquent et sont rendues à
  nouveau. Rien de périmé ne peut être prouvé à jour : l'actualité est
  prouvée contre le registre des générations en base de données, jamais
  contre le palier qui détenait les octets.
- **Un leader de reconstruction est mort en plein travail.** Son bail est
  repris une fois que l'heure du magasin dépasse l'expiration, et la propre
  publication de l'ancien leader est barrée plutôt que mise en course avec
  la nouvelle. Il ne publie rien ; la réponse de sa requête est tout de même
  servie.
- **Un fichier L1 a été déchiré par un crash ou un disque plein.** Rien ne
  le sert. Chaque fichier porte une empreinte sur sa propre trame, si bien
  qu'un fichier tronqué ou altéré échoue à cette vérification et est un
  miss ; le magasin le retire, ainsi que tout fichier temporaire résiduel, à
  sa prochaine ouverture. Une écriture partielle est ici auto-réparatrice
  plutôt qu'une entrée définitivement empoisonnée.
- **La base de données a été restaurée depuis une sauvegarde.** Ce cas a une
  procédure plutôt qu'une phrase ; voir « Restaurer la base de données »
  ci-dessous.
- **Vous avez besoin que tout disparaisse, maintenant.**
  `render-cache:epoch-advance`. Sur plus d'un nœud, exécutez-la sur chacun,
  ou redémarrez ceux sur lesquels vous ne l'avez pas exécutée : l'avancement
  déplace l'epoch du registre pour tout le déploiement, mais il ne vide L0
  et n'abandonne l'epoch loué que dans le processus qui l'a exécuté. La
  procédure de restauration ci-dessous explique pourquoi.

## Restaurer la base de données

Le registre des générations est l'autorité contre laquelle chaque hit est
prouvé, si bien que restaurer la base de données change ce que « à jour »
signifie pour chaque entrée déjà stockée. Trois faits tirés du code décident
de ce qu'une entrée stockée fait ensuite, et aucun d'eux n'est « elle est
discrètement écartée ».

**Une entrée déplacée n'est pas automatiquement retenue.** La comparaison de
cohérence (`CoherenceCheck::compare`) est une inégalité dans *l'un ou
l'autre* sens, si bien qu'une entrée stockée dont les générations observées
diffèrent de celles du registre restauré est un déplacement quel que soit le
sens des nombres. Mais un déplacement n'est pas un refus de servir : le
middleware évalue une entrée déplacée à un âge effectif d'au moins son
intervalle de fraîcheur (`freshness_state` dans
`framework/src/render_cache/middleware.rs`), et sur une route qui déclare
une fenêtre périmée-servable cela la place dans la bande périmée-servable.
**Le visiteur reçoit une fois la copie d'avant la restauration, sous
`Warning`, pendant que la reconstruction s'exécute derrière la requête.**
C'est le même passage de relais que décrit
[Générations de RenderCache](render-cache-generations.md), et l'étape 4 de
`an_orm_write_invalidates_the_todos_document_through_generations` l'asserte.
Une route `PrivateCached` ne fait jamais cela - son bord de mort est son
bord de fraîcheur - et une route qui n'a déclaré aucune fenêtre
périmée-servable non plus ; toutes deux reconstruisent au premier plan.

**Un avancement d'epoch est propre au processus.**
`RenderCache::advance_epoch` fait avancer l'epoch du registre, puis abandonne
le bail d'epoch de *ce* processus et vide le L0 de *ce* processus. Ses nœuds
frères gardent les deux : leurs entrées L0 et l'epoch d'avant la
restauration qu'ils ont loué. Chacun l'apprend à sa prochaine lecture
d'autorité - immédiatement à son tout prochain hit sous
`CoherenceMode::Authority`, et jusqu'à `max_age_ms` plus tard sous
`CoherenceMode::Lease` - ce qui est exactement ce que mesurent les trois
tests `an_epoch_advanced_by_another_node_*` dans
`framework/tests/render_cache/middleware.rs`. D'ici là, un frère peut servir
une entrée d'avant la restauration, et sur une route périmée-servable il
peut la servir sous `Warning` comme ci-dessus.

**Le palier partagé n'est pas balayé par un simple changement d'epoch.** Le
balayage du palier fichier retire une entrée quand sa rétention s'est
écoulée *ou* que son epoch de barrière est `<` à l'epoch courant. Si
restaurer la sauvegarde a abaissé l'epoch du registre en dessous de valeurs
sous lesquelles le déploiement avait déjà publié des entrées, ces entrées
portent un epoch de barrière désormais *supérieur* à l'epoch courant, si
bien que cette clause ne les récupère pas ; elles attendent plutôt la fin de
leur rétention. Le palier base de données n'est balayé que par un
`RenderCache::sweep()`
explicite. Le palier Redis se récupère lui-même, mais selon le calendrier
propre à Redis : chaque hash d'entrée est stocké sous
`<RENDER_CACHE_REDIS_PREFIX>entry:<key>` (préfixe par défaut
`suprnova_render:`) avec un `PEXPIRE` réglé depuis la rétention de l'entrée,
si bien qu'attendre la fin de la plus longue rétention que vous avez
déclarée est l'option passive.

Donc la procédure, dans l'ordre :

1. **Exécutez `render-cache:epoch-advance` une fois**, avant que le
   déploiement restauré ne serve. Elle échoue bruyamment plutôt que de
   rapporter un succès quand le singleton d'epoch est absent, ce qui est
   aussi la façon dont vous découvrez que la migration n'est pas revenue
   avec les données.
2. **Videz le palier L1 partagé.** Supprimez le contenu du répertoire du
   palier fichier, `DELETE FROM suprnova_render_entries`, ou supprimez les
   clés Redis correspondant à `<prefix>entry:*` - selon le palier que le
   profil configure. Faites-le plutôt que d'attendre un balayage, pour la
   raison ci-dessus.
3. **Couvrez le L0 de chaque nœud, trafic toujours coupé.** L'avancement n'a
   vidé que le nœud qui l'a exécuté, si bien que tant que cette étape n'est
   pas faite un frère non couvert peut encore servir une fois une entrée
   d'avant la restauration - c'est pourquoi le trafic reste coupé jusqu'ici,
   et pas seulement jusqu'à l'étape 2. Soit redémarrez les autres nœuds - un
   processus neuf a un L0 vide et aucun epoch loué, donc sa première requête
   lit l'autorité restaurée - soit exécutez `render-cache:epoch-advance` sur
   chacun d'eux, ce qui vide le L0 de chacun au passage. La seconde option
   incrémente l'epoch du registre une fois par nœud, ce qui ne coûte rien :
   l'epoch ne fait ensuite qu'avancer, et chaque nœud finit par lire la
   dernière valeur. Les deux sont sûres ; le redémarrage est le plus simple
   à raisonner, et c'est le seul qui ne demande aucun calcul en mode
   `Lease`.

Les étapes 2 et 3 sont ce qui rend l'étape 1 complète plutôt que partielle.
Sautez-les et, sur une route dotée d'une fenêtre périmée-servable, une
représentation d'avant la restauration peut encore être servie une fois -
correctement marquée `Warning`, et reconstruite juste après, mais servie.

## Le mesurer

RenderCache livre deux benchmarks, et ce sont des **outils à la demande,
jamais des étapes du gate** :

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

Cela exécute le bench du moteur (`render_cache_budget`, les mesures de hit
chaud et d'assemblage composite avec un allocateur compteur), puis le bench
de charge du framework (`render_cache_workloads`, la même route à travers
tout le middleware), puis le test de contrat sur les résultats versionnés.
Les deux sont épinglés à `SUPRNOVA_LIVE_S1_CPUSET`.

Une exécution complète a besoin d'un PostgreSQL jetable (`PG_TEST_URL`) et
d'un Redis jetable (`REDIS_TEST_URL`), parce que le contrat des résultats
versionnés exige les trois profils enregistrés. **Une exécution partielle
doit rediriger les deux fichiers de résultats** avec
`SUPRNOVA_LIVE_BENCH_RESULT` et `SUPRNOVA_LIVE_WORKLOADS_RESULT` sous
`benchmarks/local/` ; sans cela elle écrase les résultats versionnés par un
fichier plus court et échoue ensuite à son propre contrat.

Les chiffres versionnés, tirés de
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` et de
`render-cache-workloads-v1.json` :

| Mesure | Valeur |
|---|---|
| Travail du moteur pour un hit L0 `Complete` frais, p95 | 0,76 microseconde |
| Allocations sur le tas, hit frais | 3 |
| Allocations sur le tas, hit conditionnel `304` | 3 |
| Allocations sur le tas, hit borné par une échéance de graine | 4 |
| Copies de corps sur l'un quelconque d'entre eux | aucune ; le tampon est partagé |
| La même route à travers le middleware, côté serveur, p95 | 14,4 microsecondes |
| La même requête sur un aller-retour HTTP en loopback, p95 | 109 microsecondes |
| Instructions SQL par hit chaud (mode bail) | 0 |

Les chiffres du middleware valent pour un corps de 65 536 octets dont le
rendu a lu 12 lignes, enregistrées comme 14 identités de dépendance
observées.

**Lisez-les comme exploratoires, non comme des preuves qualifiées.** Chaque
résultat versionné porte `"classification": "local_exploratory"` et
`"s1_requirements_met": false` : ils ont été produits sur une station de
travail de développement avec un CPU partagé, un gouverneur `powersave`, et
des fournisseurs en loopback. Ils sont utiles pour attraper une régression
d'un ordre de grandeur entier et pour rien de plus fin. Un chiffre n'est une
preuve qualifiée que lorsqu'il a été produit sur le runner dédié avec son
attestation posée, et ceux-ci ne l'ont pas été.

### Pourquoi Suprnova diverge

Les paquets de mise en cache de réponses de Laravel laissent l'exploitation
au magasin de cache sous-jacent. Inspecter une entrée revient à trouver sa
clé à la main et à lire la valeur - qui est la page rendue, si bien que la
regarder revient à imprimer le HTML de quelqu'un dans un terminal - et tout
invalider revient à vider un magasin qui détient aussi vos sessions, vos
limitations de débit, et votre file d'attente. L'observabilité est ce que le
pilote du magasin émet, quel qu'il soit.

Suprnova donne au cache sa propre surface d'exploitation, délibérément
étroite. L'inspection est sans corps par construction, si bien qu'un
opérateur peut confirmer qu'une entrée existe, sous quelle classe elle est
stockée, et quelle taille elle fait, sans qu'on lui montre jamais son
contenu. L'invalidation est un incrément d'epoch qui ne coûte rien à
appliquer et ne touche que ce cache - vos sessions et votre file d'attente
ne sont pas dans le périmètre d'impact. La télémétrie est un ensemble fermé
de six compteurs à ensembles d'attributs fermés, et c'est ce qui rend un
tableau de bord bâti dessus stable d'une version à l'autre plutôt qu'un jeu
de chaînes qui dérivent. Le marché, c'est qu'il n'y a aucune commande
« supprime cette clé-ci » : les leviers sont en lecture seule par entrée, ou
à l'échelle de l'epoch.

## Suivant

- [RenderCache](render-cache.md) - les déclarations sur lesquelles ces
  commandes opèrent
- [Observabilité](observability.md) - où les compteurs ci-dessus sont
  exportés
- [Tests](testing.md) - les conventions de test environnantes dans
  lesquelles s'inscrivent les motifs ci-dessus
- [Déploiement](deployment.md) - la checklist de production autour d'eux
