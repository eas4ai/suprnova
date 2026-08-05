# Vector

Suprnova livre une façade `Vector` à la Laravel, adossée à l'un de
quatre drivers - Memory en mémoire dans le process, Qdrant, Pinecone,
ou le type natif `VECTOR(N)` de MariaDB - choisi explicitement à
l'amorçage via `Vector::register`. La façade est une couche mince
par-dessus un trait `VectorDriver`, si bien que des backends
personnalisés se branchent de la même façon que les backends
intégrés.

## Démarrage rapide

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// Amorçage (généralement une fois au démarrage de l'app)
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// Utilisation
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## Le contrat

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` porte un id `String` arbitraire, un `embedding:
Vec<f32>`, et des `metadata: serde_json::Value` libres (doit être un
objet JSON ou `null`). `VectorMatch` retourne l'id d'origine, le score
de similarité du backend, et la même forme de métadonnées.

Le trait est volontairement réduit. Quand vous avez besoin
d'expressions de filtre sur la recherche, de vecteurs creux, de
scroll/list, d'instantanés, ou de réglages de quantification,
redescendez vers le SDK sous-jacent du driver via son échappatoire
publique `client()`.

### Pourquoi Suprnova diverge

Laravel ne livre les vecteurs que via `pgvector` de Postgres. C'est la
réponse à la forme PHP : choisir un seul backend de stockage, le
cacher derrière un unique driver, et considérer que c'est fait.
Suprnova traite ce choix comme un problème de configuration. Le même
trait couvre un `HashMap` en mémoire dans le process pour les tests,
une base de données vectorielle dédiée (Qdrant, Pinecone) quand le
nombre d'embeddings justifie le coût opérationnel, et un backend
relationnel (MariaDB 11.7+) quand vous préférez garder les vecteurs à
côté des lignes qui les ont produits. Weaviate, Milvus, LanceDB,
pgvector et LibSQL attendent une demande réelle des utilisateurs -
aucun n'est bloqué par la forme du trait.

Quand le reste de votre app tient sur un seul moteur, MariaDB 11.7+
garde les vecteurs aux côtés des tables relationnelles, des documents
JSON, et des données temporelles versionnées par le système - moins
de pièces mobiles que de faire tourner Postgres + Redis + Qdrant
séparément. Voir [Déploiement](deployment.md) pour la recommandation
en contexte.

## Drivers

### Memory - `MemoryVectorDriver`

Driver en mémoire dans le process, adossé à un `HashMap`. Similarité
cosinus, les points dont la dimension ne correspond pas sont
silencieusement ignorés lors de la requête (si bien que des données
de test à dimensions mixtes ne font pas tout exploser), les requêtes
à vecteur nul renvoient une erreur claire.

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

À utiliser en tests et en dev. Chaque instance de
`MemoryVectorDriver::new()` est hermétique - aucun état partagé entre
deux `new()`.

### Qdrant - `QdrantVectorDriver`

Communique avec Qdrant via gRPC (port 6334 par défaut) à travers le
SDK officiel `qdrant-client`.

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // par défaut
    .with_auto_create(true);                // par défaut

Vector::register("docs", Arc::new(driver));
```

Pour Qdrant Cloud :

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**Correspondance des ID.** Qdrant exige que les ID de point soient
soit un `u64`, soit un UUID valide. Le framework fait le pont avec
des chaînes arbitraires via trois règles :

1. Si la chaîne s'interprète comme un `u64`, utiliser la variante
   `Num(u64)`.
2. Si la chaîne est un UUID valide, utiliser la variante
   `Uuid(String)` telle quelle.
3. Sinon, dériver un UUID v5 déterministe à partir d'un namespace
   stable.

La chaîne d'origine de l'appelant est rangée dans le payload du point
sous la clé réservée `__suprnova_id` (exportée sous le nom
`SUPRNOVA_ID_PAYLOAD_KEY`) et retirée de `VectorMatch.metadata` à la
lecture. Les utilisateurs avancés qui interrogent Qdrant directement
via `driver.client()` peuvent filtrer sur `__suprnova_id` pour faire
le pont entre les écritures du framework et les appels directs.

**Création automatique.** Au premier `upsert` sur une collection
inconnue, le driver la crée avec la dimension déduite du premier
élément et la métrique de distance configurée (Cosine par défaut).
Sûr en cas de concurrence - des appels `upsert` concurrents sur la
même collection toute fraîche n'échouent pas ; celui qui crée en
premier l'emporte, l'autre continue. Désactivez via
`.with_auto_create(false)` pour exiger une création explicite.

