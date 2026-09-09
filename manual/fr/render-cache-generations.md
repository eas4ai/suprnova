# Générations de RenderCache

La plupart des caches expirent. RenderCache expire aussi, mais l'expiration
est le filet de sécurité plutôt que le mécanisme. Le mécanisme est une
**génération** : chaque donnée qu'un rendu a lue possède un compteur dans la
base de données, le rendu stocke les compteurs qu'il a vus, et une écriture
fait avancer le compteur de ce qu'elle a changé. Une représentation stockée
est à jour quand les compteurs qu'elle a vus correspondent encore aux
compteurs que la base détient maintenant. Vous n'écrivez aucune règle
d'invalidation pour vos propres données, parce qu'un `model.save()`
ordinaire en est déjà une.

Ce chapitre porte sur cette machinerie vue de l'extérieur : ce dont un rendu
est enregistré comme dépendant, à quel point ces dépendances sont
grossières, ce que le framework ne peut pas voir et ne peut donc pas
invalider, comment la vérification de cohérence se paie sur un hit, quelle
requête reconstruit quand plusieurs veulent la même entrée en même temps, et
ce qu'un visiteur reçoit dans la fenêtre entre « plus à jour » et
« reconstruit ». Chaque affirmation ci-dessous est tenue par un test nommé
ou par une mesure versionnée ; les exemples dogfood sont des routes de
`app/src/live/mod.rs` prouvées par `app/tests/live_render_cache.rs`.

## Ce dont un rendu est enregistré comme dépendant

Pendant qu'un rendu s'exécute, un collecteur à la portée de la requête
enregistre chaque dépendance qu'il peut nommer : une lecture de table, un
enregistrement lu par clé primaire, une classe de requête, une relation, une
identité de configuration, une fonctionnalité, une locale, une route, et une
identité `Broad` toujours présente que chaque représentation observe. Les
lectures via l'ORM et le générateur de requêtes s'enregistrent elles-mêmes ;
vous n'écrivez rien.

`/live/todos` est tout le motif dans un seul handler :

```rust
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView { count: titles.len(), titles })
    }
    .await;
    result.map_err(failed)
}
```

`Todo::all()` enregistre la table `todos`. Rien d'autre dans le handler ni
dans le template ne lit la session, le visiteur connecté, ou une traduction,
et c'est ce qui permet à la route de rester une représentation partagée.

## Une écriture ordinaire est l'invalidation

`an_orm_write_invalidates_the_todos_document_through_generations` parcourt
tout le cycle dans l'application en cours d'exécution :

1. Le premier `GET /live/todos` se rend et publie.
2. Le second est un hit : il n'atteint jamais le handler, ne porte aucun
   `Warning`, et ne planifie rien.
3. Un `POST /todos/random` écrit une ligne - via la propre route de
   l'application, avec la session et le jeton CSRF qu'un navigateur
   enverrait.
4. Le `GET` suivant est servi avec `Warning: 110 - "Response is Stale"` et
   planifie exactement une reconstruction en arrière-plan. Ses cinq minutes
   de fraîcheur ont à peine commencé, donc la génération avancée de la table
   `todos` est la seule chose qui puisse expliquer l'un ou l'autre.
5. Cette reconstruction s'exécute vraiment : le test attend sur le compteur
   de rendus - une barrière d'état, pas une attente temporisée - jusqu'à ce
   qu'un rendu que le test lui-même n'a pas déclenché ait eu lieu.
6. La ligne écrite est vraiment dans le listing. C'est une étape
   **distincte**, et délibérément pas une assertion sur la sortie propre de
   la reconstruction en arrière-plan : le test vide d'abord L0 et rend à
   nouveau, parce que la publication de la reconstruction atterrit à un
   moment que rien d'atteignable depuis l'application ne rend observable, si
   bien qu'asserter sur la requête qui se trouverait l'attraper serait une
   course.
7. Et la route revient à un simple hit contre l'entrée republiée.

Aucune clé de cache n'a été nommée nulle part dans cette séquence. Une
écriture ORM à l'intérieur d'un `DB::transaction` fait avancer ses
générations à l'intérieur de cette même transaction, si bien qu'une écriture
annulée ne fait rien avancer du tout.

## À quel point l'invalidation est étroite

C'est la chose la plus importante à savoir avant de dimensionner une route
mise en cache.

