# Sessions

La session est le sac clé/valeur propre à chaque utilisateur qui survit
d'une requête à l'autre sur le même navigateur. Suprnova livre d'origine
un driver adossé à la base de données, le câble via `SessionMiddleware`
et expose la session active à travers deux fonctions libres -
`session()` pour les lectures, `session_mut()` pour les écritures.
Utilisez-la chaque fois qu'une valeur doit survivre à une requête sans
pour autant être quelque chose que l'URL ou un JWT devrait transporter.

## Comment une requête voit la session

`SessionMiddleware` s'exécute à chaque requête et fait cinq choses, dans
l'ordre :

1. Lit l'id de session et l'horodatage de la dernière mise à jour
   d'activité réussie depuis le cookie `suprnova_session` (chiffré en
   AES-256-GCM). Les cookies altérés, indéchiffrables ou malformés sont
   traités comme absents.
2. Charge `SessionData` depuis le magasin uniquement quand un cookie
   valide nomme une session. Les requêtes sans cookie démarrent sur une
   session en mémoire vierge et ne provoquent pas d'échec de lecture en
   base garanti. Un cookie dont la ligne n'existe plus est effacé sans
   recréer de ligne vide. Une erreur de lecture du magasin journalise un
   `warn!` et laisse passer une requête sans état, mais une mutation
   dans le handler échoue alors de manière fermée plutôt que d'écraser
   un état stocké inconnu.
3. Fait vieillir les données flash : `_flash.old.*` est abandonné,
   `_flash.new.*` est renommé en `_flash.old.*`. Après cette étape, tout
   ce que la requête précédente a mis en flash est lisible ; tout ce que
   cette requête met en flash sera lisible la fois suivante.
4. Lie la session dans un emplacement task-local pour la durée du
   handler. `session()` et `session_mut()` consultent cet emplacement.
5. Après le retour du handler, persiste l'état de session modifié ou une
   mise à jour d'expiration glissante bornée, n'attache un cookie
   chiffré de remplacement qu'après une écriture réussie, et vide les
   cookies hors bande en attente (par exemple un cookie « se souvenir de
   moi » fraîchement renouvelé). Une requête sans cookie et non modifiée
   ne fait aucune E/S sur le magasin de sessions et ne reçoit aucun
   cookie de session.

L'étape 5 porte une garantie de sûreté qui mérite d'être isolée : **si
la session a été modifiée pendant cette requête et que l'écriture dans
le magasin échoue, la réponse est remplacée par un 500.** Retourner le
succès du handler reviendrait à remettre au client un cookie pour un
état que la base n'a jamais enregistré - la requête suivante chargerait
une session vide et la mutation (connexion, rotation CSRF, flash)
disparaîtrait silencieusement. Les requêtes en lecture seule qui
n'échouent que sur une mise à jour de `last_activity` arrivée à échéance
journalisent un `warn!`, gardent le cookie existant et passent.

## Lire la session

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // première visite
    }
}
```

`session()` clone la `SessionData` courante. Retourne `None` en dehors
d'une portée de requête (un test unitaire qui n'a pas installé le
middleware, une sous-commande CLI). Pour une valeur typée, `get::<T>`
désérialise depuis le JSON sous-jacent ; sur une clé absente ou un
mauvais type, vous obtenez `None` et aucune panique.

## Écrire dans la session

`session_mut` prend une fermeture qui reçoit `&mut SessionData` :

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

La fermeture est synchrone - les gardes du verrou sous-jacent sont
libérées avant tout `.await`, si bien que cela se compose à l'intérieur
de handlers async sans tenir le verrou à travers les suspensions. Tout
ce que vous sérialisez doit implémenter `Serialize` ; la
désérialisation dans `get` exige `DeserializeOwned`.

La forme en fermeture (plutôt qu'un retour de garde) est délibérée. Dans
Tokio, une future peut reprendre sur un thread de travail différent de
celui où elle a démarré, donc la session doit vivre dans un emplacement
`task_local!` et être empruntée à travers une section critique liée à
une portée. La forme `|s|` rend cette frontière explicite et vous
empêche de tenir accidentellement une garde de mutex à travers un
`.await`.

## Données flash

Les valeurs flash sont visibles pendant **une** requête suivante, puis
disparaissent. Le motif habituel : un contrôleur écrit un flash,
retourne une redirection, et la page suivante rend le flash.

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

À la requête suivante :

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` retire la valeur au moment où il la retourne. Pour la
variante qui lit sans consommer, utilisez
`get::<String>("_flash.old.status")`, mais c'est la forme consommatrice
que veulent d'ordinaire les contrôleurs.

Toute la surface flash de Laravel est disponible :

- `flash(key, value)` - écrit pour la requête suivante
- `now(key, value)` - écrit pour la requête courante seulement
- `reflash()` - remet en flash, pour un tour de plus, tout ce qui est
  visible actuellement
- `keep(&["k1", "k2"])` - remet en flash un sous-ensemble précis
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - le sac de
  saisie de formulaire qu'utilisent `Redirect::with_input` et les
  helpers `old()`

## Régénérer et invalider