**Invalidation du cache.** Si une collection est supprimée de
l'extérieur (ou si Qdrant redémarre avant que la persistance n'ait
été vidée sur disque), le driver détecte l'erreur « not found » à
l'upsert, supprime l'entrée du cache, relance `ensure_collection`, et
retente une fois.

**Échappatoire.** `driver.client()` retourne le
`qdrant_client::Qdrant` sous-jacent - utilisez-le pour les
expressions de filtre sur la recherche, le scroll, les instantanés,
ou toute autre API non exposée via le trait.
`QdrantVectorDriver::resolve_point_id`, `build_point`, et
`decode_match` vous permettent de mélanger des appels directs et des
appels routés par le trait sans perdre la traduction des ID.

**Mise en place locale.** Lancez Qdrant via Docker :

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

Les tests d'intégration se lancent via :

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **Conditionné par une feature - désactivé par défaut.**
> Activez-la avec `cargo build --features vector-pinecone` (ou
> ajoutez `features = ["vector-pinecone"]` sous la dépendance
> `suprnova` de votre `Cargo.toml`). La feature ne coûte aucune
> dépendance supplémentaire - elle conditionne seulement la
> compilation du driver, rien de plus - elle est donc désactivée
> simplement parce que la plupart des apps n'utilisent pas Pinecone
> et ne devraient pas payer pour la compiler.

Communique avec Pinecone via son API REST, en utilisant le client
HTTP que le framework transporte déjà.

> **Pourquoi pas le SDK officiel ?** Le driver enveloppait
> auparavant `pinecone-sdk`, qui parle gRPC. La dernière version de
> cette crate (0.1.2, publiée le 2024-09-06) épingle `tonic 0.11 →
> rustls 0.22 → rustls-webpki 0.102`, et `rustls-webpki 0.102` porte
> quatre avis RustSec, tous corrigés en amont dans `>= 0.103.13`.
> Une crate abandonnée bloquait tout l'arbre de dépendances, sans
> qu'aucune forme d'« attendre que l'amont s'en charge » n'ait de fin
> en vue. Pinecone expose en HTTPS toutes les opérations dont ce
> driver a besoin, si bien que la route REST a supprimé quatre avis
> et deux dépendances d'un coup.

```rust
use suprnova::PineconeVectorDriver;

// Clé API directement
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// Ou via l'env : PINECONE_API_KEY, plus PINECONE_CONTROLLER_HOST
// et PINECONE_API_VERSION en option
let driver = PineconeVectorDriver::from_env()?;

// Se lier à un namespace non par défaut
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

Le nom de store passé via `Vector::store(name)` correspond à un nom
d'index Pinecone. Le driver résout l'hôte de cet index
paresseusement, à la première utilisation, via le `GET
/indexes/{name}` du control plane, puis le met en cache. Évitez
l'aller-retour en épinglant l'hôte que vous connaissez déjà :

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

Un hôte appris depuis le control plane est toujours contacté en
`https`, quoi que dise la réponse. Un hôte épinglé via
`with_index_host` garde le schéma que vous lui avez donné, si bien
qu'un émulateur local en `http://` fonctionne.

**Version de l'API.** Pinecone versionne son API REST par date et
veut que cette version soit épinglée dans un en-tête. Le driver
épingle `2025-04` - la version contre laquelle ses formes de requête
et de réponse ont été écrites et testées - et expose
`with_api_version` (ou `PINECONE_API_VERSION`) pour évoluer
délibérément. Elle ne flotte pas : la convention de clé de namespace
dans `describe_index_stats` est l'une des choses qui a changé entre
les versions, et `count()` lit cette map.

**Pas de création automatique.** La création d'un index Pinecone
exige de choisir le cloud (AWS/GCP/Azure), la région, la dimension du
vecteur, la métrique de distance, et la protection contre la
suppression - trop de compromis pour un choix par défaut
satisfaisant. Créez les index via la console Pinecone, la CLI
Pinecone, ou un appel `control_plane_post` avant l'enregistrement,
puis pointez le framework vers le nom existant.

C'est la principale asymétrie avec le driver Qdrant, qui crée
automatiquement les collections au premier upsert.

