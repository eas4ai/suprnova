# Authentification

Suprnova livre un système d'authentification à la Laravel : une
façade `Auth` statique, des guards nommés résolus via un
`AuthManager`, des fournisseurs d'utilisateurs enfichables, un trait
`Authenticatable` sur votre modèle User, et un middleware pour
filtrer les routes. Un projet scaffoldé démarre avec un guard de
session (`web`) et un guard de token (`api`) déjà câblés contre votre
`User` typé, si bien que la connexion, l'inscription, et les routes
protégées fonctionnent le jour même où vous lancez `suprnova new`.

## Les pièces

| Type | Rôle |
|---|---|
| `Auth` | Façade du framework pour les guards, plus les opérations Magnetar de mot de passe, lien magique, passkey et OAuth |
| `MagnetarConfig` / `init_magnetar` | Composer puis installer atomiquement les moteurs par défaut de mot de passe, session, verrouillage, passkey et facteur |
| `Authenticatable` | Trait que votre modèle applicatif implémente ; expose `get_auth_identifier() -> String` et le hash de mot de passe |
| `UserProvider` | Trait qui récupère les utilisateurs applicatifs ; `EloquentUserProvider<M>` et `DatabaseUserProvider` sont livrés intégrés |
| `AuthManager` | Détient `AuthConfig` et les fournisseurs enregistrés ; résout les guards nommés à la demande |
| `SessionGuard` / `TokenGuard` | Contrats de guards avec état et sans état du framework |
| `BearerTokenMiddleware` | Résout les sessions bearer Magnetar en état d'authentification de requête du framework |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | Guards de route |
| `Credentials` | Map d'identifiants au format JSON, typiquement `{ "email", "password" }` |

