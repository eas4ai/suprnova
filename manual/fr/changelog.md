# Journal des modifications

Un journal lisible, par version, de ce qui a changé dans Suprnova. Chaque
section de version est le compte-rendu de publication de cette version.
Une version est publiée quand son commit de version et le tag
`v<version>` correspondant sont poussés atomiquement. Les plus récentes
en premier.

## Non publié

## 1.3.0 - 2026-08-24

### Sécurité

- **Magnetar encadre désormais les mutations de credential et de session par l'acteur authentifié et l'ère d'auth du compte.** Les écritures de mot de passe, passkey, compte lié, second facteur, session opaque, JWT, remember, OAuth et autorisation d'appareil refusent les acteurs périmés ou révoqués. La première preuve réussie de réinitialisation de mot de passe, de lien magique ou d'e-mail OAuth vérifié sur un compte non vérifié fait avancer l'ère et retire atomiquement les credentials provisoires, les sessions, l'état remember et l'enrôlement TOTP du squatteur. Les comptes vérifiés conservent leurs credentials légitimes pendant une réinitialisation de mot de passe. La vérification d'e-mail exige le propriétaire authentifié du token, et OAuth ne relie jamais automatiquement un compte existant non vérifié à partir du seul e-mail.
- **Un `_previous.url` relatif au protocole ne peut plus produire une redirection ouverte hors origine via `Redirect::back()`, ni à l'écriture ni à la lecture.** `SessionMiddleware` ne persiste plus une URL courante relative au protocole : l'écriture passe par le même assainisseur que `InertiaValidationRedirectMiddleware` utilise pour `Referer`, et un chemin de requête tel que `//host` (ou contenant un octet de contrôle ASCII) n'est jamais enregistré. Sans cela, une route `fallback!` (le schéma standard de shell Inertia/SPA, où tout chemin non trouvé répond `200`) pouvait enregistrer textuellement `GET //evil.test/anything`. `SessionData::previous_url()` applique aussi ce contrôle à chaque **lecture**, afin qu'un cookie de session survivant à une mise à niveau depuis une version antérieure se répare en « rien d'enregistré » au lieu d'être approuvé. Ainsi, ni un ancien cookie empoisonné ni une nouvelle requête malveillante ne peuvent fournir une `Location` hors origine à `Redirect::back()`, `Redirect::refresh()` ou `url::previous()`. Une valeur qui échoue à l'un ou l'autre contrôle est considérée absente, jamais remplacée par une valeur synthétisée, donc une bonne URL précédente n'est jamais écrasée.
- **Le contrôle de `Referer` du pont de redirection de validation Inertia ferme deux autres contournements de même origine.** La cible `303` de `InertiaValidationRedirectMiddleware` ne rejetait qu'un `Referer` commençant littéralement par `//` ou `/\` : `Referer: /<TAB>/evil.test` passait, car l'analyseur d'URL WHATWG retire tabulations et retours à la ligne ASCII avant de comparer les origines. Le contrôle rejette désormais tout octet de contrôle ASCII (C0 ou DEL) dans le candidat. De plus, le repli final  -  le chemin de la requête en échec lorsque ni `Referer` ni l'URL précédente de session n'est exploitable  -  n'était jamais assaini ; une cible HTTP origin-form peut syntaxiquement commencer par `//`. Les deux branches partagent maintenant un contrôle root-relative et replient sur `/` même si le chemin de requête échoue.
- **Le texte chiffré des cookies est maintenant lié à son nom logique de cookie par l'AAD v2 contextualisée.** `Cookie::encrypted` / `Cookie::read_encrypted_for` empêchent une valeur émise pour un emplacement de cookie de se déchiffrer dans un autre, et cette liaison de nom logique rend sûre une modification ultérieure de préfixe filaire `__Host-` / `__Secure-`. La fenêtre de compatibilité sans version essaie v2 sur tout le trousseau, puis v1 sur tout le trousseau ; les cookies existants survivent au déploiement, mais le repli v1 conserve l'ancienne faiblesse de rejeu jusqu'à sa suppression prévue en 1.4.0.
- **Les préfixes de cookies de session et remember-me sont validés au démarrage et appliqués au rendu.** `SESSION_COOKIE_PREFIX=__Host-` exige `Secure`, `Path=/` et aucune valeur `Domain` ; `__Secure-` exige `Secure`. Les combinaisons invalides échouent avant le service, et le moteur de rendu réécrit les en-têtes préfixés invalides plutôt que de laisser les navigateurs les ignorer silencieusement.

### Ajouté

- **L’authentification Suprnova s’exécute désormais sur le moteur interne Magnetar.** La façade `Auth` détenue par le framework conserve les points d’appel existants pour les mots de passe, les liens magiques, les passkeys, OAuth, bearer, le verrouillage, les sessions et le double facteur, tout en supprimant la dépendance à Torii. Le moteur par défaut installe de manière atomique les adaptateurs mot de passe/session et passkey, stocke les baux de livraison du cycle de vie dans la base de données de l’application et partage les identités `i64` `app_users` canoniques de l’application.
- **Un exécuteur de migration d’authentification sensible à la forme couvre désormais les sources Torii, Suprnova web et Suprnova API.** Les simulations associent un id de plan stable à des empreintes durables de lignes et de schéma, ainsi qu’à des décisions d’identité de destination. L’application utilise des imports transactionnels, des journaux de réessais, un nettoyage détenu par la forme et un refus des collisions. MySQL utilise un échange fantôme protégé par une barrière d’écriture avec des journaux de précopie, une parité des lignes et du schéma, des renommages reprenables et une restauration préservant le nettoyage.
- **`MAIL_DRIVER=file` écrit un RFC 5322 `.eml` par message** dans `MAIL_FILE_PATH` (par défaut `storage_path("mail")` ; une valeur relative est ancrée dans le répertoire de base de l'application, non dans le CWD du processus), afin que le mail local puisse être ouvert dans un client de messagerie plutôt que lu dans une ligne de log. Le fichier porte le même surensemble d'en-têtes que SMTP, y compris `X-Priority`, `Importance`, `X-Tag`, `X-Metadata-*` et `Return-Path`. Comme `log` et `memory`, il ne délivre pas : un démarrage de production le refuse sauf si `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **`FrameworkError::External` transporte l'erreur qu'il encapsule.** `FrameworkError::from_external(e)` et `FrameworkError::from_external_with("saving user", e)` conservent l'erreur originale accessible comme source `std::error::Error` au lieu de la fondre dans une chaîne. `FrameworkError::external_source()` la retourne pour le downcasting ; utilisez-la plutôt que `source()`, qui retourne le handle `Arc` partagé. Les deux constructeurs correspondent à HTTP 500.
- **Les logs 5xx rendent maintenant toute la chaîne de sources d'erreur.** `render_error_chain` parcourt `source()` et est branché dans la ligne de log framework-error, la charge utile de l'événement `ErrorOccurred` et le champ `debug_message` émis sous `APP_DEBUG=true`. Les corps de réponse côté client restent inchangés et les corps 5xx restent assainis.
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` / `try_scroll_wrapped`.** Ces méthodes placent l'instruction de fusion d'une prop scroll sous `<key>.<wrap_key>` plutôt que sous la clé nue  -  `mergeProps: ["users.data"]` au lieu de `["users"]`  -  pour une valeur enveloppe. Le `ScrollProp` de Laravel enveloppe inconditionnellement sous `"data"` ; les paginateurs intégrés de Suprnova retournent un tableau nu, donc cela reste opt-in. Le nouveau trait `ProvidesScrollMetadata` (`page_name` / `previous_page` / `next_page` / `current_page`, avec `scroll_metadata()` par défaut) reflète l'interface Laravel pour un paginateur inconnu de cette crate ; `LengthAwarePaginator`, `Paginator` et `CursorPaginator` l'implémentent désormais. Les champs `.match_on(...)` d'une prop scroll sont aussi émis dans `matchPropsOn`, comme `resolveMergeMatchingKeys` de Laravel.
- **`Prop::merge_with_path`, `match_on` multi-champ et les props de fusion adossées à un résolveur.** `Prop::merge_with_path(path)` fusionne un champ imbriqué au lieu de toute la prop  -  `Prop::eager(v).merge().merge_with_path("data")` émet `mergeProps: ["<key>.data"]`  -  et ne fusionne jamais aussi la racine ; `.deep_merge()` l'ignore. `Prop::match_on` accepte maintenant un ou plusieurs champs (`match_on(["id", "slug"])`) en plus de la composition chaînée. `InertiaResponse::merge_lazy` / `merge_lazy_with` sont les homologues à résolveur de `.merge` / `.merge_with`.
- **Les partial reload `only`/`except` comprennent la notation pointée.** `X-Inertia-Partial-Data: user.name` réduit `user` à `{ name: ... }`, et `X-Inertia-Partial-Except: user.email` élague seulement ce champ. `except` l'emporte lorsque les deux en-têtes nomment le chemin ; une entrée nue signifie encore la prop entière, et un chemin imbriqué inconnu ou de type incompatible est ignoré sans toucher ses frères. Les props `Always` sont toujours envoyées entières.
- **Imbrication des props à clés pointées.** `.with("user.name", value)` et toute autre méthode d'attache de prop imbriquent maintenant dans `props.user` au lieu d'expédier la clé littérale `"user.name"`, comme le déballage `Arr::set` de Laravel. Deux appels partageant un préfixe s'accumulent dans un objet ; une clé sans point n'est pas touchée. Les clés du registre partagé `App::inertia_share*` s'imbriquent de même sur le fil, mais le déballage ne touche jamais la valeur interne d'une prop, de sorte que le sac `errors` conserve ses noms de champs pointés.
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`.** Homologues de `Inertia::getShared` / `Inertia::flushShared`, ils lisent et nettoient le registre statique de partage (`App::inertia_share` / `_lazy` / `_once`). `inertia_shared` admet la même notation pointée en lecture et retourne `None` pour un partage lazy, once ou absent. `flush_inertia_shared` efface seulement le registre statique ; un fournisseur de trait enregistré par `App::register_inertia_shared` reste intact.
- **`InertiaResponse::always_with(key, resolver)`.** Homologue à résolveur async de `.always(key, value)` pour une prop toujours incluse assez coûteuse pour justifier une résolution lazy, comme `Inertia::always(fn () => …)` de Laravel.
- **`InertiaSharedData::share` reçoit maintenant le nom de composant de page**, afin qu'un fournisseur puisse varier sa sortie par page, à l'image de `RenderContext` de Laravel. Voir Mise à niveau.
- **Composition des props Inertia.** Une `Prop` porte désormais des flags orthogonaux au lieu d'être l'une de neuf variantes fermées : une prop peut être deferred et mergeable, mergeable et cached, ou optional et cached. Construisez-la avec `Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent`, chaînez `.always()`, `.optional()`, `.defer()`, `.group()`, `.rescue()`, `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on()`, `.once()`, `.as_key()`, `.until()`, `.fresh()`, `.scroll()`, puis attachez-la avec `InertiaResponse::prop(key, prop)`. Une prop `defer().merge()` est annoncée sous `deferredProps` au premier rendu et arrive sous `mergeProps` à la requête suivante. `MergeMode` et `Visibility` décrivent les flags ; tous les raccourcis existants restent inchangés.
- **Pause / reprise de file.** `Queue::pause(connection, queue)` / `resume` / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` / `paused_queues(connection, &queues)` utilisent `Cache` comme le signal de redémarrage. `resume_all` ne supprime pas une pause par file, comme Laravel. La barrière de claim du worker est juste avant chaque pop, donc un job en cours se termine toujours ; une pause globale court-circuite `--queue=...`, et une pause par file ne prend effet que pour un worker lancé avec une liste explicite. Les nouvelles commandes `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]` (alias `queue:continue`) et `QUEUE_PAUSABLE=false` permettent de désactiver la fonctionnalité. Nouveaux événements : `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed`.
- **`suprnova::testing::TestResponse`** est un wrapper fluent, de forme Laravel-`TestResponse`, autour du triplet `(status, headers, body)` que produit déjà chaque harness HTTP : `assert_status`, `assert_ok`, `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`, `assert_see`, `assert_header`, `assert_cookie` et, avec `.with_session_store(...)`, `assert_session_has`. Chaque assertion retourne `&Self` et panic en cas d'échec, suivant le contrat de `expect!`.
- **`suprnova new` scaffold une entrée SSR.** Chaque starter (Svelte, React, Vue) livre désormais `frontend/src/ssr.{ts,tsx}` et un script npm `build:ssr` (`vite build --ssr`), reliés à leur propre répertoire de sortie (`frontend/bootstrap/ssr/`) afin que le bundle SSR ne percute jamais le build client dans `public/assets/`.
- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`.** La gateway SSR peut vérifier qu'un bundle construit existe sur disque avant d'expédier un rendu, comme la configuration `ensure_bundle_exists` de Laravel. Un worker jamais démarré, ou un bundle jamais construit, échoue rapidement au lieu de consommer `ssr_timeout`. Activez-le avec `.ssr_bundle_path(...)`; le chemin n'est jamais auto-détecté, donc les configurations SSR existantes qui n'en donnent pas restent inchangées.
- **Les échecs de validation lors d'une visite Inertia redirigent maintenant en arrière au lieu de retourner du JSON `422`.** `Inertia::install` enregistre un quatrième middleware, `InertiaValidationRedirectMiddleware`, qui transforme un `422` de validation sur une requête `X-Inertia` en `303` vers la page de formulaire avec les erreurs flashées, afin que `useForm().errors` se remplisse sans code de handler. Les requêtes non Inertia conservent l'enveloppe `422`, les dry-run Precognition ne changent pas, et `X-Inertia-Error-Bag` restreint le sac flashé. La cible est le `Referer` de même origine, puis l'URL précédente de session, puis le chemin de requête passé par le même assainisseur, avec repli sur `/`, jamais une valeur approuvée textuellement.
- **`InertiaConfig::with_all_errors(bool)`** conserve chaque message de validation par champ au lieu de réduire au premier, comme `Inertia\Middleware::$withAllErrors` de Laravel.
- **`suprnova::testing::AssertableInertia`** apporte des assertions fluent, de forme Laravel-`AssertableInertia`, sur un objet page Inertia analysé depuis une réponse JSON `X-Inertia` ou l'élément `<script data-page="app">` d'un shell HTML de hard navigation : `component`, `url`, `version`, `prop`, `has`, `missing`, `where_`, `count`, `has_flash`. Construisez-le avec `AssertableInertia::from_response` depuis un `HttpResponse`, ou `TestResponse::assert_inertia()` depuis un `TestResponse`. `reload_only`, `reload_except` et `load_deferred_props` rejouent une partial reload via une closure `with_reload(...)` fournie par l'appelant.
- **`Cookie::queue`/`queued`/`unqueue`/`expire`.** Un pot de cookies task-local, le `CookieJar` de Laravel, permet à tout code de mettre un cookie en file pour la prochaine réponse sans posséder un `HttpResponse` : listener d'événement, service du conteneur ou middleware avant le handler. Il utilise le même emplacement par requête que `Auth::login_remember`; `SessionMiddleware` le vide à côté du cookie de session. `Cookie::expire(name, path, domain)` met en file un cookie de suppression construit par `Cookie::forget_with`. Les quatre appels sont des no-op silencieux hors de `SessionMiddleware`, comme `App::flash` hors d'une portée flash.
- **`HttpResponse::event_stream(stream, end)` et `HttpResponse::stream_json(stream)`.** Ce sont les homologues `ResponseFactory::eventStream` / `streamJson` de Laravel et les formes filaires attendues par `@laravel/stream-{react,vue,svelte}`. `event_stream` encadre `Stream<Item = sse::StreamedEvent>` comme `event: update` sauf événement explicitement nommé, JSON-encode les charges non chaînes et ajoute une trame terminale configurable (`EndSignal::default()` est `data: </stream>`; `EndSignal::None` l'omet). `stream_json` diffuse tout `Stream<Item = impl Serialize>` comme un tableau JSON flushé progressivement. Les deux reposent sur le pipeline `sse`/`stream_bytes` existant et partagent annulation et isolation des panics.
- **`suprnova serve` relance un processus de dev crashé au lieu de fermer toute la session.** Le backoff exponentiel commence à 200ms, double à chaque crash consécutif, est plafonné à 5s et revient au plancher après 30s de stabilité. `--no-restart` restaure le comportement précédent. `--restart-tries <N>` (par défaut `5`, comme `--restart-tries=5` de Laravel) cesse les nouvelles tentatives après ce nombre de crashes, laisse les autres processus et la session fonctionner et affiche un message exploitable. `--timestamps` préfixe chaque ligne par `HH:MM:SS`. Un tableau `[[serve.process]]` dans `Suprnova.toml` déclare les processus de dev du projet, chacun avec `[name]` et une couleur optionnelle ; clé inconnue ou `name`/`command` vide est une erreur d'analyse stricte. `--json` produit plutôt un objet JSON NDJSON par ligne pour les événements de processus, de régénération de types et d'arrêt ; avec `--timestamps`, c'est inoffensif mais redondant.
- **`RequestBuilder::retry_when(predicate)`.** Le prédicat est consulté avant chaque réessai que la politique intégrée (`.retry(...)` / `.retry_non_idempotent(...)`) ferait, avec `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }`. Il compose avec la politique : `false` interdit un réessai prévu, sans jamais pouvoir en forcer un au-delà de `max_attempts` ni un que la politique ne ferait pas.
- **`#[model(touches = [...])]` touche maintenant réellement.** Après création, sauvegarde, mise à jour ou suppression d'un enfant, chaque propriétaire `BelongsTo` nommé reçoit `UPDATE <owner> SET updated_at = ? WHERE <key> = ?` sur le même exécuteur ; dans `DB::transaction`, le touch rejoint donc la transaction. Les modèles sans `timestamps`, les clés étrangères `NULL` et les propriétaires soft-deleted sont ignorés. Une entrée `touches` qui ne nomme pas une relation `BelongsTo` déclarée est maintenant une erreur de compilation ; les propriétaires polymorphes ne sont pas encore pris en charge.
- **`without_touching_on::<M, _, _>(fut)`.** Homologue de `Model::withoutTouchingOn([M::class], $cb)` : il supprime `m.touch()` et toute cascade de propriétaire vers `M`, tandis que les propriétaires d'autres types continuent d'être touchés. Les portées s'imbriquent, et `without_touching` supprime désormais aussi la cascade de propriétaire.
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`.** Homologues de `touchOwners()` de Laravel pour les cas où l'écriture enfant emprunte un chemin que le framework ne possède pas.
- **Règles de validation de forme valeur : `ArrayKeys` et `Distinct`.** Le nouveau trait `ValueRule` (`passes(&self, value: &serde_json::Value)`) vit à côté de `Rule` avec le même contrat de messages à clés. `rules::ArrayKeys(&[...])` refuse un objet JSON ayant une clé hors liste (`array:keys` de Laravel) ; `rules::Distinct { ignore_case, strict }` refuse un tableau JSON ayant un élément répété. Les lignes `validate!` acceptent les deux types dans la même liste de champ, dispatchés selon le trait implémenté.
- **`Job::delay()`.** Les jobs peuvent déclarer un délai par défaut (`fn delay() -> Option<Duration>`, `None` par défaut), respecté par `Queue::push` et `Queue::bulk` : `available_at` devient `now + delay`. Un délai explicite de site d'appel l'emporte toujours ; `Queue::push_later(job, at)` et `Queue::later(delay, job)` utilisent textuellement l'horodatage appelant.
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`.** Une notification en file (`Notify::queue`) transporte maintenant ses défauts de réglage de file vers chaque push `SendNotificationJob` par la primitive `EnvelopeOverrides` utilisée par `Mail::on_queue`. `fail_on_timeout(&self) == true` dead-letter dès le premier timeout, comme l'attribut de notification `#[FailOnTimeout]` de Laravel. Les cinq valeurs par défaut restent celles de `SendNotificationJob`, donc une notification sans override ne change pas.
- **`Mail::on_queue` / `Mail::on_connection` + `Queue::push_with`/`later_with`.** Un mailable en file se route avec `Mail::to(..).on_queue("emails").queue(mailable)`, ou par défaut via `Mailable::queue(&self)`. Ces réglages devancent tout `Queue::route` et `Job::queue()`/`Job::connection()`. `EnvelopeOverrides` couvre aussi timeout, fail-on-timeout, max-tries et backoff pour un push. Les snapshots en file de `MailFake` portent la `queue` résolue, avec `queued_on(...)` / `assert_queued_on(name, queue)`.
- **`Application::http_bootstrap(f)`.** Ce hook de démarrage uniquement HTTP s'exécute après `bootstrap` et seulement pour `serve` / `web:run`; workers de file, planification, workflow et binaire console ne l'exécutent jamais. Les images worker et console n'ont plus besoin d'un manifeste frontend construit pour démarrer : `Inertia::install` échoue toujours fermé en production s'il manque, mais le contrôle ne s'exécute que dans un processus servant réellement HTTP.
- **`Router::inertia(path, component, props)`.** Homologue de `Route::inertia` de Laravel pour une page statique dont le handler ferait une ligne. Il enregistre `GET` (HEAD y tombe aussi) et retourne un `RouteBuilder`, afin que la route puisse être nommée et recevoir un middleware. `Router::view` demeure un alias.
- **Options d'envoi SES v2.** Le transport SES émet maintenant `TenantName`, `ConfigurationSetName` et `ListManagementOptions` dans `SendEmail`. Chacune possède un défaut de transport (`SesMailTransport::tenant_name` / `configuration_set_name` / `list_management`) et un override par en-tête (`X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`, `X-SES-LIST-MANAGEMENT-OPTIONS`), l'en-tête l'emportant. Les en-têtes sont consommés lors de la construction de la requête et jamais rendus dans le message.
- **`without_cookies` sur chaque builder de réponse.** `HttpResponse`, `Response` (via `ResponseExt`), `Redirect` et `RedirectRouteBuilder` expirent une liste de cookies en un appel ; `Redirect` / `RedirectRouteBuilder` reçoivent aussi le `without_cookie` à nom unique qui leur manquait. `Cookie::forget_with(name, path, domain)` construit un cookie de suppression borné au chemin et au domaine d'origine ; un `forget` simple n'efface jamais un cookie défini hors de `/`.
- **`Queue::fake()` attribue un id d'enveloppe à chaque push capturé.** `pushed_with_id::<J>()` retourne les paires `(job, id)`, et le fake dispatch désormais la même paire `JobQueueing` / `JobQueued` que le driver réel, portant cet id. Les helpers fake existants ne changent pas.
- **Événement de file `UniqueJobSkipped`.** `Queue::push_unique` dispatch maintenant `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` lorsqu'il supprime un doublon, de sorte que le dédoublonnage est observable ; la valeur de retour reste `Ok(false)`.
- **`model_keys()` sur le query builder et les collections.** `User::query().model_keys().await?` retourne les clés primaires de toutes les lignes correspondantes sans hydrater un modèle, en projetant la clé qualifiée de table (`users.id`) afin de survivre à une jointure. `Collection::model_keys()` est l'homologue déjà hydraté. `#[suprnova::model]` déclare aussi le type Rust de clé comme `EloquentModel::Key`, ainsi les deux méthodes retournent les noms `key_type` plutôt qu'un turbofish imposé par l'appelant.

