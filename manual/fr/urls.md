# Génération d'URL

Les URL sont la façon dont votre application se référence elle-même -
chaque redirection, chaque lien d'e-mail, chaque href de `<Link>`
Inertia, chaque téléchargement signé doit bien venir de quelque part.
Coder les chemins en dur rend les refactorisations pénibles et les
renommages de route dangereux. Suprnova livre un petit espace de noms
`url::` et un helper `route()` qui l'accompagne : tous deux prennent un
nom plus des paramètres et vous rendent une chaîne, avec l'encodage en
pourcentage pris en charge, la génération de signatures disponible et
une vérification qui correspond octet pour octet au format réseau de
Laravel.

Ce chapitre est la référence de la surface de génération d'URL. Le
chapitre [Routage](routing.md) couvre la façon de déclarer des routes et
de les nommer ; celui-ci couvre ce que vous faites de ces noms ensuite.

```rust
use suprnova::{route, url};

// Recherche par nom → URL
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// URL absolue par rapport à APP_URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// Lien signé pour la réinitialisation de mot de passe
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// Vérification sur la requête entrante
if url::has_valid_signature(&request)? {
    // agir en conséquence
}
```

Tout ce que couvre ce chapitre est réexporté sous `suprnova::url::*` et
`suprnova::route`, pour que le code consommateur n'ait jamais à aller
chercher directement dans le module de routage.

## Routes nommées

Un nom est une étiquette de chaîne attachée à une route au moment de
l'enregistrement. Une fois qu'un nom existe, `route(name, params)` le
résout en un motif d'URL et substitue les paramètres. Les noms vivent
dans un unique registre global au processus - il y a une table
`name → path` par binaire en cours d'exécution, pas une par `Router`.

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

L'appel `.name(...)` enregistre `"users.show" → "/users/{id}"`. À partir
de là, n'importe quel endroit du processus peut résoudre le nom :

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

Réenregistrer la même paire `(name, path)` est idempotent - utile quand
l'enregistrement des routes s'exécute plus d'une fois pendant
l'amorçage. Enregistrer un nom sous un chemin *différent* panique ;
cette collision est un bug à forme de sécurité, car des helpers comme
`Redirect::route` viseraient silencieusement le côté qui a gagné la
course.

### Les helpers de recherche

| Fonction | Retourne | Quand la route est absente |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

La paire indulgente `route` / `route_with_params` laisse tel quel dans
sa sortie tout segment `{placeholder}` non rempli - acceptable pour des
journaux de débogage, dangereux à envoyer à un navigateur. La paire
stricte `try_route` / `try_route_with_params` retourne
`RouteUrlError::MissingParams { name, missing }`, qui liste les segments
`{placeholder}` non remplis pour que l'appelant échoue explicitement au
lieu de rediriger un utilisateur vers `/users/{id}`.

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* redirection sûre */,
    Err(RouteUrlError::MissingParams { name, missing }) => {
        // missing == vec!["id"]
        return Err(FrameworkError::internal(
            format!("cannot build URL for {name}: missing {missing:?}"),
        ));
    }
    Err(RouteUrlError::NameNotFound(name)) => {
        return Err(FrameworkError::internal(format!("unknown route: {name}")));
    }
}
```

`Redirect::route` utilise `try_route_with_params` sous le capot pour
exactement cette raison - une redirection portant un `{id}` brut dans
l'en-tête `Location` serait pire qu'un échec.

### L'encodage en pourcentage est automatique

Les valeurs de paramètre sont encodées selon les règles de segment de
chemin de la RFC 3986 avant d'être substituées. Cela couvre les
gen-delims et les sub-delims (`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`), les
caractères de contrôle, l'espace et `%` lui-même. Les caractères non
réservés (`A-Z a-z 0-9 - _ . ~`) passent sans changement.

```rust
use suprnova::route;

// Un slug contenant une barre oblique tient dans un seul segment :
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// Les tentatives de traversée de chemin ne peuvent pas sortir du segment :
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// L'Unicode véritable passe intact :
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

Le côté correspondance préserve cet aller-retour - une requête vers
`/posts/hello%2Fworld` correspond à la route `/posts/{slug}`, et un
handler qui lit `req.param("slug")` voit `"hello/world"`, décodé.
Encodez à la frontière, décodez à la frontière ; ne voyez jamais les
octets bruts dans le code du handler.

### Recherche inverse

