# CORS

`CorsMiddleware` répond aux requêtes préflight `OPTIONS` et décore les
réponses cross-origin ordinaires avec des en-têtes
`Access-Control-Allow-*`. Vous l'installez une seule fois dans
`bootstrap()` quand un navigateur sur une origine différente appelle
votre API - API publiques, SPA hébergée sur un autre domaine, webview
mobile, ou site de documentation hébergé séparément. Les applications
same-origin (Inertia servi depuis le même hôte que le backend, le défaut
Suprnova) n'ont pas besoin de CORS du tout. Le middleware reflète le
`HandleCors` et le `config/cors.php` de Laravel, mais sous la forme d'un
builder typé sur `CorsConfig`.

## Installation globale

```rust,ignore
use std::time::Duration;
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

pub fn register() {
    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600)),
    ));
}
```

Un préflight est une requête `OPTIONS` portant un en-tête
`Access-Control-Request-Method`. Le routeur n'a aucune route `OPTIONS`,
donc un préflight ne *correspond* jamais à une route - mais le serveur
de Suprnova exécute la chaîne de middleware globale sur les requêtes sans
correspondance (en terminant par un 404), donc un `CorsMiddleware`
installé globalement voit le préflight et le court-circuite avec `204`
avant que le 404 ne soit produit. **C'est pourquoi CORS doit être
installé globalement, et non par route.**

## Choisir une politique d'origine

Il n'y a intentionnellement aucun `Default` pour `CorsConfig`. Une
politique permissive par réflexe est un piège de sécurité, vous devez
donc choisir :

| Builder | Comportement |
| --- | --- |
| `CorsConfig::allow_origins([...])` | Allowlist fixe. L'origine n'est renvoyée que lorsqu'elle correspond exactement à une entrée. |
| `CorsConfig::any_origin()` | Caractère générique `*`. Avec les identifiants activés, le middleware renvoie l'origine précise de la requête au lieu de `*` (la combinaison `*` + identifiants est invalide selon la spécification Fetch). |
| `.allow_origin_patterns([...])` | Motifs regex ajoutés par-dessus la liste littérale. Utile pour les sous-domaines dynamiques. |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

Les motifs sont ancrés automatiquement - `^` et `$` sont ajoutés en tête
et en queue s'ils manquent, de sorte qu'une correspondance partielle sur
une URL de redirection comme `https://evil.com/?u=https://app.example`
ne peut pas se faufiler.

Une regex invalide panique au moment de la configuration (à l'amorçage),
pas au moment de la requête - mieux vaut faire remonter le bug de
configuration de manière visible que de s'ouvrir silencieusement.

`allowed_origins_patterns` (alias au nom Laravel) est également
disponible.

## Limiter les chemins auxquels CORS s'applique

La config `cors.php` de Laravel a un tableau `paths` (`['api/*',
'sanctum/csrf-cookie']`) qui limite l'application de CORS à des motifs
d'URL précis. Suprnova reflète cela :

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

Sans `paths` défini, CORS s'exécute sur chaque requête (le défaut de
Suprnova - puisque le middleware est opt-in par son enregistrement).
Avec au moins un motif défini, seules les requêtes correspondantes
reçoivent le traitement CORS (les préflights **et** la décoration de la
réponse réelle) ; tout le reste passe sans être touché.

Les motifs utilisent la sémantique `Str::is` de Laravel : `*` est un
caractère générique multi-segment, gourmand à travers les `/`. Le `/` de
début est normalisé, donc `"api/*"` et `"/api/*"` sont équivalents.

```rust,ignore
"api/*"             // correspond à /api/users, /api/users/42
"api/*/posts"       // correspond à /api/v2/posts, /api/v1/posts
"sanctum/csrf-cookie" // littéral à correspondance exacte
"*"                 // correspond à tout
```

## Ignorer via un prédicat

Pour les prédicats sur la forme de la requête qui n'entrent pas dans un
motif de chemin (ignorer selon un en-tête, n'exécuter CORS qu'en
production, ignorer pendant les vérifications de santé), utilisez
`skip_when` :

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

Reflète le `HandleCors::skipWhen(Closure)` de Laravel, mais réside sur
la politique plutôt que sous forme d'état global mutable. Plusieurs
callbacks `skip_when` peuvent être enregistrés ; il suffit que l'un
d'eux retourne `true` pour que CORS soit ignoré.

## Méthodes, en-têtes, en-têtes exposés

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // défaut = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // restreint ; défaut = renvoyer ceux demandés
    .allow_any_headers()                          // « renvoyer ce qui a été demandé », explicite
    .expose_headers(["X-Total-Count", "Link"])    // en-têtes que JS peut lire sur la réponse
```

Alias au nom Laravel (pour que les utilisateurs de `cors.php` trouvent
ce qu'ils attendent) :

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## Identifiants et `*`

Selon la spécification Fetch, `Access-Control-Allow-Origin: *` est
invalide combiné avec des identifiants - le navigateur rejette la
réponse. Avec une liste d'origines explicite (`allow_origins([...])`)
plus `allow_credentials(true)`, le middleware renvoie l'`Origin` précise
de la requête plutôt que `*`, et la politique fonctionne comme prévu.

**`any_origin() + allow_credentials(true)` panique à la construction.**
La combinaison contourne entièrement l'allowlist d'origines : n'importe
quelle page d'attaquant peut émettre des requêtes cross-origin avec
identifiants et en lire les réponses. Plutôt que d'émettre le mauvais
en-tête à l'exécution, le constructeur de politique échoue explicitement
pour que cette erreur de configuration n'atteigne jamais un déploiement
en fonctionnement. Utilisez plutôt une allowlist explicite :

```rust,ignore
// CORRECT - allowlist explicite avec identifiants.
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → sur une requête avec Origin: https://app.example
// → réponse : Access-Control-Allow-Origin: https://app.example
//             Access-Control-Allow-Credentials: true

