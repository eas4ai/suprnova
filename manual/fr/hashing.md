# Hachage

Le module `suprnova::hashing` est la surface de hachage des mots de
passe du framework, avec trois drivers de premier rang - **bcrypt**
(par défaut, comme Laravel), **Argon2i** (memory-hard, résistant aux
attaques par canal auxiliaire) et **Argon2id** (recommandation OWASP
2024). Utilisez-le pour stocker les mots de passe utilisateur, hacher
les tokens de vérification « se souvenir de moi », ou partout où une
fonction à sens unique est la bonne primitive. Le choix du driver est
piloté par l'environnement, et la façade est consciente de
l'algorithme de bout en bout (`info`, `is_hashed`, `needs_rehash`,
`verify`), si bien qu'un hash bcrypt stocké se vérifie toujours après
avoir basculé `HASH_DRIVER=argon2id`.

## Vue d'ensemble

```rust
use suprnova::hashing;

// Async (à privilégier dans les handlers de requête Tokio - exécute
// le hash, coûteux en CPU, sur spawn_blocking pour que le thread de
// travail reste libre) :
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// Sync (tests, outils CLI, contextes non async) :
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

La façade en fonctions libres lit le driver actif depuis
`HASH_DRIVER` (ou se replie sur bcrypt). Pour des appels à driver
explicite, construisez directement le type du driver et passez-le à
`hash_with` / `verify_with` / `needs_rehash_with`.

## Configuration

| Variable | Description | Défaut | Plage |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | Algorithme actif | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Facteur de coût bcrypt | `12` | `4..=31` (bcrypt uniquement) |
| `HASH_MEMORY` | Coût mémoire Argon en KiB | `65536` (64 Mio) | `>= 8` (argon uniquement) |
| `HASH_TIME` | Itérations temporelles Argon | `4` | `>= 1` (argon uniquement) |
| `HASH_THREADS` | Parallélisme / voies Argon | `1` | `>= 1` (argon uniquement) |
| `HASH_VERIFY` | Si vrai, `verify()` rejette les hash inter-algorithmes | `false` | `true` / `false` |

Une mauvaise configuration (valeur incorrecte, paramètre hors
plage) remonte sous la forme d'un `FrameworkError::param` au premier
appel à `hash` / `verify` / `needs_rehash` - pas sous la forme d'une
valeur par défaut silencieuse.

### Exemple de `.env` pour argon2id

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### Pourquoi les valeurs Argon2 par défaut de Suprnova sont plus robustes que celles de Laravel

| Paramètre | Défaut Laravel | Défaut Suprnova | Source |
|-------|-----------------|------------------|--------|
| Mémoire | 1 024 KiB (1 Mio) | 65 536 KiB (64 Mio) | OWASP 2024 |
| Temps | 2 itérations | 4 itérations | OWASP 2024 |
| Threads | 2 | 1 | OWASP 2024 / aligné sur libsodium |

Les valeurs par défaut de Laravel supposent le modèle PHP d'un
processus par requête - un worker ne peut consacrer qu'un temps
limité à chaque hash de mot de passe avant que la machine ne sature.
Le `spawn_blocking` de Tokio permet à Suprnova de confier le hash à un
pool de threads bloquants sans geler la boucle de requêtes, si bien
que les chiffres OWASP 2024 sont réalistes sur du matériel de
production réel.

## Drivers

### Bcrypt (par défaut)

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

Bcrypt impose un **plafond de bloc de 72 octets** sur l'entrée du mot
de passe - la primitive sous-jacente tronque silencieusement les
entrées plus longues, ce qui signifie que deux passphrases distinctes
partageant leurs 72 premiers octets se hachent vers la même valeur.
Suprnova rejette en amont (le chemin bcrypt du framework échoue sur
`hash()` et retourne `Ok(false)` sur `verify()` pour les mots de passe
trop longs, ce qui garde uniforme la réponse « identifiants invalides
» du flux d'authentification). Argon2 n'a pas ce plafond.

Le plafond bcrypt est exposé sous
`suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES` (71 - la limite
utilisable après le terminateur nul de bcrypt).

### Argon2id (recommandation OWASP 2024)

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 Mio
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// Argon2 accepte des passphrases de longueur arbitraire - le
// plafond de 72 octets de bcrypt ne s'applique pas.
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

Même forme qu'Argon2id ; `Argon2iHasher::new(opts)`. Utilisez
Argon2id pour les nouveaux projets - Argon2i est prise en charge par
souci de parité, mais Argon2id est la recommandation moderne.

## Bcrypt avec un coût explicite (`hash_with_cost`)

`hash_with_cost(password, cost)` et `hash_with_cost_async(password,
cost)` produisent un hash bcrypt à un facteur de coût fourni par
l'appelant, sans tenir compte de `HASH_DRIVER`. Utilisez-les quand une
politique ou une config par tenant fait circuler un coût jusqu'au
site d'appel plutôt que dans l'env du processus - par exemple, une
classe de comptes à haute sécurité qui utilise le coût 14 tandis que
le reste de l'application tourne au défaut 12.

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// Sync - tests, outils CLI.
let h = hash_with_cost("my_password", 14)?;

// Async - dans les handlers de requête Tokio.
let h = hash_with_cost_async("my_password", 14).await?;
```

