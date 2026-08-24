# Collections Eloquent

`Collection<T>` est le type de collection en forme Laravel de
Suprnova - la valeur de retour de `Builder::get`, `Model::all`, chaque
`pluck`, chaque terminal de chargement de relation qui produit plus
d'une ligne. C'est un mince wrapper autour de `Vec<T>` qui se
déréférence en `&[T]`, si bien que chaque méthode de slice existante
(`.len()`, `.iter()`, l'indexation, `.contains(&v)`) fonctionne sans
changement. Par-dessus vient la surface Laravel : `map`, `filter`,
`pluck`, `group_by`, `sort_by`, `where_eq`, `sum`, `avg`, et le reste.

Ce chapitre est la référence autonome de la surface collection. Le
chapitre parent [Eloquent API](eloquent.md) la résume ; ce chapitre
parcourt chaque méthode, le contrat emprunter-vs-consommer, la règle
de sérialisation qui mord si vous la sautez, et quand redescendre vers
`Vec<T>` à la place.

## Table des matières

- [D'où viennent les collections](#d-où-viennent-les-collections)
- [Les deux blocs impl](#les-deux-blocs-impl)
- [Surface générique - fonctionne sur tout `Collection<T>`](#surface-générique-fonctionne-sur-tout-collection-t)
- [Surface consciente du modèle - `Collection<M>` où `M: Model`](#surface-consciente-du-modèle-collection-m-où-m-model)
- [Chargement hâtif sur une collection](#chargement-hâtif-sur-une-collection)
- [Sérialisation - `to_array` vs serde](#sérialisation-to-array-vs-serde)
- [Emprunter vs consommer](#emprunter-vs-consommer)
- [Collection vs `Vec`](#collection-vs-vec)
- [`LazyCollection<M>` - résultats en flux](#lazycollection-m-résultats-en-flux)
- [Pourquoi Suprnova diverge](#pourquoi-suprnova-diverge)
- [Suivant](#suivant)

## D'où viennent les collections

Tout terminal qui retourne plus d'une ligne vous remet une
`Collection<M>` :

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

Vous pouvez aussi envelopper n'importe quel `Vec<T>` que vous avez
déjà :

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` implémente `Default`, `Clone`, `Serialize`,
`Deserialize`, `PartialEq`, et `IntoIterator` (à la fois par valeur et
par `&`). Il est `Send` quand `T: Send`.

## Les deux blocs impl

Les méthodes de `Collection` se divisent en deux familles selon le
paramètre de type.

```rust
impl<T> Collection<T> { /* méthodes génériques - fonctionnent pour tout T */ }

impl<M> Collection<M> where M: Model { /* méthodes de modèle à clé de chaîne */ }
```

Le bloc générique vous donne `map`, `filter`, `reject`, `chunk`,
`first`, `last`, `unique`, et une version à base de closure de chaque
accesseur de colonne (`pluck_by`, `group_by_with`, `sort_with`,
`key_by_with`). Ceux-ci fonctionnent sur `Collection<i32>`,
`Collection<String>`, `Collection<MyDto>`, n'importe quoi.

Le bloc conscient du modèle ajoute du sucre à clé de chaîne
(`pluck("name")`, `group_by("role")`, `sort_by("created_at")`,
`sum::<f64>("balance")`) qui route chaque ligne à travers l'accesseur
`Model::field_value` émis par la macro. Ceux-ci n'existent que quand
`T` implémente `Model`.

Choisissez la forme par closure quand vous le pouvez - le vérificateur
de types valide l'accès au champ. Choisissez la forme à clé de chaîne
quand vous voulez correspondre à la syntaxe de Laravel, ou quand le
nom de colonne est une valeur d'exécution.

## Surface générique - fonctionne sur tout `Collection<T>`

### Lecture

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - via Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` prennent `&&T` parce que le prédicat
s'exécute via `Iterator::find` sur `Iter<'_, T>`. Déréférencez deux
fois (`**n`).

### Transformer - consomme `self`, retourne une nouvelle collection

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` change le type d'élément :

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` exécute un effet de bord et garde la collection pour un
chaînage ultérieur (Suprnova diverge délibérément de Laravel ici -
voir plus bas) :

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### Regroupement et tri par clé de closure

```rust
use std::collections::HashMap;

// Répartit les éléments par clé dérivée d'une closure.
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// Indexe les éléments par clé dérivée d'une closure (les doublons ultérieurs écrasent).
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// Trie par comparateur dérivé d'une closure.
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// Déduplique par clé dérivée d'une closure.
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// Projette chaque élément par closure dans une nouvelle collection.
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

Le suffixe `*_with` / `*_by` est la convention de nommage universelle
« cette méthode prend une closure » à travers le bloc générique. Le
bloc conscient du modèle abandonne le suffixe et prend à la place une
chaîne de nom de colonne.

### Réduction et agrégation

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

Pour des agrégats numériques typés sur des collections de modèles,
voir `sum` / `avg` / `min` / `max` dans la section consciente du
modèle - ils fonctionnent sur tout champ qui se désérialise en un
type numérique.

### Opérations d'ensemble

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // alias de concat
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` sont des alias - Laravel livre les deux noms.
`diff` / `intersect` sont en O(n*m) ; si vous avez de grandes
collections, projetez d'abord vers un `HashSet`.

### Échantillonnage aléatoire

```rust
let one: Option<&i32>     = nums.random();        // emprunte un
let many: Collection<i32> = nums.clone().random_n(3); // en choisit 3
```

Les deux utilisent le RNG thread-local (`rand::rng()`). Passez
vous-même un RNG à graine fixe si vous avez besoin de déterminisme
dans les tests.

## Surface consciente du modèle - `Collection<M>` où `M: Model`

Ces méthodes n'existent que quand le type contenu est un modèle
Suprnova. Elles routent les lectures ligne par ligne à travers
l'accesseur `Model::field_value(name)` émis par la macro, qui retourne
`Option<serde_json::Value>`. Les lignes dont le champ n'existe pas ou
ne se désérialise pas dans le type cible sont silencieusement
ignorées - à l'image du comportement de Laravel pour une clé
manquante.

### Projection

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` emprunte (`&self`), si bien que la collection d'origine reste
disponible ensuite. Le paramètre typé (`::<String>`) est le type cible
dans lequel la valeur JSON est désérialisée.

`pluck_keyed` produit un `HashMap<K, V>` à partir de deux colonnes :

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

Les lignes suivantes écrasent les précédentes pour la même clé.

`model_keys` est le raccourci de clé primaire, et la seule projection qui
retourne un simple `Vec` plutôt qu'une `Collection` :

```rust
let users: Collection<User> = User::query().get().await?;
let ids: Vec<i64> = users.model_keys();
```

Il lit le champ de clé déjà hydraté, il ne coûte donc aucune requête. Si
vous ne voulez que les clés et n'avez pas encore chargé les lignes, utilisez
plutôt le terminal du builder  -
`User::query().model_keys().await?` projette la colonne de clé sans rien
hydrater. `Vec` plutôt que `Collection` correspond à `modelKeys()` de
Laravel et garde les deux moitiés de la paire cohérentes sur une même forme.

### Regroupement et indexation

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

Les deux méthodes transforment la valeur de colonne en chaîne pour la
clé `String`. Une colonne `id` numérique arrive sous la forme `"1"` /
`"2"` - à l'image du contrat de `groupBy('team_id')` de Laravel, où la
sortie est toujours à clé de chaîne quel que soit le type sous-jacent.

Si vous voulez des clés typées, utilisez la forme par closure du bloc
générique :

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### Filtrage

Les méthodes `where_*` conscientes du modèle prennent un
`serde_json::Value` parce qu'elles comparent contre la forme encodée
en JSON de la colonne :

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` et `where_in` retirent les lignes dont `field_value`
retourne `None`. `where_not_in` *garde* les lignes où le champ est
absent - la négation de « dans l'ensemble » est « pas dans l'ensemble
OU absent ».

### Tri

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

La comparaison est du mieux-effort à travers les formes de valeur
JSON : numérique vs numérique et chaîne vs chaîne se trient proprement
au sein de leur genre ; les colonnes hétérogènes mixtes retombent sur
`Ordering::Equal`. `None` se trie avant toute valeur présente (reflète
`NULL FIRST` de Postgres pour ASC).

Les deux méthodes clonent le `Vec<M>` sous-jacent avant de trier parce
que le comparateur emprunte `m.field_value(field)` alors que `sort_by`
a besoin de `&mut [M]`. Si vous avez une boucle serrée, triez plutôt
avec `sort_with` sur le bloc générique - il opère en place.

### Agrégats

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` retourne `T::default()` quand aucune ligne ne contribue de
valeur (zéro pour les types numériques). Les trois autres retournent
`None` pour que l'appelant ne divise pas par zéro ni ne compare contre
un défaut fantôme.

Le paramètre typé (`::<f64>`) est la cible de désérialisation JSON.
Choisissez le type numérique le plus large que votre colonne utilise
raisonnablement - `i64` pour les colonnes entières, `f64` pour le
décimal/flottant, `chrono::DateTime<Utc>` pour les horodatages, etc.

## Chargement hâtif sur une collection

Quand vous avez déjà une `Collection<M>` et voulez charger des
relations sur chaque ligne, utilisez `load` / `load_missing` :

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

Les deux méthodes prennent `&mut self` (elles mutent le cache hâtif
par ligne) et sont `async`. Les deux acceptent la même syntaxe de
chemin à points que `Builder::with([...])` accepte - `"posts"`,
`"posts.comments"`, `"posts.comments.author"`.

`load_missing` partitionne ligne par ligne. Les lignes qui ont déjà la
relation en cache sont laissées telles quelles ; celles qui ne l'ont
pas reçoivent le chargement en masse :

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// Certaines lignes ont déjà des posts en cache. load_missing ne touche
// que le reste - et récurse dans les posts déjà en cache pour `comments`.
users.load_missing(["posts.comments"]).await?;
```

La récursion s'exécute à chaque segment d'un chemin à points plus
long. Avec `"a.b.c"`, chaque ligne est partitionnée à chaque niveau :
`a` n'est chargé que là où il manque, puis pour les lignes qui avaient
déjà `a`, `b` n'est chargé que là où il manque sur ces `a`, etc.

Les deux méthodes respectent le routage
`#[model(connection = "...")]` - elles résolvent la même connexion
depuis laquelle la ligne a été chargée à l'origine.

## Sérialisation - `to_array` vs serde

C'est le seul piège de la surface collection. Lisez-le attentivement.

`Collection<T>` dérive `Serialize`. Donc ceci fonctionne :

```rust
let json: String = serde_json::to_string(&users)?;
```

Mais - l'implémentation générique de serde `Serialize for Vec<T>`
appelle directement `T::serialize` sur chaque élément. Cela
**contourne** la redéfinition de `Model::to_array()` que la macro
`#[suprnova::model]` émet. Ce qui veut dire que cela contourne vos
attributs de modèle `hidden = ["password"]`, `visible = [...]`, et
`appends = [...]`.

Si votre modèle a des champs `hidden`, **ne** sérialisez **pas** la
collection via serde. Utilisez `to_array()` ou `to_json()` :

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

Les deux méthodes routent à travers `Model::to_array()` pour chaque
ligne, si bien que le pipeline de filtres par modèle s'applique - les
champs `hidden` restent masqués, les listes blanches `visible` sont
imposées, et les `appends` pilotés par accesseur apparaissent.

La même mise en garde s'applique à tout ce qui appelle
`serde_json::to_value(&collection)` en coulisse : `Inertia::render`
quand vous fourrez une collection dans des props, `JsonApi`/`Resource`
si vous leur passez des modèles bruts au lieu de structs ressource,
les expéditeurs de logs qui encodent leurs charges utiles via serde.
Le motif sûr est de convertir via un type ressource
([Ressources JSON:API](eloquent-resources.md)) ou via `to_array()`
avant que la valeur n'atteigne un quelconque chemin de code serde.

Pour des collections de types non-modèles (`Collection<MyDto>`,
`Collection<String>`), le chemin serde est sans problème - le souci ne
s'applique que quand `T` est une struct `#[suprnova::model]` avec des
`hidden`/`visible`/`appends` déclarés.

## Emprunter vs consommer

Les méthodes se divisent nettement en deux contrats :

| Prend | Méthodes |
|---|---|
| `&self` (emprunt) | `len`, `is_empty`, `is_not_empty`, `first`, `last`, `first_where`, `last_where`, `contains_where`, `random`, `as_slice`, `pluck_by`, `pluck`, `pluck_keyed`, `group_by`, `key_by`, `sum`, `avg`, `min`, `max`, `to_array`, `to_json` |
| `self` (consommation) | `map`, `filter`, `reject`, `each`, `reduce`, `chunk`, `take`, `skip`, `slice`, `reverse`, `shuffle`, `random_n`, `unique`, `unique_by`, `sort_with`, `sort_by`, `sort_by_desc`, `where_eq`, `where_in`, `where_not_in`, `concat`, `merge`, `diff`, `intersect`, `group_by_with`, `key_by_with`, `map_to_map` |
| `&mut self` | `load`, `load_missing` |

Si vous voulez garder la collection après un appel consommateur,
faites `.clone()` avant l'appel. `Collection<T>: Clone` quand
`T: Clone`.

Un motif pratique : lire d'abord, transformer en dernier :

```rust
let users: Collection<User> = User::all().await?;

// Les lectures par emprunt d'abord - la collection reste vivante après chacune.
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// Maintenant on consomme.
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## Collection vs `Vec`

Le wrapper est délibérément mince. Les chemins de conversion
fonctionnent dans les deux sens et restent peu coûteux :

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` vous donne automatiquement chaque méthode de
slice. Cela inclut :

```rust
let users: Collection<User> = User::all().await?;

users.len();             // méthode de slice
users.iter();            // méthode de slice
users[0].name.clone();   // indexation de slice
users.contains(&u);      // méthode de slice
users.binary_search(&u); // méthode de slice
&users[1..4];            // indexation par tranche
```

`IntoIterator` est implémenté deux fois - pour `Collection<T>` (par
valeur) et `&Collection<T>` (par référence), si bien que les deux
fonctionnent :

```rust
for user in &users {           // itère par &User
    /* ... */
}

for user in users.clone() {    // itère par User (consomme)
    /* ... */
}
```

`DerefMut` ne produit que `&mut [T]` - une slice, pas un `Vec`. Cela
veut dire que la mutation en place des champs d'élément fonctionne :

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

Mais la mutation de `Vec` possédé (`push`, `pop`, `clear`,
`truncate`) n'est pas disponible directement sur la collection -
appelez `into_vec()` d'abord :

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

C'est délibéré. La surface Laravel traite une collection comme un
instantané immuable que vous transformez avec des méthodes chaînées ;
la mutation possédée de la séquence interne est le contrat de `Vec`,
pas le contrat de `Collection`.

### Quand redescendre vers `Vec`

Tournez-vous vers `into_vec()` quand :

- Vous avez besoin de méthodes spécifiques à `Vec` (`push`, `pop`,
  `swap_remove`, `drain`, `with_capacity`).
- Vous transmettez les données à une API qui prend un `Vec<T>` par
  valeur et vous ne voulez pas du wrapper dans la signature.
- Vous stockez les lignes sur le long terme dans votre propre struct
  et la surface Laravel ne vous apporte rien.

Pour tout le reste - les retours de handler, les transformations, les
props Inertia (aussi longtemps que vous respectez la
[règle de sérialisation](#sérialisation-to-array-vs-serde)) - gardez
le `Collection<T>`.

## `LazyCollection<M>` - résultats en flux

`Collection<M>` matérialise chaque ligne en mémoire. Pour des jeux de
données trop grands pour y tenir, le builder offre trois terminaux en
flux qui retournent plutôt `LazyCollection<M>` :

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| Méthode | Stratégie |
|---|---|
| `Builder::lazy()` | Pagination par curseur de clé primaire avec la taille de lot par défaut (1000) |
| `Builder::lazy_by_id(n)` | Pagination par curseur de clé primaire avec une taille de lot `n` |
| `Builder::cursor()` | Alias Laravel pour `lazy()` |

`LazyCollection<M>` est en coulisse un
`Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>`, mais
expose `.next().await` directement pour que vous n'ayez pas besoin
d'importer `futures::StreamExt`. Chaque `.next()` déclenche la
livraison de la ligne suivante ; la récupération par lot sous-jacente
ne s'exécute que quand le buffer du lot en cours se vide, si bien
qu'un consommateur lent n'accumule pas de lignes.

Le wrapper est `Send` (il traverse donc `tokio::spawn`) mais pas
`Sync` - c'est un flux à consommateur unique par construction.

Voir [Eloquent - itération par chunk et en mode lazy](eloquent.md#chunking-and-lazy-iteration)
pour le guide complet sur le motif de flux à choisir.

## Pourquoi Suprnova diverge

L'`Illuminate\Support\Collection` de Laravel est mutable :
`$c->filter(...)` modifie le tableau interne du même objet et retourne
`$this` pour le chaînage. PHP n'a pas de notion de possession, si bien
que ce contrat est invisible.

Rust a une notion de possession, et prétendre le contraire rendrait la
surface collection malhonnête. Suprnova choisit à la place la forme à
sémantique de valeur : chaque transformation consomme `self` et
retourne une nouvelle `Collection`. Vous voyez le coût dans votre
propre code - si vous voulez garder l'original, vous faites
`.clone()`. Si vous ne le faites pas, vous ne le faites pas.

Ce choix se répercute sur le reste de la surface :

- **`each` retourne `Self`** plutôt que `&self` pour qu'un appel à
  effet de bord (journalisation, métriques) ne casse pas une chaîne.
  Le `each` de PHP s'exécute pour son effet et retourne la collection ;
  vous ne pourriez pas faire `$c->each(...)->filter(...)` proprement
  sans re-récupérer. En Rust, nous faisons transiter `self`, ce qui
  garde la chaîne fluide.

- **Des équivalents par closure pour chaque méthode à clé de
  chaîne.** `pluck_by`, `group_by_with`, `key_by_with`, `sort_with`,
  `unique_by`, `map_to_map`, `contains_where`. Les closures vous
  laissent lire des champs que le vérificateur de types valide, au
  lieu de chaînes que le compilateur ne peut pas voir. Les formes à
  clé de chaîne existent pour la parité avec la syntaxe Laravel et
  pour les noms de colonne décidés à l'exécution.

- **`sum` / `avg` / `min` / `max` prennent des paramètres typés
  `::<T>`.** La version PHP de Laravel convertit le type à la volée ;
  en Rust, la cible de désérialisation fait partie de l'appel. Les
  lignes dont la valeur ne fait pas l'aller-retour vers `T` sont
  silencieusement ignorées (à l'image du comportement de Laravel pour
  une clé manquante), mais vous choisissez le type intentionnellement.

- **`Deref<Target = [T]>`, pas `Deref<Target = Vec<T>>`.** Une
  `Collection` est conceptuellement un « instantané de lignes », pas
  un buffer mutable. Les méthodes de slice arrivent via `Deref` ; si
  vous voulez `push`/`pop`, `into_vec()` vous donne le `Vec` brut et
  supprime toute prétention.

- **La sérialisation diverge au service de la correction.**
  `to_array` et `to_json` routent à travers `Model::to_array()` pour
  que les `hidden`/`visible`/`appends` par modèle s'appliquent ; le
  contournement par le `Serialize for Vec` générique de serde est
  documenté comme le [piège](#sérialisation-to-array-vs-serde) qu'il
  est. Le `toArray()` de Laravel fait le même routage ; nous devons
  simplement nommer l'écart explicitement parce que les utilisateurs
  Rust se tourneront vers `serde_json::to_string` par réflexe.

Le compromis est exactement celui que Suprnova fait partout : la
forme de surface de Laravel, la sémantique de valeur de Rust.

## Suivant

- [Eloquent API](eloquent.md) - le chapitre parent, avec le query
  builder, les relations, les scopes, et le cycle de vie complet du
  modèle.
- [Ressources JSON:API](eloquent-resources.md) - les structs ressource
  sérialisent les collections via `IntoJsonResource` avec des jeux de
  champs partiels et des chaînes `?include=` ; la bonne forme pour
  toute collection qui quitte votre API.
- [Frontend - Réponses Inertia](frontend-inertia-responses.md) - les
  règles pour remettre des collections à des props Inertia sans
  déclencher le piège de sérialisation.
- [Validation](validation.md) - les payloads de requête produisent
  souvent des vecteurs que vous enveloppez dans une `Collection` pour
  un traitement en aval.
- [Tests](testing.md) - des motifs pour affirmer sur le contenu d'une
  collection (longueur, éléments contenus, ordre) à l'intérieur des
  tests de handler et de modèle.
