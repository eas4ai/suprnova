# Flux d'authentification

`suprnova::auth_flows` est la couche de cycle de vie au-dessus de
[l'authentification](authentication.md). Là où `auth::*` répond à « qui est
cette requête », `auth_flows::*` couvre la preuve de boîte aux lettres, la
récupération de mot de passe, le verrouillage de compte, et les défis TOTP
fournis par le framework.

Cinq surfaces sont livrées sous cet espace de noms :

- `EmailVerification` produit et consomme les `auth_flow_tokens` du framework,
  envoie le courrier via la façade [`Mail`](mail.md), et marque comme vérifié
  le propriétaire authentifié du token via le `UserProvider` configuré.
- `PasswordReset` utilise le moteur Magnetar installé lorsqu’il est disponible. Sans Magnetar, les comptes vérifiés peuvent réinitialiser leur mot de passe via le `UserProvider` configuré et les `auth_flow_tokens` du framework. Les comptes non vérifiés sont refusés de manière sécurisée, car un fournisseur générique ne peut pas appliquer la stratégie atomique de Magnetar pour la première preuve d’adresse e-mail.
- `BruteForce` et `LoginThrottleMiddleware` délèguent l'état de verrouillage de
  compte à l'engine Magnetar installé.
- `TwoFactor` est la façade TOTP possédée par le framework au-dessus de
  `two_factor_credentials`. Elle fournit l'inscription, la confirmation, la
  vérification, les codes de récupération, la rotation de secret, la promotion
  du défi, et la protection contre le rejeu à la granularité du pas de temps.
- `remember_me` ré-exporte le module remember historique du framework pour la
  compatibilité de l'espace de noms. Quand Magnetar est installé, les flux
  `Auth` et `SessionMiddleware` remember normaux utilisent à la place les
  credentials remember Magnetar.

Deux middlewares de garde de route sont livrés dans le même espace de noms :

- `EnsureEmailVerifiedMiddleware` se compose après `AuthMiddleware` et filtre
  les routes sur `email_verified_at`.
- `TwoFactorChallengeMiddleware` se compose avant `AuthMiddleware` et renvoie
  une session avec un défi TOTP du framework en attente vers le formulaire de
  défi.

Les messages transactionnels passent toujours par la façade
[`Mail`](mail.md). Magnetar fournit les moteurs et les contrats de stockage de
sécurité ; il n'installe pas une seconde pile de transport de mail dans
l'application.

### Où vit l'état

Les tokens de vérification d'e-mail vivent dans la table `auth_flow_tokens` du
framework et l'horodatage vérifié est écrit via le `UserProvider` configuré. La
vérification est liée à l'acteur : l'utilisateur authentifié courant doit
posséder le token.

Les tokens de réinitialisation de mot de passe, les credentials de mot de
passe, les lignes de verrouillage, les sessions opaques, les credentials
remember, les cérémonies passkey, les cérémonies OAuth, et les ères d'auth
appartiennent à l'engine d'hôte Magnetar installé. La réinitialisation de mot
de passe, le lien magique, et la complétion OAuth d'un e-mail vérifié partagent
la barrière atomique de première preuve d'e-mail de Magnetar pour reprendre en
main des comptes non vérifiés.

La façade publique `TwoFactor` de ce chapitre conserve son schéma
`two_factor_credentials` possédé par le framework. Magnetar a aussi un moteur
de facteur utilisé par les flux intégrés de mot de passe, lien magique,
passkey, OAuth, et session. Ne supposez pas que les deux magasins sont
interchangeables : utilisez une seule surface d'inscription de manière
cohérente pour une application donnée.

Suprnova continue de posséder le middleware HTTP, les cookies, le mail sortant,
les événements, et le pont `UserProvider`. Le code applicatif utilise les
façades du framework plutôt que d'appeler directement les moteurs de stockage.

## Sémantique d'échec à travers les flux

Chaque façade suit une règle d'ordonnancement unique : le changement d'état
durable se commite en premier, puis les effets de bord de notification se
déclenchent. Une panique d'écouteur, un échec transitoire du transport de mail,
ou une erreur de dispatcher après la mutation ne peuvent pas annuler la
mutation.

- `EmailVerification::verify` exige le propriétaire authentifié du token,
  consomme le token, et marque l'utilisateur vérifié avant de déclencher
  `EmailVerified`.
- `PasswordReset::complete` valide l’opération via le moteur Magnetar installé lorsqu’il est disponible, y compris la stratégie de première preuve, l’avancement de l’époque d’authentification et la révocation atomique. Le fournisseur de repli est réservé aux comptes vérifiés : il consomme le jeton du framework, renouvelle le mot de passe du fournisseur, puis indique les résultats de révocation des sessions et de l’option Se souvenir de moi du framework. Les e-mails et les événements sont traités ensuite.
- `BruteForce::unlock_account` commite le déverrouillage avant de déclencher
  `AccountUnlocked`.
- `TwoFactor::confirm` estampille `confirmed_at` avant de déclencher
  `TwoFactorEnrolled` ; `TwoFactor::disable` supprime la ligne avant de
  déclencher `TwoFactorDisabled` ; `TwoFactor::complete_challenge` fait passer
  l'état de pending à authed avant de distribuer la paire standard
  `auth::Login` + `auth::Authenticated` suivie de `TwoFactorChallenged`.

Un écouteur qui a besoin de durabilité devrait mettre son travail en tampon
(mettre en file un job depuis le corps de l'écouteur) ; la façade elle-même ne
retente jamais.

## Amorçage

Initialisez Magnetar après `DB::init` et après que `APP_KEY` a initialisé `Crypt` :

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` crée le schéma d'auth par défaut, sauf si les migrations sont
désactivées, puis installe les adaptateurs password/session et passkey de façon
atomique. L'appeler une seconde fois renvoie une erreur. Les tests qui doivent
installer le moteur au niveau du processus doivent passer par un binaire
d'intégration dédié, car un moteur déjà installé ne se remplace pas.

### Configuration de la vérification d'e-mail

La vérification d'e-mail a besoin de :

1. Un `UserProvider` enregistré qui peut récupérer des utilisateurs par e-mail
   et marquer l'horodatage de vérification.
2. `MustVerifyEmail` sur le type utilisateur de l'application.
3. Une colonne `email_verified_at` nullable.
4. La table `auth_flow_tokens` du framework.

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

Le handler de vérification doit s'exécuter dans le périmètre d'une session
authentifiée. Un token valide appartenant à un autre utilisateur est rejeté
sans être consommé.

### Configuration de la réinitialisation de mot de passe et du verrouillage

`BruteForce` nécessite le moteur de mots de passe Magnetar installé. La réinitialisation du mot de passe privilégie ce moteur, mais `EloquentUserProvider<M>` prend en charge la réinitialisation pour les utilisateurs déjà vérifiés lorsque `M` implémente `MustVerifyEmail + CanResetPassword`. Les utilisateurs non vérifiés ne reçoivent aucun lien de réinitialisation fourni par le fournisseur. Installez Magnetar pour utiliser la réinitialisation comme première preuve atomique de la boîte aux lettres.

La réinitialisation de mot de passe normalise une adresse inconnue en `Ok(())` seulement après réussite des vérifications de limiteur d'abus, de configuration mail, de moteur et de stockage. Les chemins compte connu/inconnu peuvent encore différer dans les échecs et dans le temps d'exécution.

de révocation de session ou de remember.

### Enregistrer les migrations 2FA

Le framework fournit le schéma ; votre application y adhère en listant les deux
migrations dans son propre migrateur :

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... vos propres migrations ...

            // Crée `two_factor_credentials`.
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // Ajoute `last_used_timestep` pour la protection contre le rejeu TOTP.
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

Les deux sont idempotentes face à une base de données où elles ont déjà été
appliquées (la v1 utilise `CREATE TABLE IF NOT EXISTS` ; la v2 est un ajout de
colonne). Réexécuter `suprnova migrate` sur une base de données de production
qui possède déjà le schéma est un no-op.

### Environnement

Les mailables transactionnels lisent deux variables d'environnement à l'heure
de l'envoi :

| Variable | Défaut | Utilisée pour |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Marquage du sujet et label d'émetteur `otpauth://` affiché par les applications d'authentification. |
| `MAIL_FROM` | aucun  -  **erreur si non définie** | En-tête `From` de chaque message sortant. Définissez-le sur un domaine expéditeur vérifié. |

`MAIL_FROM` n'a délibérément aucun défaut. Lui donner une valeur de
remplacement comme `noreply@example.com` briserait silencieusement DMARC / SPF
en production et enverrait depuis un domaine que l'opérateur ne contrôle pas ;
la façade échoue donc de manière fermée. `EmailVerification::send_link` et
`PasswordReset::send_link` remontent l'erreur comme `Err` ;
`PasswordReset::complete` journalise via `tracing::warn!` et continue (le
changement de mot de passe est déjà commité, le chemin de notification ne peut
donc pas l'annuler).

Les applications définissent en plus `APP_URL` afin que les contrôleurs puissent
dériver l'URL de base utilisée dans les appels `send_link` ; la façade du
framework prend elle-même l'URL de base comme paramètre.

Le driver de mail se configure séparément via `MAIL_DRIVER`  -  voir la
documentation [Mail](mail.md).

## Vérification d'e-mail

`EmailVerification` produit, vérifie, et consomme des tokens de
vérification contre la table `auth_flow_tokens`, et marque
l'utilisateur vérifié via le fournisseur configuré. Quatre opérations
couvrent le cycle de vie :

| Méthode | Signature | Notes |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Produit + envoie, à partir d'un utilisateur déjà en main. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Normalise un résultat de fournisseur inconnu en `Ok(())`; les échecs de stockage et de mail renvoient toujours `Err`, et le temps d'exécution n'est pas égalisé. |
| `check` | `check(token: &str) -> Result<bool>` | Non consommant - sûr à appeler sur une page d'atterrissage. |
| `verify` | `verify(token: &str) -> Result<String>` | Lié à l'acteur et à usage unique : l'utilisateur authentifié doit posséder le token ; en cas de succès, il est consommé, l'utilisateur est marqué vérifié, et son ID est retourné. |

```rust
use suprnova::auth_flows::EmailVerification;

// Après une inscription fraîche, avec l'utilisateur qui vient d'être
// créé en main :
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Vérification optionnelle sur la page d'atterrissage - non
// consommante, si bien qu'un rafraîchissement de page ne brûle pas
// le token.
let valid: bool = EmailVerification::check(&token_str).await?;

// Le handler de clic s'exécute derrière l'authentification. `verify`
// ne consomme le token que lorsque `Auth::id()` correspond à son propriétaire.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` déclenche `EmailVerified` en cas de succès - les écouteurs
sont le bon endroit pour débloquer des fonctionnalités
supplémentaires (e-mail de bienvenue, abonnements par défaut, CTA
« complétez votre profil ») sans les coupler au handler de
vérification. L'événement porte l'id utilisateur du fournisseur.

### Le point de terminaison resend (anti-énumération)

`resend` ne prend que l'e-mail - la façade recherche l'utilisateur via le fournisseur actif et, quand un compte est enregistré, produit un token et envoie le mail. Une adresse inconnue est normalisée en `Ok(())` seulement après les mêmes vérifications en amont ; les échecs de stockage ou de distribution continuent de remonter en `Err`, et le temps d'exécution n'est pas égalisé :
```rust

use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` effectue la recherche + l'anti-énumération en interne.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "Si cette adresse e-mail est enregistrée, un lien de vérification a été envoyé.",
    ))
}
```

`send_link` et `resend` construisent tous deux l'URL comme
`{base_url}?token={plaintext_token}`. Une barre oblique de fin sur
`base_url` est retirée avant que la chaîne de requête ne soit
ajoutée, si bien que `https://app.example.com/verify/` et
`https://app.example.com/verify` produisent tous deux une URL propre.