**ID et métadonnées.** Pinecone accepte nativement des ID `String`
arbitraires, si bien que `VectorItem::id` passe directement. Les
métadonnées sont transportées en JSON de bout en bout -
`PineconeVectorDriver::metadata_from_json` / `metadata_to_json` ne
font qu'appliquer la propre règle du framework selon laquelle les
métadonnées sont un objet ou null. Pinecone lui-même restreint les
*valeurs* des métadonnées aux chaînes, nombres, booléens et listes de
chaînes, et rejette les objets imbriqués côté serveur ; le driver ne
réimplémente pas cette vérification, parce que les règles de Pinecone
sont versionnées et qu'une copie locale finirait par diverger.

**Limites de lot.** Pinecone documente un maximum de 1000 vecteurs
par upsert et 1000 ID par delete. Le driver envoie ce que vous lui
donnez en une seule requête plutôt que de découper silencieusement en
lots - une écriture partiellement réussie est plus difficile à
raisonner qu'une écriture rejetée. Faites vos propres lots si vous
dépassez ces limites.

**Namespaces.** Une instance de driver se lie à un seul namespace.
Pour utiliser plusieurs namespaces du même index, enregistrez un
driver par namespace sous des noms de store différents :

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**Débit.** Rien ne se sérialise. Le driver met en cache une chaîne
d'hôte par index, pas un handle de connexion, et les requêtes
partagent le pool de connexions de `reqwest` - si bien que des appels
concurrents vers le même index avancent en parallèle. (Le driver
gRPC que celui-ci remplace gardait un `Index` par nom derrière un
`tokio::Mutex`, parce que `pinecone-sdk` n'exposait `Index` que
derrière `&mut self`.)

**Échappatoire.** `control_plane_get`, `control_plane_post` et
`data_plane_post` atteignent n'importe quel endpoint que Pinecone
expose, avec vos propres types de requête et de réponse, via le
transport authentifié et à hôte résolu du driver - expressions de
filtre, vecteurs creux, fetch-by-id, `/vectors/list`, gestion des
index :

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**Tests.** Les tests de contrat réseau tournent par défaut sous la
feature : ils font tourner le driver contre un fake local et
vérifient la méthode, le chemin, les en-têtes et le corps JSON exacts
qu'il met sur le réseau. Ceux-ci épinglent le driver au contrat
*documenté* de Pinecone. Confirmer que la documentation correspond au
service réel nécessite les tests d'intégration marqués `#[ignore]`,
qui exigent les deux variables d'env :

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

Communique avec MariaDB 11.7+ via un `sqlx::MySqlPool` direct, en
utilisant le type de colonne natif `VECTOR(N)` de MariaDB et
l'indexation HNSW. La première fois que vous appelez une méthode du
driver, elle exécute `SELECT VERSION()` et rejette tout ce qui est en
dessous de 11.7 - les serveurs plus anciens n'ont pas les fonctions
vectorielles.

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // par défaut

