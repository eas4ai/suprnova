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
  Un champ de modèle déclare son timing sur l'attribut, comme
  `#[model(debounce = 250)]` ; un debounce dure 100, 250 ou 500
  millisecondes, les durées qu'accepte `live:model.debounce.<n>ms`, et toute
  autre valeur ne compile pas.
  Un formulaire envoyé avec `live:submit` propose chaque contrôle de modèle
  qu'il contient dans une seule requête, qui porte jusqu'à 127 champs en plus
  de l'action ; `live:check` refuse un formulaire plus grand.
  Une proposition que le champ ne peut pas décoder, comme un nombre vide pour
  un champ `u64`, est une erreur de validation sur ce champ : l'action ne
  s'exécute pas, le champ garde sa valeur et `live:error` affiche l'erreur.
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
`live:key` nomme l'identité stable d'un élément d'un morph à l'autre et c'est le seul attribut de clé qu'un template écrit : le runtime le lit pour l'identité du morph, pour les contrôles de morph comme `live:preserve.self` et pour les portées qui conservent l'état du navigateur ; `data-suprnova-live-key` est l'orthographe propre du moteur sur les racines qu'il rend. Les valeurs de `live:key` et les identifiants d'élément à l'intérieur d'un îlot utilisent un seul alphabet, celui que le runtime vérifie à chaque morph : d'abord une lettre ou un chiffre ASCII, puis des lettres, des chiffres, `_`, `-`, `.` et `:`, au plus 128 octets, chacun unique dans l'îlot. `live:check` refuse une clé ou un identifiant littéral hors de cet alphabet, ainsi qu'un identifiant littéral à l'intérieur d'une boucle, que chaque élément après le premier répéterait.

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

## Routage

`Router::try_live()` installe l'espace de noms réservé exactement une fois :
`/__live/action`, `/__live/upload`, les routes de contrôle et la
poignée de main WebSocket de `/__live/async/*`, ainsi que les routes
immuables de `/__live/assets/*`. Le démarrage échoue si une route
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
crée, transfère et achève le téléversement via `/__live/upload` ; le
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
aux handlers enregistrés de l'îlot. Le polling est le rendu frais
ordinaire : l'état de l'îlot se remet à jour lorsqu'un transport est
indisponible, mais les charges d'événements publiées entre-temps ne sont pas
rejouées à leurs handlers, ce que le runtime signale comme un flux dégradé
plutôt qu'à jour. Un composant qui déclare exactement un flux voit sa racine
d'îlot abonnée à celui-ci ; un composant avec plusieurs flux s'abonne à chacun
par les appels enregistrés du runtime.

Un flux se termine avec la session qui l'a ouvert. Quand une session est
détruite sur le nœud qui tient le flux, par une simple déconnexion,
invalidation, régénération de l'identifiant ou une "déconnexion partout",
chaque adhésion qu'elle y a ouverte est retirée aussitôt et aucun événement
ultérieur ne l'atteint. Une session détruite sur un autre nœud est rattrapée
par la livraison elle-même : la session de chaque adhésion est revérifiée
auprès du magasin de sessions au plus une fois toutes les dix secondes, si
bien que les événements cessent dans cet intervalle. Le gate du flux est de
toute façon interrogé à nouveau avant chaque livraison, de sorte qu'un
changement de politique arrête la livraison immédiatement sur chaque nœud.

## Assets et usage sans build

Le framework sert les artefacts de runtime exacts et relus à
`/__live/assets/<identity>/<file>` avec un cache immuable, des validateurs
forts et des attributs d'intégrité dans les balises de bootstrap. Une politique
stricte `script-src 'self'` tient parce que les documents ne contiennent aucun
script inline. Pour publier les mêmes octets sur un CDN ou dans un répertoire
statique :

```bash
suprnova live:assets --out public/__live
```

La publication est atomique et refuse de remplacer un répertoire dont les
octets diffèrent, sauf si vous passez `--replace`.

## Bibliothèque de composants

Suprnova livre les fondations d'une bibliothèque de composants pour Live : une
feuille de style de tokens avec une couche de base, et une famille de
formulaires de composants de présentation bâtis sur des contrôles natifs et le
vocabulaire `live:model`, `live:error` et `live:loading`. La base est un
artefact du runtime. Qu'un document y souscrive et elle arrive comme un seul
lien de feuille de style sous le même contrat d'identité, d'intégrité et de
cache que les scripts du runtime :

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

