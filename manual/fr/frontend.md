# Présentation du frontend

Suprnova connecte les handlers Rust à un frontend monopage via
[Inertia.js](https://inertiajs.com/) 3.4.0. Vous écrivez les contrôleurs en Rust
et les pages en Svelte, React ou Vue ; le framework déplace les props typés
entre eux sans API HTTP séparée au milieu.

## Trois démarrages de première classe

`suprnova new <name>` crée un projet fonctionnant. Le flag `--frontend`
choisit la couche SPA :

```bash
suprnova new my-app                       # Svelte 5 (par défaut)
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

Les trois frameworks partagent la même pile :

| Couche | Version |
|---|---|
| Adaptateur client Inertia | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| Outil de construction | Vite 8 |
| Style | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | mode strict |

Le choix se fait par projet. Il n'y a pas de framework « primaire » du côté
serveur - `inertia_response!` résout l'extension que votre framework choisi
utilise (`.svelte`, `.tsx`, `.vue`), et `App::inertia_share`, les rechargements
partiels et la génération de types TypeScript se comportent de manière identique
entre les trois.

## Architecture

```
                       Browser
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  typed props from Rust struct       |
   |   +-------v-------------------------------+     |
   |   |        Inertia client adapter         |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON on XHR, HTML on first load)
                          v
   +-------------------------------------------------+
   |                  Suprnova server                |
   |   +------------------------------------------+  |
   |   |          Controllers / handlers          |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

La première requête retourne une coque HTML avec l'objet de page initial
intégré dans l'attribut `data-page` du nœud de montage. Les visites suivantes
passent par `<Link>` / `router.visit`, envoient `X-Inertia: true` et
récupèrent un objet page JSON - l'adaptateur échange le composant sans
rechargement complet.

## Un aller-retour de page complet

Le contrôleur définit ses props comme une structure Rust, en dérivant
`InertiaProps` et en passant la valeur à la macro `inertia_response!` :

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

La macro fait plusieurs choses pour vous. D'abord, elle valide au moment
de la compilation que le fichier du composant de page existe réellement sous
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` - les fautes de frappe
s'affichent comme une erreur de construction, pas comme un 404 dans le
navigateur. Ensuite, elle sérialise la structure `HomeProps`, la déplie en
une prop par clé de haut niveau afin que les rechargements partiels puissent
filtrer, et résout les props lazy ou deferred par rapport à `&req` avant de
retourner. La macro s'évalue en un `Result<HttpResponse, FrameworkError>`,
que le type de retour `Response` accepte directement.

La page Svelte correspondante (le scaffold par défaut) :

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

Pour les équivalents React et Vue, voir [Composants de page](frontend-pages.md).

## Générer les types TypeScript

Chaque structure `#[derive(InertiaProps)]` dans votre `src/` devient une
interface TypeScript dans `frontend/src/types/inertia-props.ts` :

```bash
suprnova generate-types
```

Passez `--routes` et la même commande émet également
`frontend/src/types/routes.ts` - des paires URL + méthode type-safe extraites
de votre macro `routes!` qui fonctionnent directement avec les API Inertia v2+. Le tableau complet de mappage de types et
la forme du helper de routes se trouvent dans [Types TypeScript](frontend-typescript-types.md).

## Données partagées

