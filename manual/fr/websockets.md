# WebSockets

Les routes WebSocket de Suprnova se trouvent aux côtés des routes HTTP
dans le même routeur. Vous enregistrez un chemin et un handler ; le
framework détecte la requête `Upgrade: websocket` sur ce chemin,
exécute la même chaîne de middleware qu'un GET HTTP sur ce chemin
exécuterait, termine le handshake RFC 6455, et appelle votre handler
avec un `WsSocket` typé plus la `Request` d'origine. Il n'y a pas de
serveur WebSocket séparé - les connexions sont mises à niveau depuis
le même listener TCP hyper qui sert votre trafic HTTP. Le framework
traque aussi chaque handler spawné dans un `JoinSet` par serveur, si
bien qu'un arrêt gracieux vide les connexions en vol avant que le
listener TCP ne se termine.

## Démarrage rapide

Ajoutez un `EchoHandler` et enregistrez-le dans `routes!`.

`src/ws/echo.rs` :

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs` (à l'intérieur de `routes! { ... }`) :

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

Démarrez l'application et connectez-vous avec `wscat` :

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

Quand `recv_text()` retourne `Ok(None)`, le pair a fermé la connexion ;
la boucle se termine, le handler retourne `Ok(())`, et le framework
envoie une frame Close(1000) propre.

## Cycle de vie d'une mise à niveau

Un handshake WebSocket est un GET HTTP avec `Upgrade: websocket`. Le
framework exécute le pipeline de requête complet dessus avant que la
moindre frame ne circule :

1. **Correspondance de route.** Le routeur recherche le chemin dans la
   table de routes WS ; en cas d'absence de correspondance, la requête
   retombe sur le fallback HTTP.
2. **Politique d'origine.** La [`OriginPolicy`](#politique-d-origine)
   configurée est appliquée. Une violation retourne un HTTP 403 sans
   mise à niveau.
3. **Négociation de sous-protocole.** Si la route a des
   `accepted_protocols`, le premier token offert par le client qui
   recoupe est renvoyé en écho sur la réponse 101.
4. **Chaîne de middleware.** `RequestIdMiddleware` s'exécute le plus à
   l'extérieur, suivi de tout middleware enregistré globalement, suivi
   du middleware par route de la route. Une réponse non-2xx de
   n'importe quel middleware court-circuite la mise à niveau - le pair
   reçoit l'erreur HTTP, et le futur WebSocket est détruit proprement.
5. **Handshake.** `hyper_tungstenite::upgrade` produit le futur qui se
   résout en un `WebSocketStream`.
6. **Dispatch du handler.** La `Request` (possiblement réécrite par le
   middleware) et un `WsSocket` fraîchement construit sont remis à
   `WebSocketHandler::handle`.
7. **Battement de cœur + handler.** Le framework spawne une tâche de
   battement de cœur par connexion et attend le futur du handler sous
   un span de tracing `ws.connection` portant l'id de requête.
8. **Handshake de fermeture.** Sur `Ok(())`, le framework envoie
   Close(1000) ; sur `Err(_)`, il envoie Close(1011 "internal error").
   Le forwarder est attendu afin que la frame de fermeture soit
   flushée sur le réseau avant que la tâche suivie de la connexion ne
   soit signalée comme terminée.

La sémantique de la valeur de retour est inversée par rapport à HTTP :
il n'y a pas de corps. `Ok(())` signifie une déconnexion propre ;
`Err(_)` est journalisé et le pair voit Close(1011). Dans les deux
cas, la connexion se termine.

## L'API `WsSocket`

`WsSocket` est le handle bidirectionnel que le framework passe à votre
handler. En interne, le flux tungstenite sous-jacent est scindé en
deux moitiés Sink + Stream : une tâche forwarder possède le sink et
vide un mpsc ; les méthodes d'envoi exposées au handler mettent en
file d'attente sur le mpsc. Le handler lit directement depuis la
moitié stream. Cette scission signifie que le framework peut aussi
pousser des frames (pings de battement de cœur, fan-out du diffuseur)
sans entrer en concurrence avec le chemin d'envoi du handler.

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

Met en file d'attente une frame texte UTF-8. Retourne `Err` seulement
quand la connexion est déjà fermée.

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

Met en file d'attente une frame binaire. Accepte tout ce qui
implémente `Into<Vec<u8>>`. Même sémantique d'erreur que `send_text`.

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) signifie que le pair a fermé.
```