Une lecture ponctuelle par clé primaire qui retourne une ligne enregistre
l'identité de cette **ligne** et l'identité d'**écriture sans clé** de la
table, pas la table elle-même. `Model::find`, `Model::find_or_fail`, et
`Model::find_many` observent chacun une identité d'enregistrement par ligne
hydratée et une identité d'écriture sans clé à côté, si bien qu'une
écriture au niveau de la ligne ailleurs dans la table laisse l'entrée à
jour, tandis qu'un `update_all` ou `delete_all` massif, une écriture par
`DB::table(..)`, ou une instruction brute sur la table l'atteint encore.
Une lecture ponctuelle qui ne retourne aucune ligne observe la table à la
place, parce qu'insérer la ligne manquante est ce qui changerait la
réponse.

Toute autre lecture est à la granularité de la table : `Model::all`, chaque
terminal `Builder`, et chaque chargement de relation enregistrent la table
entière, si bien que toute écriture dans cette table invalide chaque entrée
mise en cache qui y a lu. C'est sûr - cela ne peut qu'invalider trop, jamais
trop peu - et c'est mesuré plutôt que supposé. La charge de tempête
d'invalidation dans `framework/benches/render_cache_workloads.rs` publie 64
clés sur 12 identités d'enregistrement, exécute 1 000 écritures, et
enregistre l'ampleur de propagation observée dans
`crates/suprnova-live/benchmarks/render-cache-workloads-v1.json` (abrégé ;
l'objet enregistré porte aussi les champs de rafale, de balayage, de hit, de
reconstruction, d'instruction et de latence) :

```json
"invalidation_storm": {
  "keys": 64,
  "identities": 12,
  "writes": 1000,
  "every_write_invalidates_every_key": false,
  "point_read_invalidation_ratio": 0.09375,
  "rebuilds_per_write": 1.28,
  "final_bodies_coherent": true
}
```

`every_write_invalidates_every_key` vaut `false` parce qu'une lecture
ponctuelle ne dépend plus de toute sa table ; `point_read_invalidation_ratio`
est le nombre qui l'a remplacée comme celui qu'il vaut la peine de
surveiller.

Concevez en conséquence. Une route mise en cache adossée à une table dans
laquelle votre application écrit sans cesse reconstruira sans cesse, quelle
que soit sa fenêtre de fraîcheur. Une route mise en cache adossée à une
table qui change quand un éditeur publie quelque chose restera immobile
pendant des heures. Si vous avez besoin d'une granularité plus fine que la
table, la réponse honnête aujourd'hui est que vous ne l'avez pas.

## Ce que le framework ne peut pas voir

Une dépendance qui ne peut pas être nommée ne peut pas être invalidée, et le
framework est délibéré sur celles qu'il refuse de stocker et celles qu'il
laisse passer.

**Refusé d'emblée.** Le SQL brut via `DB::select`, `DB::select_one`,
`DB::scalar`, ou `DB::select_on` ne peut pas nommer les tables que son
instruction a lues, donc le rendu est marqué inobservable et n'est jamais
stocké. La réponse est tout de même servie, correctement, à chaque fois.
Les propres vérifications de rôle et de permission RBAC du framework
nomment les cinq tables qu'elles lisent - `roles`, `permissions`,
`role_permissions`, `model_roles`, et `model_permissions` - donc une route
mise en cache qui en évalue une est observée avec précision et mise en
cache normalement.
Les lectures via `DB::table(..)` connaissent leur table et se mettent en
cache normalement.

**Invisible, et de votre responsabilité.** Un en-tête de requête lu via
`Request::header` et un appel à `Config::get` changent tous deux ce qu'un
rendu produit sans que le collecteur ne voie quoi que ce soit. Déclarez la
dimension de variance correspondante sur une telle route ; rien ici ne peut
rattraper cet oubli à votre place.

**Portées globales.** Une portée globale Eloquent déclare ce dont dépend
son filtre. Une `GlobalScope` qui retourne `ScopeDependency::Constant`
n'enregistre rien et ne coûte aucun succès de cache. La valeur par défaut,
`ScopeDependency::PerRequest`, exige que le `apply` de la portée lise cet
état via un accesseur instrumenté - `suprnova::live::current_tenant()`,
`Auth::id()`, `Lang::locale()`. Une portée propre à la requête dont
l'évaluation n'en lit aucun restreint le rendu à `Uncacheable` et se nomme
elle-même dans le refus, si bien qu'un filtre de tenant invisible vous
coûte le cache plutôt que de coûter à vos visiteurs les lignes les uns des
autres.

**Flags de fonctionnalité.** Une lecture d'un flag que la table `features`
contient - à n'importe quelle clé de portée, la valeur par défaut globale
incluse - observe la génération propre de ce flag.
`DatabaseEvaluator::set_flag` la fait avancer après que la nouvelle valeur
est visible pour les lecteurs, et `DatabaseEvaluator::reload()` la fait
avancer pour chaque flag dont les règles stockées ont changé, et indique à
l'évaluateur mis en cache lesquels c'était. Un flag que la table ne
contient pas n'enregistre rien : ce rendu dépendait de la valeur par défaut
compilée dans `is_enabled!`, pas d'un état stocké.

**Le côté écriture.** Tout processus dont la configuration active
RenderCache et dont la base de données porte la migration RenderCache fait
avancer les générations, si bien qu'une écriture faite par un worker de
file d'attente, une tâche planifiée, ou une commande console invalide
exactement ce que la même écriture invalide dans le serveur, et
`RenderCache::bump_permission_version()` fonctionne depuis n'importe lequel
d'entre eux. Un processus avec `RENDER_CACHE_ENABLED=false`, ou dont la
base de données ne porte pas la migration, ne fait rien avancer et n'émet
aucun SQL de RenderCache. Voir
[Exploitation de RenderCache](render-cache-operations.md).

## Ce que coûte un hit

La vérification de cohérence est ce qui transforme « nous avons des octets »
en « ces octets sont à jour », et c'est le seul travail que fait un hit.

Un hit n'exécute **aucun handler, aucune requête ORM, aucun template, et
aucun sérialiseur**, et ne copie aucun octet de corps : les octets que le
serveur écrit sur la socket sont les octets que le magasin détient, prouvé
par adresse plutôt que par valeur dans
`framework/tests/render_cache/bypass.rs`. Ce qui reste est la lecture en
base qui prouve l'actualité, et la fréquence à laquelle vous la payez est le
`CoherenceMode` de la politique :

| Mode | Instructions SQL par hit chaud | Ce à quoi il se fie |
|---|---|---|
| `Authority` (par défaut) | exactement 1 | au registre, relu à chaque hit |
| `Lease { max_age_ms }` | 0 | à un bail de validation accordé localement, jusqu'à son expiration |

`an_authority_mode_hit_issues_exactly_one_statement` tient le mode autorité
à un seul aller-retour : les générations observées et l'epoch d'autorité
sont lus ensemble dans un unique `UNION ALL`, pas en deux lectures.
`a_lease_mode_hit_runs_nothing_and_issues_no_statement` tient le mode bail à
zéro, parce que l'epoch sous lequel la clé a été dérivée est loué en même
temps que les générations plutôt que lu à chaque requête.

L'epoch lui-même est lu une fois par processus, pas une fois par requête.
`the_epoch_is_read_once_at_first_use` mesure le premier miss d'un runtime
neuf contre un second par ailleurs identique et trouve que le premier paie
exactement une instruction de plus - l'unique lecture d'autorité qui remplit
le bail d'epoch. Chaque requête suivante ne paie rien pour cela.

## Quand l'epoch bouge

`render-cache:epoch-advance` est l'invalidation d'urgence, et l'epoch est
intégré à chaque clé de recherche, donc ce qui se passe ensuite dépend de
l'endroit où vous vous tenez :

- **Sur le nœud qui a exécuté la commande**, la toute prochaine requête voit
  le nouvel epoch. L0 est vidé purement et simplement au même instant, et le
  cache est invalidé immédiatement.
- **Sur un autre nœud**, une route en mode `Authority` l'apprend à son tout
  prochain hit. Une route en mode `Lease` l'apprend à sa prochaine relecture
  d'autorité, soit au plus `max_age_ms` plus tard.

Une route dotée d'une fenêtre périmée-servable sert l'entrée déplacée une
fois sous `Warning` pendant que la reconstruction s'exécute derrière la
requête ; une route sans cette fenêtre reconstruit au premier plan et le
demandeur attend. Cette différence est toute la raison de déclarer une
fenêtre périmée-servable, et elle s'applique à tout déplacement, pas
seulement à un avancement d'epoch.

Trois tests dans `framework/tests/render_cache/middleware.rs` tiennent ces
chemins par leur nom :
`an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`,
`an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`,
et
`an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.

## Une reconstruction par clé : singleflight et requêtes en attente

Quand une entrée est absente ou n'est plus à jour, les requêtes qui arrivent
pour elle ne se rendent pas toutes. Elles sont admises par un
**coordinateur de reconstruction**, qui en choisit exactement une :

- Le **leader** est la seule requête qui rend et qui peut publier. Il
  détient un bail sur cette clé pendant toute la durée de son rendu.
- Les **requêtes en attente** sont celles qui arrivent pour la même clé
  pendant que le leader détient le bail. Elles attendent dans le processus,
  et quand le leader relâche, elles réévaluent ce qui est désormais stocké
  et le servent. Une requête en attente ne se fie jamais à l'attente : si le
  cycle du leader n'a pas réussi à publier, ou a publié quelque chose que sa
  propre vérification de fraîcheur trouve mort, elle rend elle aussi plutôt
  que de servir ce qu'elle a trouvé.
  `a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh` dans
  `framework/tests/render_cache/middleware.rs` est cette règle.
- Une requête qui arrive alors que `RENDER_CACHE_MAX_WAITERS` (128 par
  défaut) attendent déjà **contourne** : elle rend et ne publie rien, plutôt
  que de faire grossir une file non bornée.

`concurrent_misses_render_once_and_waiters_reuse_the_publication` prouve le
cas ordinaire de bout en bout - deux miss concurrents, un rendu, des corps
identiques - et `one_leader_per_key_and_fence_with_bounded_waiters` dans
`crates/suprnova-live/tests/render_cache_singleflight.rs` prouve le plafond
directement contre le coordinateur : au-delà de sa limite d'attente,
l'admission répond `Bypass`.

Deux publications pour une même clé ne peuvent jamais être acceptées toutes
les deux, quoi qu'ait décidé le coordinateur. Un leader forge un jeton de
publication sous son bail, et le magasin compare cette barrière avant
d'écrire : un epoch plus ancien, ou un epoch égal avec un jeton inférieur,
perd. C'est ce qui rend le *rendu* dupliqué sûr à accepter alors que la
*publication* dupliquée ne l'est pas, et c'est pourquoi il n'y a aucune
attente inter-nœuds : une clé qu'un autre nœud reconstruit est ici un
contournement. Voir
[Déploiement de RenderCache](render-cache-deployment.md).

## Servir quelque chose pendant la reconstruction

Les quatre états de fraîcheur, les bandes que `FreshnessPolicy` règle, et
les `Warning` et `Age` que porte une réponse périmée sont définis dans
[Représentations de RenderCache](render-cache-representations.md). Ce qui
compte ici, c'est qu'un déplacement de génération fait entrer une entrée
dans ces bandes plus tôt : une entrée déplacée est évaluée à un âge effectif
d'**au moins** son intervalle de fraîcheur, quel que soit son âge réel. Son
âge réel décide encore de la bande dans laquelle cela la place :

- Âge réel inférieur à `fresh_ms + stale_servable_ms`, sur une route qui
  déclare une fenêtre périmée-servable : périmée-servable. La copie stockée
  est servie une fois sous `Warning` et la reconstruction s'exécute derrière
  la requête. C'est l'étape 4 du test d'écriture ci-dessus, sur une entrée
  dont les cinq minutes de fraîcheur avaient à peine commencé.
- Âge réel au-delà, mais pas encore au bord de mort : périmée-sur-erreur. La
  requête attend une reconstruction au premier plan et ne voit la copie
  stockée que si cette reconstruction échoue.
- Sur une route sans aucune fenêtre périmée-servable, et sur chaque route
  `PrivateCached` (dont le bord de mort *est* le bord de fraîcheur), un
  déplacement est Mort : la requête reconstruit au premier plan et attend.

`stale_service_is_marked_and_rebuilt_in_the_background` montre le même
passage de relais mené par l'horloge plutôt que par une écriture : au-delà
des 300 000 millisecondes de fraîcheur de `/live/todos` et à l'intérieur de
ses 60 000 millisecondes périmées-servables, le visiteur reçoit la copie
disponible sous `Warning: 110 - "Response is Stale"` et `Age: 300`,
exactement une reconstruction est planifiée, et cette reconstruction
s'exécute vraiment.

Le repli périmé-sur-erreur couvre la requête qui mène une reconstruction
**et** une requête en attente derrière un leader dont la reconstruction a
échoué. Les deux reçoivent la même réponse : les octets périmés sous
`Warning`, plutôt que l'échec. `framework/tests/render_cache/races.rs`
prouve chaque branche séparément -
`a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on`
et `a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it` -
et la seconde par retour arrière : retirer le repli de la branche en attente
fait passer ses assertions finales de `200` à `500`.

Les routes cousues sont l'exception, et elle est délibérée. Une entrée
`Composite` n'est jamais servie par le repli périmé-sur-erreur et ne
déclenche jamais de reconstruction en arrière-plan : servir une coque
stockée après une reconstruction échouée répondrait à une requête que la
propre chaîne d'autorisation de la route n'a jamais eu l'occasion de
filtrer, et une reconstruction en arrière-plan ne porte aucun état
d'autorisation de la requête, si bien que sa coque serait ce que la page
rend pour personne. Sur une route cousue, ce que le client voit est le
résultat propre de la reconstruction échouée.

## Les routes mises en cache sont des chemins de lecture

Le rendu du leader s'exécute à l'intérieur d'une transaction de base de
données, ouverte en `REPEATABLE READ` sur PostgreSQL et MySQL, si bien que
les générations qu'il enregistre et les données qu'il a lues partagent un
seul instantané. Deux conséquences en découlent.

Le handler d'une route mise en cache qui **écrit** entre en concurrence avec
des écrivains simultanés pour les mêmes lignes, et sur PostgreSQL un handler
qui met à jour une ligne qu'une autre transaction a modifiée après le début
du rendu voit un échec de sérialisation. Concevez les routes mises en cache
comme des chemins de lecture.

Une écriture faite en dehors de toute transaction - `model.save()` seul -
commite d'abord sa ligne et fait avancer ses générations dans une
transaction qui suit immédiatement. L'instant entre les deux est
« nouvelles données, ancienne génération » : il coûte une reconstruction
supplémentaire et ne sert jamais de contenu périmé.

Enfin, une fois le rendu terminé, les dépendances observées et l'epoch sont
relus, en dehors de la vue transactionnelle propre au rendu. Tout ce qui a
bougé pendant le rendu écarte le candidat au lieu de le publier. C'est
pourquoi une écriture qui atterrit en plein rendu coûte une reconstruction
au lieu d'une page fausse.

### Pourquoi Suprnova diverge

Le cache de Laravel est un magasin clé-valeur et ses paquets de mise en
cache de réponses sont bâtis dessus, si bien que l'invalidation est quelque
chose que vous écrivez. Vous appelez `Cache::forget`, ou vous étiquetez les
entrées et vous videz une étiquette, ou vous enregistrez un observateur de
modèle qui efface les clés dont vous croyez que ce modèle les alimente.
Chacun de ces gestes est une correspondance que vous maintenez à la main, et
le mode de défaillance est silencieux : la page que personne n'a pensé à
oublier continue d'être servie jusqu'à l'expiration de son TTL.

Suprnova inverse le sens. Le rendu enregistre ce qu'il a lu, l'écriture fait
avancer ce qu'elle a changé, et les deux se rencontrent dans un registre en
base de données plutôt que dans votre tête. Il n'y a aucun appel à `forget`
à oublier. Le prix est que la dépendance enregistrée est une table plutôt
qu'une ligne, si bien qu'une table active fait reconstruire souvent ce qui
en dépend, et que les lectures via du SQL brut sont refusées au stockage
plutôt que mises en cache avec une dépendance que personne ne peut nommer.
Ces deux points sont visibles et mesurés - l'ampleur de propagation dans la
charge de tempête versionnée, le refus dans votre propre en-tête `Age`
absent - plutôt qu'une page périmée dont un client vous apprend l'existence.

## Suivant

- [Déploiement de RenderCache](render-cache-deployment.md) - les profils,
  les fournisseurs, et la migration qui rend la vérité des générations
  durable
- [Représentations de RenderCache](render-cache-representations.md) - ce qui
  est réellement stocké, et sous quelle clé
- [Base de données](database.md) - les transactions et l'isolation, à
  l'intérieur desquelles s'exécutent les rendus mis en cache
