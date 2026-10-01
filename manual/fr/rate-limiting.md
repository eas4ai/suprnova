# Limitation de débit

Suprnova livre deux surfaces de limitation de débit complémentaires :

| Surface | À utiliser quand... | Backend |
|---------|----------------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | Vous voulez une application stricte à fenêtre glissante contre un stockage arbitraire (ZSET Redis, deque en mémoire) | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | Vous voulez des limiteurs nommés à la Laravel, des callbacks de workflow `attempt()`, ou des en-têtes de réponse `X-RateLimit-*` | Magasin `Cache` (mémoire ou Redis) |

Le driver à fenêtre glissante est la forme native de Suprnova - un
slot par requête, pas de clé de timer séparée, éval Lua atomique sur
Redis. La façade Laravel est ce que les apps migrées utilisent
d'instinct et ce qu'exige le pattern limiteur-nommé /
callback-de-réponse. Les deux coexistent par conception, et une
route peut superposer les deux.

## SPI du driver à fenêtre glissante

`RateLimiterDriver` est le SPI de stockage pour l'algorithme à
fenêtre glissante. Chaque clé suit une deque d'horodatages de hit. À
chaque `try_acquire`, les entrées plus anciennes que `now - window`
sont évincées ; si le compte restant est sous `max_requests`, `now`
est ajouté et l'appel accepte. Sinon, il rejette.

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait est l'Option<Duration> jusqu'à ce que le plus vieux slot
    // du seau expire.
}
```

### Drivers intégrés

| Driver | Stockage | Sélectionné via |
|--------|----------|------------------|
| `InMemoryRateLimiter` | `HashMap<String, Bucket>` par process, avec `tokio::time::Instant` pour que les tests `start_paused` puissent piloter l'horloge | `RATE_LIMIT_DRIVER=memory` (par défaut) |
| `RedisRateLimiter` | ZSET Redis + vérification-et-enregistrement atomique en Lua | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` câble le driver correspondant dans le
conteneur. Hors production, une valeur de driver inconnue retombe
sur memory avec un log `warn!`.

### La production échoue fermée sur le driver en mémoire

En production, résoudre vers le limiteur en mémoire fait échouer
l'amorçage :

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

Le driver en mémoire garde ses seaux dans le tas d'un seul process.
Derrière N réplicas, chacun garde son propre compte, si bien qu'une
limitation « 5 tentatives par 15 minutes » sur la réinitialisation de
mot de passe est en réalité 5N, et chaque déploiement les remet tous
à zéro. La limite que vous avez configurée n'est pas la limite que
vous obtenez - et rien ne le signale, parce que les requêtes
réussissent, ce à quoi ressemble une limitation qui fonctionne, vue
de l'extérieur. Cela se manifeste comme un incident de bourrage
d'identifiants ou d'énumération de comptes, pas comme une erreur.

Une valeur de driver **non reconnue** échoue pour la même raison :
elle retombe sur memory. `RATE_LIMIT_DRIVER=Redis` - avec une
majuscule - avertirait sinon une fois à l'amorçage et laisserait
silencieusement un déploiement multi-réplica limiter par process.
C'est le cas le plus susceptible d'atteindre la production, parce
qu'il a l'air configuré.

Soit vous le pointez vers Redis :

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

soit, si vous faites vraiment tourner un seul process, vous le
déclarez :

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

Le développement, les tests et le **staging** ne sont pas touchés.
Le staging n'est délibérément pas filtré par ce contrôle, selon le
même raisonnement que le garde-fou du mail : le faire échouer
durement pousserait les équipes à activer la dérogation de façon
globale, ce qui désarme le contrôle précisément là où il compte.

### `RateLimitMiddleware`

L'enveloppe HTTP autour du driver. Construisez-la avec une closure
`key_fn` pour piloter la sélection du seau par requête :

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

En cas de rejet (au-dessus du quota), elle renvoie un HTTP 429 avec
un en-tête `Retry-After`.

### Limiter par destinataire, pas seulement par appelant