Le handler de clic doit s'exécuter derrière `AuthMiddleware`. Il récupère le
token depuis la chaîne de requête et appelle `verify` :

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;

    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` compare `Auth::id()` au propriétaire du token avant de le consommer.
Un token appartenant à un autre compte retourne la même réponse de token
invalide et reste inutilisé. En cas de succès, le fournisseur marque le
propriétaire authentifié comme vérifié et la façade déclenche `EmailVerified`.

### Routes réservées aux vérifiés : `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` filtre les routes selon le
`email_verified_at` de l'utilisateur authentifié. Composez-le après
`AuthMiddleware`, et la chaîne bloque toute requête dont
l'utilisateur n'a pas encore complété l'étape de vérification.

Le choix entre **403 JSON** et **redirection HTML 302** se fait au
moment de l'enregistrement de la route via le constructeur - il n'y
a pas de sniffing du contenu de la requête, ce qui correspond au
motif posé par `AuthMiddleware::new` / `AuthMiddleware::redirect_to` :

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// Surface API - 403 avec un corps JSON.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Surface web - 302 (ou 409 + X-Inertia-Location pour les visites Inertia).
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

Si aucun utilisateur n'est authentifié, le middleware retombe dans la
même branche de réponse que « authentifié mais non vérifié » - ce
qui correspond à la forme `! $request->user() || !
hasVerifiedEmail()` de Laravel. Composez `AuthMiddleware` en premier
quand vous voulez un `401` séparé pour les requêtes non
authentifiées.

Pour un branchement à l'intérieur du handler (par exemple rendre
conditionnellement un CTA « merci de vérifier » sans rediriger),
chargez l'utilisateur typé via le guard de session et lisez la
méthode du trait :

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // branchez là-dessus
}
```

