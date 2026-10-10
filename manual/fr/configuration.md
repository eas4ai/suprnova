# Configuration

Suprnova lit la configuration à partir de variables d'environnement (chargées
depuis `.env` en développement, depuis l'environnement du processus en
production) et les expose à votre code sous deux formes :

1. **Accès direct aux env** - `env::env`, `env_required`, `env_optional`
   pour des lectures ponctuelles
2. **Structures config typées** - `Config::register` / `Config::get` pour
   tout ce que vous lisez plus d'une fois, avec typage fort

Le framework lit lui-même une poignée de variables d'env (`APP_KEY`,
`APP_ENV`, `DATABASE_URL`, etc.) ; les autres sont les vôtres.

## Le fichier `.env`

`suprnova new` écrit un fichier `.env` de démarrage avec les valeurs dont votre
application a besoin pour s'amorcer :

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # pages d'erreur détaillées + logs détaillés
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Base de données - SQLite par défaut ; remplacez par postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # mettre true en production (HTTPS uniquement)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# E-mail - par défaut utilise le driver `log` (écrit les e-mails sortants
# dans le log de traçage, bon pour le dev). Définissez MAIL_DRIVER à l'une de
# ces valeurs pour la production : smtp / ses / mailgun / postmark / sendgrid /
# resend / log / memory
MAIL_DRIVER=log
# Identifiants SMTP (lus uniquement quand MAIL_DRIVER=smtp) :
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Si laissé vide, il dérive des identifiants ci-dessus
# - starttls s'ils sont présents, none sinon. La production refuse de s'amorcer
# sans chiffrement ; voir le chapitre E-mail.
MAIL_SMTP_ENCRYPTION=
```

Un fichier `.env.example` complémentaire expose les mêmes clés avec des valeurs
de remplacement - commitez-le ; ne commitez pas `.env`. Le `.gitignore` par
défaut exclut déjà `.env`.

## Fonctionnement du chargement `.env`

Au démarrage, le framework :

1. Détecte l'environnement à partir de `APP_ENV` (insensible à la casse,
   `prod`/`dev`/`stage`/`stg`/`test` sont aussi reconnus).
2. Charge `.env` à partir de la racine du projet.
3. Si un fichier par environnement existe (`.env.staging`, `.env.production`),
   le charge par-dessus - ses valeurs remplacent celles de `.env`.
4. Les vrais variables d'environnement du processus remplacent les deux (c'est
   sur quoi l'orchestration de conteneurs s'appuie).

L'ordre en une ligne : **env du processus > `.env.<environment>` > `.env`**.

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

Lors d'une exécution CI avec `APP_ENV=testing`, le framework charge `.env.testing`
par-dessus `.env` pour que vous puissiez remplacer les URL de base de données et
désactiver les drivers de mail sans toucher au `.env` de dev.

## Accès direct aux env

Pour des lectures ponctuelles de chaînes, nombres, booléens - toute chose
implémentant `std::str::FromStr` - utilisez la famille `env::*` :

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // avec valeur par défaut
let url: String = env_required("APP_URL");                   // paniques si manquante - démarrage uniquement
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // None si manquante
```

- `env(key, default)` - lecture avec coercition de type et valeur de repli
- `env_required(key)` - paniques si la clé manque ou ne peut pas être parsée.
  À utiliser uniquement au démarrage (dans `bootstrap()` ou `config::register()`)
  où une valeur manquante devrait arrêter immédiatement le processus
- `env_optional(key)` - retourne `Option<T>` ; `None` si manquante ou non parsable

Chaque clé unique est aussi loggée une fois à la première lecture, de sorte que
vous pouvez auditer exactement quelles variables d'env votre application
touche.

## Structures config typées

Pour tout ce que votre application lit plus d'une fois, définissez une structure
typée et enregistrez-la. Le motif est :

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

Puis lisez-la n'importe où en une seule ligne :

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

Le registre est indexé par `TypeId`, de sorte que chaque structure est stockée
une fois. Appeler `Config::register` à nouveau avec le même type remplace
l'entrée précédente - pratique pour les tests.

### Filage de l'enregistrement dans votre application

Le `cmd/main.rs` du scaffolding inclut une étape `.config(…)` dans le pipeline
d'amorçage fluide :

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← ceci appelle votre enregistrement
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` délègue généralement à chaque module de section :

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### Désérialisation de structures entières depuis env

Pour les configurations plus grandes, vous pouvez désérialiser directement
à partir des variables d'env via `serde`. Suprnova expose deux assistants :

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// Lit SERVER_HOST / SERVER_PORT depuis l'environnement
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - désérialise à partir de toutes les variables d'env du processus
- `Config::resolve_prefixed::<T>("PREFIX_")` - désérialise uniquement les
  variables avec le préfixe donné (le préfixe est supprimé avant la
  désérialisation)

Les deux retournent `Result<T, FrameworkError>` de sorte qu'un champ requis
manquant remonte en tant que `FrameworkError::Internal` portant le diagnostic
envy au lieu d'une panique.

## Configuration spécifique à l'environnement

L'énumération `Environment` couvre l'ensemble standard :

| Variante | Valeurs de `APP_ENV` reconnues |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod` |
| `Testing` | `testing`, `test` |
| `Custom(String)` | tout le reste (préserve votre casse, utilisé pour la recherche `.env.<custom>`) |