Chaque règle y vit dans la couche de cascade `suprnova-ui`, si bien que vos
propres styles hors couche l'emportent sans bataille de spécificité. Chaque
valeur visuelle est une propriété personnalisée `--sn-` pour la couleur, la
police, l'espacement, le rayon, l'ombre, le mouvement, la densité et l'état,
avec des valeurs claires et sombres : surchargez un token sur `:root` pour
rethémer, ou retirez la couche et gardez chaque comportement, nom et attribut
d'état, car un composant stylise ses états à partir des attributs que le
vérificateur prouve (`aria-invalid`, `aria-busy`, `aria-expanded`,
`aria-pressed`, `aria-current`, `aria-selected`, `:disabled`), jamais à partir
d'une classe. Un préréglage `@theme` de Tailwind CSS 4 relie les tokens aux
espaces de noms de Tailwind ; Tailwind n'est jamais requis. Les composants
s'installent avec `live:add`, un répertoire chacun sous la racine réservée
`templates/suprnova-ui/` : la vue à macros Askama, la feuille de style, le
JavaScript quand le composant en a un, et le manifeste qui les nomme :

```bash
suprnova live:add field
suprnova live:add password-input
```

`live:add` enregistre le digest de chaque fichier qu'il écrit, si bien qu'une
exécution ultérieure remplace un fichier que vous n'avez jamais modifié quand
la bibliothèque le change, conserve un fichier que vous avez modifié et le
signale ; `--force` remplace aussi un fichier modifié. Un composant tiers
s'installe depuis son propre manifeste avec `--manifest`, sous sa propre
racine, et chaque fichier qu'il nomme doit être un fichier ordinaire dans le
répertoire du manifeste, jamais un lien symbolique. Appelez les macros depuis
vos vues, servez la feuille de style et le script avec `try_live_ui_assets()`
et liez-les depuis le document :

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

`try_live_ui_assets()` lit les fichiers de `templates/suprnova-ui/` sous le
chemin de base de l'application à chaque requête : livrez donc ce répertoire
avec le binaire et démarrez l'application depuis le répertoire qui le
contient, ou pointez `APP_BASE_PATH` sur ce répertoire. Quand le répertoire
ne peut pas être lu, l'application refuse de démarrer et le nomme.

Le vérificateur développe les macros, si bien que `live:check` prouve une vue
de la bibliothèque comme n'importe quelle autre. La famille de formulaires
aujourd'hui : champ, libellé, saisie, zone de texte, saisie numérique,
curseur, saisie de recherche, saisie de mot de passe avec révélation, case à
cocher et groupe de cases, groupe de boutons radio, interrupteur, sélecteur,
bouton et bouton-lien, groupe de boutons, fieldset, actions du formulaire,
résumé de validation et saisie de fichier. Les composants de la bibliothèque
se nomment `suprnova.*` et le registre refuse ce préfixe à toute autre crate ;
les éléments personnalisés sont en light DOM et portent le préfixe `sn-`.

Chaque contrôle de valeur prend la valeur courante de l'îlot, si bien que la
page l'affiche et qu'un envoi qui n'a rien changé la renvoie inchangée :
`value=` pour une saisie, une zone de texte, une saisie numérique, un curseur,
une saisie de recherche et un sélecteur de date, `checked=` pour une case à
cocher et un interrupteur, et `selected=` pour un sélecteur, un groupe de
boutons radio et un groupe de cases, qui prend la liste des valeurs cochées.
Un mot de passe et un code à usage unique ne rendent jamais leur valeur. Les
saisies des groupes de boutons radio et de cases ont leur valeur pour clé, si
bien qu'un choix que l'utilisateur n'a pas encore envoyé survit à un nouveau
rendu. Un groupe de cases est proposé comme la liste des valeurs cochées quel que
soit son nombre d'options, tout comme n'importe quel champ auquel plus d'une
case à cocher est liée ; une seule case est proposée comme un booléen. Un
rendu qui doit remplacer ce que l'utilisateur a saisi, comme celui qui répond
à une réinitialisation, passe un numéro de séquence comme `authority=`, et le
rendu suivant n'en passe aucun :