## Réinitialisation de mot de passe

`PasswordReset` a quatre opérations :

| Méthode | Signature | Notes |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Retourne `Ok(())` pour une adresse inconnue après que les vérifications du limiteur d'abus, de la configuration mail, du moteur et du stockage ont réussi ; les autres échecs retournent toujours `Err`. |
| `check` | `check(token: &str) -> Result<bool>` | Validation non consommatrice via le moteur Magnetar installé. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Consomme atomiquement le token, applique la politique de première preuve, fait tourner les credentials, révoque les sessions et l'état remember, et retourne l'ID utilisateur. |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | Exécute la même transaction et retourne les comptes de révocation committés. |

```rust
use suprnova::auth_flows::PasswordReset;

// Depuis le formulaire « mot de passe oublié ». Toujours Ok(()) -
// la façade recherche l'utilisateur et n'envoie que si un compte
// est enregistré.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Vérification optionnelle sur la page d'atterrissage avant de
// rendre le formulaire de nouveau mot de passe.
let valid: bool = PasswordReset::check(&token).await?;

// Le handler de clic, après que l'utilisateur a soumis un nouveau
// mot de passe : consomme le token + fait tourner le mot de passe,
// en retournant l'id utilisateur.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` transmet le mot de passe en clair via `SecretString` ; Magnetar le
hache dans le moteur de credentials. Ne le pré-hachez pas. Un mot de passe vide
ou constitué uniquement d'espaces retourne HTTP 400 avant l'appel du moteur.

### Comportement d'anti-énumération borné

`PasswordReset::send_link` retourne `Ok(())` pour une adresse inconnue uniquement
après que les vérifications du limiteur d'abus, de la configuration mail, du
moteur et du stockage ont réussi. Les échecs de configuration, du limiteur, du
stockage et du mail retournent toujours `Err`. Le contrôleur dogfood donne aux
requêtes réussies pour des comptes connus et inconnus le même statut HTTP et le
même corps, mais l'implémentation n'égalise pas leur temps d'exécution.

