# Délais d'attente de requête

`TimeoutMiddleware` impose une échéance stricte à chaque requête HTTP. Un
handler lent - une requête de base de données bloquée, une API amont qui
ne répond pas, une boucle infinie accidentelle dans un hot path quelconque -
maintiendrait sinon une connexion hyper ouverte jusqu'à ce que le client
abandonne ou que l'OS tue le processus. Le middleware de délai d'attente
plafonne cette attente, détruit le handler en vol, et retourne `503
Service Unavailable` afin que l'opérateur voie l'échec au lieu que
l'application fasse fuiter des connexions silencieusement.

Tournez-vous vers lui quand vous construisez quoi que ce soit qui parle à
l'internet public, quoi que ce soit qui fait du fan-out vers des API
tierces, ou quoi que ce soit où « la base de données pourrait être lente
aujourd'hui » est un mardi réaliste.

```rust
use suprnova::{global_middleware, TimeoutMiddleware};

pub async fn register() {
    // Chaque route HTTP obtient un plafond de 30 secondes.
    global_middleware!(TimeoutMiddleware::default());
}
```

Cette seule ligne donne à toute l'application le même plafond par défaut
que celui que Suprnova utilise pour son délai de connexion à la base de
données - choisissez une fois, appliquez partout. Les surcharges par route
tiennent chacune en une ligne. Le reste de ce chapitre explique exactement
ce que l'échéance borne, ce qu'elle ne borne intentionnellement pas, et
comment elle interagit avec la limite de panique, les réponses en
streaming, et les WebSockets.

## Le middleware

`TimeoutMiddleware` réside à `suprnova::TimeoutMiddleware`. Il expose
trois constructeurs et un accesseur :

```rust
use std::time::Duration;
use suprnova::TimeoutMiddleware;

let default_30s = TimeoutMiddleware::default();
let custom      = TimeoutMiddleware::new(Duration::from_millis(2_500));
let whole_secs  = TimeoutMiddleware::seconds(5);

assert_eq!(default_30s.duration(), Duration::from_secs(30));
assert_eq!(custom.duration(),      Duration::from_millis(2_500));
assert_eq!(whole_secs.duration(),  Duration::from_secs(5));
```

`TimeoutMiddleware::default()` utilise une échéance de 30 secondes. Ce
nombre n'est pas arbitraire - il correspond à `DB_CONNECT_TIMEOUT` (30s
également), si bien qu'une requête bloquée en attendant une toute nouvelle
connexion à la base de données et une requête bloquée à l'intérieur du
handler partagent un seul plafond. Si vous augmentez l'un, augmentez
l'autre.

`TimeoutMiddleware::seconds(n)` est un raccourci pour le cas courant des
secondes entières. `TimeoutMiddleware::new(Duration::…)` est
l'échappatoire quand vous avez besoin d'une précision à la milliseconde
(un contrôle de santé interne qui ne devrait jamais prendre plus de 200 ms
; une sonde synthétique avec un budget de 50 ms).

## Installation globale

Un délai d'attente global est le bon point de départ : il donne à chaque
route un plafond sans que personne n'ait à se souvenir de l'ajouter.
Installez-le dans `bootstrap.rs` à côté de votre autre middleware global :

```rust
// src/bootstrap.rs
use suprnova::{
    global_middleware, CorsConfig, CorsMiddleware, DB, RequestIdMiddleware, TimeoutMiddleware,
};
use crate::middleware::LoggingMiddleware;

pub async fn register() {
    DB::init().await.expect("database connect");

    // L'ordre d'exécution compte : request-id en premier (pour que les journaux
    // du délai d'attente le portent), puis journalisation (pour que les requêtes
    // lentes soient quand même observées), puis le délai d'attente lui-même.
    global_middleware!(RequestIdMiddleware);
    global_middleware!(LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());

    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]),
    ));
}
```

L'ordre compte parce que le middleware global enveloppe le reste de la
chaîne dans l'ordre d'enregistrement : `RequestIdMiddleware` s'exécute en
premier à l'entrée et en dernier à la sortie, si bien que l'id de requête
est dans la portée pendant que le délai d'attente déclenche son `503`.
Placer le délai d'attente avant la journalisation cacherait du journal
d'accès les requêtes lentes qui ont fini par se terminer.

## Resserrer par route

Un plafond global de 30 secondes est généreux à dessein - il est là pour
attraper les handlers hors de contrôle, pas pour appliquer des SLA. Quand
un point de terminaison spécifique devrait échouer plus vite, attachez-lui
un délai d'attente par route :

```rust
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    // Point de terminaison de rapport public : doit répondre en 5s ou nous préférons
    // renvoyer 503 et laisser le client réessayer plutôt que de bloquer.
    .get("/report", controllers::report::show)
    .middleware(TimeoutMiddleware::seconds(5));
```