Après un changement d'identifiants (connexion, réinitialisation de mot
de passe, validation 2FA), vous faites tourner l'id de session pour
qu'un id fixé avant le changement ne soit plus valide :

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // nouvel id, mêmes données
regenerate_csrf_token();        // nouveau token CSRF, id et données inchangés
```

Pour effacer entièrement la session (déconnexion) :

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // efface les données + forge un token CSRF neuf
```

Pour un événement de sécurité qui doit révoquer toutes les sessions d'un
utilisateur (réinitialisation de mot de passe ailleurs, récupération de
compte, déconnexion forcée par un administrateur) :

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

Cela enveloppe `SessionStore::destroy_for_user` sur le
`DatabaseSessionDriver` par défaut du framework. Si vous avez lié un
magasin personnalisé, appelez `destroy_for_user` directement dessus.

## Helpers d'authentification

`auth_user_id()` retourne l'id de l'utilisateur actuellement authentifié
(en consultant d'abord l'état d'authentification à portée de requête,
puis en se repliant sur le champ persisté en session) :

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

Vous pilotez normalement l'authentification à travers la façade
[Auth](authentication.md) - `Auth::login`, `Auth::logout`,
`Auth::user()`. Les helpers de session sont la couche de bas niveau sur
laquelle ces façades reposent ; recourez-y quand vous devez inspecter la
session brute ou quand vous implémentez votre propre guard.

## Autres opérations

L'API `SessionData` reflète la surface `Store` de Laravel :

