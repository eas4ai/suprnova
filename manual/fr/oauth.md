# OAuth, Apple et connexion par lien magique

Suprnova expose OAuth, Sign in with Apple et les liens magiques sans mot de passe via la façade `Auth` possédée par le framework. Magnetar fournit les moteurs de credentials, de cérémonies, d'identités, de porte de facteur et de sessions derrière cette façade.

Les points d'entrée publics sont :

- `Auth::oauth(provider)` pour OAuth et Apple.
- `Auth::magic_link()` pour la connexion e-mail sans mot de passe.

Suprnova n'installe pas de routes pour ces flux. Les applications fournissent de petits handlers de démarrage et de callback, et décident comment livrer l'e-mail de lien magique.

## Initialiser Magnetar avec OAuth

Configurez OAuth sur la même `MagnetarConfig` qui initialise les services de mot de passe, de clé d’accès, de session, de verrouillage et d’authentification à deux facteurs. Le registre des fournisseurs est publié de manière atomique avec ces services : si un service ne peut pas être construit, aucun d’eux ne devient visible.

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

Le framework réexporte le contrat `OAuthProvider`, les cinq fournisseurs internes et les types de configuration, ainsi que tous les types nécessaires pour implémenter un fournisseur personnalisé. `ReqwestOAuthTransport` fournit les E/S de production pour les jetons, userinfo et la révocation. `FrameworkAbuseLimiter` utilise le `RateLimiterDriver` configuré par l'application. Les applications n'ont besoin ni d'une dépendance directe à `suprnova-magnetar` ni d'adaptateurs de transport et de limiteur écrits à la main.

`MagnetarConfig` crée son schéma lorsque `apply_migrations` est activé, ce qui est le comportement par défaut. Utilisez `.apply_migrations(false)` uniquement lorsque le déploiement prépare séparément le même schéma. Une seconde initialisation renvoie une erreur au lieu de remplacer un moteur installé.

### Conserver les utilisateurs et les sessions existants

Une application peut utiliser Magnetar pour les cérémonies OAuth et la preuve
du fournisseur sans faire de Magnetar l'autorité pour les mots de passe, les
passkeys, les sessions du framework ou l'état remember-me. Construisez le même
`MagnetarOAuthHostConfig`, puis installez-le avec l'initialiseur OAuth
uniquement :

```rust,no_run
use suprnova::{
    MagnetarOAuthOnlyConfig, init_magnetar_oauth_only,
};

let database = DB::connection()?;
init_magnetar_oauth_only(
    MagnetarOAuthOnlyConfig::from_sea_orm(
        database.inner().clone(),
        oauth,
    ),
)
.await?;
```

Démarrez la cérémonie normalement avec `Auth::oauth(provider).begin()`. Dans
le callback, appelez `verify_oauth_identity(code, state)`, associez le subject
vérifié du fournisseur à la table utilisateur de l'application, puis
établissez la session existante du framework avec `Auth::login`. N'appelez pas
`complete` dans ce mode : `complete` applique l'association de comptes et de
sessions par défaut de Magnetar, tandis que l'initialisation OAuth uniquement
laisse ces décisions à l'application.

L'initialisation OAuth uniquement et l'initialisation complète par défaut sont
des alternatives. Un second initialiseur échoue au lieu de mélanger deux
autorités de session.

### Exigences du fournisseur GitHub

Le point de terminaison utilisateur REST de GitHub exige un `User-Agent`; un fournisseur communautaire l'ajoute, ainsi que toute valeur `Accept` de type média dont il a besoin, via `OAuthProvider::userinfo_headers`. Suprnova ajoute séparément l'en-tête bearer `Authorization` et rejette les tentatives du fournisseur de le remplacer.

La réponse `/user` de GitHub inclut une adresse e-mail uniquement lorsque l'utilisateur l'a rendue publique. L'adresse principale vérifiée nécessite une seconde requête `/user/emails`, tandis que `resolve_identity` n'effectue délibérément aucune E/S et reçoit une réponse userinfo. Un fournisseur GitHub peut renvoyer `email: None` et utiliser la cérémonie de complétion d'e-mail de Suprnova, ou faire pointer `userinfo_endpoint` vers un adaptateur hôte qui combine `/user` avec l'e-mail principal vérifié. Ne traitez pas une adresse non vérifiée ou simplement publique comme une preuve de propriété du compte.

## Liaison de session

Le démarrage OAuth exige `SessionMiddleware`. Magnetar lie la cérémonie à un digest de la session framework initiatrice, afin que le callback ne puisse pas être déplacé vers une autre session de navigateur.

Une connexion réussie par mot de passe, lien magique, passkey ou OAuth fait tourner l'ID de session framework et le token CSRF, enregistre l'ID utilisateur applicatif, et stocke une liaison web opaque Magnetar. L'hydratation remember-me fait tourner à la fois le credential Magnetar et la liaison de session framework.

## Démarrer un flux OAuth

