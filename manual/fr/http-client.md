# Client HTTP

La façade `Http` est le côté sortant du HTTP - l'équivalent Rust du
helper `Http::` de Laravel. Vous vous en servez quand votre handler,
job, ou tâche planifiée doit appeler l'API de quelqu'un d'autre : une
passerelle de paiement, un géocodeur, une cible de webhook, un message
Slack. Builder fluide, JSON en entrée et en sortie, réessais avec
jitter, fakes de test déterministes qui enregistrent ce que vous avez
envoyé. La même surface que vous utilisiez sous Laravel, avec une
isolation task-local pour que les tests parallèles ne voient pas les
fakes les uns des autres.

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

Voilà la forme : `Http::<verbe>(url)` retourne un `RequestBuilder` ;
vous chaînez de la configuration par-dessus ; `.send().await` retourne
une `ClientResponse`. Le client sous-jacent est un unique
`reqwest::Client` partagé avec TLS rustls, un timeout par défaut de
30s, et un user agent `suprnova/<version>` - construit paresseusement
au premier appel.

## Les verbes

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

Chaque verbe retourne un `RequestBuilder`. L'URL peut être n'importe
quel `impl Into<String>` - un `&str`, un `String`, ou un `Cow<str>`.
Aucun helper de construction d'URL n'est livré dans la façade ;
formatez l'URL vous-même ou faites appel à une crate de chaîne de
requête.

## Corps

Trois façons d'attacher un corps. Chacune remplace tout corps
précédemment défini.

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` accepte tout ce qui implémente `serde::Serialize`. Le
`Content-Type` sur le réseau est défini automatiquement à
`application/json`. Si la sérialisation échoue (par ex. une map avec
une clé non-chaîne), le builder enregistre l'erreur et `send()` la
fait surface au lieu d'envoyer silencieusement un corps `null`.

### Formulaire

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` sérialise la valeur en `application/x-www-form-urlencoded`.
La valeur doit se sérialiser en un objet JSON ; les clés deviennent
les champs du formulaire. Même sémantique d'erreur de corps que
`.json` - un échec de sérialisation fait surface à travers
`send().await?`, jamais comme un corps vide silencieux.

### Octets bruts

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` prend tout ce qui est `impl Into<Bytes>`. Vous êtes
responsable de l'en-tête `Content-Type` - `.body` n'en définit pas.

## En-têtes et authentification

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` ajoute ; le framework ne déduplique pas, donc
deux appels avec le même nom envoient deux en-têtes et reqwest les
joint selon la sémantique HTTP. Deux raccourcis pour les schémas
d'authentification courants :

- `.bearer_token(token)` - définit `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - définit `Authorization: Basic <b64>` ;
  `password` est `Option<&str>` donc `.basic_auth("api-key", None)`
  encode la forme `api-key:` que certains fournisseurs veulent

## Délais d'attente

Le client partagé a un délai d'attente par défaut de 30 secondes.
Redéfinissez-le par requête quand vous en avez besoin :

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` redéfinit à la fois le délai de connexion et le délai
total de la requête pour cet appel unique. Il n'y a pas de bouton
`connect_timeout` séparé sur le builder ; le client reqwest
sous-jacent utilise un délai combiné unique.

## Redirections

Le client partagé suit les redirections par défaut (jusqu'au plafond de `reqwest`,
10) - le bon comportement quand vous appelez un point de
terminaison de confiance qui répond `http → https` ou vous transmet
une URL de CDN.

