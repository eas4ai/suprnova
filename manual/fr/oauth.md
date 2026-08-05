# OAuth, Apple et connexion par lien magique

Suprnova livre trois méthodes de connexion adossées à torii derrière la
façade `Auth` : l'**OAuth générique** (GitHub, Google, ou tout provider
OIDC/OAuth2), la **connexion avec Apple**, et les **liens magiques**
sans mot de passe. Elles partagent un seul prérequis (`init_torii` plus
la migration de cérémonie) et la même forme de façade -
`Auth::oauth(provider)` / `Auth::magic_link()` - et aucune d'elles ne
livre de routes : vous ajoutez un contrôleur mince (démarrage +
callback) et le framework s'occupe de l'état CSRF, de PKCE, de
l'échange de token, de la vérification d'identité, de l'upsert
utilisateur, et de la création de session.

Toute la surface vit dans `framework/src/torii_integration/`. Il n'y a
**aucun** contrat de variable d'env du framework pour tout cela -
chaque identifiant est passé de manière programmatique (récupérez le
vôtre depuis l'environnement) ; les exemples de ce chapitre utilisent
`std::env::var(...)` uniquement pour montrer où vont vos secrets.

## Prérequis

1. **Initialisez torii une fois au démarrage** - cela alimente
   l'upsert utilisateur et la création de session :

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // dans bootstrap::register(), après DB::init()
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **Exécutez la migration de cérémonie.** OAuth et Apple rangent une
   cérémonie CSRF-`state` + PKCE de courte durée (10 minutes) dans la
   table `auth_ceremony_tokens`. Enregistrez la migration
   `m20251209_000000_create_auth_ceremony_tokens_table` dans votre
   `Migrator` (les kits de démarrage l'incluent déjà). Planifiez
   éventuellement
   `suprnova::torii_integration::ceremony::prune_expired()` pour purger
   les lignes périmées.

3. **`SessionMiddleware` sur la route de *démarrage* OAuth.** `begin()`
   écrit le `state` dans la session ; un appel sans session échoue
   avec un 500.

Les liens magiques n'ont besoin que de l'étape 1.

## OAuth générique (GitHub, Google, personnalisé)

### Configurer un fournisseur

Enregistrez chaque fournisseur une fois au démarrage. Le registre est
global au processus et idempotent, si bien que réenregistrer le même
fournisseur ne fait que remplacer la config :

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → la table prédéfinie intégrée
    apple_key_pair: None,       // Apple uniquement ; laissez None pour GitHub/Google
    apple_team_id: None,        // Apple uniquement
});
```

Les points de terminaison authorize/token/userinfo prédéfinis sont
intégrés pour `github`, `google`, et `apple`. Pour tout autre
fournisseur - ou un serveur auto-hébergé / de test - fournissez-les
vous-même :

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // repli /emails façon GitHub pour une adresse principale privée
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### Démarrer le flux (URL d'autorisation)

```rust
// GET /auth/oauth/github/start  (la route DOIT porter SessionMiddleware)
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - redirigez le navigateur ici
// kickoff.state - state CSRF, déjà stocké dans la session pour vous
```

`begin()` produit le `state` CSRF (UUID v4) et un vérificateur/défi
S256 PKCE conforme à la RFC 7636, enregistre la cérémonie (TTL de 10
minutes), et retourne l'URL d'autorisation du fournisseur. Redirigez
l'utilisateur vers `authorization_url`.

### Terminer le flux - `verify` vs `complete`

Sur le callback, vous disposez de deux points d'entrée (scindés en
0.5.4). Choisissez selon que votre table `users` **est** ou non le
schéma de torii :

| Méthode | Retourne | Effets de bord | À utiliser quand |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **Aucun** - vérifie la cérémonie, échange le code, récupère les userinfo, extrait un e-mail vérifié + un `subject` stable. Pas d'utilisateur, pas de session. | Votre application possède sa propre table `users` et vous voulez chercher / créer l'utilisateur vous-même. |
| `complete(code, state)` | `(User, Session)` | Fait l'upsert de l'utilisateur dans torii (`get_or_create_user`) et produit une session. | Votre table `users` est le schéma de torii. |

```rust
// Table users personnalisée :
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject est l'id stable du fournisseur ; id.email est vérifié ou None.
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// …ou, adossé à torii :
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

Un `email` retourné par `verify` est toujours une adresse *vérifiée*
(`email_verified` OIDC, GitHub traité comme vérifié, ou le repli
`/emails`) ; un e-mail non vérifié ou absent revient sous forme de
`None`, et les connexions répétées se résolvent par `subject`.

### Routes que vous ajoutez

Le framework ne fournit aucune route OAuth - câblez deux handlers
minces (reproduisez la forme des contrôleurs `auth_verify` /
`auth_reset` existants dans le kit de démarrage) :