Vous pouvez aussi attacher un délai d'attente plus strict à un groupe de
routes. C'est la forme typique pour une API publique où chaque requête
devrait être rapide, tandis que le reste de l'application garde le défaut
de 30 secondes :

```rust
use suprnova::Router;
use suprnova::TimeoutMiddleware;

Router::new()
    .group("/api", |r| {
        r.get("/users",       controllers::api::users::index)
         .post("/users",      controllers::api::users::create)
         .get("/users/{id}",  controllers::api::users::show)
    })
    .middleware(TimeoutMiddleware::seconds(3));
```

### Le global est un plafond ; le par-route ne peut que resserrer

Le middleware global s'exécute **à l'extérieur** du middleware de route.
La chaîne s'enveloppe de l'intérieur vers l'extérieur :

```
Délai global (30s) → Délai de route (3s) → handler
```

Les deux futures `tokio::time::timeout` sont armés ; le plus intérieur se
déclenche en premier parce qu'il a l'échéance la plus courte. Donc un
délai d'attente par route ne peut que rendre une route *plus stricte* que
le global, jamais plus permissive.

Si un point de terminaison a légitimement besoin de s'exécuter *plus
longtemps* que le défaut global - un rapport lent, un gros upload, un
repli en long-polling - vous avez deux options :

1. Augmentez la valeur globale. Le plus simple, mais cela relâche le plafond pour toutes les autres routes aussi.
2. Cantonnez le middleware global à un groupe de routes qui *exclut* le point de terminaison long, et attachez un délai d'attente séparé (ou aucun) à la route lente. Cela conserve le défaut strict partout ailleurs.

La seconde option est la bonne forme pour un cas isolé ; la première est
la bonne quand toute une catégorie de travail a besoin de plus de marge.

## Ce que l'échéance borne réellement

L'échéance entre en course avec le future retourné par `next(request)`. Ce
future se résout au moment où votre handler retourne sa `HttpResponse` -
pas quand le corps termine son streaming. Cette distinction est
structurelle :

- **Les handlers normaux** construisent tout leur corps avant de retourner, si bien que l'échéance borne effectivement le temps total du handler. Un handler qui sérialise une liste JSON, rend une page Inertia, ou assemble une réponse HTML retient le future jusqu'à ce que le travail soit terminé.
- **Les réponses en streaming** (`HttpResponse::sse(...)`, `HttpResponse::stream_bytes(...)`) retournent *immédiatement* avec un corps paresseux. La chaîne de middleware s'est déjà terminée au moment où hyper commence à tirer des octets du flux, si bien que l'échéance n'observe jamais la durée de vie du corps. Un flux d'événements SSE peut rester ouvert pendant des heures sous un délai d'attente de 30 secondes, par conception - voir [Événements serveur](sse.md) pour le modèle de streaming.
- **Les mises à niveau WebSocket** sont explicitement ignorées. Voir la section suivante.

C'est le comportement que vous voulez presque certainement. Si vous
enveloppiez un flux SSE de longue durée dans un délai d'attente de 30
secondes, le framework démolirait la connexion en plein milieu du flux
toutes les 30 secondes et la fonctionnalité serait inutilisable.

## Exception WebSocket

Le middleware inspecte la requête avant d'armer l'échéance :

```rust
if is_websocket_upgrade(request.headers()) {
    return next(request).await;
}
```

Toute requête portant `Upgrade: websocket` échappe entièrement au délai
d'attente. La vérification est insensible à la casse sur la valeur du
token (`WebSocket`, `websocket`, `WEBSOCKET` correspondent tous), et un
simple `Connection: upgrade` sans `Upgrade: websocket` n'est *pas* traité
comme une mise à niveau WS - cela passe par le délai d'attente
normalement.

Aujourd'hui, les mises à niveau WebSocket prennent un chemin de serveur
séparé qui ne fait tourner aucun middleware global, donc ce garde-fou est
une défense en profondeur - elle empêche le délai d'attente de jamais
borner un canal bidirectionnel de longue durée le jour où cela changera.
Voir [WebSockets](websockets.md) pour la façon dont les mises à niveau
sont dispatchées et la durée de vie d'un socket connecté.

## Ce qui se passe à l'échéance

Quand `tokio::time::timeout` s'écoule avant que le handler ne se termine,
le middleware fait trois choses, dans l'ordre :

1. **Détruit le future du handler en vol.** Le future était en cours de polling à l'intérieur du combinateur `timeout` ; le combinateur retourne `Err(Elapsed)` et le future est détruit à l'endroit où il était suspendu en dernier.
2. **Journalise un avertissement** avec le chemin de la route et la durée du délai d'attente en millisecondes :

   ```
   WARN suprnova::timeout request exceeded its timeout; returning 503 Service Unavailable
       route=/report timeout_ms=5000
   ```

   Le journal est au niveau `WARN`, si bien qu'il apparaît par défaut dans les tableaux de bord des opérateurs, séparément des journaux d'accès `INFO` des requêtes normales.