Quand l'URL de la requête est influencée par une entrée non fiable, ce
défaut devient un vecteur de falsification de requête côté serveur
(SSRF) : un point de terminaison hostile peut répondre avec un `3xx`
dont `Location` pointe vers un service interne ou une adresse de
métadonnées cloud (`http://169.254.169.254/…`), et un client qui suit
les redirections s'y rendrait. Désactivez le suivi de redirection pour
ces requêtes avec `.no_redirects()` :

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// Le 3xx est retourné tel quel au lieu d'être suivi - inspectez-le et
// rejetez plutôt que de laisser le client suivre l'en-tête Location.
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` fait passer la requête par un client séparé qui ne
suit pas les redirections ; le client par défaut - et chaque requête
qui ne l'appelle pas - reste inchangé. C'est l'analogue pour client
générique du verrouillage de redirection que l'expéditeur web-push
applique déjà aux points de terminaison push contrôlés par un
attaquant.

## Réessais

`Http` livre des réessais à backoff exponentiel avec jitter complet - la recette AWS, la même que Laravel utilise. Les deux modes de réessai gèrent les échecs de transport pour toutes les méthodes HTTP. Ils diffèrent sur le point de savoir si un 5xx reçu peut rejouer `POST` et `PATCH`.

### `.retry(max_attempts, base_backoff)` - réessais de transport pour toutes les méthodes

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` inclut le premier essai, donc `retry(4, ...)` réessaie jusqu'à trois fois après la tentative initiale. Le délai avant la tentative `n+1` est une durée aléatoire uniforme dans `[0, base_backoff * 2^(n-1)]`, plafonnée à 30 secondes. Jitter complet, pas un backoff exponentiel plus un sommeil fixe, si bien que de nombreux workers qui réessaient la même panne ne se synchronisent pas en une ruée.

`.retry()` réessaie les échecs de transport pour toutes les méthodes. Si une réponse arrive, il réessaie un statut 5xx sauf si la méthode est `POST` ou `PATCH`. Il renvoie les réponses 4xx et 2xx/3xx telles quelles. Après épuisement des réessais, la dernière réponse ou la dernière erreur de transport est renvoyée à l'appelant.

Cette distinction compte pour les écritures. Un échec de transport sur `POST` ou `PATCH` peut signifier que le serveur a commité l'écriture mais que la réponse a été perdue, et pourtant le contrat actuel réessaie quand même cet échec. Une réponse 5xx reçue pour ces méthodes est renvoyée après une seule tentative, sauf si l'appelant utilise `.retry_non_idempotent(...)`.

### `.retry_non_idempotent(...)` - opt-in pour POST/PATCH

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

Quand vous avez fourni une clé d'idempotence que l'amont honore, ou que vous avez autrement rendu la requête sûre à rejouer, passez à `.retry_non_idempotent(...)`. Elle conserve les réessais d'erreur de transport pour toutes les méthodes et autorise en plus les réessais de réponse 5xx pour `POST` et `PATCH`. Elle renvoie toujours les réponses 4xx et 2xx/3xx telles quelles.

### Retry-After est honoré sur 503

Pour un `503 Service Unavailable`, le framework respecte un en-tête `Retry-After` - sous forme de delta-secondes (`Retry-After: 30`) ou de date HTTP (`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`). L'attente réelle est la plus grande des deux entre le backoff jitterisé et l'indication `Retry-After`, toujours plafonnée à 30 secondes. Un serveur hostile ou mal configuré qui retourne `Retry-After: 86400` ne mettra pas votre tâche en pause pour une journée entière.

### `.retry_when(predicate)`  -  restreindre davantage la politique

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .retry_when(|ctx| ctx.method == "GET")
    .send()
    .await?;
```

`retry_when` enregistre un prédicat consulté avant chaque réessai que la politique ci-dessus effectuerait autrement. Il peut opposer son veto à un réessai éligible, mais ne peut pas en créer un. En particulier, il ne peut pas transformer une réponse 2xx, 3xx ou 4xx en réessai, et il ne peut pas rendre un 5xx reçu réessayable pour `POST` ou `PATCH` sans `.retry_non_idempotent(...)`. Il est consulté avant les réessais d'erreur de transport pour toutes les méthodes, y compris `POST` et `PATCH` configurés avec `.retry()` simple. Sans politique `.retry(...)` ou `.retry_non_idempotent(...)`, un `retry_when` isolé n'a rien à opposer.

Le prédicat reçoit `RetryContext { attempt, method, url, outcome }`, où `outcome` vaut `RetryOutcome::TransportError` (l'envoi a échoué avant qu'une réponse n'arrive) ou `RetryOutcome::Status(n)` (une réponse 5xx éligible).


## Lire la réponse

`ClientResponse` expose le statut, les en-têtes, et trois méthodes de
lecture du corps. Chaque méthode de corps consomme la réponse.

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// Choisissez-en une - chacune consomme la réponse.
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` est insensible à la casse. `.json::<T>()` retourne
`Result<T, FrameworkError>` et utilise `serde_json` pour le décodage.
`.text()` impose l'UTF-8 et fait surface une `FrameworkError` si le
corps n'est pas de l'UTF-8 valide.

