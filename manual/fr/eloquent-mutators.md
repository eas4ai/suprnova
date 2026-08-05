# Eloquent - Casts, accesseurs et mutateurs

Un cast fait la médiation à la frontière entre ce qu'une colonne
détient sur le disque et ce que votre modèle porte en mémoire. Un
accesseur invente un attribut virtuel à partir des colonnes que vous
avez déjà. Un mutateur route les écritures d'un champ à travers votre
propre transformation. Avec les timestamps auto-gérés, ce sont les
quatre pièces mobiles qui transforment une ligne plate en une valeur
Rust typée.

Ce chapitre couvre la surface complète des casts (chaque type
intégré, la redéfinition à l'exécution `casts!`, le chiffrement et le
hachage), les macros d'attribut `#[accessor]` et `#[mutator]`, le
contrat d'auto-timestamp incluant `touch()` et `without_touching`, et
l'événement de cycle de vie `Replicating` qui se déclenche quand vous
clonez un modèle avec `replicate()`.

Pour la surface de modèle plus large (`#[suprnova::model]`, query
builder, relations, observateurs), voir le chapitre
[Eloquent API](eloquent.md). Pour les événements de cycle de vie de
bout en bout, voir [Événements et écouteurs](events.md). Pour la
façade crypto que les casts chiffrés utilisent, voir
[Chiffrement](encryption.md).

## Comment fonctionnent les casts

Chaque cast est une struct qui implémente le trait `Cast` :

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` est le type Rust que vous écrivez dans votre struct modèle
(`bool`, `chrono::NaiveDate`, `rust_decimal::Decimal`, votre propre
enum). `Storage` est le type que SeaORM voit sur la colonne (`i64`
pour une colonne booléenne SQLite, `String` pour une date TEXT). Les
deux directions sont faillibles - l'analyse temporelle et décimale
peut rejeter une entrée malformée - si bien que la macro propage le
`Result` à travers `From<inner::Model>` et le chemin d'écriture
`ActiveModel`.

Les casts sont explicites. Un champ `Vec<String>` ne devient pas
implicitement `AsArray<String>`, parce que l'inspection du type de
champ au moment de la macro casserait dès que vous renommeriez un
alias ou importeriez un `Vec` différent. Vous déclarez les casts sur
l'attribut de macro :

```rust
use suprnova::{model, AsArray, AsBool, AsJson};

