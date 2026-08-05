# Flux d'authentification

`suprnova::auth_flows` est la couche de cycle de vie au-dessus de
l'[authentification par session](authentication.md). Là où `auth::*`
répond à « qui est cette requête », `auth_flows::*` répond à tout ce
qui entoure cette question - prouver que l'adresse e-mail est
réelle, la récupérer quand le mot de passe est perdu, la défendre
contre le bourrage d'identifiants, et la protéger avec un second
facteur. Cinq flux sont livrés sous un seul espace de noms :

- `EmailVerification` - produit, vérifie, et consomme des tokens de
  vérification à usage unique ; `send_link` / `resend` distribuent le
  mail de vérification via la façade [`Mail`](mail.md), et `verify`
  marque l'utilisateur vérifié via le fournisseur d'utilisateurs
  configuré.
- `PasswordReset` - `send_link` anti-énumération, `check` non
  consommant, et `complete`. `complete` fait tourner le mot de passe
  via le fournisseur d'utilisateurs configuré, révoque chaque session
  et chaque ligne remember-me de l'utilisateur, et envoie une
  notification de sécurité `PasswordChangedMail`.
- `BruteForce` + `LoginThrottleMiddleware` - état de verrouillage
  adossé à torii plus un middleware HTTP qui court-circuite avec
  `429 Too Many Requests` avant que le handler de connexion ne soit
  invoqué.
- `TwoFactor` - inscription TOTP, confirmation, vérification, codes
  de récupération, rotation de secret, le flux de défi complet qui
  filtre une connexion par mot de passe via le second facteur, et
  une protection contre le rejeu à la granularité d'un pas de temps
  de 30 secondes.
- `remember_me` - ré-export de `crate::auth::remember` (cookies
  persistants ligne-BD + bcrypt + rotation à usage unique) pour la
  cohésion de l'espace de noms.

Deux middleware de filtrage de route sont livrés dans le même
espace de noms :

- `EnsureEmailVerifiedMiddleware` - se compose après `AuthMiddleware`
  pour filtrer les routes sur `email_verified_at`.
- `TwoFactorChallengeMiddleware` - se compose devant `AuthMiddleware`
  pour renvoyer une session avec un défi 2FA en attente vers le
  formulaire de défi plutôt que vers la page de connexion.

Chaque message transactionnel est livré via la façade
[`Mail`](mail.md). La feature optionnelle `mailer` de torii est
délibérément désactivée dans `framework/Cargo.toml` : faire tourner
une seconde pile de mail à l'intérieur de torii fractionnerait la
télémétrie, doublerait la surface de configuration de transport, et
forcerait les applications à câbler deux adresses « from ».

### Où vit l'état

La vérification d'e-mail et la réinitialisation de mot de passe sont
**agnostiques du fournisseur**. Les tokens de vérification et de
réinitialisation vivent dans la propre table `auth_flow_tokens` du
framework (à usage unique, hachée en SHA-256), et la recherche +
mutation d'utilisateur passent par le
[`UserProvider`](authentication.md) que l'application a enregistré -
le même fournisseur contre lequel `Auth::user` résout. Il n'y a
aucune instance d'auth globale à initialiser pour ces deux flux : une
application fraîchement scaffoldée a déjà `EloquentUserProvider<User>`
lié, et c'est tout ce dont `EmailVerification` et `PasswordReset` ont
besoin.

Torii possède toujours l'état de sécurité pour les flux qui en
dépendent réellement - le compteur de verrouillage par force brute
par compte, les cérémonies OAuth / passkey / WebAuthn, et le pool de
sessions. Suprnova possède les préoccupations transversales à travers
chaque flux - le mail sortant, le dispatch d'événements, la table
TOTP 2FA, les cookies remember-me, et le middleware HTTP. Le code
applicatif ne touche jamais qu'à `suprnova::auth_flows::*`. Laravel
replie la surface équivalente dans Fortify ; Suprnova garde les
traits de modèle (`MustVerifyEmail` / `CanResetPassword`) et le
magasin de tokens dans le framework, si bien que les flux
fonctionnent contre n'importe quel backend utilisateur.

## Sémantique d'échec à travers les flux

Chaque façade suit une règle d'ordonnancement unique : le changement
d'état durable se commite en premier, puis les effets de bord de
notification se déclenchent. Une panique d'écouteur, un échec
transitoire du transport de mail, ou une erreur de dispatcher après
la mutation ne peuvent pas annuler la mutation.

