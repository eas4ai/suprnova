# Variables d'environnement

Ceci est la liste auditée de chaque variable d'environnement que le
framework Suprnova lit à l'exécution, regroupée par le sous-système
qui la consulte. Chaque entrée a été validée contre le code source du
framework - les valeurs par défaut, les types et le comportement
reflètent ce que le code fait réellement, pas ce que le `.env` de
démarrage livre par hasard.

La liste couvre aussi les variables que le binaire CLI `suprnova` lit
(serveur de dev, worker SSR) puisqu'elles apparaissent dans le `.env`
de démarrage et que les lecteurs les chercheront ici.

Voir [Configuration](configuration.md) pour les règles de chargement
(`.env` → `.env.<environment>` → env du processus), les helpers
`env*` (`env`, `env_required`, `env_optional`), et le motif
d'enregistrement typé `Config::*`.

## Conventions

- **Défaut** - la valeur que le framework utilise quand la variable
  n'est pas définie. `aucun` signifie qu'il n'y a pas de défaut ; le
  framework échoue soit à l'amorçage, soit retombe sur un défaut de
  feature (par ex. le driver `Memory`), soit traite la valeur comme
  `None`.
- **Type** - le type Rust dans lequel la variable est analysée. Les
  valeurs `bool` acceptent `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`
  (insensible à la casse). Les valeurs hors intervalle ou non
  analysables pour les réglages typés du framework sont bornées
  (workflow), loguées en `warn!` puis remplacées par le défaut
  (`env()` / `env_optional()` laxistes), ou font échouer l'amorçage
  (`try_from_env` strict).
- **Requis** - `boot` signifie que le framework refuse de démarrer
  sans elle dans les environnements listés. `driver` signifie qu'elle
  n'est requise que quand le driver parent est sélectionné (par ex.
  `MAIL_SES_REGION` n'a pas d'importance sauf si
  `MAIL_DRIVER=ses`). Tout le reste est facultatif.

Là où un `.env` de démarrage livre une clé que le framework ne lit
jamais (`MAIL_FROM_ADDRESS`, `FILESYSTEM_DISK`), c'est signalé à la
fin de ce chapitre.

## Application