Quand vous avez un motif de route correspondant et que vous voulez le
nom enregistré - par exemple pour la journalisation ou pour des
vérifications `Request::route_is("users.show")` - utilisez
`route_name_for_pattern` :

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

C'est un balayage en O(n) du registre des noms. n est le nombre de noms
enregistrés ; même avec un nombre de routes à quatre chiffres, le coût
est négligeable face au cycle de vie de requête qui l'entoure. La
fonction est exposée pour l'outillage et le middleware -
`Request::route_is` l'appelle déjà pour vous quand vous comparez avec
une route nommée dans un handler.

## URL absolues

Pour tout le reste - composer des e-mails, partager des URL, envoyer des
métadonnées Open Graph - vous voulez une URL absolue avec le bon schéma
et le bon hôte. `url::to` joint un chemin à `APP_URL` :

```rust
use suprnova::url;

// Dans l'env : APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// Les URL déjà absolues passent sans changement :
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

L'hôte, le schéma et le port viennent tous d'`APP_URL`. Si `APP_URL`
vaut `http://localhost:8765`, alors `url::to("/foo")` donne
`"http://localhost:8765/foo"`. La barre oblique finale d'`APP_URL` est
normalisée et retirée, si bien que vous ne vous retrouvez jamais avec
`https://host//path`.

### Forcer HTTPS

`url::secure(path)` construit la même URL absolue mais fait passer le
schéma à `https://`, même si `APP_URL` est en `http://` :

```rust
use suprnova::url;

// Dans l'env : APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

En production, vous définissez typiquement `APP_URL` sur votre hôte
HTTPS une fois pour toutes et vous n'appelez jamais `secure`
directement - la promotion sert aux environnements où le développement
local passe en HTTP mais où un lien précis doit être en HTTPS (par
exemple une URL de callback intégrée à une session de paiement).

### Lire l'URL courante

À l'intérieur d'un handler, la requête elle-même fait autorité :

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // URL précédente enregistrée par la session
    // ...
}
```

| Helper | Retourne | Source |
|---|---|---|
| `url::current(&req)` | chemin + chaîne de requête de cette requête | La `Request` courante |
| `url::full(&req)` | URL absolue de cette requête | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | URL précédente enregistrée par le middleware de session | `_previous.url` dans la session, ou `fallback` |

`previous` est ce qui alimente `Redirect::back` - le middleware de
session enregistre l'URL de chaque GET HTML réussi, pour qu'un `POST` de
formulaire puisse revenir vers la page qui l'a soumis. Les rechargements
partiels Inertia, les requêtes JSON-API (`Accept: application/json` sans
`text/html`) et les réponses hors 2xx/3xx sont ignorés, pour que vous ne
reveniez jamais vers un point de terminaison intermédiaire que
l'utilisateur n'a jamais vu. Le middleware refuse également d'enregistrer une
URL qui n'est pas relative à la racine et de même origine : une requête avec
un chemin en forme de `//host` ou `/\host` (les deux que le navigateur lit
comme relatif au protocole, pas comme un chemin) ou contenant un octet de
contrôle ASCII n'importe où (une `TAB` ou une nouvelle ligne que l'analyseur
d'URL du navigateur supprime avant de comparer les origines, transformant ce
qui semble un chemin sûr en l'une des deux formes ci-dessus) n'est jamais
stocké - et la même vérification s'exécute à nouveau à chaque lecture, donc
une valeur stockée par une version plus ancienne continue d'échouer, plutôt
que d'être confiée uniquement parce qu'elle est déjà dans la session. De
toute façon, `previous` et `Redirect::back` ne peuvent pas être détournés
hors-domaine par un chemin de requête inhabituel atteignant votre
application, dans le passé comme aujourd'hui.

## URL signées

Les URL signées vous permettent de forger une URL qui prouve qu'elle
vient de votre serveur, sans stocker l'URL nulle part. La signature est
un HMAC-SHA256 sur la forme canonique de l'URL, calculé avec votre
`APP_KEY` ; le serveur recalcule le HMAC sur la requête entrante et
n'accepte que les signatures qui correspondent.

Recourez aux URL signées dans ces cas :

- **Des liens envoyés par e-mail** - réinitialisation de mot de passe,
  vérification d'e-mail, invitation par e-mail, connexion par lien
  magique. L'URL doit survivre à un aller-retour par une boîte de
  réception sans qu'on puisse la stocker comme état opaque.
- **Des téléchargements éphémères** - les liens « votre export CSV est
  prêt » qui expirent au bout de 24 heures, les alternatives aux liens
  S3 signés quand vous voulez que l'URL reste sur votre domaine.