### Effets de bord de `complete`

Magnetar commit la réinitialisation de mot de passe dans une transaction :

1. Consomme le token de réinitialisation à usage unique.
2. Applique la politique de première preuve d'e-mail quand le compte n'est pas
   encore vérifié.
3. Hache et remplace le mot de passe.
4. Fait avancer l'ère d'authentification.
5. Révoque les anciennes sessions opaques et credentials remember.
6. Supprime les credentials provisoires lorsque cette réinitialisation est la
   première preuve de boîte aux lettres du compte.

Après le commit, le framework envoie `PasswordChangedMail` et distribue
`PasswordResetCompleted`. Un échec de mail ou d'écouteur ne peut pas annuler la
réinitialisation.

Sur un compte déjà vérifié, la réinitialisation préserve les passkeys légitimes,
les comptes liés et l'inscription deux facteurs confirmée. Sur un compte non
vérifié squatté, la première preuve supprime les credentials provisoires afin
que l'ancien inscrivant ne puisse pas conserver l'accès.

## Protection contre la force brute

La couche de force brute a deux parties : la façade `BruteForce` qui
enregistre et interroge l'état de verrouillage, et
`LoginThrottleMiddleware` qui court-circuite au niveau de la couche
HTTP avant que le handler ne soit invoqué.

### La façade `BruteForce`

Appelez `record_failed_attempt` depuis la branche d'échec d'auth de
votre handler de connexion, et `reset_attempts` depuis la branche de
succès :

```rust
use suprnova::auth_flows::BruteForce;

// Dans le chemin d'échec d'auth :
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Faites éventuellement remonter une réponse personnalisée. Le
    // middleware le fera pour vous à la *prochaine* requête - voir
    // ci-dessous.
}

// Dans le chemin de succès :
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` retourne le `LockoutStatus` mis à jour
(`is_locked`, `failed_attempts`, et `locked_until` en cas de
verrouillage). Passez l'`ip` optionnelle pour les logs d'audit ;
passez `None` si votre transport ne fait pas remonter proprement une
IP client.

Deux opérations supplémentaires :

```rust
// Lecture seule - sûr sur des e-mails sans historique.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Déverrouillage admin / forcé. Déclenche `AccountUnlocked`
// uniquement sur une vraie transition d'état (un déverrouillage
// no-op sur un compte déjà déverrouillé ne déclenche rien).
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` retourne `true` quand le compte était verrouillé au
moment de l'appel, `false` sinon. L'événement `AccountUnlocked` ne se
déclenche que sur `true` - un retour `false` est le no-op qu'il est,
pas un événement d'audit.

### `LoginThrottleMiddleware`

Le middleware lit l'état de verrouillage pour l'e-mail que cible une
requête, et court-circuite avec `429 Too Many Requests` quand le
compte est verrouillé. Le handler de connexion n'est jamais invoqué,
si bien qu'un compte verrouillé n'a même pas l'occasion de tenter une
vérification d'identifiants :

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// L'extracteur d'e-mail est une fermeture sync sur `&Request`. Lire
// un corps JSON/form est async et consomme `Request`, si bien que la
// fermeture ne peut pas lire le corps - récupérez-le plutôt depuis un
// en-tête, une chaîne de requête, ou un paramètre de route.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Surfaces d'extraction pratiques :

- Un en-tête (`X-Login-Email`), positionné par un pré-processeur
  précédent - le motif utilisé dans l'application dogfood.
- Un paramètre de chaîne de requête (`?email=…`).
- Un paramètre de route (`/login/{email}`).

Retourner `None` depuis l'extracteur est le signal explicite « je
n'ai rien à vérifier » - le middleware laisse passer la requête
inchangée. Cela rend le middleware sûr à installer sur des routes qui
voient occasionnellement du trafic anonyme (par exemple le même point
de terminaison `POST /login` qui gère aussi une sous-action « demande
de réinitialisation de mot de passe » sans e-mail).

En cas de verrouillage, le middleware retourne :

- Statut `429 Too Many Requests`.
- En-tête `Retry-After` - en secondes, calculé depuis le `locked_until` du
  verrouillage via `LockoutStatus::retry_after_seconds`. Se replie sur `900`
  (15 minutes, période de verrouillage par défaut de Magnetar) si l'horodatage
  est d'une manière ou d'une autre absent.
- Corps : `"Account locked due to too many failed login attempts. Try
  again later."`

### Échec fermé par défaut sur les erreurs de backend

Si `get_lockout_status` retourne une erreur, `LoginThrottleMiddleware` journalise l'échec et, par défaut, renvoie HTTP `503 Service Unavailable` avec `Retry-After: 1` sans invoquer le handler de connexion. Pour garder la connexion disponible pendant une panne du backend de verrouillage, optez explicitement pour `.on_backend_error(BackendErrorPolicy::FailOpen)` ; seul ce comportement transmet la requête au handler.


### Empiler avec `RateLimitMiddleware`