3. **Retourne `503 Service Unavailable`** avec un corps en texte brut :

   ```
   HTTP/1.1 503 Service Unavailable
   Content-Type: text/plain
   Content-Length: 42

   Service Unavailable: request timed out
   ```

Le 503 est enveloppé dans `Err(HttpResponse::…)`, si bien qu'il
court-circuite le reste de la chaîne exactement comme n'importe quelle
autre requête rejetée par un middleware. Le middleware extérieur
(journalisation, request-id, CORS) exécute quand même son côté
post-handler, si bien que la réponse part avec les bons en-têtes.

### Pourquoi 503 et pas 504

`504 Gateway Timeout` est le bon code quand *vous* êtes la passerelle et
qu'un service *amont* a expiré. `503 Service Unavailable` est le bon code
quand *ce* service n'a pas pu produire la réponse à temps. Le middleware
de délai d'attente borne *notre propre* handler, donc il retourne 503. Si
vous voulez une forme différente - un corps JSON, un statut différent, un
code lisible par une machine - enveloppez votre propre middleware
extérieur autour du délai d'attente et traduisez sa réponse 503.

## Sécurité d'annulation

Quand l'échéance s'écoule, le future du handler est **détruit** à son
point `.await` courant. C'est une annulation Tokio normale ; la même chose
se produit quand un client ferme la connexion en plein milieu d'une
requête. Tout ce qui est détenu à travers la frontière de l'await est
libéré par son impl `Drop` :

- **Les transactions de base de données** font un rollback. Une `DatabaseTransaction` SeaORM a un impl `Drop` qui émet un `ROLLBACK` sur la connexion sous-jacente.
- **Les gardes Mutex et RwLock** se libèrent. Une garde de la bibliothèque standard ou de `parking_lot` se libère au drop ; un autre thread en attente peut la prendre immédiatement.
- **Les descripteurs de fichiers** se ferment. Le descripteur au niveau de l'OS est libéré quand le `tokio::fs::File` est détruit.
- **Les connexions réseau** retournent au pool ou se ferment, selon le comportement au drop du pool.

Le résultat est qu'un handler qui a expiré ne laisse rien pendre -
l'opérateur voit le 503, la base de données voit le rollback, la requête
suivante voit un pool propre.

### Ce qui n'est *pas* annulé

Tout ce que vous avez déplacé hors de la requête avec `tokio::spawn` est
**détaché**. Les tâches spawnées vivent sur le runtime, pas sur le future
de la requête, si bien que détruire la requête ne les arrête pas. Cela
compte quand vous avez écrit quelque chose comme ceci :

```rust
pub async fn webhook(req: Request) -> Response {
    let payload: WebhookPayload = req.json().await?;

    // Travail en arrière-plan « fire-and-forget ». Survit à l'expiration de la requête.
    tokio::spawn(async move {
        if let Err(e) = process_webhook(payload).await {
            tracing::error!("webhook processing failed: {e}");
        }
    });

    Ok(HttpResponse::new().status(204))
}
```

Si la requête expire *avant* que la ligne `spawn` ne s'exécute, le spawn
n'a jamais lieu. Si la requête expire *après* le spawn, la tâche en
arrière-plan continue de s'exécuter - elle n'est pas annulée avec la
requête. C'est presque toujours ce que vous voulez pour un travail de
type webhook, mais cela signifie que le nettoyage après un long `.await`
à l'intérieur du handler n'est **pas** garanti de s'exécuter :

```rust
pub async fn upload(req: Request) -> Response {
    let temp_path = save_to_temp(&req).await?;

    // Si c'est ce qui expire, le nettoyage ci-dessous NE S'EXÉCUTE PAS.
    let processed = long_running_processing(&temp_path).await?;

    // Pas garanti sous un délai d'attente.
    tokio::fs::remove_file(&temp_path).await?;

    Ok(HttpResponse::json(serde_json::to_value(&processed)?))
}
```

La correction consiste à utiliser RAII. Enveloppez le fichier temporaire
dans une struct dont l'impl `Drop` le supprime ; alors le nettoyage
s'exécute que le handler retourne, retourne une erreur, ou soit détruit en
plein `.await` par le délai d'attente. C'est la même discipline que vous
appliqueriez pour n'importe quelle source d'annulation - déconnexion du
client, arrêt du runtime, récupération de panique.

## Interaction avec la limite de panique

Le serveur Suprnova enveloppe toute la chaîne de middleware dans
[`execute_chain_safely`](lifecycle.md), qui utilise
`AssertUnwindSafe(...).catch_unwind()` pour traduire les paniques en un
`500 Internal Server Error` assaini. Une requête qui a expiré n'est
**pas** une panique - le future est détruit proprement - donc le `503` du
délai d'attente part sans impliquer la limite de panique du tout.