Une limite à clé d'adresse répond à *un client fait-il trop de
requêtes*. Elle ne peut pas répondre à *une boîte mail est-elle en
train d'être inondée*. Un attaquant réparti sur un botnet, un pool de
proxys, ou un seul `/64` IPv6 reste sous chaque budget par IP tout en
envoyant à une victime des milliers d'e-mails de réinitialisation de
mot de passe - la boîte de réception est la ressource épuisée, et
l'adresse de la victime est la seule chose que ces requêtes ont en
commun. L'inverse fait mal aussi : derrière un NAT de classe
opérateur ou une passerelle de bureau, les limites par IP punissent
une foule pour le comportement d'un seul de ses membres.

`identity_key` indexe un seau sur le compte *visé par l'action* :

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Empilez-le *à côté* d'un limiteur par IP plutôt que de remplacer l'un
par l'autre. Chacun attrape ce que l'autre ne peut pas : le par-IP
arrête un hôte qui énumère plusieurs adresses ; le par-destinataire
arrête plusieurs hôtes qui ciblent une seule adresse.

Trois détails portent la sécurité :

- **`key_reads_body`** met le corps en tampon (jusqu'au plafond
  donné) avant que la clé ne soit calculée, si bien que le champ
  peut être lu aussi bien depuis un POST form-encodé que depuis une
  query string. C'est opt-in, parce que la mise en tampon est un
  travail qu'un appelant non authentifié peut vous faire faire ; le
  plafond le borne. Un corps au-dessus du plafond est rejeté avec un
  413 plutôt que transmis sans clé - sinon, remplir le corps de
  padding serait un moyen de sortir de la limite.
- **`only_when`** saute le limiteur pour les requêtes qui ne nomment
  personne. Sans cela, elles tombent dans le repli sur adresse
  d'`identity_key` et sont comptées contre le quota de *ce*
  limiteur - et comme un budget par destinataire est normalement le
  plus serré des deux, il deviendrait silencieusement la limite
  contraignante pour chaque route qui ne nomme personne.
- **La valeur est normalisée et hachée.** `Alice@Example.com` et
  `alice@example.com` atteignent la même boîte mail et doivent
  partager un seau, sinon la limite est contournée en changeant la
  casse. Le résultat est haché parce qu'un backend de limitation de
  débit est fréquemment un Redis partagé avec un contrôle d'accès
  plus faible que la base de données primaire, et un dump de clés
  ne devrait pas se lire comme une liste de qui est en train de
  réinitialiser son mot de passe.

### Politique d'erreur backend

`BackendErrorPolicy` gouverne ce qui se passe quand le *backend* du
limiteur lui-même est en erreur - par exemple, Redis est injoignable -
à distinguer d'une requête qui dépasse légitimement son quota. Le
backend ne peut pas prendre de décision, si bien que le middleware
doit choisir entre la disponibilité et la garantie de la limite.

| Policy | Comportement | Quand l'utiliser |
|--------|--------------|-------------------|
| `FailOpen` (défaut) | Laisse passer la requête ; log en `warn` | La plupart des API publiques - une panne du limiteur ne devrait pas faire tomber le trafic |
| `FailClosed` | Rejette avec HTTP 503 + `Retry-After: 1` ; log en `error` | Routes sensibles (connexion, réinitialisation de mot de passe, paiements) où un trafic non borné pendant une panne du backend est pire qu'un rejet bref |

Choisissez avec `.on_backend_error(BackendErrorPolicy::FailClosed)`
sur le middleware. Les requêtes à quota épuisé sont toujours en 429,
quelle que soit la policy - la policy n'affecte que le repli en cas
d'erreur backend.

## Façade à la Laravel adossée au cache

`RateLimiter` (la struct) reflète `Illuminate\Cache\RateLimiter`.
C'est un compteur à fenêtre fixe construit par-dessus la façade
[`Cache`](cache.md) de Suprnova. Utilisez-la pour des limiteurs
nommés, des workflows `attempt()`, ou chaque fois que vous voulez les
en-têtes `X-RateLimit-*` que les apps Laravel attendent.

### Disposition du stockage