- **Des webhooks qui pointent vers vous** - les callbacks de tiers qui
  doivent refuser les appels forgés sans exiger une recherche en base de
  données à chaque requête.

```rust
use suprnova::url;
use chrono::Utc;

// URL signée permanente - n'expire jamais.
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// URL signée temporaire - expire dans une heure.
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

Notez qu'`expires_at_epoch_seconds` est un **timestamp UNIX absolu**,
pas une durée. Calculez-le au site d'appel :

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

Cela garde la signature du helper petite et vous laisse réutiliser la
même fonction pour des échéances relatives à maintenant comme pour des
échéances absolues explicites.

### Vérification

Du côté entrant, vous vérifiez la signature contre la requête en cours :

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // La signature est bonne et non expirée - on continue.
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`has_valid_signature` retourne `true` uniquement quand le HMAC
correspond ET que l'URL n'a pas expiré. Pour la distinction à trois
voies entre *invalide*, *expirée* et *valide*, utilisez
`signature_verdict` :

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // On continue.
        }
        SignatureVerdict::Expired => {
            // Renvoyer l'utilisateur vers une page qui explique que le
            // lien a expiré et propose d'en envoyer un nouveau.
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // Rendre un 403 générique - ne faites pas fuiter si la
            // signature était malformée, absente ou simplement fausse.
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` est déprécié et répond désormais
exactement ce que répond `has_valid_signature`. Recourez plutôt au
`signature_verdict` ci-dessus ; une URL sans paramètre de requête
`expires` n'a « jamais expiré » par définition, dans Suprnova comme dans
Laravel.

### Pourquoi Suprnova diverge

Le `URL::signatureHasNotExpired($request)` de Laravel dit littéralement
« pas expirée », si bien qu'une signature **forgée** revient à `true` -
elle n'a jamais eu d'expiration à manquer. Celui de Suprnova collait
autrefois à ce comportement. Ce n'est plus le cas : le helper exige
d'abord une signature valide.

La raison, c'est qu'`expires` est fourni par l'attaquant tant que le
HMAC ne dit pas le contraire, si bien qu'aucune réponse qui en dérive ne
veut dire quoi que ce soit avant que la signature ne soit vérifiée - et
une fonction dont le nom laissait croire à un garde-fou faisait passer
chaque URL forgée à travers tout code qui l'appelait seule.

Exiger la validité la fait se confondre avec `has_valid_signature`, et
c'est pourquoi elle porte une dépréciation plutôt qu'un flag de
comportement. Cette fusion n'est pas une perte : sous un verdict à trois
états, il n'existe aucun « pas expirée » qu'un simple `bool` puisse
rapporter honnêtement, sinon `Valid`. Si vous voulez distinguer
*expirée* d'*invalide* - pour dire « demandez un nouveau lien » plutôt
que « interdit » - c'est à cela que sert `signature_verdict`, et il le
dit dans le type.

### Signer des URL arbitraires