### Corrigé

- **Les suppressions logiques PostgreSQL utilisent maintenant des espaces réservés adaptés au backend, et les écritures d'horodatage générées respectent les casts déclarés.** `delete()` et `restore()` rendent les espaces réservés ordinaux PostgreSQL au lieu des `?` MySQL et SQLite. Les écritures générées de création, mise à jour, sauvegarde, touch et suppression logique convertissent aussi les horodatages selon le type de stockage `Cast` du champ, afin que les colonnes `TIMESTAMPTZ` natives ne reçoivent plus de texte. Merci à [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) pour le signalement et le correctif dans la [PR #3](https://github.com/eas4ai/suprnova/pull/3).
- **Les exécutions par défaut du workspace et de la gate Magnetar n'exigent plus de services PostgreSQL ou MySQL vivants.** Les suites de comportement propres aux backends sont des tests de qualification explicites et ignorés qui échouent encore lorsqu'elles sont délibérément invoquées sans leur base configurée. Les tests de simple reachability et les exigences d'environnement permanentes de gate ont disparu, de sorte que les modifications sans rapport ne paient plus la configuration de bases externes à chaque vérification.
- **`PartialFilter::narrow` est maintenant `pub`.** Ses quatre prédicats frères (`should_include`, `should_include_eager`, `should_include_optional` et le type) étaient publics, mais le passage qui réduit une valeur résolue aux chemins pointés effectivement demandés par `only`/`except` était `pub(crate)`. Un appelant bâtissant une gestion de partial reload personnalisée n'avait aucun moyen public de reproduire cette réduction et expédiait une valeur entière pour une entrée `only` pointée alors que `should_include_eager` déclarait la clé incluse.
- **Le `QueuedSnapshot` de `MailFake` peut maintenant asserter `.on_connection(...)`.** `Queue::fake()` avait obtenu `assert_pushed_on_connection` avec `assert_pushed_on_queue`, mais `Mail::fake()` n'avait que la moitié queue. Une surcharge de connexion était appliquée au dispatch réel mais non assertable par le fake. `QueuedSnapshot::connection`, `MailFake::queued_on_connection` et `MailFake::assert_queued_on_connection` comblent cette lacune.
- **Une prop partagée pointée était inaccessible par une entrée `only` nue.** Après `App::inertia_share("auth.user", …)`, `router.reload({ only: ['auth'] })` retournait `props: {"errors":{}}`. Les entrées `only`/`except` sont maintenant symétriques : elles peuvent nommer la clé exactement, un chemin à l'intérieur qui réduit, ou un ancêtre qui expédie toute la prop. `except: ['auth']` supprime toutes les clés sous lui, avec une frontière de segment, donc `authAgent.user` reste intact.
- **Un champ `#[data(lazy(deferred))]` contournait la liste blanche `?include=`.** Le chemin de résolution marqué propriétaire ne sélectionnait que `Prop::is_lazy()`, faux pour une valeur avec le flag `Visibility::Deferred`. `Prop::resolve_with_owner` borne maintenant chaque prop adossée à un résolveur, et `resolve_props` exécute cette barrière avant le reste : un champ hors `?include=` est supprimé entier, et un champ demandé mais absent de la liste blanche du DTO lève son `400` avant que `X-Inertia-Partial-Data` ne l'absorbe.
- **`deferredProps` était réannoncé lors d'une partial reload correspondante.** Une partial nommant une clé deferred annonçait encore toutes les autres ; Laravel retourne `[]` dans `resolveDeferredProps` dès qu'une requête est partial. Le bloc est maintenant supprimé sur toute partial correspondante, tandis qu'une reload pour un composant différent reste une visite standard.
- **Le sac `errors` était filtré différemment selon son origine.** Le sac flashé de session était injecté avant la boucle de résolution, mais un `.with("errors", …)` de handler passait par les barrières ordinaires. Les deux chemins traitent maintenant `errors` comme toujours visible, comme `Inertia::always(...)` de Laravel ; un flag de visibilité explicite sur la clé l'emporte toujours.
- **`Queue::fake()` peut maintenant observer les `EnvelopeOverrides` par push.** Un job envoyé par `Queue::push_with`/`Queue::later_with` était indiscernable d'un `Queue::push` simple. `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` retourne chaque push avec ses overrides ; `assert_pushed_on_queue::<J>(queue)` et `assert_pushed_on_connection::<J>(connection)` couvrent les cas courants. Les autres points d'entrée enregistrent `EnvelopeOverrides::default()`.
- **Un worker SSR bloqué au milieu du corps de réponse pouvait suspendre un rendu indéfiniment.** `SsrConfig::timeout` bornait seulement l'attente des en-têtes. Les deux phases partagent maintenant un même délai, qui borne tout l'appel SSR et permet le repli CSR ou l'erreur `ssr_throw_on_error`.
- **Les cookies en file, y compris le cookie remember-me de `Auth::login_remember`, étaient perdus silencieusement sur trois chemins internes fail-closed de `SessionMiddleware`.** Les échecs de lecture de session, d'écriture de session et de chiffrement du cookie de session retournaient un `500` synthétisé avant de vider les cookies en attente. Les trois chemins les vident maintenant avant de retourner, comme une erreur ou redirection issue d'un handler ; cela ne couvre pas une panic non interceptée.
- **`Queue::push_unique` respecte maintenant `Job::delay()`, comme `Queue::push`, `Queue::push_with` et `Queue::bulk`.** Il calculait auparavant `available_at` directement depuis `Utc::now()`. `Queue::push_unique_later` et `Queue::later_unique` restent inchangés : ils reçoivent déjà un horodatage ou délai explicite et ne consultent jamais `Job::delay()`.

### Modifié

- **La branche de développement actuelle utilise SeaORM 2.0 et exige Rust 1.94.0.** Suprnova conserve ses formes de code source Eloquent, `#[model]`, migrations et façade de base de données. Les applications appelant SeaORM directement doivent importer `ExprTrait` pour les méthodes d'expression SeaQuery et employer les méthodes de connexion `*_raw` pour des valeurs `Statement` préconstruites. SeaQuery est maintenant 1.0 et le driver vectoriel MariaDB direct utilise SQLx 0.9. Les bases existantes n'exigent aucune migration de données ; les schémas PostgreSQL neufs gardent des clés primaires adossées à serial.
- **Trois dépendances inutilisées supplémentaires ont été supprimées.** `pretty_assertions` et `qrcode` quittent la crate framework (`totp-rs` porte déjà la fonctionnalité `qr`), et `notify-debouncer-mini` quitte le CLI (`notify` reste utilisé par les watchers `serve` et `generate-types`). Les trois ont été confirmées inutilisées par `cargo-udeps` et une recherche couvrant aussi les doc tests.
- **`suprnova-macros` ne dépend plus de `serde` ni de `serde_derive_internals`.** Aucun n'était utilisé : les chemins `::serde::Serialize` émis par les macros se résolvent dans la crate en aval, non dans la crate macro. Aucun effet sur le code généré.
- **Le champ `match_on` de `MergeStrategy` porte maintenant plusieurs noms de champs.** `Append`, `Prepend` et `Deep` passent de `match_on: Option<String>` à `match_on: Option<Vec<String>>`, donc `InertiaResponse::merge_with` / `merge_lazy_with` peuvent dédupliquer sur plusieurs champs, comme `.prop(key, Prop::eager(v).match_on([...]))`. Voir Mise à niveau.
- **Les props scroll émettent maintenant des sémantiques `reset` et fusion identiques à Laravel.** `scrollProps[key].reset` vaut `true` exactement si le client a nommé `key` dans `X-Inertia-Reset`, non plus à chaque visite sans `X-Inertia-Infinite-Scroll-Merge-Intent`. Une prop scroll porte inconditionnellement des métadonnées de fusion, append par défaut ; une visite fraîche émet `reset: false` et une entrée `mergeProps`. Une clé de `X-Inertia-Reset` est exclue de `mergeProps` / `prependProps`.
- **`ssr:check` vérifie maintenant que la route SSR `GET /health` répond 2xx**, plutôt que de confirmer seulement qu'une connexion TCP est acceptée, comme `Inertia\Ssr\HttpGateway::isHealthy()` de Laravel.
- **La prop Inertia `errors` porte maintenant une chaîne par champ, non un tableau.** Un sac de validation flashé rend `{ email: "The email field is required." }` au lieu de `{ email: ["The email field is required."] }`, comme Laravel et `ErrorValue = string` d'Inertia. `InertiaConfig::with_all_errors(true)` restaure le tableau ; une prop `errors` définie par handler reste inchangée, et le flash de session conserve des tableaux.
- **`Model::TOUCHES` a été déplacé d'une const inhérente vers `EloquentModel`.** La cascade parent-touch vit sur un défaut de trait qui ne peut lire une const inhérente. `Comment::TOUCHES` se résout encore, mais exige `use suprnova::EloquentModel;`. Les modèles sans attribut `touches` reçoivent le défaut vide du trait.
- **`RelationEntry` gagne `related_updated_at_column`.** Tout code construisant un `RelationEntry` à la main doit fournir ce champ ; rien dans l'arbre ne le fait et la macro les émet tous.
- **`Router::view` rejette maintenant les props qui ne sont pas un objet JSON.** Il les ignorait silencieusement et enregistrait une route à sac de props vide. `null` reste accepté comme absence de props ; `Router::try_inertia` est la forme fallible.
- **La version d'asset Inertia est maintenant par défaut un hash du manifeste de build Vite** au lieu de la chaîne `"1.0"`. `InertiaConfig::manifest_path(...)` redirige aussi le résolveur ; `.version(...)` / `.version_with(...)` explicites l'emportent. Sans manifeste sur disque, le développement local replie sur `"1.0"`. `VersionResolver::from_manifest(path)` expose le résolveur directement.

### Obsolète

- **`Cookie::read_encrypted` est désormais le lecteur legacy v1 uniquement.** Du code qui émet avec `Cookie::encrypted` puis lit avec `read_encrypted` échoue à l'exécution dès la première valeur écrite après cette version ; passez à `read_encrypted_for(name, wire)`. Les points d'entrée `CryptPurpose::Cookie` sans contexte sont également remplacés. Les deux suppressions sont prévues pour 1.4.0.

### Mise à niveau

- **Les avertissements de déchiffrement de cookies ont deux axes indépendants.** Un avertissement `KeyOrigin::Previous(index)` signifie rechiffrer sous l'`APP_KEY` actuel et retirer cette clé précédente seulement après la fin de la rotation ; `AadVersion::Legacy` signifie réémettre le cookie par l'API liée au nom avant la suppression du repli 1.4.0. Une valeur peut signaler les deux.
- **`SESSION_COOKIE_PREFIX` est opt-in.** Déployez `__Host-` seulement avec HTTPS, `SESSION_SECURE=true`, `SESSION_PATH=/` et aucune valeur `SESSION_DOMAIN`; les scaffolds HTTP locaux le laissent vide. `CsrfMiddleware::with_session_config` garde le nom littéral `XSRF-TOKEN`; utilisez `.xsrf_cookie_name("__Host-XSRF-TOKEN")` lorsqu'un client utilise ce nom distinct.
- **`DecryptOrigin` est maintenant une struct `#[non_exhaustive]` à deux axes.** Lisez indépendamment ses champs `key` et `aad`, et conservez une stratégie de match compatible wildcard pour les enums `KeyOrigin` / `AadVersion`.
- **`SessionConfig` et `CookieOptions` sont maintenant `#[non_exhaustive]`.** Les littéraux de struct et record updates fonctionnels du code d'application doivent devenir `Type::default()` suivi d'assignations de champs publics ou de méthodes builder.
- **`FrameworkError` est maintenant `#[non_exhaustive]`.** Un `match` dans votre code doit porter un bras wildcard. C'est la dernière version où ajouter une variante aurait été un changement cassant.
- **Le champ `match_on` de `MergeStrategy::Append`/`Prepend`/`Deep` est maintenant `Option<Vec<String>>`, non `Option<String>`.** `MergeStrategy::Append { match_on: Some("id".into()) }` ne compile plus ; enveloppez le nom dans un `Vec` : `Some(vec!["id".into()])`. `match_on: None` ne change pas.
- **Une partial reload correspondante n'émet plus `deferredProps`.** Le code lisant `page.deferredProps` sur une réponse partial ne trouve plus la clé ; lisez les annonces lors de la visite initiale non partial, où Laravel et le client officiel les lisent.
- **Une entrée `except` nue supprime désormais les clés de props pointées sous elle.** `X-Inertia-Partial-Except: auth` laisse auparavant `auth.user` dans la réponse ; elle est maintenant supprimée. Si une page comptait sur l'élagage de la seule clé exacte, nommez `except: ['auth.user']` ou réduisez avec un chemin pointé.
- **`errors` ignore `only`/`except`.** Une partial reload qui excluait ou réduisait `.with("errors", …)` expédie maintenant tout le sac. Mettez à jour les tests attendant un objet `errors` vide ou réduit. Pour le garder délibérément hors réponse, marquez-le `.prop("errors", Prop::eager(…).optional())`.
- **`Prop::resolve_with_owner` borne aussi les props à flags.** Il résolvait auparavant toute prop qui n'était pas `Prop::is_lazy()` sans consulter l'include set. Il borne maintenant chaque prop à résolveur et laisse seulement une valeur déjà matérialisée passer sans barrière. Un champ `#[data(lazy(deferred))]` exige donc `?include=<field>` pour être résolu ou annoncé ; ajoutez-le à `?include=` ou supprimez `lazy(...)` s'il n'a jamais dû être opt-in.
- **`reset` d'une prop scroll ne suit plus l'en-tête merge-intent.** Un composant custom ou snapshot qui lit `page.scrollProps[key].reset` voit `reset: false` et une entrée `mergeProps` lors d'une revisite ordinaire, au lieu de `reset: true` sans métadonnées. `<InfiniteScroll>` officiel ne vide l'état accumulé que lorsque le serveur nomme réellement la clé dans `X-Inertia-Reset`; envoyez explicitement `X-Inertia-Reset: <key>` là où l'ancien comportement était requis.
- **`Prop::match_on` prend `impl MatchOnFields`, non `impl Into<String>`.** Cette borne permet `match_on(["id", "slug"])` et sa liste d'impl est volontairement fermée : `&str`, `String`, `[T; N]` et `Vec<T>`. `&String`, `Cow<'_, str>` et `Box<str>` ne compilent plus ; passez un `&str` : `match_on(name.as_str())`, `match_on(name.as_ref())` ou `match_on(&*name)`.
- **Une entrée `only`/`except` pointée réduit maintenant sa prop de haut niveau au lieu de l'exclure entièrement.** `X-Inertia-Partial-Data: user.name` fournit désormais `{ user: { name: ... } }`; auparavant `user` était silencieusement supprimé. La correction s'applique à `should_include_optional` : `permissions.read` déclenche maintenant réellement une prop `Optional` ou `Defer` de haut niveau et peut donc exécuter un résolveur BD ou externe qui ne s'exécutait pas auparavant. Surveillez le volume d'appels de résolveur après mise à niveau.
- **`InertiaSharedData::share` prend maintenant le nom du composant de page.** Ajoutez un paramètre `component: &str` après `req` :

```diff
-async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
+async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
```
Ignorez-le (`_component`) si le fournisseur ne varie pas par page ; `RenderContext` de Laravel porte le même couple (`component`, `request`) pour `ProvidesInertiaProperties::toInertiaProperties`.
- **`Prop` est une struct, non une enum.** Ses variantes disparaissent ; construisez et lisez les props par méthodes :
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (`Prop::lazy(closure)` ne change pas)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` -> `prop.is_absent()`