La famille `APP_*` est l'identité et la racine cryptographique du
framework. Ce sont les variables que chaque app Suprnova définit ; le
reste du fichier devient pertinent au fur et à mesure que vous optez
pour des sous-systèmes.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | Nom de l'application. Utilisé comme émetteur TOTP (2FA), le royaume `WWW-Authenticate` du HTTP Basic, le branding du sujet des e-mails, et les champs de journal structuré. |
| `APP_ENV` | `local` | `String` | Pilote `Environment::detect()` et la recherche `.env.<suffix>`. Alias reconnus (insensible à la casse) : `local`, `development`/`dev`, `staging`/`stage`/`stg`, `production`/`prod`, `testing`/`test`. Toute autre valeur est préservée comme `Environment::Custom(...)` avec la casse d'origine. |
| `APP_DEBUG` | conscient de l'env (voir Requis) | `bool` | Pages d'erreur détaillées + logs supplémentaires. Le défaut est `true` en `local`/`development`/`testing` et `false` partout ailleurs (y compris `staging`, `production`, et tout environnement personnalisé non reconnu). Une valeur explicite l'emporte toujours ; une valeur non analysable retombe sur le défaut conscient de l'env avec un `warn!`. La variante stricte `try_from_env` interrompt l'amorçage en cas d'échec d'analyse. |
| `APP_URL` | `"http://localhost:8765"` (AppConfig) / `"http://localhost"` (repli d'URL) | `String` | URL de base pour la génération d'URL absolues, les URL signées, et les redirections Inertia. Les barres obliques finales sont retirées à la lecture. |
| `APP_KEY` | aucun - requis en non-dev | `String` (base64-url sans padding, 32 octets) | Clé AES-256-GCM pour `Crypt`, les sessions chiffrées, les curseurs de pagination, les URL signées, et tout autre chemin de chiffrement au repos. L'amorçage **échoue de façon fermée** quand elle est manquante ou malformée hors de `local`/`development`/`testing`. Générez-la avec `suprnova key:generate`. |
| `APP_KEY_PREVIOUS` | aucun | `String` (clés base64 séparées par des virgules, 8 max) | Clés précédentes séparées par des virgules utilisées durant la rotation. `Crypt::decrypt` essaie d'abord l'`APP_KEY` courante, puis chaque entrée dans l'ordre. Plafond strict de 8 entrées - `crypto::MAX_PREVIOUS_KEYS`. Une entrée à moitié rotée qui échoue à se décoder interrompt l'amorçage. Voir [Chiffrement](encryption.md#key-rotation). |
| `APP_PREVIOUS_KEYS` | aucun | `String` (alias de `APP_KEY_PREVIOUS`) | Alias de compatibilité Laravel accepté pour qu'un `.env` Laravel déposé dans un déploiement Suprnova déchiffre encore gracieusement les données héritées. Quand les deux sont définies avec des valeurs différentes, `APP_KEY_PREVIOUS` gagne avec un `warn!` pour signaler le doublon ; des valeurs identiques sont acceptées silencieusement. |
| `APP_BASE_PATH` | répertoire de travail courant | `Path` | Répertoire racine que le résolveur de chemin utilise pour `config/`, `database/`, `public/`, `storage/`, `resources/`, `lang/`. Utile quand vous exécutez le binaire depuis un répertoire de travail différent de la racine du projet (par ex. une unité systemd, `WorkingDirectory=` ne pointant pas vers la racine du projet). Retombe sur le répertoire de travail, puis sur `.` si le répertoire de travail est indisponible. |
| `APP_TRUSTED_PROXIES` | aucun - allowlist vide | `String` (IP séparées par des virgules) | Adresses de pair TCP dont les en-têtes `X-Forwarded-*` / `X-Real-IP` peuvent être crus par `Request::ip()` et les accesseurs d'hôte / schéma / port. **Vide par défaut, si bien que les en-têtes de proxy sont ignorés et que le pair TCP gagne toujours** - voir la note ci-dessous avant de déployer derrière un proxy. Une entrée non analysable fait échouer l'amorçage (`try_from_env`). |
| `AUTH_GUARD` | `"web"` | `String` | Nom du guard par défaut lu par `Auth::*`. Reflète Laravel - seul le défaut est sélectionnable par env ; les guards nommés vivent dans le code via `AuthConfig::guard(name, …)`. |

Deux autres variables `APP_*` - `APP_LOCALE` et
`APP_FALLBACK_LOCALE` - sont lues par le sous-système de localisation
plutôt que par `AppConfig`, elles sont donc listées sous
**Localisation** ci-dessous.

### Derrière un proxy inverse, définissez `APP_TRUSTED_PROXIES`

Ignorer les en-têtes de proxy est le défaut sûr - `X-Forwarded-For`
est fourni par l'appelant et lui faire confiance sans condition
permet à quiconque de revendiquer n'importe quelle adresse. Mais dès
qu'un proxy terminant se trouve devant vous (nginx, Traefik, un ALB,
Cloudflare), le pair TCP est *le proxy*, sur chaque requête, et
laisser ceci non défini ne fait pas que perdre l'adresse du client :

- **Les limites de débit par IP s'effondrent en un seul compartiment.**
  La clé par défaut de `ThrottleRequestsMiddleware` est
  `request.ip()`, donc `ThrottleRequestsMiddleware::with(20, 1,
  "login")` arrête de signifier « 20 tentatives de connexion par
  client par minute » et commence à signifier 20 *au total, tous
  confondus*. C'est à la fois plus faible (aucun budget par
  attaquant) et activement dangereux : un seul appelant peut dépenser
  le quota et verrouiller chaque utilisateur légitime hors du
  formulaire de connexion. Voir [Limitation de débit](rate-limiting.md).
- `Request::host()`, `scheme()` et `port()` retombent sur la
  connexion plutôt que sur `X-Forwarded-Host` / `-Proto` / `-Port`, si
  bien que les URL absolues générées peuvent nommer l'adresse et le
  schéma internes au lieu du public.

Listez les adresses depuis lesquelles les sauts du proxy vous
atteignent - pas celle du client :

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

Rien ne détecte cela pour vous : une app derrière un proxy avec la
variable non définie a l'air saine, sert correctement, et limite
silencieusement le débit de tout le monde comme un seul utilisateur.

### Matrice de nécessité d'`APP_KEY`

| Environnement | `APP_KEY` requise à l'amorçage |
|---|---|
| `local` | non (génère une clé éphémère si manquante) |
| `development` | non |
| `testing` | non |
| `staging` | oui - l'amorçage sort avec un code non nul et un message de remédiation |
| `production` | oui |
| `Custom(...)` | oui - tout ce qui n'est pas dans la liste sûre est traité comme la production pour ce contrôle |

## Serveur