Si l'URL que vous voulez signer ne provient pas d'une route nommée
enregistrée - une URL de callback que vous a remise un tiers, un chemin
construit dynamiquement à l'exécution - utilisez directement
`signed_url` :

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // expiration à 10 minutes
)?;
```

Passez `None` comme expiration pour forger une signature permanente. Le
côté vérification est identique - `has_valid_signature(&req)` se moque
de savoir si l'URL a été forgée depuis une route nommée ou depuis un
chemin brut.

### Format réseau

Deux URL qui ne diffèrent que par l'ordre de leurs paramètres de requête
produisent des signatures identiques, car la forme canonique trie les
paires de la chaîne de requête par ordre lexicographique avant le
hachage. Cela compte parce que les clients réordonnent parfois les
paramètres de requête en transit (proxys, générateurs d'aperçus de
liens, applications d'e-mail mobiles), et une URL signée qui casserait
sous un réordonnancement serait inutilisable.

| Composant | Valeur |
|---|---|
| Algorithme | HMAC-SHA256 |
| Clé | Les octets bruts de l'`APP_KEY` active |
| Charge utile | `path?<sorted-query>` (le `?` est omis en l'absence de paramètres) |
| Ordre de tri | `(key, value)` - chaque paire, répétitions comprises |
| Encodage | Empreinte de 64 caractères encodée en hexadécimal |
| Comparaison | En temps constant via `subtle::ConstantTimeEq` |
| Clés réservées | `signature`, `expires` |

**Les clés répétées sont signées, pas fusionnées.** `?tag=a&tag=b`
emporte les deux valeurs dans la charge utile, si bien qu'aucune ne peut
être ajoutée, retirée ou substituée sans casser la signature. C'est le
tri sur `(key, value)` plutôt que sur la seule clé qui maintient cet
ordre total, de sorte que la garantie de réordonnancement ci-dessus
tient encore quand une clé apparaît plus d'une fois.

Cela vaut la peine d'être dit, car l'alternative fait très mal. Une
version antérieure canonicalisait dans une map, qui ne gardait que la
dernière valeur d'une clé répétée. `Request::query_param`, lui,
retournait la *première*. Un `?user=victim` légitimement signé pouvait
donc être rejoué en `?user=attacker&user=victim` avec la signature
d'origine : la vérification voyait `victim` et passait, et le handler
agissait sur `attacker`. L'URL signée et l'URL exécutée n'étaient pas la
même. Les trois accesseurs de chaîne de requête - `query_param`,
`query_params` et `Context::query_param` - résolvent désormais une clé
répétée vers sa dernière valeur, et la forme canonique ne perd rien.

Un `signature` ou un `expires` répété est refusé d'emblée. Ce sont des
paramètres de contrôle ; deux exemplaires de l'un ou de l'autre ne
laissent aucune réponse non arbitraire à « lequel fait foi ? », et le
vérificateur ne doit pas être le composant qui devine.

La charge utile du HMAC exclut tout paramètre de requête `signature`
préexistant (donc signer par-dessus une signature est sans effet) et
réémet une valeur `expires` fraîche à partir des arguments d'appel. Un
client qui retire ou réécrit l'`expires` casse la signature ; un client
qui retire la `signature` échoue en `Invalid`. Les deux échouent de
manière fermée.

Le fragment (`#section`) est retiré de la forme canonique, car les
navigateurs ne retransmettent jamais les fragments au serveur. Signer
par-dessus un fragment invaliderait chaque lien dès qu'un client
ajouterait une ancre - `?signature=...#docs` ne se vérifierait pas côté
serveur.

### Paramètres de requête réservés

`signature` et `expires` sont des noms de paramètre de requête réservés.
Une route qui attend légitimement un paramètre de requête nommé
`signature` ou `expires` entrerait en collision avec la machinerie des
URL signées, et le vérificateur attribuerait la valeur au mauvais
endroit. Renommez le paramètre, ou enveloppez les paramètres entrants de
la route sous un autre espace de noms.

```rust
// Mauvais - `signature` entre en collision avec le nom réservé.
get!("/api/check", check)  // prend ?signature=hash

// Bon - donnez-lui son propre espace de noms.
get!("/api/check", check)  // prend ?body_signature=hash
```

Les constantes sont exposées par symétrie avec le format réseau de
Laravel :

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### Rotation des clés

Les URL signées utilisent le même `APP_KEY` qui alimente
`Crypt::encrypt` et l'intégrité du cookie de session. Faire tourner
`APP_KEY` invalide toute signature forgée auparavant et encore en
circulation - un e-mail de réinitialisation de mot de passe en
circulation devient un 403 au prochain clic de l'utilisateur.

Pour la plupart des applications, c'est le bon comportement. S'il vous
faut une rotation en douceur avec recouvrement (pour que les anciens
liens continuent de fonctionner pendant une fenêtre de déploiement),
utilisez `APP_KEY_PREVIOUS` pour reporter la clé précédente ; le
trousseau de clés essaie chaque clé installée à la vérification. Voir le
chapitre [Hachage](hashing.md) pour toute l'approche du trousseau.

## Erreurs et cas limites

Une poignée de modes de défaillance méritent d'être connus :

- **`route(name, ...)` retourne `None`** quand le nom n'est pas
  enregistré. C'est la surface indulgente - l'échec silencieux est
  intentionnel, pour que le code appelant puisse se replier sur une
  valeur par défaut. Utilisez `try_route` pour un échec explicite.
- **`try_route` retourne `Err(NameNotFound)`** pour un nom inconnu et
  `Err(MissingParams { name, missing })` quand un `{placeholder}` requis
  n'a pas de valeur correspondante.
- **`url::signed_route` et ses semblables retournent `FrameworkError`**
  quand la clé de chiffrement n'est pas installée (par exemple vous avez
  oublié `APP_KEY` dans `.env`). Cela échoue au démarrage en production,
  car `Crypt::init` s'exécute pendant `Server::from_config` ; le chemin
  d'erreur existe ici pour faire remonter une mauvaise configuration de
  manière visible au lieu de produire des liens invérifiables.