| Méthode | Ce qu'elle fait |
|---|---|
| `get::<T>(key)` | lecture typée |
| `put(key, value)` | écriture typée |
| `forget(key)` | retire une seule clé |
| `forget_many(&[..])` | retire plusieurs clés |
| `flush()` | efface toutes les données (garde l'id) |
| `has(key)` / `missing(key)` | test de présence |
| `has_any(&[..])` / `has_all(&[..])` | présence en lot |
| `all()` | emprunte la map sous-jacente |
| `only(&[..])` / `except(&[..])` | clones filtrés |
| `pull::<T>(key)` | lire-et-oublier en un seul coup |
| `push(key, value)` | ajoute à une valeur de type tableau |
| `increment(key, n)` / `decrement(key, n)` | compteurs entiers |
| `remember::<T>(key, \|\| default())` | lire, ou calculer puis poser |
| `replace(&[(k, v), ..])` | vider puis poser en lot |
| `put_many(&[(k, v), ..])` | pose en lot avec fusion |
| `previous_url()` / `set_previous_url(url)` | ce que lit `Redirect::back` |
| `password_confirmed()` / `password_confirmed_at()` | horodatage « l'utilisateur vient de confirmer son mot de passe » |

Recourez-y à l'intérieur de `session_mut` pour les opérations de
mutation, et à `session()` pour les lectures. L'emplacement
`previous_url` est peuplé automatiquement par le middleware sur les
réponses GET HTML réussies, si bien que `redirect()->back()` fonctionne
sans que vous ayez quoi que ce soit à faire.

## Configuration

Configurez les sessions par variables d'environnement -
`SessionConfig::from_env` les lit à l'amorçage :

```env
# Durée de vie en minutes. Pilote à la fois le TTL de la ligne et le Max-Age du cookie.
SESSION_LIFETIME=120

# Secondes minimales entre deux écritures d'expiration glissante (5 minutes par défaut).
# À l'exécution, cette valeur est plafonnée sous la durée de vie de la session.
SESSION_TOUCH_INTERVAL=300

# Cadence supervisée de collecte des lignes expirées, en secondes (1 heure par défaut).
SESSION_GC_INTERVAL=3600

# Nom du cookie chez le client.
SESSION_COOKIE=suprnova_session

# Attributs du cookie
SESSION_SECURE=true          # exige HTTPS ; LA VALEUR PAR DÉFAUT EST true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # facultatif ; non défini = hôte seul
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_PARTITIONED=false    # activation de CHIPS
SESSION_EXPIRE_ON_CLOSE=false # true → omet Max-Age, le navigateur l'abandonne à la fermeture

# Connexion BD nommée pour le magasin de sessions (facultatif)
SESSION_CONNECTION=sessions

# Durée de vie du token/cookie « se souvenir de moi », en minutes (30 jours par défaut)
REMEMBER_LIFETIME=43200
```

Quelques valeurs par défaut méritent d'être signalées :

- **`SESSION_SECURE` vaut `true` par défaut.** Des sessions transmises
  en HTTP simple constitueraient un risque de fuite d'identifiants, donc
  le flag secure est actif par défaut. Pour le développement local en
  HTTP, mettez `SESSION_SECURE=false` dans votre `.env` local.
- **`HttpOnly` est toujours actif.** Il n'existe aucun réglage pour le
  désactiver - exposer le cookie de session à JavaScript sacrifie la
  protection principale contre le XSS, et il n'y a aujourd'hui aucune
  raison légitime de le vouloir.
- **`SameSite` vaut `Lax` par défaut.** `Strict` bloque la session sur
  la plupart des navigations GET intersites (y compris les liens de
  retour depuis un e-mail) ; `Lax` est d'ordinaire la bonne réponse.

Pour une configuration programmatique, utilisez le builder fluide :

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1 heure
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```

## Le câblage

`SessionMiddleware` est installé comme middleware global dans le
bootstrap de votre application. L'ordre des middleware compte : la
session doit venir avant [CSRF](csrf.md), puisque CSRF lit le token
propre à la session.

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` enregistre aussi le superviseur de nettoyage configuré.
    // Utilisez `SessionMiddleware::new(config)` si vous préférez
    // planifier le nettoyage vous-même via `Schedule`.
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` enregistre une tâche de nettoyage
[supervisée](supervisors.md) qui appelle `gc()` toutes les
`SESSION_GC_INTERVAL` (une fois par heure par défaut). La variante
`install_with_gc(config, interval).await` prend un intervalle
personnalisé ; `new(config)` saute la tâche de nettoyage (utile si vous
préférez appeler `gc()` depuis une entrée de
[Schedule](scheduling.md)). La tâche supervisée participe à la vidange
d'arrêt du framework, donc la boucle de nettoyage se termine proprement
sur `Ctrl-C` / `SIGTERM` au lieu d'être avortée de force.

Des points de terminaison d'exploitation protégés peuvent exposer
l'état du collecteur sans interroger la table des sessions :

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

Pour utiliser un magasin qui n'est pas une base de données - pour les
tests, ou pour un driver adossé à Redis que vous écrivez vous-même -
implémentez `SessionStore` et passez-le via `with_store` :

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

## La table `sessions`

Le driver par défaut attend une table `sessions` de cette forme
(l'entité SeaORM dans `framework/src/session/driver/database.rs` fait
autorité) :

| Colonne | Type | Notes |
|---|---|---|
| `id` | VARCHAR PK | id de session alphanumérique en minuscules, de 40 caractères |
| `user_id` | VARCHAR NULL | id d'utilisateur authentifié (chaîne, accepte les id opaques) |
| `payload` | TEXT | map des données de session sérialisée en JSON |
| `csrf_token` | VARCHAR | token CSRF propre à la session |
| `last_activity` | TIMESTAMP | dernier accès ; pilote l'expiration et le nettoyage |

Deux index sont livrés avec la table : `idx_sessions_user_id` (pour
`destroy_for_user`) et `idx_sessions_last_activity` (pour `gc()`).

Une application scaffoldée inclut une migration `create_sessions_table`
conforme à cette forme. Si vous apportez vos propres migrations,
reproduisez exactement les noms de colonne - SeaORM les résout
positionnellement et une colonne renommée ne correspondra pas.

### Pourquoi Suprnova diverge

Deux endroits où Laravel a fait un choix à forme PHP que Tokio nous
permet de faire autrement :

**Le nettoyage des sessions expirées.** Laravel joue une loterie 2/100 à
chaque requête : chaque requête a 2 % de chances de déclencher le
nettoyage de session en ligne. Cela marche en PHP parce que chaque
requête engendre de toute façon un processus neuf. Sur Tokio, nous avons
des workers de longue durée, donc `SessionMiddleware::install`
enregistre une unique tâche [supervisée](supervisors.md) qui appelle
`gc()` à intervalle fixe. Aucun surcoût par requête, aucune surprise
probabiliste - une planification explicite au lieu d'une loterie, et la
boucle de redémarrage du superviseur rattrape les paniques, si bien
qu'un seul nettoyage défaillant ne tue pas le démon.

**`session_mut` en forme de fermeture.** Laravel vous remet
`$request->session()` et vous laisse appeler des méthodes dessus. Nous
ne le faisons pas, parce que les handlers de Suprnova sont des futures
et qu'une future peut reprendre sur un thread de travail différent de
celui où elle a démarré. La session vit dans un emplacement
`task_local!` de Tokio, ce qui veut dire que l'accès emprunté doit se
produire à l'intérieur d'une portée. La forme en fermeture rend cette
portée explicite et empêche statiquement l'erreur consistant à tenir une
garde de mutex à travers un `.await`.

**Échec fermé sur les écritures d'une session modifiée.** Une mise à
jour d'activité bornée qui échoue journalise un `warn!` et laisse passer
la requête avec son cookie existant (l'état visible par l'utilisateur
est intact). L'échec d'écriture d'une session *modifiée* - connexion,
flash, rotation CSRF - retourne 500. Remettre silencieusement au client
un cookie pour un état que le magasin n'a jamais enregistré ferait
disparaître une connexion « réussie » dès la requête suivante ; mieux
vaut faire remonter l'échec de manière visible.

## Suivant

- [Authentification](authentication.md) - `Auth::login`, les guards, la chaîne de fournisseurs d'utilisateurs
- [Flux d'authentification](auth-flows.md) - réinitialisation de mot de passe, 2FA, limitation des attaques par force brute, se souvenir de moi
- [CSRF](csrf.md) - comment le token CSRF de la session est vérifié sur les écritures
- [Middleware](middleware.md) - écrire votre propre middleware qui lit ou écrit la session
- [Cycle de vie des requêtes](lifecycle.md) - où `SessionMiddleware` se situe dans la chaîne