L'écouteur HTTP et les plafonds de corps de requête.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | Adresse de liaison. Définissez `0.0.0.0` pour exposer hors de l'interface loopback (par ex. dans des conteneurs). |
| `SERVER_PORT` | `8765` | `u16` | Port de liaison. L'analyse laxiste avertit et retombe sur le défaut ; le `try_from_env` strict interrompt l'amorçage sur une faute de frappe. |
| `SERVER_MAX_BODY_SIZE` | `8388608` (8 Mio) | `usize` (octets) | Taille maximale de corps de requête, globale au processus. Les redéfinitions par `FormRequest::max_body_bytes` s'appliquent quand même sur des points de terminaison individuels. La valeur configurée est câblée dans le plafond global durant `Server::from_config`. |
| `SERVER_MAX_CONNECTIONS` | non défini (illimité) | `usize` | Plafond des connexions TCP actives simultanées. Non défini signifie aucun plafond. Une valeur nulle ou non analysable retombe sur un `10000` fini avec un avertissement plutôt que de silencieusement revenir à illimité - une limite ratée reste une demande de limite. |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64` (secondes) | Délai limite pour lire l'en-tête complet d'une requête. La mitigation slowloris. Zéro est traité comme invalide, pas comme « désactiver », et retombe sur le défaut. Ne s'applique pas aux connexions WebSocket/SSE établies. |
| `SERVER_HEALTH_READINESS_TOKEN` | non défini (préparation publique) | `String` | Secret partagé requis pour atteindre `/_suprnova/health/ready` et `/_suprnova/health?db=true`, envoyé comme `X-Suprnova-Health-Token`. Sans lui, ces chemins répondent 404, indistinguables de tout chemin non routé ; la vivacité reste publique. Voir [Déploiement](deployment.md#health-check). |

## Base de données

URL de connexion et réglage du pool sqlx. `DATABASE_URL` est requise
pour toute sous-commande qui touche la base de données (`migrate*`,
`db:sync`, `db:seed`, `queue:work` avec `QUEUE_DRIVER=database`,
`workflow:work`, le magasin de session en base) et pour `serve` quand
l'application a des migrations enregistrées.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `DATABASE_URL` | aucun - requise quand des migrations existent | `String` | URL de connexion. Le schéma sélectionne le driver : `sqlite://path`, `postgres://...` / `postgresql://...`, `mysql://...`, `mariadb://...`. Le framework crée automatiquement le répertoire parent pour les chemins SQLite. `serve` ignore entièrement la connexion à la base de données quand le `Migrator` configuré n'a aucune migration. |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | Plafond du pool sqlx. |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | Plancher du pool sqlx (maintenu chaud). |
| `DB_CONNECT_TIMEOUT` | `30` (secondes) | `u32` | Combien de temps sqlx attendra une connexion initiale avant d'échouer. |
| `DB_LOGGING` | `false` | `bool` | Quand vrai, sqlx journalise chaque instruction (à utiliser avec parcimonie en production - bavard). |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | Quand vrai, une auto-migration en échec durant l'amorçage de `serve` est journalisée mais n'interrompt pas. Le défaut échoue de façon fermée : l'amorçage sort avec un code non nul plutôt que de démarrer contre un schéma partiellement migré. Passez `--no-migrate` pour ignorer entièrement l'auto-migration. |

## Session

Attributs de cookie et durée de vie pour le sous-système de session.
Notez que `SESSION_SECURE` a pour défaut **`true`** - sûr pour la
production par défaut ; ne le désactivez que pour le développement
HTTP local.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `SESSION_LIFETIME` | `120` (minutes) | `u64` | Durée de vie de session en minutes. Analysée via `env_optional` ; retombe silencieusement sur le défaut si non analysable. |
| `SESSION_TOUCH_INTERVAL` | `300` (secondes) | `u64` | Cadence minimale de persistance de l'expiration glissante. L'application à l'exécution la plafonne à la moitié de la durée de vie de session. |
| `SESSION_GC_INTERVAL` | `3600` (secondes) | `u64` | Cadence du collecteur supervisé de sessions expirées installé par `SessionMiddleware::install`. |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | Nom du cookie de session. |
| `SESSION_PATH` | `"/"` | `String` | Attribut `Path=` du cookie. |
| `SESSION_DOMAIN` | non défini | `String` | Attribut `Domain=` du cookie. Laissez non défini pour des cookies host-only (le défaut le plus sûr pour la plupart des apps). |
| `SESSION_SECURE` | `true` | `bool` | Attribut `Secure` du cookie. Défaut `true` ; définissez `false` seulement en développement HTTP local. `cookie_http_only` est toujours `true` et n'est pas configurable par env. |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | Attribut `SameSite`. Accepte `Strict`, `Lax`, `None` (insensible à la casse). |
| `SESSION_COOKIE_PREFIX` | non défini | `String` (`__Host-` / `__Secure-`) | Préfixe appliqué aux noms réseau de la session et du cookie « se souvenir de moi ». `Config::init` valide la valeur et ses contraintes `SESSION_DOMAIN` / `SESSION_PATH` à l'amorçage ; les combinaisons invalides échouent avant de servir. |
| `SESSION_PARTITIONED` | `false` | `bool` | Émet l'attribut de cookie `Partitioned` / CHIPS pour les cookies isolés tiers. |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | Quand vrai, abandonne `Max-Age` pour que le navigateur supprime le cookie à la fermeture (sémantique cookie de session). |
| `SESSION_CONNECTION` | non défini | `String` | Connexion DB nommée pour le magasin de session. Non défini signifie la connexion par défaut. |
| `REMEMBER_LIFETIME` | `43200` (30 jours, en minutes) | `u64` | Durée de vie du cookie/token « se souvenir de moi », en minutes. |

