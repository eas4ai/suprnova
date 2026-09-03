# Live

Suprnova Live est le moteur d'interaction piloté par le serveur du framework.
Un composant Live est une struct Rust dont l'état vit sur le serveur, dont la
vue est un template Askama et dont les actions s'exécutent via un protocole
signé depuis un petit runtime navigateur qui morphe sur place le HTML
re-rendu. Il n'y a aucun modèle d'état côté client à garder synchronisé,
aucun outil de build à installer pour utiliser le runtime livré et aucun
JavaScript inline dans vos documents.

Ce chapitre couvre la surface côté application : écrire un composant,
l'enregistrer, servir des documents et des îlots, les frontières de sécurité
que franchit chaque requête Live, les téléversements, les mises à jour
asynchrones, les assets, les tests, le diagnostic et la récupération. Tout ce
qui suit n'utilise que `suprnova::live` et `suprnova::view`.

## Démarrage rapide

Un projet créé par `suprnova new` est prêt pour Live : il livre
`src/live/mod.rs` avec un registre de composants vide et une fonction
`routes()`, son bootstrap lie le registre et `cmd/main.rs` installe les
routes. Générez un composant, puis vérifiez-le :

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` écrit `src/live/counter.rs` et `templates/live/counter.html`,
enregistre le composant dans `src/live/mod.rs` et affiche les étapes
suivantes. `live:check` compile votre application et prouve chaque vue
enregistrée avec le vérificateur intégré.

## Écrire un composant

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` est le nom enregistré du composant. Utilisez un nom pointé en
  kebab-case tel que `app.counter` ; la CLI dérive `<package>.<kebab>`.
- `view` est l'identité du template, relative à la racine des templates.
- Les champs `#[public]` sont rendus et transportés dans l'instantané signé.
  Les champs `#[model]` acceptent en plus des propositions du navigateur via
  `live:model`.
- Les méthodes `#[action]` sont les seuls points d'entrée que le navigateur
  peut invoquer. Elles reçoivent des arguments validés et peuvent renvoyer des
  résultats typés comme une redirection ou un flash.

Chaque type de champ doit implémenter `Default` ; un îlot neuf part de ces
valeurs par défaut sauf si un hook de montage en décide autrement.

## Vues

Les vues sont des templates Askama. La racine des templates est `templates/`
sauf si un `askama.toml` nomme d'autres répertoires, donc `live/counter.html`
se trouve dans `templates/live/counter.html` :

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

Les directives utilisent la grammaire fermée `live:` : `live:click`,
`live:submit`, `live:model`, `live:upload`, `live:key`, `live:loading` et le
reste de l'ensemble documenté. Le vérificateur prouve chaque directive contre
le composant : une action inconnue, un champ de modèle inconnu, un filtre
`safe` brut ou une violation d'accessibilité fait échouer `live:check` avec le
fichier, la ligne et la colonne.

Les documents qui placent des îlots sont des vues ordinaires déclarées avec
`#[suprnova::view]` ; la seule valeur non échappée qu'elles acceptent est
`TrustedHtml` via le filtre `trusted_html`.

## Enregistrement et bootstrap

`src/live/mod.rs` possède le registre et les routes :

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Liez-le pendant le bootstrap afin que le serveur, les workers et les commandes
`suprnova live:*` voient les mêmes composants :

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

Le registre est immuable une fois le runtime assemblé. Un nom de composant ou
une vue en double, ou un composant dont les actions nécessitent une validation
sans port de validation, fait échouer l'enregistrement avec un `RegistryError`
typé.

## Routes

`Router::try_live()` installe l'espace de noms réservé exactement une fois :
`/__live/v1/action`, `/__live/v1/upload`, les routes de contrôle et la
poignée de main WebSocket de `/__live/v1/async/*`, ainsi que les routes
immuables de `/__live/v1/assets/*`. Le démarrage échoue si une route
applicative peut revendiquer `/__live`.

