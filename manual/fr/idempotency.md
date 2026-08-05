# Idempotence

Quand un client réessaie un POST, vous voulez que le second appel
soit sûr. Le réseau n'est pas fiable et les clients réessaient - mais
`POST /charges` ne devrait jamais débiter la carte deux fois, et
`POST /orders` ne devrait jamais produire deux commandes pour un seul
clic. Les clés d'idempotence sont le contrat qui dit « si vous
revoyez cette même clé, donnez-moi la réponse d'origine ; ne refaites
pas le travail. »

`Idempotency` de Suprnova est une fine façade par-dessus `Cache::lock`
qui vous donne trois garanties croissantes : déduplication seule,
déduplication avec réessai en cas d'échec, et rejeu de résultat à la
Stripe. Toutes les trois gardent le bail du verrou vivant aussi
longtemps que le corps s'exécute, si bien qu'un corps lent ne peut
jamais laisser le verrou expirer et laisser passer un doublon.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // S'exécute exactement une fois par clé dans la fenêtre de 24 heures.
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* premier appel - id est la nouvelle commande */ {},
    Idempotent::FreshUnfenced(id) => {
        // La commande a été passée, mais le bail du verrou a été perdu
        // en cours de route, donc un autre appelant peut en avoir passé
        // une aussi. Réconciliez ou alertez - voir « Quand l'exclusivité
        // est perdue » plus bas.
    },
    Idempotent::Duplicate => /* même clé déjà utilisée */ {},
}
```

## Les trois primitives

| Méthode | Le corps s'exécute | Le doublon voit | L'échec libère le verrou ? | À utiliser quand |
|---|---|---|---|---|
| `Idempotency::once` | exactement une fois par fenêtre | marqueur `Duplicate` | non | les effets de bord ne doivent JAMAIS se répéter (mail envoyé, débit tenté) |
| `Idempotency::commit_on_success` | une fois par succès par fenêtre | marqueur `Duplicate` | oui | les échecs transitoires doivent pouvoir être réessayés, mais un succès tient |
| `Idempotency::remember` | une fois par succès par fenêtre | la valeur de retour d'origine | oui | les doublons doivent recevoir le payload d'origine, pas un marqueur |

Toutes les trois vivent sous `suprnova::idempotency` et sont
ré-exportées depuis la racine de la crate comme `Idempotency`,
`Idempotent`, et `Replay`. Elles partagent le même hachage de clé, le
même renouvellement de bail, et la même sémantique de verrou - seule
la policy de succès/échec diffère.

### `Idempotency::once` - au plus une fois

Le contrat le plus strict. Le premier appelant dans la fenêtre TTL
exécute le corps et obtient `Fresh(value)`. Chaque appelant suivant
dans la fenêtre obtient `Duplicate` et le corps NE s'exécute PAS à
nouveau - même si le corps du premier appelant a retourné `Err`. Le
TTL EST la fenêtre de déduplication.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// Envoie un e-mail de bienvenue exactement une fois par
// inscription, peu importe combien de fois le callback
// d'inscription réessaie.
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

Utilisez `once` quand l'effet de bord est du genre « j'ai essayé ;
même si j'ai échoué après l'effet de bord, ne réessayez pas » -
envoyer un e-mail, poster vers une API externe qui n'honore pas ses
propres clés d'idempotence, écrire une entrée de journal d'audit dont
la double écriture corromprait des analyses en aval.

### `Idempotency::commit_on_success` - au moins une fois en cas de succès, réessai en cas d'échec

Comme `once`, mais si le corps retourne `Err`, le verrou de
déduplication est libéré, si bien que le prochain appelant dans la
fenêtre TTL peut réessayer. Un corps réussi garde le verrou pour le
reste de la fenêtre.

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // Poste un message vers un service amont. Les erreurs réseau
        // sont transitoires - le réessai suivant devrait réentrer, pas se
        // faire dire « déjà fait » quand rien ne s'est vraiment passé.
        social_media_client.post(&post).await
    },
)
.await?;
```

