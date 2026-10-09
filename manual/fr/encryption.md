# Chiffrement

Suprnova livre un chiffrement au niveau applicatif sous la forme
d'une façade globale au processus nommée `Crypt`. Elle chiffre des
chaînes ou toute valeur `Serialize` sous AES-256-GCM, avec pour clé
votre `APP_KEY`. Utilisez-la chaque fois que vous devez placer
quelque chose de sensible dans un stockage auquel vous ne faites pas
entièrement confiance - une colonne, un cookie, un curseur de
pagination - et devez le relire intact plus tard.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

Le framework lui-même utilise `Crypt` pour les cookies chiffrés, les
curseurs de pagination chiffrés, les secrets 2FA, les codes de
récupération, et les casts Eloquent `AsEncrypted*`. La même façade est
disponible pour votre code sans câblage supplémentaire une fois
`APP_KEY` configuré (voir
[configuration.md](configuration.md#the-env-file)).

## Le format réseau

`encrypt_string` et `encrypt` retournent tous deux du base64
compatible URL (sans padding) sur `nonce || ciphertext_with_tag` :

```
base64url( [nonce aléatoire de 12 octets] || [texte chiffré] || [tag GCM de 16 octets] )
```

Chaque appel échantillonne un nonce frais de 12 octets depuis le RNG
de l'OS, si bien que deux chiffrements du même texte en clair sous la
même clé produisent des textes chiffrés distincts. Il n'y a pas
d'oracle de padding pour faire fuiter une information de longueur
au-delà du texte en clair lui-même.

La sortie peut être placée sans risque dans des chaînes de requête
URL, des corps JSON, des en-têtes, et des cookies sans encodage
supplémentaire. Une valeur réseau valide minimale fait 28 octets (12
de nonce + 16 de tag) - tout ce qui est plus court est rejeté en
amont.

## `APP_KEY` - le seul secret qui compte

Suprnova lit une unique clé symétrique de 32 octets depuis la
variable d'environnement `APP_KEY`. Le format attendu est du base64
compatible URL, sans padding, se décodant en exactement 32 octets (43
caractères base64) :

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Générez-en une avec la CLI :

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Ou redirigez-la directement dans l'environnement :

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### Validation au démarrage - échec fermé

`Server::from_config` valide `APP_KEY` **à chaque démarrage**, pas
seulement au premier. Les règles :

| Environnement | `APP_KEY` non défini | `APP_KEY` malformé |
|---|---|---|
| `local`, `development`, `testing` | Clé transitoire générée, avertissement dans les logs | Erreur fatale - le démarrage échoue |
| `staging`, `production`, tout le reste | Erreur fatale - le démarrage échoue | Erreur fatale - le démarrage échoue |

Une clé malformée est **toujours** une erreur fatale, même en
`local` - mieux vaut échouer au démarrage que masquer une faute de
frappe. Une valeur d'environnement `Custom` que le framework ne
reconnaît pas (par exemple `APP_ENV=k8s`) est traitée comme
production-like : pas d'`APP_KEY`, pas de démarrage.

Le diagnostic indique le correctif :

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - séparation de domaine via l'AAD

Chaque appel `Crypt::*` prend un `CryptPurpose`. La variante se
traduit par une étiquette d'octets stable, liée dans le tag
d'authentification AES-GCM en tant que données associées (AAD) :

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

L'étiquette n'est **pas** stockée dans la valeur réseau. GCM mélange
l'AAD dans le tag d'authentification sans l'inclure dans le texte
chiffré, si bien que :

- Le format réseau reste inchangé - toujours
  `base64(nonce || ciphertext || tag)`.
- Une valeur réseau produite sous `CryptPurpose::Cookie` est
  **rejetée** par tout appel de déchiffrement qui fournit un usage
  différent. La vérification du tag GCM échoue avant que la moindre
  analyse post-déchiffrement ne s'exécute.
- Ajouter une nouvelle surface (un futur chiffrement de payload de
  file d'attente, un en-tête de fichier chiffré) signifie ajouter une
  nouvelle variante - pas changer le format réseau.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// Même clé, même valeur réseau, usage différent - échoue.
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// Même usage - réussit.
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### Pourquoi Suprnova diverge

Le `Crypt::encryptString` de Laravel ne prend pas d'usage. L'unique
`APP_KEY` est réutilisée à travers les cookies, les URL signées, les
tokens d'expiration signés, et tout appel utilisateur à
`Crypt::encrypt`, sans séparation de domaine au niveau de la couche
crypto. Si deux surfaces se trouvent accepter un texte chiffré de la
même forme de texte en clair, une valeur produite pour une surface
peut être rejouée dans l'autre.

Suprnova réutilise la même `APP_KEY` pour la même raison - les
opérateurs gèrent un seul secret - mais lie chaque surface à sa
propre étiquette AAD. Le rejeu de texte chiffré entre surfaces est
rejeté à la vérification du tag GCM, avant que la moindre analyse ne
s'exécute. Le coût pour l'appelant est un paramètre enum
supplémentaire ; le gain est une propriété que le format réseau seul
ne peut pas rompre.

Le suffixe `:v1` sur chaque étiquette est réservé pour une future
rotation par surface : faire passer `suprnova:cookie:v1` à
`suprnova:cookie:v2` invalide **uniquement** l'ancien texte chiffré
des cookies - laisse intacts les curseurs, les secrets 2FA, et les
colonnes de cast.

## AAD liée au nom du cookie (v2)

Les cookies chiffrés utilisent une seconde génération d'AAD lorsque
l'appelant connaît le nom logique du cookie.
`Cookie::encrypted("suprnova_session", value)` lie
`suprnova:cookie:v2:suprnova_session` dans le tag GCM, et
`Cookie::read_encrypted_for("suprnova_session", wire)` fournit le même
contexte au retour :

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

Le nom lié est logique, pas rendu. Un préfixe de nom réseau `__Host-` ou
`__Secure-` ajouté ultérieurement ne modifie donc pas l'AAD et ne déconnecte
pas les utilisateurs. Le préfixe relève du navigateur et de l'en-tête ; le
nom du cookie est le domaine cryptographique.

### La fenêtre de compatibilité

Le format réseau est inchangé et sans version : il ne porte toujours que le
nonce, le texte chiffré et le tag d'authentification. Aucun octet de version
ne permet au lecteur de choisir une branche. `decrypt_string_for` utilise un
essai de déchiffrement aveugle de même forme que la rotation de clé : il
essaie l'AAD v2 contextuelle sur tout le trousseau de clés, puis l'AAD v1 non
contextuelle sur tout le trousseau. Cela maintient lisibles les cookies écrits
avant la liaison du nom tandis que la rotation de `APP_KEY` est aussi en
cours.

La fenêtre préserve l'ancienne faiblesse de rejeu pendant toute sa durée. Un
cookie v1 d'un emplacement de cookie peut encore être rejoué dans un autre
tant que le repli non contextuel existe ; le bénéfice de la liaison au nom
commence lorsque ce repli est supprimé dans 1.4.0. Rien ne retire
automatiquement le repli : `Crypt::encrypt_string(CryptPurpose::Cookie, ...)`
continue d'émettre v1, et le point d'entrée non contextuel est remplacé avec
une suppression prévue pour 1.4.0. Basculez les écritures de cookies vers
`Cookie::encrypted` et les lectures vers `read_encrypted_for` avant cette
échéance.

La fenêtre a un coût mesurable. Un déchiffrement de cookie en échec paie deux
passes d'essai sur le trousseau. Le middleware de session effectue deux
lectures chiffrées par requête lorsqu'un cookie de session et un cookie « se
souvenir de moi » sont tous deux présents ; une requête anonyme avec un
cookie « se souvenir de moi » obsolète paie donc `2 × (1 + N)` deux fois, où
`N` est le nombre de clés précédentes.

### Lire `DecryptOrigin`

`Crypt::decrypt_string_for_inner` retourne un `DecryptOrigin` à deux axes
indépendants :

- `origin.key = KeyOrigin::Previous(index)` signifie que la valeur dépend
  encore de `APP_KEY_PREVIOUS[index]`. Rechiffrez la valeur sous la clé
  courante et ne retirez cette clé précédente qu'après la disparition de la
  traîne de rotation.
- `origin.aad = AadVersion::Legacy` signifie que la valeur a utilisé le repli
  v1 non contextuel. Pour un cookie, réémettez-le via l'API liée au nom ; le
  repli doit être supprimé dans 1.4.0.

Les deux axes peuvent être obsolètes ensemble. Le lecteur public journalise
les avertissements correspondants sans inclure de texte en clair ou de texte
chiffré. Traitez l'avertissement de clé comme une tâche de nettoyage de
rotation et l'avertissement d'AAD comme une tâche de migration ; une
correspondance sur un axe ne doit pas masquer l'autre.

## Les deux paires chiffrer / déchiffrer

Il y a deux formes pour deux cas d'usage.

### Chaînes - `encrypt_string` / `decrypt_string`

Pour les chaînes UTF-8 :

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

Le chemin de déchiffrement retourne un `String` - des octets non
UTF-8 (qu'une exécution normale de chiffrement ne peut pas produire,
mais qu'une valeur réseau corrompue ou fournie par un attaquant le
pourrait) remontent comme un `FrameworkError::Internal` explicite.

### N'importe quoi de `Serialize` - `encrypt` / `decrypt`

Pour les valeurs structurées, encodage JSON puis chiffrement en un
seul appel :

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

Le format réseau est le même - base64 sur `nonce || ciphertext ||
tag` - la seule différence est que le texte en clair est constitué
des octets `serde_json` de `value` plutôt que de l'UTF-8 d'une
chaîne. Utilisez ceci pour n'importe quelle forme d'enregistrement :
un blob de config, un payload de session, un tuple d'argument de file
d'attente.

### `appears_encrypted` - vérification de forme, pas de falsification

Pour un middleware qui doit ignorer les valeurs déjà chiffrées lors
du passage de sortie (à l'image du comportement `EncryptCookies` de
Laravel), `Crypt::appears_encrypted` effectue une vérification
heuristique peu coûteuse :

```rust
if Crypt::appears_encrypted(cookie_value) {
    // laisse passer - déjà enveloppé
} else {
    // chiffre avant l'envoi
}
```

Elle retourne `true` quand l'entrée se décode comme du base64
compatible URL et que la longueur décodée fait au moins 28 octets
(nonce + tag). Elle n'appelle jamais AES-GCM, si bien qu'elle **ne
peut pas** distinguer un texte chiffré valide d'octets aléatoires de
la bonne forme. Les appelants qui ont besoin d'authentification
doivent appeler `decrypt_string` / `decrypt` et gérer l'erreur.

## Rotation des clés - le trousseau

Suprnova prend en charge une rotation sans interruption de service
grâce à un *trousseau* de clés : une clé courante (utilisée pour
chaque nouveau chiffrement) plus une liste ordonnée de clés
précédentes (essayées comme repli au déchiffrement). Vous faites
tourner `APP_KEY` sans avoir à rechiffrer chaque colonne en
lock-step.

`APP_KEY_PREVIOUS` est le nom canonique de Suprnova. `APP_PREVIOUS_KEYS` est accepté comme alias compatible Laravel. Si les deux variables sont définies, `APP_KEY_PREVIOUS` l'emporte. Quand leurs valeurs épurées diffèrent, l'amorçage journalise un avertissement et ignore `APP_PREVIOUS_KEYS`.

Positionnez `APP_KEY_PREVIOUS` à une liste de clés base64 séparées
par des virgules, de la plus ancienne à la plus récente :

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# Ou pour une rotation en plusieurs étapes (plus ancienne → plus récente) :
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

Le chiffrement utilise **toujours** la clé courante. Le déchiffrement
essaie d'abord la clé courante ; si cela échoue, chaque clé
précédente est essayée dans l'ordre. Sur une réussite via une clé
précédente, `Crypt` émet un `tracing::warn!` :

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

La ligne de log exclut délibérément à la fois le texte en clair et le
texte chiffré - seul le fait-même de la rotation, plus une piste
d'action, y voyage. Les opérateurs qui lancent une recherche de log
sur `APP_KEY_PREVIOUS` retombent sur chaque colonne encore dépendante
d'une ancienne clé.

### Le plafond - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` est plafonné à 8 entrées. Une chaîne de rotation
réaliste compte 1 à 3 entrées (une rotation en cours, peut-être une
rotation précédente restée bloquée que l'opérateur n'a pas
nettoyée) ; 8 laisse une marge généreuse. Au-delà du plafond, le
démarrage **échoue explicitement** avec un diagnostic qui nomme à la
fois le compte et le plafond :

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

Une troncature silencieuse laisserait tomber une clé dont
l'opérateur pourrait encore dépendre, laissant des colonnes
indéchiffrables sans aucun diagnostic. Le plafond strict est
intentionnel.

Les entrées vides sont tolérées :
`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` s'analyse en deux clés réelles.
Une entrée malformée (faute de frappe, mauvaise longueur, base64
incorrect) est une erreur fatale - des secrets à demi tournés font
échouer le démarrage, plutôt que de laisser tomber silencieusement un
repli.

### Procédure de rotation

```bash
# 1. Produisez une nouvelle clé.
NEW=$(suprnova key:generate --show)

# 2. Déplacez la clé courante vers APP_KEY_PREVIOUS, installez la
#    nouvelle. Éditez votre .env ou votre gestionnaire de secrets :
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. Déployez. Les nouvelles écritures utilisent la nouvelle clé ; les
#    lignes existantes continuent de se déchiffrer via le repli sur
#    la clé précédente. Les logs identifient les colonnes encore sur
#    l'ancienne clé.

# 4. Lancez une passe de rechiffrement. Pour chaque modèle avec des
#    casts chiffrés :
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` utilise toujours la clé courante, si bien
#    qu'un chargement-puis-sauvegarde no-op migre la ligne.

# 5. Une fois que les avertissements ne réapparaissent plus dans les
#    logs, retirez APP_KEY_PREVIOUS et redéployez.
```

Toute la procédure se fait en ligne - à aucun moment il n'y a de
fenêtre où les nouvelles requêtes échouent.

### Observer le trousseau

Pour les tableaux de bord d'exploitation ou les vérifications de
santé :

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

Les octets de la clé eux-mêmes ne sont jamais accessibles depuis
l'API publique. L'impl `Debug` d'`EncryptionKey` affiche
`"[REDACTED]"`, et il n'existe aucun accesseur qui expose une clé
brute hors de la crate.

## Intégration Eloquent - les casts `AsEncrypted*`

Le chiffrement au niveau applicatif est le plus utile à la frontière
de la colonne. La famille de casts `AsEncrypted*` enveloppe
`Crypt::encrypt_string`, si bien que les champs de votre modèle
restent du texte en clair typé à l'exécution et du texte chiffré au
repos :

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| Cast | Type à l'exécution | Forme de stockage |
|---|---|---|
| `AsEncrypted` | `String` | chaîne chiffrée |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → chaîne chiffrée |
| `AsEncryptedObject<T>` | `T` | JSON → chaîne chiffrée |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → chaîne chiffrée |

Les quatre passent par `CryptPurpose::Cast`. Une valeur réseau
produite par un cast chiffré est rejetée par tout code qui essaie de
la déchiffrer comme un cookie ou un curseur - même si `APP_KEY` est
la même, l'étiquette AAD diffère.

Pour la surface de cast complète, la table des modes d'échec, et les
recettes de rechiffrement, voir [eloquent.md](eloquent.md). La
mécanique de chiffrement est la même que la façade ci-dessus - le
cast est du sucre qui exécute
`Crypt::encrypt_string(CryptPurpose::Cast, …)` à la frontière du
stockage.

### Chiffrement vs hachage - choisir le bon outil

`AsEncrypted` est **réversible**. Le texte en clair peut être
récupéré avec `APP_KEY`. Utilisez-le pour des données que votre
application doit relire : des tokens API que vous affichez sur une
page de réglages, des secrets tiers que vous transmettez à des
services amont, des adresses vers lesquelles vous expédiez des
commandes.

Pour des données que votre application n'a jamais besoin que de
*vérifier* - mots de passe, préfixes de clé API que vous comparez
contre des tokens entrants - utilisez plutôt un hash. Les hash sont à
sens unique : il n'y a pas de texte en clair à faire fuiter même si
`APP_KEY` est compromise. Voir [hashing.md](hashing.md) pour la
façade Bcrypt / Argon2id et le cast `AsHashed`.

## Où `Crypt` est utilisé ailleurs dans le framework

Vous n'avez rien à faire pour activer cela - c'est câblé
automatiquement une fois `APP_KEY` configuré.

- **Cookies chiffrés** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted(...)` utilisent `CryptPurpose::Cookie`. Le
  cookie de session, le cookie « se souvenir de moi », et le cookie de
  contournement du mode maintenance en dépendent tous. Voir
  [responses.md](responses.md) et [session.md](session.md).
- **Pagination par curseur** - `CursorPaginator` encode le curseur
  sous `CryptPurpose::Cursor`, si bien que la valeur réseau
  `?cursor=…` ne peut être ni falsifiée ni rejouée entre surfaces.
  Voir [eloquent.md](eloquent.md#cursor-pagination).
- **Secrets 2FA** - le secret TOTP base32 chiffré sur
  `two_factor_authentications.secret` utilise
  `CryptPurpose::TwoFactorSecret` ; les codes de récupération
  utilisent `CryptPurpose::TwoFactorRecovery`. Des usages distincts
  empêchent le rejeu de texte chiffré entre colonnes d'une même
  ligne. Voir [auth-flows.md](auth-flows.md).
- **Signature dérivée par HMAC** - les URL signées et les tokens de
  réinitialisation de mot de passe dérivent une clé HMAC depuis
  `APP_KEY` plutôt que de chiffrer sous celle-ci. Les octets bruts de
  la clé ne sont pas exportés ; la dérivation vit à l'intérieur du
  framework. Voir [routing.md](routing.md#signed-urls).

## Tests avec `Crypt`

La façade `Crypt` est adossée à un `OnceLock`, si bien que le premier
installeur dans un binaire de test gagne. Les helpers de test gèrent
le code répétitif :

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // idempotent - peut être appelé sans risque depuis chaque test

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

La clé de test est déterministe, si bien que les tests peuvent déchiffrer des fixtures stables et exercer la rotation contre une clé connue. Les chaînes de texte chiffré ne doivent pas être comparées pour l'égalité entre appels ou exécutions : chaque chiffrement utilise toujours un nonce aléatoire frais.


Pour les tests de rotation, installez un trousseau directement et
produisez du texte chiffré historique avec `_test_encrypt_with` :

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// Simule une valeur écrite quand `old` était la clé courante.
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// Le trousseau courant la déchiffre via le repli sur la clé
// précédente, en émettant la ligne d'avertissement de rotation.
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

Les deux helpers sont exclus de la compilation des binaires de
production quand la feature `testing` est désactivée
(`default-features = false`).

## Modes d'échec - à quoi ressemblent les erreurs

Chaque appel `Crypt::*` faillible retourne `Result<_,
FrameworkError>`. Les cinq erreurs que vous pouvez rencontrer :

| Cause | Où | Se manifeste comme |
|---|---|---|
| `Crypt` non initialisée | Tout appel avant le démarrage | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| La valeur réseau n'est pas un base64 valide | `decrypt_string`, `decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| Valeur réseau trop courte (< 28 octets) | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| La vérification du tag échoue - mauvaise clé, mauvais AAD, octets altérés | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| L'encodage / décodage JSON échoue | `encrypt`, `decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

Il n'y a pas de repli silencieux vers des données aléatoires. Une
mauvaise clé contre un texte chiffré existant est toujours une
erreur fatale, à la fois au niveau de la façade et au niveau du cast.
Cela correspond au comportement de l'`Encrypter` de Laravel et c'est
la propriété qui rend la rotation sûre : une colonne oubliée
remonterait immédiatement, plutôt que de retourner un texte en clair
plausible mais erroné.

Quand une clé précédente déchiffre avec succès une valeur réseau,
l'appel retourne toujours `Ok(...)` - mais la ligne `tracing::warn!`
se déclenche en parallèle, si bien qu'une alerte pilotée par les logs
attrape la traîne de la rotation avant que `APP_KEY_PREVIOUS` ne soit
retiré.

## Suivant

- [configuration.md](configuration.md) - `APP_KEY`, `APP_ENV`, et le
  reste de l'environnement de démarrage.
- [eloquent.md](eloquent.md) - les casts `AsEncrypted*`, la table de
  cast complète, et la procédure de rotation pour les colonnes de
  modèle.
- [hashing.md](hashing.md) - alternative à sens unique quand vous
  avez besoin de *vérifier*, pas de *récupérer* ; les façades bcrypt
  et Argon2id plus `AsHashed`.
- [auth-flows.md](auth-flows.md) - stockage des secrets 2FA et des
  codes de récupération, qui s'appuient sur `Crypt` sous leurs
  propres usages.
- [session.md](session.md) - le cookie de session, chiffré et signé
  par `Crypt` via `CryptPurpose::Cookie`.