Retourne le prochain message texte, en écartant silencieusement les
types de frame dont un handler texte seul n'est pas censé se
soucier :

- `Message::Binary` - payload binaire du pair
- `Message::Ping` - ping initié par le pair (tungstenite gère le pong automatiquement)
- `Message::Pong` - réponse pong du pair à un battement de cœur du framework (le compteur de pings manqués est remis à zéro comme effet de bord)
- `Message::Frame` - variantes de frame brutes provenant de contextes côté serveur ; jamais attendues à ce niveau

Une frame avalée a disparu ; il n'y a aucun moyen rétroactif de la
voir. Si le handler a besoin d'observer les frames binaires ou les
codes de fermeture, utilisez [`recv`](#recv) dès la toute première
lecture.

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

Retourne le prochain message de n'importe quel type, y compris
Binary, Ping, Pong, et Close. `Pong` remet quand même le compteur de
pings manqués à zéro comme effet de bord avant d'être retourné.
`Ok(None)` signifie que le flux sous-jacent s'est terminé.

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

Met en file d'attente une frame de fermeture et retourne. Le
forwarder écrit la frame dans le sink, appelle `close()` sur le sink,
et se termine. Les envois suivants sur le même socket retournent
`Err` parce que le forwarder a disparu. Retournez toujours `Ok(())`
immédiatement après avoir appelé `close`.

`close` valide ses arguments en amont, conformément à la RFC 6455
§7.4 + §5.5.1 :

- `code` doit satisfaire `CloseCode::is_allowed()`. Les codes réservés
  ou invalides (1004, 1005, 1006, 1015, tout ce qui est en dessous de
  1000, tout ce qui est au-dessus de 4999) sont rejetés avec `Err` et
  **aucune frame n'est envoyée** - la connexion reste ouverte et
  l'appelant peut réessayer avec un code valide. Utilisez 1000 pour
  une fermeture normale, 1001-1013 pour les raisons définies,
  3000-3999 pour les codes enregistrés IANA, ou 4000-4999 pour les
  codes privés d'application.
- `reason` est plafonné à 123 octets (la limite de 125 octets pour une
  frame de contrôle, moins les deux octets du code). Les raisons plus
  longues sont rejetées sans rien mettre en file d'attente.

### Pourquoi Suprnova diverge

Les frameworks PHP boulonnent le support WebSocket comme un processus
séparé (ratchet, soketi, pusher). La route WebSocket de Suprnova vit
dans le même `routes! { ... }` que vos routes HTTP, servie par le même
listener TCP hyper, vidée par le même chemin d'arrêt gracieux. Il y a un
seul binaire, une seule config, un seul déploiement. Les connexions de
longue durée sont citoyennes de première classe parce que Tokio les
rend bon marché ; le framework n'a pas à s'en excuser.

## Paramètres de chemin

Les routes WebSocket prennent en charge la même syntaxe de capture
`{param}` que les routes HTTP. Les valeurs capturées sont disponibles
sur la `Request` passée au handler.

```rust
// In routes!:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` retourne `Result<&str, ParamError>` ; le `?` propage
une `FrameworkError::ParamError` si le segment est manquant, ce qui
fait que le handler retourne `Err` et que le framework envoie
Close(1011). En pratique, la capture est toujours présente quand la
route a correspondu - le chemin d'erreur est un filet de sécurité
contre les fautes de frappe dans les noms de paramètres.

Les segments façon Express `:id` sont aussi acceptés
(`ws!("/ws/rooms/:id", h)`) et se convertissent en forme matchit en
interne.

Pour l'API `Request` complète - en-têtes, cookies, chaîne de requête,
adresse du pair - voir [la doc des requêtes](requests.md).

## Middleware par route

Chaînez `.middleware(M)` sur l'entrée `ws!`. Plusieurs middleware se
composent de gauche à droite et s'exécutent dans le même ordre fixe
qu'une requête HTTP sur le même chemin exécuterait :
`RequestIdMiddleware` le plus à l'extérieur, puis tout middleware
enregistré globalement, puis la chaîne par route, puis le handler.

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

Une réponse non-2xx de n'importe quel middleware court-circuite la
mise à niveau. Le pair reçoit le rejet (par ex. 401, 403) avec
`X-Request-Id` défini, le futur WebSocket jamais réveillé est détruit
proprement, et le handler n'est jamais appelé. C'est le bon niveau
pour les vérifications au niveau transport : qui a le droit d'ouvrir
la connexion, d'où elle vient, combien de connexions simultanées par
identité.

Un middleware peut substituer une `Request` modifiée en appelant
`next(modified_req)`. Le terminateur capture ce que la chaîne laisse
finalement passer, et c'est ce que le handler voit comme argument
`Request`. Un middleware qui résout une identité (une recherche de
session, une vérification de token) peut attacher le résultat via les
extensions de `Request` ; le handler le relit de la même façon que les
contrôleurs HTTP.

Les variantes directes sur `Router` (`Router::ws`,
`Router::ws_with_middleware`, `Router::ws_with_config`,
`Router::ws_with_middleware_and_config`) couvrent la même surface pour
le code qui construit un `Router` en dehors de la macro. Chacune a un
homologue faillible `try_*` qui retourne `Err(FrameworkError)` en cas
de motif dupliqué ou malformé plutôt que de paniquer.

### Pourquoi Suprnova diverge

La plupart des écosystèmes soit sautent le middleware sur les mises à
niveau WebSocket (la convention Node), soit imposent une cérémonie
d'enregistrement séparée pour un « middleware WebSocket » (la
convention .NET / Spring). Suprnova traite la mise à niveau comme le
GET HTTP qu'elle est réellement : la même chaîne s'exécute, dans le
même ordre, avec la même sémantique de court-circuit. Il n'y a pas de
second concept à apprendre - `AuthMiddleware`, `RateLimitMiddleware`,
`RequestIdMiddleware`, `CorsMiddleware` fonctionnent sur les routes WS
parce qu'ils fonctionnent sur n'importe quelle route. L'application de
la politique d'origine est la seule ride supplémentaire, et c'est une
propriété de `WsConfig`, pas un middleware séparé.