Utilisez `commit_on_success` quand le corps a des modes d'échec
réessayables (erreurs réseau transitoires, limites de débit en
amont, identifiants expirés qu'un rafraîchissement corrigerait) et
que vous voulez au moins une fois en cas de succès, mais que le
verrou se rende sur un échec pour qu'un réessai puisse réentrer.

### `Idempotency::remember` - rejeu de résultat à la Stripe

Le contrat pour lequel l'en-tête HTTP `Idempotency-Key` a été
inventé. Le premier appelant exécute le corps, stocke la valeur de
succès, et obtient `Replay::Fresh`. Un appelant ultérieur dans la
fenêtre obtient `Replay::Replayed(<valeur d'origine>)` - la valeur de
retour enregistrée, pas un marqueur. Un appelant concurrent qui
arrive *pendant* que le premier tourne encore obtient
`Replay::InProgress`.

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // Extrait l'en-tête en un String possédé avant de consommer `req` pour le corps.
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // Même réponse pour le client, mais ça vaut une métrique :
            // l'exclusivité n'a pas tenu pour tout le corps.
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Remarquez que `Fresh` et `Replayed` sont traités de façon identique
par la réponse client-facing - tout l'intérêt de `remember` est que
le second appelant ne peut pas savoir s'il est celui qui a exécuté le
corps ou s'il a reçu le résultat enregistré.

`InProgress` est le cas qui vaut la peine d'y réfléchir : un doublon
est arrivé pendant que le corps du premier appelant s'exécutait
encore, donc il n'y a pas encore de résultat enregistré à renvoyer.
`409 Conflict` avec un en-tête `Retry-After: 1` est la réponse
canonique - le client recule brièvement, puis réessaie, et la seconde
tentative soit course l'original jusqu'au court-circuit `Cache::get`,
soit tombe sur `Replayed`.

## Matériel de clé

Les trois méthodes acceptent un `&str` arbitraire pour la clé. Avant
qu'elle ne touche le backend de cache, la clé est hachée en SHA-256
dans un digest hex de 64 caractères. Cela vous achète trois choses :

1. **Longueur de clé backend bornée.** Un client qui POST un en-tête
   `Idempotency-Key` de 10 Ko produit quand même une clé de cache de
   64 octets.
2. **Les identifiants bruts ne fuitent pas dans l'outillage de
   cache.** Si la clé contient une adresse e-mail, un id de session,
   ou un id utilisateur interne, ceux-ci n'apparaissent pas dans
   `redis-cli KEYS idem:*`.
3. **Pas de collision de classe de caractères.** Tout ce que le
   backend de cache interprète spécialement (deux-points, caractères
   glob, octets de contrôle) a déjà disparu - le hash est
   hexadécimal uniquement.

Le hash porte sur la clé fournie par l'utilisateur, pas sur le
préfixe de clé de cache - `Idempotency::once("k", …)` et
`Idempotency::once("k", …)` depuis deux points d'appel différents
dans le même process entrent en collision exprès. Préfixez vos clés
vous-même si vous ne voulez pas de ça :

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## Renouvellement du bail - le problème du corps lent

Une combinaison naïve verrou + TTL a un bug de fenêtre : si le corps
s'exécute plus longtemps que le TTL, le verrou expire pendant que le
corps tourne encore, et un second appelant peut acquérir un verrou
neuf et exécuter le corps à nouveau, en même temps. Le contrat de
déduplication casse précisément pour les opérations assez lentes pour
en avoir besoin.

Suprnova résout cela en spawnant une tâche d'arrière-plan qui
rafraîchit le verrou à un tiers du TTL (avec un plancher de 50 ms)
pour toute la durée du corps. Un `tokio::select!` avec un ordre
`biased` garantit que la branche du corps est la seule à jamais
résoudre le future.

Une *erreur* de rafraîchissement n'est pas traitée comme un bail
perdu. Elle signifie que le backend n'a pas pu être sollicité, pas
que quelqu'un d'autre a pris le verrou, si bien que le renouvellement
réessaie à l'intervalle suivant et n'abandonne qu'après plusieurs
échecs consécutifs. Abandonner au premier accroc garantissait que le
bail expirerait même quand le backend récupérait quelques
millisecondes plus tard.

### Quand l'exclusivité est perdue

Le renouvellement peut quand même échouer véritablement : le token
cesse de correspondre, parce que le verrou a expiré et que quelqu'un
d'autre l'a réclamé. À ce moment-là, deux appelants peuvent être en
train d'exécuter le même corps.

Le corps n'est **pas** annulé. Au moment où un bail est perdu, il a
peut-être déjà débité une carte ou envoyé un message, et l'annuler
laisserait ça à moitié fait sans que rien ne l'enregistre. Le corps
s'exécute jusqu'au bout et la perte est signalée :

| Résultat | Signifie |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | le corps a tourné, l'exclusivité a tenu tout du long |
| `FreshUnfenced(v)` | le corps a tourné et a produit `v`, mais un autre appelant a peut-être tourné en même temps |

`FreshUnfenced` est un variant séparé plutôt qu'un flag sur `Fresh`,
spécifiquement pour qu'un `match` exhaustif ne puisse pas l'ignorer
par accident. Quoi en faire est à vous de décider - réconcilier,
alerter, compenser - mais le traiter comme `Fresh` jette le seul
signal que vous obtenez que la garantie n'a pas tenu.

Perdre un bail exige que le backend soit injoignable pendant
plusieurs intervalles de rafraîchissement, ou une pause
stop-the-world plus longue que le TTL. C'est rare. Ce n'est pas
impossible, et c'était invisible avant.

En pratique : choisissez un TTL basé sur votre fenêtre de
déduplication (`combien de temps une requête dupliquée devrait-elle
être dédupliquée ?`), pas sur la durée du corps dans le pire cas. Un
corps de 30 minutes avec un TTL d'1 minute convient très bien - le
verrou sera rafraîchi environ quatre-vingt-dix fois pendant
l'exécution du corps.

Un test qui exerce ceci : un TTL de 200 ms avec un corps qui bloque
pendant 500 ms, et un second appelant qui arrive à 400 ms. Sans
renouvellement, le second appelant réexécuterait le corps. Avec
renouvellement, il voit `Duplicate`. Le verrou tient.

## Backend partagé

La déduplication inter-process exige un cache inter-process. Le
backend en mémoire garde les verrous dans un `HashMap` par process,
si bien que deux instances `cargo run` sur la même machine ne
verront pas les clés d'idempotence l'une de l'autre. Les déploiements
de production où l'un de ces cas compte - plusieurs processus d'app,
mise à l'échelle horizontale, déploiements blue/green avec des
fenêtres de trafic qui se chevauchent - doivent définir
`CACHE_DRIVER=redis` et fournir une `REDIS_URL` joignable.

L'amorçage échoue fermé : si `CACHE_DRIVER=redis` et que Redis est
injoignable, l'app refuse de démarrer plutôt que de rétrograder
silencieusement vers de la mémoire par process. Voir
[cache.md](cache.md) pour le contrat complet du backend de cache.

## Gestion des erreurs

Le `FrameworkError` du corps se propage à travers `Idempotency` sans
changement. Un échec d'acquisition de verrou (Redis est en panne en
cours de requête, le backend renvoie une erreur) se propage comme un
`FrameworkError` depuis la couche cache - il n'y a pas de repli
silencieux. Le type d'erreur est le `FrameworkError` standard du
framework, si bien que les handlers peuvent le faire remonter avec
`?` jusqu'au convertisseur d'erreur de leur contrôleur :

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