Vector::register("documents", Arc::new(driver));
```

`from_url` est paresseuse - elle valide la syntaxe de l'URL mais
n'ouvre PAS de connexion avant la première utilisation, si bien que
l'appeler à l'amorçage de l'app est sûr même avant que la base de
données ne soit joignable. Enveloppez un pool existant avec
`MariaDbVectorDriver::from_pool(pool)` quand vous avez besoin
d'options de pool personnalisées.

**Le schéma vous appartient.** Le driver ne crée pas les tables
automatiquement - le schéma est un problème de migration. Le chemin
recommandé est `driver.ensure_table_sql_for(name, dim)`, qui hérite
de la distance configurée sur le driver, si bien que la clause
`DISTANCE=` de la migration et la fonction de requête utilisée par
`similar` sont garanties de correspondre :

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// Résultat :
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

Pour les générateurs de migration qui n'ont pas de driver dans leur
portée (outils CLI, scripts de build), utilisez la fonction statique
`MariaDbVectorDriver::ensure_table_sql(name, dim, distance)` et
passez la même `MariaDbDistance` que celle que vous configurerez plus
tard sur le driver.

**La distance doit correspondre des deux côtés.** MariaDB retombe
silencieusement sur un scan complet de table quand la fonction
utilisée au moment de la requête ne correspond pas à la clause
`DISTANCE=` de l'index. Le driver se protège contre cela sur deux
niveaux :

1. **`ensure_table_sql_for(name, dim)`** lit `self.distance` à la
   fois pour le SQL de migration émis et pour la fonction
   d'exécution dans `similar` - elles ne peuvent pas diverger l'une
   de l'autre, par construction.
2. **Une vérification à l'exécution au premier appel `similar`**
   exécute un `SHOW CREATE TABLE` par store, extrait la clause
   `DISTANCE=` réelle depuis le schéma en place, et renvoie une
   erreur claire si elle est en désaccord avec `with_distance(...)`.
   Le résultat est mis en cache, si bien que les appels suivants sont
   sans coût. Cela attrape les migrations écrites à la main ou les
   configurations `from_pool` qui contournent
   `ensure_table_sql_for`.

**Sécurité du nom de store.** Les noms de store s'interpolent dans le
SQL émis (MySQL ne paramètre pas les identifiants). Les noms sont
validés selon `[A-Za-z_][A-Za-z0-9_]*` d'une longueur ≤ 64 ; le nom
validé est ensuite entouré de backticks dans chaque instruction. Les
noms invalides échouent avec `FrameworkError::param` à la frontière
de `register`/`upsert`/`similar`/`delete`/`count`.

**ID et métadonnées.** `VARCHAR(255)` accepte des ID `String`
arbitraires - pas de dérivation d'UUID, pas de clé de payload
réservée. Les métadonnées font l'aller-retour via le type de colonne
`JSON` de MariaDB ; des métadonnées `null` sont stockées comme un SQL
`NULL`. Des métadonnées non-objet (tableaux, primitives) sont
rejetées avec `FrameworkError::param`, en parité avec Qdrant et
Pinecone.

**Normalisation du score.** MariaDB renvoie une *distance* brute
(plus bas = plus proche). Le contrat du trait est un *score* (plus
haut = plus similaire) - le driver convertit selon la métrique :

| Métrique  | MariaDB renvoie      | `score` exposé                |
| --------- | --------------------- | ------------------------------ |
| Cosine    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`     |
| Euclidean | `[0, ∞)` norme L2     | `1.0 / (1.0 + d)` → `(0, 1]`   |

Dans les deux cas, le classement est préservé (meilleur résultat en
premier), mais les valeurs absolues de score ne sont PAS comparables
entre drivers - seul l'ordre l'est. Chaque backend retombe sur une
convention « plus haut = meilleur », mais les plages diffèrent : le
cosinus de Memory renvoie `[-1, 1]`, le cosinus normalisé de MariaDB
renvoie `[0, 1]`, Qdrant émet sa similarité cosinus native en `[-1,
1]`, et Pinecone renvoie la similarité brute pour la métrique avec
laquelle l'index a été créé. Utilisez `score` pour trier au sein du
jeu de résultats d'un seul driver ; ne comparez pas les scores
numériques entre drivers sans les re-normaliser vous-même.

**Échappatoire.** `driver.pool()` retourne le `sqlx::MySqlPool`
sous-jacent pour les requêtes brutes que le trait ne couvre pas.
`MariaDbVectorDriver::embedding_to_vec_text`, `score_from_distance`,
et `ensure_table_sql` sont des fonctions pures que vous pouvez
appeler indépendamment quand vous mélangez du SQL direct avec des
appels routés par le trait.

**Comportement de l'upsert en masse.** `upsert` émet une instruction
multi-lignes `INSERT ... VALUES (...), (...), ...` par tranche de
500 lignes, le tout enveloppé dans une seule transaction. Les
allers-retours réseau chutent d'environ 500x par rapport à des
insertions ligne par ligne lors du chargement d'un corpus neuf ;
l'appel reste atomique sur l'ensemble du lot. La taille du lot est
interne - appelez `upsert` une fois avec tous vos éléments et le
driver gère le découpage.

**Les index HNSW se reconstruisent au moment du commit.** MariaDB met
à jour le graphe HNSW au fur et à mesure que les lignes entrent, mais
le travail d'indexation se concentre au commit. Un `upsert` d'1
million de lignes garde la transaction ouverte pendant toute la durée
de la construction de l'index, ce qui peut prendre plusieurs minutes.
Pour de très gros chargements initiaux, découpez le corpus en lots
de 10 000 à 100 000 lignes et appelez `upsert` de façon répétée, si
bien que chaque lot commite et libère le verrou entre les tours.
(Des appels `upsert` plus petits ne sont pas plus lents par ligne -
ils étalent simplement le travail d'indexation sur plus de points de
commit.)