```html
{% call input::input("email", kind="email", value=email, authority=authority) %}{% endcall %}
{% call checkbox::checkbox_group("topics", "Topics", topic_options, topics) %}{% endcall %}
```

La famille des overlays est livrée sur les mêmes fondations : tooltip,
collapsible et accordion, popover, un menu déroulant à un seul niveau, dialog,
sheet et drawer. Chacun tient son état ouvert par la primitive du navigateur
avant tout script : `details` pour les disclosures, l'attribut `popover` pour
les popovers et les menus, et `dialog` pour les trois modales, que les éléments
embarqués `sn-dialog`, `sn-sheet` et `sn-drawer` ouvrent avec `showModal()` et
ferment en rendant le focus au déclencheur. Ouvrir et fermer n'émet jamais de
requête Live ; seule une action que vous placez dans un overlay le fait. Chaque
racine d'overlay porte une clé stable et `live:preserve.self`, si bien qu'un
overlay ouvert survit à un morph qui n'a pas remplacé sa région :

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

L'attribut `popover` fixe le plancher pris en charge à Chrome et Edge 114,
Firefox 128 et Safari 17. Là où le positionnement par ancre CSS existe, le
popover et le menu se placent sous leur déclencheur ; ailleurs le navigateur
les centre. Le mode d'ouverture unique de l'accordion repose sur `details
name`, que les versions prises en charge plus anciennes traitent comme des
disclosures indépendants. Le tooltip reste ouvert pendant que le pointeur
passe de son déclencheur à la bulle, si bien que son texte peut être lu ou
sélectionné.

La famille feedback et la famille navigation suivent. Feedback : alert,
skeleton, spinner, progress, empty state et une région de toasts avec une
région de flash à côté. Chacun présente un état que le serveur ou le runtime
détient déjà. Un alert choisit son rôle selon sa variante et marque chaque
variante d'un glyphe et d'une étiquette masquée, jamais de la couleur seule.
Un spinner ou un skeleton est lié par `live:loading.show` à une action
enregistrée et livré masqué, si bien que le runtime le révèle après son propre
délai et le garde au-delà de son minimum ; une action rapide ne le fait jamais
clignoter. Progress est l'élément natif `progress` avec une étiquette et une
lecture en texte, et ne porte une valeur que pour un travail déterminé.
L'empty state prend sa raison (vide, aucun résultat, aucune permission,
déconnecté) de l'état rendu par le serveur et n'offre une action suivante que
là où l'appelant en rend une. Un toast annonce une fois depuis une région
d'état polie et ne prend jamais le focus ; l'élément vendorisé
`sn-toast-region` fait expirer les toasts, se met en pause tant que le
pointeur est sur une partie quelconque d'un toast ou que le focus est à
l'intérieur, borne le nombre affiché à la fois et répond au bouton de fermeture,
chaque toast étant à clé et préservé pour qu'un toast fermé le reste après un
morph. Une erreur critique appartient aussi à un alert ; un toast n'est jamais sa seule surface. Les toasts sont rendus dans une boucle, donc leurs clés passent par le filtre `live_key`, et l'island qui les monte l'expose avec `pub mod filters { pub use suprnova::view::filters::live_key; }`. `live_key` fait échouer le rendu de l'îlot pour une valeur hors de l'alphabet des clés, comme une adresse e-mail ; donnez une clé à ces données avec `live_key_digest`, qui transforme toute valeur en une clé stable dans l'alphabet et s'exporte de la même façon depuis `suprnova::view::filters`. La région de flash rend une seule fois ce que la requête
précédente a laissé dans la session :

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