Pour une clé de compteur de tentatives `K` avec un decay de `D`
secondes :

- `K` - compteur i64 incrémenté à chaque `hit`. La valeur d'amorçage
  est 0 (via `Cache::add`).
- `K:timer` - i64 en secondes unix depuis l'epoch, pour quand la
  fenêtre se termine, positionné via `Cache::add` si bien que seul
  le premier appelant d'une fenêtre fixe l'échéance.

Les deux clés portent le même TTL, si bien que le cache les nettoie
automatiquement quand la fenêtre se termine. Quand le compteur a
atteint `max_attempts` mais que `:timer` a disparu,
`too_many_attempts` réinitialise le compteur - c'est ce qui fait
glisser la fenêtre vers l'avant après une période à quota épuisé.

### API du compteur

```rust
use suprnova::RateLimiter;

// Brûle une tentative ; amorce la fenêtre si elle est absente.
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// Brûle une tentative ET teste la limite en un seul aller-retour
// atomique. Renvoie `true` quand ce hit a poussé le seau au-dessus
// de `max` (refuser la requête), `false` quand elle a été admise.
// Utilisez ceci plutôt qu'une paire séparée `too_many_attempts` +
// `hit` : vérifier puis frapper en deux appels laisse des requêtes
// concurrentes se glisser sous la limite (une race check-then-act).
// `i64::MAX` comme max signifie « illimité » - admet toujours, mais
// compte quand même.
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* retourner 429 */ }

// Incrémente de N ; utile pour des limites « pondérées par coût »
// (chaque requête brûle plus d'une tentative).
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// Lit le compte actuel (0 quand jamais frappé ou expiré).
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// Nombre de secondes jusqu'à ce que la fenêtre rouvre (0 quand
// aucune fenêtre n'est ouverte).
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// Tentatives restantes avant de basculer.
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left est l'alias à l'orthographe Laravel de remaining.
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// Le seau est-il au-dessus de sa limite MAINTENANT (avec la
// fenêtre encore ouverte) ?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// Supprime seulement le compteur (le timer reste - la fenêtre est
// toujours fixée).
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// Supprime à la fois le compteur et le timer.
RateLimiter::clear("login:1.2.3.4").await?;
```

### Workflow `attempt()`

Exécute un callback seulement quand le seau est sous quota ; le hit
n'est brûlé que quand le callback s'exécute :

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* le callback a tourné, la tentative comptée */ }
    None => { /* au-dessus de la limite, le callback n'a PAS tourné */ }
}
```

C'est la bonne forme pour les formulaires de connexion - vous ne
brûlez pas de tentative à moins que le travail n'ait réellement
atteint le callback.

### Limiteurs nommés

Enregistrez à l'amorçage, résolvez au moment de la requête. Le nom
côté Laravel `for` est un mot-clé réservé en Rust, si bien que le nom
principal côté Rust est `define` ; l'alias Laravel littéral est
exposé via `r#for`.