#[model(
    table = "posts",
    casts = {
        tags = AsArray<String>,
        published = AsBool,
        metadata = AsJson<serde_json::Value>,
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub tags: Vec<String>,
    pub published: bool,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

La macro développe chaque entrée `field = CastType` en appels vers
`Cast::to_storage` et `Cast::from_storage` à chaque lecture et
écriture. Vous n'invoquez jamais le cast vous-même - vous écrivez le
type runtime, le cast câble la forme de la colonne.

### Pourquoi Suprnova diverge

Laravel déclare les casts comme
`protected $casts = ['tags' => 'array']`. La chaîne `'array'` se
résout vers une classe via une recherche à l'exécution, ce qui veut
dire que les noms de cast vivent comme des chaînes non typées jusqu'à
leur exécution. Suprnova prend le type directement -
`AsArray<String>` est un vrai type Rust que la macro vérifie à la
compilation. Une coquille dans le nom du cast est une erreur de
compilation, pas une exception à l'exécution trois semaines après le
déploiement.

## Les casts primitifs

Cinq casts couvrent les types scalaires SQL.

### `AsBool`

`bool` ↔ `INTEGER` (0 / 1). SQLite n'a pas de colonne booléenne
native ; Postgres et MySQL font tous deux l'aller-retour de `i64`
proprement à travers la frontière `Value::Int` de SeaORM. Une forme de
stockage unique vous permet d'utiliser le même cast contre chaque
backend.

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

Un entier plus étroit (`i32`, `u32`, `i16`) ↔ `i64`. SeaORM stocke les
entiers en `i64` sur la colonne ; le cast rétrécit à la lecture et
élargit à l'écriture. Les valeurs hors plage produisent une erreur de
validation à la lecture plutôt que de tronquer silencieusement.

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

Utilisez `AsInt<i64>` (ou omettez le cast) quand le type runtime
correspond déjà au stockage.

### `AsFloat`

`f64` ↔ `REAL`. Transparent dans les deux directions - le cast existe
pour la parité de nommage avec le cast `'float'` de Laravel ; les
backends font l'aller-retour des flottants nativement.

### `AsString`

`String` ↔ `TEXT`. Également transparent ; le cast existe pour que la
redéfinition à l'exécution `Builder::with_casts(...)` puisse l'effacer
vers un `DynCast` comme tout autre cast.

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT`. `P` est la précision (nombre de
décimales) ; les valeurs sont arrondies à `P` décimales en chemin vers
le stockage. Le défaut est `P = 4`. Le stockage est une chaîne à
format fixe pour que les aller-retours soient agnostiques au backend -
le type de colonne `Decimal` natif de SeaORM a une sémantique de
précision différente sur chaque driver, et l'aller-retour par chaîne
évite cela.

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // devise, 2 décimales
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## Les casts temporels

Six casts couvrent les dates, les datetimes, les variantes immuables,
et les timestamps Unix. Tous les casts non-timestamp se stockent en
`TEXT` (ISO-8601 / RFC-3339) pour que l'aller-retour fonctionne sur
chaque driver - SQLite stocke les datetimes comme des chaînes
nativement, et Postgres / MySQL les acceptent à travers la frontière
`Value::String` de SeaORM.

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT` (`YYYY-MM-DD`).

```rust
use chrono::NaiveDate;
use suprnova::AsDate;

#[model(table = "people", casts = { birthday = AsDate })]
pub struct Person {
    pub id: i64,
    pub birthday: NaiveDate,
}
```

### `AsDateTime`

`chrono::DateTime<Utc>` ↔ `TEXT` (RFC-3339). Le cast par défaut pour
des timestamps arbitraires quand vous voulez une représentation en
horloge murale.

### `AsImmutableDate` et `AsImmutableDateTime`

Même forme de stockage que `AsDate` / `AsDateTime`. Le vérificateur
d'emprunt de Rust impose déjà l'immutabilité via les références `&`,
si bien que ces casts partagent les types sous-jacents - ils existent
pour la parité avec `immutable_date` / `immutable_datetime` de Laravel
et pour documenter l'intention au site de déclaration du modèle.

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>`. Auto-injecté par le flag
`#[model(soft_deletes)]` pour la colonne marqueur de suppression
nullable (`deleted_at` par défaut - voir
[Suppressions logicielles](eloquent.md#deleting-and-soft-deletes)).
L'option enveloppée garde la colonne de stockage nullable, si bien que
les lignes supprimées de façon logicielle et les lignes vivantes se
distinguent sur `IS NULL` sans valeur sentinelle.

Utilisez le cast directement sur toute autre colonne datetime nullable
que vous voulez faire aller-retour en texte RFC-3339 :

```rust
#[model(
    table = "subscriptions",
    casts = { cancelled_at = AsOptionalDateTime },
)]
pub struct Subscription {
    pub id: i64,
    pub cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### `AsTimestamp`

`i64` en epoch Unix ↔ `INTEGER`. À utiliser quand la colonne est
requêtée comme une plage numérique ou utilisée dans de
l'arithmétique. Distinct de `AsDateTime` - choisissez `AsTimestamp`
quand vous voulez `WHERE created_unix > 1700000000` et `AsDateTime`
quand vous voulez des chaînes RFC-3339 dans vos logs.

## Les casts structurés

Cinq casts couvrent les collections, les structs, et le JSON
arbitraire. Tous sérialisent la valeur runtime en texte JSON et la
stockent dans une colonne `TEXT`. Les colonnes `JSON` / `JSONB`
natives de Postgres et `JSON` de MySQL acceptent la même charge utile
en chaîne - si vous voulez un type de colonne JSON natif pour
l'indexation, déclarez-le manuellement dans une migration ; la couche
de cast ne contraint pas le type de colonne.

### `AsArray<T>`

`Vec<T>` ↔ `TEXT` encodé en JSON. Le type d'élément doit être
`Serialize + DeserializeOwned`.

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

Une struct `Serialize + DeserializeOwned` ↔ `TEXT` encodé en JSON. À
utiliser quand la forme runtime est un enregistrement fixe avec des
clés connues statiquement.

```rust
use serde::{Deserialize, Serialize};
use suprnova::AsObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

#[model(table = "users", casts = { prefs = AsObject<Prefs> })]
pub struct User {
    pub id: i64,
    pub prefs: Prefs,
}
```

### `AsCollection<T>`

`Collection<T>` ↔ `TEXT` encodé en JSON. Mince wrapper au-dessus de
`AsArray` qui fait l'aller-retour à travers le `Collection<T>` de
Suprnova (un newtype `Vec<T>` avec la surface de slice en style
Laravel - voir [Collections](eloquent.md#collections)).

### `AsJson<T>`

N'importe quel type `Serialize + DeserializeOwned` ↔ `TEXT` encodé en
JSON. À utiliser quand le champ est un `serde_json::Value` ou une
struct définie par l'utilisateur qui est déjà entièrement descriptible
en termes serde mais ne correspond pas au motif à forme fixe
`AsObject` (par exemple des payloads d'enum, des maps non typées).

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ `TEXT` encodé en JSON. À utiliser quand la
forme runtime est une map à clé dynamique et que l'ordre des clés
compte (l'ordre d'affichage des labels dans l'UI, l'ordre canonique
d'un bloc de config). `IndexMap` plutôt que `HashMap` est délibéré :
serde préserve l'ordre d'insertion via `IndexMap`, et le `serde_json`
de Suprnova est déjà configuré avec `preserve_order` pour la même
raison.

Pour des enregistrements à forme fixe, utilisez `AsObject` ; pour des
tableaux, utilisez `AsArray`.

## Le cast enum

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT`. Le nom de la variante de l'enum
(ou sa chaîne personnalisée via `AsRefStr`) est ce qui atteint la
colonne. Il n'y a aucun verrouillage du framework sur `strum`, mais
c'est le moyen le plus ergonomique d'obtenir les deux bornes sans les
écrire à la main :

```rust
use suprnova::AsEnum;

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    casts = { role = AsEnum<Role> },
)]
pub struct User {
    pub id: i64,
    pub role: Role,
}
```

Le stockage par discriminant entier n'est délibérément pas le défaut.
Un `Role::Admin = 0` qui devient plus tard `Role::Admin = 2` après un
réordonnancement échangerait silencieusement chaque admin dans la
base de données. Les noms de variante sont auto-descriptifs dans un
navigateur de BD et stables à travers les réordonnancements.

## Chiffrement et hachage

Cinq casts font la médiation de transformations cryptographiques à la
frontière de stockage. Les quatre casts `AsEncrypted*` partagent tous
la façade [`Crypt`](encryption.md) - la façade doit être initialisée
avant qu'aucun d'eux ne s'exécute. Les applications de production
obtiennent cela via `Server::from_config` (qui lit `APP_KEY` depuis
l'environnement) ; les tests appellent
`suprnova::testing::install_test_encryption_key()` une fois au
démarrage.

### `AsEncrypted`

`String` ↔ `String` chiffrée en AES-256-GCM. La colonne sur disque
détient du base64 URL-safe de `nonce || ciphertext_with_tag`. Chaque
écriture utilise un nonce aléatoire frais, si bien que deux écritures
du même texte en clair produisent des textes chiffrés distincts -
votre admin BD ne peut pas identifier des secrets dupliqués au repos.

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // le runtime est de l'UTF-8 en clair
}
```

La valeur runtime est la chaîne UTF-8 déchiffrée ; vous la lisez et
l'écrivez comme n'importe quel autre `String`.

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ JSON chiffré en AES-256-GCM. Le
pipeline est : sérialiser en JSON → chiffrer → base64 → stocker ;
l'inverse à la lecture. Le type d'élément / de valeur doit être
`Serialize + DeserializeOwned`.

```rust
use suprnova::AsEncryptedObject;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[model(
    table = "billing",
    casts = { card = AsEncryptedObject<CardOnFile> },
)]
pub struct Billing {
    pub id: i64,
    pub card: CardOnFile,
}
```

### Rotation des clés

La façade `Crypt` prend en charge la rotation via `APP_KEY_PREVIOUS` :
le chiffrement utilise toujours `APP_KEY`, mais le déchiffrement
essaie `APP_KEY` d'abord et retombe sur `APP_KEY_PREVIOUS` si la clé
primaire échoue. Une stratégie de rechiffrement progressif est :
positionnez `APP_KEY` sur la nouvelle clé, déplacez l'ancienne clé
vers `APP_KEY_PREVIOUS`, puis appelez `save()` sur chaque ligne
chiffrée pour réécrire les textes chiffrés sous la nouvelle clé. La
couche de cast n'a pas besoin de connaître la rotation - elle fait
l'aller-retour à travers `Crypt` à chaque lecture et écriture, si bien
qu'un `User::all().await?` suivi de la sauvegarde de chaque ligne
migre la colonne en place. Voir [Chiffrement](encryption.md) pour le
protocole de rotation complet.

### `AsHashed`

`String` ↔ une chaîne hachée à l'écriture, en utilisant le driver de
hachage actif (variable d'env `HASH_DRIVER` - bcrypt par défaut,
argon2i et argon2id aussi supportés). La valeur runtime EST la chaîne
hachée ; il n'y a pas de direction inverse. Reflète le cast `hashed`
de Laravel.

```rust
use suprnova::AsHashed;

#[model(
    table = "users",
    casts = { password = AsHashed },
)]
pub struct User {
    pub id: i64,
    pub password: String,
}
```

`AsHashed::to_storage` est **idempotent** : une valeur qui ressemble
déjà à N'IMPORTE QUEL hash reconnu (bcrypt `$2*$`, argon2i / argon2id
au format PHC) passe inchangée. Sans ce garde-fou,
`User::find(id).await?.save().await?` rehacherait le hash existant en
un hash de hash, cassant `Hash::check(plain, stored)` et invalidant
chaque mot de passe existant.

Associez `AsHashed` au motif `#[mutator]` (plus bas) quand vous avez
besoin d'appliquer plus qu'un hachage à l'écriture - par exemple
normaliser les espaces ou rejeter les mots de passe vides avant le
hachage.

## Redéfinition de cast à l'exécution - macro `casts!`

Les casts déclarés dans `#[model(casts = { ... })]` sont statiques -
ils se déclenchent à chaque lecture de ce modèle. Quand vous avez
besoin d'un cast différent sur une seule requête (un outil de debug
veut la forme stockée brute, un script d'export veut une
représentation JSON différente), utilisez `Builder::with_casts(...)` :

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