Navigation : barre d'en-tête, footer, sidebar à groupes repliables, fil
d'Ariane, tabs, pagination et load more. Chaque destination est une ancre
avec une vraie URL de route et chaque action un bouton ; l'élément courant
porte `aria-current` depuis la valeur que vous liez, jamais depuis
l'emplacement du navigateur. Les groupes de la sidebar sont des `details`
natifs, à clé et préservés. Les tabs exigent un mode : `local`, des panneaux
avec la sémantique tablist, les flèches du clavier via l'élément vendorisé
`sn-tabs` et aucune requête au changement, ou `route`, des tabs sous forme
d'ancres. Les tabs s'imbriquent : une instance de tabs intérieure ne
sélectionne que ses propres tabs et panneaux. La pagination exige aussi un
mode : les pages de route sont des liens canoniques, les pages Live sont des
boutons sur vos actions dont le résultat reflète la nouvelle query dans
l'entrée d'historique courante via `url_intent`, sans entrée par page. Load
more est un bouton sur une action
enregistrée qui ajoute à une liste à clés, si bien que le morph garde chaque
ligne déjà présente, et le contrôle quitte la vue quand vous le rendez épuisé. Une réflexion d'URL est un résultat du protocole 2, donc une island qui pagine via `url_intent` déclare `minimum_protocol_version = 2` ; ses lignes à clés passent par `live_key` comme un toast :

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

La famille d'affichage de données clôt l'ensemble intégré. Présentationnels :
separator, scroll area, aspect image, card, badge, avatar et groupe
d'avatars, list group, description list et stat card. Chacun conserve l'ordre
du document et la sémantique native : le separator est un `hr` ou un rôle
separator étiqueté, la scroll area une région étiquetée focalisable qui
défile nativement, l'aspect image l'`img` lui-même avec un ratio nommé, la
card un article ou une section étiquetée par son propre titre avec ses
actions dans un groupe étiqueté, et la description list un `dl`. Un badge
porte toujours son texte, un avatar nomme sa personne dans `alt` ou dans
l'étiquette de ses initiales, et la tendance d'une stat card dit "Up", "Down"
ou "Flat" en texte avant le delta, si bien qu'aucun statut ne repose sur la
seule couleur. La list group attribue une clé à chaque élément via
`live_key`, donc un réordonnancement garde chaque nœud. Le chart est rendu
sur le serveur : l'island appelle `render_chart` de `suprnova::live::charts`,
qui trace des barres ou des lignes via `charts-rs` à partir de séries typées
bornées et renvoie un balisage de confiance, et la macro rend le SVG à côté
d'un résumé textuel et d'une table de données dans un disclosure, de sorte
que le document canonique se lit sans l'image et qu'aucun script de
graphiques n'atteint le navigateur :

```rust
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};

pub fn chart_svg(&self) -> TrustedHtml {
    render_chart(
        ChartKind::Bar,
        &["Apr", "May", "Jun"],
        &[ChartSeries::new("Revenue", vec![42.0, 47.0, 51.0])],
    )
    .expect("a bounded fixed series renders")
}
```

`render_chart` renvoie une erreur plutôt que de dessiner une valeur dont la
magnitude dépasse 1e9, au-delà de la plage que tient l'arithmétique des axes
du moteur de rendu.

La datatable est le dernier composant, avec une island par table. C'est une
`table` native avec une caption qui nomme le nombre de résultats, des
en-têtes de colonne avec `scope` et `aria-sort` sur la colonne triée. Le tri
et le filtre sont des submits Live sur les champs model de l'island, les
changements de page sont des boutons Live, et l'island déclare le tri
appliqué, la direction, le filtre et la page comme champs `#[url]` et les
reflète via `url_intent` après chaque action, si bien que la barre d'adresse
contient toujours une URL partageable et que le document monte la même vue à
partir d'elle :

```rust
#[live(name = "app.invoices", view = "live/invoices.html", minimum_protocol_version = 2)]
pub struct Invoices {
    #[model]
    pub sort: String,
    #[url(key = "sort")]
    pub sorted_by: String,
    #[url(key = "dir")]
    pub direction: String,
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    #[url(key = "page")]
    pub page: u64,
    pub rows: Vec<Invoice>,
}
```

La famille live-native est la dernière : les composants qui n'ont de sens que sur le runtime en marche. Le widget de téléversement présente le protocole de téléversement fourni : son champ fichier porte `live:upload` pour le champ de téléversement de l'îlot, son élément `progress` est la racine de progression du runtime, et annuler, réessayer et retirer agissent sur la référence temporaire via `live:upload.cancel` et ses frères. Chaque état connu du domaine est rendu en texte et affiché d'après le `data-live-upload-state` de la racine de progression, et « ready » se lit comme vérifié mais non enregistré, car rien n'est durable avant l'action de finalisation :