Les deux points d'entrée rejettent un `cost` hors de
`MIN_BCRYPT_COST..=MAX_BCRYPT_COST` (`4..=31`) avec
`FrameworkError::param`, reflétant la validation côté env de
`HASH_ROUNDS` :

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

Cette vérification de bornes compte, car chaque incrément de coût
double le temps CPU. À un coût de 31, un seul hash bcrypt prend des
heures sur du matériel grand public - la vérification de bornes à
l'intérieur du framework empêche une faute de frappe de politique ou
de config d'épingler accidentellement un thread de travail pour le
reste de la journée. La variante async passe par `spawn_blocking`, si
bien que même un coût légitimement élevé ne gèle pas la boucle de
requêtes.

## `needs_rehash` conscient de l'algorithme

`needs_rehash` retourne `true` quand le hash stocké devrait être
re-haché sous le driver actif. Trois cas sont couverts :

1. **Incompatibilité d'algorithme** - hash bcrypt stocké alors que
   `HASH_DRIVER=argon2id` (ou l'inverse). Déclenche une rotation à la
   prochaine vérification réussie.
2. **Faiblesse de paramètre** - coût bcrypt inférieur à `HASH_ROUNDS`,
   ou `m`/`t`/`p` d'argon inférieurs à
   `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS`.
3. **Variantes bcrypt historiques** - `$2a$`, `$2x$`, `$2y$` tournent
   vers la forme canonique `$2b$` même au coût configuré.

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // Conservez `fresh`. Motif Laravel standard « re-hachage à la
    // connexion réussie » ; fonctionne à travers les algorithmes.
}
```

Une entrée malformée retourne `true` - l'appelant fait naturellement
tourner tout ce qu'il ne peut pas analyser.

## Inspection du hash (`info` + `is_hashed`)

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// Vrai pour tout hash reconnu ; faux pour un texte en clair ou des
// données invalides.
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` est l'une des valeurs : `Bcrypt`, `Argon2i`,
`Argon2id`, `Argon2d` (reconnu mais jamais produit), `Unknown`.

`is_hashed` est ce que le cast eloquent `AsHashed` utilise pour éviter
de re-hacher une colonne déjà hachée - cela fonctionne à travers les
trois drivers, si bien que basculer `HASH_DRIVER` en cours de projet
ne provoque pas de boucle de hash-de-hash à la prochaine sauvegarde.

## Gate de vérification inter-algorithmes (`HASH_VERIFY`)

Par défaut, `verify()` vérifie le mot de passe contre le hash quel
que soit l'algorithme qui a produit ce hash - c'est ce qui permet aux
anciens hash bcrypt de continuer à se vérifier après avoir basculé
`HASH_DRIVER=argon2id` (afin de pouvoir les faire tourner à la
connexion). Positionnez `HASH_VERIFY=true` une fois que chaque
utilisateur a tourné, pour imposer strictement l'algorithme actif :