## Authentification à la connexion

Le handler reçoit la `Request` réécrite par le middleware. Trois
motifs fonctionnent bien, par ordre croissant d'intégration avec le
reste du framework :

**Motif 1 - token bearer en ligne dans le handler.** Le plus simple.
Fonctionne sans aucun middleware d'auth. `wscat`, les clients
navigateur, et les load balancers transmettent tous les en-têtes
proprement.

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**Motif 2 - filtrer la mise à niveau avec un middleware de route.**
Rejette les ouvertures non autorisées avant que la moindre frame ne
circule. Séparation des responsabilités plus nette ; le handler ne
voit que des connexions authentifiées.

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` retourne 401 sur les requêtes non authentifiées ; la
mise à niveau est abandonnée avec la réponse de rejet et le handler
n'est jamais appelé.

**Motif 3 - filtre middleware plus relecture par le handler.** Le
middleware court-circuite les ouvertures non autorisées ; le handler
relit alors le même identifiant (token, cookie, etc.) qu'il sait
désormais présent pour identifier quel utilisateur vient de se
connecter :

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // Le middleware a déjà validé le bearer ; on n'arrive ici que s'il était valide.
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**Motif 4 - laisser le middleware authentifier et lire le résultat.**
Préféré quand un middleware d'auth s'exécute déjà sur la mise à
niveau. L'identité qu'il a résolue est portée sur la requête
elle-même :

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` vient du middleware de session/token, pas de quoi que
    // ce soit envoyé par le client dans une frame.
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