La macro `casts!` construit un
`HashMap<&'static str, Arc<dyn DynCast>>`. Chaque entrée est
`field_name = CastType` ; chaque cast intégré implémente `IntoDynCast`,
si bien que l'ombre `DynCast` à type effacé est automatique. La map de
redéfinition à l'exécution ne s'applique que pour la durée de la
requête chaînée - le pipeline de cast statique du modèle est inchangé.

Utilisez cette surface avec parcimonie. L'attribut de modèle est le
bon endroit pour les casts que vous voulez voir s'appliquer à chaque
lecture ; la redéfinition à l'exécution est l'échappatoire pour les
requêtes ponctuelles.

## Accesseurs - attributs virtuels à partir de colonnes réelles

Un accesseur est une méthode `impl` sur le modèle annotée avec la
macro `#[accessor]`. Quand vous listez le nom de la méthode dans
`#[model(appends = [...])]`, le `to_json()` du modèle appelle la
méthode et insère le résultat sous cette clé.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

Un `serde_json::to_value(&user)` (ou `user.to_json()`) contient
désormais :

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

La méthode est aussi appelable directement (`user.full_name()`) - la
macro `#[accessor]` est surtout un marqueur pour que la macro
`#[suprnova::model]` au niveau de la struct puisse câbler le dispatch
de `to_json()`. Il n'y a aucun coût à l'appeler depuis votre propre
code.

