# Événements serveur

Les événements envoyés par le serveur (SSE, *Server-Sent Events*) sont
le canal de push unidirectionnel minimal du serveur vers le
navigateur : le navigateur ouvre `EventSource(url)`, le serveur
maintient ouverte une réponse `text/event-stream`, et pousse des
événements encadrés au fur et à mesure qu'ils se produisent. Pas de
handshake WebSocket, pas de permessage-deflate, pas de bibliothèque de
framing - juste des lignes `data:`, `event:`, `id:`, `retry:`
terminées par une ligne vide, selon la spécification
[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html).

La primitive SSE de Suprnova se branche sur le chemin de corps en
streaming : construisez un `Stream<Item = SseEvent>`, remettez-le à
`HttpResponse::sse(...)`, et le framework gère la connexion, le
framing, les en-têtes et l'isolation des paniques. La connexion reste
ouverte jusqu'à ce que le flux producteur se termine ou que le client
se déconnecte.

## Quand utiliser SSE plutôt que les WebSockets

| Propriété | SSE | WebSockets |
|----------|-----|------------|
| Direction | Serveur → navigateur | Bidirectionnelle |
| Transport | HTTP/1.1 ou HTTP/2 simple | Upgrade uniquement |
| Reconnexion | Automatique, avec `retry:` et `Last-Event-ID` | Manuelle |
| Proxys / CDN | Fonctionne à travers tout ce qui autorise les réponses HTTP longues | A besoin d'un support explicite d'Upgrade dans la plupart des cas |
| API navigateur | `EventSource` (native) | `WebSocket` (native) |
| Frames binaires | Texte uniquement (UTF-8) | Texte ou binaire |
| Plafond de connexions par onglet | 6 (HTTP/1.1) / illimité (HTTP/2) | Illimité |

Tournez-vous vers SSE quand vous avez seulement besoin de push
serveur-vers-client (fils d'activité, notifications, suivi de logs,
streaming IA). Tournez-vous vers les [WebSockets](websockets.md) quand
vous avez besoin de trafic bidirectionnel ou de frames binaires.

## Démarrage rapide

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // client déconnecté
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

Sortie réseau pour un tick :

```text
event: tick
id: 0
data: tick 0

```

Le navigateur analyse ceci et déclenche un événement `tick` avec
`evt.data === "tick 0"` et `evt.lastEventId === "0"`.

## L'API `SseEvent`

`SseEvent` est le type que vous poussez sur le flux. Il a deux
formes :