### Plafond du corps de réponse

Un amont lent ou hostile peut sinon diffuser un corps non borné en
mémoire. Pour s'en protéger, chaque lecture de corps mise en tampon
est plafonnée - 25 Mio par défaut. Redéfinissez globalement à
l'amorçage :

```rust
use suprnova::Http;

// Une fois, quelque part dans bootstrap.
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 Mio
```

Ou par requête quand un appel gère légitimement une charge utile plus
grande :

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 Mio
    .send()
    .await?
    .bytes()
    .await?;
```

Une réponse qui déclare un `Content-Length` au-dessus du plafond est
rejetée avant que le moindre corps ne soit lu ; la boucle de streaming
impose aussi le plafond contre les octets réels, au cas où
`Content-Length` serait absent ou mentirait.

## Échappatoire - reqwest brut

Le framework couvre les cas courants. Quand vous avez besoin de
quelque chose que nous n'exposons pas - corps en streaming,
téléversements multipart, inspection de la politique de redirection,
mises à niveau websocket - appelez `.into_inner()` pour déballer le
`reqwest::Response` sous-jacent :

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` retourne `Err(FrameworkError::internal(...))` quand
appelée sur une réponse fake - il n'y a pas de `reqwest::Response`
sous-jacente dans ce cas. Le plafond de corps de réponse ne
s'applique non plus une fois que vous prenez la réponse brute ; vous
possédez la lecture à partir de là.

Pour les téléversements multipart sortants aujourd'hui, redescendez
directement vers `reqwest::Client` via la même échappatoire. Une
future version pourrait ajouter un builder `.multipart(...)` quand le
motif de demande se dessinera de lui-même.

## Tester avec `Http::fake`

C'est la partie que vous utiliserez tous les jours. `Http::fake`
exécute le corps de votre test à l'intérieur d'une portée
`tokio::task_local!` où chaque appel sortant est intercepté, capturé,
et répond avec ce que vous avez mis en file d'attente.

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### Faire correspondre des réponses prédéfinies

`fake_response(method, url_substring, status, body)` met en file
d'attente une réponse prédéfinie. La première requête sortante dont la
méthode correspond (insensible à la casse) et dont l'URL contient
`url_substring` consomme l'entrée prédéfinie et retourne cette
réponse. Utilisez la méthode `"*"` pour correspondre à n'importe
quelle méthode.

Les requêtes correspondantes suivantes retombent sur l'entrée
prédéfinie suivante de même forme, ou - si aucune ne correspond -
retournent un `200 {}` vide. Mettez en file d'attente une réponse
prédéfinie par appel attendu :

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// Deux GET vers /v1/customer obtiennent des réponses distinctes ; un troisième obtient 200 {}.
```

### Assertions

```rust
// Passe si au moins une requête enregistrée correspond.
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// Passe si aucune requête enregistrée ne correspond.
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` expose `method: String`, `url: String`,
`headers: Vec<(String, String)>`, et `body: Option<Vec<u8>>`. Le
prédicat s'exécute contre chaque requête enregistrée ; les échecs
d'assertion impriment la liste enregistrée avec les valeurs d'en-tête
et les corps caviardés (une petite allowlist de `Content-Type`,
`Accept`, et `User-Agent` est montrée en entier ; tout le reste est
`<redacted>`). Cela garde les bearer tokens et les charges utiles de
webhook hors des journaux CI même quand une assertion explose.

### Les tests s'exécutent en parallèle sans risque

L'état du fake vit dans un `tokio::task_local!` - chaque portée de
fake est cantonnée à la tâche qui exécute le test, pas au processus.
Deux tests s'exécutant simultanément sur des tâches différentes
obtiennent chacun leur propre vec de requêtes enregistrées et leur
propre file de réponses prédéfinies. Pas de mutex partagé, pas d'ordre
de test, pas de `#[serial]`.

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // La requête du test voisin vers /b est invisible ici.
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## Le piège de la tâche spawnée