Utilisez `begin` dans le handler de démarrage du fournisseur :

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// Renvoyez une redirection HTTP vers kickoff.authorization_url.
```

Le `OAuthKickoff` retourné contient :

- `authorization_url`, l'URL à envoyer au navigateur.
- `state`, le sélecteur à usage unique lié à la session initiatrice.

Magnetar possède la génération de state, la politique PKCE, la persistance de cérémonie, l'échange avec le fournisseur, la vérification d'identité et la limitation des abus. Le contrôleur hôte possède la redirection HTTP et la route de callback.

## Vérifier ou terminer le callback

Le callback possède deux points d'entrée :

| Méthode | Résultat | Effets de bord |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | Vérifie la preuve du fournisseur et retourne le fournisseur, le sujet, l'e-mail vérifié et le nom d'affichage sans créer de session applicative. |
| `complete(code, state)` | `(User, Session)` | Résout l'identité via le moteur hôte installé, applique la politique de liaison de compte et la porte de facteur, fait tourner la session framework, et retourne l'utilisateur possédé par le framework et les valeurs de session Magnetar. |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` n'est présent que lorsque le fournisseur a livré un e-mail vérifié. Persistez le fournisseur et le sujet comme identité externe stable. L'e-mail n'est pas un identifiant de fournisseur stable.

## Politique de liaison de compte

La complétion OAuth ne considère pas la possession d'une chaîne d'e-mail non vérifiée comme la preuve que l'appelant possède un compte applicatif existant.

Le résultat de complétion peut exiger davantage de travail au lieu d'émettre une session :

- **Complétion d'e-mail requise** retourne HTTP 409 lorsque l'identité du fournisseur exige une cérémonie séparée d'e-mail vérifié.
- **Liaison explicite requise** retourne HTTP 409 lorsqu'un compte vérifié existant doit autoriser la liaison.
- **Facteur requis** retourne HTTP 401 lorsque la politique de compte exige un second facteur avant l'émission de session.

Une complétion d'e-mail vérifié qui gagne la limite de première preuve d'e-mail récupère atomiquement un compte non vérifié squatté. La transaction fait avancer l'ère d'authentification, supprime les credentials provisoires, révoque les anciennes sessions et credentials remember, et attache le compte fournisseur vérifié. Un compte vérifié n'est jamais lié automatiquement par le seul e-mail.

## Sign in with Apple

Apple utilise la même façade `Auth::oauth("apple")`, mais son callback utilise couramment `response_mode=form_post`. Enregistrez le callback comme route `POST` et transmettez le champ de formulaire Apple optionnel `user` aux méthodes spécifiques à Apple :

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` inclut le sujet stable, l'e-mail vérifié optionnel, `email_verified` et `is_private_email`. Persistez le sujet comme clé stable. Apple ne peut fournir le nom d'affichage que lors de la première autorisation ; l'adaptateur de fournisseur doit donc préserver cette première valeur `form_post`.

La vérification de token et d'identité Apple appartient à l'implémentation de fournisseur installée. Les fournisseurs Magnetar actuels exigent les vérifications de signature, émetteur, audience, expiration et nonce plutôt que de faire confiance au JSON décodé d'un ID token.

## Connexion par lien magique

La connexion par lien magique utilise le moteur password/session Magnetar installé. Le framework retourne le token à usage unique en clair, tandis que l'application possède la composition du mail et la forme de l'URL :

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` applique le budget d'abus d'authentification avant l'émission du token. `consume` est à usage unique, applique la porte de facteur, lie la session obtenue à la session de requête framework, et retourne l'utilisateur et la session Magnetar.

Pour un compte préexistant non vérifié, la consommation réussie d'un lien magique constitue une première preuve d'e-mail. La transaction récupère le compte et supprime l'état provisoire de mot de passe, passkey, compte lié, deux facteurs, session et remember afin qu'un squatteur antérieur ne puisse pas conserver l'accès.

## Routes à ajouter

Une application typique ajoute ces routes :

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

Appliquez `SessionMiddleware` à chaque route de démarrage/callback OAuth et passkey. La session porte le sélecteur de cérémonie et lie l'aller-retour au navigateur qui l'a démarré.

## Migration d'authentification

La crate `suprnova-magnetar` inclut un moteur de migration sensible à la forme pour les schémas Torii, Suprnova web, Suprnova API et Magnetar existants. C'est une surface de bibliothèque et un exemple, non une sous-commande CLI `suprnova`.

Activez la fonctionnalité `migration` ainsi que le driver de base source, puis exécutez un plan à blanc avant de l'appliquer. Pour PostgreSQL :

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

Utilisez `seaorm-mysql` ou `seaorm-sqlite` à la place lorsque c'est le driver de base de données source et applicatif.

Ajoutez `--apply` pour appliquer le plan révisé. L'exécuteur revérifie les empreintes de source et de schéma avant l'importation, enregistre l'état de nouvelle tentative, refuse les collisions d'identité et utilise des imports transactionnels. Les migrations MySQL dans la même base utilisent un échange shadow protégé par barrière d'écriture, avec des chemins de restauration et d'abandon reprenables.

Conservez le plan et le rapport générés dans les archives de déploiement. N'appliquez pas un plan dont l'empreinte source a changé après révision.

## Référence

- Amorçage par défaut : `MagnetarConfig`, `PasskeyConfig` et `init_magnetar`.
- Façades : `Auth::oauth(provider)` et `Auth::magic_link()`.
- Installation OAuth : `MagnetarConfig::oauth`, `ReqwestOAuthTransport` et `FrameworkAbuseLimiter`.
- Bibliothèque de migration : `magnetar::migration` depuis la crate `suprnova-magnetar`.
- Authentification bearer : `BearerTokenMiddleware`.

## Suivant

- [Authentification](authentication.md) couvre mot de passe, passkey, guards, sessions framework et initialisation du moteur.
- [Flux d'authentification](auth-flows.md) couvre vérification d'e-mail, réinitialisation de mot de passe, verrouillage et authentification deux facteurs.
- [E-mail](mail.md) couvre la livraison de lien magique possédée par l'application.
- [Session](session.md) couvre la session navigateur qui lie les cérémonies OAuth et passkey.
