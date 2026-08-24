# OAuth, Apple et connexion par lien magique

Suprnova expose OAuth, Sign in with Apple et les liens magiques sans mot de passe via la façade `Auth` possédée par le framework. Magnetar fournit les moteurs de credentials, de cérémonies, d'identités, de porte de facteur et de sessions derrière cette façade.

Les points d'entrée publics sont :

- `Auth::oauth(provider)` pour OAuth et Apple.
- `Auth::magic_link()` pour la connexion e-mail sans mot de passe.

Suprnova n'installe pas de routes pour ces flux. Les applications fournissent de petits handlers de démarrage et de callback, et décident comment livrer l'e-mail de lien magique.

## Initialiser Magnetar

Initialisez les moteurs par défaut de mot de passe, passkey, session, verrouillage et deux facteurs après `DB::init` et après que `APP_KEY` a initialisé `Crypt` :

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`MagnetarConfig` utilise la connexion SeaORM de l'application. Le moteur par défaut crée son schéma lorsque `apply_migrations` est activé, ce qui est le défaut. Définissez `.apply_migrations(false)` uniquement lorsque le déploiement exécute séparément la même configuration du schéma.

`init_magnetar` installe atomiquement les adaptateurs password/session et passkey. Une seconde installation retourne une erreur au lieu de remplacer le moteur et de scinder l'état d'authentification.

## Installation du moteur OAuth

OAuth est compilé dans la fonctionnalité par défaut `magnetar-oauth` du framework, mais l'enregistrement du fournisseur reste toujours une étape d'exécution explicite. Dans une build `--no-default-features`, activez `magnetar-oauth` explicitement. `init_magnetar` n'expose ni ne retourne son moteur hôte concret interne, donc l'exemple ci-dessous ne s'applique qu'à une application qui construit et conserve son propre `MagnetarHostEngine` ; il ne peut pas être ajouté à l'exemple d'initialisation par défaut précédent. L'API publique actuelle n'a pas de méthode de commodité pour ajouter un registre OAuth à un moteur déjà installé via `MagnetarConfig`.
```rust,ignore
use std::sync::Arc;
use suprnova::magnetar_integration::install_magnetar_oauth_engine;


// These values must be in the scope that constructed the custom host engine.
let oauth = host_engine.oauth_service(oauth_host_config)?;
install_magnetar_oauth_engine(Arc::new(oauth))?;
```

`MagnetarOAuthHostConfig` prend une liste explicite de valeurs
`MagnetarOAuthProviderConfig`, un transport HTTP, un limiteur d'abus, une
politique d'autorisation et une politique de liaison automatique. Le registre
de fournisseurs devient la source d'autorité une fois installé. Un fournisseur
inconnu échoue de manière fermée au lieu de se rabattre sur une autre
implémentation d'authentification.

Les implémentations de fournisseurs et leurs dossiers d'authentification du
client proviennent de la crate `suprnova-magnetar`. Les applications qui
construisent le moteur OAuth doivent ajouter cette crate comme dépendance
directe avec les fonctionnalités des fournisseurs qu'elles utilisent. Le
framework ne déduit pas les identifiants client OAuth ni les secrets depuis les
variables d'environnement. Lisez-les via la configuration de l'application ou
un gestionnaire de secrets et construisez le registre des fournisseurs pendant
le bootstrap.


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
- Installation OAuth : `suprnova::magnetar_integration::install_magnetar_oauth_engine` et les types de configuration dans `suprnova::magnetar_integration::engine`.
- Bibliothèque de migration : `magnetar::migration` depuis la crate `suprnova-magnetar`.
- Authentification bearer : `BearerTokenMiddleware`.

## Suivant

- [Authentification](authentication.md) couvre mot de passe, passkey, guards, sessions framework et initialisation du moteur.
- [Flux d'authentification](auth-flows.md) couvre vérification d'e-mail, réinitialisation de mot de passe, verrouillage et authentification deux facteurs.
- [E-mail](mail.md) couvre la livraison de lien magique possédée par l'application.
- [Session](session.md) couvre la session navigateur qui lie les cérémonies OAuth et passkey.