`tokio::task_local!` est cantonné à la tâche courante. Le travail qui
passe par `tokio::spawn` atterrit sur une tâche neuve et n'hérite PAS
du fake - par défaut, les appels sortants depuis la future spawnée
atteignent le vrai réseau. Deux helpers traitent ce cas.

### `Http::fail_on_real_calls()` et `FailOnRealCallsGuard`

Bascule un flag global au processus qui transforme tout appel sortant
non apparié en une `FrameworkError::internal(...)` au lieu de le
laisser atteindre le réseau. C'est l'analogue Suprnova du
`Http::preventStrayRequests()` de Laravel - il attrape exactement le
bug que le piège crée.

Utilisez la garde RAII pour que le flag se réinitialise à la fin du
test, même en cas de panique :

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // Tout appel HTTP sortant non faké depuis n'importe où à l'intérieur
    // de ce test - y compris depuis une tâche spawnée par `tokio::spawn`
    // - échoue avec un message nommant l'URL. Aucune E/S réseau ne se
    // produit réellement.
}
```

Les gardes imbriquées se composent correctement : le `Drop` de la
garde intérieure restaure l'état PRÉCÉDENT, pas inconditionnellement
« autorisé ». Donc un helper de test intérieur qui installe sa propre
garde à l'intérieur d'une portée gardée extérieure ne désarme pas la
garde extérieure en sortant.

Le flag est global au processus par conception. Le but est
d'attraper une future spawnée par `tokio::spawn` qui s'échappe
silencieusement d'une portée de fake et contacte un vrai tiers depuis
la CI. Un flag par tâche manquerait ce cas.

### `Http::spawn_with_fake_inheritance(future)`

Quand le code sous test spawne légitimement une tâche - un worker de
file d'attente, un synchroniseur en arrière-plan, une sous-tâche - et
que vous voulez que ses appels sortants passent par le fake du parent,
échangez `tokio::spawn` pour `Http::spawn_with_fake_inheritance` :

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // S'exécute sur une tâche NEUVE, mais l'état de fake du parent
        // est réinstallé dans la portée task-local de cette tâche.
        // L'envoi est intercepté ; la réponse est le 204 ci-dessus.
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // Les requêtes enregistrées depuis l'enfant apparaissent ici -
    // l'Arc<Mutex<FakeState>> est partagé, pas pris en instantané.
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

Si aucune portée de fake n'est active quand vous appelez
`spawn_with_fake_inheritance`, c'est équivalent à `tokio::spawn` -
l'enfant s'exécute sans aucun contexte de fake. Vous pouvez donc
l'utiliser sans condition dans du code qui est parfois testé avec
`Http::fake` et parfois non.

### Ceinture et bretelles dans la configuration de test

Les deux se combinent. Un test qui veut être sûr de façon explicite
les associe :

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // Si une faute de frappe sur l'URL ou la méthode dérive loin du
        // fake, la requête retombe sur la garde, qui échoue avec un
        // message nommant l'URL - au lieu de retourner silencieusement
        // un 200 vide qui cache la discordance.
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

Sans la garde, une URL ou une méthode qui dérive du fake retombe
silencieusement sur un `200 {}` par défaut, et votre test passe malgré
le fait que le code de production appelle un point de terminaison
différent. Avec la garde, vous échouez explicitement à la première
discordance.

## Propagation de trace OpenTelemetry

Quand le framework est construit avec la feature `otel` et qu'un
propagateur W3C TraceContext est installé, chaque requête sortante
`Http::*` injecte `traceparent` (et `tracestate` quand non vide) dans
ses en-têtes - si bien que les services en aval peuvent continuer la
trace. Aucune configuration au site d'appel ; le propagateur lit
`opentelemetry::Context::current()` au moment de l'envoi.

Sans contexte OTel actif, aucun en-tête n'est injecté et les requêtes
sortantes ont exactement la même allure qu'avant. Voir
[Observabilité](observability.md) pour la configuration du
propagateur.

## Pourquoi Suprnova diverge

Trois petites divergences par rapport à la façade `Http::` de Laravel
valent la peine d'être signalées.

**Des fakes task-local plutôt qu'un registre de mocks global au
processus.** Le `Http::fake()` de Laravel modifie un registre global
au processus ; les tests se sérialisent sur lui, ou vous acceptez que
des runners parallèles puissent entrer en course. Le `Http::fake` de
Suprnova utilise `tokio::task_local!` si bien que deux tests sur deux
tâches voient chacun leur propre fake - pas d'ordre de test, pas de
mutex partagé. Le prix est que le travail spawné par `tokio::spawn`
n'hérite pas du fake par défaut, ce qui explique l'existence de
`Http::spawn_with_fake_inheritance` et de `FailOnRealCallsGuard`.
Ensemble, ils vous donnent la même garantie « impossible de toucher la
production par accident » que fournit `Http::preventStrayRequests()`
sous Laravel, avec un cantonnement plus strict.

**Les réessais refusent POST/PATCH par défaut.** Le client HTTP de
Laravel réessaie n'importe quelle méthode par défaut. Le `.retry(...)`
de Suprnova est idempotent uniquement ; les méthodes non idempotentes
ont besoin d'un opt-in explicite via `.retry_non_idempotent(...)`. Le
raisonnement est qu'une réponse 5xx d'un point de terminaison
d'écriture signifie souvent « j'ai commité l'écriture et ensuite la
réponse s'est perdue » - rejouer cela à l'aveugle duplique une charge,
un remboursement, un fan-out. Nous forçons l'appelant à décider : avez-
vous fourni une clé d'idempotence que l'amont honore ? Si oui, faites
entrer POST/PATCH dans les réessais. Si non, acceptez le 5xx.

**`retry_when` ne peut que restreindre, jamais élargir.** Le callback `$when`
de `retry()` de Laravel remplace entièrement la décision « faut-il
réessayer ? », il peut donc réessayer des statuts que le framework ne
toucherait autrement pas (un 404, par exemple). Le `retry_when` de Suprnova
ne peut opposer son veto qu'à un réessai que `.retry(...)` /
`.retry_non_idempotent(...)` avait déjà décidé d'effectuer  -  même raisonnement
que les réessais idempotents seuls par défaut : un prédicat capable de
transformer une réponse 4xx ou non idempotente en réponse réessayée laisserait
une closure d'une ligne dupliquer un effet de bord que les règles par défaut
existent pour empêcher.

## Cas limites et petits caractères

- **`Http::*` est fermé pour la v1.** Nous n'exposons délibérément pas
  le `reqwest::Client` sous-jacent. Pour agrandir la surface, ajoutez
  une méthode à la façade plutôt que de faire appel directement à
  `reqwest` - sauf via l'échappatoire documentée `into_inner()` sur
  une vraie réponse.
- **Le client partagé est construit une fois et vit pour toujours.**
  Construit paresseusement au premier appel à n'importe quel verbe
  `Http::*`, gardé dans un `OnceLock`. La pile TLS rustls et le délai
  d'attente par défaut de 30s sont figés dedans.
- **Les échecs de sérialisation JSON/formulaire échouent
  explicitement.** Un builder `.json(&unserializable)` enregistre
  l'erreur et `send()` la retourne comme
  `FrameworkError::internal(...)`. La requête ne part jamais - nous ne
  dégradons pas vers un corps `null`.
- **Le plafond de réessai de 30s est strict.** Le calcul du backoff
  plafonne à 30 secondes ; l'interprétation de `Retry-After` plafonne
  à 30 secondes ; aucun sommeil de réessai unique ne met une tâche en
  pause plus longtemps.
- **Le plafond global au processus est ponctuel.**
  `Http::set_max_response_bytes` est une écriture sur un atomique
  global au processus - définissez-le une fois à l'amorçage, puis
  redéfinissez-le par requête au besoin. Il n'y a pas d'appel
  « réinitialiser au défaut ».

## Suivant

- [E-mail](mail.md) - e-mail sortant, qui utilise des motifs de
  fake / driver similaires pour les tests
- [Notifications](notifications.md) - les canaux de notification, web
  push compris, partagent tous la même philosophie de fake de test
- [File d'attente](queues.md) - les jobs qui font des appels HTTP
  sortants, plus le motif `spawn_with_fake_inheritance` pour tester
  les workers
- [Tests](testing.md) - `#[suprnova_test]`, `TestContainer`, et le
  reste de la surface de fakes
- [Observabilité](observability.md) - la configuration du propagateur
  OTel qui fait s'allumer l'injection de `traceparent`