**La dimension est fixée à la création de la table.** `VECTOR(N)`
fixe la dimension ; changer de modèle d'embedding, par exemple d'un
modèle à 768 dimensions vers un modèle à 1536 dimensions, impose une
migration complète de la table (nouvelle table, ré-embedding,
bascule). Planifiez les montées en version de modèle de la même
façon que vous planifieriez une migration de schéma - il n'existe
pas de chemin « ALTER COLUMN VECTOR(768) → VECTOR(1536) ».

**Dimensionnement du pool.** `from_url` utilise les
`MySqlPoolOptions` par défaut de sqlx - `max_connections = 10` au
moment de l'écriture. Pour les charges à fort QPS (des centaines
d'appels `similar` par seconde), construisez le pool vous-même avec
`MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` et
passez-le à `from_pool`. Le driver n'impose pas son propre plafond de
connexions.

**Mise en place locale.** Lancez MariaDB 11.7+ via Docker :

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

Les tests d'intégration se lancent via :

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## Comparaison des drivers

| Aspect | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| Stockage sous-jacent | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| Persistance | Aucune | Oui | Oui | Oui |
| Création automatique | n/a | Oui (configurable) | Non (l'utilisateur crée l'index) | Non (la migration est à vous) |
| ID en chaîne | Natif | Haché en UUID-5 | Natif | Natif |
| Clé de métadonnées réservée | Aucune | `__suprnova_id` | Aucune | Aucune |
| Débit | Par process | Concurrent | Concurrent (limité par le pool) | Concurrent (limité par le pool) |
| Métrique de distance | Cosine | Configurable | Fixée à la création de l'index | Cosine / Euclidean |
| Version requise | - | N'importe laquelle | N'importe laquelle | **11.7+** |

## Notes opérationnelles

**Conventions de nom de store.** Le nom de store passé à
`Vector::register` et `Vector::store` est une étiquette - ce peut
être n'importe quelle chaîne. Pour Qdrant, le framework l'utilise
comme nom de collection ; pour Pinecone, comme nom d'index. Faites
correspondre l'étiquette au schéma de nommage déjà en place du
backend.

**Ré-enregistrer** un nom avec une nouvelle instance de driver est,
par conception, une opération où la dernière écriture l'emporte -
utile pour permuter les drivers dans les harnais de test sans
redémarrer le process.

**Isolation des tests.** Les tests du driver Memory comme ceux
adossés au registre utilisent des noms de store uniques marqués d'un
timestamp pour éviter les collisions lors d'exécutions de tests en
parallèle.

**Sémantique des erreurs.** `Vector::store(name)` renvoie
`FrameworkError::not_found` pour les noms non enregistrés. Les échecs
au niveau du driver (réseau, authentification, dimension qui ne
correspond pas) reviennent sous forme de `FrameworkError::internal`
ou `FrameworkError::param`, avec la chaîne de cause dans le message
d'affichage.

## Étendre

Pour ajouter un cinquième backend (Weaviate, Milvus, LanceDB,
pgvector, LibSQL, ...) :

1. Ajoutez un nouveau `framework/src/vector/<backend>.rs`
   implémentant `VectorDriver`.
2. Ré-exportez le type de driver depuis
   `framework/src/vector/mod.rs` et depuis la racine de la crate.
3. Reprenez le découpage des tests de Pinecone : les tests de
   fonctions pures et les tests de contrat réseau (contre un fake
   `wiremock` local) tournent toujours ; les tests d'intégration
   sont conditionnés par `#[ignore]` derrière des variables d'env
   pour les identifiants. C'est la couche du milieu qui justifie sa
   place - un backend que personne ne peut atteindre depuis la CI a
   quand même un format réseau qu'une faute de frappe peut casser.

Le trait est volontairement réduit pour que la barre à franchir pour
livrer un nouveau driver reste basse. Si un backend a besoin d'une
surface qui ne rentre pas (expressions de filtre, vecteurs creux,
recherche hybride), exposez-la via une échappatoire sur le driver -
ne surchargez pas le trait.

## Suivant

- [Déploiement](deployment.md) - la recommandation
  MariaDB-comme-défaut-en-production, en contexte
- [Base de données](database.md) - configuration SeaORM
  multi-driver, y compris MariaDB comme backend relationnel aux
  côtés des vecteurs
- [Variables d'environnement](env-vars.md) - `QDRANT_URL`,
  `PINECONE_API_KEY`, `MARIADB_URL` et les autres contrats d'env des
  drivers
- [Cache](cache.md) - façade sœur avec la même forme de trait-driver
- [Carte de parité Laravel](parity.md) - où se situe la recherche
  vectorielle par rapport à Scout