```rust
// start - redirige vers le fournisseur
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// callback - GitHub/Google utilisent GET ?code&state
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

Placez la route `/start` (au minimum) derrière `SessionMiddleware`.

## Connexion avec Apple

Apple utilise la même façade - `Auth::oauth("apple")` - avec quelques
règles spécifiques à Apple intégrées en dur :

- **Le callback est un `POST`.** Apple utilise
  `response_mode=form_post`, si bien que la redirection livre `code` +
  `state` dans un corps de formulaire, pas dans des paramètres de
  requête. Enregistrez le callback Apple comme une route `post!` et
  lisez les champs depuis le formulaire.
- **Pas de PKCE.** Apple rejette `code_challenge`, donc l'URL
  d'autorisation l'omet (le secret client est à la place un JWT
  signé).
- **`client_secret` est inutilisé** - laissez-le à `String::new()`.
  Suprnova produit le secret client JWT de courte durée à partir de
  votre clé `.p8` à chaque échange de token.
- **Les ID tokens sont vérifiés par rapport au JWKS d'Apple (RS256)**
  depuis la 0.5.6, et non plus acceptés sur la seule foi de leur
  structure.

### Fournir votre clé Apple - `AppleKeyPair`

`AppleKeyPair` est le seul type Apple ré-exporté pour les
applications (vous n'avez donc besoin d'aucune dépendance directe vers
`apple`). Construisez-le à partir de votre clé de signature `.p8` :

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // *Key ID* Apple (pas le Team ID)
    &std::env::var("APPLE_P8_PATH")?,  // chemin vers AuthKey_XXXXXX.p8
)?;
// ou : AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### Configurer Apple

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // votre Services ID
    client_secret: String::new(),                  // inutilisé - produit à partir de la clé
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // Team ID de 10 caractères
});
```

### Terminer le flux Apple

Même scission que pour l'OAuth générique. `complete` fait l'upsert +
la session ; le chemin verify retourne une `AppleIdentity` pour une
table users personnalisée :

```rust
// POST /auth/apple/callback - lisez code + state depuis le corps FORM
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// …ou table users personnalisée :
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id : AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` n'est `Some(_)` que quand Apple affirme qu'il
est vérifié ; un e-mail non vérifié est refusé (401) avant même que
l'identité ne soit construite. `is_private_email` est positionné quand
l'utilisateur a choisi l'adresse de relais privé d'Apple - persistez
le `subject` comme clé stable, puisque l'adresse de relais est le seul
e-mail que vous obtiendrez.

## Connexion par lien magique

Connexion par e-mail sans mot de passe, adossée à torii, via
`Auth::magic_link()`. Le framework émet et vérifie le token ; **c'est
vous** qui envoyez le lien par e-mail (il n'envoie lui-même jamais de
mail), ce qui se compose proprement avec le chapitre [E-mail](mail.md).

```rust
use suprnova::Auth;

// POST /auth/magic - demande un lien
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// Construisez le lien et envoyez-le vous-même par e-mail :
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - le consomme (usage unique ; un second appel échoue)
let (user, session) = Auth::magic_link().consume(&token).await?;
```

L'utilisateur est créé automatiquement au premier usage. `send`
retourne le token **en clair** afin que vous contrôliez la forme de
l'URL et la livraison.

> **Remarque - `TokenPurpose::MagicLink`.** L'enum `TokenPurpose` de
> `auth_flows` a une variante `MagicLink` (ajoutée en 0.5.5), mais
> c'est un *discriminant réservé* pour le `TokenStore` générique -
> aucun flux intégré ne le consomme. Le chemin de lien magique
> fonctionnel et pris en charge est `Auth::magic_link()` ci-dessus. Ne
> recourez à `TokenPurpose::MagicLink` que si vous bricolez votre
> propre flux sur la table `auth_flow_tokens`.

## Une remarque sur la configuration

Aucune de ces méthodes ne lit de variable d'environnement du
framework - les id de fournisseur, les secrets, les URL de
redirection, et les clés Apple sont tous passés à `configure(...)` de
manière programmatique. Chargez-les comme vous voulez
(`std::env::var`, une struct de config typée, un gestionnaire de
secrets) et enregistrez les fournisseurs une fois pendant le
`bootstrap`. Cela garde les configurations de fournisseur multi-tenant
/ par déploiement de premier rang, au lieu d'imposer un schéma de
nommage de variable d'env fixe.

## Référence

- Points d'entrée de la façade : `Auth::oauth(provider)`,
  `Auth::magic_link()` (`suprnova::Auth`)
- Config : `suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- Résultats OAuth : `OAuthKickoff { authorization_url, state }`,
  `OAuthIdentity { provider, subject, email, name }`,
  `AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- Bootstrap : `suprnova::{init_torii, ToriiConfig}`
- Magasin de cérémonies : table `auth_ceremony_tokens` +
  `suprnova::torii_integration::ceremony::prune_expired()`

## Suivant

- [Authentification](authentication.md) - guards, fournisseurs, et le
  modèle utilisateur `Authenticatable` pour lequel ces flux créent des
  sessions
- [Flux d'authentification](auth-flows.md) - vérification d'e-mail,
  réinitialisation de mot de passe, et 2FA
- [E-mail](mail.md) - envoyer l'e-mail de lien magique (et la config
  d'expéditeur `MAIL_FROM` / `MAIL_FROM_NAME`)
- [Sessions](session.md) - ce qu'est la `Session` retournée et comment
  elle est persistée