Branches communes :

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // cookies stricts, driver de mail réel, etc.
}

if Config::is_debug() {
    // pages d'erreur détaillées, logging des requêtes
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* chemin dev/test */ },
}
```

`is_debug()` retourne `true` quand `APP_DEBUG=true` est défini explicitement,
ou - quand `APP_DEBUG` est non défini - quand l'environnement détecté est
`Local`, `Development`, ou `Testing`. Production, staging, et tout
environnement personnalisé non reconnu retournent `false` par défaut. Gardez-le
désactivé en production ; il contrôle le détail des pages d'erreur et quelques
valeurs par défaut internes.

### `APP_KEY` est requis en non-développement

En production (tout `APP_ENV` autre que `local`/`development`/
`testing`), Suprnova exige que `APP_KEY` soit défini à une chaîne base64
compatible URL de 32 octets valide. L'amorçage sans elle échoue de façon
fermée avec un message d'erreur descriptif - il n'y a pas de fallback
silencieux.

Si vous n'avez pas encore d'`APP_KEY` :

```bash
suprnova key:generate          # affiche la clé avec un conseil vous rappelant de l'ajouter à .env
suprnova key:generate --show   # affiche uniquement la clé, adaptée à `APP_KEY=$(suprnova key:generate --show)`
```

Aucune forme ne modifie `.env` pour vous - copiez vous-même la clé affichée
dans votre `.env` (ou votre gestionnaire de secrets).

Pour la rotation de clé (où les anciennes données chiffrées doivent toujours
se déchiffrer pendant la fenêtre de migration), voir
[Chiffrement](encryption.md#key-rotation).

## Configuration dans les tests

Dans les tests, enregistrez la configuration dans l'initialisation du test
plutôt que de compter sur `.env` :

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … votre test
}
```

L'attribut `#[suprnova_test]` configure aussi un état de conteneur isolé de
sorte que les tests concurrents ne voient pas les liaisons les uns des autres -
voir [Tests](testing.md).

## Variables d'env communes que Suprnova lit

Une liste non-exhaustive - ce sont les variables que le framework lui-même
consulte. Votre application en lit d'autres par-dessus.

| Variable | Défaut | Ce qu'elle fait |
|---|---|---|
| `APP_NAME` | `"app"` | Loggée au démarrage, utilisée dans certains messages d'erreur par défaut |
| `APP_ENV` | `local` | Pilote `Environment::detect` et la recherche `.env.<suffix>` |
| `APP_DEBUG` | conscient de l'env (`false` en production) | Pages d'erreur détaillées + logs supplémentaires |
| `APP_URL` | `http://localhost:8765` | URL de base pour la génération d'URLs absolues, URLs signées |
| `APP_KEY` | aucune (requise en prod) | Clé AES-256 pour `Crypt`, sessions, curseurs |
| `APP_KEY_PREVIOUS` | aucune | Clés précédentes séparées par des virgules pour la rotation (max 8) |
| `SERVER_HOST` | `127.0.0.1` | Adresse de liaison |
| `SERVER_PORT` | `8765` | Port de liaison |
| `DATABASE_URL` | aucune | Requise si votre application utilise la base de données |
| `DB_MAX_CONNECTIONS` | `10` | Max du pool sqlx |
| `DB_MIN_CONNECTIONS` | `1` | Min du pool sqlx |
| `DB_CONNECT_TIMEOUT` | `30` (secondes) | Timeout de connexion du pool sqlx |
| `SESSION_LIFETIME` | `120` (minutes) | Expiration de session |
| `SESSION_TOUCH_INTERVAL` | `300` (secondes) | Cadence d'écriture d'expiration glissante minimale |
| `SESSION_GC_INTERVAL` | `3600` (secondes) | Cadence de nettoyage de session expirée supervisée |
| `SESSION_COOKIE` | `suprnova_session` | Nom du cookie |
| `SESSION_SECURE` | `true` | Définit le flag `Secure` du cookie. Remplacez par `false` pour le développement en HTTP local. |
| `SESSION_SAME_SITE` | `Lax` | `Strict`, `Lax`, ou `None` |
| `MAIL_DRIVER` | `log` | L'une de : `smtp`, `ses`, `mailgun`, `postmark`, `sendgrid`, `resend`, `log`, `memory` |
| `CACHE_DRIVER` | `memory` | L'une de : `memory`, `redis`, `database` |
| `QUEUE_DRIVER` | `memory` | L'une de : `memory`, `redis`, `database` (les valeurs inconnues avertissent et retournent à `memory`) |
| `RATE_LIMIT_DRIVER` | `memory` | L'une de : `memory`, `redis` |
| `LOG_FORMAT` | conscient de l'env (`pretty` en dev/local, `json` en production) | `pretty` ou `json` |
| `LOG_LEVEL` | `info` | L'une de : `error`, `warn`, `info`, `debug`, `trace` |

La liste complète auditée se trouve dans [Variables d'environnement](env-vars.md).

## Suivant

- [Amorçage de l'application](bootstrap.md) - où l'enregistrement de config
  typée est appelé depuis
- [Conteneur de service](container.md) - comment la config enregistrée est lue
  aux côtés des services liés
- [Variables d'environnement](env-vars.md) - la liste de référence complète
- [Déploiement](deployment.md) - mise en place de l'env en production