```env
HASH_VERIFY=true
```

Avec le gate activé, `verify()` retourne `Ok(false)` pour tout hash
dont l'algorithme diffère du driver actif - même forme que le
`RuntimeException` de Laravel, mais Suprnova retourne false plutôt que
de lever une exception, car l'appelant du flux d'authentification
attend de toute façon un `Result<bool>`.

## Async vs sync

Bcrypt au coût 12 (~250 ms) et Argon2id à memory=64 Mio (~80 ms) sont
tous deux volontairement coûteux en CPU - c'est tout le principe du
hachage lent. Appeler `hash` / `verify` en synchrone directement
depuis un handler de requête Tokio bloque le thread de travail
pendant toute la durée du hash, ce qui affame les autres requêtes sur
le même worker.

Utilisez les homologues `*_async` à l'intérieur des handlers `async
fn`. Ils enveloppent l'appel coûteux en CPU dans
`tokio::task::spawn_blocking`, si bien que le worker reste libre pour
d'autres requêtes :

```rust
// BON - à l'intérieur d'un handler async
let hashed = hashing::hash_async(&form.password).await?;

// MAUVAIS - bloque le worker pendant ~250 ms
let hashed = hashing::hash(&form.password)?;
```

Les variantes sync sont destinées aux tests, aux outils CLI, et aux
autres contextes non async où bloquer ne pose pas de problème.

## Intégration Eloquent : le cast `AsHashed`

Le cast eloquent `#[cast(AsHashed)]` hache un champ en clair à
l'écriture en utilisant le driver actif, et il est **idempotent à
travers tous les drivers** - sauvegarder un modèle dont la colonne
`password` contient déjà un hash reconnu (bcrypt ou argon) laisse la
valeur passer inchangée. Sans ce garde-fou, `User::find(id).await?
.save().await?` hacherait le hash existant à chaque sauvegarde,
cassant l'authentification.

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

La vérification d'idempotence utilise `hashing::is_hashed`, si bien
que basculer `HASH_DRIVER` en cours de projet est sûr - les anciens
hash bcrypt comme les hash argon2id récents sont reconnus et
ignorés à la resauvegarde.

## Utilisation avec `Auth::attempt`

`Auth::attempt(&credentials)` appelle
`UserProvider::validate_credentials`, qui appelle à son tour
`hashing::verify_async` contre le hash stocké de l'utilisateur.
`verify` se base sur l'algorithme du hash *stocké*, pas sur le driver
configuré - donc après avoir basculé `HASH_DRIVER=argon2id`, chaque
hash bcrypt existant se vérifie toujours, et `needs_rehash` retourne
`true`, si bien que le motif standard de rotation à la connexion fait
passer la base d'utilisateurs vers le nouvel algorithme une connexion
à la fois.

## Remplacer le driver dans les tests

`set_default_driver(Box<dyn Hasher>)` installe un driver de manière
programmatique pour les tests et les outils CLI embarqués qui
construisent le driver sans passer par `HASH_DRIVER`. C'est ponctuel -
le premier appel gagne, et un second appel retourne
`FrameworkError::internal` plutôt que de remplacer le driver en cours
de processus. Utilisez-le au démarrage de la suite, avant que le
moindre chemin de code ne résolve le défaut.

## Suivant

- [Authentification](authentication.md) - `Auth::attempt`, le trait
  de fournisseur d'utilisateurs, et comment le hachage s'intègre à la
  connexion
- [Flux d'authentification](auth-flows.md) - `PasswordReset::complete`
  fait tourner le hash de mot de passe stocké à travers le driver
  actif ; les tokens « se souvenir de moi » sont hachés avant stockage
  via `hash_async`
- [Eloquent](eloquent.md) - référence de `#[cast(AsHashed)]` et la
  surface de cast plus large
- [Chiffrement](encryption.md) - chiffrement authentifié bidirectionnel
  pour les données au repos ; le complément du hachage à sens unique
- [Modèle d'erreur](error-model.md) - à quoi ressemble
  `FrameworkError::param` quand une valeur de configuration de hachage
  est rejetée