```rust
use suprnova::{Limit, RateLimiter};

// À l'amorçage - `define` est le nom principal côté Rust.
RateLimiter::define("api", |req| {
    // `req.ip()`, pas l'en-tête brut `X-Forwarded-For` - voir plus bas.
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Alias côté Laravel - la même chose sous l'orthographe échappée du mot-clé.
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// Résolution.
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

Un callback de limiteur nommé renvoie un [`LimitResult`],
constructible à partir de :

- Un seul `Limit` - applique cette limite.
- Un `Vec<Limit>` - applique chaque limite ; la première à basculer
  l'emporte.
- Une `HttpResponse` - court-circuite immédiatement avec cette
  réponse (utilisé pour « l'admin a un accès illimité » via
  `Limit::none()`, ou pour refuser carrément la requête).

### Assainir les clés

`RateLimiter::clean_rate_limiter_key(key)` retire les marqueurs
d'entité HTML `&abc;` d'une clé - Laravel utilise ceci pour les
chaînes fournies par l'utilisateur qui font l'aller-retour par
`htmlentities`. Suprnova reproduit exactement l'étape de retrait mais
NE préfixe PAS l'encodage `htmlentities` (qui n'a d'importance que
pour les entrées non-UTF-8, sans intérêt pour un `String` Rust). La
fonction est déterministe et idempotente à l'intérieur de Suprnova ;
les consommateurs qui ont besoin d'un hachage identique octet pour
octet avec un service PHP devraient faire tourner leur propre étape
préalable `htmlentities` sur l'entrée.

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## Builder `Limit`

Le type de données renvoyé par les callbacks de limiteur nommé. Des
constructeurs raccourcis reflètent le `Limit::per*` de Laravel :

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 10 par 1 seconde (max_attempts, decay_seconds)
Limit::per_minute(60);              // 60 par minute
Limit::per_minutes(5, 100);         // 100 par 5 minutes (decay en premier, signature Laravel)
Limit::per_hour(1_000);             // 1000/h
Limit::per_hours(6, 5_000);         // 5000 par 6 heures
Limit::per_day(10_000);             // 10000/jour
Limit::per_days(7, 50_000);         // 50000 par 7 jours
Limit::new(123, Duration::from_secs(45));  // ctor nu

// Chaîne de builder.
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - fixe la clé du seau. Une clé vide veut dire « global »
  (chaque appelant partage un seul seau).
- `.response(callback)` - génère une réponse personnalisée quand la
  limite bascule ; le défaut est un simple 429 « Too Many
  Attempts. ».
- `.after(callback)` - ne brûle la tentative que quand
  `callback(response)` renvoie true. Usage canonique : ne compter
  que les connexions échouées (`after(|r| r.status_code() >= 400)`).

`Limit::none()` renvoie un `Unlimited` (un `GlobalLimit` avec
`max_attempts = i64::MAX`). Le renvoyer depuis un limiteur nommé est
le pattern Laravel pour le contournement. `GlobalLimit` lui-même est
une fine enveloppe autour de `Limit` avec une clé vide, gardée pour
la parité avec `Illuminate\Cache\RateLimiting\GlobalLimit`.

## `ThrottleRequestsMiddleware`

Enveloppe HTTP autour de la façade adossée au cache. Reflète
`Illuminate\Routing\Middleware\ThrottleRequests`. Trois
constructeurs :

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// Limiteur nommé - se résout au moment de la requête via RateLimiter::limiter(name).
ThrottleRequestsMiddleware::by_name("api");

// max/decay/préfixe en ligne - la forme littérale Laravel `throttle:60,1`.
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Liste explicite de Limits - la première à basculer l'emporte ; la plus idiomatique en Rust.
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

Câblez-le dans un groupe de routes :

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### Clé sur `req.ip()`, jamais sur l'en-tête

`X-Forwarded-For` est fourni par l'appelant. Un limiteur indexé sur
l'en-tête brut est déjoué en envoyant une valeur différente à chaque
requête - l'attaquant choisit son propre seau, si bien que le quota
devient par-requête plutôt que par-client.

`Request::ip()` est la lecture sûre. Elle renvoie `X-Forwarded-For` /
`X-Real-IP` **seulement quand le pair TCP est listé dans
`APP_TRUSTED_PROXIES`**, et sinon l'adresse du pair, si bien qu'un
en-tête venant de n'importe qui d'autre que votre propre proxy est
ignoré.

Le corollaire compte tout autant : avec cette variable non définie -
le défaut - `req.ip()` derrière un proxy qui termine la connexion
renvoie l'adresse *du proxy* à chaque requête, et chaque limite par
IP de l'app s'effondre en un seul seau partagé.
`ThrottleRequestsMiddleware::with(20, 1, "login")` signifie alors 20
tentatives par minute pour tous les utilisateurs combinés, que
n'importe quel appelant peut dépenser pour verrouiller tout le
monde. Déployer derrière nginx, Traefik, un ALB ou Cloudflare
signifie définir
[`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies).

### En-têtes de réponse

Chaque réponse enveloppée porte :

- `X-RateLimit-Limit` - le `max_attempts` configuré.
- `X-RateLimit-Remaining` - tentatives restantes pour ce seau.

Les réponses 429 portent en plus :

