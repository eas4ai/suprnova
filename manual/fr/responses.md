# Réponses

Chaque handler Suprnova retourne une `Response`, qui est un alias de
`Result<HttpResponse, HttpResponse>`. La branche `Ok` porte la réponse
de succès, la branche `Err` porte une réponse d'erreur déjà rendue, et
l'opérateur `?` réduit au passage tout type d'erreur qui possède un
`From` vers `HttpResponse`. Ce chapitre est la référence pratique pour
construire le côté `Ok` - les builders `HttpResponse`, le builder
`Redirect`, l'API des cookies et les courts-circuits `abort_*`. Pour
l'approche des erreurs, voir [Modèle d'erreur](error-model.md) et
[Gestion des erreurs](errors.md).

## Les builders `HttpResponse`

`HttpResponse` est le type de réponse tel qu'il part sur le réseau. Les
constructeurs posent des valeurs par défaut raisonnables ; les méthodes
chaînables les redéfinissent.

### Constructeurs de corps

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (n'importe quelle serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // Octets bruts avec un content-type explicite - utilisé par la
    // sérialisation JSON:API et tout autre corps d'octets non-JSON.
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

Deux constructeurs de streaming existent pour les réponses de longue
durée :

- `HttpResponse::sse(stream)` - Server-Sent Events. Enveloppe un
  `Stream` de valeurs `SseEvent`, pose les quatre en-têtes requis
  (`Content-Type: text/event-stream`, `Cache-Control: no-cache`,
  `Connection: keep-alive`, `X-Accel-Buffering: no`) et maintient la
  connexion ouverte jusqu'à ce que le flux producteur se termine. Voir
  [Événements serveur](sse.md).
- `HttpResponse::stream_bytes(stream)` - réponse générique par blocs.
  Prend un `Stream<Item = Result<Bytes, Infallible>>`. Le type d'erreur
  est `Infallible` à dessein : chaque producteur du framework convertit
  ses propres erreurs en un message terminal du flux avant que le flux
  ne s'achève, car il n'existe aucun moyen de faire remonter au client
  une erreur au niveau du transport en plein milieu d'une réponse.

### Statut, en-têtes, cookies

Chaque méthode retourne `Self`, alors chaînez librement :

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| Méthode | Comportement |
|---|---|
| `.status(code)` | Définit le statut HTTP. Les codes hors de `100..=599` sont rétrogradés en 500 à la frontière du réseau, avec un avertissement dans le journal. |
| `.header(name, value)` | Ajoute un en-tête. Les doublons sont permis (conforme à la sémantique de `Set-Cookie`). |
| `.replace_header(name, value)` | Retire toutes les occurrences antérieures et en pose une. |
| `.with_headers([(k, v), ...])` | En ajoute plusieurs d'un coup. Accepte n'importe quel `IntoIterator<Item = (K, V)>`. |
| `.without_header(name)` | Retire toutes les occurrences (insensible à la casse). |
| `.header_value(name)` | Relit la première valeur posée. Utile dans les tests. |
| `.cookie(Cookie)` | Attache un cookie sous forme de `Set-Cookie`. |
| `.with_cookies([Cookie, ...])` | En attache plusieurs. |
| `.without_cookie(name)` | Planifie une suppression (équivalent à `Cookie::forget(name)`). |

Les mêmes méthodes chaînables sont disponibles sur une `Response` (le
`Result`) à travers le trait `ResponseExt`, afin que les macros restent
ergonomiques :

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` expose `.status`, `.header`, `.with_headers`,
`.without_header`, `.cookie`, `.with_cookies` et `.without_cookie`.

### Validation à la frontière du réseau

`HttpResponse::into_hyper` exécute deux filtres de sûreté avant de
remettre la réponse à hyper :

- **Plage de statuts.** Tout ce qui sort de `100..=599` est rétrogradé
  en 500 avec un `tracing::warn!`. Cela attrape à la frontière les
  fautes de frappe du type `AppError::status(700)`, au lieu de laisser
  des codes non conformes partir sur le réseau.
- **Injection de CRLF dans les en-têtes.** Chaque nom et chaque valeur
  d'en-tête est validé par les `HeaderName::try_from` /
  `HeaderValue::try_from` de hyper. Tout en-tête rejeté est écarté avec
  un avertissement dans le journal et la réponse est construite sans
  lui. Les valeurs contrôlées par un attaquant qui se retrouvent
  reflétées dans un en-tête (allow-headers CORS, `X-Forwarded-*`,
  en-têtes de débogage maison) ne peuvent pas scinder la réponse.

Les deux filtres sont silencieux sur le chemin nominal - vous ne les
voyez dans les journaux que lorsque quelque chose a tenté de passer
entre les mailles.

## Macros de réponse

Deux macros de la forme `Response` existent pour les cas courants :

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

Les deux se développent en `Ok(HttpResponse::...)`. Chaînez les méthodes de
`ResponseExt` sur l'une comme sur l'autre pour ajuster le statut, les
en-têtes ou les cookies.

## Cookies

`Cookie::new(name, value)` produit un cookie aux valeurs par défaut
sûres - `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`. Redéfinissez-les
cookie par cookie :

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

Trois constructeurs de commodité couvrent les motifs courants :

- `Cookie::forget(name)` - valeur vide, `Max-Age=0`. Utilisez-le à la
  déconnexion pour demander au navigateur d'abandonner le cookie.
- `Cookie::forever(name, value)` - `Max-Age` de cinq ans.
- `Cookie::encrypted(name, plaintext)` - chiffré en AES-256-GCM et lié
  à l'AAD `CryptPurpose::Cookie`, pour qu'un chiffré de cookie ne
  puisse pas être rejoué vers une autre surface du framework (curseurs,
  secrets 2FA, casts). Exige que `APP_KEY` soit défini à l'amorçage.
  Son homologue `Cookie::read_encrypted(wire)` déchiffre une valeur
  produite par le même chemin. Voir [Chiffrement](encryption.md).

La sérialisation de l'en-tête encode en pourcentage chaque octet qui
n'est pas un cookie-octet valide au sens de la RFC 6265, caractères de
contrôle compris. Un CRLF dans un nom ou une valeur de cookie est
encodé, pas propagé - l'injection d'en-tête par les cookies est fermée
au niveau du sérialiseur.

## Redirections

`Redirect` couvre toute la surface du redirecteur de Laravel. Chaque
variante implémente `From<Redirect> for Response`, donc la forme
idiomatique est `Redirect::...().into()`.

### Cibles

```rust
use suprnova::{Redirect, redirect_to};

// URL ou chemin explicite
let _ = Redirect::to("/dashboard");

// La même chose, en fonction libre un peu plus courte
let _ = redirect_to("/dashboard");

// Route nommée (retourne RedirectRouteBuilder)
let _ = Redirect::route("users.show").with("id", "42");

// URL externe explicite - identique à `to`, mais le nom signale
// « on quitte le site » pour les audits de redirection ouverte
let _ = Redirect::away("https://external.example.com");

// Rafraîchit la page (lit l'URL précédente dans la session ; retombe
// sur "/" si aucune portée de session n'est active)
let _ = Redirect::refresh();

// Idem, mais en prenant une Request explicite quand aucune portée
// n'est active
// let _ = Redirect::refresh_for(&request);

// previous_url de la session, avec repli quand aucune session n'est en portée
let _ = Redirect::back("/login");

// URL prévue stockée en session, consommée à la lecture, avec repli
let _ = Redirect::intended("/home");

// Redirection invité : met de côté l'URL de la requête courante comme
// « prévue » et envoie l'utilisateur vers une page de connexion
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`, `Redirect::intended`, `Redirect::guest` et
`Redirect::refresh` s'intègrent tous à la session. Sans portée de
session, ils retombent silencieusement sur leurs valeurs par défaut -
pratique pour des montages de test partiels. Voir
[Sessions](session.md).

### Validation de la route nommée

La macro procédurale `redirect!` valide le nom de route à la
compilation et se développe en `Redirect::route(name)` :

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // La compilation échoue si "users.index" n'est pas un nom de route
    // enregistré ; le message d'erreur liste les routes disponibles et
    // suggère les correspondances proches.
    redirect!("users.index").into()
}
```

### Codes de statut

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

La valeur par défaut est 302.

### Données flash

Les builders `Redirect` portent leur propre flash bag. À la conversion
en `Response`, le sac se vide dans la session vivante et survit à
exactement une requête de plus :

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // clé/valeur unique
    .with_input([                              // repeupler le formulaire
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // sac d'erreurs par défaut
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // sac d'erreurs nommé
        ("password", "Required"),
    ]);
```

La page réceptrice les relit via `session.get(...)` (pour `with`),
`session.get_old_input(...)` (pour `with_input`), et la table de sacs
que vide `session.pull_errors_flash()` (pour `with_errors` /
`with_errors_bag`). La couche Inertia consomme automatiquement le flash
d'erreurs - la prop `errors` de chaque réponse Inertia est amorcée
depuis la session, si bien que `Redirect::back().with_errors(...)` fait
apparaître les messages sur la destination sans câblage supplémentaire.
L'en-tête de requête `X-Inertia-Error-Bag` cantonne la prop sous un sac
nommé pour les pages à plusieurs formulaires.

Notez que sur `RedirectRouteBuilder` (ce que retournent
`Redirect::route` et `redirect!`), `.with(key, value)` définit un
**paramètre de route**, pas une entrée flash - utilisez
`.flash(key, value)` à cet endroit :

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // paramètre de route
    .flash("status", "Updated");               // flash de session
```

### Cookies, en-têtes, fragments

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // ajoute #invoices
    .without_fragment();                       // OU retire tout fragment antérieur
```

`with_fragment` accepte le fragment avec ou sans `#` initial. Appeler
`with_fragment` après `without_fragment` en rattache un.

### Préserver le fragment à travers la redirection

Pour les applications Inertia où la destination doit préserver le hash
de l'URL *d'origine*, utilisez `preserve_fragment` :

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

À la conversion, cela dépose `_inertia.preserve_fragment = true` en
flash dans la session ; la réponse Inertia suivante lit le flag et émet
`preserveFragment: true` dans son objet de page. Pas de portée de
session - le flag est silencieusement abandonné.

### Redirections signées

Deux builders enveloppent la surface de signature d'URL pour les
redirections à usage unique vers des routes nommées (réinitialisation de
mot de passe, vérification d'e-mail, liens de téléchargement) :

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

Les deux retournent `Result<Redirect, FrameworkError>` - propagez
l'erreur avec `?`, puisque `Redirect` se convertit proprement en
`Response`. Voir [Génération d'URL](urls.md) pour la surface de
signature.

### Enregistrer l'URL prévue

`Redirect::set_intended_url` écrit la cible prévue dans la session sans
effectuer de redirection - typiquement appelé depuis un middleware
d'authentification avant de rediriger vers `/login`, pour qu'un
`Redirect::intended` ultérieur puisse récupérer l'URL demandée à
l'origine :

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## Interrompre depuis un handler

Trois fonctions libres court-circuitent un handler à un statut donné.
Elles retournent `Result<(), FrameworkError>` ; combinez-les avec `?` :

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

L'erreur sous-jacente est `FrameworkError::Domain { message, status_code }`,
si bien qu'elle se rend à travers la même enveloppe JSON et les mêmes
règles d'assainissement des 5xx que tout autre chemin d'erreur. Les
codes de statut hors limites sont forcés à 500 par le moteur de rendu de
réponse. Voir [Modèle d'erreur](error-model.md) pour le contrat de
conversion complet.

## Retourner des erreurs directement

Comme `Response` est `Result<HttpResponse, HttpResponse>`, vous pouvez
retourner directement une branche `Err` - utile quand la forme de la
réponse est déjà un corps JSON précis et que vous le voulez tel quel sur
le réseau :

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

Pour quoi que ce soit de plus riche - erreurs de domaine typées,
validation, observabilité - utilisez la surface du
[Modèle d'erreur](error-model.md) (`AppError`, `FrameworkError`,
`#[domain_error]`).

## Référence rapide

| Besoin | Utiliser |
|---|---|
| Réponse JSON | `HttpResponse::json(v)` ou `json_response!({...})` |
| Réponse texte | `HttpResponse::text(s)` ou `text_response!(s)` |
| Réponse HTML | `HttpResponse::html(s)` |
| Octets bruts + content-type | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent Events | `HttpResponse::sse(stream)` - voir [SSE](sse.md) |
| Flux par blocs | `HttpResponse::stream_bytes(stream)` |
| Définir le statut | `.status(code)` |
| Ajouter un en-tête | `.header(k, v)` / `.with_headers([...])` |
| Retirer un en-tête | `.without_header(name)` |
| Attacher un cookie | `.cookie(c)` / `.with_cookies([...])` |
| Oublier un cookie | `.without_cookie(name)` |
| Redirection simple | `Redirect::to(path).into()` ou `redirect_to(path).into()` |
| Redirection vers une route nommée | `redirect!("name").into()` ou `Redirect::route("name")` |
| Redirection en arrière | `Redirect::back(fallback)` |
| Redirection vers l'URL prévue | `Redirect::intended(default)` |
| Redirection invité (met de côté l'URL prévue) | `Redirect::guest(&req, login)` |
| Définir la cible prévue | `Redirect::set_intended_url(url)` |
| URL externe | `Redirect::away(url)` |
| Rafraîchir la page courante | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| Redirection vers une route signée | `Redirect::signed_route(name, &[(k, v)])?` |
| Paramètre de route sur la redirection | `.with("key", "value")` |
| Paramètre de requête sur la redirection | `.query("key", "value")` |
| Données flash | `.with(key, value)` (ou `.flash` sur `RedirectRouteBuilder`) |
| Saisie flash | `.with_input([(k, v), ...])` |
| Erreurs flash | `.with_errors([(k, msg), ...])` |
| Sac d'erreurs nommé | `.with_errors_bag(bag, [(k, msg)])` |
| Ajouter un fragment | `.with_fragment("section")` |
| Retirer le fragment | `.without_fragment()` |
| Préserver le fragment (Inertia) | `.preserve_fragment()` |
| Redirection permanente | `.permanent()` (301) |
| Statut de redirection personnalisé | `.status(303)` |
| Interrompre tôt | `abort_with(code, msg)?`, `abort_if(cond, code, msg)?`, `abort_unless(cond, code, msg)?` |

## Suivant

- [Modèle d'erreur](error-model.md) - `FrameworkError`, `AppError`,
  `HttpError`, et l'unique conversion qui rend chaque erreur en une
  `HttpResponse`
- [Gestion des erreurs](errors.md) - les motifs pratiques de handler
  pour `?`, `AppError` et les erreurs de domaine personnalisées
- [Événements serveur](sse.md) - construire et consommer des réponses
  `sse(...)`
- [Génération d'URL](urls.md) - URL signées, résolution des routes
  nommées, la surface derrière `Redirect::signed_route`
- [Sessions](session.md) - les données flash, les URL prévues, le sac
  dans lequel écrivent `Redirect::with`/`with_input`/`with_errors`