Les routes de requête réservées portent une politique stricte : chaque requête
a besoin de faits de session, d'origine, de CSRF, de principal, de tenant et
de limitation de débit. Le framework enregistre la session et la preuve CSRF ;
votre application attache le reste avec le garde de routes :

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Installez les routes depuis le point d'entrée afin que le runtime et le
catalogue de montages soient prêts avant la première requête :

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Documents et îlots

Une route de document déclare ses îlots une fois, les rend via `LiveDocument`
et émet les balises de bootstrap :

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` déclare un îlot que tout visiteur peut rendre ; son
  état est une graine réutilisable promue en instance à la première action.
- `LiveMount::identity_bound` déclare un îlot qui appartient à la session et
  au principal courants ; la route de document doit authentifier.
- Montez chaque îlot avant `bootstrap`, et appelez `bootstrap` une seule fois.
  Le bootstrap émet l'élément de configuration inerte et les balises script de
  la stratégie ESM ou classique, en ajoutant les rôles de téléversement et
  asynchrone lorsqu'un composant monté en a besoin et le pont Stimulus à la
  demande.
- Le template du document place `{{ bootstrap|trusted_html }}` dans `<head>`
  et chaque îlot à sa place.

## Frontières de sécurité

Live ne contourne jamais le middleware du framework. Ce dont chaque requête a
besoin :

| Fait | Enregistré par |
|---|---|
| Session | `SessionMiddleware` |
| Origine et CSRF | `CsrfMiddleware` avec la vérification d'origine activée |
| Principal | `AuthMiddleware` dans sa branche authentifiée |
| Tenant | `LiveTenantMiddleware` avec votre résolveur |
| Limitation de débit | `RateLimitMiddleware` dans sa branche autorisée |

Le runtime livré envoie le type de média Live et l'en-tête propre au
navigateur `Sec-Fetch-Site` ; il ne transporte aucun jeton de session.
Le middleware CSRF vérifie cette preuve lui-même pour chaque requête Live,
quelle que soit la politique d'origine configurée : une requête Live de même
origine passe avec la disposition CSRF sans état, tandis qu'une requête
inter-sites ou sans en-tête retombe sur la validation par jeton et est
refusée. Les routes ordinaires conservent la validation par jeton sous la
politique par défaut ; utiliser Live n'assouplit rien d'autre :

```rust
global_middleware!(CsrfMiddleware::new());
```

Les visiteurs anonymes rendent des graines publiques et peuvent agir dessus
lorsque le garde utilise `AuthMiddleware::optional()` : un principal connecté
est enregistré, un visiteur anonyme continue, et le type de montage décide.
Une graine publique est alors promue pour la propre session du visiteur à la
première action, tandis qu'un îlot lié à l'identité refuse toujours une
requête sans preuve de principal. Avec `AuthMiddleware::new()`, le garde répond
`401` à toute requête anonyme avant tout travail du moteur. Les îlots liés à
l'identité exigent une session et un principal ; le tenant est lié à la portée
de l'îlot dès que votre résolveur en nomme un, et un résolveur qui ne peut pas
déterminer le tenant doit renvoyer une erreur plutôt que `None`. Chaque refus
est fermé : un `409` pour un instantané périmé ou altéré ne porte
aucun corps, et les messages de production n'incluent jamais d'instantanés, de
jetons, de cookies ni de HTML rendu.

## Téléversements

Déclarez une politique de téléversement sur un champ de modèle :

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

La vue lie le champ avec `<input type="file" live:upload="avatar">`. Le runtime
crée, transfère et achève le téléversement via `/__live/v1/upload` ; le
fichier attend en quarantaine jusqu'à l'exécution de l'action de finalisation
déclarée, moment où le framework le remet à votre `UploadFinalizer`. Liez le
finaliseur, ainsi que tout scanner ou validateur, avant que le runtime ne
s'assemble :

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Les téléversements sont autorisés par champ et par contrôle via le gate.
Définissez les capacités `live:<component>.upload.<field>.<Control>` pour
`Create`, `Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`,
`Complete`, `Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`,
`Expire` et `Fail`.

Un navigateur qui a perdu son jeton de transfert le réacquiert via une route
que votre application possède hors de l'espace de noms réservé :

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

La route exige les mêmes faits qu'une action, ne répond qu'à la session et au
principal qui ont créé le téléversement, et renvoie un jeton neuf avec l'état
courant du transfert.

## Mises à jour asynchrones

Un composant déclare les flux qu'il écoute ; le runtime navigateur s'abonne
via SSE ou WebSocket et retombe sur le polling :

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Définissez la capacité `live:<component>.stream.<name>` pour les abonnés, puis
publiez depuis n'importe où dans l'application :

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

Un refresh demande aux îlots abonnés un rendu frais ; un événement est délivré
aux gestionnaires enregistrés de l'îlot. Le polling est le rendu frais
ordinaire : l'état de l'îlot se remet à jour lorsqu'un transport est
indisponible, mais les charges d'événements publiées entre-temps ne sont pas
rejouées à leurs gestionnaires, ce que le runtime signale comme un flux dégradé
plutôt qu'à jour. Un composant qui déclare exactement un flux voit sa racine
d'îlot abonnée à celui-ci ; un composant avec plusieurs flux s'abonne à chacun
par les appels enregistrés du runtime.

## Assets et usage sans build

Le framework sert les artefacts de runtime exacts et relus à
`/__live/v1/assets/<identity>/<file>` avec un cache immuable, des validateurs
forts et des attributs d'intégrité dans les balises de bootstrap. Une politique
stricte `script-src 'self'` tient parce que les documents ne contiennent aucun
script inline. Pour publier les mêmes octets sur un CDN ou dans un répertoire
statique :

```bash
suprnova live:assets --out public/__live
```

La publication est atomique et refuse de remplacer un répertoire dont les
octets diffèrent, sauf si vous passez `--replace`.

## Tests

`suprnova::live::testing` prépare le runtime et le catalogue de montages d'un
routeur pour les tests en processus. Les tests applicatifs dans
`app/tests/live_*.rs` montrent le schéma complet : une base de données en
mémoire, un cookie de session préparé, la vraie pile de middleware globale et
des requêtes via `handle_request` :

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Décodez l'instantané d'un îlot depuis son attribut
`data-suprnova-live-snapshot`, envoyez une action avec le cookie de session et
`Sec-Fetch-Site: same-origin`, puis vérifiez le rendu accepté. Un instantané
périmé répond `409` avec un corps vide ; un principal absent répond `401`.

## Diagnostic et exploitation

- `suprnova live:check` prouve chaque vue enregistrée ; `--allow-unproved`
  accepte les structures dynamiques sur lesquelles le vérificateur ne se
  prononce délibérément pas.
- `suprnova live:inspect` rapporte le registre lié, les limites de
  configuration, les capacités de téléversement installées, les services de
  runtime assemblés et l'identité des assets sans exposer d'état ni de secret.
- `LiveConfig` borne les octets de requête et de réponse ainsi que la durée de
  vie du contexte de confiance ; liez-en un personnalisé avant que le runtime
  ne s'assemble.
- Les erreurs portent des sortes fermées comme `live_document_context_rejected`
  et `invalid_live_bootstrap` ; les étiquettes de télémétrie sont des
  énumérations fermées.

## Récupération

- Un `409` demande au runtime un rendu frais de l'îlot ; l'opération n'est pas
  rejouée.
- Un transport asynchrone fermé est retiré et le runtime se reconnecte avec une
  nouvelle génération de transport ; une génération périmée est refusée.
- Une session qui expire ou tourne invalide le travail lié à l'identité ;
  l'application expose son chemin de connexion et le visiteur reprend depuis
  un document frais.

Live fonctionne intégralement sans RenderCache ; la mise en cache des
documents Live est une fonctionnalité distincte avec son propre chapitre
lorsqu'elle arrivera.

## Référence de la CLI

| Commande | Rôle |
|---|---|
| `suprnova live:make <name>` | Générer un composant et sa vue et l'enregistrer |
| `suprnova live:check` | Prouver chaque vue enregistrée avec le vérificateur intégré |
| `suprnova live:inspect` | Rapporter l'état sûr du runtime, du registre, des fournisseurs et des artefacts |
| `suprnova live:assets --out <dir>` | Publier atomiquement les artefacts de runtime relus |