- `EmailVerification::verify` consomme le token et marque
  l'utilisateur vérifié via le fournisseur avant de déclencher
  `EmailVerified`.
- `PasswordReset::complete` consomme le token et fait d'abord tourner
  le mot de passe via le fournisseur, puis révoque chaque session et
  chaque ligne remember-me de l'utilisateur (journalisé en cas
  d'échec, non remonté), puis distribue `PasswordChangedMail` en
  fire-and-forget, puis déclenche `PasswordResetCompleted`.
- `BruteForce::unlock_account` commite le déverrouillage avant de
  déclencher `AccountUnlocked`.
- `TwoFactor::confirm` estampille `confirmed_at` avant de déclencher
  `TwoFactorEnrolled` ; `TwoFactor::disable` supprime la ligne avant
  de déclencher `TwoFactorDisabled` ; `TwoFactor::complete_challenge`
  fait passer l'état de pending → authed avant de distribuer la
  paire standard `auth::Login` + `auth::Authenticated` suivie de
  `TwoFactorChallenged`.

Un écouteur qui a besoin de durabilité devrait mettre son travail en
tampon (mettre en file un job depuis le corps de l'écouteur) ; la
façade elle-même ne retente jamais.

## Amorçage

La vérification d'e-mail et la réinitialisation de mot de passe sont
adossées à un fournisseur et n'ont **besoin d'aucun torii**. La
protection par force brute et la 2FA ont quand même besoin de torii.
Câblez ce que les flux que vous utilisez requièrent - ils sont
indépendants.

### Vérification d'e-mail + réinitialisation de mot de passe

Trois choses, que possède déjà une application scaffoldée :

1. **Un fournisseur d'utilisateurs qui implémente la surface
   auth-flow.** Enregistrez `EloquentUserProvider<User>` (le même
   fournisseur contre lequel `Auth::user` résout) comme liaison `dyn
   UserProvider` dans `bootstrap.rs::register()`. Les deux façades
   résolvent le fournisseur actif en interne ; aucune instance n'est
   passée au site d'appel.

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **Les deux traits de modèle sur votre `User`.**
   `EloquentUserProvider<User>` n'implémente les méthodes auth-flow
   (`retrieve_by_email` / `mark_email_verified` / `set_password` /
   `is_email_verified`) que quand `User` implémente à la fois
   `MustVerifyEmail` et `CanResetPassword` - les analogues Suprnova
   des contrats `MustVerifyEmail` / `CanResetPassword` de Laravel :

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // La valeur arrive déjà hachée - stockez-la telle quelle.
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` a un défaut qui suit l'horodatage
   (`email_verified_at().is_some()`), et `name()` a par défaut
   `None` - surchargez-la pour saluer les utilisateurs par leur nom
   dans le mail.

3. **Deux colonnes / tables dans votre migrateur.** La table `users`
   a besoin d'un horodatage `email_verified_at` nullable (le
   fournisseur le lit dans `is_email_verified` et l'estampille dans
   `mark_email_verified`), et la table `auth_flow_tokens` à usage
   unique du framework détient les tokens de vérification /
   réinitialisation. Le framework livre le `CREATE` de la table de
   tokens ; listez-la dans votre migrateur :

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   Ajoutez `email_verified_at` à `users` dans votre propre migration
   de colonne (un `timestamp_with_time_zone` nullable) ; `NULL`
   signifie non vérifié, si bien que les lignes existantes se
   rétro-remplissent correctement.

Les tokens sont à usage unique et hachés en SHA-256 au repos - un
dump de base de données ne donne jamais de token en clair utilisable.
Les TTL par défaut sont de **24 heures** pour la vérification
d'e-mail et de **15 minutes** pour la réinitialisation de mot de
passe.

### Force brute + 2FA : câbler torii

`BruteForce` / `LoginThrottleMiddleware` et `TwoFactor` sont adossés
à torii - ils ont besoin que l'instance torii globale soit
initialisée dans `bootstrap.rs::register()`, après `DB::init`.
(OAuth, les passkeys, et les cérémonies WebAuthn passent par la même
instance - voir [Authentification](authentication.md).)

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` est idempotent. La garde `OnceLock` fait que le second
appel est un no-op, si bien que les harnais de test qui re-entrent
dans `register()` par fixture ne migrent pas deux fois. Pour les
tests, substituez `ToriiConfig::sqlite_in_memory()` - cela fait
démarrer une base de données en mémoire à cache partagé qui survit
entre les runtimes :

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

### Enregistrer les migrations 2FA

Le framework livre le schéma ; votre application y adhère en listant
les deux migrations dans son propre migrateur :

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

Les deux sont idempotentes contre une base de données déjà appliquée
(la v1 utilise `CREATE TABLE IF NOT EXISTS` ; la v2 est un ajout de
colonne). Relancer `suprnova migrate` contre une base de données de
production qui a déjà le schéma est un no-op.

### Environnement

Les mailables transactionnels lisent deux variables d'environnement
au moment de l'envoi :

| Var | Défaut | Utilisée pour |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | L'image de marque du sujet et l'étiquette d'émetteur `otpauth://` que les applications d'authentification affichent. |
| `MAIL_FROM` | aucun - **erreur si non défini** | Le `From` d'enveloppe sur chaque message sortant. Positionnez un domaine d'expéditeur vérifié. |

`MAIL_FROM` n'a délibérément aucun défaut. Se replier sur un
placeholder comme `noreply@example.com` casserait silencieusement
DMARC / SPF en production et expédierait depuis un domaine que
l'opérateur ne contrôle pas, si bien que la façade échoue de manière
fermée à la place. `EmailVerification::send_link` et
`PasswordReset::send_link` font remonter l'erreur comme `Err` ;
`PasswordReset::complete` journalise via `tracing::warn!` et continue
(le changement de mot de passe a déjà commité, si bien que le chemin
de notification ne peut pas l'annuler).

Les applications positionnent en plus `APP_URL` afin que les
contrôleurs puissent dériver l'URL de base utilisée dans les appels
`send_link` ; la façade du framework elle-même prend l'URL de base
comme paramètre.

Le driver de mail se configure séparément via `MAIL_DRIVER` - voir la
doc [E-mail](mail.md).

## Vérification d'e-mail

`EmailVerification` produit, vérifie, et consomme des tokens de
vérification contre la table `auth_flow_tokens`, et marque
l'utilisateur vérifié via le fournisseur configuré. Quatre opérations
couvrent le cycle de vie :

| Méthode | Signature | Notes |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Produit + envoie, à partir d'un utilisateur déjà en main. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Anti-énumération : recherche l'utilisateur par e-mail ; une adresse inconnue est un `Ok(())` silencieux. |
| `check` | `check(token: &str) -> Result<bool>` | Non consommant - sûr à appeler sur une page d'atterrissage. |
| `verify` | `verify(token: &str) -> Result<String>` | Usage unique : consomme le token, marque l'utilisateur vérifié, retourne l'id utilisateur. |

```rust
use suprnova::auth_flows::EmailVerification;

// Après une inscription fraîche, avec l'utilisateur qui vient d'être
// créé en main :
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Vérification optionnelle sur la page d'atterrissage - non
// consommante, si bien qu'un rafraîchissement de page ne brûle pas
// le token.
let valid: bool = EmailVerification::check(&token_str).await?;

// Le handler de clic consomme le token et estampille l'utilisateur,
// en retournant l'id de l'utilisateur vérifié.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` déclenche `EmailVerified` en cas de succès - les écouteurs
sont le bon endroit pour débloquer des fonctionnalités
supplémentaires (e-mail de bienvenue, abonnements par défaut, CTA
« complétez votre profil ») sans les coupler au handler de
vérification. L'événement porte l'id utilisateur du fournisseur.

### Le point de terminaison resend (anti-énumération)

`resend` ne prend que l'e-mail - la façade recherche l'utilisateur
via le fournisseur actif et, quand un compte est enregistré, produit
un token et envoie le mail ; un e-mail inconnu est un no-op
silencieux qui retourne quand même `Ok(())`. Le handler ne branche
jamais sur l'existence elle-même, si bien qu'un appelant qui sonde ne
peut pas distinguer « envoyé » de « aucun compte de ce genre » :

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
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` et `resend` construisent tous deux l'URL comme
`{base_url}?token={plaintext_token}`. Une barre oblique de fin sur
`base_url` est retirée avant que la chaîne de requête ne soit
ajoutée, si bien que `https://app.example.com/verify/` et
`https://app.example.com/verify` produisent tous deux une URL propre.

Le handler de clic récupère le token depuis la chaîne de requête et
appelle `verify` :

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

Le handler n'a pas besoin de rechercher l'utilisateur - `verify`
consomme le token, marque l'utilisateur vérifié via le fournisseur,
retourne l'id utilisateur, et déclenche `EmailVerified`. Usage
unique : un second `verify` sur le même token retourne une erreur.

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

`PasswordReset` a trois opérations :

| Méthode | Signature | Notes |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Anti-énumération : recherche l'utilisateur par e-mail ; une adresse inconnue est un `Ok(())` silencieux. |
| `check` | `check(token: &str) -> Result<bool>` | Non consommant - confirmez le token avant de rendre le formulaire de nouveau mot de passe. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Usage unique : consomme le token, fait tourner le mot de passe, révoque sessions + remember-me, envoie la notification de changement, retourne l'id utilisateur. |

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

`complete` hache `new_password` avant de le remettre au fournisseur -
passez le texte en clair, pas une valeur déjà hachée. Un mot de
passe vide / blanc est rejeté en amont avec un `400`.

### Anti-énumération

`send_link` est structuré de sorte que la forme de la réponse ne
fuite jamais si une adresse e-mail a un compte :

- Elle retourne toujours `Ok(())`. Quand l'e-mail est absent, aucun
  token n'est produit, aucun mail n'est distribué, et aucun
  événement `PasswordResetLinkSent` ne se déclenche - mais l'absence
  ne remonte pas non plus via le type de retour, si bien qu'un
  appelant (et un observateur réseau) ne peut pas distinguer « aucun
  compte de ce genre » de « lien envoyé ».
- Le contrôleur dogfood associe `send_link` à un corps de réponse 200
  fixe, si bien qu'un appelant qui sonde ne peut pas distinguer via
  le code de statut, le corps de réponse, ou le minutage de la
  réponse.

### Effets de bord de `complete`

`complete` exécute quatre étapes dans l'ordre :

1. Consomme le token (usage unique) et fait tourner le hash de mot de
   passe via le fournisseur configuré (la seule étape qui peut faire
   échouer l'appel).
2. Révoque chaque ligne de session de l'utilisateur via
   `crate::session::destroy_all_for_user` (best-effort : les échecs
   font un `tracing::warn!`).
3. Révoque chaque ligne remember-me via
   `crate::auth::remember::revoke_all_for_user` (best-effort).
4. Distribue `PasswordChangedMail` en fire-and-forget, puis déclenche
   `PasswordResetCompleted`.

Une session volée et un cookie remember-me capturé ne doivent pas
survivre à l'identifiant dont ils dépendaient. Les révocations se
produisent à chaque réinitialisation réussie, pas seulement celles
initiées par l'utilisateur, si bien qu'une réinitialisation forcée
par une équipe de sécurité éjecte aussi un attaquant actif.

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
- En-tête `Retry-After` - en secondes, calculé depuis le
  `locked_until` du verrouillage via
  `LockoutStatus::retry_after_seconds`. Se replie sur `900` (15
  minutes - la période de verrouillage par défaut de torii) si
  l'horodatage est d'une manière ou d'une autre absent.
- Corps : `"Account locked due to too many failed login attempts. Try
  again later."`

### Échec ouvert sur les erreurs de backend

Si `get_lockout_status` retourne un `Err` (accroc transitoire de base
de données), le middleware laisse passer la requête. Le handler de
connexion en aval fera alors lui-même l'appel et pourra décider
d'échouer de manière fermée ou ouverte. Le middleware pèche du côté
de la disponibilité : mettre à terre le point de terminaison de
connexion chaque fois que la base de données d'auth a un soubresaut
est pire que de laisser le handler faire l'appel directement.

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

Le `BruteForceProtectionConfig` de torii a pour défaut **5
tentatives échouées avant verrouillage** et une **période de
verrouillage de 15 minutes**. C'est ce que câble `init_torii`
aujourd'hui ; configurer des valeurs par application exige d'aller
chercher dans la surface de configuration propre à torii, et ce n'est
pas exposé via le builder `ToriiConfig` de Suprnova. Les valeurs par
défaut sont délibérément conservatrices - acceptez « cinq fautes de
frappe me verrouillent pour 15 minutes » avant de décider de les
assouplir.

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

`user_id` est la clé de stockage opaque - typiquement
`torii::UserId.as_str()`, mais n'importe quel identifiant stable par
utilisateur fonctionne. La table 2FA s'indexe sur elle ; il n'y a pas
de FK vers votre table utilisateur.

`email` est replié dans le segment `account_name` de l'URL
`otpauth://`, si bien que l'application d'authentification affiche la
ligne avec une étiquette lisible par un humain (par exemple « MyCorp
(alice@example.com) »).

Un motif courant est un petit newtype qui enveloppe votre modèle
utilisateur :

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
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

Le `user_id` de la 2FA est délibérément un `String`. S'il était typé
comme `i64`, `Uuid`, ou `torii::UserId`, la table 2FA serait
définitivement liée à la forme choisie en premier par le framework -
les applications qui stockent leurs utilisateurs sous une forme
différente (UUID contre entiers auto-incrémentés, ou des applications
qui n'utilisent pas torii du tout mais veulent le module 2FA) seraient
exclues. Un `user_id` sous forme de chaîne laisse chaque application
choisir l'identifiant stable par utilisateur qu'elle préfère ; le
compromis est un `.to_string()` au site d'appel. Le Fortify de
Laravel lie la colonne équivalente à `User::id` d'Eloquent - Suprnova
la découple, si bien que `TwoFactor` est une primitive de cycle de
vie réutilisable, pas un accessoire à la forme de User.

## Se souvenir de moi

`suprnova::auth_flows::remember_me` ré-exporte
`suprnova::auth::remember` - le module de cookie persistant qui était
déjà livré aux côtés de l'auth par session. Le ré-export est
purement organisationnel : tout ce qui a la forme d'un auth-flow vit
sous `auth_flows::*`, même quand l'implémentation précède cet espace
de noms.

La conception livrée :

- **Ligne-BD + hash bcrypt** - chaque token émis a une ligne dans la
  table `remember_tokens` qui ne stocke que le hash bcrypt, jamais le
  texte en clair. Un dump de base de données ne peut pas donner des
  identifiants permettant de se ré-authentifier.
- **Rotation à usage unique** - une vérification réussie fait un
  DELETE sur la ligne correspondante et en émet une fraîche. Un
  cookie capturé ne peut pas être réutilisé ; si un attaquant et une
  victime sont en course pour l'utiliser, le perdant voit la ligne
  disparue et échoue à s'authentifier.
- **Révocation** - `revoke_all_for_user` efface chaque ligne d'un
  utilisateur en un seul DELETE. `Auth::logout` enchaîne cela, si
  bien qu'une vraie déconnexion efface réellement l'état persistant,
  et `PasswordReset::complete` fait de même, si bien qu'une
  réinitialisation de mot de passe invalide chaque cookie persistant
  existant.
- **Purge** - `prune_expired` nettoie les lignes expirées suivant un
  planning.

En pratique, le middleware de session du framework fait le gros du
travail ; l'application typique n'appelle pas directement le module
`remember_me`. La doc [Authentification](authentication.md) couvre
la surface visible côté utilisateur - le flag `remember` sur
`Auth::login`, le nom du cookie, et les réglages de durée de vie.

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

### Tests d'intégration pour la vérification d'e-mail + la réinitialisation de mot de passe

Les tests de verify / reset n'ont besoin d'aucun torii -
provisionnez la table `auth_flow_tokens` sur une base de données en
mémoire, enregistrez un fournisseur, positionnez `MAIL_FROM`, et
pilotez la façade sous `Mail::fake()`. Les propres tests du framework
produisent la table directement depuis
`create_auth_flow_tokens_table()` :

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // Les façades lisent MAIL_FROM (échec fermé) ; positionnez-la
    // pour le test.
    // SAFETY : sérialisé par `#[serial]` - aucun observateur parallèle.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // ... piloter EmailVerification::send_link(&user, base) ...
    fake.assert_sent_to("ada@example.com");
}
```

Les chemins adossés à un fournisseur (`resend` / `verify` /
`complete`) enregistrent en plus une liaison `dyn UserProvider` afin
que la recherche + mutation se résolvent - voir
`framework/tests/email_verify.rs` et
`framework/tests/password_reset.rs`.

### `ToriiConfig::sqlite_in_memory()` pour les tests de force brute + 2FA

Les tests de force brute et de 2FA font démarrer un torii frais sur
une base de données SQLite en mémoire. Les fichiers de test
d'exemple dans `framework/tests/` utilisent un motif runtime partagé +
`once_cell::sync::Lazy<()>` pour amortir le coût entre les tests,
plus `#[serial]` pour garder le transport de mail global au
processus stable entre les tests qui entrelacent `Mail::fake()` :

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // ... utilisez Mail::fake() / EventFacade::fake() ici ...
    });
}
```

Exemples canoniques - copiez-les quand vous écrivez les vôtres :

- `framework/tests/email_verify.rs` - aller-retour de token verify,
  découpe de la barre oblique de fin de `send_link`, assertions
  `Mail::fake()` sur le sujet/HTML.
- `framework/tests/password_reset.rs` - aller-retour de
  réinitialisation avec authentification par le nouveau mot de
  passe, anti-énumération sur les e-mails inconnus, `complete`
  rejette les tokens réutilisés.
- `framework/tests/brute_force.rs` - cycle de vie complet du
  verrouillage, `AccountLocked` se déclenche une fois par transition,
  `unlock_account` retourne `was_locked`.
- `framework/tests/two_factor.rs` - enroll → confirm → verify complet
  avec un vrai code TOTP calculé depuis l'URL otpauth, usage unique
  des codes de récupération, la ré-inscription écrase le secret,
  rejet du rejeu à travers deux vérifs concurrentes.
- `framework/tests/two_factor_challenge_flow.rs` - le flux de défi de
  bout en bout avec rotation de session, réémission de remember-me,
  et distribution d'événements.
- `framework/tests/email_verified_middleware.rs` et
  `two_factor_challenge_middleware.rs` - formes de réponse du
  middleware (403 JSON contre 302 contre 409 + X-Inertia-Location).

## Référence

| Symbole | Objectif |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, `verify` - adossé à un fournisseur ; `verify` retourne l'id utilisateur. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` pour 403 JSON, `redirect_to(path)` pour 302 / 409 + X-Inertia-Location. Vérifie l'`is_email_verified` du fournisseur configuré (échec fermé). |
| `suprnova::auth_flows::PasswordReset` | `send_link`, `check`, `complete` - adossé à un fournisseur ; `complete` retourne l'id utilisateur. |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | Traits de modèle qu'un utilisateur derrière `EloquentUserProvider` implémente afin que les façades verify / reset puissent lire son e-mail + écrire son horodatage de vérification / hash de mot de passe. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | `CREATE TABLE` SeaORM pour `auth_flow_tokens` - à lister dans votre migrateur. |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`, `reset_attempts`, `get_lockout_status`, `is_locked`, `unlock_account`. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | Middleware HTTP qui répond 429 avant le handler quand le compte ciblé est verrouillé. |
| `suprnova::auth_flows::TwoFactor` | `enroll`, `re_enroll`, `confirm`, `verify`, `consume_recovery_code`, `regenerate_recovery_codes`, `is_enabled`, `is_enabled_by_id`, `start_challenge`, `pending_user_id`, `cancel_challenge`, `complete_challenge`, `disable`. |
| `suprnova::auth_flows::TwoFactorUser` | Trait faisant le pont entre le modèle utilisateur de l'application et la façade 2FA. |
| `suprnova::auth_flows::EnrollmentResponse` | Valeur de retour de `TwoFactor::enroll` - `otpauth_url`, `qr_code_svg`, `recovery_codes`. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | `new()` pour 403 JSON, `redirect_to(path)` pour 302 / 409 + X-Inertia-Location. À composer devant `AuthMiddleware`. |
| `suprnova::auth_flows::two_factor::migration::Migration` | Migration SeaORM pour `two_factor_credentials`. À lister dans votre `Migrator::migrations()`. |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | Ajout de colonne pour `last_used_timestep` (protection contre le rejeu TOTP). À lister après la migration de création de table. |
| `suprnova::auth_flows::remember_me` | Ré-export de `suprnova::auth::remember`. |
| `suprnova::auth_flows::events::*` | Neuf événements - voir [Événements](#événements). |
| `suprnova::auth_flows::EmailVerificationMail` | Mailable transactionnel. Sujet `"Verify your email for {APP_NAME}"`. |
| `suprnova::auth_flows::PasswordResetMail` | Mailable transactionnel. Sujet `"Reset your {APP_NAME} password"`. |
| `suprnova::auth_flows::PasswordChangedMail` | Mailable de notification de sécurité. Sujet `"Your {APP_NAME} password was changed"`. |
| `suprnova::torii_integration::ToriiConfig` | Config de bootstrap de torii. `from_sea_orm(conn)` pour la production, `sqlite_in_memory()` pour les tests. |
| `suprnova::torii_integration::init_torii` | Init globale idempotente. À appeler une fois depuis `bootstrap.rs::register()`. |

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