Les deux limites gèrent des modes de défaillance différents :

| Défaillance | Limite | Statut | Corps |
|---|---|---|---|
| `.await` du handler dépasse l'échéance | `TimeoutMiddleware` | `503` | `Service Unavailable: request timed out` |
| Le handler panique (`.unwrap()` sur `None`, etc.) | `execute_chain_safely` | `500` | `{"message": "Internal Server Error"}` |
| Le handler retourne `Err(HttpResponse)` | flux `Response` normal | ce que le handler a défini | ce que le handler a défini |

Vous n'avez pas à choisir - les deux limites sont toujours installées. Un
handler qui panique *après* avoir dépassé son délai d'attente produit
quand même un 503 (le future a été détruit avant que la panique ne puisse
se produire). Un handler qui panique *avant* de dépasser son délai
d'attente produit un 500.

## Réglage opérationnel

Trois considérations pour choisir des valeurs de délai d'attente :

1. **Alignez-vous sur votre délai de connexion à la base de données.** Si `DB_CONNECT_TIMEOUT=30` (le défaut), un délai d'attente de requête plus court que 30s se déclenchera avant même qu'une connexion lente ne se termine - l'utilisateur voit un `503` au lieu d'avoir une chance de récupérer. Soit vous augmentez le délai de connexion, soit vous acceptez que « 30s » est le plancher.
2. **Tenez compte du handler légitime le plus lent.** Regardez un histogramme des durées de requête de niveau `INFO`. Le p99 de la queue lente devrait se situer confortablement sous le délai d'attente, avec de la marge pour la dérive d'horloge et le jitter de la boucle d'événements. Un délai d'attente qui se déclenche systématiquement sur du trafic sain est une mauvaise configuration, pas une fonctionnalité.
3. **Les délais d'attente par route sont de l'observabilité.** Resserrer `TimeoutMiddleware::seconds(3)` sur `/api/*` transforme une API dégradée en une alerte visible (journaux pleins de WARN, 503 dans le load balancer) plutôt qu'un problème de latence qui s'installe insidieusement. Utilisez-les là où vous avez un SLA et voulez un échec net quand vous le manquez.

Les propres tests d'intégration du framework utilisent des durées dans la
plage des millisecondes
(`TimeoutMiddleware::new(Duration::from_millis(50))`) pour exercer
l'échéance de façon déterministe. Les échéances en production sont presque
toujours en secondes entières.

### Pourquoi Suprnova diverge

Dans un déploiement Laravel + PHP-FPM, les délais d'attente de requête
vivent en dehors de l'application : le `proxy_read_timeout` de nginx, le
`request_terminate_timeout` de PHP-FPM, le délai d'inactivité du load
balancer. Le processus PHP est tué quand le budget est épuisé, et tout
état ouvert - connexions à la base de données, descripteurs de fichiers -
fuit jusqu'à ce que la requête suivante réutilise le worker.

Suprnova borne la requête à l'intérieur de l'application parce qu'il le
peut. Le handler est un future Tokio, pas un processus PHP, donc le
détruire exécute les impls `Drop` proprement : les transactions font un
rollback, les verrous se libèrent, les descripteurs se ferment, le pool de
connexions reste sain. Le 503 part aussi *comme une vraie réponse HTTP* -
les clients voient un code de statut correct au lieu d'une
réinitialisation en amont.

C'est aussi pourquoi le middleware n'essaie pas d'être une couche
`Timeout` de Tower. La couche de Tower est générique sur n'importe quel
service Tokio et retourne `tower::timeout::error::Elapsed`, que les
appelants doivent ensuite mapper vers un statut HTTP. Le middleware
Suprnova sait qu'il enveloppe un pipeline de requête HTTP ; il retourne
directement `503`, journalise la route fautive, et respecte les exceptions
WebSocket et streaming du framework sans que l'appelant n'ait à en tenir
compte. La couche de Tower est la bonne primitive pour un service Tokio
générique ; pour une requête HTTP, c'est la bonne forme.

## Suivant

- [Middleware](middleware.md) - le trait, la chaîne, l'enregistrement global vs par route, les hooks terminables
- [Cycle de vie des requêtes](lifecycle.md) - où le délai d'attente se situe dans la chaîne, et comment `execute_chain_safely` gère les paniques
- [Événements serveur](sse.md) - le modèle de réponse en streaming que le délai d'attente ne borne pas intentionnellement
- [WebSockets](websockets.md) - le chemin de mise à niveau qui contourne entièrement le délai d'attente
- [Gestion des erreurs](errors.md) - comment les réponses 5xx sont dispatchées comme des événements `ErrorOccurred` pour l'observabilité