* **Frame** - un événement normal avec `event` / `id` / `retry`
  facultatifs et un payload `data` multi-lignes. Construit via
  [`SseEvent::data`](#constructeurs), `SseEvent::json`, ou
  `SseEvent::error`.
* **Comment** - un keep-alive visible seulement sur le réseau (`:\n\n`
  ou `: <text>\n\n`). Construit via `SseEvent::comment(text)` ou
  `SseEvent::keep_alive()`. Le navigateur ignore les commentaires par
  spec ; les octets qui traversent la connexion sont ce qui empêche
  les proxys et les load balancers inactifs de la fermer.

### Constructeurs

| Constructeur | Produit | Usage |
|-------------|----------|-----|
| `SseEvent::data(text)` | Frame avec seulement des lignes `data:` | L'événement minimal |
| `SseEvent::json(event, &payload)` | Frame avec `event:` + `data:` en JSON | Le cas à 95 % - `JSON.parse(evt.data)` côté client |
| `SseEvent::error(message)` | Frame avec `event: error` | Événement d'erreur de niveau domaine, distinct de l'`error` de niveau connexion que le navigateur déclenche en cas d'échec de transport |
| `SseEvent::comment(text)` | Comment | Keep-alive avec un marqueur que l'opérateur peut repérer dans les journaux |
| `SseEvent::keep_alive()` | Comment vide (`:\n\n`) | Battement de cœur canonique à octets minimaux |

### Builders

| Builder | Effet | Sur `Comment` |
|---------|--------|--------------|
| `.with_event(name)` | Définit le champ `event:` | No-op silencieux |
| `.with_id(id)` | Définit le champ `id:` - requis pour la sémantique de reprise | No-op silencieux |
| `.with_retry(Duration)` | Définit le champ `retry:` (ms) ; la spec dit que `Duration::ZERO` signifie « reconnecter immédiatement » | No-op silencieux |
| `.try_with_event(name)` | Variante faillible - voir [Contrat de sécurité](#contrat-de-sécurité) | `Ok(self)` inchangé |
| `.try_with_id(id)` | Variante faillible de `with_id` | `Ok(self)` inchangé |

Les builders sur `Comment` sont des no-op à dessein - le format réseau
n'a aucun moyen d'exprimer « commentaire avec un nom d'événement ».
Une mauvaise utilisation reste silencieuse plutôt que de convertir
l'événement en frame et de surprendre le producteur.

### Accesseurs

| Méthode | Retourne |
|--------|---------|
| `.event()` | `Option<&str>` - le nom de l'événement, si défini |
| `.id()` | `Option<&str>` - le dernier id d'événement, si défini |
| `.retry()` | `Option<Duration>` - le délai de reconnexion, si défini |
| `.payload()` | `&str` - le payload `data:` (ou `""` pour `Comment`) |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - le texte du commentaire, si c'est un `Comment` |

### Encodage réseau

`SseEvent::to_wire()` sérialise l'événement en `Bytes` prêts pour le
flux du corps :

**Frame :**

```text
event: <event>\n   (seulement si Some)
id: <id>\n         (seulement si Some)
retry: <ms>\n      (seulement si Some)
data: <line>\n     (une par ligne du payload, après normalisation \r/\r\n)
\n                 (terminateur - requis par la spec)
```

**Comment :**

```text
: <line>\n         (une par ligne du texte du commentaire ; `:\n` pour les lignes vides)
\n                 (frontière de flush)
```

## Contrat de sécurité

Le format réseau SSE utilise CR / LF / NUL comme terminateurs de champ
sans mécanisme d'échappement. Un producteur qui laisse une entrée
utilisateur atteindre `event:` ou `id:` sans l'assainir exposerait une
vulnérabilité d'injection de champ - une valeur `"legit\ndata:
injected"` produirait deux champs `data:` sur le réseau, et
`"legit\n\nevent: spoofed"` terminerait l'événement en cours et en
démarrerait un nouveau.

Le `to_wire()` de Suprnova se défend sur deux niveaux :

* **Les valeurs des champs `event:` et `id:`** - chaque CR / LF / NUL
  est retiré au moment de la sérialisation. Un `WARN` structuré se
  déclenche à chaque retrait : `target: "suprnova::sse"`, `field =
  "event"|"id"`. Le warn ne journalise jamais la valeur - ces octets
  sont contrôlés par l'attaquant par construction.
* **Le texte `data:` et le texte de commentaire** - `\r\n` et les `\r`
  isolés sont normalisés en `\n` avant la découpe en lignes, si bien
  qu'un producteur qui intègre un `\r` dans un payload ne peut pas
  faire synthétiser au parseur du récepteur un champ `data:` /
  `event:` / `id:` au moment du parsing. NUL est retiré du texte de
  commentaire avec un `WARN` correspondant.

Si vous voulez **échouer vite** sur une mauvaise entrée plutôt que de
la retirer silencieusement, utilisez les homologues `try_with_*` :

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // retourne Err sur CR/LF/NUL
    .try_with_id(&user_supplied_id)?;
```

Le `FrameworkError::validation(field, ...)` retourné nomme le champ ;
il ne répète PAS la valeur en écho, si bien qu'un 400 remonté au
client est sûr à journaliser.

## Keep-alive et délais d'inactivité des proxys

Les connexions SSE de longue durée sont silencieuses par défaut. La
plupart des déploiements en production se trouvent derrière un proxy /
load balancer / CDN qui ferme les connexions inactives pour libérer
des ressources :

* nginx par défaut : 60 secondes
* AWS ALB par défaut : 60 secondes
* Cloudflare par défaut : 100 secondes

Un commentaire `keep_alive()` toutes les 15 à 30 secondes maintient la
connexion active à travers tout cela sans dispatcher d'événement
`message` au navigateur. La forme à octets minimaux (`:\n\n`) suffit à
vider les buffers d'écriture des proxys sans envoyer de payload.

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// Tâche de heartbeat - indépendante du producteur d'événements.
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // client parti
        }
    }
});

// Producteur d'événements ... envoie des frames dans `tx` au fur et à
// mesure qu'elles se produisent.
```

## Reprise après coupure (`Last-Event-ID`)

Quand l'`EventSource` du navigateur perd la connexion, il se
reconnecte automatiquement et envoie le dernier `id:` qu'il a vu comme
en-tête `Last-Event-ID` sur la nouvelle requête. Marquez chaque
événement avec `.with_id(...)` et lisez l'en-tête sur la requête de
reprise :

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Construit le flux producteur à partir de `resume_from + 1`. La
    // closure possède son propre compteur courant, si bien que la
    // mutation reste à l'intérieur du flux.
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` retourne `None`
quand l'en-tête est absent OU quand la valeur contient un octet NUL
(selon la spec WHATWG, NUL invalide un last-event-id et le parseur du
navigateur le rejetterait). La `String` retournée est par ailleurs une
entrée utilisateur opaque - parsez-la comme votre propre curseur /
séquence / offset avant de l'utiliser.

## Erreurs de niveau domaine

`SseEvent::error("...")` produit la forme conventionnelle `event:
error\ndata: <msg>\n\n`. Les abonnés peuvent écouter cela séparément
de l'`error` de niveau connexion que le navigateur déclenche en cas
d'échec de transport :

```js
const es = new EventSource("/stream");

// Erreurs de connexion / de transport (pas de `data`).
es.onerror = (evt) => console.warn("transport error", evt);

// Erreurs de niveau domaine émises par SseEvent::error(...).
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

Quand vous mappez un `Stream<Item = Result<T, E>>` vers un
`Stream<Item = SseEvent>`, le motif idiomatique est `map(|r| match r {
Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` - le
mapping d'erreur côté consommateur reste entre les mains du
producteur, et le framework n'a jamais à inventer une forme par
défaut.

## Diffusion d'un flux vers plusieurs abonnés

Le fan-out vers de nombreux abonnés SSE est déjà couvert par le
[sous-système de diffusion](broadcasting.md) : abonnez-vous à un canal
`BroadcastHub` et adaptez le `broadcast::Receiver` en flux `SseEvent`
avec `tokio_stream::wrappers::BroadcastStream` + `.map(...)`. Chaque
connexion obtient son propre receiver ; le hub gère la politique de
consommateur lent (erreurs `Lagged(n)` quand un abonné prend du
retard) et vous décidez comment remonter cela au client.

L'exemple dogfood qui fonctionne, dans
`app/src/controllers/sse_example.rs`, implémente ceci en ~25 lignes :

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

L'événement `lagged` permet au client de déclencher un refetch complet
et de reprendre - la connexion reste ouverte à travers le retard.

## Configuration de production

### En-têtes de réponse

`HttpResponse::sse(...)` définit les en-têtes requis pour vous :

| En-tête | Valeur | Pourquoi |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | Défini par la spec ; l'`EventSource` du navigateur l'exige |
| `Cache-Control` | `no-cache` | Empêche les intermédiaires de mettre le flux en cache |
| `Connection` | `keep-alive` | Réponse HTTP/1.1 de longue durée |
| `X-Accel-Buffering` | `no` | Désactive le buffering du proxy nginx - les événements sont flushés immédiatement. No-op hors nginx |

### Réglage de la reconnexion

Le délai de reconnexion par défaut du navigateur est de 3 secondes.
Envoyez un champ `retry:` une fois au début du flux pour le
surcharger :

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

`Duration::ZERO` est valide selon la spec (« reconnecter
immédiatement ») et est émis tel quel - pas de coercition. Pour des
flux en production, un retry de 5 à 15 secondes équilibre récupération
rapide et absence de marteau sur le serveur pendant une panne
régionale.

### Pourquoi Suprnova diverge

Laravel livre SSE comme un helper ponctuel sur `Response` :
`Response::eventStream(fn () => ...)` prend une closure qui yield d'un
générateur et encadre chaque valeur yieldée comme une ligne `data:`.
Il ne modélise pas `event:` / `id:` / `retry:` comme des champs de
premier ordre, n'a pas de primitive de keep-alive intégrée, et
n'assainit pas les valeurs qui injecteraient des champs
supplémentaires sur le réseau.

Suprnova traite SSE comme un vrai sous-système plutôt que comme un
helper ponctuel :

- `SseEvent` est une valeur typée avec des builders faillibles
  (`try_with_*`) et infaillibles (`with_*`), des formes `Frame` et
  `Comment` distinctes, et un contrat d'assainissement documenté sur
  chaque champ mono-ligne.
- `HttpResponse::sse(stream)` se branche sur le même pipeline de corps
  `stream_bytes` utilisé par n'importe quelle autre réponse de longue
  durée, si bien que SSE partage une seule annulation, les mêmes
  en-têtes, et le même chemin d'isolation des paniques que le reste du
  framework.
- Les producteurs composent n'importe quel `Stream<Item = SseEvent>` -
  `tokio::sync::mpsc`, `tokio::sync::broadcast`,
  `futures::stream::iter`, ou l'adaptateur de fan-out
  [BroadcastHub](broadcasting.md). Aucun d'eux ne nécessite
  d'échappatoire du framework.
- Un lecteur de `Last-Event-ID` (`sse::last_event_id`) et la règle
  WHATWG de rejet des NUL sont dans la boîte, si bien que la reprise
  après coupure est à un appel de parsing plutôt qu'à un utilitaire
  d'en-tête personnalisé par application.

## Référence

| Symbole | Objet |
|--------|-------|
| `suprnova::sse::SseEvent` | Une pièce émettable d'un flux SSE. Deux formes - `Frame` (événement avec `event` / `id` / `retry` facultatifs + `data`) et `Comment` (keep-alive). |
| `SseEvent::data(text)` | Construit une frame avec seulement des lignes `data:`. |
| `SseEvent::json(event, &payload)` | Construit une frame dont le payload est `payload` sérialisé par `serde_json` ; définit `event:` à `event`. Retourne `Result<Self, serde_json::Error>`. |
| `SseEvent::error(message)` | Construit une frame avec `event: error` et le message fourni comme `data`. |
| `SseEvent::comment(text)` | Construit un événement commentaire seul (`: <text>\n\n`). Invisible pour le navigateur ; garde les proxys éveillés. |
| `SseEvent::keep_alive()` | Raccourci pour le commentaire vide `:\n\n`. Battement de cœur à octets minimaux. |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | Builders infaillibles sur une `Frame` ; no-op silencieux sur un `Comment`. Retirent CR / LF / NUL au moment de `to_wire()` avec un `WARN` structuré. |
| `.try_with_event(name)` / `.try_with_id(id)` | Homologues faillibles - retournent `Err(FrameworkError::validation(...))` sur CR / LF / NUL. À utiliser quand la valeur vient d'une entrée utilisateur et que vous voulez un 4xx plutôt qu'un retrait silencieux. |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | Accesseurs. `payload()` est nommé ainsi pour éviter d'entrer en collision avec le constructeur `data`. |
| `SseEvent::to_wire()` | Sérialise en `Bytes` au format réseau SSE. Public afin que les tests et adaptateurs puissent encoder sans passer par le builder de réponse. |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | Lit l'en-tête `Last-Event-ID`. Retourne `None` quand il est absent OU quand la valeur contient un octet NUL (WHATWG rejette les ids invalides). |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | Helper pur exposant le même contrat de validation - testable unitairement sans construire de `Request`. |
| `HttpResponse::sse(stream)` | Construit une réponse en streaming à partir de n'importe quel `Stream<Item = SseEvent> + Send + Sync + 'static`. Définit `Content-Type`, `Cache-Control`, `Connection`, `X-Accel-Buffering`. |

## Suivant

- [WebSockets](websockets.md) - l'autre connexion de longue durée, quand vous avez besoin de bidirectionnel ou de frames binaires.
- [Diffusion](broadcasting.md) - le fan-out `BroadcastHub` partagé avec les abonnés WebSocket.
- [Notifications](notifications.md) - les drivers de canal pour la livraison push non-streaming (mail, base de données, diffusion).
- [Web Push](web-push.md) - notifications poussées par le serveur qui atteignent le client quand aucun `EventSource` n'est ouvert.
- [Réponses](responses.md) - le reste de la surface du builder `HttpResponse`.