Chaque nom dans `appends` doit correspondre à une vraie méthode
`#[accessor]` par identifiant. Une coquille
(`appends = ["fullName"]` quand la méthode est `full_name`) est
attrapée à la compilation avec un message d'erreur pointé.

### Retourner des valeurs non-`String`

Les accesseurs peuvent retourner n'importe quel type `Serialize`. La
macro convertit la valeur retournée via `serde_json::to_value` avant
l'insertion, donc :

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

se rend en `"word_count": 42` dans la sortie JSON.

### Masquer les colonnes source

Quand la valeur de l'accesseur est ce que le consommateur devrait
voir et que les colonnes sous-jacentes sont du bruit, associez
`appends` à `hidden` :

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` retire les colonnes nommées de la sortie sérialisée ;
`appends` insère ensuite la valeur de l'accesseur. L'ordre est fixe -
les filtres s'exécutent d'abord, l'injection de l'accesseur s'exécute
après. Voir [Hidden, visible et appends](eloquent.md#mass-assignment)
pour la surface complète.

## Mutateurs - écritures routées à travers votre transformation

Un mutateur est le pendant côté écriture. Quand le nom du champ
apparaît dans `#[model(mutators = [...])]`, chaque chemin
d'affectation en masse (`create` / `update`) route la valeur à
travers `self.set_<field>(value)?` au lieu d'affecter le champ
directement.