`LoginThrottleMiddleware` est par compte - il filtre une seule
adresse e-mail une fois le seuil franchi. Pour des quotas par IP,
empilez-le avec [`RateLimitMiddleware`](rate-limiting.md). Les deux
se composent naturellement :

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Ensemble, ils couvrent les formes réalistes du bourrage
d'identifiants : distribué (un e-mail × de nombreuses IP) est le
travail de la limitation de débit ; concentré (de nombreuses
tentatives × un e-mail) est le travail du middleware de limitation.

### Configuration

`MagnetarConfig` accepte un `LockoutConfig`. Le défaut est cinq tentatives
échouées, une période de comptage et de verrouillage de 15 minutes, une
rétention des tentatives de sept jours, et `BackendErrorPolicy::FailClosed` :

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

Utilisez `LockoutConfig::disabled()` uniquement lorsqu'un autre contrôle
d'identité à échec fermé remplace le verrouillage de compte.

## Deux facteurs (TOTP)

`TwoFactor` couvre la 2FA basée sur TOTP - le genre qui s'apparie
avec n'importe quelle application d'authentification conforme aux
standards (Google Authenticator, 1Password, Bitwarden, Authy). Le
flux est inscription → confirmation → vérification continue, plus des
codes de récupération à usage unique pour quand l'utilisateur perd
son appareil, plus le flux de défi qui recoud tout cela dans le cycle
de vie de la connexion.

### Le trait `TwoFactorUser`

Le framework ne peut pas atteindre le stockage d'utilisateurs de
votre application, si bien que les appelants implémentent un petit
trait pour faire le pont entre leur modèle utilisateur et la façade
2FA :

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` est une clé de stockage opaque. Il peut être un ID applicatif
numérique rendu sous forme de texte, un UUID ou un `UserId` Magnetar. La table
TOTP du framework n'a aucune clé étrangère vers la table utilisateur de
l'application.

`email` est replié dans le segment `account_name` de l'URL `otpauth://`, de
sorte que l'application d'authentification affiche un libellé de compte
reconnaissable.

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### Stockage

L'état 2FA vit dans la table `two_factor_credentials` possédée par
le framework. Les secrets et les codes de récupération sont chiffrés
au repos avec `crate::crypto::Crypt::encrypt_string`, qui exige une
`EncryptionKey` globale au processus. Les applications adhèrent au
schéma en listant les deux migrations dans leur
`Migrator::migrations()` - voir [Amorçage](#amorçage).

### Inscrire, confirmer, vérifier

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Inscription : génère un secret frais + 10 codes de récupération,
//    les persiste chiffrés, retourne tout ce qu'il faut pour rendre
//    le QR code.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - lien profond `otpauth://totp/...`
// response.qr_code_svg - <svg> enveloppant un PNG base64, à intégrer en ligne
// response.recovery_codes - Vec<String>, 10 codes en clair - à montrer UNE SEULE FOIS

// 2. Confirmation : l'utilisateur ouvre l'application
//    d'authentification et saisit le code à 6 chiffres. `confirm`
//    le valide et estampille `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// déclenche `TwoFactorEnrolled`

// 3. Lors des connexions suivantes, filtrez la session via `verify` :
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` retourne les codes de récupération en clair **exactement une
fois**. Il n'existe aucune API pour les récupérer plus tard - la
colonne chiffrée est à sens unique à partir de ce moment. Montrez-les
sur la page de succès d'inscription, encouragez l'utilisateur à les
sauvegarder, et ne stockez le texte en clair nulle part ailleurs.

`enroll` refuse d'écraser une inscription **confirmée** - il retourne
un `409` pour pousser l'appelant vers `re_enroll`, qui exige une
preuve de possession. Se réinscrire sur une ligne non confirmée (en
attente) est autorisé : l'inscription précédente n'est jamais devenue
faisant autorité.

### Protection contre le rejeu

`verify` écrit le pas de temps TOTP courant dans
`last_used_timestep` en cas de succès. Les vérifications suivantes où
`current_timestep <= last_used_timestep` sont rejetées même quand le
code lui-même est structurellement valide, déjouant un rejeu de code
volé à l'intérieur de la fenêtre de 30 secondes.

La revendication du pas de temps est atomique. L'estampille
s'applique via un `UPDATE … WHERE last_used_timestep IS NULL OR
last_used_timestep < :current` conditionnel, et le verify ne réussit
que quand l'instruction affecte exactement une ligne. Deux vérifs
concurrentes sur le même pas de temps ne peuvent pas gagner toutes
les deux : la première fait basculer la colonne, le prédicat de la
seconde ne correspond plus, et la seconde est traitée comme un
rejeu. Un simple lire-modifier-écrire serait une condition de course
TOCTOU - les deux vérifs liraient la ligne avant estampille,
valideraient toutes les deux le même code, estampilleraient toutes
les deux, et réussiraient toutes les deux. Les tentatives
concurrentes sont aussi comptées comme des tentatives échouées, si
bien que le compteur de force brute les enregistre.

### Codes de récupération

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

Usage unique : un code correspondant est retiré de la ligne avant que
l'appel ne retourne, si bien qu'une seconde tentative contre le même
code retourne `false`. Les codes sont 12 chiffres décimaux sous la
forme `NNNNNN-NNNNNN` (environ 40 bits d'entropie chacun, ce qui
correspond au format de Laravel Fortify).

`consume_recovery_code` n'accepte des codes que quand la 2FA est
entièrement confirmée - il court-circuite vers `Ok(false)` tant que
`confirmed_at` est NULL. Sans ce gate, un attaquant qui a déclenché
une inscription sur le compte d'une victime (ou n'importe quel flux
qui crée la ligne sans confirmer) pourrait s'authentifier en
utilisant seulement un code de récupération frais, contournant
entièrement TOTP. Le contrat est symétrique avec le garde-fou
« inscription confirmée uniquement » de `verify`.

### Faire tourner les codes de récupération et les secrets

Quand un utilisateur épuise ses codes de récupération, ou veut les
faire tourner après une compromission suspectée :

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` doit se valider soit comme un code TOTP courant, soit comme
un code de récupération inutilisé. Sans cette vérification de
preuve, un attaquant ayant détourné une session pourrait faire
disparaître silencieusement les codes de récupération de
l'utilisateur légitime (déni de service contre la récupération de
compte). Les codes frais remplacent l'ensemble persisté ; le secret
existant et `confirmed_at` sont préservés, si bien que l'application
d'authentification de l'utilisateur continue de fonctionner sans
ré-appairage. Erreurs :