- `Retry-After` - secondes jusqu'à ce que la fenêtre rouvre.
- `X-RateLimit-Reset` - secondes unix depuis l'epoch quand le seau
  rouvre.

Cela correspond exactement à la forme du
`ThrottleRequests::getHeaders` de Laravel.

### Limiteur nommé manquant

Quand une route est câblée à `by_name("X")` mais qu'aucun limiteur
sous `X` n'a été enregistré, le middleware renvoie un HTTP 503 avec
un corps qui nomme le limiteur manquant. Laravel lève
`MissingRateLimiterException` ; nous l'exposons comme une réponse
HTTP pour qu'un amorçage mal configuré ne fasse pas paniquer le
worker thread.

### Composition driver-vs-façade

Les deux middlewares peuvent coexister sur un seul router. Superposez
le driver à fenêtre glissante pour l'équité de bas niveau, puis la
limitation adossée au cache pour des limites nommées par point de
terminaison :

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## Configuration

Le SPI du driver se configure via des variables d'environnement ; la
façade adossée au cache se configure là où votre magasin
[`Cache`](cache.md) est configuré (mémoire ou Redis).

| Variable | Utilisée par | Défaut |
|----------|---------------|--------|
| `RATE_LIMIT_DRIVER` | Amorçage du SPI du driver | `memory` (refusé en production - voir ci-dessus) |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | Dérogation à l'échec fermé en production | non défini |
| `RATE_LIMIT_REDIS_URL` | Driver Redis | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Préfixe de clé Redis | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Façade `RateLimiter` adossée au cache (voir [`Cache`](cache.md)) | divers |

## Migration depuis Laravel

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` ou `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` middleware | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` middleware | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` headers | Mêmes en-têtes, même forme |

### Pourquoi Suprnova diverge

Laravel livre une seule forme : `Illuminate\Cache\RateLimiter`
(compteur à fenêtre fixe adossé au cache) avec
`Illuminate\Routing\Middleware\ThrottleRequests` comme enveloppe
HTTP. Suprnova livre à la fois cette forme *et* un SPI de driver à
fenêtre glissante natif, parce que deux vraies questions ont besoin
de deux vraies réponses.

Un compteur adossé au cache est la bonne réponse à « j'ai des
limiteurs nommés, des callbacks de réponse, des after-callbacks pour
ne compter que les connexions échouées, et je veux rester compatible
au niveau source avec les migrations Laravel. » C'est la mauvaise
réponse à « j'ai besoin d'une application exacte à un
slot-par-requête, à fenêtre glissante, contre un ZSET Redis avec éval
Lua atomique et sans clé de timer séparée. » Cette seconde question
est ce que la plupart des services Rust qui heurtent les limites de
concurrence de Tokio ont réellement, si bien que `RateLimiterDriver` +
`RateLimitMiddleware` existent en parallèle, pas derrière un flag
de feature.

La policy d'erreur backend est aussi un ajout de Suprnova. Le
middleware de Laravel ne fait jamais remonter une décision « le
limiteur est cassé » parce que le cycle de vie par-requête de PHP la
cache - la requête suivante obtient un process neuf. Un worker Tokio
longue durée qui perd Redis pendant dix secondes doit décider quoi
faire des requêtes qui arrivent pendant cette fenêtre ;
`BackendErrorPolicy::FailOpen` (défaut) contre `FailClosed` est
cette décision exposée explicitement.

## Suivant

- [Middleware](middleware.md) - comment le middleware se compose,
  s'exécute, et court-circuite dans la chaîne de requête
- [Cache](cache.md) - le magasin sur lequel la façade `RateLimiter`
  à la Laravel est construite
- [Configuration](configuration.md) - la config typée pour les
  backends cache et Redis
- [Flux d'authentification](auth-flows.md) - `LoginThrottleMiddleware`
  et le pattern de verrouillage anti-force-brute s'appuient sur
  cette surface
- [Modèle d'erreur](error-model.md) - pourquoi `Result<HttpResponse,
  HttpResponse>` laisse le middleware court-circuiter proprement