C'est ce qui rend significatif le hook `authorize` d'un canal de
diffusion privé : il reçoit la même `Request`, si bien qu'il peut
filtrer sur une identité dérivée du serveur plutôt que sur une valeur
choisie par le client. Avant que `auth_user_id` n'existe, un canal
n'avait rien de fiable à consulter, et le palliatif évident - «
accepter tout abonné dont la frame d'abonnement porte un token qui a
l'air correct » - n'est pas un filtre du tout.

Les accesseurs thread-local qui fonctionnent dans les contrôleurs
HTTP - `session()`, `Auth::user()`, le sac `Context` par requête - ne
sont toujours **pas** peuplés à l'intérieur d'un handler WebSocket. Les
portées task-local de la chaîne de middleware se démontent quand la
chaîne retourne ; le handler s'exécute dans une tâche fraîchement
spawnée qui n'hérite que de l'id de requête et de l'id d'auth résolu.
Lisez tout le reste dont le handler a besoin directement sur la
`Request` (en-têtes, cookies via `req.cookie("...")`, params capturés,
le token bearer via `req.bearer_token()`) - ceux-là survivent dans la
tâche du handler.

### Pourquoi Suprnova diverge

Laravel autorise les canaux de diffusion via un point de terminaison
HTTP séparé (`/broadcasting/auth`), si bien que le callback du canal
s'exécute dans une requête ordinaire avec la session complète
disponible. Suprnova autorise plutôt in-process pendant la mise à
niveau - une seule connexion, pas de second aller-retour - ce qui
signifie que l'identité doit être transportée explicitement à travers
la frontière du spawn plutôt que d'être recherchée à nouveau.

## `WsConfig`

`WsConfig` contrôle le comportement par connexion. Les défauts visent
les points de terminaison publics, exposés au navigateur - chaque
connexion active réserve un buffer tungstenite dimensionné à
`max_message_size`, si bien que le framework part petit par défaut et
laisse les routes qui ont besoin de plus relever les limites
explicitement.