- `400` - aucune inscription confirmée n'existe ; appelez d'abord
  `enroll`/`confirm`.
- `401` - `proof` ne se valide ni comme un code TOTP, ni comme un
  code de récupération inutilisé.
- `429` - le compte est verrouillé par la limitation par force
  brute.

Pour faire tourner le **secret** (se ré-apparier à un nouvel
appareil) sans désactiver la 2FA au préalable :

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

Même modèle de preuve que `regenerate_recovery_codes`. La ligne est
réécrite avec un secret frais + 10 codes de récupération frais ;
`confirmed_at` retombe à NULL, si bien que l'utilisateur doit
`confirm` avec un code venant du nouvel authentificateur avant que la
2FA ne soit de nouveau active.

### Désactiver

```rust
TwoFactor::disable(&user_2fa).await?;
// déclenche `TwoFactorDisabled` seulement si une ligne a été retirée
```

Idempotent : un disable sur un utilisateur qui ne s'est jamais
inscrit n'est pas une erreur. L'événement `TwoFactorDisabled` ne se
déclenche que sur une vraie transition d'état, si bien que les
écouteurs d'audit voient une entrée par désactivation réelle plutôt
qu'une par clic sur un bouton no-op.

### Flux de défi (filtrer la connexion via le second facteur)

Les primitives enroll / confirm / verify sont les briques de base ;
le **flux de défi** les recoud dans le cycle de vie de la connexion,
si bien qu'un utilisateur avec la 2FA activée ne peut pas atteindre
les pages protégées avec le seul mot de passe.

Le flux :

1. La connexion par mot de passe résout un utilisateur.
2. Si `TwoFactor::is_enabled_by_id(&user_id)` retourne `true`, le
   handler de connexion appelle
   `TwoFactor::start_challenge(user_id, remember)` - cela range l'id
   utilisateur comme **en attente** dans la session, vide
   l'emplacement pleinement authentifié, révoque tout cookie
   remember-me émis par `Auth::attempt`, et se souvient si
   l'utilisateur avait opté pour remember-me afin que le cookie
   puisse être réémis une fois le défi terminé. `Auth::id()` retourne
   `None` à partir de ce point et jusqu'à ce que le défi soit
   terminé.
3. Le handler redirige vers une route `/two-factor-challenge` qui
   affiche le formulaire de code.