Un échec de libération sur le chemin `Err` de `commit_on_success` ou
`remember` est **journalisé, jamais retourné** - l'erreur du corps
est la seule erreur que l'appelant voit sur ce chemin. Une libération
échouée signifie que le verrou tiendra jusqu'à ce que le TTL expire ;
un réessai dans la fenêtre verra `Duplicate` ou `InProgress` jusque-
là. Les logs incluent la clé hachée (jamais le matériel de clé brut),
si bien que les opérateurs peuvent corréler sans faire fuiter de
données personnelles.

## Annulation

Si l'appelant abandonne le future `Idempotency::remember` avant que
le corps ne se termine, le corps est annulé comme n'importe quelle
autre branche `tokio::select!` - le verrou n'est **pas** libéré, et
un doublon qui arrive avant l'expiration du TTL voit `InProgress`
(puis, après le TTL, `Fresh` à nouveau). C'est le défaut sûr : un
corps à moitié terminé dont vous ne connaissez pas les effets ne
devrait pas être présumé sûr à réessayer. Enveloppez dans un
`tokio::spawn` les corps qui portent des effets de bord non gérés, et
joignez le handle si vous avez besoin de rendre le corps
non-annulable.

## Intégration à la file d'attente

La couche de file d'attente utilise `Idempotency::commit_on_success`
en interne pour implémenter `Queue::push_unique`. Si vous voulez
qu'un job soit mis en file d'attente au plus une fois par fenêtre
`Job::unique_for()` par `Job::unique_id(&self)`, vous n'avez pas
besoin d'appeler `Idempotency::*` vous-même :

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // Nous avons gagné la course ; le job est dans la file.
} else {
    // Un autre appelant l'a déjà mis en file ; traitez ça comme un succès.
}
```

Voir [queues.md](queues.md) pour le contrat complet d'unicité de job.

## Entrée des webhooks de paiement

Le handler de webhook de paiement N'utilise PAS `Idempotency::*`.
L'entrée des webhooks a une exigence plus stricte - chaque événement
doit être auditable, même à la première livraison, si bien que la
ligne d'audit est la source de vérité et que la clé de déduplication
est la contrainte BD `UNIQUE(provider, provider_event_id)`.
`Idempotency::remember` stockerait le payload de réponse dans le
cache ; le handler de webhook stocke l'*enveloppe complète de
l'événement plus le résultat du traitement* dans
`payments_webhook_events`, ce qui signifie qu'un opérateur peut
rejouer ou retraiter des événements hors ligne en lisant la table.

Les deux patterns sont complémentaires. Utilisez `Idempotency::*`
pour des clés pilotées par le client, avec déduplication bornée par
TTL ; utilisez une table d'audit indexée `UNIQUE` pour l'entrée de
webhooks pilotée par le fournisseur qui a besoin d'une auditabilité
au-delà du TTL du cache. Voir [payments.md](payments.md) pour le
contrat de webhook.

### Pourquoi Suprnova diverge

`Cache::lock` de Laravel est une primitive ; le contrat d'idempotence
à la Stripe (enregistrer le résultat, le rejouer, distinguer en-cours
de doublon) est laissé comme une recette userland. Chaque projet
Laravel qui en a besoin finit par écrire la même danse
verrou-et-cache, généralement avec l'un de ces trois bugs :

1. **Pas de renouvellement de bail.** Un corps qui survit au TTL se
   réexécute en même temps chez un appelant en doublon. Le verrou
   était là ; il a juste expiré au mauvais moment.
2. **Libération sur le chemin de succès.** Libérer le verrou quand
   le corps réussit ouvre une fenêtre entre `body() -> Ok` et le
   prochain appelant qui acquiert un verrou neuf - exactement la
   fenêtre que la déduplication était censée fermer.
3. **Clés brutes dans le backend de cache.** Les en-têtes
   `Idempotency-Key` fournis par le client vont directement dans des
   clés Redis, faisant fuiter des données personnelles dans
   l'outillage opérateur et produisant des tailles de clé non
   bornées.

Suprnova livre la recette comme une primitive de premier ordre, si
bien que chaque appelant obtient le même renouvellement de bail, la
même sémantique de libération à échec fermé, la même sûreté de clé
hachée. Les trois méthodes (`once`, `commit_on_success`, `remember`)
nomment les trois policies entre lesquelles vous devez réellement
choisir - prenez celle qui correspond au modèle d'échec de votre
corps et passez à autre chose.

## Tests

`Idempotency` résout son `CacheStore` à travers le conteneur, si bien
que les tests qui lient un `InMemoryCache` obtiennent un cache neuf
et isolé par test :

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

Le propre `framework/tests/idempotency.rs` du framework couvre la
surface du contrat : suppression des doublons, expiration du TTL,
policy de libération erreur-vs-succès, renouvellement de bail à
travers des durées de corps qui survivent au TTL, la course
`InProgress`, et le cas où `release_lock` du cache lui-même échoue en
erreur. Lisez ces tests si vous voulez voir le comportement exact sur
lequel vous pouvez compter.

## Pièges

- **`Idempotency::once` consomme la fenêtre en cas d'erreur.** Un
  premier appelant en échec garde quand même le verrou jusqu'à
  l'expiration du TTL. Utilisez `commit_on_success` si vous voulez
  des réessais dans la fenêtre.
- **`Idempotency::remember` stocke `T` dans le backend de cache.**
  La clé est hachée, mais le *payload* est sérialisé avec serde et
  écrit dans le backend. Ne mettez pas de secrets dans une valeur
  rejouée qui ne doit pas apparaître dans votre magasin de cache.
- **Deux processus ont besoin d'un cache partagé.** La déduplication
  en mémoire est par process. La correction inter-process exige
  `CACHE_DRIVER=redis` (ou un autre magasin inter-process).
- **Les TTL sous 150 ms ne sont pas testés au niveau du bail.** Le
  plancher de renouvellement est 50 ms, donc un TTL de 100 ms se
  rafraîchit environ toutes les 50 ms - correct pour le contrat,
  mais les tests de bail du framework tournent à `ttl >= 1s`.
  Utilisez des fenêtres de déduplication réalistes ; une fenêtre
  d'idempotence mesurée en millisecondes signifie généralement que
  le contrat n'est pas tout à fait le bon outil.
- **L'annulation du corps ne libère pas le verrou.** Un corps annulé
  laisse le verrou tenir jusqu'à l'expiration du TTL. C'est le choix
  à échec fermé ; agencez vos timeouts pour que l'annulation
  corresponde à ce qu'un appelant en doublon devrait voir.

## Suivant

- [cache.md](cache.md) - la primitive de verrou sous-jacente et la
  sélection de `CACHE_DRIVER`.
- [queues.md](queues.md) - comment `Queue::push_unique` s'appuie sur
  `Idempotency::commit_on_success` pour la déduplication au niveau
  job.
- [payments.md](payments.md) - l'entrée de webhooks qui utilise
  l'idempotence par ligne BD plutôt que la déduplication par clé de
  cache, et quand utiliser laquelle.
- [rate-limiting.md](rate-limiting.md) - middleware adjacent qui
  utilise le même backend `Cache` pour l'application à fenêtre
  glissante.
- [middleware.md](middleware.md) - comment factoriser l'extraction de
  clé d'idempotence dans un middleware réutilisable par-dessus vos
  routes POST/PUT.