// REJETÉ à la construction - panique avec un message de remédiation.
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-age

```rust,ignore
.max_age(Duration::from_secs(600))   // typé
.max_age_secs(600)                   // secondes entières, façon Laravel
```

`Access-Control-Max-Age` indique au navigateur combien de temps il peut
mettre en cache le résultat du préflight. Plus la valeur est élevée,
moins il y a d'allers-retours de préflight, et plus les changements de
politique mettent de temps à se propager.

## Ce que le middleware émet réellement

### Préflight (`OPTIONS` + `Access-Control-Request-Method`)

Si l'origine est autorisée :

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // quand les identifiants sont activés
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // quand défini
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

Si l'origine n'est pas autorisée : un `204` nu + `Vary` (aucun
`Access-Control-*`). C'est la vérification d'en-tête manquant du
navigateur qui produit l'erreur CORS - conformément à la convention de
`tower-http`.

### Réponse cross-origin réelle

Quand la requête porte un en-tête `Origin` et que l'origine est
autorisée :

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // quand activé
Access-Control-Expose-Headers: X-Total, Link  // quand configuré
Vary: Origin                                  // seulement quand ce n'est pas "*"
```

Un ACAO à `*` est identique pour toutes les origines, donc aucun `Vary`
n'est nécessaire ; une origine précise varie d'une origine à l'autre,
donc les caches partagés doivent l'intégrer à leur clé.

## Tester les handlers CORS

CORS est appliqué côté navigateur - le serveur exécute quand même le
handler lorsque l'origine n'est pas autorisée ; il ne décore simplement
pas la réponse. C'est cela, le comportement testable :

```rust,ignore
let (status, headers, body) = request_with_origin(
    "/api/data",
    "https://app.example",
).await;
assert_eq!(status, 200);
assert_eq!(
    headers.get("access-control-allow-origin"),
    Some(&"https://app.example".to_string()),
);
```

Pour une origine non autorisée, le handler s'exécute et le corps
revient, mais c'est l'absence d'`Access-Control-Allow-Origin` qui
empêche le navigateur de le lire.

## Matrice de parité avec Laravel

| `cors.php` de Laravel | Builder Suprnova |
| --- | --- |
| `paths` | `.paths([...])` |
| `allowed_methods` | `.methods([...])` / `.allowed_methods([...])` |
| `allowed_origins` | `CorsConfig::allow_origins([...])` |
| `allowed_origins_patterns` | `.allow_origin_patterns([...])` / `.allowed_origins_patterns([...])` |
| `allowed_headers` | `.allow_headers([...])` / `.allowed_headers([...])` |
| `exposed_headers` | `.expose_headers([...])` / `.exposed_headers([...])` |
| `max_age` | `.max_age(Duration)` / `.max_age_secs(u64)` |
| `supports_credentials` | `.allow_credentials(bool)` / `.supports_credentials(bool)` |
| `HandleCors::skipWhen(closure)` | `.skip_when(\|req\| ...)` |

Le middleware est enregistré globalement plutôt qu'« installé
automatiquement pour `paths` » façon Laravel - la chaîne de middleware
de Suprnova est explicite, voir [Middleware](middleware.md) pour la
conception.

### Pourquoi Suprnova diverge

Le `HandleCors` de Laravel est rattaché automatiquement au kernel et lit
sa politique depuis `config/cors.php`. Cette forme fonctionne pour PHP
parce que le tableau de config est le seul endroit où un framework
request-per-process peut partager de la configuration sans la réévaluer
à chaque requête. Suprnova expose les mêmes options sous la forme d'un
builder `CorsConfig` typé que vous enregistrez explicitement avec
`global_middleware!`, ce qui garde la chaîne de middleware visible dans
`bootstrap()` et laisse le compilateur imposer le choix entre allowlist
et caractère générique (pas de `Default` pour `CorsConfig`, vous ne
pouvez donc pas livrer par accident `Access-Control-Allow-Origin: *`
pour avoir oublié de remplir une valeur de config).

L'autre divergence est que les préflights atteignent le middleware même
sur des chemins non routés. Laravel fait passer `OPTIONS` par son
routeur, donc le préflight correspond à une route `OPTIONS` (enregistrée
automatiquement pour chaque route REST). Le routeur de Suprnova n'a
aucune route `OPTIONS` ; à la place, le serveur exécute la chaîne de
middleware globale sur les requêtes sans correspondance avant de
retourner 404, donc un `CorsMiddleware` installé globalement
court-circuite le préflight avec `204` avant que le chemin « non
trouvé » ne soit emprunté. C'est pourquoi CORS *doit* être installé
globalement - un enregistrement par route ne verrait jamais le
préflight.

## Suivant

- [Middleware](middleware.md) - le trait, la chaîne, l'enregistrement
  global contre l'enregistrement par route, les hooks terminables
- [CSRF](csrf.md) - l'autre middleware global que la plupart des
  applications installent à côté de CORS
- [Routage](routing.md) - comment les routes sont mises en
  correspondance (et pourquoi les préflights n'y correspondent pas),
  plus le chemin sans fallback sur lequel s'exécute la chaîne globale
- [Cycle de vie des requêtes](lifecycle.md) - où se situe CORS dans la
  chaîne par rapport à la session, à CSRF et au handler
- [Configuration](configuration.md) - les motifs de config typée pour
  les middleware qui ont besoin de réglages pilotés par l'environnement