4. Le handler POST du défi appelle
   `TwoFactor::complete_challenge(code)` - vérifie le code (TOTP
   **ou** un code de récupération inutilisé, ce qui correspond au
   contrôleur de challenge de Fortify), fait passer l'état de pending
   → authed, fait tourner l'id de session (déjouant la fixation de
   session) et le token CSRF, réémet le cookie remember-me quand
   l'utilisateur avait opté pour, et distribue les événements de
   cycle de vie standards `auth::Login` + `auth::Authenticated` plus
   `TwoFactorChallenged`, spécifique à la 2FA.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Rétrograde en « pending » : emplacement d'auth vidé,
                // pending positionné, cookie remember-me révoqué. Fait
                // passer le flag remember du formulaire pour que
                // `complete_challenge` puisse réémettre le cookie en
                // cas de succès.
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // L'id de session + le CSRF ont tourné ; remember-me a été
    // réémis si le formulaire de connexion original l'avait
    // positionné. Les écouteurs qui s'accrochent à `auth::Login` /
    // `auth::Authenticated` ont vu une connexion normale.
    redirect!("/dashboard").into()
}
```

`complete_challenge` fait tourner l'id de session et le token CSRF
dans le cadre de la promotion vers authed. Cela ferme l'attaque
classique de fixation de session, où un attaquant plante un id de
session connu sur une victime avant qu'elle ne se connecte - après la
rotation, l'id planté est mort et seul l'id fraîchement généré porte
l'état authentifié. Le contrat correspond à `Auth::login_id` /
`Auth::login_using_id`, si bien que les connexions 2FA sont
indistinguables des connexions sans 2FA en termes d'état de session
et d'observabilité par les écouteurs.

Filtrez chaque groupe de routes protégé avec
`TwoFactorChallengeMiddleware` **avant** `AuthMiddleware`, afin qu'une
session en attente soit renvoyée vers la page de défi plutôt que vers
la page de connexion :

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

La page de défi elle-même (le GET qui rend le formulaire, le POST qui
appelle `complete_challenge`) ne doit PAS installer
`TwoFactorChallengeMiddleware` - elle est la destination. Le handler
POST vérifie aussi typiquement `TwoFactor::pending_user_id().is_some()`
en amont, afin qu'un lien périmé n'atteigne pas la logique de verify
avec une session vide.

`TwoFactor::cancel_challenge()` vide les deux emplacements en attente
sans authentifier personne - câblez-le à un lien « retour à la
connexion » sur la page de défi.

**Repli sur code de récupération.** `complete_challenge(code)` essaie
d'abord le chemin TOTP, puis se replie sur la consommation d'un code
de récupération, si bien qu'un utilisateur qui a perdu son
authentificateur peut quand même entrer. Chaque code de récupération
est à usage unique.

**Lien avec la force brute.** Les codes de défi échoués alimentent le
compteur de force brute par compte via
`BruteForce::record_failed_attempt`, de la même manière que le
`TwoFactor::verify` nu. Un attaquant qui martèle le formulaire de défi
déclenchera `AccountLocked` au-delà du seuil configuré. Une seule
soumission mauvaise compte comme **une** tentative échouée, même si
`complete_challenge` essaie en interne à la fois le chemin TOTP et le
chemin code de récupération - les routines de validation silencieuse
sautent le compteur de force brute, si bien que la couche externe
enregistre la tentative canonique exactement une fois.

**Gate de verrouillage.** `complete_challenge` vérifie
`BruteForce::is_locked` en amont et retourne `429 Too Many Requests`
si le compte est déjà verrouillé - même quand le code soumis est
correct. Sans ce gate interne à la méthode, un attaquant qui a
déclenché le verrouillage pourrait quand même entrer en soumettant le
bon code à la requête suivante : le compteur de force brute est
indexé sur l'e-mail de l'utilisateur, mais `verify` lui-même ne le
consulte pas. Le `LoginThrottleMiddleware` du chemin mot de passe
impose la même contrainte au niveau de la couche route ; le composer
devant la route POST du défi ne pose pas de problème - les deux gates
sont idempotents.

**Événement d'échec.** `complete_challenge` distribue
`TwoFactorChallengeFailed { user_id }` sur un mauvais code (ou un
compte verrouillé), distinct du `auth::Failed` du chemin mot de
passe. Les écouteurs qui surveillent « l'utilisateur a essayé la 2FA
et a échoué » s'abonnent au nouvel événement ; les écouteurs qui
surveillent « le mot de passe n'a pas authentifié » restent sur
`auth::Failed`. Les deux surfaces sont maintenues séparées afin
qu'une faute de frappe 2FA ne ressemble pas à un échec de mot de
passe pour les pipelines d'audit.

### Pourquoi Suprnova diverge

Le `user_id` TOTP du framework est un `String`. Un type fixe `i64`, UUID ou
d'identifiant Magnetar lierait la façade réutilisable à un schéma applicatif
unique. La frontière de chaîne laisse une application choisir tout identifiant
stable, au coût d'une conversion au site de l'appel.

La porte de facteur intégrée de Magnetar est distincte de cette façade
conservée. Cette séparation préserve la compatibilité pour les applications qui
utilisent `two_factor_credentials`, mais elles ne devraient pas inscrire le
même compte dans les deux magasins.

## Se souvenir de moi

`suprnova::auth_flows::remember_me` ré-exporte le module historique
`suprnova::auth::remember` pour des raisons de compatibilité.

Quand Magnetar est installé, `Auth::attempt(..., true)`,
`Auth::issue_remember_cookie`, et l'hydratation de `SessionMiddleware` utilisent
les credentials remember liés à une finalité de Magnetar. Magnetar stocke des
digests de vérificateur, contrôle l'ère d'auth, fait tourner les credentials
lors de leur emploi réussi, les révoque avec la session utilisateur, et signale
les anomalies de rejeu ou de credential malformé sans exposer le secret.

Le cookie visible au navigateur reste possédé par le framework. Il est chiffré
avec le nom logique `remember_me`, suit `SESSION_COOKIE_PREFIX`, et est effacé
avant la révocation backend afin qu'un échec de stockage ne laisse pas le
navigateur envoyer l'ancien credential.

L'implémentation historique par ligne de base de données reste disponible
lorsqu'aucun moteur Magnetar n'est installé. Les nouvelles applications
devraient initialiser Magnetar et considérer le ré-export historique comme une
surface de transition.

## Événements

Neuf événements se déclenchent à travers les flux, un par transition
d'état de sécurité :

| Événement | Déclenché par | Porte |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` en cas de succès | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` en cas de succès - anti-énumération silencieuse pour les e-mails absents | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` en cas de succès | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` sur la transition déverrouillé → verrouillé | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` quand un déverrouillage réel s'est produit | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` en cas de succès | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` a fait passer pending → authed | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` a rejeté un mauvais code ou refusé un compte verrouillé | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` quand une ligne a réellement été retirée | `user_id: String` |