Le code source des guards/fournisseurs du framework vit dans `framework/src/auth/`.
Les adaptateurs et façades d'hôte Magnetar vivent dans
`framework/src/magnetar_integration/` ; le crate du moteur vit dans
`crates/suprnova-magnetar/`. Les flux de plus haut niveau - vérification
d'e-mail, réinitialisation de mot de passe, verrouillage, et TOTP - vivent à
côté dans `framework/src/auth_flows/` et ont leur propre chapitre :
[Flux d'authentification](auth-flows.md). L'OAuth, Apple, et la connexion par
lien magique sont couverts par [OAuth et connexion sans mot de passe](oauth.md).

La piste dans les sources est courte :
`framework/src/auth/{guard,manager,contract,
authenticatable,middleware,session_guard,token_guard,eloquent_provider,
database_provider}.rs`. Les flux de plus haut niveau - vérification
d'e-mail, réinitialisation de mot de passe, limitation par force
brute, 2FA TOTP - vivent à côté dans `framework/src/auth_flows/` et
ont leur propre chapitre : Flux d'authentification.

## Modèle d'identifiant

L'id de l'utilisateur authentifié circule à travers Suprnova comme
un `String` de bout en bout - le stockage de session,
[`UserProvider::retrieve_by_id`], la table « se souvenir de moi »,
chaque événement d'auth. La surface canonique est
`Authenticatable::get_auth_identifier() -> String` (le
`getAuthIdentifier` de Laravel). Les clés primaires numériques se
transforment trivialement en chaîne ; les UUID, les ULID, et les id
opaques de fournisseur OAuth circulent sans changement.

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` est ce contre quoi les fournisseurs intégrés
vérifient un mot de passe en clair via `hashing::verify_async`.
Retournez `None` pour les utilisateurs qui s'authentifient par
d'autres moyens (OAuth, passkey, lien magique). La méthode
`auth_identifier_name() -> &'static str` (par défaut `"id"`) nomme la
colonne où vit l'id. La méthode de confort `auth_identifier() -> i64`
analyse par défaut l'id chaîne et se replie sur `0` pour les id non
numériques - Suprnova lui-même ne l'appelle jamais ; ne la surchargez
que pour les modèles à clé entière qui veulent éviter l'analyse.

### Pourquoi Suprnova diverge

Le `getAuthIdentifier()` de Laravel retourne `mixed`. PHP ne se
soucie pas de savoir si l'id est un int, une chaîne UUID, ou une clé
primaire typée en chaîne venue d'une table héritée. Rust a besoin
d'un unique type concret sur lequel la session, le fournisseur, et
les événements s'accordent tous. `String` est le seul choix qui
accommode toutes les formes d'id sans forcer le framework à savoir
laquelle votre application utilise. La méthode de confort entière
`auth_identifier()` existe pour le cas courant où votre colonne est
un `BIGINT`, mais le framework n'en dépend jamais - basculez votre
`User` vers un ULID demain, et rien dans la pile d'auth ne le
remarque.

## Câbler l'auth au démarrage

L'analogue Rust de `config/auth.php` est un `AuthConfig` enregistré
comme singleton `AuthManager` sur le conteneur, plus un
`UserProvider` enregistré sous un nom. `bootstrap.rs` fait
typiquement les deux en deux lignes :

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ... DB::init, installation de SessionMiddleware, etc.

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` lit le guard par défaut depuis `AUTH_GUARD`
(par défaut `"web"`) et livre d'origine deux guards nommés : un guard
de session `web` et un guard de token `api`, tous deux adossés au
fournisseur `"users"`. Les applications qui ont besoin de plus de
guards (fournisseur `admins` séparé, guards stateful et stateless
distincts) construisent la config explicitement :

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```
## Initialiser le moteur Magnetar

Le starter API initialise Magnetar après la base de données et après que
`APP_KEY` a mis `Crypt` en place :

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

Le moteur par défaut partage la connexion SeaORM de l'application et crée son
schéma sauf si `.apply_migrations(false)` est choisi. Il installe
atomiquement les adaptateurs password/session et passkey. Une nouvelle
initialisation renvoie une erreur au lieu de remplacer un adaptateur alors
qu'une autre requête utilise encore l'ancien magasin.

`MagnetarConfig` accepte aussi les valeurs de politique `session_config`,
`lockout_config`, `two_factor_config`, et `passkey_config` :

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

La liaison d'hôte par défaut utilise la table canonique `app_users` avec des ID
d'application `i64`. Le `UserId` public de Magnetar reste opaque à la frontière
de la façade ; la liaison par défaut analyse l'identifiant stocké seulement
quand il entre dans la table applicative.

### Méthodes de façade adossées à Magnetar

Le moteur installé alimente les méthodes du framework suivantes :

- `Auth::password().register(...)`.
- `Auth::password().authenticate(...)`.
- `Auth::magic_link().send(...)` et `.consume(...)`.
- `Auth::passkey().begin_registration(...)` et `.finish_registration(...)`.
- `Auth::passkey().begin_authentication(...)` et `.finish_authentication(...)`.
- `Auth::oauth(provider)` quand un délégué OAuth est installé.
- L'émission, la rotation, et la révocation du remember-me.
- La résolution de session bearer via `BearerTokenMiddleware`.
- `list_sessions`, `revoke_session`, et `revoke_all_sessions` dans
  `suprnova::magnetar_integration`.

Une connexion réussie fait tourner l'id de session du framework et le token
CSRF, stocke l'ID utilisateur applicatif, et enregistre une liaison web
Magnetar opaque. Le framework continue à posséder le middleware HTTP, les
cookies, le mail, les événements, et ses contrats guard/fournisseur.

### Authentification par mot de passe

Utilisez la façade password de Magnetar quand l'application veut la voie
intégrée credential, lockout, factor-gate, et session :

```rust,ignore
let user = Auth::password()
    .register("alice@example.com", password)
    .await?;

let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`authenticate` retourne des erreurs HTTP 401 pour des identifiants invalides,
un verrouillage, ou un second facteur requis. Les échecs de stockage et de
moteur restent des erreurs serveur. La méthode ne renvoie jamais le mot de
passe.

### Passkeys

Les appels passkey begin et finish exigent `SessionMiddleware` parce que le
sélecteur de cérémonie à usage unique est stocké dans la session du framework :

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

L'inscription suit la paire correspondante `begin_registration` /
`finish_registration`. L'enrôlement sur un compte existant exige un acteur de
requête vérifié et une réauthentification récente via le chemin de plugin ; un
simple ID utilisateur dans une session héritée n'est pas promu en acteur de
credential.

### Première preuve d'e-mail et ères d'auth

Magnetar traite la première preuve de boîte aux lettres réussie sur un compte
non vérifié comme une frontière atomique de credential. La réinitialisation de
mot de passe, la consommation d'un lien magique, et la complétion OAuth d'un
e-mail vérifié peuvent gagner cette frontière.

La transaction fait avancer l'ère d'auth du compte, révoque les anciennes
sessions et credentials remember, et retire les credentials provisoires qu'un
squatteur aurait pu enregistrer avant l'arrivée du propriétaire de la boîte aux
lettres. Les écritures password, passkey, compte lié, et second facteur
transportent un instantané d'acteur et échouent si l'ère du compte a changé
pendant l'opération.

Pour un compte déjà vérifié, une réinitialisation de mot de passe conserve les
passkeys légitimes, les comptes liés, et l'inscription à deux facteurs tout en
faisant tourner le mot de passe et en invalidant les sessions. OAuth ne relie
jamais automatiquement un compte existant non vérifié par simple e-mail ; il
exige une complétion d'e-mail vérifié ou un liaison explicite selon la
politique d'hôte.

### Surface directe du crate Magnetar

La plupart des applications restent sur les façades du framework. Les
applications qui construisent un hôte d'identité personnalisé peuvent dépendre
directement de `suprnova-magnetar` pour :

- les routes et handlers d'effets neutres au framework ;
- les plugins de mot de passe et de gestion du mot de passe ;
- les moteurs passkey et second facteur ;
- l'autorisation OAuth, les grants, les plugins de provider, l'autorisation de
  périphérique, et les services de broker de tokens ;
- les moteurs de session opaque, JWT, remember, et grant ;
- les liaisons de stockage personnalisées et le schéma SeaORM par défaut ;
- la migration d'auth-data sensible à la forme.

L'usage direct ne transfère pas la propriété HTTP ou utilisateur applicatif à
Magnetar. L'hôte mappe toujours ses requêtes wire, effets de mail, ID
applicatifs, drivers de rate-limit, et liaisons de session dans son propre
framework.


## La façade `Auth`

La façade statique `Auth` est la surface à la Laravel que vous
appelez depuis les contrôleurs et le middleware. Les méthodes basées
sur les identifiants et sur l'utilisateur délèguent au **guard par
défaut** (ce que pointe `AuthConfig::default_guard`, `"web"` par
défaut) ; les lectures synchrones `check`/`guest`/`id` sont le chemin
rapide adossé à la session et n'ont besoin d'aucun manager.

```rust
use suprnova::{Auth, Credentials};

// Valide les identifiants et connecte l'utilisateur. Déclenche
// Attempting → (Login + Authenticated), respecte remember-me.
// Retourne l'utilisateur résolu, ou None si les identifiants sont
// mauvais.
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// Connecte directement un utilisateur déjà connu.
Auth::login(user, remember).await?;

// Connecte par id sans revérifier les identifiants (par exemple
// juste après une inscription).
Auth::login_using_id(&id, remember).await?;

// Valide les identifiants sans persister de session (boîtes de
// dialogue de confirmation de mot de passe).
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// Authentifie pour cette requête seulement - aucune écriture de
// session. Le `once` de Laravel.
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// Chemin rapide adossé à la session (aucun AuthManager requis).
if Auth::check()    { /* authentifié */ }
if Auth::guest()    { /* non authentifié */ }
if let Some(id) = Auth::id() { /* id sous forme de chaîne */ }

// Indique si l'utilisateur courant a été authentifié par le cookie
// remember-me pour cette requête. Le `viaRemember()` de Laravel.
if Auth::via_remember() { /* … */ }

// Résout l'utilisateur courant (via le fournisseur enregistré).
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// Démonte l'auth + révoque remember-me + fait tourner le CSRF +
// déclenche Logout.
Auth::logout().await?;

// Destruction complète de la session (régénère l'id + vide + révoque
// remember-me + déclenche Logout).
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` retourne l'utilisateur résolu en cas de succès plutôt
qu'un simple `bool` - plus riche que l'API de Laravel, et évite
l'appel `Auth::user()` de suivi. `Ok(None)` signifie que les
identifiants n'ont résolu aucun utilisateur ; `Err` signifie un échec
de base de données / hachage / configuration qui doit remonter.

Si vous avez déjà vérifié vous-même l'identité d'un utilisateur et
que vous voulez seulement établir la session - par exemple après
qu'un callback OAuth s'est terminé - recourez à la primitive
synchrone :

```rust
// Sync, aucun fournisseur, aucun AuthManager, aucun événement.
// Retourne Err si appelé hors d'une portée de requête
// (SessionMiddleware non installé), si bien qu'une connexion
// abandonnée silencieusement ne peut jamais ressembler à un succès.
Auth::login_id(user.id.to_string())?;
```

`login_id` régénère l'id de session (empêchant la fixation de
session) et fait tourner le token CSRF, puis écrit l'id dans la
session. C'est délibérément conçu pour échouer explicitement : les
versions précédentes ne faisaient rien silencieusement hors d'une
portée de session, et l'audit a corrigé cela - une « connexion
réussie » qui n'a jamais atterri est le genre de bug que rien
d'autre n'attrape.

## `Auth::user()` et `user_as<T>`

`Auth::user()` retourne l'utilisateur derrière le trait :

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

Cet objet trait couvre quiconque implémente `Authenticatable`. Pour
récupérer votre `User` concret, effectuez un downcast via
`user_as::<T>()` :

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // Accès direct aux champs sur le modèle.
    println!("Welcome, {}!", user.name);
}
```

`user_as` retourne `Ok(None)` à la fois quand aucun utilisateur
n'est authentifié *et* quand l'utilisateur résolu n'est pas un `T`
(par exemple un `Auth::set_user(...)` d'un type différent ailleurs
dans la pile). À l'intérieur d'une requête, l'utilisateur est mis en
cache par requête, si bien qu'appeler `Auth::user()` de manière
répétée ne touche le fournisseur qu'une seule fois.

## Guards nommés

Les méthodes `Auth::*` nues parlent au guard par défaut. Pour agir
contre un guard spécifique, résolvez-le par son nom :

```rust
use suprnova::Auth;

// Les opérations en lecture seule fonctionnent sur chaque driver.
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attempt ont besoin d'un guard stateful. Les guards de
// token échouent explicitement ici.
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` retourne `Arc<dyn Guard>` (le contrat de
lecture) et `Auth::stateful_guard("name")` retourne `Arc<dyn
StatefulGuard>` (ajoute `attempt`/`login`/`logout`). Demander le
contrat stateful sur un guard de token retourne une erreur avec un
message de remédiation plutôt que de limiter silencieusement l'API.

## Fournisseurs d'utilisateurs

Un `UserProvider` indique à la pile d'auth comment récupérer et
valider les utilisateurs. Deux fournisseurs sont livrés intégrés, si
bien que le cas courant n'a besoin d'aucune implémentation
personnalisée :

- **`EloquentUserProvider<M>`** - résout via un `User` typé
  `#[suprnova::model]` qui est aussi `Authenticatable`. Recherche par
  clé primaire pour les id, par `email` (par défaut) pour les
  identifiants.
- **`DatabaseUserProvider`** - résout une table brute par son nom
  vers un `GenericUser` (id + map d'attributs). Utilisez-le quand
  vous n'avez pas ou ne voulez pas de modèle typé.

Les deux filtrent les recherches d'identifiants contre une liste
blanche (par défaut `["email"]`) - une map d'identifiants hostile ne
peut pas injecter de prédicats `WHERE` supplémentaires. Personnalisez
la liste blanche avec `.credential_columns([...])`, la colonne de
recherche avec `.identifier_column("uuid")`, ou la stratégie de
liaison d'id avec `.with_id_parser(...)`.

Pour brancher une source personnalisée (LDAP, une API externe),
implémentez `UserProvider` directement. `retrieve_by_id` prend
l'identifiant comme un `&str` :

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … récupère depuis LDAP, retourne comme Arc<dyn Authenticatable>
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials ont des défauts
    // de trait qui retournent None / false. Surchargez-les pour
    // prendre en charge `Auth::attempt` et `Auth::validate` contre
    // votre source.
}
```

Enregistrez-le sur le manager :

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## Protéger les routes

### `AuthMiddleware`

Filtre les routes réservées aux utilisateurs authentifiés. Les
requêtes non authentifiées sont redirigées vers une page de connexion
ou reçoivent un `401` :

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` retourne plutôt `401 Unauthorized` - le mieux
pour les API JSON. `AuthMiddleware::redirect_to("/login")` émet un
`302` pour les requêtes normales et un `409 X-Inertia-Location` pour
les requêtes Inertia (que le client Inertia transforme en une visite
pleine page). Pour filtrer sur un guard spécifique, chaînez
`for_guard` :

```rust
// 401 sauf si le guard api est authentifié.
.middleware(AuthMiddleware::new().for_guard("api"))
```

Un guard de token (`for_guard("api")`) dépend du middleware de bearer
token qui s'exécute plus tôt dans la chaîne pour peupler l'id d'auth
de la requête ; sans cela, le guard rapporte toujours non
authentifié.

### `GuestMiddleware`

L'inverse - pour les pages de connexion et d'inscription que les
utilisateurs authentifiés ne devraient pas voir :

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` fonctionne de la même manière
que `AuthMiddleware::for_guard`.

### `BasicAuthMiddleware`

Authentification HTTP Basic depuis l'en-tête `Authorization: Basic`
contre le fournisseur d'un guard :

```rust
use suprnova::BasicAuthMiddleware;

// Stateful - connecte l'utilisateur dans la session en cas de succès
// (le `basic` de Laravel).
.middleware(BasicAuthMiddleware::new())

// Stateless - authentifie pour cette requête seulement (le
// `onceBasic` de Laravel).
.middleware(BasicAuthMiddleware::once())
```

Le nom d'utilisateur décodé est comparé à l'identifiant `field` (par
défaut `"email"`) ; un en-tête manquant, malformé, ou invalide
retourne `401` avec un défi `WWW-Authenticate: Basic realm="..."`.
Configurez avec `.field(...)`, `.realm(...)`, et `.for_guard(...)`.

## Événements de cycle de vie

Les guards dispatchent cinq événements de cycle de vie. Écoutez-les
via l'[`EventFacade`](events.md) :

| Événement | Quand |
|---|---|
| `Attempting` | une tentative d'identifiants commence (`attempt`/`once`) |
| `Authenticated` | un utilisateur est activement authentifié pour cette requête (`login`/`once`/`once_using_id`) |
| `Login` | un utilisateur est persisté dans la session (`login`/`attempt` réussi) |
| `Logout` | un utilisateur est déconnecté |
| `Failed` | une tentative d'identifiants échoue (mauvais mot de passe ou id inconnu) |

Chaque événement porte le nom du guard et un id utilisateur sous
forme de chaîne - jamais le mot de passe en clair, et jamais la map
d'identifiants brute. `Authenticated` ne se déclenche que quand un
utilisateur est activement établi, pas lors d'une résolution passive
de `Auth::user()` sur une session existante, si bien que les
écouteurs n'obtiennent pas un flux de doublons à chaque requête
authentifiée.

## Le flux de connexion scaffoldé

`suprnova new` génère un contrôleur d'authentification qui utilise `Auth::attempt` contre le fournisseur enregistré. `FormRequest` et `Validate` produisent l'enveloppe de validation `{ message, errors }`. Pour une requête Inertia, le middleware installé de redirection de validation transforme cet échec en une redirection HTTP `303 See Other` vers l'arrière et consigne les erreurs pour la page d'origine. Un client non Inertia reçoit l'enveloppe JSON HTTP `422 Unprocessable Entity`.


```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

L'inscription suit la même forme : validez le formulaire, créez
l'utilisateur, puis `Auth::login(Arc::new(user), false).await?`
connecte l'utilisateur fraîchement créé dans la session et déclenche
l'événement `Login`.

## Le modèle `User` scaffoldé

Le `User` généré est un `#[suprnova::model]` qui implémente aussi `Authenticatable`. Il contient également `email_verified_at: Option<DateTime<Utc>>` et implémente `MustVerifyEmail` ainsi que `CanResetPassword`. Ces ponts permettent à `EloquentUserProvider<User>` de marquer la vérification par e-mail et de fournir les données d'identité pour la réinitialisation du mot de passe. L'extrait ci-dessous montre seulement les champs et helpers de connexion du guard ; utilisez le modèle généré complet pour l'implémentation complète du flux d'authentification. Ses helpers de mot de passe utilisent le module [`hashing`](hashing.md).


```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

L'attribut `hidden = ["password", "remember_token"]` fait que le
modèle ignore ces colonnes lors de la sérialisation en JSON pour le
réseau - elles existent sur la struct mais ne fuient jamais à travers
une réponse Inertia.

## Se souvenir de moi

Quand un moteur Magnetar est installé, `Auth::attempt(credentials, true)` et
`Auth::issue_remember_cookie` émettent des credentials remember liés à
l'objectif. Le navigateur reçoit toujours le cookie chiffré du framework,
`remember_me`, tandis que Magnetar possède le stockage des vérificateurs, les
contrôles d'ère d'auth, la rotation à usage unique, la gestion des anomalies, et
la révocation.

Sur une requête sans connexion active du framework, `SessionMiddleware` consomme
le cookie via le moteur installé, fait tourner le credential remember, émet une
nouvelle session Magnetar, et lie les deux couches de session. Une ère d'auth
périmée, une session de compte révoquée, un credential mal formé, ou un rejeu ne
parvient pas à authentifier la requête.

`Auth::revoke_remember_tokens()` invalide tous les credentials remember de l'utilisateur courant. Le cookie de suppression est mis en file avant la révocation côté serveur, de sorte que le navigateur abandonne son credential même si l'opération de stockage échoue.

Quand aucun moteur Magnetar n'est installé, le framework conserve le fallback
historique `remember_tokens` pour compatibilité. Les nouvelles applications
doivent initialiser Magnetar plutôt que de s'appuyer sur ce fallback.

## Garanties de sécurité

Une courte liste des invariants que la pile d'auth établit :

- **`Auth::login_id` échoue explicitement hors d'une portée de
  requête.** Les versions précédentes abandonnaient silencieusement
  l'écriture de session ; une « connexion réussie » qui n'a jamais
  atterri est le genre de bug que rien d'autre n'attrape.
- **L'id de session et le token CSRF se régénèrent à chaque
  connexion.** `login_id` et le `login`/`attempt` adossé au guard les
  font tous deux tourner pour empêcher la fixation de session.
- **La déconnexion efface l'état d'auth avant de révoquer
  remember-me.** Si la révocation en BD échoue, la session est déjà
  dans un état déconnecté, si bien qu'un emplacement d'auth périmé ne
  peut pas survivre à une déconnexion partielle. Le cookie
  d'effacement de remember-me est mis en file *avant* la suppression
  en BD, si bien que le navigateur abandonne le cookie même quand la
  suppression de la ligne échoue (le balayage de purge nettoie plus
  tard).
- **Les listes blanches d'identifiants bloquent l'injection.** Les
  deux fournisseurs intégrés filtrent `retrieve_by_credentials`
  contre `credential_columns`, si bien que des clés supplémentaires
  dans une map d'identifiants influencée par un attaquant ne peuvent
  pas devenir des prédicats `WHERE` supplémentaires.
- **Les écritures de credential sont contraintes par l'acteur.** Les mutations
  de mot de passe, passkey, compte lié, second facteur, session, et remember
  transportent l'ID utilisateur et l'ère d'auth établis par une authentification
  vérifiée. Une révocation ou un changement d'ère de première preuve fait
  échouer une écriture obsolète en vol.
- **La première preuve de boîte aux lettres est atomique.** Sur un compte non
  vérifié, une réinitialisation de mot de passe, la consommation d'un lien
  magique, ou la complétion OAuth d'un e-mail vérifié font avancer l'ère d'auth
  et retirent les credentials provisoires dans la même transaction. Une écriture
  concurrente d'un squatteur ne peut pas rétablir l'accès après le commit.
- **La vérification d'e-mail est liée à l'acteur.** La façade de vérification du
  framework exige un utilisateur authentifié dont l'id correspond au propriétaire
  du token. Un token d'un autre compte est rejeté sans être consommé.
- **L'e-mail OAuth ne prouve pas la propriété du compte.** Un compte existant non vérifié n'est jamais lié automatiquement à partir du seul e-mail du fournisseur. Les comptes vérifiés exigent une liaison explicite ; les comptes non vérifiés exigent le parcours de complétion de la première preuve d'e-mail.
- **Les événements d'auth ne portent jamais de texte en clair.** Nom
  du guard + id utilisateur sous forme de chaîne, rien de plus. Le
  suivi des tentatives échouées (verrouillages indexés par e-mail)
  relève de `BruteForce` dans les [Flux
  d'authentification](auth-flows.md), pas des événements de cycle de
  vie.

Le chapitre [Session](session.md) couvre la configuration des cookies
(`SESSION_LIFETIME`, `SESSION_COOKIE`, `SESSION_SECURE`,
`SESSION_SAME_SITE` et `SESSION_COOKIE_PREFIX`) dont héritent les guards
adossés à la session.

## Suivant

- [Flux d'authentification](auth-flows.md) - vérification d'e-mail,
  réinitialisation de mot de passe, limitation par force brute avec
  `LoginThrottleMiddleware`, 2FA TOTP, la suite d'événements
  `auth_flows`
- [OAuth et connexion sans mot de passe](oauth.md) - OAuth Magnetar, Apple, liens magiques, politique de fournisseur, et migration des données d'authentification
- [Autorisation](authorization.md) - `Gate`, policies, `Authorizable`
  pour « ce que cet utilisateur est autorisé à faire »
- [Sessions](session.md) - le cookie + le stockage qui soutient les
  guards de style `web`
- [Protection CSRF](csrf.md) - comment les requêtes qui modifient
  l'état sont filtrées
- [Hachage](hashing.md) - les helpers bcrypt + argon2 derrière
  `verify_password`