## Localisation

Les trois variables `APP_*` que le sous-système de localisation lit.
Tout le reste à son sujet - la chaîne de détection, la clé de session
et le nom de cookie qu'il consulte, les marques d'isolation Unicode -
est de la configuration au niveau du code sur `LocalizationConfig`,
pas de l'env. Voir [Localisation](localization.md).

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String` (BCP-47) | Locale utilisée quand la chaîne de détection (session → cookie → `Accept-Language`) ne trouve rien. Aussi la locale depuis laquelle `suprnova generate-types` extrait les clés de message pour `lang-keys.ts`. Une valeur qui n'est pas un identifiant BCP-47 valide fait échouer l'amorçage plutôt que de retomber silencieusement sur un défaut. |
| `APP_FALLBACK_LOCALE` | `"en"` | `String` (BCP-47) | Locale consultée quand une clé manque dans le catalogue de la locale courante. Une clé manquante des deux se rend comme la clé elle-même plus un `warn!` unique ; `Lang::try_get` retourne `Err` à la place. Même analyse stricte qu'`APP_LOCALE`. |
| `APP_LOCALE_PARENTS` | aucun - map vide | `String` (paires `enfant=parent` séparées par des virgules, BCP-47 de chaque côté) | Parents de repli par locale consultés avant `APP_FALLBACK_LOCALE`, par ex. `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`. La chaîne de repli de `Lang` les parcourt transitivement, et `FluentTranslator` aplatit la chaîne de parents configurée de chaque locale dans son catalogue servi. Une paire malformée, une locale invalide, un enfant nommé plus d'une fois, ou un cycle (y compris une locale se nommant comme son propre parent) fait échouer l'amorçage plutôt que de se dégrader à l'exécution. Voir [Chaînes de repli](localization.md#fallback-chains). |

Les catalogues eux-mêmes sont des fichiers, pas de l'env :
`lang/<locale>/*.ftl` sous `APP_BASE_PATH`. Un répertoire `lang/`
manquant n'est pas une erreur - l'app démarre avec le catalogue de
validation anglais embarqué du framework.

## Cache

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String` (`memory`/`in-memory`/`inmemory`, `redis`) | Sélectionne la cible d'amorçage. Memory garde tout dans le processus ; Redis requiert `REDIS_URL` et fait échouer l'amorçage s'il est inaccessible. Des valeurs inconnues font échouer l'amorçage avec une erreur claire. |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL de connexion Redis (consultée seulement quand `CACHE_DRIVER=redis`). |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | Préfixe de clé pour les entrées de cache (évitement de collision pour un Redis partagé). |
| `CACHE_DEFAULT_TTL` | `3600` (secondes) | `u64` | TTL par défaut en secondes. `0` signifie « pas d'expiration ». Appliqué à `Cache::put(None)` / `Cache::tags_put(None)` ; `Cache::forever` et `Cache::remember_forever` contournent toujours. |

## File d'attente

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String` (`memory`, `redis`, `database`) | Backend de file d'attente actif. Les valeurs inconnues journalisent un `warn!` et retombent sur memory. |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL Redis (requise par le driver quand `QUEUE_DRIVER=redis`). |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | Clé de Redis Stream utilisée pour le fan-out. |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | Nom du groupe de consommateurs. |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | Nom du consommateur au sein du groupe. À définir par worker pour des workers parallèles. |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | Combien de temps un job réclamé reste invisible avant qu'un autre consommateur puisse le réclamer. Faites correspondre ceci à votre job le plus lent. |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | Nom de table pour le driver database. Validé comme un identifiant SQL - une valeur malformée fait échouer l'amorçage, pas la composition SQL. Requise par le driver quand `QUEUE_DRIVER=database` ; le driver requiert aussi que `DB::init()` se soit déjà exécuté. |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | Table dans laquelle le magasin de lettre morte écrit. Lié automatiquement quand `QUEUE_DRIVER=database` - `queue:retry` la lit et `Queue::retry_failed` en a besoin, donc la table fait partie du contrat de ce driver. Non utilisée par `memory` (éphémère par construction) ni par `redis` (aucune table où écrire). À la différence de `QUEUE_DB_TABLE`, un identifiant malformé ici ne fait **pas** échouer l'amorçage : il journalise en `error!` et ne lie aucun magasin, si bien que les jobs mis en lettre morte sont journalisés en entier plutôt que persistés. Récupérable à la main, mais pas par `queue:retry`. |

## Planification

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | non défini | `bool`-esque | Reconnaît qu'une tâche marquée `on_one_server()` élit un leader à travers un cache **par processus**. Cette élection n'est partagée qu'autant que le cache derrière elle, donc en production `CACHE_DRIVER=memory` plus une tâche mono-serveur est un échec dur de l'amorçage qui nomme les tâches fautives, plutôt qu'une dégradation silencieuse vers « chaque réplique l'exécute ». Ne définissez ceci que là où le déploiement exécute véritablement un seul planificateur ; sinon définissez `CACHE_DRIVER=redis`. Voir [Planification](scheduling.md). |

## Flux de travail

Le worker `#[workflow]` à état, de longue durée. Toutes les valeurs
sont bornées à des minimums sûrs plutôt qu'honorées à l'aveugle - un
`WORKFLOW_CONCURRENCY=0` mettrait le sémaphore du worker en pause pour
toujours, donc le framework avertit et borne au lieu d'accepter une
configuration manifestement cassée.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | Nombre maximal d'exécutions de flux de travail concurrentes par processus worker. Borné à `>= 1`. |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` (ms) | `u64` | À quelle fréquence le worker sonde les flux de travail nouvellement dus. |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` (secondes) | `u64` | Délai de récupération pour une ligne de flux de travail réclamée dont le worker est mort. |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | Nombre maximal de tentatives par exécution de flux de travail avant qu'elle ne soit marquée en échec. Borné à `>= 1`. |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | Backoff linéaire par tentative. Borné à `>= 0` - un backoff négatif planifierait des réessais dans le passé et produirait une récupération en boucle serrée. |

## E-mail

`MAIL_DRIVER` a pour défaut **`log`** - le mail sortant s'imprime dans
le subscriber de traçage configuré plutôt que d'atteindre le réseau.
Basculez vers `memory` dans les tests, `file` pour des aperçus `.eml` que
vous pouvez ouvrir dans un client de messagerie, et `smtp`/`ses`/etc. en
production. Les clés/tokens spécifiques au fournisseur ne sont requis que
quand ce driver est sélectionné ; une valeur de driver inconnue journalise un
`warn!` et retombe sur `log`.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String` (`log`, `memory`, `file`, `smtp`, `ses`, `sendgrid`, `mailgun`, `postmark`, `resend`) | Sélectionne la cible d'amorçage. |
| `MAIL_FROM` | aucun - requis par les façades auth-flow | `String` | Adresse d'expéditeur par défaut pour les façades auth-flow (`EmailVerification`, `PasswordReset`, `TwoFactor`). Requise pour ces chemins ; en son absence, cela échoue au site d'appel plutôt que de silencieusement retomber sur un placeholder qui casserait DMARC/SPF. |
| `MAIL_FROM_NAME` | non défini | `String` | Nom d'affichage facultatif pour le `From` auth-flow (depuis la **0.5.9**). Quand défini, l'en-tête se rend `Name <MAIL_FROM>` ; `MAIL_FROM` reste une adresse nue. Lu au moment de l'envoi, donc s'applique aussi au mail auth-flow mis en file d'attente. |

### Fichier (`MAIL_DRIVER=file`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | Répertoire dans lequel un fichier `.eml` RFC 5322 est écrit par envoi. Jamais purgé. Les chemins absolus sont utilisés tels quels ; les chemins relatifs sont ancrés au répertoire de base de l'application (voir `APP_BASE_PATH`). |

### SMTP (`MAIL_DRIVER=smtp`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | Hôte SMTP. |
| `MAIL_SMTP_PORT` | `587` | `u16` | Port SMTP. |
| `MAIL_SMTP_USER` | non défini | `String` | Nom d'utilisateur SMTP. `MAIL_SMTP_USER` **et** `MAIL_SMTP_PASS` doivent être définis pour un transport chiffré ; sans les deux, la connexion retombe par défaut sur le mode non chiffré local-catcher. Définir exactement l'un des deux avertit à l'amorçage. |
| `MAIL_SMTP_PASS` | non défini | `String` | Mot de passe SMTP. Voir `MAIL_SMTP_USER` pour le comportement en cas d'identifiants partiels. |
| `MAIL_SMTP_ENCRYPTION` | dérivé | `starttls` \| `tls` \| `none` | Comment la connexion est chiffrée. Non défini dérive des identifiants : `starttls` quand les deux sont définis, `none` quand aucun ne l'est. `tls` sélectionne le TLS implicite (port 465). `ssl` et `null` sont acceptés comme alias compatibles Laravel. Une valeur non reconnue fait échouer l'amorçage dans **chaque** environnement - une faute de frappe ne doit pas dégrader vers le texte en clair. |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | non défini | `bool`-esque | La production refuse de démarrer sur une connexion SMTP non chiffrée. Définissez `1`/`true`/`yes`/`on` pour reconnaître le texte en clair - défendable seulement quand le relais n'est atteignable que sur un réseau privé. |

### Postmark (`MAIL_DRIVER=postmark`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | requis par le driver | `String` | Token de serveur Postmark. |
| `MAIL_POSTMARK_ENDPOINT` | défaut Postmark | `String` | Redéfinit le point de terminaison de l'API (régional ou serveur mock). |

### Amazon SES (`MAIL_DRIVER=ses`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | requis par le driver | `String` | Clé d'accès AWS. |
| `MAIL_SES_SECRET_KEY` | requis par le driver | `String` | Clé secrète AWS. |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | Région AWS. |
| `MAIL_SES_ENDPOINT` | défaut AWS pour la région | `String` | Redéfinit le point de terminaison SES (régional ou serveur mock). |

### SendGrid (`MAIL_DRIVER=sendgrid`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | requis par le driver | `String` | Clé API SendGrid. |
| `MAIL_SENDGRID_ENDPOINT` | défaut SendGrid | `String` | Redéfinit le point de terminaison de l'API. |

### Mailgun (`MAIL_DRIVER=mailgun`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | requis par le driver | `String` | Clé API Mailgun. |
| `MAIL_MAILGUN_DOMAIN` | requis par le driver | `String` | Domaine d'envoi Mailgun. |
| `MAIL_MAILGUN_ENDPOINT` | défaut Mailgun | `String` | Redéfinit le point de terminaison de l'API (par ex. UE contre US). |

### Resend (`MAIL_DRIVER=resend`)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | requis par le driver | `String` | Clé API Resend. |
| `MAIL_RESEND_ENDPOINT` | défaut Resend | `String` | Redéfinit le point de terminaison de l'API. |

## Limitation de débit

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String` (`memory`, `redis`) | Sélectionne le backend du limiteur de débit. Hors production, une valeur inconnue journalise un `warn!` et retombe sur memory ; **en production, memory - y compris via une valeur inconnue - fait échouer l'amorçage** sauf si `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` est défini. |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | non défini | `bool`-esque | Reconnaît des compartiments de limitation de débit par processus en production. Exact seulement si vous exécutez exactement un processus : derrière N répliques, chaque quota est effectivement multiplié par N et se réinitialise à chaque déploiement. |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL Redis (requise par le driver quand `RATE_LIMIT_DRIVER=redis`). |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Préfixe de clé dans Redis. |

## Hachage

Driver de hachage de mot de passe et paramètres par algorithme. Des
valeurs invalides retournent une `FrameworkError::param` au premier
hachage, faisant surface immédiatement la mauvaise configuration
plutôt que de silencieusement revenir au défaut.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String` (`bcrypt`, `argon`/`argon2i`, `argon2id`) | Algorithme de hachage actif. Insensible à la casse. |
| `HASH_ROUNDS` | `12` | `u32` | Coût bcrypt (intervalle `4..=31`). Les valeurs hors intervalle échouent avec une erreur claire. |
| `HASH_MEMORY` | `65536` (64 Mio, unités KiB) | `u32` | Mémoire Argon2 en KiB. Minimum `8`. Argon uniquement. |
| `HASH_TIME` | `4` | `u32` | Temps / itérations Argon2. Minimum `1`. Argon uniquement. |
| `HASH_THREADS` | `1` | `u32` | Parallélisme Argon2 (correspond à OWASP / libsodium). Minimum `1`. Argon uniquement. |
| `HASH_VERIFY` | `false` | `bool` | Quand vrai, `verify()` rejette les hachages d'un algorithme différent de `HASH_DRIVER` (retourne `Ok(false)`). Défaut `false` pour que les hachages bcrypt hérités se vérifient encore après un changement de driver, jusqu'à ce qu'ils soient rotés. |

## Flux d'authentification

L'authentification à deux facteurs utilise `APP_NAME` (couverte sous
Application) comme chaîne d'émetteur TOTP - il n'y a pas de variable
d'env `2FA_ISSUER` dédiée. L'émetteur retombe sur `"Suprnova"` quand
`APP_NAME` n'est pas défini.

## Inertia / Frontend

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String` (`svelte`, `react`, `vue`) | Frontend actif. Insensible à la casse. Pilote `Frontend::detect_from_env()`, le point d'entrée Vite par défaut, et l'ordre de recherche d'extension de composant de page à la compilation. Les valeurs inconnues ou non définies retombent sur `svelte`. |

## Mode de maintenance

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String` (`file`, `cache`) | Sélectionne comment l'état `down`/`up` est stocké. `file` écrit dans le chemin de stockage du framework ; `cache` s'appuie sur le driver de cache configuré (utile quand plusieurs instances d'app doivent coordonner l'état de maintenance). Toute autre valeur retombe sur `file`. |

## Événements

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | Plafond des tâches d'écouteur mises en file d'attente concurrentes. Les valeurs `<= 0` ou non analysables retombent sur le défaut. S'applique à `Event::queue` / aux écouteurs mis en file d'attente ; les écouteurs synchrones ne sont pas soumis à cette limite. |

## Journalisation

`LOG_FORMAT` est **conscient de l'environnement** : en production
(`APP_ENV=production`) le défaut est `json` pour la convivialité avec
les agrégateurs de journaux ; partout ailleurs le défaut est `pretty`
pour une sortie locale/dev lisible par un humain. Une valeur explicite
l'emporte toujours.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String` (`error`, `warn`, `info`, `debug`, `trace` - insensible à la casse) | Niveau de filtre tracing-subscriber. |
| `LOG_FORMAT` | conscient de l'env (`json` en production, `pretty` ailleurs) | `String` (`json`, `pretty`) | Format de sortie tracing-subscriber. |

## Observabilité (OpenTelemetry)

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | non défini (télémétrie désactivée) | `String` | Point de terminaison du collecteur OTLP. Quand non défini (ou blanc), les exportateurs ne sont pas installés et le framework continue d'utiliser le subscriber `tracing` standard. |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | Attribut de ressource `service.name` sur chaque enregistrement de span / métrique / journal. |
| `OTEL_SERVICE_VERSION` | `CARGO_PKG_VERSION` à la compilation | `String` | Attribut de ressource `service.version`. |
| `OTEL_SDK_DISABLED` | `false` | `bool` | Interrupteur d'arrêt OTel standard. Quand vrai, les exportateurs ne sont pas installés, indépendamment d'`OTEL_EXPORTER_OTLP_ENDPOINT`. |

## CLI / serveur de dev

Celles-ci sont lues par le binaire CLI `suprnova` (serveur de dev,
worker SSR) plutôt que par le framework à l'exécution - elles
apparaissent dans le `.env` de démarrage ou sont honorées par
`suprnova serve` / `suprnova ssr:*`.

| Var | Défaut | Type | Objet |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | Port sur lequel Vite se lie dans `suprnova serve`. `--frontend-port` en CLI redéfinit. |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | Runtime sous lequel lancer le worker SSR (`suprnova ssr:start`). `--runtime` en CLI redéfinit. |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | Chemin vers le bundle SSR construit. `--bundle` en CLI redéfinit. |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | URL du worker SSR pour `suprnova ssr:check`. `--url` en CLI redéfinit. |

## Sous-systèmes sans variables d'env

Quelques sous-systèmes sont configurés entièrement en code Rust via le
conteneur ou l'enregistrement de service - ils ont **zéro** variable
d'env que le framework lit :

- **Système de fichiers / stockage.** Les disques sont enregistrés
  avec `FilesystemRegistry::add_disk(name, driver)` dans
  `bootstrap()`. Il n'y a pas de variable d'env `FILESYSTEM_DISK` (le
  nom apparaît dans certains fichiers `.env` de démarrage mais n'est
  pas consulté par le framework - voir « Variables que le framework
  ne lit pas » ci-dessous).
- **Diffusion et WebSockets.** Les canaux sont enregistrés avec la
  macro `ws!()` et la configuration `BroadcastHub` en code. Le driver
  lui-même s'appuie sur ce que le `CACHE_DRIVER` configuré
  sélectionne.
- **CORS, CSRF, Idempotence, Délai d'attente.** Configurés via des
  structs builder passées aux constructeurs de middleware dans
  `bootstrap()`. Les défauts sont assez conservateurs pour qu'une app
  typique n'y touche jamais.
- **Magnetar et OAuth.** `MagnetarConfig` est construit dans l'amorçage de
  l'application. Le starter API lit `PASSKEY_RP_ID` et `PASSKEY_RP_ORIGIN`,
  mais le framework lui-même ne les lit pas. Les ID de fournisseur OAuth, les
  secrets, les URL de callback, les portées, les transports et les valeurs de
  policy sont fournis par programmation à travers le registre de fournisseurs
  Magnetar. Les applications peuvent obtenir ces valeurs depuis des variables
  d'environnement ou un gestionnaire de secrets.
- **Recherche vectorielle, Notifications, Paiements, Flags de
  fonctionnalité.** Chacun enregistre des drivers concrets via
  `App::bind` dans `bootstrap()`. Choisissez votre driver en Rust ;
  passez les URL/clés dont il a besoin comme vos propres variables
  d'env.

## Variables que le framework ne lit pas

Le `.env` de démarrage scaffoldé liste quelques clés pour la
commodité de l'auteur humain que le framework ne consulte jamais.
Elles sont documentées ici pour qu'un lecteur qui les cherche ne reste
pas perplexe :

- `MAIL_FROM_ADDRESS` - un placeholder à la Laravel que le framework
  ne consulte jamais. L'adresse d'expéditeur réelle que les façades
  auth-flow utilisent est `MAIL_FROM` (couverte sous Mail). Vos
  propres types `Mailable` peuvent la lire via `env_optional` si vous
  voulez garder le nom Laravel, mais rien dans `suprnova::*` ne le
  fait. (`MAIL_FROM_NAME` **est** lue depuis la 0.5.9 - voir le
  chapitre Mail - elle n'est donc plus listée ici.)
- `FILESYSTEM_DISK` - placeholder pour le nom du disque par défaut.
  Définissez le défaut en code via
  `FilesystemRegistry::set_default(name)` à la place.

## Comment les valeurs sont analysées

Une référence courte pour les trois variantes de helper d'env - voir
[Configuration](configuration.md#direct-env-access) pour le
traitement complet :

| Helper | Comportement si manquante | Comportement si non analysable |
|---|---|---|
| `env(key, default)` | retourne `default` | `warn!` + retourne `default` |
| `env_required(key)` | **panique** | **panique** |
| `env_optional(key)` | retourne `None` | `warn!` + retourne `None` |
| `env_strict(key)` (interne, utilisé par `try_from_env`) | retourne `Ok(None)` | retourne `Err(FrameworkError)` - l'amorçage s'interrompt |

Les variantes strictes (`AppConfig::try_from_env`,
`ServerConfig::try_from_env`) sont ce que `Config::init` appelle,
donc une faute de frappe dans `APP_DEBUG=tru` ou `SERVER_PORT=80a0`
interrompt l'amorçage avec une erreur structurée au lieu de
silencieusement revenir au défaut. Les variantes laxistes existent
pour la population plus large de sites d'appel (y compris
`impl Default`) où un échec d'analyse ne doit pas paniquer.

## Redéfinitions par environnement

Le chargeur lit les fichiers dans cet ordre, chacun redéfinissant le
précédent :

1. `.env`
2. `.env.<environment>` (par ex. `.env.production`, `.env.staging`,
   `.env.testing`, `.env.<custom>` pour `APP_ENV=<custom>`)
3. Env du processus

Cela signifie qu'un déploiement de production conteneurisé peut
livrer un `.env.production` minimal ne redéfinissant que les clés qui
diffèrent de `.env` (noms de driver, URL, matériel de clé), et que
l'env de conteneur réel redéfinit les deux pour les secrets qui ne
devraient jamais atterrir dans un fichier commité.

Voir [Configuration](configuration.md#how-env-loading-works) pour le
comportement exact du chargeur et le suivi `LOADED_KEYS` qui empêche
des valeurs `.env` périmées de se promouvoir dans le palier « vrai env
système » à travers les rechargements.

## Suivant

- [Configuration](configuration.md) - enregistrement typé
  `Config::*`, les helpers `env*`, la détection d'environnement
- [Déploiement](deployment.md) - ce qu'il faut définir en production
- [Chiffrement](encryption.md) - rotation d'`APP_KEY` via
  `APP_KEY_PREVIOUS`
- [Amorçage de l'application](bootstrap.md) - où l'ordre d'amorçage piloté par l'env est établi