- **`has_valid_signature` retourne `Ok(false)`**, pas `Err`, pour une
  signature invalide ou expirée. La variante `FrameworkError` est
  réservée aux échecs du type « le serveur ne peut même pas vérifier »
  (clé absente).
- **Une URL signée dont l'`expires` a été altéré** se vérifie en
  `Invalid`, pas en `Expired`. La charge utile du HMAC inclut la valeur
  `expires`, donc la modifier casse d'abord la signature.

```rust
use suprnova::{routing::SignatureVerdict, url};

// Tous ceux-ci sont Invalid, pas Expired :
url::signature_verdict(&req)?;  // paramètre de requête signature absent
url::signature_verdict(&req)?;  // signature non hexadécimale, du charabia
url::signature_verdict(&req)?;  // chemin altéré (/orders/1 → /orders/2)
url::signature_verdict(&req)?;  // une valeur de paramètre de requête altérée
url::signature_verdict(&req)?;  // valeur expires altérée

// Celui-ci est Expired :
url::signature_verdict(&req)?;  // HMAC valide, mais maintenant > expires
```

## Pourquoi Suprnova diverge

La façade `URL` de Laravel porte `asset()`, `secureAsset()`,
`assetFrom()` et `action()`. Suprnova n'en livre aucune - pour des
raisons délibérées.

**Les assets**. L'approche frontend de Suprnova, c'est Vite plus les
disques du système de fichiers
([Système de fichiers et stockage](filesystem.md)), pas un helper
d'assets autonome. La directive `@vite('resources/app.ts')` de Vite (ou
son équivalent dans l'adaptateur Inertia) émet les bonnes URL hachées en
production et l'URL du serveur de développement en développement. Bâtir
un canal `URL::asset()` parallèle scinderait la gestion des assets entre
deux systèmes qui devraient s'accorder sur le hachage, le versionnage et
le manifeste qui fait autorité. Le côté Vite a déjà remporté cette
responsabilité.

**Le routage par action**. Le `action('UserController@show', ['id' => 1])`
de Laravel repose sur le routage par chaîne de classe de PHP - les
contrôleurs sont des classes avec des méthodes, et le framework peut
faire une recherche inverse sur une chaîne `action`. Les handlers Rust
sont des fonctions libres. L'analogue le plus proche, ce sont les routes
nommées, et `route("users.show", &[("id", "1")])` est déjà la bonne
interface. Réintroduire un routage par chaîne d'action par-dessus les
types de handler Rust n'apporterait rien de réel par rapport aux routes
nommées.

**`URL::forceScheme()` / `URL::forceRootUrl()`**. Laravel les expose
pour les tests et pour les sites derrière des proxys inverses qui ne
transmettent pas `X-Forwarded-Proto`. Suprnova traite les deux cas par
la configuration : `APP_URL` porte l'hôte et le schéma canoniques ; pour
les environnements avec proxy, le middleware de proxy de confiance
([Middleware](middleware.md)) lit les en-têtes `X-Forwarded-*` et met à
jour l'URL de la requête avant qu'elle n'atteigne votre handler. Il n'y
a rien que `forceScheme` puisse redéfinir - `APP_URL` dit déjà quel est
le schéma.

Ce qui atterrit bien ici, c'est la forme visible que les consommateurs
utilisent, avec les mêmes noms de forme Laravel là où ils se transposent
proprement. L'élagage est intentionnel, pas un oubli.

## Suivant

- [Routage](routing.md) - déclarer des routes, les nommer, les groupes
  de routes, le routage de ressource et toute la surface de
  correspondance par méthode
- [Réponses](responses.md) - `Redirect::route`,
  `Redirect::signed_route`, `Redirect::back` et le reste de la famille
  de helpers de redirection qui consomme la génération d'URL
- [Hachage](hashing.md) - le cycle de vie d'`APP_KEY`, la rotation des
  clés et le trousseau partagé qui soutient la signature d'URL aux côtés
  du chiffrement
- [Flux d'authentification](auth-flows.md) - les usagers en production
  des URL signées : réinitialisation de mot de passe, vérification
  d'e-mail et cookies « se souvenir de moi »
- [Requêtes](requests.md) - `Request::path`, `Request::query`,
  `Request::route_is` et l'envers de chaque helper de ce chapitre