| Champ                 | Défaut         | Type            | Effet |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | À quelle fréquence le framework envoie une frame Ping pour maintenir la connexion active. |
| `max_message_size`    | 1 Mio          | `usize`         | Taille maximale d'un message réassemblé, en octets. Les messages plus grands sont rejetés par tungstenite. |
| `max_frame_size`      | 64 Kio         | `usize`         | Taille maximale d'une seule frame WebSocket, en octets. |
| `max_missed_pings`    | 2              | `usize`         | Pongs manqués consécutifs avant que le battement de cœur ne ferme la connexion avec le code 1011. `usize::MAX` désactive l'application de la règle. |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | Vérification de l'en-tête Origin appliquée au moment de la mise à niveau. Voir [Politique d'origine](#politique-d-origine). |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | Tokens `Sec-WebSocket-Protocol` acceptés par le serveur. Vide signifie pas de négociation. Voir [Sous-protocoles](#sous-protocoles). |

Surcharges recommandées selon le cas d'usage :

- **Chat / notifications / positions de curseur** - les défauts
  conviennent. Réduisez `ping_interval` à 5-10s si votre LB a un délai
  d'inactivité agressif.
- **Flux internes de confiance** (fan-out serveur-à-serveur, export en
  masse, gros transferts binaires) - partez de `WsConfig::generous()`,
  qui relève `max_message_size` à 64 Mio et `max_frame_size` à 16 Mio
  en gardant les autres défauts.
- **Payload spécifique en surtaille** (une route qui téléverse des
  fichiers audio de 256 Mio) - définissez les champs directement ;
  n'appliquez pas la limite plus large aux routes qui n'en ont pas
  besoin.

La struct de config est constructible via `Default` et chaque champ
est public :

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

Appliquez la surcharge par route, soit sur l'entrée `ws!`, soit sur
`Router::ws_with_config` :

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` est validé à l'enregistrement de la route. Un
`ping_interval` à zéro ou un `max_missed_pings` à zéro corromprait la
tâche de battement de cœur ; les deux sont rejetés à l'amorçage plutôt
que de paniquer à la première connexion.

### Battement de cœur et fermeture en l'absence de pong

Pour chaque connexion mise à niveau, le framework spawne une tâche de
battement de cœur qui envoie un `Ping(b"")` à chaque `ping_interval`. À
chaque tick, le compteur de pings manqués s'incrémente ; à chaque Pong
du pair, il repasse à zéro. Si le compteur atteint
`max_missed_pings`, le battement de cœur envoie Close(1011 "no pong
response") et la connexion se termine. Réglez `max_missed_pings` sur
`usize::MAX` pour désactiver l'application de la règle (les pings
continuent de circuler, mais la connexion n'est jamais fermée pour des
pongs manqués).

Le premier tick est consommé au démarrage de la tâche afin que le
pair obtienne au moins un intervalle complet de grâce avant le premier
ping.

## Politique d'origine

Les navigateurs envoient toujours un en-tête `Origin` sur les
handshakes WebSocket. À la différence de `fetch()` /
`XMLHttpRequest`, les mises à niveau WebSocket ne sont pas protégées
par un middleware de token CSRF (le handshake ne porte aucun token),
si bien qu'une vérification `Origin` same-origin est la seule chose
qui se dresse entre une page malveillante et un point de terminaison
WS privilégié sur la session d'un utilisateur connecté. Le framework
applique la politique configurée avant que `hyper_tungstenite::upgrade`
ne soit appelé ; une violation retourne un HTTP 403 sans mise à
niveau.

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| Variante      | Comportement |
|--------------|----------|
| `SameOrigin` (défaut) | Autorise seulement quand l'hôte de `Origin` (et le port si présent) correspond à l'en-tête `Host` de la requête. Un `Origin` manquant est rejeté. Le schéma n'est pas comparé (TLS se termine en amont, donc le serveur ne peut pas dire de façon fiable si le schéma public était https ou http). |
| `AllowAny`   | Ignore la vérification. À utiliser seulement pour les points de terminaison non-navigateur (serveur-à-serveur, apps natives, mocks de test). |
| `AllowList(Vec<String>)` | Autorise seulement quand `Origin` correspond exactement (insensible à la casse) à l'une des origines fournies. Chaque entrée est la forme complète `scheme://host[:port]` qu'un navigateur enverrait. |

Les clients non-navigateur (outils CLI, serveurs, apps natives)
n'envoient typiquement pas d'en-tête `Origin`. Les routes qui servent
exclusivement de tels clients devraient utiliser `AllowAny` ; les
routes qui servent les deux devraient utiliser `AllowList` en
énumérant chaque origine frontend de production.

## Sous-protocoles

Un sous-protocole WebSocket est un token de niveau applicatif (par ex.
`graphql-transport-ws`, `jsonrpc-2.0`) sur lequel le client et le
serveur s'accordent pendant le handshake. Remplissez
`accepted_protocols` pour y participer :

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

Quand le client offre `Sec-WebSocket-Protocol`, le framework choisit
le premier token offert par le client (dans l'ordre de préférence du
client selon la RFC 6455 §4.2.2) qui recoupe `accepted_protocols`,
comparé sans tenir compte de la casse, et le renvoie en écho sur la
réponse 101. Si le client a offert des protocoles mais qu'aucun n'a
correspondu, la mise à niveau réussit quand même sans en-tête
`Sec-WebSocket-Protocol` - la RFC 6455 exige alors que le navigateur
fasse échouer la connexion côté client, ce qui est le bon comportement
(un serveur qui continuerait parlerait silencieusement le mauvais
protocole).

Quand `accepted_protocols` est vide, la négociation est entièrement
sautée - la réponse de mise à niveau omet `Sec-WebSocket-Protocol` et
le client retombe sur son traitement de protocole par défaut.

## Déploiement en production

Le framework gère le handshake et les I/O de frame. Vous n'avez
besoin d'aucune configuration supplémentaire côté framework pour la
production.

**La terminaison TLS se fait en amont.** Les clients se connectent en
`wss://` sur nginx, Caddy, ou le load balancer cloud ; le proxy retire
le TLS et transmet du `ws://` simple au framework. Le framework n'a
besoin ni d'une feature `rustls` ni d'un certificat TLS.

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` et `proxy_send_timeout` doivent être
suffisamment longs pour couvrir les intervalles d'inactivité entre les
battements de cœur. Avec le `ping_interval` par défaut de 30s, 3600s
est un plafond confortable.

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

Caddy gère `Upgrade` / `Connection` automatiquement lors du proxying ;
les directives `header_up` explicites ci-dessus sont là par souci de
clarté.

### Load balancers cloud (AWS ALB, GCP GLB)

Activez le support WebSocket sur la règle du listener TCP (AWS ALB le fait
automatiquement quand le protocole du target group est HTTP/1.1 avec
les sticky sessions désactivées). Assurez-vous que le délai
d'inactivité du load balancer est au moins aussi long que
`ping_interval` ; le battement de cœur du framework garde le réseau
actif, mais le LB coupe les connexions qui lui semblent inactives de
son point de vue.

## Arrêt gracieux

Chaque handler WebSocket spawné est suivi dans le `JoinSet`
`WS_TASKS` du serveur. Sur `Ctrl-C` ou un signal d'arrêt externe, le
listener TCP arrête d'accepter de nouvelles connexions et `Server::run`
vide l'ensemble avant que le processus ne se termine. Le futur du
handler ne se résout pas avant que le handshake de fermeture n'ait été
flushé : après que le `handle` de l'utilisateur retourne, le framework
attend le forwarder afin que la frame Close(1000) ou Close(1011)
finale soit écrite sur le réseau avant que la tâche de la connexion ne
soit signalée comme terminée. Dans un arrêt propre, les pairs voient
une fermeture normale, pas une réinitialisation TCP.

Les handles terminés sont récoltés de façon opportuniste pendant la
durée de vie du serveur, si bien que le `JoinSet` ne croît pas sans
limite sous un fonctionnement de longue durée.

## Référence

| Symbole | Objet |
|---|---|
| `suprnova::ws::WebSocketHandler` | Trait : `async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`. `Send + Sync + 'static`. |
| `suprnova::ws::WsSocket` | Handle bidirectionnel. Méthodes : `send_text`, `send_binary`, `recv_text`, `recv`, `close`. `close` valide le code + la longueur de la raison en amont. |
| `suprnova::ws::WsConfig` | Config par connexion. Champs : `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`. Constructeurs `Default` + `generous()`. Validé à l'enregistrement. |
| `suprnova::ws::OriginPolicy` | `SameOrigin` (défaut), `AllowAny`, `AllowList(Vec<String>)`. Appliqué au moment de la mise à niveau. |
| `ws!(path, Handler)` | Forme macro pour `routes! { ... }`. Retourne un `WsRouteDef` supportant `.config(WsConfig)` et `.middleware(M)` dans les deux ordres. |
| `Router::ws(path, handler)` | Enregistrement direct. Retourne `Router`. |
| `Router::ws_with_config(path, handler, cfg)` | Surcharge `WsConfig` par route. |
| `Router::ws_with_middleware(path, handler, mws)` | Liste de middleware par route. |
| `Router::ws_with_middleware_and_config(...)` | Les deux. |
| `Router::try_ws*` (famille) | Homologues faillibles - retournent `Err(FrameworkError)` en cas de motif dupliqué ou malformé plutôt que de paniquer. |

## Suivant

- [Diffusion](broadcasting.md) - canaux, présence, le protocole réseau par-dessus `ws!`
- [Événements serveur](sse.md) - push unidirectionnel pour les navigateurs derrière des proxys stricts
- [Routage](routing.md) - ce en quoi `routes!` et `ws!` se développent réellement
- [Middleware](middleware.md) - écrire un middleware qui filtre HTTP et WS de façon uniforme
- [Requêtes](requests.md) - en-têtes, cookies, query, extensions sur la `Request` que votre handler reçoit