- **Ce correctif protège aussi les sessions que vous avez déjà, pas seulement les requêtes à venir.** Une mise à niveau suffit : un cookie de session écrit par une version précédente peut encore contenir un `_previous.url` qui n'a jamais été assaini, et `SessionData::previous_url()` le rejette désormais à la lecture la première fois que cette session est utilisée après la mise à niveau, au lieu de lui faire confiance parce qu'il est déjà stocké. Vous n'avez pas besoin d'invalider les sessions existantes, de migrer la table de session, ni de forcer une reconnexion. Une requête dont le chemin ressemble à un chemin relatif au protocole (`//host`) ne met plus non plus à jour l'URL précédente enregistrée à l'avenir - si votre route `fallback!` d'application (ou toute route répondant `200` et atteignable sur un chemin inhabituel) s'appuyait légitimement sur un tel chemin pour devenir la cible de `Redirect::back()`, ce ne sera plus le cas. Dans tous les cas, la précédente valeur sûre de la session reste en place (ou le fallback propre à `Redirect::back(fallback)` l'emporte, si rien de sûr n'a jamais été enregistré). Aucun changement de code n'est nécessaire sauf si vous dépendiez de ce cas limite précis, qui était déjà un risque de redirection ouverte.
+ **Retirez `[0]` de chaque liaison `errors.<field>` dans vos pages.** Avec le nouveau schéma par défaut, `errors.email` est une chaîne, donc `errors.email[0]` rend son premier caractère au lieu du message. Modifiez en même temps le type TypeScript de `string[]` à `string`. Si vous préférez ne pas toucher vos pages, mettez `InertiaConfig::with_all_errors(true)` sur la config passée à `Inertia::install` et ajoutez l'augmentation de module `errorValueType: string[]` pour `@inertiajs/core`. Les frontends de starter livrent le nouveau schéma.
+ **Un handler qui réécrivait à la main le redirect-back après un échec de validation peut maintenant le supprimer.** Le pont est automatique désormais ; un handler qui se redirige encore lui-même continue de fonctionner, car le middleware n'agit que sur un `422` qui contient un objet `errors` rempli.
+ **Un enfant crashé de `suprnova serve` redémarre désormais au lieu de mettre fin à la session.** Si vous comptiez sur le crash pour arrêter `suprnova serve` net (un smoke check CI, un script qui traite la sortie comme « quelque chose ne va pas »), passez `--no-restart` pour retrouver exactement ce comportement. Les nouvelles tentatives sont aussi bornées par défaut : un processus qui crashe 5 fois d'affilée cesse d'être relancé (augmentez la limite avec `--restart-tries`, ou utilisez `--no-restart` pour le comportement original « un crash et c'est fini »).
+ **`Model::TOUCHES` n'est plus une const inhérente.** Le code qui lisait directement `Comment::TOUCHES` a désormais besoin de `use suprnova::EloquentModel;` (ou `suprnova::eloquent::EloquentModel`) dans le scope - la const a été déplacée là afin que la cascade de parent-touch, défaut du trait `Model`, puisse la lire. Une recherche `grep -rn TOUCHES` dans votre app trouve tous les points d'appel ; la plupart des apps n'en ont aucun, puisque la const ne faisait rien à l'exécution auparavant.
+ **`RelationEntry` a gagné un champ.** Seul le code qui construit un `RelationEntry` à la main doit changer - ajoutez `related_updated_at_column` au littéral. Les enregistrements de relations générés par macro que le framework livre déjà l'émettent, donc une application normale qui se contente de déclarer des relations via `#[suprnova::model]` n'est pas affectée.
+ **`Router::view` avec des props non objet lève désormais une panic au démarrage.** Il les ignorait auparavant silencieusement et enregistrait une route avec un sac de props vide ; `view` délègue maintenant à `Router::inertia`, qui exige un objet (ou `null`) et panique sinon. Si un appel à `view` peut transporter des props non objet, passez à `Router::try_inertia` et gérez le `Err` - sinon rien ne change pour vous.
+ **La valeur par défaut du manifeste de version Inertia peut changer votre chaîne de version dès qu'un build existe.** Une application ou un test qui code en dur `X-Inertia-Version: 1.0` continue à fonctionner seulement jusqu'à ce qu'un manifeste Vite apparaisse sur disque ; dès qu'il est là, la version devient le hash du manifeste. Si vous avez besoin de l'ancienne constante, lisez-la vous-même depuis `VersionResolver::from_manifest(path)` ou épinglez explicitement `.version(...)`. Attendez-vous à ce que le premier déploiement après la mise à niveau force un cycle complet de rechargement de page pour les clients déjà connectés - une seule fois, et c'est le but du changement. La valeur de repli sans manifeste est exportée comme `suprnova::MANIFEST_VERSION_FALLBACK`, donc vous n'avez plus jamais besoin de coder en dur `"1.0"`.
+ **Déplacez `Inertia::install` et l'enregistrement `global_middleware!` hors de `bootstrap::register`.** Mettez-les dans une nouvelle fonction et passez-la à la place à `.http_bootstrap(...)` - la nouvelle forme du scaffold est une `register_http_stack()` synchrone appelée comme `.http_bootstrap(|| async { bootstrap::register_http_stack() })`. Les apps qui sautent cela conservent le comportement d'aujourd'hui, y compris l'échec du boot worker sur un manifeste frontend manquant.
## 1.2.4 - 2026-08-18

### Sécurité

- **Le secret de contournement du mode de maintenance est comparé en temps constant.** `MaintenanceMiddleware` comparait l'URL secrète avec une simple comparaison de chaînes, qui s'arrête au premier octet différent. Comme le secret est un identifiant au porteur transporté dans le chemin de la requête, cette différence de temps indiquait à un attaquant la longueur du préfixe qu'il avait deviné correctement. La comparaison s'exécute désormais sur toute la longueur en octets via `subtle::ConstantTimeEq`, et ne court-circuite que sur une différence de longueur - la même forme que la comparaison du cookie de contournement à côté d'elle.
- **`rules::Url` rejette désormais les URI de script.** La règle acceptait tout schéma que `url::Url` pouvait analyser, y compris `javascript:` et `vbscript:`, si bien qu'une URL validée pouvait quand même servir de puits d'exécution de script une fois rendue dans un `href`. Elle applique désormais la forme de la règle `url` de Laravel (`^(PROTOCOLS)://HOST` de `Illuminate\Support\Str::isUrl`) : le schéma doit figurer dans la liste blanche de Laravel, être suivi de `://`, **et** être suivi d'un hôte non vide - le groupe hôte de Laravel n'a pas de `?`, donc un hôte absent ou vide ne correspond jamais, même avec un schéma listé. La liste des schémas et l'exigence `://` + hôte sont celles de Laravel mot pour mot ; l'hôte lui-même est analysé par la crate `url` plutôt que par la regex de Laravel, si bien que quelques cas limites diffèrent encore - un port hors plage est rejeté ici et accepté là-bas, et les hôtes IDN se normalisent différemment. Le nouveau `Url::protocols(&[...])` reflète `url:http,https` de Laravel ; `HttpUrl` n'est désormais que du sucre et conserve son propre message. **Changement de comportement :** une URL avec un schéma non listé qui validait auparavant échoue désormais - nommez le schéma avec `Url::protocols(&["myapp"])` si vous vouliez l'accepter. Deux autres changements de comportement : `mailto:`, `data:` et `tel:` sont nommément sur la liste blanche de Laravel mais ne portent pas de composante d'autorité, donc ils échouent désormais ; et les chemins de la forme `file:///etc/passwd` - `scheme://` avec rien entre les deux derniers slashes - échouent désormais aussi, puisqu'une chaîne vide n'est pas non plus un hôte. Les deux découlent de la règle `://` + hôte de Laravel elle-même.
- **Les réponses Inertia annoncent désormais `Vary: X-Inertia` partout.** L'en-tête n'était défini que sur les réponses de l'objet de page lui-même. Les redirections, les 404, les 422 et les réponses statiques n'en portaient aucun, si bien qu'un cache partagé indexé uniquement par l'URL pouvait servir l'objet de page JSON à une navigation complète du navigateur, ou le shell HTML à un XHR Inertia. Le nouveau `InertiaHeadersMiddleware` - enregistré par `Inertia::install` comme le plus externe des trois - le fixe sur chaque réponse et transforme un `200` vide lors d'une visite Inertia en un `303` de retour au lieu d'une réponse que le client rejette comme non Inertia. `InertiaVersionMiddleware` re-flashe maintenant la session avant son `409`, si bien qu'une erreur flashée survit au `GET` de page complète suivant du client.
- **Trois correctifs sur les réponses Inertia.** `InertiaResponse::location_for(&req, url)` retourne `409` + `X-Inertia-Location` pour un XHR Inertia et un simple `302`
  + `Location` pour une navigation dure, si bien qu'un rebond OAuth ou SSO amorcé hors du SPA ne se termine plus en cul-de-sac sur un `409` sans corps. La variante `location(url)` existante conserve sa forme toujours-`409`. `App::clear_history()` flashe le drapeau d'effacement de l'historique dans la session, si bien qu'il survit à la redirection de déconnexion et atterrit sur la page qui est réellement rendue - la `.clear_history()` par réponse ne marquait que la redirection que le navigateur jette, laissant l'historique chiffré de la session précédente déchiffrable. Et une prop `once` n'est désormais ignorée que lors d'une visite Inertia complète : un `router.reload({ only: ['stats'] })` explicite la réévalue, au lieu de ne rien renvoyer.
- **Le transport SES envoie désormais les en-têtes de message personnalisés.** `Mail::to(..).header("List-Unsubscribe", ...)` et `Mailable::headers()` étaient ignorés en silence sous `MAIL_DRIVER=ses` : le corps de requête `Content.Simple` n'avait pas de champ `Headers`, et le constructeur MIME brut ne lisait jamais `OutgoingMessage::headers`, alors que tous les autres transports les relaient. Les deux chemins SES les transportent maintenant - `Headers` comme liste `{Name, Value}` de SES v2, et le MIME brut comme de vraies lignes d'en-tête - si bien que les liens de désabonnement, les en-têtes de fil et les indices de routage survivent à un changement de driver. Les noms d'en-tête sont validés à l'avance sur les deux chemins - CR, LF et NUL (les octets d'injection que le transport Mailgun rejette déjà) et tout ce qui n'est pas un nom de champ RFC 5322 valide (espaces, deux-points, non-ASCII) - si bien qu'ajouter un fichier joint ne change jamais si un message est accepté.

### Corrigé

- **Les échecs de validation imbriquée atteignent désormais le corps 422.** `#[validate(nested)]` sur une struct imbriquée ou sur un élément d'un `Vec<T>` validé étaient perdus entre le validateur et la réponse : la requête était correctement rejetée avec un 422, mais la map `errors` revenait vide, si bien qu'aucun message ne s'affichait et que le client ne pouvait pas savoir quel champ était en cause. Les échecs imbriqués sont désormais aplatis dans la notation pointée de Laravel - `address.street`, `items.1.name`, `order.items.2.sku` - à côté de ceux du niveau supérieur.
- **L'`url` de l'objet de page Inertia conserve la chaîne de requête.** `page.url` n'était que le chemin de la requête, si bien que le client enregistrait `/users` pour une visite à `/users?page=2&sort=name`. Chaque navigation avant/arrière et chaque `router.reload()` rejouait alors la page sans son curseur de pagination, son tri ou ses filtres. C'est désormais le chemin plus la chaîne de requête - la même dérivation que `InertiaVersionMiddleware` utilisait déjà pour `X-Inertia-Location`, si bien que, par défaut, les deux concordent octet pour octet. Le nouveau `InertiaConfig::url_resolver(...)` redéfinit la manière dont l'*objet de page* nomme la page (le `Inertia::resolveUrlUsing` de Laravel) ; le rebond de version continue de nommer l'URL qui est arrivée, parce que c'est l'URL que le navigateur doit récupérer.
- **`Inertia::install` applique désormais sa config à chaque réponse.** La config passée à `Inertia::install` était lue pour trois champs puis abandonnée, si bien que chaque `InertiaResponse` construit sans `.with_config(...)` explicite rendait à partir de `InertiaConfig::default()`. Une application scaffoldée avec `--frontend react` servait le point d'entrée Svelte et aucun préambule de refresh React à moins que `SUPRNOVA_FRONTEND` ne soit défini dans l'environnement ; le SSR activé dans la config n'atteignait jamais une réponse ; et la version d'asset de l'objet de page provenait d'une config différente du résolveur du middleware de version. La config installée est désormais conservée dans le registre Inertia du conteneur et sert de base à `InertiaResponse::new`. `.with_config(...)` par réponse continue de l'emporter, les applications qui n'appellent jamais `Inertia::install` restent inchangées, et une installation échouée (fail-closed) ne conserve rien. Effet secondaire : le manifeste Vite de production est désormais analysé une fois par processus plutôt qu'une fois par réponse.
- **Les applications générées installent maintenant les middlewares du protocole Inertia.** Le `bootstrap.rs` écrit par `suprnova new` enregistrait les middlewares de session, locale, CSRF et include, mais n'appelait jamais `Inertia::install`, si bien qu'une application générée n'avait ni `InertiaVersionMiddleware` ni `Inertia303Middleware` : un navigateur qui exécutait encore le bundle précédent n'était jamais invité à recharger après un déploiement, et un `PUT`/`PATCH`/`DELETE` qui redirigeait restait sur un `302` que le client pouvait suivre avec le verbe d'origine. L'appel arrive maintenant après `SessionMiddleware` - là où le middleware de version peut reflasher la session - avec une constante nommée `INERTIA_VERSION` à incrémenter quand les assets changent, et il épingle le frontend avec lequel le projet a été scaffoldé (`.frontend(Frontend::React)` pour `--frontend react`), si bien que le shell HTML charge le point d'entrée Vite de ce framework au lieu de retomber sur celui de Svelte. Le `.env` généré définit maintenant `SUPRNOVA_FRONTEND` en conséquence. Le starter `--api` est inchangé ; il n'a pas de frontend.
- **`Queue::push_unique` n'indique plus qu'un job en file a été omis.** La valeur de retour était calculée avec `matches!(outcome, Idempotent::Fresh(()))`, ce qui réduisait `Idempotent::FreshUnfenced` à `false` - le cas où l'enveloppe *avait* bien été poussée, mais où le bail de déduplication était perdu en plein push. Les appelants qui bifurquaient sur ce booléen se voyaient dire qu'un job sur le point de s'exécuter avait été supprimé comme doublon. Les trois issues sont maintenant matchées de façon exhaustive : un bail perdu renvoie `true` avec un `warn` nommant le job et sa clé unique, et seul un vrai doublon renvoie `false`. `push_unique_later` et `later_unique` partagent le chemin et sont corrigés avec lui.
### Modifié

- **La base de parité passe à Laravel 13.25.0.** Les notes de version 13.23.0, 13.24.0 et 13.25.0 ont été retracées point par point jusqu'à la surface du framework. Tout ce qui atteignait un chemin de code Suprnova est soit corrigé dans cette version, soit indiqué dans [`parity.md`](parity.md) avec `not yet` ou `by design no`.

### Mise à niveau

Deux changements peuvent altérer une application en service sans aucune modification de code de votre part.

- **Les réglages de la config que vous passez à `Inertia::install` prennent désormais effet.** Ils étaient lus pour trois champs puis abandonnés. Si votre config d'installation définit `.ssr(...)`, le SSR est désormais activé : démarrez le worker (`suprnova ssr:start`) avant de déployer, ou retirez l'appel `.ssr(...)`. `.entry_point`, `.assets_base_url`, `.default_title` et `.encrypt_history(...)` définis là atteignent désormais aussi la page.
- **`rules::Url` rejette davantage.** Les valeurs qui passaient et ne passent plus : tout schéma hors de la liste blanche de Laravel, y compris `javascript:` et `vbscript:` ; `mailto:`, `data:` et `tel:`, qui figurent dans la liste blanche mais ne portent pas d'hôte `://` ; et `scheme://` avec un hôte vide, comme `file:///path`. Si vous vouliez accepter un schéma, nommez-le : `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Corrigé

- **Les casts de date et heure lisent désormais le texte `CURRENT_TIMESTAMP`
  natif de la base de données.** `AsDateTime`, `AsImmutableDateTime` et
  `AsOptionalDateTime` continuent d'écrire du RFC-3339 canonique, tandis que
  les lectures acceptent aussi le texte PostgreSQL avec fuseau et les valeurs
  SQLite/MySQL sans fuseau. Les valeurs sans fuseau sont interprétées en UTC.

## 1.2.2 - 2026-08-14

### Corrigé

- **Les valeurs nullables non textuelles fonctionnent désormais dans toutes
  les écritures basées sur des attributs avec PostgreSQL.** Les
  `Builder::update_all` et `Builder::upsert` typés, les
  `DB::table().insert/update` sans modèle et les attributs supplémentaires des
  pivots plusieurs-à-plusieurs émettent les nulls JSON explicites sous la forme
  SQL `NULL`, tout en continuant à lier chaque valeur non nulle. Cela préserve
  le type de la colonne cible au lieu d'envoyer un paramètre null typé comme du
  texte que PostgreSQL rejette pour les colonnes bigint, integer, boolean,
  timestamp et autres colonnes non textuelles. Les upserts à plusieurs lignes
  rejettent maintenant aussi les colonnes manquantes ou supplémentaires au lieu
  de convertir silencieusement une ligne mal formée en null. Les timestamps
  automatiques des pivots plusieurs-à-plusieurs sont liés comme des datetimes
  UTC typés plutôt que comme du texte.

### Sécurité

- **Le gate de release distingue désormais les métadonnées dormantes
  du lockfile des dépendances compilées dans tout le workspace.** Cargo
  enregistre la dépendance de compatibilité optionnelle rkyv 0.7 inutilisée de
  rust_decimal dans `Cargo.lock` ; le gate prouve désormais que ni rkyv ni son
  crate de dérivation ne sont accessibles depuis aucun membre du workspace,
  aucune feature, aucune target ni aucune arête de dépendance. L'exception
  RustSec correspondante est attribuée et expire le 2026-11-14 ; elle doit être
  supprimée lorsque rust_decimal n'enregistrera plus cette ancienne dépendance
  optionnelle.

## 1.2.1 - 2026-08-09

### Modifié

- **Suprnova a quitté l'organisation GitHub `entrepeneur4lyf` pour
  `eas4ai`.** Les URL du dépôt dans
  les métadonnées des paquets, la documentation, les exemples de dépendances et
  les modèles de scaffold utilisent désormais `github.com/eas4ai`. Les nouveaux
  projets utilisent également l'adresse d'auteur surveillée
  `shawn@eas4ai.com`. Cette version n'a modifié aucun comportement à l'exécution.

## 1.2.0 - 2026-08-05

### Ajouté

- **Le manuel est distribué en sept langues.** `manual/es/`, `manual/fr/`,
  `manual/de/`, `manual/pt-BR/`, `manual/ja/` et `manual/zh-Hans/`
  portent chacun le manuel complet de 104 chapitres - chaque chapitre,
  la table des matières et ce journal des modifications - traduit depuis
  la source anglaise. L'anglais reste canonique : la structure des
  chapitres, les blocs de code, les identifiants, les commandes CLI et
  les variables d'environnement sont maintenus identiques octet par
  octet à la source, si bien qu'un chapitre traduit ne peut jamais
  contredire l'anglais sur ce que fait le framework - seulement le dire
  dans la langue du lecteur.

  Les traductions ont été produites et relues pour suprnova.app, qui
  rend ce manuel comme son `/docs`. Chaque section y porte un registre
  de relecture : les verdicts sont enregistrés contre des hachés de
  contenu de l'anglais et de la traduction, deux relecteurs indépendants
  doivent approuver les octets exacts pour qu'une section compte comme
  approuvée, et des glossaires par langue fixent les décisions de
  terminologie (quels termes restent en anglais, lesquels prennent le
  mot natif, et pourquoi). Les corrections sont bienvenues dans l'un ou
  l'autre dépôt - un correctif ici atteint le site à sa prochaine
  synchronisation.

## 1.1.0 - 2026-08-02

### Ajouté

- **Chaînes de repli par locale.** `LocalizationConfig` gagne `parents`
  (`APP_LOCALE_PARENTS`, paires `child=parent` séparées par des
  virgules, ou le builder chaînable `.parent(child, parent)`) : une
  locale peut hériter d'une locale sœur configurée avant de retomber
  plus loin sur le `fallback_locale` global - `pt-PT` depuis `pt-BR`,
  `en-AU` depuis `en-GB`, et ainsi de suite, transitivement.
  `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has` parcourent tous
  la chaîne, locale courante en premier, ce qui fonctionne donc pour
  n'importe quel driver `Translator`, pas seulement celui fourni. Une
  paire malformée, une locale invalide, un enfant nommé deux fois, ou
  un cycle (y compris une locale se nommant son propre parent) échoue
  explicitement au chargement de la config plutôt que de se dégrader
  au moment de la requête.

  Les catalogues servis restent aplatis par chaîne à l'avance :
  `FluentTranslator` construit désormais le catalogue
  `/_suprnova/lang/<locale>.ftl` de chaque locale comme un pliage - le
  catalogue du framework embarqué en bas pour les locales `en`/`en-*`,
  puis la chaîne de parents configurée de la locale, puis ses propres
  fichiers `*.ftl` - si bien qu'une locale chaînée reste un seul
  fichier autonome que le navigateur récupère une fois, sans
  conscience de chaîne côté client. L'aplatissement ne couvre que les
  parents configurés ; le `fallback_locale` terminal reste un repli au
  niveau de la façade `Lang`, pas cuit dans les octets servis.

  Cela rend praticables les catalogues de type delta : un répertoire
  `lang/pt-PT/` peut ne contenir que la poignée de chaînes qui diffère
  réellement de `lang/pt-BR/`, plutôt qu'un catalogue dupliqué en
  entier. La fusion qui rend cela possible opère au niveau de l'AST
  Fluent - la valeur d'un enfant remplace celle du parent, les
  attributs fusionnent par nom (un override qui ne mentionne pas un
  attribut ne le perd plus), les expressions select se remplacent en
  bloc (les catégories plurielles CLDR dépendent de la locale, donc
  une fusion variante par variante n'est pas cohérente), et les
  entrées propres à l'enfant s'ajoutent. Voir la nouvelle section
  « Fallback chains » de `manual/localization.md` pour le contrat
  complet.

### Modifié

- **`LocalizationConfig` a gagné le champ `parents`.** `from_env()` et
  le builder ne sont pas affectés ; un constructeur de struct littéral
  (des tests qui construisent une `LocalizationConfig` à la main) a
  besoin d'un champ de plus.
- **Le texte des catalogues servis est désormais normalisé par le
  sérialiseur pour chaque locale**, et la fusion multi-fichiers
  intra-locale (plusieurs fichiers `.ftl` dans un même répertoire de
  locale) passe désormais par la même fusion au niveau AST que les
  chaînes de parents plutôt que par un simple écrasement de bundle.
  Les traductions résolues sont inchangées à part les deux
  améliorations strictes ci-dessous ; les octets sous-jacents tournent
  quand même - `ETag`/`?v=<hash>` tourne une fois lors de la mise à
  niveau. Les améliorations : un override ne fait plus silencieusement
  disparaître les attributs qu'il ne mentionne pas, et un override
  qui ne porte que des attributs ne dépouille plus la valeur propre du
  message (auparavant une erreur ou une résolution de repli ; il
  résout désormais vers la valeur de l'override précédent).

## 1.0.0 - 2026-08-02

### Ajouté

- **Localisation.** Des catalogues de messages dans
  `lang/<locale>/*.ftl` ([Fluent](https://projectfluent.org)), une
  façade `Lang` avec la macro `__!("key", name: value)`, une
  détection de locale par requête (`LocaleMiddleware` : session →
  cookie → `Accept-Language` → `APP_LOCALE`), et un formatage
  sensible à la locale pour les nombres, la devise, les dates, les
  heures, les listes et les temps relatifs via ICU4X.
  `manual/localization.md` est le chapitre.

  Les règles de validation intégrées cessent de coder l'anglais en
  dur. Chacune renvoie un message avec clé (`validation-min` plus ses
  arguments et un repli anglais), traduit une seule fois à la
  frontière de sérialisation - si bien qu'une app espagnole obtient
  des erreurs de validation en espagnol en déposant
  `lang/es/validation.ftl`, sans habillage de règle et sans copie
  divergente des messages du framework. Les noms de champs
  s'humanisent via une recherche `field-<name>`. `Rule::passes` (et
  `ContextualRule` / `AsyncRule`) renvoient désormais
  `Result<(), ValidationMessage>` ; le corps `Err("…".into())` d'une
  règle personnalisée compile encore et se rend encore verbatim, mais
  la signature de votre `impl` a besoin du nouveau type.

  Le navigateur reçoit les mêmes octets que ceux résolus par le
  serveur : le catalogue fusionné est servi à
  `/_suprnova/lang/<locale>.ftl` avec un ETag et une forme immuable
  `?v=<hash>`, les trois kits de démarrage le parsent avec
  `@fluent/bundle`, et `suprnova generate-types` émet une union
  `MessageKey` si bien que renommer un message pointe le compilateur
  TypeScript vers chaque site d'appel.

  Fluent plutôt que des tableaux PHP façon Laravel, parce qu'un seul
  format doit servir à la fois le serveur et le navigateur, et parce
  que les catégories plurielles CLDR sont ce qui donne le russe, le
  polonais et l'arabe corrects - les intervalles d'entiers de
  `trans_choice` ne le peuvent pas, ce qui explique qu'il n'y ait pas
  de `trans_choice` ici. Derrière une feature `localization` activée
  par défaut ; `--no-default-features` compile encore et valide
  encore, en utilisant les replis anglais embarqués.

- **`IntoInertiaScroll` pour `Paginator`.** Le trait était implémenté
  pour `LengthAwarePaginator` et `CursorPaginator` mais pas pour le
  paginateur simple, si bien que les résultats de `simple_paginate` ne
  pouvaient pas du tout alimenter `Inertia::paginate` - malgré les
  docs de module de `simple.rs` elles-mêmes qui le désignent comme le
  chemin de génération d'URL. Cela laissait les collections Inertia
  paginées par décalage face à un choix entre un `COUNT(*)` par
  requête et bricoler les métadonnées de scroll à la main. `next_page`
  provient de la sonde de dépassement `LIMIT n+1` plutôt que d'une
  dernière page calculée, faute d'un total à partir duquel en calculer
  une.

### Corrigé

- **`suprnova generate-types` émettait un fichier différent à chaque
  exécution.** Le tri topologique amorçait sa file de travail en
  itérant une `HashMap`, et Rust randomise l'ordre d'itération du hash
  par processus, si bien que des exécutions consécutives ordonnaient
  les mêmes interfaces différemment. La sortie est un artefact
  versionné, donc chaque exécution produisait un diff - et un fichier
  généré qui bouge sans raison est un fichier que l'on arrête de
  régénérer, après quoi il cesse silencieusement de décrire le Rust
  qu'il prétend décrire. Le parcours de répertoire est désormais trié
  aussi, si bien que la sortie ne dépend plus non plus de l'ordre du
  système de fichiers. Deux exécutions de la même source sont
  désormais identiques octet pour octet.

- **`topological_sort` faisait l'inverse de ce que disait son
  commentaire de doc**, en émettant les dépendants avant les
  dépendances. Sans conséquence - une interface TypeScript peut
  référencer une interface déclarée plus loin dans le même fichier -
  le commentaire est donc corrigé plutôt que l'ordre, ce qui aurait
  remanié un fichier suivi pour aucun bénéfice.

## 0.9.1 - 2026-08-01

Trois défauts, tous trouvés en exécutant l'app dogfood sous un harnais
conteneurisé plutôt qu'en lisant le code. Chacun d'eux est invisible à
une suite de tests qui n'arrête jamais un processus comme la
production l'arrête.

Ils se combinent dans un ordre précis : un déploiement glissant envoie
un SIGKILL à un worker en plein job (le premier), et ce job emprunte
alors un chemin de récupération qui n'a jamais compté la tentative (le
second).

### Corrigé

- **`schedule:work`, `queue:work` et `workflow:work` ignoraient
  SIGTERM.** Chacun sélectionnait uniquement sur
  `tokio::signal::ctrl_c()`, qui installe un handler SIGINT - si bien
  que SIGTERM n'avait de handler nulle part dans le processus, et
  SIGTERM est ce que `docker stop`, Coolify, systemd et Kubernetes
  envoient. Les trois avaient déjà un vidage borné et soigné derrière
  ce `select!` ; rien de tout cela ne s'était jamais exécuté sous un
  superviseur. Mesuré avant le correctif : un `docker stop` sur un
  conteneur `queue:work` consumait toute sa fenêtre de grâce de 40s et
  sortait en 137 avec le job en vol détruit. En tant que PID 1 - ce
  qu'exécute un conteneur - le noyau écarte purement et simplement un
  SIGTERM non géré, si bien que le processus ne mourait pas mal ; il
  ne mourait pas du tout avant le SIGKILL. `Server::run` gérait déjà
  correctement les deux signaux et son listener TCP est désormais
  partagé, ce qui referme aussi une fenêtre de signal manqué dans la
  boucle du planificateur.

- **Un job qui tuait son worker ne pouvait jamais être mis en lettre
  morte.** Un job dont le *handler* échoue est nacké et sa tentative
  comptée, si bien qu'il passe en lettre morte après `max_tries`. Un
  job qui *tue son worker* - OOM, abort, segfault, ou le SIGKILL
  ci-dessus - ne clôture rien ; sa réservation s'éteint simplement, et
  chaque driver avait l'habitude de le redistribuer identique à
  l'octet près. Un tel job est immortel : il tue chaque worker qui le
  réclame, revient inchangé, et tue le suivant, aussi longtemps que
  quelque chose redémarre des workers. Les trois drivers imputent
  désormais la tentative au moment où ils apprennent qu'un worker est
  mort, parce que changer `QUEUE_DRIVER` ne doit pas changer si un job
  toxique peut être arrêté. `attempts` signifie désormais « livraisons
  à un worker » plutôt que « échecs du handler » - documenté dans
  `manual/queues.md`, parce qu'un worker perdu pour des raisons sans
  rapport consomme aussi une tentative.

- **… et le job épuisé est désormais mis en lettre morte avant d'être
  dispatché.** Compter la tentative était nécessaire et pas
  suffisant. Chaque décision de mise en lettre morte vivait dans le
  chemin de clôture du worker, qui suppose que le handler retourne -
  si bien qu'elle ne s'exécutait jamais précisément pour les jobs qui
  ne pouvaient pas retourner. Avec le seul correctif du driver, le
  compteur grimpait (mesuré : 0 → 1 → 2 sur trois workers tués) et
  rien n'agissait dessus. Le budget est désormais dépensé avant que
  le handler ne s'exécute. Repéré seulement en ré-exécutant
  l'expérience du conteneur après que le premier correctif ait semblé
  correct.

- **Les daemons n'avaient aucun subscriber de traçage.** `serve` en
  obtient un via `init_telemetry` ; `queue:work`, `schedule:work`,
  `schedule:run` et `workflow:work` passent par un chemin d'amorçage
  différent et n'obtenaient rien, si bien que chaque ligne `tracing::`
  qu'ils émettaient n'allait nulle part et `LOG_LEVEL` était inerte
  pour eux. C'est l'essentiel de ce qu'ils ont à dire - un worker qui
  met un job en lettre morte, un planificateur qui saute un tick
  qu'il a perdu, un verrou qu'il n'a pas pu relâcher. Dans un
  conteneur, la seule sortie visible était la bannière de démarrage,
  et le processus semblait inactif alors qu'il faisait tout cela.
  Deux des défauts de cette version étaient invisibles avant ce
  correctif.

- **Une mise en lettre morte sans magasin de jobs échoués lié était
  une suppression silencieuse.** L'étape de persistance se trouvait
  dans un `if let Some(store) = ..`, si bien que sans magasin le bras
  ne correspondait pas et l'exécution retombait sur l'ack - plus
  silencieux que le chemin d'échec juste au-dessus, qui laisse au
  moins la réservation intacte. Un magasin absent était traité comme
  plus réussi qu'un magasin cassé. Cela journalise désormais
  l'enveloppe complète en ERROR, parce que c'est ce que `queue:retry`
  repousse : la différence entre du travail récupérable à la main et
  du travail qui a cessé d'exister.

- **`QUEUE_DRIVER=database` lie désormais un magasin de jobs
  échoués.** `failed_jobs` fait partie du contrat de ce driver -
  `queue:retry` le lit et `Queue::retry_failed` ne peut pas
  fonctionner sans lui - mais `bootstrap_from_env` câblait le driver
  et laissait le magasin non défini, si bien qu'une file d'attente
  adossée à la base de données mettait en lettre morte vers rien à
  moins que l'app n'en lie un à la main. Configurable via
  `QUEUE_FAILED_DB_TABLE`. Seulement pour ce driver : `memory` est
  éphémère par construction et `redis` n'a aucune table où écrire.

- **La latence de récupération Redis suit désormais
  `--visibility-timeout`.** Le flag fixe le seuil d'inactivité de
  XAUTOCLAIM, mais une horloge séparée gouverne la fréquence à
  laquelle un consommateur regarde, et le driver la laissait au
  défaut de sea-streamer de 30s - si bien que
  `--visibility-timeout 5` signifiait en réalité « jusqu'à 35
  secondes ». L'intervalle suit désormais le timeout configuré, borné
  entre 1s et 30s, si bien qu'un timeout court ne peut pas devenir une
  tempête de XAUTOCLAIM et qu'un long ne peut que rendre la
  récupération plus rapide qu'avant.

### Ajouté

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** -
  exécuter une tâche planifiée exactement une fois par tick dû, à
  travers les répliques. Sans cela, rien n'élit un leader pour un
  tick : chaque processus `schedule:work` évalue la planification
  indépendamment, et trois répliques ont été mesurées exécutant
  chaque tâche due trois fois, chaque minute, sans variance. Un job
  de facturation nocturne sur trois répliques facturait chaque client
  trois fois.

  `without_overlapping()` ne couvre pas ce cas et ne le peut pas : son
  verrou est indexé sur la tâche et relâché quand le handler retourne,
  si bien qu'une tâche rapide le libère avant qu'une seconde réplique
  ne regarde. `on_one_server` s'indexe sur la tâche *et le tick* et
  retient le verrou au-delà du handler, le laissant expirer sur TTL.
  Les deux se composent.

  Opt-in, à l'image de Laravel. Diverge de Laravel en échouant fermé :
  l'élection n'est partagée qu'à la mesure du cache derrière elle, si
  bien qu'un démarrage en production avec `CACHE_DRIVER=memory` et une
  tâche mono-serveur est refusé, en nommant les tâches fautives, avec
  `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` pour les déploiements
  qui font vraiment tourner un seul planificateur.

### Modifié

- `manual/deployment.md` ne dit plus « exécutez exactement un
  processus `schedule:work` » comme unique option, et gagne une
  section **Arrêt propre** couvrant les fenêtres de vidage par
  sous-système, comment dimensionner le délai de grâce de terminaison
  d'une plateforme au-dessus d'elles, et pourquoi PID 1 rend un
  handler de signal manquant pire qu'il n'y paraît.

## 0.9.0 - 2026-07-31

### Sécurité

- **L'émission d'authentification ne pouvait être limitée que par
  appelant, jamais par destinataire.** Une limite à clé d'adresse
  répond à *un client est-il bruyant* ; elle ne peut pas répondre à
  *une boîte mail est-elle en train d'être inondée*. Un attaquant
  réparti sur un botnet ou un seul `/64` IPv6 restait sous chaque
  budget par IP tout en remplissant la boîte de réception d'une
  victime avec des e-mails de réinitialisation de mot de passe, et
  rien dans le framework ne pouvait exprimer la limite qui aurait pu
  l'arrêter - une fonction de clé pouvait lire le chemin, les
  en-têtes et la query string, mais pas un corps form-encodé, si bien
  que l'adresse était invisible précisément sur la route qui la
  porte.

  `identity_key` indexe un seau sur le compte visé par l'action. Elle
  lit d'abord la query string puis un corps de formulaire mis en
  tampon, si bien qu'une seule fonction de clé couvre les deux
  formes ; la valeur est trimée et mise en minuscules, parce que
  `Alice@Example.com` atteint la même boîte mail que
  `alice@example.com` et qu'une limite contournée en maintenant la
  touche majuscule n'est pas une limite ; et elle est hachée, parce
  qu'un backend de limitation de débit est fréquemment un Redis
  partagé avec un contrôle d'accès plus faible que la base de données
  primaire.

  Deux nouveaux builders de middleware la rendent possible.
  `key_reads_body(cap)` met le corps en tampon avant le calcul de la
  clé - opt-in, parce que la mise en tampon est un travail qu'un
  appelant non authentifié peut vous faire faire, et un corps
  au-dessus du plafond est rejeté avec un 413 plutôt que transmis
  sans clé. `only_when(pred)` saute entièrement un limiteur pour les
  requêtes sur lesquelles il n'a rien à dire, ce qui est ce qui
  empêche un budget par destinataire empilé de devenir silencieusement
  la limite contraignante sur les routes qui ne nomment aucun
  destinataire.

  L'app dogfood empile désormais les deux sur son groupe d'émission :
  10 par 5 minutes par adresse, 3 par 15 minutes par destinataire.

Une revue des chemins de session, mot de passe, OAuth et passkey de
Torii a mis au jour huit défauts, tous corrigés dans le fork épinglé
(`suprnova-torii-rs` `968b0be`).

- **Des sessions expirées pouvaient être rafraîchies pour reprendre
  vie.** Le `refresh` du repository de session SeaORM n'avait aucun
  prédicat d'expiration et prolongeait `expires_at` inconditionnellement,
  et `OpaqueSessionProvider::refresh_session` sautait la vérification
  `is_expired()` qu'effectue `get_session`. Un token détenu au-delà de
  son expiration pouvait être renouvelé indéfiniment. Corrigé aux deux
  niveaux. Pas atteignable via la propre surface de Suprnova - ni
  `Torii` ni le framework n'exposent de refresh de session - mais
  c'est une API publique des deux crates.
- **Le formulaire de connexion laissait fuir quels comptes existent,
  par timing.** L'authentification retournait dès que l'e-mail ne
  correspondait pas, sautant complètement Argon2 : mesuré à 54µs pour
  une adresse inconnue contre 719ms pour un mauvais mot de passe, un
  écart d'environ 13 000x lisible sur le réseau. Les deux chemins
  d'échec vérifient désormais contre un hash factice pour coûter la
  même chose. Celui-ci *était* atteignable via la connexion par mot de
  passe de Suprnova.
- **La claim JWT `iss` était écrite mais jamais vérifiée.**
  L'épinglage d'algorithme était déjà correct - `alg: none` et la
  confusion HS/RS n'ont jamais été possibles - mais l'émetteur n'était
  que décoratif, si bien que deux services partageant une clé de
  signature accepteraient les sessions l'un de l'autre. Désormais
  imposé quand un émetteur est configuré.
- **Un vérificateur PKCE à usage unique pouvait être réclamé deux
  fois.** La consommation était une lecture suivie d'une suppression,
  si bien que deux callbacks OAuth pour le même `csrf_state` pouvaient
  tous deux la lire avant qu'aucune suppression n'aboutisse. Désormais
  réclamé en une seule opération - `DELETE ... RETURNING` sur
  Postgres, une suppression par clé primaire dont le nombre de lignes
  affectées désigne le gagnant sur SeaORM.
- **Des sessions expirées étaient listées comme actives.**
  `find_by_user_id` n'avait aucun filtre d'expiration, et les lignes
  expirées survivent jusqu'à ce qu'un nettoyage s'exécute, si bien
  qu'un écran « appareils sur lesquels vous êtes connecté » proposait
  aux utilisateurs de révoquer des sessions mortes sans rien dire de
  la session vivante.
- **Une recherche de passkey s'appelait `authenticate`.**
  `PasskeyService::authenticate_credential` de Torii prenait un ID de
  credential et renvoyait l'utilisateur propriétaire, et
  `PasskeyAuth::authenticate` en émettait une session. Torii stocke
  des passkeys - elle ne porte aucune dépendance WebAuthn et ne peut
  pas vérifier une assertion, si bien que la seule chose que ces
  appels prouvaient était que l'appelant connaissait un ID de
  credential : une valeur que le navigateur envoie en clair et
  qu'`allowCredentials` remet à quiconque peut démarrer une cérémonie.
  Renommés en `find_user_by_credential` et
  `create_session_for_verified_credential`, documentant tous deux que
  la vérification est la responsabilité de l'appelant. Pas atteignable
  via Suprnova, qui pilote `webauthn-rs` elle-même (voir
  `torii_integration::passkey`) et n'atteint Torii que pour le
  stockage des credentials.
- **Un défi WebAuthn était rejouable pendant tout son TTL.** Aucun des
  deux backends ne consommait un défi à la lecture, et le
  `get_challenge` de SeaORM ignorait aussi complètement `expires_at`,
  renvoyant des défis expirés comme actifs. Les lectures excluent
  désormais les lignes expirées sur les deux backends, et un nouveau
  `take_challenge` en réclame un exactement une fois - la même forme
  où la suppression décide du gagnant que le correctif PKCE.

### Rupture

- **Azure Blob Storage et Google Cloud Storage sont passés derrière
  les nouvelles features `filesystem-azure` et `filesystem-gcs`.**
  `Storage::register_azblob`, `register_azblob_with`, `register_gcs`,
  `register_gcs_with`, `AzBlobConfig` et `GcsConfig` n'existent plus à
  moins d'activer la feature correspondante. Si vous utilisez l'un ou
  l'autre backend, ajoutez-le à votre dépendance :

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  Vous obtenez une erreur de compilation qui nomme l'élément manquant,
  pas un échec à l'exécution.

  Les deux crates de service opendal tirent `rsa`, qui porte
  RUSTSEC-2023-0071 (l'attaque temporelle Marvin) sans version
  corrigée en amont. C'étaient les seules crates à activer
  `reqsign-core/jwt`, la feature derrière laquelle se trouve le `rsa`
  optionnel de `reqsign-core`, si bien que les conditionner coupe
  d'un coup les trois chemins opendal qui y mènent. `rsa` est
  désormais *évitable* : `--no-default-features --features
  filesystem,database-postgres` se résout sans lui et garde quand
  même le sous-système de stockage. Auparavant, aucune combinaison de
  features ne pouvait s'en débarrasser tout en gardant le stockage.

  Un build par défaut standard porte toujours `rsa` - `database-mysql`
  est une feature par défaut et `sqlx-mysql 0.8.6` en dépend de façon
  non optionnelle - si bien que l'exception d'audit reste ouverte. S3
  n'est délibérément **pas** conditionné : `reqsign-aws-v4` prend
  `reqsign-core` sans `jwt`, si bien que le driver S3 n'a jamais
  contribué de chemin, et le conditionner casserait le backend cloud
  le plus utilisé sans rien retirer.

### Ajouté

- **`suprnova --version`**, avec `-v` en plus du `-V` par défaut de
  clap. Demander sa version à un CLI avec le flag que tout autre CLI
  utilise ne devrait pas afficher une erreur d'usage.

### Corrigé

- **Deux opérations Redis n'avaient aucune borne supérieure.** Le
  vidage de tag du cache lisait tout l'ensemble de membres d'un tag
  avec `SMEMBERS` et supprimait clé par clé, si bien qu'un tag avec un
  grand nombre de membres bloquait la connexion et qu'une écriture
  concurrente pouvait être perdue entre la lecture et la suppression ;
  les tags sont désormais basés sur une génération, vidés
  atomiquement, et parcourus avec un `SSCAN` borné. La passe de
  promotion de la file différée déplaçait chaque job dû en un seul
  `ZRANGEBYSCORE` non borné, si bien qu'un arriéré arrivant à échéance
  en même temps produisait un unique script énorme ; elle promeut
  désormais par batches.
- **Deux vidages d'arrêt attendaient indéfiniment.** `schedule:work`
  sur Ctrl-C et le worker de workflow après annulation attendaient
  tous deux chaque tâche en vol sans délai limite, si bien qu'une
  tâche qui ne retournait jamais gardait le processus ouvert jusqu'au
  `SIGKILL` - un opérateur voit un daemon qui « ne s'arrête pas ». Les
  deux attendent désormais un délai de grâce borné, puis abandonnent
  ce qui reste et rapportent le compte.
- **Le balayage d'épinglage de version du release ne reconnaissait
  qu'une des deux syntaxes d'épinglage**, si bien que chaque fichier
  portant une ligne `cargo install --tag vX.Y.Z` et aucun extrait de
  dépendance n'était jamais découvert. `suprnova-cli/README.md` disait
  aux lecteurs d'installer la v0.6.0 depuis trois versions ;
  `manual/cli.md` et `manual/cli-new.md` étaient restés à la v0.7.2 ;
  `manual/installation.md` portait les deux formes et en avait une
  mise à jour pendant que l'autre restait figée. La découverte et la
  réécriture lisent désormais depuis une seule table de motifs, et les
  règles d'un fichier sont dérivées de son contenu.
- **`cargo doc` échouait pour tout build avec `filesystem` mais sans
  `testing`** - sept liens intra-doc de `Storage::fake` ne pouvaient
  pas se résoudre, et `lib.rs` interdit les liens cassés. `testing`
  est une feature par défaut, donc aucune étape de gate n'avait jamais
  construit cette combinaison ; `check-feature-matrix.sh` le fait
  désormais.
- **Les migrations de Torii ne pouvaient pas être rejouées sur leur
  propre schéma**, si bien qu'une base de données la détenant sans la
  table de suivi `torii_migrations` - restaurée depuis un dump qui l'a
  sautée, ou migrée à la main - ne pouvait pas être ramenée sous
  gestion. Chaque `Table::create()` portait `.if_not_exists()` ;
  aucun des 19 appels `Index::create()` ne le faisait, pas plus que
  l'alter `ADD COLUMN locked_at`, si bien que le rejeu traversait les
  tables sans encombre et mourait sur le premier `CREATE INDEX`.
  Corrigé dans le fork épinglé (`suprnova-torii-rs` `a0f956d`) via
  `has_index` / `has_column` plutôt que `IF NOT EXISTS`, que sea-query
  abandonne silencieusement pour MySQL - le correctif syntaxique
  aurait laissé cassé un build aux features par défaut.
- **Une migration Torii échouée interrompait le processus au lieu de
  renvoyer une erreur.** `SeaORMStorage::migrate` faisait un `unwrap`
  sur le migrateur et renvoyait `Ok(())` inconditionnellement, si bien
  que le mappage par `init_torii` de l'échec vers une `FrameworkError`
  était du code inatteignable.
- **La table `users` propre à une app supprimait silencieusement
  celle de Torii**, parce que `.if_not_exists()` ne peut pas
  distinguer « déjà la mienne » de « déjà celle de quelqu'un
  d'autre ». La migration rapportait un succès et l'authentification
  échouait plus tard sur une colonne manquante - la raison pour
  laquelle le starter `--api` nomme sa table `app_users`. La migration
  de Torii avertit désormais au moment de la migration quand une table
  `users` existante manque de colonnes qu'elle requiert, en nommant
  les colonnes et le remède. Cela reste un avertissement plutôt qu'un
  échec dur, pour que les déploiements existants continuent de
  démarrer.
- **Les guides de déploiement Railway et DigitalOcean pointaient la
  vérification de santé de la plateforme vers un chemin qui pouvait
  sonder Postgres.** Les deux plateformes redémarrent le conteneur
  quand cette vérification échoue, si bien que suivre ce conseil
  transformait un incident passager de base de données en boucle de
  redémarrage à travers toutes les répliques. Les deux utilisent
  désormais `/_suprnova/health/live`, la base de données étant sondée
  à la main depuis la console. Les anciens chemins se résolvent
  toujours ; rien de ce qui est déjà déployé n'a besoin de changer.

## 0.8.0 - 2026-07-30

Remédiation d'un audit red-team externe. L'audit a renvoyé 19 constats
P1 et un verdict NO-GO pour la 1.0 ; cette version en referme **les
dix-neuf**, plus un certain nombre de défauts trouvés en les corrigeant
que l'audit n'avait pas nommés.

Plusieurs correctifs transforment délibérément une mauvaise
configuration silencieuse en un amorçage refusé. Lisez **Mise à
niveau** avant de déployer - une app en production qui tournait sans
souci pourrait ne pas démarrer.

### Mise à niveau

Trois configurations qui avaient l'habitude de démarrer avec un
avertissement (ou en silence) échouent désormais fermées en
production. Chaque erreur nomme la variable qui la débloque, et
chacune a un override explicite pour le déploiement où le risque est
véritablement absent.

- **Un driver mail qui ne livre pas.** `MAIL_DRIVER` non défini,
  `log`, `memory`, ou une valeur non reconnue se résolvaient tous vers
  un transport qui rend le mail puis le jette - si bien que les
  réinitialisations de mot de passe rapportaient un succès alors que
  rien n'était envoyé. Override : `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **SMTP en clair.** Trois des quatre combinaisons d'identifiants
  atterrissaient sur un transport non chiffré, et le cas où les deux
  étaient non définis journalisait un avertissement et envoyait quand
  même. Override : `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **Le limiteur de débit en mémoire.** Ses seaux vivent dans le tas
  d'un seul processus, si bien que derrière N répliques chaque quota
  est en réalité N× et chaque déploiement les réinitialise. Pointez
  `RATE_LIMIT_DRIVER` vers `redis`, ou définissez
  `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true` si vous faites vraiment
  tourner un seul processus. Une valeur de driver *non reconnue*
  échoue pour la même raison, parce qu'elle retombait sur `memory` -
  `RATE_LIMIT_DRIVER=Redis`, avec une majuscule, est le cas le plus
  susceptible d'atteindre la production parce qu'il a l'air configuré.

Le développement, les tests et le staging sont inchangés dans les
trois cas. Le staging n'est délibérément pas conditionné : le faire
échouer dur pousserait les équipes à définir l'override globalement,
ce qui désarme la vérification là où elle compte.

Deux changements de comportement qui ne sont pas des échecs
d'amorçage :

- **`fill` et `first_or_new` rejettent les valeurs malformées.** Une
  valeur qui ne pouvait pas se décoder dans le type de son champ
  devenait auparavant le `Default` de ce champ et renvoyait `Ok` -
  `fill(attrs!{ age: "abc" })` fixait `age = 0` et rapportait un
  succès. Elle renvoie désormais une `ValidationError` qui nomme le
  champ, et laisse le modèle intact. Les colonnes inconnues sont
  toujours ignorées silencieusement (parité Laravel), et
  l'élargissement numérique fonctionne toujours.
- **`/_suprnova/health?db=true` ne renvoie plus l'erreur du driver.**
  Le détail se déplace vers le log ; le corps garde
  `"database": "error"`. Les builds debug l'incluent toujours. Les
  dashboards qui parsent `status` / `database` ne sont pas affectés.
- **`url::signature_has_not_expired` requiert désormais une signature
  valide**, et est dépréciée. Elle répondait auparavant `true` pour
  une URL forgée - une mauvaise signature n'est pas « expirée », parce
  qu'elle n'a jamais eu d'expiration à manquer - si bien que tout
  handler qui se gardait sur elle seule acceptait les forgeries. Elle
  est désormais identique à `has_valid_signature`. Si vous l'utilisiez
  pour distinguer *expirée* d'*invalide* (pour afficher « demandez un
  nouveau lien » plutôt qu'un 403), passez à `url::signature_verdict`,
  qui renvoie les trois états. Ceci diverge délibérément de
  `URL::signatureHasNotExpired` de Laravel.

Deux ajouts qui ne vous concernent que si vous choisissez d'y opter :

- **`QueueDriver` a gagné `settle` et `release`**, tous deux avec des
  implémentations par défaut, si bien que les impls de driver
  existantes compilent encore sans changement. Implémentez `settle`
  si votre backend peut committer une écriture de suivi et un
  acquittement dans une seule transaction ; implémentez `release` s'il
  peut remettre en file un message réservé sur place.
- **La comptabilité de batch peut désormais être durable.**
  `DatabaseBatchRepository` a besoin de deux nouvelles tables,
  `job_batches` et `job_batch_settlements` - ajoutez-les à vos
  migrations, comme pour `jobs` et `failed_jobs`. Le schéma est dans
  `manual/queues.md`. Rien ne change si vous restez sur
  `MemoryBatchRepository`.

### Sécurité

- **Slowloris (SEC-07).** Le timeout de lecture d'en-têtes de hyper
  était documenté à 30s mais inerte - il ne s'arme que quand un timer
  est installé sur le connection builder, et aucun ne l'était. Un
  client pouvait tenir une connexion, et un permis
  `SERVER_MAX_CONNECTIONS`, indéfiniment. Désormais armé et
  configurable via `SERVER_HEADER_READ_TIMEOUT`.
- **Téléversements multipart (SEC-05).** Le plafond s'appliquait aux
  payloads de parties individuelles mais pas au flux brut, si bien
  qu'un corps pouvait dépasser la limite en agrégat. Désormais
  plafonné au niveau du flux.
- **HMAC de webhook avec une clé vide (SEC-08).** Les deux adaptateurs
  de paiement acceptaient un secret vide, ce qui vérifie n'importe
  quoi. Refusé sur les deux.
- **Parsing de signature Paddle (P2-11).** Une `paddle-signature` de
  longueur impaire ou non hexadécimale atteignait le SDK épinglé et
  paniquait à l'intérieur. Désormais validée en premier : une
  signature malformée est un 401.
- **Enrôlement de passkey et tokens de réinitialisation (SEC-01,
  SEC-02).** L'enrôlement anonyme contre un e-mail existant,
  l'enrôlement par un non-propriétaire, et l'enrôlement par le
  propriétaire sans réauthentification récente sont chacun refusés
  avec des statuts distincts. Une connexion par mot de passe
  estampille désormais la fenêtre de réauthentification.
- **`dev:tls` (SEC-10).** Un projet pouvait choisir le CA auquel la
  commande fait confiance.
- **Docker Compose généré (P2-12).** Publiait Postgres et Redis sur
  toutes les interfaces avec des identifiants commités dans ce dépôt.
  Désormais lié au loopback avec des mots de passe générés par
  scaffold, `.env` écrit en 0600, et les cibles symlinkées refusées.
- **Endpoint de santé (P2-01, CI-05).** Il décidait s'il fallait
  interroger la base de données avec `query.contains("db=true")` - un
  test de sous-chaîne, si bien que `?nodb=true` déclenchait aussi la
  sonde. Désormais parsé correctement. Le 503 n'embarque plus l'erreur
  du driver, qui nommait des hôtes, des ports, des schémas et des
  versions.
- **Limitation de l'émission d'identifiants (P2-02).** Les quatre
  routes d'émission d'auth de l'app de référence ne portaient aucune
  limite de débit du tout, et la seule route qui en avait une indexait
  son seau sur l'en-tête brut `x-forwarded-for` - que n'importe quel
  client peut faire varier par requête pour obtenir un seau frais. Les
  deux corrigés ; le budget d'émission est partagé entre les quatre
  routes si bien que tourner entre elles ne le multiplie pas.
- **Une étape de chaîne re-livrée repoussait son successeur sous un
  nouvel id (DATA-02b, partiel).** La clôture pousse le maillon
  suivant de la chaîne *avant* d'acquitter, délibérément : acquitter
  en premier signifie qu'un crash dans cette fenêtre perd la chaîne
  définitivement, et un doublon est récupérable là où une perte
  silencieuse ne l'est pas. Mais l'enveloppe du successeur recevait un
  `Uuid::new_v4()` frais à chaque push, si bien que le doublon produit
  par cet échange était indiscernable d'une nouvelle étape légitime -
  pour le driver, pour un outbox, et pour le handler.

  Ce dernier point est le vrai coût. Le contrat de livraison du
  framework est au-moins-une-fois, et sa réponse aux doublons est
  « les handlers doivent être idempotents » - mais un handler indexé
  sur `env.id`, le seul identifiant qu'il reçoit, ne pouvait pas
  satisfaire ce contrat pour un job chaîné, parce que le doublon
  arrivait sous un nouvel id à chaque fois. Le contrat était
  insatisfiable par construction.

  L'id du successeur est désormais un UUIDv5 dérivé de celui de son
  prédécesseur, qui est stable à travers les propres re-livraisons de
  ce prédécesseur. Une étape re-livrée repousse l'id qu'elle avait
  poussé avant. Aucun changement de schéma, aucun nouveau champ,
  aucune nouvelle dépendance.

  Cela rend le doublon **détectable**, qui est la primitive qui
  manquait au reste de DATA-02b. Cela ne rend pas le push atomique
  avec l'acquittement (cela demande l'outbox), et rien ne rejette
  encore le doublon à l'entrée. Les deux restent ouverts.
- **Les URLs signées vérifiaient une URL et en exécutaient une autre
  (SEC-04).** La forme canonique réduisait les paires de la query en
  une map, si bien qu'une clé répétée ne gardait que sa **dernière**
  valeur - alors que `Request::query_param` renvoyait la **première**.
  Un `?user=victim` légitimement signé pouvait donc être rejoué comme
  `?user=attacker&user=victim` avec la signature d'origine intacte :
  la vérification canonicalisait sur `victim` et passait, et le
  handler agissait sur `attacker`.

  La forme canonique porte désormais chaque paire, triée par
  `(key, value)`, si bien que la signature couvre le multiset exact des
  paramètres - ajouter, retirer, ou substituer n'importe quelle valeur
  casse le HMAC. Un `signature` ou un `expires` répété est refusé
  d'emblée, puisque deux occurrences de l'un ou l'autre ne laissent
  aucune réponse non arbitraire à la question de savoir lequel fait
  foi.

  `Request::query_param` résout désormais une clé répétée vers sa
  dernière valeur, en accord avec `query_params` et
  `Context::query_param` ; c'était la seule des trois à être en
  désaccord, et ce désaccord était l'autre moitié du défaut.
  **Les liens signés existants continuent de fonctionner** - sans clé
  répétée, les octets du payload sont inchangés, ce qu'un test
  épingle, parce qu'un changement de forme canonique qui invaliderait
  silencieusement chaque lien de réinitialisation de mot de passe
  encore valide serait pire que le bug.

  Six tests de régression, incluant les deux ordres d'attaque, une
  clé légitimement répétée qui doit toujours signer et vérifier, et la
  garantie de réordonnancement. *Non* changé : `signature_has_not_expired`
  rapporte toujours une signature forgée comme « pas expirée ». C'est
  le comportement de Laravel, réglé délibérément comme un correctif de
  documentation, et il a son propre test qui l'épingle contre une
  « correction » bien intentionnée.
- **RBAC sous Postgres.** Vérifié contre un vrai Postgres plutôt que
  SQLite seul.
- **Quatre avis RustSec éliminés, pas renouvelés.** Le driver Pinecone
  a été réécrit contre l'API REST de Pinecone, abandonnant
  `pinecone-sdk 0.1.2` - dont la version la plus récente date du
  2024-09-06 - et avec elle `tonic 0.11 → rustls 0.22 →
  rustls-webpki 0.102` et RUSTSEC-2026-0049 / -0098 / -0099 / -0104.
  Les quatre étaient corrigés en amont dans `rustls-webpki >= 0.103.13`,
  que cet espace de travail avait déjà résolu pour ses autres
  utilisateurs de TLS ; une crate abandonnée retenait l'arbre sur la
  ligne vulnérable. `.cargo/audit.toml` passe de cinq exceptions à une
  seule. Voir **Modifié** pour ce que cela signifie pour l'API du
  driver.
- **Les exceptions d'audit expirent désormais.** Chaque entrée de
  `.cargo/audit.toml` porte un `OWNER` et une date `EXPIRES`, et
  `scripts/check-audit.sh` fait échouer le gate de release sur un
  owner manquant, une date manquante ou non parsable, ou une date
  dépassée. `cargo audit` n'a aucune notion d'une exception expirante,
  si bien qu'une exception ajoutée « temporairement » restait jusqu'à
  ce que quelqu'un relise le fichier. L'entrée restante
  (RUSTSEC-2023-0071, `rsa`, qui n'a aucune version corrigée du tout)
  est attribuée et datée.
- **Les prétentions d'accessibilité sont vérifiées, pas seulement
  affirmées.** `scripts/check-feature-matrix.sh` résout de vrais
  arbres de dépendances et vérifie qu'aucun build - y compris
  `--all-features`, ce que `cargo audit` lit réellement - ne contient
  `pinecone-sdk`, `rustls-webpki 0.102.x` ou `tonic 0.11.x`. Une
  exception justifiée par un commentaire que rien ne vérifie cesse
  d'être vraie la première fois que quelqu'un ajoute une dépendance.

### Corrigé

- **Chaque release sur une file d'attente adossée à la base de
  données était silencieusement sans effet.** `JobOutcome::Released` -
  un verrou `WithoutOverlapping` occupé, un backoff de limiteur de
  débit - était implémenté comme « pousser une copie, puis acquitter
  l'original ». L'id de l'enveloppe est la clé primaire de la table
  `jobs`, si bien que la copie entrait en collision avec la ligne
  détenant encore la réservation vivante et le push échouait avec
  `UNIQUE constraint failed: jobs.id`. Le worker refusait alors
  correctement d'acquitter, si bien que le délai demandé n'était
  jamais appliqué, aucun événement `JobReleased` ne se déclenchait, et
  le job restait simplement garé jusqu'à ce que l'expiration de
  visibilité le redistribue. Les releases sont désormais un seul
  appel driver, fait sur place.
- **Un dispatch de batch partiel rendait orphelins les jobs déjà mis
  en file (DATA-02).** Quand un `driver.push` échouait en plein
  milieu de la boucle, `PendingBatch::dispatch` supprimait la ligne du
  batch - mais les enveloppes déjà dans la file portaient toujours
  l'id de ce batch, si bien que chacune d'elles se clôturait contre un
  batch qui n'existait plus, renvoyant `Err(batch not found)` à
  chaque livraison, pour toujours. Le batch est désormais clôturé à
  la place : les jobs non dispatchés sont enregistrés comme des
  échecs et le batch est annulé, si bien que ceux déjà en file se
  clôturent normalement et les callbacks terminaux se déclenchent
  quand même.
- **Rien ne testait que `url::has_valid_signature` rejette une URL
  forgée.** Trouvé en vérifiant le correctif SEC-04 : la suite
  complète du framework passait avec le garde-fou principal des URLs
  signées réécrit pour accepter n'importe quelle signature.
- **Une app scaffoldée ne pouvait ni migrer sa base de données ni
  construire son image (REL-01b).** Aucun des deux scaffolds ne
  déclarait `default-run`, si bien que les neuf wrappers CLI qui
  shellent vers `cargo run` échouaient sur un projet fraîchement créé.
  Le Dockerfile généré avait cinq défauts indépendants - un `COPY` de
  lockfile manquant, `npm ci` sans lock, une étape de cache qui ne
  construisait qu'un binaire factice pour l'un des deux binaires
  déclarés, un build frontend copié depuis un chemin que vite ne crée
  jamais, et une copie de `frontend/src/pages` manquante que
  `inertia_response!` valide à la compilation. L'image d'un scaffold
  standard ne pouvait pas se construire.
- **`docker:init` émettait un seul Dockerfile pour chaque type de
  projet.** Sur un projet `--api`, sa première instruction, `COPY
  frontend/package.json`, échouait d'emblée. Les projets API
  reçoivent désormais un Dockerfile sans frontend.
- **Placeholders SQL (DATA-01).** Rendus par backend plutôt qu'en
  supposant un seul dialecte.
- **Clôture de file d'attente (DATA-02a, P2-06c).** Les suivis se
  clôturent avant que la réservation ne soit acquittée, et une erreur
  de relâchement de verrou ne convertit plus un job déjà réussi en
  retry.
- **Un batch annulé déclenchait `Catch`, jamais `Then`.**
- **`Builder::clone` faisait silencieusement disparaître le plan
  d'eager-load (P2-09a).** `User::query().with("posts")` cloné
  n'importe où - pagination, `count()`, tout scope qui clone -
  renvoyait des lignes sans relations et sans erreur.
- **Les rosters de présence perdaient des membres (P2-08).** Le
  roster était capturé en instantané avant l'abonnement, si bien que
  quiconque rejoignait pendant cette fenêtre n'apparaissait dans
  aucun des deux, en permanence.
- **Pinecone sérialisait chaque acquisition d'index (P2-14).** Le
  verrou d'écriture était tenu à travers deux allers-retours réseau,
  et le `RwLock` équitable de `tokio` signifiait qu'un index froid
  bloquait chaque index chaud.
- **Le watcher de types jetait les rafales (P2-13).** Le debounce à
  front montant régénérait sur le premier fichier d'une rafale et
  abandonnait le reste sans exécution finale, si bien que la dernière
  sauvegarde ne prenait jamais effet.
- **`ssr:check` pouvait se bloquer, et n'essayait qu'une seule adresse
  (P2-13).** Le DNS s'exécutait entièrement en dehors du timeout, et
  seule la première adresse résolue était essayée - si bien qu'un
  hôte avec un enregistrement AAAA et aucune route IPv6 rapportait le
  worker comme down alors qu'il écoutait en v4.
- **`suprnova serve` installait `cargo-watch` sans épinglage
  (P2-13).** Désormais `--locked` avec une borne de version majeure.
- **Le bumper de release réécrivait cinq README et rien d'autre.**
  Quatre chapitres du manuel et un doc comment public épinglaient des
  tags qu'aucune release ne mettait jamais à jour - le doc comment
  avait deux versions de retard. La découverte remplace désormais la
  liste maintenue à la main, et le smoke test grep l'arbre bumpé
  indépendamment plutôt que de faire confiance à la propre étape de
  vérification du bumper.
- **`db:sync` traitait le schéma de la base de données comme une
  entrée de confiance (CLI-01).**
- **`migrate:fresh` est filtré par `--force` plus une confirmation
  typée (CLI-02)**, dans le binaire app comme dans le CLI.
- **Le driver mail `log` journalise désormais le message entier**,
  comme le fait Laravel, et n'écrit plus de liens bearer dans le log
  en production.

### Ajouté

- **Clôture terminale atomique (`QueueDriver::settle`, DATA-02).** Le
  successeur de chaîne et l'acquittement committent désormais ensemble
  sur `DatabaseQueueDriver`, refermant la fenêtre où un crash entre
  les deux perdait le reste d'une chaîne ou exécutait deux fois son
  étape suivante. La suppression indexée sur la réservation fait
  aussi office de barrière : un worker dont la visibilité a expiré en
  cours d'exécution ne committe rien et rapporte `Settled::Stale`, si
  bien qu'il ne peut pas mettre en file du travail pour un message
  qu'un autre consommateur possède désormais. Les drivers qui ne
  peuvent pas faire cela répondent `Settled::Unsupported` et gardent
  l'ordre documenté push-avant-ack.
- **`DatabaseBatchRepository` (DATA-02).** La comptabilité de batch
  survit à un redémarrage, et `pending_jobs`/`failed_jobs` sont
  dérivés de lignes de clôture indexées `(batch_id, job_id)` plutôt
  que stockés et décrémentés - si bien qu'un job re-livré ne peut pas
  amener un batch à « terminé » pendant que ses autres jobs tournent
  encore, et le garde-fou tient à travers les processus plutôt qu'au
  sein d'un seul.
- **`/_suprnova/health/live` et `/_suprnova/health/ready`.** La
  liveness ne touche à rien ; la readiness sonde les dépendances.
  Câbler une vérification de base de données dans une sonde de
  liveness transforme un incident passager de base de données en
  redémarrage glissant de toutes les répliques, ce à quoi invitait
  l'unique endpoint précédent. `/_suprnova/health` continue de
  fonctionner exactement comme documenté.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Secret partagé optionnel pour
  la sonde de readiness, comparé en temps constant. Sans lui, la
  readiness répond 404 - indiscernable d'un chemin non routé, parce
  que c'est *le* 404 du routeur lui-même. Non défini par défaut pour
  que les sondes existantes continuent de fonctionner.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, avec `ssl`
  et `null` acceptés comme alias compatibles Laravel. Non défini
  dérive des identifiants, reproduisant exactement le comportement
  précédent. Cela rend aussi accessible le TLS implicite sur le port
  465 : le transport le supportait, mais aucune combinaison de
  variables d'environnement ne pouvait le sélectionner.
- **`SERVER_MAX_CONNECTIONS` et `SERVER_HEADER_READ_TIMEOUT`**
  documentées dans `manual/env-vars.md`, où elles avaient été
  entièrement absentes.

### Modifié

La conclusion de l'audit lui-même était que le gate passait en 470s
et n'attrapait aucun des 19 P1. La majeure partie du travail de tests
de cette version vise cela.

- **Postgres tourne dans le gate.** Douze tests répartis sur six
  fichiers ne s'étaient jamais exécutés. Deux d'entre eux visaient en
  réalité un `DROP TABLE` sur n'importe quel Postgres présent par
  défaut sur `localhost:5432`, et aucun des deux n'avait jamais
  initialisé `Crypt`, si bien que les deux échouaient la première
  fois qu'ils s'exécutaient.
- **Les assertions de scaffold lisent les octets qu'un utilisateur
  reçoit**, après substitution, plutôt que la source du template. A
  trouvé un projet API livrant un doc comment nommant une base de
  données littéralement `{package_name}`, et un `.env.example`
  annonçant cinq clés mail que le framework ne lit jamais.
- **Injection de fautes dans la file d'attente.** La perte d'ACK, la
  re-livraison, l'expiration de bail et le dispatch partiel sont
  pilotés par un décorateur qui fait échouer une opération nommée sur
  un appel nommé, si bien que chaque cas est déterministe plutôt
  qu'une course de sleep.
- **Les adaptateurs de paiement ont des tests négatifs.** Le
  `verify()` de Stripe n'avait jamais été exercé avec une signature
  *valide*, si bien que chaque chemin de rejet qui dépend d'atteindre
  la comparaison HMAC n'était pas prouvé.
- **Le driver Pinecone parle REST.** *Cassant, derrière la feature
  `vector-pinecone` désactivée par défaut.* La motivation est sous
  **Sécurité** ; les changements de surface sont :
  - `client()` a disparu - il n'y a plus de `PineconeClient`. Le
    remplacent `control_plane_get`, `control_plane_post` et
    `data_plane_post`, qui atteignent *n'importe quel* endpoint
    Pinecone avec vos propres types de requête et de réponse, par-dessus
    le transport authentifié et à hôte résolu du driver. C'est
    strictement plus de portée que n'en avait l'ancienne échappatoire.
  - `json_to_metadata` → `metadata_from_json`, et les métadonnées sont
    désormais `serde_json::Map` plutôt que `prost_types::Struct`.
    `decode_match_fields` → `decode_match`, qui prend un
    `PineconeMatch`. `namespace()` renvoie `&str`.
  - Nouveau : `with_control_plane`, `with_api_version`,
    `with_index_host` (épingle un hôte connu et saute l'aller-retour
    vers le control plane), `index_host`, et les types de wire
    `PineconeVector` / `PineconeMatch`.
  - `from_env` lit toujours `PINECONE_API_KEY` et
    `PINECONE_CONTROLLER_HOST`, et désormais aussi
    `PINECONE_API_VERSION`.
  - La version de l'API REST est épinglée, pas flottante - `2025-04`,
    la version contre laquelle les formes de requête et de réponse du
    driver ont été écrites.
  - Plus rien ne sérialise. L'ancien driver mettait en cache un
    `Index` par nom derrière un `tokio::Mutex` parce que
    `pinecone-sdk` ne l'exposait que derrière `&mut self` ; le nouveau
    met en cache une chaîne d'hôte et partage le pool de connexions de
    `reqwest`.
  - Un hôte appris depuis le control plane est toujours contacté en
    `https`, quel que soit le schéma que porte la réponse.
  - `Debug` est implémenté à la main avec la clé API expurgée, si bien
    qu'un `#[derive(Debug)]` sur une struct détenant un driver ne peut
    pas l'imprimer.
- **Tests de contrat wire pour Pinecone.** Les tests d'intégration
  live ont besoin d'une `PINECONE_API_KEY` et ne peuvent donc pas
  tourner dans le gate - ce qui laissait les noms de champs d'une
  réécriture REST (`topK`, `includeMetadata`, `vectorCount`) reposer
  sur rien. Treize tests pilotent désormais le driver contre un fake
  `wiremock` local et vérifient la méthode, le chemin, les en-têtes
  et le corps JSON exacts qu'il met sur le réseau, plus qu'un non-2xx
  n'est jamais décodé comme un résultat et qu'un message d'erreur ne
  porte jamais la clé API. Ils épinglent le driver au contrat
  *documenté* de Pinecone ; seuls les tests `#[ignore]` peuvent
  confirmer que la documentation correspond au service live.

## 0.7.2 - 2026-07-28

### Corrigé

- **`generate-types` résout les structs de props imbriquées sans
  derives.** Le générateur de la 0.7.1 dégradait vers `unknown` tout
  champ de prop dont le type ne dérivait pas `InertiaProps`/`Data` -
  si bien que ré-exécuter le générateur (ou le watcher de
  `suprnova serve`) sur un projet avec un fichier de types commité
  remplaçait de vraies interfaces comme `Array<AdminArticleRow>` par
  `unknown` et cassait le type-checking à travers toute l'app. Les
  structs simples définies n'importe où dans `src/` se résolvent
  désormais vers leurs vraies interfaces, transitivement depuis les
  racines de props ; `unknown` (avec un avertissement) est réservé
  aux types que le projet ne définit vraiment pas - types de crates
  externes, enums, tuple structs.

### Modifié

- **La génération de `routes.ts` est opt-in.** `generate-types` ne
  dépose plus `frontend/src/types/routes.ts` dans chaque projet sans
  qu'on le demande ; passez `--routes` pour le générer.

- **Dépendances des starters frontend rafraîchies.** Les nouveaux
  scaffolds de `suprnova new` épinglent désormais des versions
  courantes : Vite ^8.1.5, Tailwind CSS ^4.3.3, Svelte ^5.56.8
  (vite-plugin-svelte ^7.2.0, svelte-check ^4.7.4), React ^19.2.8
  (plugin-react ^6.0.4), Vue ^3.5.40 (plugin-vue ^6.0.8,
  vue-tsc ^3.3.8), et `@types/node` ^24 (la ligne de types Node 24
  LTS). TypeScript reste délibérément à ^6.0.3 : c'est la dernière
  6.x, et l'intervalle de peer de svelte-check (`^5 || ^6`) n'admet
  pas encore TypeScript 7. Les trois starters ont été vérifiés de
  bout en bout (`npm install` + `npm run build`) contre l'ensemble
  rafraîchi.

## 0.7.1 - 2026-07-27

Une passe de correction de défauts sur le routage de file d'attente
de la 0.7.0, issue d'une revue complète post-release.

### Corrigé

- **Les jobs chaînés ne perdent plus leur file d'attente déclarée.**
  `ChainLink` capturait `max_tries`, `timeout`, et `backoff` d'un job
  au moment de la construction de la chaîne, mais pas son
  `Job::queue()`, si bien qu'un job qui atterrissait sur sa file
  déclarée quand il était poussé directement atterrissait sur
  `default` quand il était dispatché comme partie d'une chaîne - le
  palier « job » de l'ordre de résolution route → job → default
  disparaissait silencieusement pour les chaînes. La file déclarée
  est désormais capturée sur le maillon et résolue exactement comme
  un push direct. Les payloads de chaîne écrits avant cette version
  se décodent sans changement (`serde(default)`), et un maillon sans
  file déclarée se sérialise de façon identique à l'octet près à ce
  que la 0.7.0 écrivait.
- **Les enregistrements de jobs échoués portent la file sur laquelle
  le job est mort.** Le chemin de mise en lettre morte du worker
  codait en dur `queue = "default"` dans chaque enregistrement
  `FailedJob`, si bien que les échecs d'un job routé étaient
  invisibles pour un opérateur filtrant le magasin des échecs par le
  pool qui les possède. L'enregistrement porte désormais la file de
  l'enveloppe (`default` pour les jobs non routés).
- **La note de mise à niveau de la 0.7.0 sous-estimait la migration
  de `jobs`.** Elle disait « les workers non filtrés ne sont pas
  affectés et n'ont besoin d'aucune migration », mais
  `DatabaseQueueDriver::push` nomme la colonne `queue` dans son
  `INSERT` que le job soit routé ou non - un binaire 0.7.0 contre une
  table non migrée fait échouer **chaque push**, filtré ou non. La
  section 0.7.0 ci-dessous et `manual/queues.md` sont corrigées : sur
  le driver de base de données, l'`ALTER TABLE` est requis pour
  chaque déploiement, et il doit s'exécuter avant que les binaires ne
  roulent (les binaires plus anciens listent leurs colonnes
  explicitement, migrer d'abord est donc sûr).

- **Le README n'annonce plus de macro `#[job]`.** Cette macro
  n'existe pas - les jobs implémentent le trait `Job`. La ligne sur
  les files d'attente décrit désormais la vraie surface, y compris le
  routage de file de la 0.7.0.

### Modifié

- **Le chemin de release bump désormais les références de version du
  README.** `bump-workspace-version.py` réécrit le tag d'installation
  épinglé du README, l'exemple de modèle de distribution, et la ligne
  MSRV atomiquement avec les manifestes, et un README reformulé qui
  cesse de correspondre à un motif fait échouer la release
  explicitement. Le README annonçait la v0.6.0 depuis la sortie de la
  v0.7.0 parce que rien dans le chemin de release ne le touchait.
- **Le routage de connexion est documenté comme étant seulement une
  résolution de nom.** `Job::connection()` et le champ connection de
  `Queue::route` résolvent le *nom* de connexion porté par les
  événements de cycle de vie `JobQueueing` / `JobQueued` ; un unique
  driver global au processus reçoit toujours chaque push, si bien
  qu'ils ne sélectionnent pas un driver différent. Le rustdoc et
  `manual/queues.md` sous-entendaient auparavant une sélection de
  driver qui n'existe pas. La dimension file d'attente n'est pas
  affectée - elle est honorée de bout en bout. Les drivers par
  connexion restent un travail futur.
- `ChainLink` a gagné un champ public `queue: Option<String>`, ce qui
  casse la construction par littéral de struct des maillons de
  chaîne. Les maillons construits via `ChainLink::from_job` - le
  chemin normal - ne sont pas affectés.

### Mise à niveau

En venant de ≤ 0.6.x sur le driver de file d'attente base de données,
appliquez la migration 0.7.0 ci-dessous **avant** de rouler les
binaires ; elle est requise pour chaque déploiement sur ce driver, pas
seulement ceux utilisant `--queue`. La 0.7.1 elle-même n'a besoin
d'aucune migration.

## 0.7.0 - 2026-07-26

### Sécurité

- **`ammonia` mis à niveau vers 4.1.4 (RUSTSEC-2026-0213).** Les
  versions jusqu'à 4.1.3 incluse permettent un XSS via les balises
  d'animation SVG `animate` et `set`. `ammonia` est le sanitizer à la
  fin du pipeline Markdown de Suprnova (`comrak` → `syntect` →
  `ammonia`), si bien que toute app rendant du Markdown fourni par
  l'utilisateur via `content` était exposée. L'avis a été publié le
  2026-07-21 - après la sortie de la v0.6.5 - si bien que **chaque
  version jusqu'à la v0.6.5 incluse est affectée**. Mettre le
  framework à niveau est le correctif ; aucun changement de code
  applicatif n'est requis.

### Ajouté

- **Routage de file d'attente.** Les jobs peuvent être dispatchés vers
  une file d'attente et une connexion spécifiques, et les workers
  peuvent être dédiés à des files spécifiques - la surface
  `Queue::route(...)` de Laravel 13, typée. Un job déclare sa propre
  maison avec `Job::queue()` / `Job::connection()` ; un opérateur la
  surcharge de façon centralisée avec
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` dans
  `bootstrap::register()`, sans modifier le job. La résolution est
  route, puis job, puis default global, et un champ `None` dans une
  route diffère plutôt que d'effacer. `queue:work --queue=billing,default`
  ne vide que ces files. Les jobs non routés appartiennent à
  `default`, si bien qu'ils ne sont jamais abandonnés. Les jobs
  chaînés résolvent les routes par nom, puisqu'un maillon de chaîne
  stocke son job avec le type effacé.
- **`QueueDriver::pop_from`.** Un pop filtrant, avec une
  implémentation par défaut qui **rejette** un filtre qu'elle ne peut
  pas honorer plutôt que de vider silencieusement chaque file - un
  worker à qui l'on a dit de vider `billing` et qui vide tout en
  silence est indiscernable d'un déploiement qui fonctionne jusqu'à
  ce que le mauvais pool avale les mauvais jobs. Les drivers mémoire
  et base de données filtrent nativement. Les drivers personnalisés
  continuent de compiler et héritent du défaut explicite.
- **Schéma de la table `jobs` documenté.** `manual/queues.md` porte
  désormais le DDL que `DatabaseQueueDriver` attend réellement, ce qui
  n'était auparavant découvrable qu'en lisant le SQL du driver.
- **Option `serverHead` d'Inertia documentée.** Les éléments `<head>`
  pilotés par le serveur (Inertia 3.5.0) n'ont besoin d'aucun support
  du framework : le client les lit depuis une prop ordinaire, si bien
  que n'importe quel handler peut déjà les fournir. Voir
  `manual/frontend-inertia-responses.md`.

### Modifié

- `Envelope` a gagné un champ `queue: Option<String>`. Il est
  `serde(default)` et sauté quand absent, si bien qu'une enveloppe non
  routée se sérialise de façon identique à l'octet près à ce que les
  versions précédentes écrivaient - le test de format wire figé passe
  sans changement, il n'y a pas de bump de `schema_version`, et les
  flottes de versions mixtes interopèrent pendant une mise à niveau
  glissante.
- `WorkerConfig` a gagné un champ `queues: Vec<String>` (vide = tout
  vider, le comportement précédent).
- `ROADMAP.md` supprimé. Ses principes de conception vivent dans
  `manual/introduction.md`, l'accord de travail dans
  `manual/contributions.md`, et le matériel de déploiement et de
  scale-out dans `manual/deployment.md` ; les checklists
  livré/planifié étaient devenues obsolètes. Le pointeur de
  `README.md` vers lui pour « la relation avec upstream » était déjà
  pendant - cette attribution vit dans `LICENSE`.
- Les frontends de scaffold épinglent désormais
  `@inertiajs/{svelte,react,vue3}` à `^3.6.1` (depuis `^3.4.0`).
  L'intervalle 3.4.0 → 3.6.1 est seulement côté client - audité
  contre le changelog upstream et le contrat `Page` dans
  `packages/core/src/types.ts`, chaque en-tête `X-Inertia-*` envoyé
  par le client 3.6.1 était déjà géré.
- `scripts/release.sh` publie désormais lui-même la release GitHub,
  avec des notes tirées de la section `CHANGELOG.md` de la version.
  Auparavant, c'était une « étape suivante » manuelle qui se faisait
  sauter, ce pourquoi la v0.5.10 et la v0.6.1–v0.6.3 sont tag-only et
  la page Releases était restée sur une version obsolète. Le
  preflight s'exécute avant le gate si bien qu'un `gh` ou une section
  de changelog manquants échouent en quelques secondes, et la
  publication est sautée automatiquement à moins que `origin` ne soit
  GitHub.

### Mise à niveau

Les tables `jobs` existantes sur le driver de file d'attente base de
données **doivent** ajouter la nouvelle colonne - `push` la nomme dans
son `INSERT` que le job soit routé ou non, si bien qu'une table non
migrée fait échouer chaque push. Migrez d'abord, puis roulez les
binaires (les binaires plus anciens listent leurs colonnes
explicitement et ignorent la nouvelle, cet ordre est donc sûr) :

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corrigé dans la 0.7.1 - cette note prétendait à l'origine que les
déploiements non filtrés n'avaient besoin d'aucune migration.)*

## 0.6.5 - 2026-07-21

### Ajouté

- **Checkout ponctuel hébergé dans l'adaptateur Stripe.**
  `Checkout::start_session` avec `SessionMode::OneOff` et des
  `price_refs` non vides crée désormais une Checkout Session
  hébergée (`mode=payment`, une ligne par référence de prix,
  `allow_promotion_codes=true`) et renvoie
  `SessionPayload::StripeCheckoutRedirect`. Le chemin Elements avec
  seulement `amount_hint` est inchangé ; les deux formes sont choisies
  par requête.
- **Support de Stripe Managed Payments (merchant-of-record).**
  `StripeProvider::with_managed_payments(true)` - ou
  `STRIPE_MANAGED_PAYMENTS=true` dans `from_env()` - envoie
  `managed_payments[enabled]=true` à la création d'une session
  ponctuelle hébergée. Désactivé par défaut ; le champ est entièrement
  omis si bien que les comptes non inscrits ne sont pas affectés.
- **`Checkout::session_status`.** Nouvelle méthode de trait (défaut :
  `PaymentError::NotSupported`) rapportant l'état côté fournisseur
  d'une session sous la forme du nouvel enum neutre
  `CheckoutSessionState` (`Open` /
  `Complete { paid, payment_ref, amount_total }` / `Expired`). L'impl
  Stripe mappe `GET /v1/checkout/sessions/{id}` ; `payment_ref` porte
  l'id `PaymentIntent` de la session pour la corrélation avec la
  table miroir. C'est la primitive de vérification côté serveur pour
  les pages de retour de redirection et les passes de réconciliation.
- **Trait de capacité `Promotions`.** `create_promotion_code` émet un
  code restreint à un client, expirant en option, plafonné en
  rédemptions, à partir d'un coupon pré-créé. Interrogé via le
  nouveau `PaymentProvider::as_promotions()` (défaut `None`).
  Implémenté pour Stripe (`POST /v1/promotion_codes`) et le mock.
- **Mises à niveau de `MockPaymentProvider` pour ce qui précède.**
  Enregistre chaque requête `start_session` (`recorded_sessions()`),
  scripte `session_status` par id de session
  (`script_session_status()` - les sessions connues non scriptées
  rapportent `Open`, les ids inconnus `NotFound`), et implémente
  `Promotions` avec des requêtes enregistrées
  (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

### Corrigé

- **Les agrégats Eloquent se décodent de façon cohérente à travers
  les backends de base de données.** Les expressions `count`, `sum`,
  `avg`, `min`, et `max` générées utilisent désormais un unique alias
  de résultat interne stable. PostgreSQL ne renvoie plus de faux
  zéros ou de `None` parce que son driver étiquette les colonnes
  d'agrégat différemment de SQLite, et les erreurs de colonne
  manquante ou de type incompatible se propagent désormais au lieu
  d'être défaultées silencieusement.
- **Les suppressions en masse ne peuvent pas utiliser d'expressions
  de table fournies par l'appelant.** Le SQL de suppression
  exécutable dérive toujours sa cible du `M::TABLE` statique et
  validé du modèle. L'argument public historique du renderer reste
  compatible au niveau source mais ne peut plus rediriger ou injecter
  la cible de suppression.

## 0.6.3 - 2026-07-15

### Ajouté

- **Les lectures brutes typées peuvent rester sur la connexion
  épinglée d'une transaction.** `Transaction::backend()` expose le
  backend actif et `Transaction::query_all(Statement)` exécute du SQL
  d'agrégat typé ou personnalisé à travers la transaction tout en
  préservant l'instrumentation `QueryExecuted`. Les applications n'ont
  plus besoin d'une requête au niveau du pool ou d'un accès à un
  exécuteur privé quand une décision à portée de verrou dépend de
  colonnes de résultat calculées.

## 0.6.2 - 2026-07-15

### Corrigé

- **Les prédicats bruts liés sont neutres vis-à-vis du backend.**
  `filter_raw` et `where_raw` d'Eloquent acceptent désormais des
  marqueurs de liaison `?` portables sur chaque backend de base de
  données ; le rendu PostgreSQL les rebase vers des positions `$N`
  monotones à travers les prédicats antérieurs, les sous-requêtes de
  relation, les clauses HAVING, et les branches UNION. Les fragments
  PostgreSQL numérotés existants sont normalisés selon leur ordre de
  marqueur local, tandis que les styles mixtes et les désaccords de
  nombre de liaisons échouent la validation avant tout I/O. Le
  scanner conscient du SQL préserve les points d'interrogation à
  l'intérieur des chaînes entre guillemets, des identifiants, des
  commentaires, et des corps dollar-quotés ; `??` émet un opérateur
  point d'interrogation littéral dans un fragment brut lié.

## 0.6.1 - 2026-07-15

### Ajouté

- **Nettoyage de session supervisé et observable.**
  `SessionMiddleware::install` utilise la cadence configurable
  `SESSION_GC_INTERVAL` (une heure par défaut), tandis que
  `session_gc_metrics()` expose des horodatages d'exécution, de
  succès, d'échec, de lignes supprimées, et de dernier résultat,
  locaux au processus, pour les surfaces d'opérations protégées.
- **Touches de session glissante bornées.** `SESSION_TOUCH_INTERVAL`
  contrôle la cadence minimale d'écriture d'activité (cinq minutes
  par défaut) et est plafonné à la moitié de la durée de vie de la
  session, si bien que les sessions actives ne peuvent pas expirer
  entre deux touches.

### Corrigé

- **Les requêtes sans état ne créent plus de sessions durables.** Les
  requêtes sans cookie de session valide n'effectuent aucune lecture
  ni écriture sur le magasin de sessions et ne reçoivent aucun cookie
  de session à moins que le traitement ne crée de l'état. Les
  sessions propres existantes évitent les upserts inconditionnels et
  le churn de cookies, les cookies hérités migrent à leur prochaine
  requête, et les cookies dont les lignes sous-jacentes ont expiré
  sont effacés sans recréer de sessions vides.

## 0.6.0 - 2026-07-10

### Ajouté

- **Sous-systèmes du framework en opt-in, avec des défauts
  rétrocompatibles.** Le stockage du système de fichiers, les drivers
  de base de données SQLite/Postgres/MySQL, le driver vectoriel
  MariaDB, et Web Push ont désormais des features Cargo explicites.
  Les builds par défaut existants conservent toutes ces capacités,
  tandis que les consommateurs `default-features = false` peuvent
  sélectionner zéro driver ou seulement la surface
  stockage/base de données/vecteur/push qu'ils utilisent. La matrice
  de features exécutable vérifie les profils zéro-driver,
  driver-individuel, Nation X minimal, défaut, et toutes-features.
- **Import brut de clé privée VAPID P-256.** `VapidKey::from_bytes`
  accepte un scalaire P-256 big-endian de 32 octets validé, en plus
  du chemin d'import/export PKCS#8 PEM existant.

### Modifié

- **Les JWT VAPID sont désormais signés directement avec P-256.** Web
  Push sérialise désormais l'en-tête/les claims ES256 de la RFC 8292
  et les signe avec `p256`, supprimant la dépendance JWT générique
  tout en préservant les clés générées, les allers-retours PEM,
  l'encodage de clé publique, et la borne de durée de vie de 24
  heures.
- **Rafraîchissement des dépendances de sécurité.** Mise à jour des
  dépendances vulnérables du framework, dont bcrypt et ammonia, et
  réduction des features activées de Comrak tout en conservant la
  coloration syntaxique.
- **Rust 1.91.1 est le MSRV de la release.** Chaque package du
  workspace déclare le même `rust-version`, les Dockerfiles générés
  épinglent l'image de build correspondante, et le gate de release
  complet compile le profil filesystem supporté avec la toolchain
  Rust 1.91.1 exacte.
- **Épinglage de sécurité OpenDAL 0.58.** La feature filesystem
  épingle le commit `eas4ai/opendal`
  `88717391eb72c9839d3f8e79fccad9f22fc3a1b4`, un fork minimal basé
  exactement sur le commit officiel Apache OpenDAL
  `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a`. Le fork ne change que
  les déclarations Reqsign utilisées par le cœur d'OpenDAL plus S3,
  GCS, et Azure Blob, si bien que les consommateurs en aval résolvent
  le commit officiel Apache Reqsign
  `b49cd2996b9d2d9944e84481f8835ff55b188b97` et `quick-xml` 0.41.0.
  Un fork est nécessaire parce que les patches Cargo racine d'un
  dépôt de dépendance ne se propagent pas aux consommateurs ; le
  graphe publié pourrait sinon restaurer le `quick-xml` 0.38/0.40
  vulnérable.

### Corrigé

- **Métadonnées de version de release atomiques.** Le bump de
  release met désormais à jour `workspace.package.version` et chaque
  dépendance de chemin interne versionnée en une seule opération
  validée, stage chaque manifeste affecté, et prouve un workspace
  `0.6.0` temporaire avec `cargo check --workspace` avant la release.
  Les versions de release sont validées comme du SemVer 2.0 strict, y
  compris la règle du zéro non significatif pour les prereleases
  numériques. Des smokes jetables agnostiques à la version et sans
  remote prouvé dérivent une release patch ultérieure à la fois
  depuis la source actuelle et depuis une source déjà en `0.6.0`,
  rejettent les arbres de release staged/unstaged/untracked avant le
  gate, prouvent que la publication atomique commit/tag fait reculer
  les deux refs quand un tag est rejeté, et prouvent la séquence de
  release normale sans toucher au vrai remote. Les versions de
  release doivent augmenter selon la préséance SemVer, y compris les
  transitions de prerelease. Les artefacts de build des smokes
  restent toujours à l'intérieur de leur workspace temporaire, en
  ignorant tout `CARGO_TARGET_DIR` appelant.
- **Le rustdoc couvre chaque frontière de feature supportée.** Le
  module OAuth pointe vers le `OAuthAuth::complete` public, et la
  matrice exécutable construit le rustdoc zéro-driver, défaut, et
  toutes-features sans dépendances.
- **La validation de flux du système de fichiers est à portée de
  session.** Les writers, listers, et copiers du système de fichiers
  local résolvent et confinent leurs chemins une fois avant le
  premier I/O plutôt qu'une fois par chunk/élément, tandis que les
  opérations `close`/`abort` activées atteignent toujours le backend
  pour le nettoyage. Le confinement de traversée et de symlink
  existant reste appliqué pour un système de fichiers de confiance ;
  les vérifications canonicalize-puis-open n'éliminent pas les
  courses contre un principal qui modifie l'arbre en même temps.

### Sécurité

- **Le gate de release échoue fermé.** `release.sh` délègue au gate
  complet canonique avant d'éditer les manifestes ou de créer des
  commits/tags ; ce gate exécute toujours `cargo audit`, traite un
  binaire `cargo-audit` manquant comme une erreur, et s'arrête sur
  tout échec d'audit. Il construit et audite aussi un consommateur
  filesystem en aval isolé, en vérifiant les révisions source
  OpenDAL/Reqsign exactes et l'absence de `quick-xml` en dessous de
  0.41. Aucune nouvelle exception d'avis n'a été ajoutée.

## 0.5.10 - 2026-07-03

### Corrigé

- **`generate-types` ne fait plus disparaître les structs
  auto-référentes.** Une struct avec un champ qui référence son
  propre type (un nœud d'arbre avec `children: Vec<Self>`, par ex.
  une vue de commentaires en fil) créait un self-edge dans le graphe
  de dépendances de types, épinglant son degré entrant au-dessus de
  zéro si bien que le tri topologique de Kahn ne l'émettait jamais -
  laissant chaque interface qui la référençait avec un nom de type
  pendant qui faisait échouer `svelte-check`/`tsc`. Les self-edges
  sont désormais retirés avant le tri, et toute struct piégée dans un
  cycle de référence (récursion mutuelle) est émise dans un ordre
  arbitraire plutôt que d'être abandonnée, puisque les interfaces TS
  peuvent se référencer mutuellement indépendamment de l'ordre de
  déclaration.

## 0.5.9 - 2026-07-01

### Ajouté

- **`MAIL_FROM_NAME` - nom d'affichage optionnel sur les e-mails de
  flux d'auth.** Les mailables de vérification d'e-mail, de
  réinitialisation de mot de passe, et de changement de mot de passe
  rendent désormais leur en-tête `From` comme `"Name <address>"` quand
  `MAIL_FROM_NAME` est défini (lu au moment de l'envoi si bien qu'il
  survit à l'aller-retour serde de la file d'attente). `MAIL_FROM`
  reste une adresse nue ; laisser `MAIL_FROM_NAME` non défini ou vide
  garde le comportement précédent d'adresse nue. Aucun changement à
  aucun site d'appel - les mailables lisent la variable d'env
  elles-mêmes.

## 0.5.8 - 2026-06-30

### Corrigé

- **Les helpers de routes de `generate-types` sont toujours du
  TypeScript valide.** Quand plusieurs routes d'un module partagent
  un handler (par ex. une liste blanche `static_files::serve`
  mappant de nombreuses URLs de favicon/asset), la première gardait
  le nom du handler et les autres recevaient une clé dérivée du
  chemin de route - mais le chemin n'était que partiellement assaini
  (`/ { } -` → `_`), si bien qu'une extension de fichier laissait
  fuir un `.` dans la clé : `favicon_16x16.png: (...) => ...`. C'est
  un accès de membre, pas un nom de propriété, si bien que
  `tsc`/`svelte-check` rejetait le `routes.ts` généré. Les clés
  dérivées sont désormais assainies vers des identifiants légaux -
  chaque caractère non alphanumérique devient `_` et un chiffre en
  tête est préfixé - si bien que `favicon-16x16.png` →
  `favicon_16x16_png` et `2fa.json` → `_2fa_json`. Les noms de
  handler uniques restent intacts.

## 0.5.7 - 2026-06-30

### Corrigé

- **`generate-types` n'émet plus de références de type pendantes.**
  Un champ de prop dont le type est une struct qui ne dérive pas
  `InertiaProps`/`Data` (ou un type externe que le générateur ne
  peut pas voir) était émis comme un identifiant nu - par ex.
  `user: UserInfo` - produisant du TypeScript qui échoue à
  `tsc`/`svelte-check` parce que cette interface n'est jamais écrite.
  De telles références se dégradent désormais vers `unknown`
  (`user: unknown` ; `Vec<T>` → `Array<unknown>` ; `Option<T>` →
  `unknown | null`), si bien que la sortie générée passe toujours le
  type-checking, et `generate-types` affiche un avertissement nommant
  le type non résolu et le champ qui le référence, avec le correctif
  (dériver `InertiaProps`/`Data` dessus). Les paramètres génériques et
  les types `InertiaProps`/`Data` imbriqués résolus ne sont pas
  affectés.

## 0.5.6 - 2026-06-29

### Modifié

- **Connexion avec Apple : vérification JWKS RS256.** Bump de
  `suprnova-apple-rs` vers v0.3.1 - les ID tokens Apple sont désormais
  vérifiés contre le JWKS publié par Apple (RS256) plutôt que d'être
  approuvés structurellement.

## 0.5.5 - 2026-06-28

### Ajouté

- **Objectif de token `MagicLink`.** Nouvelle variante `MagicLink` sur
  l'enum `TokenPurpose` du flux d'auth, pour les tokens de connexion
  par lien magique sans mot de passe.

## 0.5.4 - 2026-06-28

### Modifié

- **Complétion OAuth composable.** Scission de la complétion OAuth
  générique en `verify_oauth_identity` (vérifier + résoudre
  l'identité) et un `complete` fin, si bien que les apps peuvent
  vérifier une identité OAuth sans déclencher tous les effets de bord
  de la complétion de session.

## 0.5.3 - 2026-06-28

### Corrigé

- **Métadonnées de version de workspace corrigées.** La v0.5.2 a été
  taguée et poussée avant que son bump de version `Cargo.toml` ne
  soit stagé, si bien que le tag v0.5.2 poussé porte encore
  `version = "0.5.1"`. La v0.5.3 recoupe la release avec la bonne
  version de workspace - aucun changement de code (la scission OAuth
  de la v0.5.2 n'est pas affectée).

## 0.5.2 - 2026-06-28

### Modifié

- **Complétion Apple composable.** Scission de la complétion Sign-In
  with Apple en `verify_apple_identity` + un `complete_apple` fin, à
  l'image de la scission OAuth générique. (Note : le tag v0.5.2
  poussé porte un champ de version `0.5.1` obsolète - corrigé en
  v0.5.3.)

## 0.5.1 - 2026-06-28

### Modifié

- **Crate Apple renommée.** Repointe la dépendance Apple vers le
  dépôt renommé `suprnova-apple-rs`.

## 0.5.0 - 2026-06-28

### Ajouté

- **Connexion avec Apple.** Échange de token OAuth + vérification
  d'ID-token + upsert utilisateur pour Apple ; endpoints well-known
  d'Apple et le mode de réponse `form_post` ; champs spécifiques à
  Apple sur `OAuthProviderConfig` ; `AppleKeyPair` ré-exporté si bien
  que les apps configurent Sign-In with Apple sans dépendance directe
  à `apple`.

### Corrigé

- Omission des paramètres PKCE de l'URL d'autorisation Apple (Apple
  rejette la requête quand ils sont présents).

### Dépendances

- Consommation du correctif magic-auth de `torii` ; ajout d'`apple-rs`
  v0.3.0.

## 0.4.1 - 2026-06-26

### Performances

- Pré-dimensionnement de `MiddlewareChain` pour éliminer les
  réallocations de `Vec` par requête.

### Corrigé

- Rendre le chemin du fichier `down` de maintenance résistant aux
  collisions sous des exécutions de tests parallèles.

### Docs

- Vérification à la compilation des exemples de doc du framework
  (`ignore` → `no_run`) ; réconciliation des notes de distribution
  avec les GitHub Releases taguées ; exclusion de tout l'arbre
  `docs/`.

## 0.4.0 - 2026-06-22

### Modifié

- **La distribution est suivie par git ; vous n'épinglez pas de
  tags.** Les apps scaffoldées dépendent de
  `suprnova = { git = "…/suprnova.git" }` et suivent la branche par
  défaut ; récupérez les mises à jour avec `cargo update -p suprnova`.
  Les versions sont publiées comme des GitHub Releases taguées
  (`v0.4.0`, …) pour le changelog, mais `Cargo.lock` épingle déjà le
  commit résolu exact - si bien que les builds restent reproductibles
  sans épingler à la main un `tag` ou un `rev`. La documentation
  d'installation ne présente plus l'épinglage de commit comme le
  chemin de mise à jour.

## 0.3.0 - 2026-06-21

### Ajouté

- **Instrumentation de requêtes pour les lectures Eloquent** -
  `Builder::get`, `Model::find`, `find_many`, et `all` émettent
  désormais `QueryExecuted`, si bien que les SELECT de modèle et les
  requêtes d'eager-load apparaissent dans `DB::listen` et le journal
  de requêtes en mémoire aux côtés des écritures et des requêtes
  brutes. Ajoute le terminal de lecture instrumenté
  `ExecutorChoice::statement_all`.
- **Autorisation de resource-route** -
  `ResourceRoutes::authorize_resource::<U, R>()` attache la
  vérification d'ability conventionnelle à chaque route de ressource
  générée comme middleware par route (parité avec `authorizeResource`
  de Laravel). La map action→ability est `index`/`show` → `view`,
  `create`/`store` → `create`, `edit`/`update` → `update`,
  `destroy` → `delete`. Un seul appel filtre toute la surface à sept
  actions plutôt que de compter sur chaque corps de contrôleur pour
  se souvenir d'un `Gate::authorize`.
- **Hit de limite de débit atomique** -
  `RateLimiter::hit_and_check(key, max, decay)` incrémente une
  fenêtre fixe et la teste en un seul aller-retour, renvoyant si le
  seau est désormais au-dessus de sa limite (`i64::MAX` signifie
  illimité).
- **Helper de comparaison en temps constant** - `constant_time_eq(a, b)`
  (adossé à `subtle`) pour la vérification de signature de webhook ;
  la doc de `WebhookHandler::verify` impose désormais une comparaison
  de digest en temps constant.
- **Client Inertia vers 3.4.0** - les scaffolds Svelte/React/Vue
  épinglent désormais `@inertiajs/{svelte,react,vue3}` à `^3.4.0`
  (depuis `3.1.1`), récupérant les modes `router.poll`, `usePoll`
  dynamique, `Inertia.once`, le correctif d'annulation
  d'`InfiniteScroll`, et l'`onSuccess` de `Form` attendu. Le serveur
  émet déjà l'objet de page et la surface d'en-têtes complets de la
  3.4.0 (once-props, la famille de scroll prepend/deep-merge,
  `matchPropsOn`, props rescued/shared), il s'agit donc d'un bump de
  fraîcheur client sans changement de protocole.
- **Plafond de connexions optionnel** - `SERVER_MAX_CONNECTIONS` (et
  le `Server::max_connections(n)` programmatique) borne les
  connexions actives concurrentes avec un sémaphore sur la boucle
  d'accept, appliquant de la back-pressure au niveau TCP. Non défini -
  ou `0` - laisse les connexions non bornées (le défaut, inchangé).
  Un filet de sécurité à associer à un reverse proxy et à
  `LimitNOFILE`, pas un remplacement pour la limitation de débit en
  amont.
- **Désactivation du suivi de redirection** -
  `RequestBuilder::no_redirects()` route une requête à travers un
  client HTTP qui ne suit pas les redirections, si bien qu'un `3xx`
  est renvoyé tel quel plutôt que poursuivi. Utilisez-le quand l'URL
  de la requête est influencée par une entrée non fiable, pour fermer
  un vecteur SSRF basé sur la redirection (un endpoint hostile
  redirigeant vers un hôte interne ou de métadonnées cloud). Le
  client par défaut continue de suivre les redirections, conformément
  à la convention des clients généralistes.

### Sécurité

- **Les resource routes** échouent fermées sur le downcast à type
  effacé du registre d'autorisation plutôt que de paniquer, et les
  refus d'`authorize_resource` / les requêtes non authentifiées sont
  refusés avant que le handler ne s'exécute.
- **Le limiteur de débit** ferme une course check-then-hit à fenêtre
  fixe en incrémentant et comparant atomiquement (`hit_and_check`).
- **Le middleware `RateLimited` de la file d'attente** admet
  désormais les jobs via ce `hit_and_check` atomique plutôt que via
  une paire séparée `too_many_attempts` + `hit`, si bien que
  des workers concurrents ne peuvent plus tous passer la vérification
  de budget avant qu'aucun d'eux n'incrémente, et sur-admettre
  au-delà de `max_attempts`.
- **Les validateurs de téléversement** (`mimetypes` / `mime`)
  sniffent le contenu des octets téléversés plutôt que de faire
  confiance au `Content-Type` fourni par le client.
- **Le garde-fou de chemin du système de fichiers** canonicalise les
  chemins pour attraper une traversée par symlink hors de la racine
  de stockage, au-delà des vérifications lexicales précédentes `../`
  / absolu / UNC.
- **L'auth** ferme un oracle temporel de connexion sans mot de passe -
  un compte trouvé mais sans mot de passe auquel un mot de passe est
  fourni exécute désormais une vérification à coût fixe, à travers
  les fournisseurs d'utilisateurs Eloquent et base de données - et
  `dummy_verify` pilote le hasher configuré si bien que le chemin
  utilisateur-non-trouvé est en temps constant.
- **Eloquent** valide les identifiants de colonne sur les chemins de
  projection `pluck` / `value` / `pluck_keyed` / `sole_value` et
  `sum` / `avg` / `min` / `max`.
- **Paiements** - le vérificateur du provider mock échoue fermé en
  dehors d'un environnement de développement, et les IP source des
  webhooks se résolvent via `TrustedProxiesConfig` (`req.ip()`)
  plutôt que via un en-tête `X-Forwarded-For` brut.
- **Le garde-fou de chemin du système de fichiers** remonte désormais
  jusqu'à l'ancêtre *existant* le plus proche quand une cible
  d'écriture n'existe pas encore, fermant une évasion par symlink où
  un symlink intermédiaire planté avec un parent immédiat manquant se
  glissait devant le garde-fou.
- **`DB::init_with`** valide l'environnement avant de se connecter (à
  l'image de `DB::init`), si bien que le repli SQLite de dev ne peut
  plus démarrer silencieusement en production par ce point d'entrée.
- **Le service de fichiers statiques** rejette les dotfiles (`.env`,
  `.git/config`, `.htpasswd`, tout segment commençant par un `.`),
  pas seulement la traversée `.`/`..`.
- **Les webhooks de paiement** sérialisent les retries concurrents du
  même événement non traité avec un verrou `FOR UPDATE` + une
  revérification, et traitent les violations d'unicité de la table
  miroir comme des « déjà appliqué » bénins ; `payments_subscription_items`
  gagne un `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** fixe par défaut le discriminant de modèle au nom de type
  entièrement qualifié, si bien que deux types authentifiables
  partageant un nom feuille ne peuvent plus hériter des
  rôles/permissions l'un de l'autre.
- **`invalidate_session()`** fait tourner l'id de session (pas
  seulement un flush), fermant une brèche de fixation de session ; le
  middleware `WithoutOverlapping` de la file d'attente relâche son
  verrou de cache même quand le job panique.
- **Les providers mail** plafonnent la lecture du corps des réponses
  d'erreur (8 KiB), à l'image du client web-push, si bien qu'un
  endpoint hostile ne peut pas piloter la mémoire de l'expéditeur.
- **Web push** désactive le suivi de redirection HTTP sur le client
  par défaut, si bien qu'un endpoint push influencé par un attaquant
  ne peut plus rediriger en `3xx` un POST de notification vers un
  hôte interne ou de métadonnées cloud (SSRF). Une redirection remonte
  désormais comme un push rejeté plutôt que comme une requête suivie
  silencieusement.
- **L'adaptateur Stripe** `Debug` expurge le secret de signature de
  webhook *et* affiche un placeholder pour le `stripe::Client` (qui
  porte la clé secrète API dans son en-tête d'auth), si bien
  qu'aucun des deux secrets ne peut atteindre les logs via un `{:?}`
  de `StripeProvider`, indépendamment du propre `Debug` du client
  upstream.
- **L'adaptateur Stripe** `from_env` rejette les identifiants
  présents mais vides, échouant fermé plutôt que de construire un
  client avec un secret HMAC de webhook vide (et donc forgeable).
- **La vérification d'e-mail OAuth** échoue fermée pour les providers
  non reconnus : un payload userinfo portant un `email` mais aucun
  flag `email_verified` n'est plus traité comme vérifié. Un provider
  inconnu doit désormais affirmer `email_verified: true` ou exposer
  un endpoint d'e-mails vérifiés, fermant un vecteur de
  liaison/prise de compte pour les apps qui indexent les comptes sur
  l'e-mail. Google (`true` explicite uniquement) et GitHub (vérifié
  par le contrat `/user`) sont inchangés.

### Corrigé

- **L'eager loading imbriqué** (`with(["posts.comments"])`) est
  désormais un nombre constant de requêtes - le segment final se
  charge en une seule requête `IN` groupée à travers tous les
  parents plutôt qu'une requête par parent (N+1).
- **`where_has`/`where_doesnt_have`** qualifient les colonnes de la
  closure avec la table cible, si bien qu'une colonne présente à la
  fois sur le pivot et la cible ne produit plus d'erreur de colonne
  ambiguë sur les relations many-to-many.
- **`delete`/`force_delete`/`touch` de soft-delete et le `persist` de
  factory** honorent le routage `#[model(connection = "…")]` d'un
  modèle (à l'image de `restore` et des autres chemins d'écriture) au
  lieu de retomber sur le pool primaire.
- **Le `Maybe::Missing` de JSON:API** utilise une sentinelle wire non
  collisionnable, si bien que des données utilisateur en forme de
  `{"__missing__": true}` ne sont plus dépouillées silencieusement.
- **Les notifications mises en file** honorent `should_send` (veto
  par canal) et `after_sending`, revérifiés sur le worker -
  auparavant, seul le chemin synchrone le faisait.
- **Les jobs relâchés** poussent la copie de retry avant d'acquitter
  l'original, si bien qu'une erreur de push transitoire du driver ne
  fait plus disparaître le job.
- **Les webhooks d'ajustement (remboursement) Paddle** indexent la
  mise à jour de la table miroir sur l'id de transaction référencé et
  lisent les montants depuis `data.totals`, au lieu d'insérer une
  ligne à montant zéro sous l'id d'ajustement.
- **Les URLs SQLite** portant une query string
  (`sqlite://db.sqlite?mode=rwc`) construisent une URL de connexion à
  requête unique valide et un nom de fichier sur disque propre.
- **HTTP** borne les valeurs `q` d'`Accept` à `[0,1]` et impose le
  `max_body_bytes` d'un `FormRequest` même quand le corps a été
  pré-mis-en-tampon ; **WebSocket** rejette une config
  `max_missed_pings < 2` (1 fermait chaque connexion dès son premier
  ping).
- **Cron** utilise une sémantique OR pour le jour-du-mois et le
  jour-de-la-semaine quand les deux sont restreints (parité
  Vixie/POSIX) ; **Markdown** `plain_text`/les extraits préservent la
  ponctuation espacée intentionnelle ; **`CachedEvaluator`** borne la
  croissance de son cache ; **`SupervisorRegistry::start_all`** ne
  double-spawn plus sur un second appel ; **le conteneur de test**
  récupère sur place d'un verrou empoisonné.
- **Le backoff de redémarrage du superviseur** revient au plancher de
  100 ms après une exécution qui reste up au moins le plafond de
  60 s, si bien qu'un daemon qui a tourné sainement pendant une longue
  période puis se termine redémarre promptement au lieu d'hériter
  d'un backoff qui avait grimpé pendant une rafale d'échecs
  antérieure. Une boucle de crash dont les exécutions n'atteignent
  jamais le seuil continue quand même de monter jusqu'au plafond, si
  bien que la réinitialisation ne masque jamais un superviseur qui
  flappe.
- Correction de docs obsolètes sur `filter_op` (les opérateurs sont
  validés par liste blanche), les URLs signées (pas compatibles à
  l'octet près avec les signatures absolues par défaut de Laravel),
  `UniqueIdKind::is_valid` (un helper pour l'appelant, pas câblé
  automatiquement dans `find`), et le plafond de longueur
  d'identifiant (128, pas 64).

### Documentation

- Documentation de l'autorisation de resource-route
  (`authorize_resource`) dans les chapitres routage et autorisation,
  et du compteur atomique `hit_and_check` dans le chapitre de
  limitation de débit.

## 0.2.0 - 2026-06-21

Ajoute le contrôle d'accès basé sur les rôles, un pipeline de contenu
Markdown / rendu de docs, et le service natif de fichiers statiques.

### Ajouté

- **RBAC de niveau 2** - trait `HasRoles` ; rôles + permissions avec
  une jointure `role_has_permissions` ; `PermissionMiddleware` /
  `RoleMiddleware` (tous deux fail-closed / default-deny) ; la
  migration `CreateRbacTables` ; et les helpers `create_role` /
  `create_permission` / `give_permission_to_role`.
- **Rendu de contenu** - rendu Markdown et un pipeline de build de
  docs : `MarkdownRenderer`, `build_docs`, `DocsCatalog` /
  `DocsChapter`, extraction de heading et `slugify_heading`. Le HTML
  rendu est assaini (`comrak` + `syntect` + `ammonia`).
- **Service natif de fichiers statiques** - handler de repli
  `StaticFiles::public()` pour servir un répertoire `public/` à la
  racine web, remplaçant les contrôleurs de liste blanche par asset
  faits main dans les apps.

### Corrigé

- Les apps fraîchement générées héritent d'un épinglage de
  compatibilité `time = 0.3.47` au niveau du framework, évitant des
  conflits de cohérence Rust 1.96 causés par `time 0.3.48` dans les
  résolutions de dépendances des scaffolds neufs.

### Documentation

- Documentation des deux starter kits livrés - **Nebula** (auth de
  niveau Breeze) et **Pulsar** (site produit + communauté) - à
  travers le manuel, le README, et la roadmap ; restructuration de la
  roadmap autour de la surface livrée ; et réconciliation des
  références de version à travers toute la doc.

## 0.1.0 - 2026-06-10

La release initiale de Suprnova. Suprnova est un framework web inspiré
de Laravel pour Rust, forké depuis Kit et emmené dans sa propre
direction. La cible de parité actuelle est Laravel 13.x.

Cette version utilise le modèle de distribution git : les
consommateurs du framework dépendent de
`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`,
et le CLI s'installe avec `cargo install --git`.

### Ajouté

#### HTTP, routage et middleware

- `Router` avec groupes de routes, préfixes, contraintes de
  paramètres, routes nommées
- Enregistrement de routes validé à la compilation via la macro
  `routes!`
- Routage de ressource (`Router::resource`) produisant les sept
  routes standards
- URLs signées (fonctions libres `url::signed_route` /
  `url::temporary_signed_route`, plus `Redirect::signed_route` /
  `Redirect::temporary_signed_route`)
- Helpers de redirection - `Redirect::to`, `Redirect::back`,
  `Redirect::route`, `Redirect::with_input`, `Redirect::with_errors`,
  `with_flash`
- Trait `Middleware` avec des couches globale, de groupe, et par route
- Middleware intégrés - CORS, CSRF, session, timeout de requête, ID de
  requête, throttle / throttle de connexion, vérification d'URL
  signée, authenticated, email-verified, brute-force
- Helpers d'abort (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - adaptateur public pour servir une
  seule requête hyper contre un router + une chaîne de middleware

#### Pont frontend Inertia.js

- `#[derive(InertiaProps)]` avec émission de types TypeScript
- Macro `inertia_response!` avec validation de composant à la
  compilation
- Trois frontends de démarrage de premier ordre - **Svelte 5** (runes
  activées), **React 19**, **Vue 3.5** - tous sur Inertia 3.1.1 +
  Vite 8 + Tailwind v4
- Rechargements partiels (`only` / `except`), props différées, layout
  persistant, historique chiffré, préservation du scroll
- `Inertia::paginate(component, key, paginator)` pour le câblage
  paginateur → prop Inertia

#### ORM de style Eloquent (par-dessus SeaORM)

- Macro d'attribut `#[suprnova::model]` qui émet une entité SeaORM et
  la struct Eloquent orientée utilisateur en une seule fois
- Trait `Model` complet - `create`, `find`, `find_or_fail`,
  `find_many`, `all`, `query`, `save`, `update`, `delete`,
  `force_delete`, `refresh`, `fresh`, `replicate`, `replicate_into`,
  `increment`/`decrement`, `destroy`, `is`/`is_not`,
  `to_array`/`to_json`
- Affectation en masse fillable / guarded avec l'enveloppe `Attrs`
- 22 casts d'attribut - booléens, entiers, flottants, dates, enums,
  hashed, encrypted, JSON, collections, monnaie, datetime avec fuseau
  horaire
- Accesseurs / mutateurs via `#[suprnova::model]`
- Horodatages automatiques (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) avec `force_delete`, `restore`,
  `trashed`, `only_trashed`, `with_trashed`
- Onze types de relation - `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Enums morph par famille + registre morph avec rotation
  `APP_KEY_PREVIOUS`
- Eager loading via `.with(...)`, `.with_count(...)`,
  `.load_missing(...)`
- Moteur EXISTS corrélé pour `has` / `where_has`
- Seize événements de cycle de vie (`retrieving`, `retrieved`,
  `creating`, `created`, `updating`, `updated`, `saving`, `saved`,
  `deleting`, `deleted`, `restoring`, `restored`, `force-deleting`,
  `force-deleted`, `replicating`, `trashed`)
- Trait `Observer<M>` avec auto-enregistrement par méthode via
  inventory
- Scopes locaux via `#[scopes(M)]`, scopes globaux via `GlobalScope`
- Surface `Collection<M>` façon Laravel - `pluck`, `key_by`,
  `group_by`, `where_in`, `first_where`, `contains_where`,
  `partition`, etc.
- Trois paginateurs - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - tous se sérialisant en JSON de forme Laravel
- `chunk` / `lazy` / `cursor` pour l'itération de lignes en masse
  sans OOM
- Verrouillage au niveau ligne `lock_for_update` / `shared_lock`
- Constructeur de requêtes `DB::table(...)` avec `DynamicRow` pour les
  requêtes ad hoc
- `DB::transaction(...)` avec points de sauvegarde,
  retry-on-deadlock, fractionnement lecture/écriture
  multi-connexions
- `DB::listen(...)` + événements `QueryExecuted` /
  `TransactionBegan` / `TransactionCommitted` /
  `TransactionRolledBack`
- Trait `Prunable` + commande console `model:prune`
- Méthodes helper de requête `dump` / `dd`
- `#[model(unique_id="...")]` pour les clés primaires UUID / ULID

#### Authentification

- Trait `Authenticatable` + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Guards nommés multiples (session web, token API)
- Flux de vérification d'e-mail - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, URLs de vérification signées,
  `EmailVerificationMail`
- Flux de réinitialisation de mot de passe - `PasswordReset`, tokens
  throttlés, `PasswordChangedMail`, événement `PasswordResetLinkSent`
- TOTP à deux facteurs - enrôlement, vérification, codes de
  récupération, protection contre le rejeu
- Brute-force / throttle de connexion - indexé sur IP + identifiant,
  `LoginThrottleMiddleware`
- Cookies remember-me avec des tokens opaques stables
- Six événements d'auth - `LoginAttempted`, `LoggedIn`,
  `Authenticated`, `LoggedOut`, `PasswordResetLinkSent`,
  `EmailVerified`
- Sessions navigateur adossées au fork Torii sur
  `github.com/eas4ai/suprnova-torii-rs`

#### Autorisation

- Façade `Gate` - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (variantes sync + async)
- Macro `#[policy(Model)]` pour l'enregistrement de policy
- Auto-autorisation de resource-route

#### Paiements

- Surface à cinq traits agnostique au provider - `Checkout`,
  `Payment`, `Subscription`, `CustomerStore`, `WebhookHandler`
- Trait parapluie `PaymentProvider` + interrogation de capacité via
  `as_payment()`
- Miroir BD - `customers`, `subscriptions`, `subscription_items`,
  `payments`, `refunds`, `payment_webhook_events` (UNIQUE pour
  l'idempotence)
- Enum `SessionPayload` tagué par flow (ponctuel vs abonnement)
- Deux adaptateurs de référence en tant que crates du workspace -
  `suprnova-payments-stripe` (gateway, impl `Payment` complète),
  `suprnova-payments-paddle` (Merchant of Record, pas d'impl
  `Payment`)
- Provider fake pour les tests

#### File d'attente, jobs, batches, chaînes

- Trait `Job` - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers - `sync`, `null`, `redis`, `database`
- Trait `JobMiddleware` - six middleware intégrés
- Batches et chaînes - `Queue::batch(jobs).dispatch()`, builder de
  chaîne fluide, annulation, suivi de progression
- Magasin de jobs échoués avec rejeu
- Worker avec arrêt propre, concurrence configurable, récupération
  de panique via `catch_unwind`, métriques de clôture
- Douze événements de file d'attente couvrant la mise en file, le
  traitement, l'échec, la libération, le cycle de vie du worker

#### Diffusion et WebSockets

- Macro `ws!()` + `Router::ws` pour des endpoints WebSocket typés
- Scission Sink/Stream de `WsSocket`
- Superviseurs à redémarrage automatique via le trait `Supervisor`
- `BroadcastHub` avec canaux `Channel`, `Private`, `Presence`
- Protocole d'enveloppe JSON, presence join/leave/here, TTL de
  presence configurable avec récupération après crash
- Pont `Broadcastable` vers `EventDispatcher`
- Battement de cœur close-on-no-pong avec vidage `WS_TASKS`
  configurable
- Middleware WebSocket par route
- Défauts plus sûrs de 1 MiB / 64 KiB + factory `WsConfig::generous()`
- Politique d'origine + fermeture 1011 en cas de violation de
  protocole

#### Notifications et e-mail

- Trait `Notification` + `Notify::send(recipient, notification).await`
- Mailable + rendu de template Markdown
- Canaux database / mail / broadcast / web-push
- Signature VAPID + chiffrement de payload ECE RFC 8291 (via
  `suprnova-web-push`)
- Validation de subject VAPID, parsing de retry-after, plafond de
  corps de rejet à 8 KiB
- Trait `Notifiable` pour le typage de destinataire

#### Événements

- Dispatcher d'événements typé - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Événements `saving`/`updating` annulables (renvoient
  `EventResult::cancel`)
- Écouteurs queueable

#### Système de fichiers

- `Storage::disk("name")` avec support multi-driver - local, S3,
  Azure, GCS via OpenDAL
- Déplacement, copie, vérification d'existence, taille, mime,
  dernière modification, prepend/append
- Téléversements et téléchargements en streaming

#### Cache

- `Cache::store("name")` + enregistrement de driver
- Drivers - memory, redis (avec connect-timeout borné), database,
  file
- `remember`, `forever`, `tags`, increment/decrement atomique, locks

#### BD vectorielle

- Trait `VectorDriver` avec quatre drivers - in-memory, Qdrant
  (mapping d'ID UUID-5), Pinecone (IDs string natifs), `VECTOR(N)`
  natif de MariaDB + index HNSW (11.7+)
- Distance cosinus / produit scalaire / euclidienne

#### Binaire console et CLI

- Binaire `console` par projet - analogue Rust de `php artisan`,
  exécute des commandes définies par l'utilisateur via
  `#[suprnova::console::command]`
- `#[derive(Command)]` pour des arguments typés
- CLI `suprnova` - `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`,
  `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- Flag `--version`
- Templates de scaffold pour les starters backend + API à travers les
  trois frontends

#### Flags de fonctionnalité

- `DatabaseEvaluator` avec chargement par instantané
- `CachedEvaluator` avec TTL
- Extracteur `FeatureMiddleware`
- Surface CRUD admin
- Trait `FeatureSync` pour une propagation infra-seconde à travers
  les processus

#### Planification

- Parseur d'expression cron
- `Schedule::task(...)` avec des prédicats composables
- Verrous mono-serveur, prévention de chevauchement, suivi de
  dispatch
- Commande console `schedule:run`

#### Validation

- Intégration de `validator` 0.20
- Macros `#[request]` + `#[derive(FormRequest)]`
- Plafond de taille par formulaire
  `#[form_request(max_body_bytes = N)]`
- Opt-out `#[form_request(custom_hooks)]` pour un `impl FormRequest`
  écrit par l'utilisateur
- Hooks de cycle de vie - `authorize`, `after_validation`,
  `after_validation_async`

#### Drivers de base de données

- Support adossé à SeaORM pour SQLite, Postgres, MySQL, MariaDB
- Détection de driver basée sur l'URL
- Système de migration + `migrate`, `migrate:rollback`,
  `migrate:status`, `migrate:fresh`, `migrate:refresh`

#### Client HTTP

- Façade `Http` - `get` / `post` / `put` / `patch` / `delete`
  renvoyant un `RequestBuilder` ; `.send().await` produit une
  `ClientResponse`
- TLS rustls, timeout par défaut de 30s, user-agent
  `suprnova/<version>`
- Méthodes chaînables `json` / `form` / `body` / `header` /
  `bearer_token` / `basic_auth` / `timeout`
- `RequestBuilder::retry(max_attempts, base_backoff)` - backoff
  exponentiel pour les échecs transitoires et les 5xx ; respecte
  `Retry-After`
- Garde de test `Http::fake(|| async { ... }).await` avec
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Chiffrement

- Façade statique `Crypt` + `EncryptionKey` (`crypto::*`) ;
  AES-256-GCM avec des nonces aléatoires de 12 octets
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- Liaison AAD `CryptPurpose` empêchant le rejeu inter-protocole
- Rotation `APP_KEY_PREVIOUS`
- Commande CLI `suprnova key:generate` pour émettre des clés fraîches

#### Tests

- Macro de test async `#[suprnova_test]`
- `TestDatabase::fresh::<Migrator>()` avec des instances sûres en
  parallèle
- `TestContainer::bind` pour des fakes par test
- Helpers de test HTTP - `Test::get`, `Test::post`, JSON / form /
  multipart
- Fakes de Queue / Mail / Notification / Event
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Modifié

- Les flux de vérification d'auth et de réinitialisation de mot de
  passe opèrent désormais via le fournisseur d'utilisateurs configuré
  plutôt que via les internes de Torii.
- Les apps générées doivent implémenter `get_auth_password` ; les
  exemples scaffoldés échouent désormais explicitement au lieu de
  laisser la connexion toujours échouer silencieusement.
- Le gate de release local est câblé dans `scripts/release.sh`, et le
  dépôt inclut un hook pre-push imposé pour fmt, clippy, tests, docs,
  et les builds de features.
- La documentation des ports de dev scaffoldés se déplace vers les
  défauts backend/frontend actuels (`8765` / `5765`), avec `dev:tls`
  et `--with-portless` documentés.
- `MAIL_FROM` est validé avant que des tokens de vérification ou de
  réinitialisation ne soient émis, évitant des lignes de flux d'auth
  orphelines quand la configuration mail est invalide.

### Corrigé

- Dérive du template de scaffold React par rapport au starter publié.
- Les groupes de routes racine ne génèrent plus de chemins `//`
  dupliqués.
- Les redirections à chemin littéral se dispatchent désormais via le
  chemin de routage prévu.
- Les tests de fanout de diffusion gèrent désormais les résultats
  `track` / `untrack`.
- Le driver mail `log` émet le corps texte rendu, si bien que les
  liens de vérification et de réinitialisation de mot de passe
  apparaissent dans les logs de développement local.
- La couverture de réinitialisation de mot de passe épingle le
  comportement de révocation de session et de remember-me.

### Remarques

- **Modèle de distribution** : basé sur git de bout en bout.
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }` ;
  CLI via `cargo install --git`. Rien n'est publié sur crates.io.