```rust
use serde_json::Value;
use suprnova::{model, mutator, FrameworkError, Model};

#[model(
    table = "users",
    fillable = ["password"],
    mutators = ["password"],
)]
pub struct User {
    pub id: i64,
    pub password: String,
}

impl User {
    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        // Normalise + hash ; AsHashed ferait le hash tout seul,
        // mais le mutateur est l'endroit où vous pouvez aussi imposer une politique.
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

`set_password` reçoit un `serde_json::Value`. Le corps possède la
désérialisation + la transformation - le type du champ sur la struct
peut rester `String`, et votre validation s'exécute avant que la
colonne ne soit touchée. Une erreur retournée se propage à travers
`create()` / `update()` comme un `bad_request`.

L'affectation directe du champ contourne le mutateur :

```rust
user.password = "raw".to_string();  // saute set_password
user.save().await?;                 // sauvegarde "raw"
```

Ceci correspond au comportement `$user->password = ...` vs
`$user->fill(...)` de Laravel. Quand vous voulez que le mutateur soit
le seul chemin, routez toutes les écritures à travers `attrs!` +
`create` / `update`.

### Combiner mutateurs et casts

Un mutateur et un cast peuvent coexister sur le même champ ; le
mutateur s'exécute sur le chemin d'écriture (quand `create` /
`update` est appelé), le cast s'exécute sur le chemin de lecture
(quand la colonne est matérialisée depuis un SELECT). Un motif
courant est d'utiliser `AsHashed` pour la garantie d'idempotence côté
lecture et le mutateur pour la validation côté écriture - le mutateur
hache, `AsHashed` voit une valeur déjà hachée et passe à travers.

## Timestamps auto-gérés

Quand un modèle porte à la fois les champs `created_at` et
`updated_at` (typés `chrono::DateTime<chrono::Utc>`), la macro :

- Positionne les deux à `Utc::now()` lors de `create()`.
- Avance `updated_at` à chaque `save()` et `update(attrs)`.
- Émet un `impl Touchable for YourStruct` pour que vous puissiez
  appeler `.touch().await` pour avancer `updated_at` sans changer
  aucune autre colonne.

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model, Touchable};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Avance updated_at sans autre changement :
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

Le stockage utilise le cast `AsDateTime` que la macro auto-injecte
pour les colonnes timestamp. Le cast permet à la même valeur
`DateTime<Utc>` de faire l'aller-retour à travers les trois drivers
SeaORM (SQLite, MySQL, PostgreSQL) sans vous forcer à choisir un type
timestamp spécifique à une base de données.

### Désactivation et noms de colonne personnalisés

`#[model(timestamps = false)]` désactive entièrement l'auto-gestion -
vous contrôlez les timestamps vous-même.

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]`
garde l'auto-gestion mais renomme les colonnes. La macro détecte les
champs renommés et câble la même logique contre eux.

Quand la struct n'a qu'UN des deux champs timestamp, la macro émet un
`compile_error!` - presque toujours une coquille (`craeted_at`) que
vous voulez voir remonter de manière visible plutôt qu'avalée
silencieusement.

### `without_touching` - suppression cantonnée à la tâche

Parfois vous voulez mettre à jour une ligne sans avancer
`updated_at` - en exécutant un backfill, en corrigeant une coquille,
en enregistrant une synchronisation interne qui ne devrait pas
réinitialiser des TTL de cache indexés sur `updated_at`. Enveloppez le
travail dans `without_touching` :

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // sans effet à l'intérieur de la portée
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

Le flag est un `tokio::task_local!` si bien qu'il ne fuit pas à
travers les frontières `tokio::spawn` - les requêtes concurrentes sur
d'autres tâches continuent d'honorer leur propre portée (ou son
absence). C'est l'analogue Suprnova du `Model::withoutTouching(closure)`
de Laravel.

### Pourquoi Suprnova diverge

Laravel utilise une propriété statique `$timestamps = false` et une
méthode statique globale `Model::withoutTouching` soutenue par un
compteur d'instances. Les deux approches supposent une isolation
request-per-process. Suprnova exécute de nombreuses requêtes sur un
seul runtime Tokio, si bien qu'un flag global au processus laisserait
une requête supprimer silencieusement les timestamps d'une autre. La
portée `tokio::task_local!` est consciente de l'async : elle suit les
futures à travers les points `.await` à l'intérieur de la même tâche
et sort de portée quand la future est droppée, quelle que soit la
façon dont la requête se termine.

## L'événement de cycle de vie `Replicating`

Parmi les 16 événements de cycle de vie du modèle (voir
[Observateurs et événements de cycle de vie](eloquent.md#observers-and-lifecycle-events)),
`Replicating` est celui qui se déclenche quand vous clonez une ligne
existante en une copie non sauvegardée en mémoire via `replicate()` :

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // non sauvegardée
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // désormais persistée avec une nouvelle PK
```