Tout ce qui doit apparaître sur chaque page (l'utilisateur authentifié, la
locale actuelle, les métadonnées de l'application) est enregistré une fois au
démarrage et fusionné dans chaque réponse Inertia :

```rust
// Dans bootstrap.rs
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// Les données partagées async / par requête passent par le trait.
App::register_inertia_shared(Arc::new(AppSharedData));
```

Trois variantes, dans l'ordre de précédence (la suivante gagne sur la même clé) :

| API | Quand la valeur se matérialise |
|---|---|
| `App::inertia_share(k, v)` | Sync, défini une fois au démarrage |
| `App::inertia_share_lazy(k, \|\| async { ... })` | Par réponse, recalculé |
| `App::inertia_share_once(k, \|\| async { ... })` | Par réponse, puis mis en cache côté client |
| `App::register_inertia_shared(Arc::new(impl))` | Par requête, voit `&req` |

Les props par page attachés au générateur de réponse écrasent toujours les
données partagées à la même clé.

## Rechargements partiels et props lazy

Le même générateur `InertiaResponse` expose la trousse complète de props Inertia v3 - eager, lazy, optional, deferred, merge, once - et Suprnova honore les
en-têtes de rechargement partiel v3 (`X-Inertia-Partial-Data`,
`X-Inertia-Partial-Except`, `X-Inertia-Reset`,
`X-Inertia-Except-Once-Props`) automatiquement. L'exemple ci-dessous
attache trois props avec des règles d'évaluation différentes :

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` couvre le cas des props eager ; tout au-delà passe par le
générateur. La surface complète - `optional`, `merge`,
`once`, `scroll`, `flash`, `paginate`, SSR, décalage de version, chiffrement
d'historique - est documentée dans
[Réponses Inertia](frontend-inertia-responses.md).

## Amorçage

Une application générée installe les quatre middlewares critiques du protocole
en un seul appel à l'intérieur de `bootstrap.rs` :

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` retourne `Result` - elle échoue de manière fermée si `InertiaConfig`
se résout au mode production (la valeur par défaut sous `APP_ENV=production`)
mais aucun manifeste Vite ne peut être trouvé, plutôt que de revenir silencieusement
à un chemin d'actif hérité. Voir [Développement vs production](#développement-vs-production)
ci-dessous.

Cela enregistre, dans l'ordre : `InertiaHeadersMiddleware` (pose
`Vary: X-Inertia` sur chaque réponse et transforme un `200` vide lors d'une
visite Inertia en un `303` de retour), `InertiaVersionMiddleware` (émet 409 +
`X-Inertia-Location` en cas de décalage de version d'actif afin que les clients
périmés se rechargent), `Inertia303Middleware` (réécrit 302 → 303 sur les
visites Inertia non-GET afin que le suivi soit sans ambiguïté un GET), et
`InertiaValidationRedirectMiddleware` (transforme un `422` lors d'une visite
Inertia en un `303` de retour vers la page du formulaire avec les erreurs
flashées). `InertiaVersionMiddleware` et `Inertia303Middleware` exigeaient
autrefois un enregistrement distinct ; `Inertia::install` rend les quatre
actifs par défaut. Voir [Réponses
Inertia](frontend-inertia-responses.md#bootstrap-inertia-install) pour l'ordre
d'enregistrement complet et les modes d'échec que chacun ferme.

## Développement vs production

En développement, le serveur de développement Vite s'exécute parallèlement au
backend et fournit les actifs activés HMR :

```bash
suprnova serve
```

Cela démarre le serveur Rust et `vite` ensemble. La coque HTML charge
les modules depuis `http://localhost:5765`.

Pour la production, construisez le frontend une fois et pointez le backend
vers le manifeste hashé sous `public/assets/` :

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` dérive le mode production vs. développement à partir
de `APP_ENV` (via `Environment::detect().is_production()`) - `APP_ENV=production`
est ce qui fait que la coque HTML charge les actifs construits à la place du serveur
de développement Vite. `Inertia::install` échoue ensuite explicitement au démarrage
s'il ne peut pas trouver un manifeste pour soutenir cette décision, plutôt que de
revenir silencieusement à un chemin codé en dur périmé.

Suprnova lit `public/assets/.vite/manifest.json` pour résoudre les points
d'entrée hashés plus les imports transitifs pour `modulepreload`. SSR est
facultatif  -  optez en pointant `InertiaConfig::ssr(...)` vers un worker
`@inertiajs/{vue3,react,svelte}/server` en cours d'exécution. `suprnova new`
scaffolde un point d'entrée SSR et un script de build pour chaque starter, et
`suprnova ssr:start` / `suprnova ssr:check` lancent et vérifient le worker ;
voir [Réponses Inertia](frontend-inertia-responses.md#ssr) pour la
configuration complète, y compris la vérification de l'existence du bundle et
le comportement de repli CSR.

### Pourquoi Suprnova diverge

Trois écarts intentionnels par rapport à ce qu'une configuration Inertia typique
ressemble ailleurs :

- **Validation de composant au moment de la compilation.** La macro
  `inertia_response!` parcourt `frontend/src/pages/` au moment de la
  construction et refuse de s'étendre si le fichier du composant est manquant,
  en suggérant la correspondance la plus proche. Vous ne pouvez pas déployer un
  contrôleur qui pointe vers une page supprimée.
- **Props typés comme source de vérité.** Les props de page sont des structures
  Rust avec `#[derive(InertiaProps)]`. `suprnova generate-types` les lit et
  écrit les interfaces TypeScript - les types frontend sont dérivés du backend,
  non maintenus en parallèle.
- **Svelte par défaut.** La documentation d'Inertia atteint Vue et React en
  premier ; le scaffolder Suprnova est par défaut Svelte 5 (runes-on). React 19
  et Vue 3.5 sont de première classe, non des arrière-pensées - même protocole,
  même pipeline de props, même sortie de générateur.

## Suivant

- [Composants de page](frontend-pages.md)
- [Réponses Inertia](frontend-inertia-responses.md)
- [Types TypeScript](frontend-typescript-types.md)
- [Routage](routing.md)
- [Contrôleurs](controllers.md)