```html
{% call upload::upload("attachment", "Attachment", accept="image/png") %}{% endcall %}
<button type="submit" live:loading.disabled="save_attachment">Save attachment</button>
```

Le fil en direct et la cloche de notifications reposent sur un îlot adossé à un flux. Le runtime écrit `data-live-stream-state` sur la racine de l'îlot et annonce chaque changement dans l'élément `[data-live-stream-status]` que les macros rendent (Updates disconnected, Connecting to updates, Updates current, Updates degraded, Reconnecting to updates, Updates closed), de sorte qu'un flux dégradé, en reconnexion ou fermé le dit, et que seul l'état current se lit comme à jour. Les entrées du fil passent par `live_key`. Le menu de compte est un dépliant `details` d'ancres et d'un formulaire de déconnexion qui envoie avec le jeton CSRF de la session ; c'est un slot de stitch sous RenderCache, donc une application le monte comme son propre îlot lié à l'identité et la coquille partagée ne contient jamais le nom du principal.

Le palier des éléments personnalisés enrichit des contrôles natifs qu'il ne remplace jamais. Chaque élément est une sous-classe de `HTMLElement` en light DOM définie uniquement par son propre fichier vendorisé, porte le préfixe `sn-` et ne détient aucune valeur de formulaire, car le champ natif qu'il contient est le contrôle : bloquez le script et le formulaire envoie toujours la même valeur. La saisie OTP est un seul champ natif (`inputmode="numeric"`, `autocomplete="one-time-code"`, un motif de longueur) sur un modèle transitoire, et `sn-input-otp` reflète les caractères saisis dans des cellules `aria-hidden`. Le sélecteur de date est un champ `type="date"`, et ses bandes d'année, de mois et de jour sont des fieldsets de radios natifs dans des conteneurs CSS scroll-snap, si bien que toucher, cliquer et les flèches sélectionnent sans script ; `sn-date-picker` compose une sélection complète dans le champ. La combobox est le motif accessible de combobox (`role="combobox"`, `aria-expanded`, `aria-activedescendant`, une `role="listbox"` d'options) sur un champ natif avec une `datalist` pour le cas sans script, et `sn-combobox` déplace l'option active et sélectionne. Par défaut, les options sont la réponse de votre serveur à la requête : rendez-les pour le champ de modèle à chaque rendu, et l'élément les affiche toutes tant que la requête à laquelle elles répondent est le texte du champ, quelle que soit la correspondance trouvée par votre recherche, et garde masquée une réponse à un texte plus ancien, de sorte qu'un résultat périmé ne remplace jamais les résultats d'une requête plus récente. Passez `remote=false` pour une liste fixe, que l'élément filtre selon le texte saisi :

```html
{% call otp::input_otp("code", "One-time code") %}{% for index in cells %}{% call otp::otp_cell(index) %}{% endcall %}{% endfor %}{% endcall %}
{% call date::date_picker("when", "Renewal date", years, months, days, min="2026-01-01", max="2028-12-31") %}{% endcall %}
{% call combo::combobox("country", "Country", countries, query=country, placeholder="Type a country") %}{% endcall %}
```

### Pourquoi Suprnova diverge

Laravel livre des composants Blade et le balisage d'un kit de démarrage ;
Suprnova livre la bibliothèque par le framework lui-même, dans le vocabulaire
propre de Live, sans qu'aucune application cliente ne possède la page.
L'habillage est activé par défaut et se retire sans rien casser, ce qui est
ici le sens de headless.

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

Live fonctionne intégralement sans RenderCache. La mise en cache des
documents Live est le rôle de RenderCache ; voir
[RenderCache](render-cache.md).

## Référence de la CLI

| Commande | Rôle |
|---|---|
| `suprnova live:make <name>` | Générer un composant et sa vue et l'enregistrer |
| `suprnova live:check` | Prouver chaque vue enregistrée avec le vérificateur intégré |
| `suprnova live:inspect` | Rapporter l'état sûr du runtime, du registre, des fournisseurs et des artefacts |
| `suprnova live:assets --out <dir>` | Publier atomiquement les artefacts de runtime relus |