L'événement `Replicating` se déclenche APRÈS que le clone en mémoire
est construit mais AVANT que vous n'ayez eu la chance de le muter.
Les écouteurs reçoivent `(&Self, Arc<Mutex<Self>>)` - l'original et la
réplique fraîchement construite derrière un `Mutex`, si bien que vous
pouvez muter la réplique depuis l'écouteur avant que l'utilisateur ne
la voie :

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // les copies démarrent non publiées
        replica.view_count = 0;          // les compteurs sont réinitialisés
        Ok(())
    }
}
```

La PK de la réplique est déjà effacée au moment où l'écouteur
s'exécute - `replicate()` appelle `reset_primary_key()` avant de
déclencher l'événement, si bien que vous ne pouvez pas accidentellement
resauvegarder sous l'ID d'origine. Les timestamps sont aussi
réinitialisés ; `created_at` / `updated_at` se déclenchent au `save()`
suivant comme n'importe quelle nouvelle ligne.

### `replicate_into<T>` - réplication entre types

Quand la réplique est d'un type différent (`Post` → `Draft`, par
exemple), utilisez `replicate_into::<Draft>()`. L'événement
`Replicating` ne se déclenche PAS sur ce chemin parce que la struct
d'événement est propre à chaque type source, et un écouteur enregistré
pour `post::events::Replicating` recevrait un `Arc<Mutex<Post>>`, pas
un `Arc<Mutex<Draft>>`. Le chemin entre types est pour quand vous
voulez un type cible neuf sans interférence d'observateur ;
enregistrez un écouteur `Creating` normal sur le type cible si vous
voulez un hook à la construction.

Voir [Réplication](eloquent.md#replication) pour le reste de la
surface de replicate (`replicate_except`, la gestion des relations de
la réplique, les règles pour les PK nullables).

## Tout assembler

Un modèle avec chaque surface de ce chapitre :

```rust
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::{
    accessor, hashing, model, mutator, AsBool, AsDateTime,
    AsDecimal, AsEncryptedObject, AsEnum, AsHashed, AsJson,
    AsOptionalDateTime, FrameworkError, Model,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    soft_deletes,
    appends = ["display_name"],
    hidden = ["password", "card"],
    fillable = ["name", "email", "password", "role", "credit"],
    mutators = ["password"],
    casts = {
        role = AsEnum<Role>,
        verified = AsBool,
        credit = AsDecimal<2>,
        card = AsEncryptedObject<CardOnFile>,
        metadata = AsJson<serde_json::Value>,
        password = AsHashed,
        last_login_at = AsOptionalDateTime,
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
    pub verified: bool,
    pub credit: Decimal,
    pub card: CardOnFile,
    pub metadata: serde_json::Value,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // deleted_at est auto-injecté par soft_deletes (AsOptionalDateTime)
}

impl User {
    #[accessor]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { self.name.clone() }
    }

    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        // Le mutateur hache ; AsHashed voit une valeur déjà hachée
        // lors des sauvegardes suivantes et passe inchangée.
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

Cette seule déclaration vous donne :

- Huit casts typés qui câblent la frontière stockage / runtime.
- Un accesseur qui synthétise `display_name` à partir de colonnes
  existantes.
- Un mutateur qui valide et hache le mot de passe.
- `created_at` / `updated_at` auto-gérés.
- Suppressions logicielles avec une colonne `deleted_at`
  auto-injectée.
- Stockage chiffré de carte enregistrée avec support de rotation de
  clé.

Chaque cast est vérifié à la compilation. Le query builder à double
API (voir [Eloquent - query builder](eloquent.md#query-builder--dual-api))
s'exécute contre les colonnes typées ; la sérialisation vers Inertia /
JSON applique les règles hidden / appends ; et un
`User::find(id).await?` matérialise la ligne à travers huit appels
`Cast::from_storage` sans que vous n'écriviez une seule ligne de code
de conversion.

## Suivant

- [Eloquent API](eloquent.md) - le reste de la surface de modèle :
  query builder, relations, observateurs, pagination, transactions.
- [Chiffrement](encryption.md) - la façade `Crypt` que les casts
  chiffrés partagent, le protocole de rotation de clé, et la surface
  crypto plus large.
- [Événements et écouteurs](events.md) - le dispatcher derrière
  `Replicating` et les 15 autres événements de cycle de vie du
  modèle.
- [Authentification](authentication.md) - le trait `Authenticatable`
  et où `AsHashed` s'insère dans le flux de mot de passe.
- [Validation](validation.md) - `FrameworkError::validation` et le
  motif que les mutateurs utilisent pour faire surgir des erreurs par
  champ.