Chaque événement est `Debug + Clone + 'static`, ne porte aucune
donnée sensible (aucun token en clair, aucune IP), et utilise des
identifiants sous forme de chaîne, si bien que les écouteurs peuvent
les sérialiser à travers les frontières de tâche sans faire fuiter
d'information de type depuis le backend de stockage des
utilisateurs.

### Écouter

Abonnez-vous via l'API d'événements standard - la même surface que
tout autre événement en cours de processus :

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ... notification Slack, ajout dans la table d'audit, etc.
        Ok(())
    }
}

// Dans bootstrap.rs :
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Les écouteurs s'exécutent sur le runtime de Tokio et sont distribués
dans l'ordre d'enregistrement. Voir le chapitre [Événements](events.md)
pour la surface complète.

## Tests

Trois fakes couvrent la surface auth-flows, et ils se composent.

### `Mail::fake()`

Installe un transport de capture local au processus. Chaque envoi
pendant la durée de vie de la garde atterrit dans un tampon en
mémoire au lieu de partir :

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... piloter le flux ...
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` expose `assert_sent`, `assert_not_sent`,
`assert_sent_count`, plus les accesseurs bruts `captured()` et
`count()`. Quand la garde se drop, le transport précédemment lié est
restauré - les tests qui entrelacent des fakes avec une liaison de
transport explicite ne fuitent pas d'état.

### `EventFacade::fake()`

La même forme, mais pour les événements :

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... piloter le flux ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

Le fake enregistre les événements distribués sans invoquer les
écouteurs, si bien qu'un écouteur qui parle à un service externe ne
se déclenchera pas pendant le test. Le compagnon
`assert_not_dispatched::<E>(pred)` affirme le négatif ;
`dispatched_count::<E>(pred)` retourne le compte brut pour des
assertions plus fines.

### Tests d'intégration pour la vérification d'e-mail et la réinitialisation de mot de passe

Les tests de vérification d'e-mail créent `auth_flow_tokens`, enregistrent un
`UserProvider`, établissent le propriétaire authentifié du token, définissent
`MAIL_FROM`, et pilotent la façade sous `Mail::fake()`.

Les tests de réinitialisation de mot de passe installent un adaptateur de test
`MagnetarPasswordAuthEngine` et affirment l'émission, la vérification non
consommatrice, la complétion atomique, la révocation de session et le
comportement à usage unique.

Les exemples de source canoniques sont :

- `framework/tests/email_verify.rs` pour la vérification liée à l'acteur et les
  tokens à usage unique.
- `framework/tests/password_reset.rs` pour la délégation Magnetar et les
  résultats de complétion.
- `framework/tests/magnetar_default_engine.rs` pour une configuration réelle du
  moteur par défaut.
- `framework/tests/brute_force.rs` pour le cycle de vie du verrouillage.
- `framework/tests/two_factor_challenge_flow.rs` pour le flux de défi TOTP
  conservé par le framework.
- `framework/tests/magnetar_remember_middleware.rs` pour la rotation remember
  et la liaison à deux sessions.

L'installation de Magnetar globale au processus est volontairement à usage
unique. Placez les tests qui nécessitent des moteurs différents dans des
binaires de test d'intégration distincts, ou installez un adaptateur de test une
seule fois pour tout le binaire.

## Référence

| Symbole | Objectif |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, et `verify` lié à l'acteur ; `verify` retourne l'ID utilisateur. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` pour 403 JSON et `redirect_to(path)` pour les redirections navigateur ou Inertia. |
| `suprnova::auth_flows::PasswordReset` | Réinitialisation prioritaire via Magnetar avec repli sur un `UserProvider` pour comptes vérifiés au moyen des `auth_flow_tokens` du framework. |
| `suprnova::MustVerifyEmail` | Contrat de l'utilisateur applicatif pour la façade de vérification du framework. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | Définition de table SeaORM pour les tokens de vérification du framework. |
| `suprnova::auth_flows::BruteForce` | Façade de verrouillage de compte adossée à Magnetar. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP qui retourne 429 avant le handler de connexion quand le compte est verrouillé. |
| `suprnova::auth_flows::TwoFactor` | Façade TOTP du framework conservée pour l'inscription, la vérification, la récupération et les défis. |
| `suprnova::auth_flows::TwoFactorUser` | Pont de l'utilisateur applicatif vers la façade TOTP du framework. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | Porte pour les sessions en attente du défi TOTP du framework. |
| `suprnova::auth_flows::remember_me` | Ré-export de compatibilité du module remember historique du framework. |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | Configuration du moteur Magnetar par défaut et installation à usage unique. |
| `suprnova::auth_flows::events::*` | Événements de cycle de vie de l'authentification. |

## Suivant

- [Authentification](authentication.md) - guards, fournisseurs, la
  façade `Auth`, `AuthMiddleware`.
- [E-mail](mail.md) - la couche de transport via laquelle les appels
  `send_link` distribuent.
- [Événements](events.md) - enregistrer des écouteurs pour les neuf
  événements auth-flow.
- [Limitation de débit](rate-limiting.md) - associez
  `RateLimitMiddleware::ip_based` avec `LoginThrottleMiddleware` pour
  une défense en couches.
- [Sessions](session.md) - ce que touchent `start_challenge` /
  `complete_challenge` quand ils font tourner l'id de session.
