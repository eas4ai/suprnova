# Journal des modifications

Un journal lisible, par version, de ce qui a changé dans Suprnova. Chaque
section de version est le compte-rendu de publication de cette version.
Une version est publiée quand son commit de version et le tag
`v<version>` correspondant sont poussés atomiquement. Les plus récentes
en premier.

## 1.3.7 - 2026-08-26

### Ajouté

- **L'emplacement du middleware de page d'erreur Inertia vous appartient
  désormais, et il est documenté.** `Inertia::install` enregistre
  `InertiaErrorPageMiddleware` comme le plus interne de la couche Inertia :
  il couvre donc le handler, les middlewares de route et tout ce que
  vous enregistrez après cet appel - c'est pourquoi le scaffold place
  `CsrfMiddleware` en dessous de lui. Il ne couvre rien de ce qui est
  enregistré *au-dessus* de l'appel, car un middleware qui répond sans
  appeler `next` ne transmet jamais sa réponse à ce qui est enregistré à
  l'intérieur. Le cas qui pose problème est celui d'une session expirée
  qui poste un formulaire : un `CsrfMiddleware` enregistré au-dessus de
  l'installation répond `419` avec `{"message":"CSRF token mismatch."}` et
  l'utilisateur obtient la modale de plantage d'Inertia sur le flux qu'il
  est le plus susceptible d'emprunter ; le `429` d'un limiteur de débit
  placé plus à l'extérieur et le `401` d'un guard d'authentification font
  de même. Enregistrer le middleware vous-même, plus à l'extérieur,
  fonctionnait déjà en 1.3.6 - le type était public et l'enregistrement
  est idempotent par type, si bien qu'un enregistrement antérieur gardait
  sa place - mais rien ne le disait et rien dans `install` ne le
  reconnaissait, ce qui en faisait un accident plutôt qu'un contrat. C'en
  est un désormais : enregistrez
  `InertiaErrorPageMiddleware::new("Error")` après `SessionMiddleware` et
  `LocaleMiddleware` et avant le middleware dont il doit couvrir les
  rejets, et `install` le détecte, journalise au niveau `debug` et
  n'enregistre pas le sien. Le composant que vous avez nommé lors de cet
  enregistrement est celui qui est rendu : vous nommez donc la page une
  seule fois et `.error_page(...)` sur la config devient facultatif -
  c'est toujours lui qui fait enregistrer un middleware par `install` pour
  une application qui n'en place pas elle-même. Les deux règles d'ordre
  sont documentées sur le type et dans le manuel.

### Corrigé

- **Une page SSR a un seul `<title>`, et c'est celui de la page.** La
  coquille HTML écrivait son `default_title` puis le head du worker SSR
  tel quel, si bien que chaque page qui rendait un titre via le composant
  `Head` d'Inertia produisait un document à deux éléments `<title>`, celui
  du framework, générique, en premier. C'est le premier que lisent
  l'onglet du navigateur, le robot d'indexation et l'aperçu de lien : le
  vrai titre ne s'affichait donc jamais. Un head de worker qui porte un
  titre remplace désormais celui de la coquille au lieu de s'y ajouter -
  `default_title` comme un `InertiaResponse::title(...)` par réponse
  s'effacent ; un head qui n'en porte aucun laisse le titre de la coquille
  exactement où il était.
- **Le document déclare la langue dans laquelle il est écrit.** La
  coquille codait `<html lang="en">` en dur : un lecteur passé au japonais
  recevait donc de la prose japonaise dans un document qui se déclarait
  anglais - un lecteur d'écran choisit sa voix d'après cet attribut et un
  moteur de recherche y lit le signal de langue de la page. Elle porte
  désormais la locale en vigueur pour la requête : ce que
  `LocaleMiddleware` a détecté, puis un remplacement par
  `Lang::set_locale`, puis l'`APP_LOCALE` configuré, sous la même forme
  BCP 47 que rend `Locale` (`pt-BR`, `zh-Hans`). Cela vaut aussi pour la
  page d'erreur, qui est rendue au retour et qui est le cas ayant fait
  apparaître le problème. Sans la feature `localization`, la coquille
  conserve `en`.

### Mise à niveau

- Rien n'est requis. Les deux correctifs s'appliquent à chaque application
  Inertia dès la mise à niveau, et `Inertia::install` se comporte
  exactement comme avant pour une application qui n'enregistre pas
  elle-même le middleware de page d'erreur.
- Une application qui insérait `<html lang="...">` dans le document fini
  au moyen d'un middleware à elle peut le supprimer - la coquille le fait
  désormais, à partir de la même locale que lisait ce middleware.
- Une application dont le `CsrfMiddleware`, le limiteur de débit ou le
  guard d'authentification est enregistré **avant** `Inertia::install`
  devrait enregistrer `InertiaErrorPageMiddleware::new("Error")` après
  `LocaleMiddleware` et avant ce middleware, pour que ses rejets rendent
  la page d'erreur au lieu d'atteindre le client sous forme de JSON brut.
  `install` s'abstient alors d'ajouter le sien, et le composant que vous
  avez nommé à l'enregistrement est celui qui est rendu :
  `.error_page("Error")` sur la config est donc facultatif - gardez-le ou
  supprimez-le. Le `bootstrap.rs` scaffoldé enregistre CSRF après
  l'installation : un projet généré par `suprnova new` n'a donc rien à
  changer.
- Une application qui rend son propre `<title>` via le composant `Head`
  d'Inertia sous SSR verra le titre de la coquille cesser d'apparaître
  dans le document - aussi bien `InertiaConfig::default_title` qu'un
  `InertiaResponse::title(...)` par réponse. C'est le correctif : le titre
  de la page est le seul du document. Si vous comptiez sur le titre de la
  coquille comme préfixe ou suffixe, déplacez-le dans le composant `Head`,
  là où vit le reste du titre.

## 1.3.6 - 2026-08-26

### Ajouté

- **Les erreurs du framework peuvent rendre votre propre page Inertia au
  lieu de la modale de plantage du client.** Un utilisateur dépourvu d'une
  permission cliquait sur un lien de navigation vers une route protégée et
  obtenait l'écran « All Inertia requests must receive a valid Inertia
  response, however a plain JSON response was received » d'Inertia : le
  `403` portait le corps d'erreur JSON du framework et aucun en-tête
  `X-Inertia`, si bien que le client le refusait. Il en allait de même pour
  un `404` sur un chemin sans route, un `429` de limitation de débit et le
  `500` d'un handler en échec. Nommez un composant de page avec
  `InertiaConfig::error_page("Error")` et ces réponses rendent cette page à
  leur statut d'origine, avec les props `status`, `message` et - lorsque
  l'erreur en portait un - `request_id`. Tous les en-têtes posés par la
  réponse d'erreur survivent au remplacement, sauf ceux qui ne décrivaient
  que le corps remplacé (`Content-*`, `Transfer-Encoding`) ou régissaient la
  façon dont il pouvait être stocké (`Cache-Control`, `Expires`, `Age`,
  `ETag`, `Last-Modified`) : `Retry-After` sur un `429`, `WWW-Authenticate`
  sur un `401`, `Vary` et `Set-Cookie` atteignent donc toujours le client.
  La page pose `Cache-Control: no-cache, private` pour elle-même : elle
  porte vos props partagées, elle ne doit donc jamais être stockée par un
  cache partagé puis servie à un autre visiteur, quoi qu'ait permis la
  réponse qu'elle remplace. Une visite Inertia reçoit l'objet de page JSON ;
  une navigation dure reçoit la coquille HTML complète, si bien que coller
  l'URL dans la barre d'adresse fonctionne aussi. Tout ce qui a déjà un
  propriétaire est laissé intact : les `422` de validation redirigent
  toujours vers le formulaire, les rebonds `X-Inertia-Location` et les
  réponses qui sont déjà des pages Inertia passent sans changement, et un
  client dont l'`Accept` préfère le JSON conserve exactement le corps qu'il
  recevait auparavant. `suprnova new` scaffolde `frontend/src/pages/Error.*`
  et pose `.error_page("Error")` : les nouveaux projets sont donc couverts
  sans rien faire.

### Corrigé

- **Un disque local ne refuse plus un chemin légitime parce qu'une autre
  tâche y a touché.** Le garde-fou de chemin résolvait chaque composant d'un
  chemin avec deux sondes puis les combinait en un seul verdict, si bien
  qu'une activité concurrente ordinaire pouvait se lire comme une évasion
  par symlink : un composant que `canonicalize` venait de signaler absent,
  et qu'une autre tâche créait ensuite comme fichier ordinaire, revenait en
  `PermissionDenied` en nommant un symlink qui n'avait jamais existé. Cela
  mordait le plus fort là où les writers entrent en concurrence par
  construction - un concurrent perdant de
  `write_with(..).if_not_exists(true)` obtenait ce refus au lieu de
  `ConditionNotMatch` chaque fois que le gagnant publiait la clé entre les
  deux sondes, ce qui, sous une suite de tests chargée, arrivait environ
  une fois sur trois. Chaque composant est désormais
  classé en une seule passe, `symlink_metadata` d'abord : rien à cet
  endroit, c'est de l'espace libre ; un fichier ou un répertoire ordinaire
  est résolu et confiné comme avant ; et seul un symlink qui reste
  impossible à résoudre est refusé. Un composant qui disparaît en pleine
  classification est réexaminé une fois de plus plutôt que refusé. Tous les
  refus de symlink sont inchangés.

### Mise à niveau

- Rien ne change pour une application existante tant qu'elle n'y a pas
  explicitement adhéré. `InertiaConfig::error_page` vaut `None` par défaut,
  et `Inertia::install` n'enregistre le middleware de page d'erreur que
  lorsqu'un composant est nommé : les réponses d'erreur conservent donc
  exactement leur corps. Pour l'adopter, ajoutez un composant de page nommé
  `Error` à côté des autres (il reçoit `status`, `message` et un
  `request_id` facultatif) et chaînez `.error_page("Error")` sur
  l'`InertiaConfig` que vous passez à `Inertia::install`. Un handler qui
  **panique** reste hors de portée : la limite de panique enveloppe toute la
  chaîne de middleware, si bien que son `500` synthétisé est construit après
  que chaque middleware a été dépilé. Retournez `Err(...)` plutôt que de
  paniquer et la page d'erreur le couvre. Notez que le critère est la
  **forme** du corps, pas son auteur : à un statut d'erreur, un corps vide,
  un objet JSON dont le `message` est une chaîne et le texte `404 Not Found`
  propre au routeur sont réécrits quel que soit le middleware qui les a
  construits, et seuls `message` et `request_id` survivent dans les props.
  Une réponse qui doit conserver son propre corps JSON devrait mettre son
  texte sous une clé autre que `message`, ou se poser `X-Inertia: true`. Et
  enregistrez `LocaleMiddleware` **avant** `Inertia::install` : la page
  d'erreur est rendue au retour, après que chaque middleware enregistré à
  l'intérieur de la couche Inertia a rendu la main, si bien qu'une portée de
  locale ouverte à l'intérieur a déjà disparu et que chaque page d'erreur
  s'afficherait dans la locale par défaut de l'application. Le
  `bootstrap.rs` scaffoldé le fait désormais, et le même raisonnement vaut
  pour tout middleware à vous, à portée de requête, dont les props partagées
  de la page lisent l'état.

## 1.3.5 - 2026-08-26

### Modifié

- **Chaque section du journal des modifications se lit dans les six
  traductions du manuel.** Les manuels de, es, fr, ja, pt-BR et zh-Hans
  portaient les sections 1.3.0 à 1.3.2 en anglais derrière une note du
  traducteur, et des sections plus anciennes avec des lignes restées en
  anglais ; chaque section, de 1.3.5 jusqu'à 0.1.0, est désormais traduite,
  et les notes ont disparu.

### Corrigé

- **Les disques de système de fichiers local publient chaque objet en une
  seule étape.** `Storage::register_fs` et `register_fs_with` préparent
  désormais `disk.write(...)`, `disk.writer(...)` et `disk.copy(...)` sous
  forme de fichier temporaire dans `<root>/.suprnova-atomic/`, puis le
  publient sur la cible avec un unique `rename(2)` : aucun d'eux n'est donc
  jamais observable à une longueur partielle. Auparavant, le driver ouvrait
  la cible avec `create + truncate` et y écrivait en flux sur place - un
  lecteur concurrent obtenait un objet vide ou à moitié écrit pendant toute
  la durée de l'écriture, et un plantage en pleine écriture laissait un
  objet tronqué sur le chemin en service. `abort()` sur un writer supprime
  maintenant le fichier de préparation au lieu d'échouer avec `Unsupported`.
- **`write_with(..).if_not_exists(true)` est une vraie création exclusive
  sur un disque local.** Elle est publiée avec `link(2)`, qui échoue
  atomiquement dans le noyau quand la cible existe : un seul appelant, parmi
  un nombre quelconque d'appelants en concurrence, réussit donc, et tous les
  autres obtiennent `ConditionNotMatch` sans rien avoir écrit. Une écriture
  préparée puis publiée par un simple renommage aurait dégradé la condition
  en une vérification suivie d'un écrasement, écartant en silence tous les
  writers sauf le dernier - c'est-à-dire l'inverse de ce pour quoi on
  emploie cette primitive.
- **Un `append` qui crée l'objet reste un `append`.** Les ajouts sont la
  seule opération sur place sur un disque local, et cela vaut désormais
  aussi pour le premier d'entre eux : deux writers qui ajoutent au même
  objet encore inexistant aboutissent donc tous les deux, au lieu que l'un
  prépare sa propre copie et écrase l'autre.

- **`suprnova serve` ne recompile plus un projet auquel personne n'a
  touché, et `suprnova generate-types --watch` non plus.** Les deux
  surveilleurs classaient un événement du système de fichiers d'après son
  seul chemin, et le générateur lit tous les fichiers `.rs` du même arbre
  `src/` qu'ils surveillent - si bien que sous Linux, où le noyau signale
  ces lectures, chaque régénération programmait la suivante. Un projet
  fraîchement scaffoldé régénérait ses types et redémarrait son backend
  toutes les demi-secondes, indéfiniment, sans la moindre modification de
  source. Seuls comptent désormais les événements qui signifient que les
  octets sur le disque ont réellement changé. `generate-types --watch`
  n'avait par ailleurs aucun debounce, si bien qu'il agissait sur le premier
  fichier d'une salve plutôt que sur le dernier ; il adopte maintenant le
  même déclenchement en fin de salve à 500 ms que `serve`, et les deux
  surveilleurs partagent une seule implémentation, pour que le prochain
  correctif ne puisse pas s'appliquer à un seul des deux. Le générateur
  compare avant d'écrire : une régénération dont la sortie est identique
  octet pour octet laisse donc le fichier, et son mtime, intacts.

- **Le surveilleur du backend est restreint aux chemins à partir desquels le
  serveur est construit.** `cargo watch` s'exécutait sans aucun `-w` : il
  surveillait donc tout le projet non ignoré par git, et sauvegarder un
  composant Svelte, ou régénérer `frontend/src/types/inertia-props.ts`,
  recompilait le framework et redémarrait le serveur. Il surveille désormais
  `src/`, `cmd/`, `Cargo.toml`, `Cargo.lock`, `.env` et `lang/` - les
  entrées de la compilation, plus les deux arbres lus une seule fois au
  démarrage - chacun n'étant inclus que s'il existe, puisque cargo-watch
  refuse un chemin `-w` inexistant. C'est dans `cmd/` que le scaffold
  full-stack garde le `main.rs` du binaire serveur. L'invocation passe aussi
  `--no-vcs-ignores`, parce que cargo-watch applique le `.gitignore` aux
  racines `-w` nommées explicitement et que le scaffold ignore `.env`, ce
  qui laisserait sinon `-w .env` ne rien surveiller ; `-w` a déjà restreint
  la surface, si bien que le flag ne peut pas l'élargir. Les modifications
  du frontend et les fichiers `.ts` générés ne redémarrent plus le backend.

- **`serde_json::Value` se génère en `JsonValue` plutôt qu'en `unknown`.**
  Il se dégradait auparavant en `unknown`, avec un avertissement disant
  qu'il « isn't a struct this project defines », un conseil qui est faux
  pour un document JSON - et les pages de connexion et d'inscription du
  scaffold lui-même le déclenchaient deux fois à chaque régénération, si
  bien que chaque nouveau projet émettait cet avertissement dès
  l'installation. Il émet désormais un alias récursif `JsonValue`, déclaré
  une seule fois en haut du fichier généré et seulement quand quelque chose
  le référence. Un `Value` nu correspond lui aussi à cet alias, sauf si le
  projet définit sa propre struct `Value`.

- **Ni `generate-types` ni `serve` ne signalent comme généré un fichier
  qu'ils n'ont pas écrit.** Comme une passe n'écrit désormais que lorsque le
  contenu émis diffère, `Generated <path>` était une affirmation sur le
  système de fichiers qui était fausse à chaque réexécution sur un projet
  inchangé. `generate-types` dit `<path> is up to date` à la place, en mode
  ponctuel comme avec `--watch`, et la passe de démarrage de `serve` dit
  `N type(s) up to date → <path>`, en conservant le compte. Le surveilleur
  de fichiers de `serve` reste maintenant silencieux sur une régénération
  qui n'a rien écrit, en texte comme sous `--json` : un événement
  `types_regenerated` signifie que le fichier généré sur le disque est
  désormais différent, si bien qu'un silence après une sauvegarde vous dit
  que votre modification n'a changé la forme d'aucune prop.

### Mise à niveau

- **`.suprnova-atomic` est réservé à la racine de chaque disque local.** Le
  répertoire de préparation doit se trouver à l'intérieur de la racine - un
  répertoire frère de la racine peut être sur un autre système de fichiers
  quand la racine est un point de montage, et chaque renommage échouerait
  alors avec `EXDEV` - si bien que le nom est réservé plutôt que simplement
  conventionnel. Tout chemin dont le premier composant est
  `.suprnova-atomic` est désormais rejeté par une erreur de permission
  (lecture, écriture, suppression, stat et listage sans distinction), tout
  comme tout chemin qui se résout dans ce répertoire à travers un lien
  symbolique, et l'entrée est filtrée de `files`, `directories`,
  `all_files` et `all_directories`. Si la racine d'un disque contient déjà
  une entrée `.suprnova-atomic` qui vous appartient, elle n'est plus
  atteignable à travers ce disque : déplacez-la avant la mise à niveau. Un
  fichier ordinaire portant ce nom est refusé à l'enregistrement avec un
  message qui le dit, plutôt que d'échouer plus tard à l'intérieur du
  driver. Le nom est exporté sous `suprnova::ATOMIC_STAGING_DIR` pour que
  les outils de sauvegarde et de synchronisation puissent l'exclure.
- **Publier par renommage remplace l'inode de la cible.** Réécrire un objet
  sur un disque local ne préserve plus son mode, son propriétaire ni ses
  liens physiques, et un lecteur qui détient un descripteur ouvert conserve
  l'ancien contenu au lieu de voir les nouveaux octets. C'est le coût
  habituel de la publication atomique, mais c'est un changement de
  comportement si vous comptiez sur l'un ou l'autre.
- **Une écriture conditionnelle exige un système de fichiers doté de liens
  physiques.** `if_not_exists` est publié avec `link(2)`, qui n'est pas pris
  en charge sur FAT, exFAT et certains systèmes de fichiers réseau.
  L'écriture y échoue franchement plutôt que de se replier sur une
  vérification suivie d'un écrasement, parce qu'un repli vous donnerait une
  garantie d'exclusivité qui ne tient pas. Rien d'autre sur le disque n'est
  affecté.
- **Un premier `append` qui échoue laisse un objet vide.** Un ajout est la
  seule opération qui n'est pas publiée en une seule étape : l'objet est
  donc créé avant que les octets n'arrivent, et un premier ajout échoué ou
  abandonné le laisse derrière lui, exactement comme un ajout sur un objet
  existant l'a toujours fait.
- **Un lien symbolique orphelin dans la racine du disque est refusé, pas
  écrasé.** Un chemin dont la cible du lien symbolique n'existe pas ne peut
  plus être écrit, complété par un ajout, recopié dessus, déplacé dessus ni
  supprimé à travers le disque. `1.3.4` remplaçait un tel lien par un
  fichier ordinaire ; le garde-fou ne peut pas prouver où mène un lien qu'il
  n'arrive pas à résoudre, et créer à travers un tel lien crée la cible du
  lien n'importe où sur l'hôte : il refuse donc désormais. Supprimez le lien
  en dehors du disque si vous vouliez vraiment écrire là.
- **Rien ne purge le répertoire de préparation.** Il contient les fichiers
  temporaires en cours ainsi que ce qu'un processus mort en pleine
  publication a laissé derrière lui : un hôte en boucle de plantage le fait
  donc grossir sans limite. Le vider pendant que rien n'écrit sur le disque
  est sans risque ; l'exclure des sauvegardes est recommandé.

## 1.3.4 - 2026-08-25

### Ajouté

- **Les disques read-through prennent un flag `copy` et résolvent `copy` /
  `rename` à travers le repli.** Positionnez `copy: false` sur
  `ReadThroughConfig` pour servir ce qui est trouvé sur le repli sans l'écrire
  au passage, ce qui transforme le disque en surcouche transparente et
  restreint chaque récupération à la plage demandée. `copy` et `rename`
  transfèrent désormais en flux vers la destination primaire une source qui ne
  vit que sur le repli ; un `rename` supprime en plus la source sur le repli,
  si bien qu'une lecture ultérieure ne peut pas ressusciter l'objet déplacé.
  Les conditions voyagent avec ce chemin en flux : `if_not_exists` refuse
  toujours une destination existante, la version de source d'une copie
  sélectionne l'objet que le repli remet, et le `if_match` d'une copie est
  refusé avec `Unsupported` plutôt qu'abandonné en silence. Un transfert qui
  échoue en cours de route ne retire qu'une destination qu'il a lui-même
  créée : il ne peut donc pas détruire un objet qui était déjà là.
- **Jobs avec debounce et écouteurs en file d'attente avec debounce.**
  `Job::debounce_for()` réduit une salve de dispatches à une seule exécution,
  une fenêtre après le plus récent d'entre eux, portant le payload le plus
  récent. C'est le miroir de `push_unique`, qui garde le premier dispatch et
  supprime les autres. `Job::max_debounce_wait()` empêche une salve continue de
  différer le travail indéfiniment, et `Job::debounce_id(&self)` cantonne la
  fenêtre par entité, si bien que vingt mises à jour d'une commande se
  réduisent sans toucher à celles d'une autre commande.
  `Queue::push_debounced(job, DebounceOptions)` définit la fenêtre au site
  d'appel, et `DebouncedListener::new(window, build).keyed_by(...)` applique un
  debounce à un écouteur d'événement avec la clé dérivée de l'événement - un
  `QueuedListener` ordinaire honore déjà une fenêtre que le job lui-même
  déclare. Chaque dispatch est quand même mis en file d'attente ; la réduction
  se règle au niveau du worker, qui acquitte une enveloppe supplantée et émet
  `JobDebounced`. Le debounce échoue en mode ouvert : une fenêtre expirée ou
  évincée exécute le job plutôt que de l'abandonner. Chaque exécution réelle
  démarre une nouvelle fenêtre d'attente maximale, si bien qu'une salve mesure
  toujours son attente maximale depuis son propre premier dispatch plutôt que
  d'hériter de celle de la salve précédente. Un job ne peut pas déclarer à la
  fois `debounce_for` et `unique_id`, et les chaînes comme les lots refusent un
  job avec debounce - un maillon supplanté laisserait le reste de sa chaîne en
  plan, et un job de lot supplanté laisserait pour toujours le compte d'attente
  du lot au-dessus de zéro. L'enveloppe porte deux champs additifs pour cela et
  reste identique octet pour octet sur le réseau pour tout push sans debounce.

- **`Storage::register_read_through` compose deux disques en un disque
  read-through.** Les lectures et les métadonnées se résolvent d'abord contre
  le primaire et retombent sur le second disque ; ce qui est trouvé sur le
  repli est écrit au passage sur le primaire, si bien qu'une migration de
  magasin s'achève sous trafic réel. Les écritures et les listages restent sur
  le primaire, et une suppression retire l'objet des deux disques. Positionnez
  `throw_on_promotion_failure` quand une promotion échouée doit remonter au
  lieu de se dégrader en lecture sur le repli. Une promotion est publiée de
  façon atomique, si bien qu'aucun lecteur ne peut voir un objet à moitié
  écrit, et elle transporte le type de contenu, le cache control, la content
  disposition, l'encodage de contenu et les métadonnées utilisateur de l'objet
  du repli. Une lecture versionnée ou conditionnelle est transmise avec sa
  condition intacte et servie sans être promue.
- **`Queue::forward` redirige une file d'attente entière par son nom.** Là où
  `Queue::route` est indexée par type de job, `Queue::forward("default", "high")`
  est indexée par nom de file d'attente - le levier pour retirer un pool du
  service, absorber un backlog ou déplacer le travail hors d'un pool que vous
  vous apprêtez à arrêter, sans toucher à un seul job ni à une seule route.
  Elle s'applique des deux côtés : les nouveaux pushs qui s'étaient résolus en
  `default` atterrissent sur `high`, *et* un worker démarré avec
  `--queue=default` vide `high`, si bien que la destination ne peut pas
  collecter du travail que personne ne réclame. Faire suivre `default` attrape
  les jobs qui n'ont nommé aucune file d'attente. Un forward est une recherche
  unique, jamais une chaîne : une permutation (`a -> b` avec `b -> a` également
  enregistré) ou une rotation plus longue est donc un échange de pools cohérent
  plutôt qu'une boucle - exactement comme chez Laravel, dont le résolveur est
  cette même recherche unique.
  La mise en pause reste évaluée sur les noms avec lesquels un worker a été
  démarré : `Queue::pause(&connection, "default")` arrête donc ce worker même
  pendant que `default` est fait suivre. `Queue::forward_on(from, to, connection)`
  restreint un forward à un seul nom de connexion, comparé au nom de connexion
  de ce processus plutôt qu'à la connexion déclarée d'un job, si bien que les
  deux moitiés de la redirection filtrent sur la même valeur.
  `Queue::forward_for(from)` relit un forward, et `Queue::try_forward` en est
  l'homologue faillible. Les appels d'inspection (`Queue::pending_jobs` et ses
  homologues) ne suivent délibérément pas un forward, si bien qu'un backlog
  laissé en arrière sur une file d'attente redirigée reste visible.

- **Les commandes Redis de forme lecture réessaient un échec transitoire au
  lieu de le faire remonter.** La connexion se rétablissait déjà en arrière-plan,
  mais la commande qui avait heurté la socket morte faisait quand même échouer
  votre appel. `GET`, `EXISTS`, les pages `SCAN` et `SSCAN` derrière
  `Cache::flush` / `Cache::flush_tags`, les lectures `XLEN` / `ZCARD` /
  `XPENDING` du driver de file d'attente et le calcul du `Retry-After` du
  limiteur de débit réessaient désormais une fois après une courte pause.
  `REDIS_COMMAND_RETRIES` en ajoute d'autres par-dessus, plafonné à 10.
  Budgétez le réessai en secondes plutôt qu'en millisecondes : la seconde
  tentative attend la connexion de remplacement, elle coûte donc tout le budget
  de connexion et de réponse du driver, et une commande partie en timeout
  compte comme transitoire au même titre qu'une commande coupée. Les écritures
  ne réessaient jamais, quel que soit le réglage : une erreur transitoire
  signifie que la connexion a échoué, pas que le serveur a refusé la commande,
  si bien que répéter un `SET`, un `INCR`, une acquisition de verrou, un
  décompte de limitation de débit ou un dépilement de file d'attente pourrait
  l'exécuter deux fois. Les messages d'erreur sont inchangés : tout ce qui s'y
  accroche continue de fonctionner.
- **Un worker en pause vous dit désormais qu'il est en pause.** `queue:work`
  imprime une ligne par transition - `2026-08-25 14:03:11 Queue billing PAUSED`,
  et `RESUMED` au retour - et le worker émet `WorkerQueuePaused` /
  `WorkerQueueResumed` pour que vous puissiez router le même signal vers vos
  propres alertes. C'est la paire côté worker ; les `QueuePaused` /
  `QueueResumed` existants se déclenchent dans le processus qui a exécuté
  `queue:pause`, ce qui n'est jamais le worker, si bien que jusqu'ici un worker
  devenu silencieux parce que quelqu'un avait mis sa file d'attente en pause
  était indiscernable d'un worker bloqué. Chaque événement se déclenche une
  fois par transition, pas une fois par interrogation. Leur champ `queue` est
  facultatif : un worker démarré sans `--queue` vide tout et n'a aucun nom de
  file d'attente à signaler sous `pause_all`, si bien qu'il rapporte `None`
  plutôt que d'inventer un nom sur lequel un écouteur pourrait s'accrocher.
- **Les chemins `?include=` sont plafonnés à cinq segments, et
  `max_relationship_depth` déplace ce plafond.** Un graphe de relations cyclique
  transforme `?include=author.posts.author.posts...` en un fan-out contrôlé par
  le client, borné seulement par la chaîne de requête. Les chemins sont
  désormais tronqués pendant leur analyse ; appelez
  `suprnova::max_relationship_depth(n)` dans `bootstrap::register()` pour
  changer la limite, ou passez `0` pour désactiver les includes.
- **`Gt`, `Gte`, `Lt` et `Lte` comparent un champ à un nombre ou à un autre
  champ.** `CompareWith` nomme l'opérande et la mesure en une seule valeur :
  `Number` pour un littéral, `NumericField` pour un champ voisin numérique, et
  `LengthField` pour un champ voisin comparé par nombre de caractères. Un
  opérande que la règle ne peut pas mesurer fait échouer le champ au lieu de
  paniquer.
- **Trois règles d'appartenance rejoignent l'ensemble intégré : `InArray`,
  `Contains` et `DoesntContain`.** `InArray` vérifie une valeur contre la liste
  d'un autre champ, et vous passez la liste directement au lieu de nommer le
  champ dans une chaîne de règle. `Contains` et `DoesntContain` s'exécutent sur
  un tableau JSON et ne font correspondre un paramètre qu'à un élément de type
  chaîne, si bien que `1` et `"1"` restent distincts.
- **Le pool de base de données a désormais des réglages de vivacité.**
  `DB_IDLE_TIMEOUT`, `DB_MAX_LIFETIME`, `DB_ACQUIRE_TIMEOUT`,
  `DB_TEST_BEFORE_ACQUIRE` et `DB_PING_AFTER_IDLE` contrôlent quand le pool
  ferme, recycle et pingue une connexion, avec les setters correspondants sur
  `DatabaseConfig::builder()`. Chacun est non défini par défaut, si bien que le
  pool d'un déploiement existant se comporte exactement comme avant.
  Employez-les quand une passerelle NAT ou un pare-feu coupe les connexions
  inactives : sqlx n'expose aucun équivalent des `keepalives_*` de libpq, si
  bien que le recyclage du pool est le mécanisme.
- **`db:seed <Class>` rapporte sa progression.** Une exécution ciblée imprime
  une ligne `RUNNING` avant le seeder et une ligne `DONE` avec les millisecondes
  écoulées après lui. Un `db:seed` nu reste silencieux. Le formateur,
  `suprnova::two_column_detail`, est disponible pour vos propres handlers
  `#[command]`.
- **Les relations plusieurs-à-plusieurs filtrent désormais sur les colonnes du
  pivot.** `where_pivot`, `where_pivot_op`, `where_pivot_in`,
  `where_pivot_not_in`, `where_pivot_null`, `where_pivot_not_null`,
  `where_pivot_between`, `where_pivot_not_between`, `where_pivot_group` et
  leurs jumelles `or_` contraignent `get`, `first` et `count` sur
  `BelongsToMany`, `MorphToMany` et `MorphedByMany`. `where_pivot_group` prend
  une closure et rend un seul groupe parenthésé, si bien qu'il reste atomique
  à l'intérieur d'un `or_where_pivot` qui suit. Les filtres de pivot ne
  s'appliquent qu'aux lectures : `attach`, `attach_with`, `detach` et `sync`
  retournent une erreur tant que l'un d'eux est posé, et le chargement hâtif ne
  les transporte pas.
- **`where_binary` compare les valeurs de colonne octet pour octet.** La
  famille (`where_binary`, `or_where_binary`, `where_not_binary`,
  `or_where_not_binary`) est livrée sur `Builder<M>`, et `where_binary` comme
  `where_not_binary` sont livrées sur `DB::table(...)`. MySQL et MariaDB
  émettent `= binary` ; Postgres et SQLite retournent une erreur au moment où
  la requête est rendue, plutôt que de retomber sur une correspondance
  dépendante de la collation.
- **`Builder::try_to_sql_with_bindings_for` rend le SQL pour un dialecte sans
  paniquer.** C'est l'homologue faillible de `to_sql_with_bindings_for`, pour
  les cas où un builder ne peut légitimement pas rendre pour un backend.
- **`Model::refresh_for_update` recharge une ligne sous un verrou
  `FOR UPDATE`.** Appelez-le à l'intérieur d'une transaction quand il vous faut
  l'état courant de la ligne et le verrou exclusif en une seule instruction.
  SQLite n'a pas de verrouillage au niveau de la ligne : la clause de verrou y
  est donc sans effet.
- **`Builder::or_where_key` et `Builder::or_where_key_not` ajoutent des filtres
  de clé primaire comme disjonction.** Tous deux se fondent dans la clause
  `WHERE` précédente de la même façon que `or_where`, et tous deux sont livrés
  avec les alias `or_filter_key` et `or_filter_key_not`.
- **`Builder::in_order_of` trie les lignes selon une séquence explicite.**
  Passez une colonne et les valeurs dans l'ordre que vous voulez ; les lignes
  dont la valeur n'est pas dans la liste sont triées en dernier. Les valeurs se
  lient comme paramètres : elles peuvent donc sans danger venir des données de
  la requête.

### Corrigé

- **Le cookie de contournement de maintenance expire désormais côté serveur.**
  Le TTL de 12 heures était un `max-age` appliqué par le navigateur, si bien
  qu'un cookie capturé continuait de fonctionner jusqu'à ce que vous fassiez
  tourner le secret. Le payload chiffré porte désormais l'échéance, et chaque
  requête la revérifie.
- **`suprnova serve` fait tourner un projet sans frontend.** Un projet
  scaffoldé avec `suprnova new --api` n'a pas de répertoire `frontend/`, et
  `serve` le rejetait avec « No frontend directory found. Are you in a Suprnova
  project directory? » à moins de passer `--backend-only`. Il saute désormais
  le panneau Vite et la génération TypeScript qui l'alimente, et sert le
  backend. `--frontend-only` échoue toujours sur un tel projet, avec un message
  qui explique pourquoi.

### Mise à niveau

- **Les cookies de contournement émis avant cette version cessent de
  fonctionner.** Le payload du cookie est passé du secret nu à un objet scellé
  `{ secret, expires_at }`, et un payload sans échéance est refusé. Visitez
  l'URL secrète une fois après la mise à niveau pour obtenir un nouveau cookie.
  Rien d'autre ne change : `down`, `up`, `--secret` et `--with-secret` se
  comportent tous comme avant.
- **Un chemin d'include de plus de cinq segments retourne désormais ses cinq
  premières relations au lieu de toutes.** Rien en dehors de la liste blanche
  d'une ressource n'a jamais été atteignable, si bien qu'aucune réponse ne
  gagne de données ; un chemin profond perd sa queue. Un code de statut change
  avec cela : un chemin dont la queue trop profonde nomme une relation que la
  ressource n'autorise pas est tronqué avant que quoi que ce soit ne le valide,
  si bien qu'il retourne désormais `200` avec les segments qui ont survécu là
  où le chemin complet retournait `400` - ajustez tout client ou test qui
  s'appuie sur ce rejet. Relevez le plafond avec
  `suprnova::max_relationship_depth(n)` si votre API documente des chemins plus
  longs.
- **`DatabaseConfig` a gagné cinq champs publics.** Le code qui en construit un
  avec un littéral de struct ne compile plus. Utilisez
  `DatabaseConfig::from_env()` ou `DatabaseConfig::builder()`, qui remplissent
  tous deux les nouveaux champs avec les valeurs par défaut qui préservent le
  comportement actuel du pool.

## 1.3.3 - 2026-08-25

### Ajouté

- **Connexion de file en failover.** `FailoverQueueDriver` enveloppe une liste
  ordonnée de connexions : un push que la première refuse est réessayé sur la
  suivante, et ainsi de suite le long de la liste. Câblez-la depuis
  l'environnement avec `QUEUE_DRIVER=failover` plus
  `QUEUE_FAILOVER_CONNECTIONS=redis,database` (chaque entrée lit les variables
  de son propre driver, si bien qu'une entrée `database` exige toujours
  `DB::init()` d'abord et apporte toujours son magasin de jobs en échec), ou
  construisez-la directement avec
  `FailoverQueueDriver::new(vec![(label, driver), ...])`. Seules les écritures
  parcourent la liste : `push` et `bulk_push` la traversent, tandis que `pop`,
  `pop_from`, `ack`, `nack`, `release`, `settle`, `clear`, les quatre compteurs
  et les trois listings d'inspection délèguent à la première connexion et à
  aucune autre, parce qu'un token de réservation n'a de sens que pour le driver
  qui l'a émis. La conséquence opérationnelle est documentée plutôt que
  masquée : un worker sur la connexion de failover ne vide que la primaire,
  donc tout ce qui a basculé vers un repli a besoin de son propre worker.
  `bulk_push` pousse chaque enveloppe séparément au lieu de transmettre un lot,
  ce qui préserve à la fois l'`available_at` propre à chaque enveloppe (Laravel
  #60950) et empêche un lot à moitié accepté par la primaire d'être repoussé en
  bloc sur le repli. Un refus dispatche
  `queue::events::QueueFailedOver { connection, job_name, exception }`,
  déclenché sur front : une connexion se signale une fois quand elle entre en
  échec et reste silencieuse jusqu'à ce qu'un push ultérieur y réussisse et la
  réarme, si bien qu'une panne produit un événement et non un par dispatch.
  `QUEUE_FAILOVER_CONNECTIONS` est requis quand ce driver est sélectionné, ne
  peut pas contenir `failover` lui-même, et fait échouer l'amorçage sur une
  entrée inconnue plutôt que de retomber sur la mémoire, si bien qu'une faute
  de frappe ne peut pas glisser un backend éphémère dans une chaîne durable.
- **API d'inspection de file.** `Queue::pending_jobs(queue)` / `delayed_jobs` /
  `reserved_jobs` listent les enveloppes réelles derrière les compteurs
  existants `pending_size`/`delayed_size`/`reserved_size`, sous forme de DTO
  `InspectedJob` (`id`, `queue`, `name`, `attempts`, `payload`, `created_at`) -
  ils reflètent l'`InspectedJob` de Laravel. Un unique filtre de file
  `Option<&str>` réduit à un seul appel la paire `pendingJobs($queue)` /
  `allPendingJobs()` de Laravel (et ses équivalents
  `delayedJobs`/`reservedJobs`). Le défaut du trait `QueueDriver` est un `Err`
  honnête - et non le défaut « collection vide » des drivers Beanstalkd/SQS de
  Laravel, qui se lit comme « rien en file » alors qu'il y a manifestement
  quelque chose - si bien qu'un driver qui n'a pas implémenté l'inspection le
  dit ; `sync`/`null` le redéfinissent avec `Ok(vec![])` parce que, pour eux,
  c'est réellement la vérité. Les drivers mémoire, base de données et Redis
  implémentent tous le listing complet : le stockage des différés du driver
  mémoire est passé d'une simple `DelayQueue<Envelope>` (qu'on ne peut pas
  parcourir) à une `DelayQueue<Uuid>` plus une map indexée par id ; le driver
  base de données réutilise les prédicats exacts des compteurs de taille plus
  `ORDER BY available_at`, et une ligne dont l'`envelope_json` n'a pas pu être
  décodé est quand même listée (`id: None`,
  `payload: {"unparseable": true}`) plutôt que jetée, si bien qu'une seule
  ligne toxique ne peut pas aveugler un opérateur sur le reste de la file ; le
  `reserved_jobs` de Redis est limité aux réservations de ce consommateur dans
  le processus (documenté), et `pending_jobs` balaie le stream par lots via
  `XRANGE`. `Queue::fake()` a gagné les helpers correspondants
  `pending_jobs()`/`delayed_jobs()`, qui projettent les pushes enregistrés avec
  `attempts` toujours à `0` et `created_at` toujours à `None`.
- **Dispatch après commit.** `Job::after_commit()` retient un push jusqu'à ce
  que la `DB::transaction` englobante commite, si bien qu'un worker sur un
  autre processus ne peut jamais dépiler une enveloppe qui décrit des lignes
  que la transaction n'a pas encore rendues durables. Tout le push attend, pas
  seulement l'écriture du driver : la construction de l'enveloppe,
  `JobQueueing` et `JobQueued` ont tous lieu au moment du commit, si bien
  qu'aucun écouteur n'est jamais informé d'un job qu'un rollback jette ensuite.
  Un rollback jette le push entièrement ; hors transaction, le push a lieu
  immédiatement, ce qui permet à un type de job de déclarer l'adhésion sans que
  chaque site de dispatch sache si son chemin de code est transactionnel. Par
  dispatch, `EnvelopeOverrides::after_commit` devance le job : `Some(true)`
  (avec le raccourci `Queue::push_after_commit(job)`) diffère un job qui n'y a
  pas adhéré, et `Some(false)` est le `beforeCommit()` de Laravel. Un
  `Queue::push` différé résout à nouveau `Job::delay()` par rapport au commit
  plutôt qu'au push, tandis que `Queue::push_later` / `later` / `later_with`
  portent l'horodatage absolu de l'appelant à travers le report, inchangé.
  `Queue::push_unique` prend son verrou de déduplication immédiatement, même
  quand l'enveloppe est différée, si bien qu'un doublon dans la même
  transaction est toujours supprimé, et un rollback libère ce verrou en le
  liant au propriétaire. `Queue::bulk` diffère d'un bloc. `Queue::fake()`
  enregistre un push immédiatement, report compris, comme le `Bus::fake` de
  Laravel. Un `DB::begin_transaction` manuel ne diffère jamais : il n'installe
  aucune transaction ambiante, donc il n'y a aucun commit auquel accrocher un
  callback. Toutes les issues qui laissent le commit non posé compensent de la
  même façon, y compris un `COMMIT` que la base refuse et un `TxHandle` fuité
  qui en bloque un, et `Transaction::rollback_to` en compte comme une pour la
  portée qu'elle déroule : un push différé à l'intérieur d'un savepoint est
  jeté quand ce savepoint est annulé et son verrou est libéré sur-le-champ,
  tandis que tout ce qui a été enregistré avant le savepoint est intact. Les
  e-mails, notifications, batches et chaînes en file ne diffèrent pas encore.
- **Jobs uniques jusqu'au traitement.** `Job::unique_until_processing()` libère
  le verrou d'unicité quand le traitement commence - après la passe de
  middleware du job, immédiatement avant l'exécution du handler - au lieu de le
  tenir pendant toute la fenêtre `unique_for`, ce que vous voulez quand le
  verrou existe pour fusionner les doublons en file plutôt que pour sérialiser
  l'exécution. Un job qu'un middleware relâche sur la file garde son verrou,
  parce qu'il n'a pas commencé à être traité ; un job qu'un middleware supprime
  ou met en lettre morte abandonne le sien. La libération est liée au
  propriétaire : `Queue::push_unique` enregistre le token de propriétaire du
  verrou de cache sur l'enveloppe (`Envelope::unique_lock_owner`, un champ
  additif qui laisse le format réseau figé octet pour octet identique pour
  chaque push non unique), et le worker libère avec ce token, si bien qu'une
  tentative redélivrée ne peut jamais forcer la libération d'un verrou qu'un
  dispatch plus récent détient désormais. La surface d'idempotence sous-jacente
  est publique elle aussi : `Idempotency::commit_on_success_owned` remet au
  corps le propriétaire du verrou et le retourne, et
  `Idempotency::release_owned(key, owner)` libère en le liant au propriétaire,
  en rapportant `Ok(false)` plutôt qu'une erreur quand le verrou est absent ou
  détenu par quelqu'un d'autre. Les jobs à simple `unique_id` sont inchangés et
  laissent toujours le TTL `unique_for` être la fenêtre de déduplication.
- **`Gate::default_denial_response` personnalise la forme par défaut d'un refus nu.** Reflète
  le `Gate::defaultDenialResponse($response)` de Laravel. Défini une fois - généralement dans
  `bootstrap::register()` - il remodèle exactement deux issues : un `false` nu (un gate booléen -
  `Gate::define` / `Gate::define_async`, y compris une méthode `#[policy]` qui retourne `bool` - ou
  un hook `before`/`after` qui a décidé `false`) et une évaluation que rien d'autre n'a tranchée (une
  ability indéfinie sur laquelle aucun hook n'a d'avis non plus). Tout cela se réduisait auparavant à un
  `Response::deny()` nu (un 403) ; désormais, ces issues remontent sous la forme du `Response` que porte le
  défaut, p. ex. `Response::deny_as_not_found()` pour un 404 qui cache l'existence d'une ressource à
  l'échelle de l'application au lieu de le faire gate par gate. Le défaut ne s'applique qu'au `false` nu -
  un gate enregistré avec `define_with` / `define_async_with` a déjà retourné le `Response` qu'il voulait,
  et celui-ci traverse toujours `Gate::inspect` intact, ce qui correspond à la règle de Laravel selon
  laquelle le défaut ne se substitue jamais à un objet `Response` retourné. Un défaut de forme
  `Response::allow()` est rejeté (journalisé, ignoré) plutôt que d'inverser silencieusement chaque gate
  booléen vers l'autorisation - voir le commentaire de doc de `Gate::default_denial_response` pour le seul
  endroit où cela diverge délibérément de Laravel, qui n'a pas ce garde-fou.
- **La famille de règles de validation `Password` est livrée, y compris la
  vérification Have I Been Pwned `uncompromised()`.** `Password::min(n)` plus
  les builders de robustesse (`.max()`, `.letters()`, `.mixed_case()`,
  `.numbers()`, `.symbols()`) portent mot pour mot les regex de la règle
  `Password` de Laravel - une simple espace satisfait `.symbols()`, comme la
  classe de séparateurs `\p{Z}` de Laravel. `.uncompromised()` (ou
  `.uncompromised_with_threshold(n)`) vérifie le mot de passe contre l'API de
  plage à k-anonymat de Have I Been Pwned : seuls les 5 premiers caractères du
  hachage SHA-1 du mot de passe quittent le processus, et un échec réseau, un
  délai d'attente dépassé ou une réponse non 2xx échoue en mode ouvert plutôt
  que de bloquer les inscriptions, exactement comme le `NotPwnedVerifier` de
  Laravel. Comme cette vérification est un aller-retour HTTP, `Password` est la
  seule règle intégrée à implémenter à la fois `Rule` (robustesse seulement,
  pour les lignes `validate!` synchrones) et `AsyncRule` (robustesse, puis la
  vérification HIBP, pour `after_validation_async`) - appeler le chemin
  synchrone sur un `Password` configuré avec `uncompromised()` est une erreur
  explicite, destinée au développeur, plutôt qu'un saut silencieux.
  `Password::defaults_with(...)` fixe le défaut valable pour tout le processus
  que `Password::defaults()` retourne. Nouvelle variable d'environnement
  `HIBP_TIMEOUT_SECS` (défaut 30 s). `Http::fake_response_text(...)` est le
  nouvel homologue à corps brut de `fake_response(...)`, pour les tests contre
  des API amont en `text/plain` comme celle de HIBP.
- **Une tâche planifiée peut désormais nommer le fuseau horaire dans lequel son
  expression cron est lue, et `schedule:list` peut rendre toute la
  planification dans n'importe quelle zone.** `.timezone(chrono_tz::Tz)`
  épingle une tâche, `.try_timezone("Area/City")` est l'homologue faillible
  pour un nom de zone qui n'existe qu'à l'exécution, et
  `Schedule::timezone(tz)` fixe un défaut pour chaque tâche enregistrée après
  lui. Rien ne change pour une tâche qui n'épingle aucune zone : elle est
  toujours évaluée par rapport à la zone locale du processus. Une zone épinglée
  n'affecte que l'échéance - le planificateur bat toujours une fois par minute
  de processus et la barrière de dédoublonnage à la même minute est intacte.
  Notez qu'une zone qui observe l'heure d'été fait que certaines minutes
  d'horloge murale se produisent deux fois et d'autres pas du tout : une tâche
  épinglée sur une telle minute peut donc s'exécuter deux fois ou être sautée ;
  le chapitre sur la planification porte l'avertissement complet.
  `schedule:list` a gagné une option `--timezone` et deux colonnes : la zone
  dans laquelle une expression affichée est écrite, et la prochaine minute où
  la tâche se déclenche. L'expression d'une tâche épinglée est réécrite dans la
  zone du listing, en se découpant sur plusieurs lignes quand elle y chevauche
  minuit, et elle est laissée exactement telle qu'écrite quand une réécriture
  fidèle est impossible - à cheval sur une transition d'heure d'été, quand un
  passage de jour devrait déplacer ensemble un jour-du-mois et un
  jour-de-la-semaine restreints, ou quand il faudrait décider de la longueur de
  février. `chrono_tz::Tz` est réexporté depuis la racine du crate, si bien que
  les applications consommatrices n'ajoutent pas `chrono-tz` à leur propre
  `Cargo.toml`.
- **Un sous-système d'images façon Laravel, dans `suprnova::media`, derrière la
  feature `media` activée par défaut.**
  `Image::from_bytes/from_path/from_disk/from_upload/from_stream` construit un
  pipeline paresseux - `resize`, `scale`, `crop`, `cover`, `contain`, `rotate`
  à n'importe quel angle, `flip_vertically`/`flip_horizontally`, `blur`,
  `sharpen`, `grayscale`, `to_format`, `quality` - que l'on termine par
  `to_bytes`, `to_response`, `save`, `store`, `dimensions`, `mime_type` ou
  `dominant_color`. Lit et écrit PNG, JPEG, WebP, GIF et BMP ; la sortie AVIF
  est différée jusqu'à la publication de l'encodeur AV1 maison, moment où elle
  ne sera qu'une nouvelle variante d'`OutputFormat` et rien d'autre. Comme la
  scission `gd`/`imagick` de Laravel, il y a deux drivers :
  `IMAGE_DRIVER=oxideav` (le défaut) tourne sur la famille de codecs en Rust
  pur [OxideAV](https://github.com/OxideAV), sans bibliothèque native ni rien à
  installer, et `IMAGE_DRIVER=magick` shelle vers un ImageMagick 7 installé sur
  l'hôte pour une prise en charge plus large des formats d'entrée, HEIC
  compris. Les limites de décodage (`IMAGE_MAX_DIMENSION`,
  `IMAGE_MAX_ALLOC_BYTES`) sont vérifiées contre l'en-tête de l'entrée
  elle-même avant toute allocation - y compris le flux binaire interne d'un
  WebP étendu, dont la taille de toile indicative ne peut pas servir à faire
  passer une image plus grande devant la gate - et tout le travail sur les
  pixels s'exécute sur un thread bloquant. Le driver `magick` épingle le coder
  d'entrée par son nom au lieu de laisser ImageMagick en choisir un d'après les
  octets, et borne chaque invocation avec `IMAGE_MAGICK_TIMEOUT_SECS`.
  `ImageDriver` est la frontière de trait pour tout le reste. Le module
  s'appelle `media` parce que les surfaces audio et vidéo adossées à OxideAV
  vivront à côté de lui. [Images](images.md)
- **La gate WebP porte une borne fixe et non configurable.** Un WebP déclare sa
  vraie taille décodée dans son chunk de flux binaire le plus interne, si bien
  que le framework parcourt le conteneur pour la trouver ; ce parcours visite
  au plus 4096 chunks par niveau et suit deux niveaux d'imbrication, et un
  fichier au-delà de l'un ou l'autre est refusé plutôt que mesuré. Annoncer un
  nombre issu d'un parcours inachevé donnerait une gate qu'assez de chunks de
  remplissage pourraient contourner. Aucune variable `IMAGE_MAX_*` ne l'affecte
  et l'erreur le dit. Une animation de 300 images n'est pas concernée ; une de
  4100 est refusée. [Images](images.md#one-bound-is-not-configurable)

- **OAuth peut désormais être installé sans remplacer l'autorité de mot de
  passe et de session existante d'une application.** `MagnetarOAuthOnlyConfig`
  et `init_magnetar_oauth_only` installent la cérémonie par défaut et le moteur
  de fournisseurs tout en laissant vides les emplacements mot de passe et
  passkey. Les applications qui ont déjà une table `users` peuvent appeler
  `verify_oauth_identity`, faire correspondre elles-mêmes le sujet vérifié du
  fournisseur, et établir leur session de framework habituelle.

### Modifié

- **`DB::transaction` peut désormais retourner `Err` après un commit réussi**,
  quand un callback après commit échoue : le message dit `after-commit callback
  failed (the transaction itself committed): …`, la valeur de retour de la
  fermeture est perdue et ses écritures ne le sont pas.
  `DB::transaction_with_attempts` ne réessaie jamais cette erreur, quelle que
  soit l'allure d'interblocage du message du callback - réexécuter une
  fermeture dont les écritures sont déjà durables les appliquerait deux fois.
- **Nouvelle clé de catalogue de validation :
  `validation-password-unverifiable`.** Un `UncompromisedVerifier`
  personnalisé qui retourne `Err` ne met plus le texte de sa propre erreur mot
  pour mot dans le corps 422. Ce texte est désormais journalisé au niveau
  `error`, et la réponse porte cette clé, qui s'affiche comme « The { $field }
  could not be checked against known data leaks. Please try again. » - la
  vérification n'a pas eu lieu, ce qui n'est pas la même chose qu'un mauvais
  mot de passe, et un détail d'infrastructure n'a pas sa place dans une
  réponse au client. Une application qui livre son propre catalogue de
  validation doit ajouter la clé, sans quoi ses utilisateurs voient le repli
  anglais intégré.
- **Le validateur de téléversement `Image` s'appelle désormais `ImageFile`.**
  `suprnova::Image` est le nouveau type du pipeline de manipulation d'images,
  correspondant à `Illuminate\Image\Image`, et la règle de téléversement par
  octets magiques prend le nom que Laravel donne à la même classe de règle,
  `Illuminate\Validation\Rules\ImageFile`. La migration tient en une ligne par
  site d'utilisation : `UploadedFile<(Image, MaxSize<N>)>` devient
  `UploadedFile<(ImageFile, MaxSize<N>)>`. Churn pré-1.0 absorbé par le modèle
  de distribution par tag git.

### Supprimé

- **La dépendance directe inutilisée `image` a disparu.** C'était une
  dépendance de base sans aucun site d'utilisation nulle part dans le
  workspace, qui tirait pour rien les codecs JPEG, PNG, WebP et GIF ; la
  retirer supprime `gif`, `image-webp`, `zune-jpeg`, `color_quant` et `weezl`
  de l'arbre. Le crate lui-même apparaît encore de façon transitive, avec sa
  seule feature `png`, derrière le rendu de QR codes de `totp-rs`. Le nouveau
  sous-système d'images est bâti sur les crates OxideAV, derrière la feature
  `media`.

### Corrigé

- **Installer OAuth n'impose plus la validation de liaison web de Magnetar aux
  applications adossées à un fournisseur.** Le chemin complet
  `init_magnetar` reste atomique et inchangé. Le chemin OAuth seul réserve les
  emplacements de moteur pendant la construction, ne publie qu'OAuth, et échoue
  plutôt que de mélanger deux autorités d'authentification.

### Mise à niveau

- **`Image` est un type différent maintenant ; le validateur de téléversement
  est `ImageFile`.** Cassant à la source pour quiconque utilise la règle de
  téléversement par octets magiques. Renommez-la à chaque site d'utilisation :
  `UploadedFile<(Image, MaxSize<N>)>` devient
  `UploadedFile<(ImageFile, MaxSize<N>)>`. `suprnova::Image` se résout
  toujours, mais c'est désormais le type du pipeline de manipulation d'images :
  un renommage oublié échoue donc à la compilation plutôt que de changer le
  comportement en silence.
- **`EnvelopeOverrides` a gagné un champ public `after_commit: Option<bool>`.**
  Chaque construction dans ce dépôt et dans les templates scaffoldés utilise
  `..Default::default()`, ce qui ne demande aucun changement. Le code qui
  construit un `EnvelopeOverrides` avec un littéral de struct exhaustif doit
  nommer le nouveau champ ; `after_commit: None` conserve le comportement
  actuel, qui est de s'en remettre à `Job::after_commit()`. Rien d'autre ne
  change : `after_commit()` vaut `false` par défaut, donc aucun job existant ne
  se met à attendre un commit qu'il n'attendait pas avant.
- **`Envelope` a gagné un champ public `unique_lock_owner: Option<String>`.**
  Le format réseau est inchangé - le champ est `#[serde(default)]` et omis
  quand il vaut `None`, si bien que les enveloppes font l'aller-retour octet
  pour octet dans les deux sens et que `schema_version` reste à 2 - mais tout
  code qui construit un `Envelope` avec un littéral de struct doit désormais le
  nommer. Ajoutez `unique_lock_owner: None`, sauf si vous portez délibérément
  un verrou d'unicité à travers le push. Le code qui ne fait que lire des
  enveloppes, ou qui les construit via `Queue::push` et ses homologues, n'a
  besoin d'aucun changement.

- Utilisez `init_magnetar_oauth_only` au lieu de `init_magnetar` quand
  l'application possède déjà les utilisateurs, les mots de passe, les sessions
  de framework et l'état remember-me. Les callbacks OAuth seul utilisent
  `verify_oauth_identity` ; les applications Magnetar complètes continuent
  d'utiliser `complete`.

## 1.3.2 - 2026-08-25

### Ajouté

- **Les fournisseurs OAuth peuvent désormais s'enregistrer via
  `MagnetarConfig::oauth`.** Suprnova réexporte le contrat `OAuthProvider`, les
  cinq types de fournisseur et de configuration internes, ainsi que les types
  HTTP, de révocation, de limiteur d'abus, d'autorisation et de liaison
  automatique dont une application a besoin. Les fournisseurs personnalisés
  n'exigent plus de dépendance directe à `suprnova-magnetar` ni de
  `MagnetarHostEngine` conservé à la main.

- **Un transport OAuth de production et un adaptateur de limiteur du framework
  sont désormais livrés à la racine de la crate.** `ReqwestOAuthTransport`
  implémente les E/S de jetons, de userinfo et de révocation, redirections
  désactivées par défaut, avec un délai de 30 secondes, un `User-Agent` par
  défaut et un plafond de réponse de 1 Mio. `FrameworkAbuseLimiter` réutilise le
  `RateLimiterDriver` configuré ; les applications n'écrivent plus l'un ni
  l'autre adaptateur à la main.

### Corrigé

- **`init_magnetar` publie désormais OAuth avec les services de mot de passe et
  de passkey comme une seule installation réservée.** Le service OAuth est
  construit avant la publication, et les trois emplacements de moteur restent
  cachés tant que la réservation est active. Une configuration OAuth en échec
  ou dupliquée ne peut pas laisser visibles l'état de mot de passe et de
  passkey sans le registre OAuth configuré.

- **Les fournisseurs personnalisés peuvent fournir des en-têtes userinfo.**
  `OAuthProvider::userinfo_headers` est fusionné avec l'en-tête bearer possédé
  par l'hôte, ce qui rend possibles des exigences comme le `User-Agent` de
  GitHub et les en-têtes `Accept` de type média, sans permettre à un
  fournisseur de remplacer `Authorization`.

### Mise à niveau

- **La bascule Magnetar de `4faaa933` a retiré le chemin d'installation OAuth
  de Torii sans câbler son remplaçant dans l'initialiseur par défaut.**
  L'ancien contournement exigeait de construire un moteur hôte personnalisé,
  d'appeler `oauth_service` et d'installer l'adaptateur séparément. Remplacez
  ce contournement par
  `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` et un seul appel
  à `init_magnetar`.

- **Les fournisseurs communautaires GitHub doivent traiter explicitement
  l'e-mail vérifié.** Le `/user` de GitHub omet généralement l'e-mail non
  public, tandis que l'adresse principale vérifiée exige `/user/emails`.
  Renvoyez `email: None` pour utiliser la cérémonie de complétion d'e-mail, ou
  faites pointer `userinfo_endpoint` vers un adaptateur hôte qui combine les
  deux réponses ; ne traitez jamais une adresse publique mais non vérifiée
  comme une preuve de propriété.

## 1.3.1 - 2026-08-24

### Corrigé

- **Les applications adossées à un fournisseur peuvent de nouveau
  réinitialiser les utilisateurs vérifiés.** Quand aucun moteur Magnetar n'est
  installé, `PasswordReset` utilise un `UserProvider` explicitement capable de
  réinitialisation et les `auth_flow_tokens` du framework pour les comptes déjà
  vérifiés. `EloquentUserProvider<M>` y adhère quand `M` implémente
  `MustVerifyEmail + CanResetPassword` ; aucune migration `app_users` n'est
  requise.
- **La ligne de framework publiée contient désormais les deux jeux de
  réparations postérieures à la publication.** La mise en page et les titres du
  changelog 1.3.0 traduit, le retour à la ligne CJK, les ancres localisées, les
  termes du glossaire et la ponctuation de la prose sont réconciliés au lieu
  d'être répartis entre des branches locale et distante divergentes.
- **Le durcissement de la CLI et de Magnetar postérieur au tag est inclus.** Le
  nettoyage des processus de développement utilise le repli par groupe de
  processus désormais achevé, et les contrats de qualification locaux couvrent
  les refs publiées et les voies SQLite du SDK de plugins.

### Sécurité

- **Le repli par fournisseur ne traite jamais la réinitialisation de mot de
  passe comme une première preuve de boîte aux lettres.** Les adresses
  inconnues et les adresses non vérifiées reçoivent la même réponse sans envoi
  de courrier. Installez Magnetar quand un compte non vérifié doit prouver la
  propriété de sa boîte aux lettres par une réinitialisation, afin que le
  nettoyage des identifiants, l'avancement de l'époque d'authentification et la
  révocation restent atomiques. L'achèvement du repli par fournisseur signale
  les échecs de révocation de la session framework et de l'option « se souvenir
  de moi » via `PasswordResetOutcome`.

### Mise à niveau

- **Déplacez chaque dépendance Git `v1.3.0` vers `v1.3.1`.** Les applications
  qui ont leur propre table `users` gardent leur `UserProvider` configuré ;
  elles n'initialisent pas le moteur `app_users` par défaut simplement pour
  réinitialiser un compte déjà vérifié. Les applications qui utilisent les
  identifiants Magnetar ou la première preuve d'un compte non vérifié
  continuent d'initialiser Magnetar.

## 1.3.0 - 2026-08-24

### Sécurité

- **Magnetar cantonne désormais les mutations d'identifiants et de session à
  l'acteur authentifié et à l'époque d'authentification du compte.** Les
  écritures de mot de passe, passkey, compte lié, deux facteurs, session
  opaque, JWT, remember, OAuth et autorisation d'appareil rejettent les acteurs
  périmés ou révoqués. La première preuve réussie
  d'e-mail vérifié - par réinitialisation de mot de passe, par lien magique ou
  par OAuth - sur un compte non vérifié fait avancer l'époque et retire de
  façon atomique les identifiants provisoires, les sessions, l'état remember
  et l'enregistrement TOTP d'un squatteur. Les comptes vérifiés conservent
  leurs identifiants légitimes pendant la réinitialisation du mot de passe. La
  vérification d'e-mail exige le propriétaire du jeton authentifié, et OAuth ne
  lie jamais automatiquement un compte existant non vérifié sur la seule foi de
  l'e-mail.

- **Une `_previous.url` relative au protocole ne peut plus produire de
  redirection ouverte hors-origine à travers `Redirect::back()`, ni du côté
  écriture ni du côté lecture.** `SessionMiddleware` ne persiste plus une URL
  courante relative au protocole : l'écriture passe par l'assainisseur
  identique à celui qu'`InertiaValidationRedirectMiddleware` emploie pour sa
  vérification du `Referer`, et un chemin de requête de la forme `//host` (ou
  portant un octet de contrôle ASCII) n'est jamais enregistré - sans cela, la
  route `fallback!` d'une application (le motif standard de coquille
  applicative Inertia/SPA, où tout chemin non apparié répond `200`) pouvait
  laisser `GET //evil.test/anything` persister ce chemin mot pour mot.
  `SessionData::previous_url()` applique désormais la même vérification à
  chaque **lecture** également, si bien qu'un cookie de session ayant survécu à
  une montée de version depuis une release antérieure à ce correctif - portant
  déjà une valeur brute, non assainie, qu'aucune écriture du processus courant
  n'a jamais produite - s'auto-répare en « rien d'enregistré » au lieu d'être
  cru sur parole. Ensemble, ni un vieux cookie empoisonné ni une nouvelle
  requête malveillante ne peuvent remettre un `Location` hors-origine à
  `Redirect::back()`, `Redirect::refresh()` ou `url::previous()`. Quand une
  valeur échoue à l'une ou l'autre vérification, elle est traitée comme absente
  plutôt que remplacée par une valeur synthétisée : une URL précédente
  réellement bonne n'est donc jamais écrasée.
- **La vérification du `Referer` du pont de redirection de validation Inertia a
  fermé deux contournements same-origin de plus.** La cible `303`
  d'`InertiaValidationRedirectMiddleware` ne rejetait qu'un `Referer`
  commençant par le préfixe littéral `//` ou `/\` - une valeur comme
  `Referer: /<TAB>/evil.test` passait au travers, parce que l'analyseur d'URL
  WHATWG retire la tabulation et le saut de ligne ASCII de la chaîne entière
  avant de comparer les origines, si bien qu'un navigateur lit cela comme
  `//evil.test` et suit le `303` hors-origine. La vérification rejette
  maintenant tout octet de contrôle ASCII (C0 ou DEL) n'importe où dans le
  candidat, et pas seulement à l'intérieur des deux préfixes nommés. Par
  ailleurs, le repli de dernier recours - le chemin de la requête en échec
  elle-même, utilisé quand ni le `Referer` ni l'URL précédente de la session ne
  sont utilisables - n'était jamais assaini : une request-target HTTP de forme
  origin a syntaxiquement le droit de commencer par `//`, si bien qu'un client
  brut ou un proxy qui ne normalise pas pouvait transformer le « dernier
  recours sûr » en redirection hors-origine lui aussi. Les deux branches
  partagent désormais une seule vérification relative à la racine, et
  retombent sur `/` si même le chemin de la requête y échoue.
- **Le texte chiffré d'un cookie est désormais lié à son nom logique par une
  AAD contextuelle v2.** `Cookie::encrypted` / `Cookie::read_encrypted_for`
  empêchent une valeur frappée pour un emplacement de cookie d'être déchiffrée
  dans un autre, tandis que la liaison au nom logique fait qu'une bascule
  ultérieure de préfixe réseau `__Host-` / `__Secure-` reste sûre. La fenêtre de
  compatibilité sans version essaie v2 sur tout le trousseau de clés, puis v1
  sur tout le trousseau, si bien que les cookies existants survivent au
  rollout ; le repli v1 préserve l'ancienne faiblesse de rejeu jusqu'à sa
  suppression prévue en 1.4.0.
- **Les préfixes des cookies de session et « se souvenir de moi » sont validés
  à l'amorçage et appliqués au rendu.** `SESSION_COOKIE_PREFIX=__Host-` exige
  `Secure`, `Path=/` et aucun `Domain` ; `__Secure-` exige `Secure`. Une
  combinaison invalide à l'amorçage échoue avant de servir, et le moteur de
  rendu réécrit les en-têtes préfixés invalides au lieu de laisser les
  navigateurs les écarter en silence.

### Ajouté

- **L'authentification Suprnova tourne désormais sur le moteur Magnetar
  interne.** La façade `Auth`, propriété du framework, préserve les sites
  d'appel existants de mot de passe, lien magique, passkey, OAuth, bearer,
  verrouillage, session et deux facteurs tout en supprimant la dépendance à
  Torii. Le moteur par défaut installe de façon atomique les adaptateurs mot
  de passe/session et passkey, stocke les baux de livraison du cycle de vie
  dans la base de données de l'application, et partage les identités
  `app_users` canoniques en `i64` de l'application.
- **Un exécuteur de migration d'authentification conscient de la forme couvre
  désormais les sources Torii, Suprnova web et Suprnova API.** Les essais à
  blanc lient un identifiant de plan stable à des empreintes durables de
  lignes et de schéma, ainsi qu'aux décisions d'identité de destination.
  La phase d'application utilise des imports transactionnels, des registres de
  réessais, un nettoyage propre à chaque forme et le refus des collisions.
  MySQL utilise un échange fantôme protégé par barrière d'écriture, avec
  journaux de pré-copie, parité de lignes et de schéma, renommages reprenables
  et restauration qui préserve le nettoyage.
- **`MAIL_DRIVER=file` écrit un `.eml` RFC 5322 par message** dans
  `MAIL_FILE_PATH` (par défaut `storage_path("mail")` ; une valeur relative
  s'ancre au répertoire de base de l'application, pas au répertoire courant du
  processus), si bien que le courrier local peut s'ouvrir dans un client de
  messagerie au lieu de se lire dans une ligne de journal. Le fichier porte le
  même sur-ensemble d'en-têtes que SMTP émet, y compris `X-Priority`,
  `Importance`, `X-Tag`, `X-Metadata-*` et `Return-Path`. Comme `log` et
  `memory`, il ne livre pas : un amorçage en production le refuse à moins que
  `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **`FrameworkError::External` porte l'erreur qu'elle enveloppe.**
  `FrameworkError::from_external(e)` et
  `FrameworkError::from_external_with("saving user", e)` gardent l'erreur
  d'origine atteignable comme source `std::error::Error` au lieu de la fondre
  en une chaîne de caractères. `FrameworkError::external_source()` la renvoie
  pour le downcasting - utilisez cela plutôt que `source()`, qui donne le
  handle `Arc` partagé. Les deux constructeurs correspondent à un HTTP 500.
- **Les journaux 5xx rendent désormais la chaîne complète des sources
  d'erreur.** `render_error_chain` parcourt `source()` et est câblé dans la
  ligne de journal d'erreur du framework, dans le payload de l'événement
  `ErrorOccurred` et dans le champ `debug_message` émis sous `APP_DEBUG=true`.
  Les corps de réponse destinés au client sont inchangés et les corps 5xx
  restent assainis.
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` /
  `try_scroll_wrapped`.** Imbriquez l'instruction de fusion d'une prop de
  scroll sous `<key>.<wrap_key>` au lieu de la clé nue - `mergeProps:
  ["users.data"]` plutôt que `["users"]` - pour une valeur qui est elle-même
  une enveloppe (`{ data: [...], meta: {...} }`). Le `ScrollProp` de Laravel
  enveloppe sous `"data"` sans condition ; les paginateurs intégrés de
  Suprnova rendent un tableau de lignes nu, si bien que c'est en opt-in
  plutôt qu'un défaut que chaque appelant devrait contourner. Le nouveau trait
  `ProvidesScrollMetadata` (`page_name` / `previous_page` / `next_page` /
  `current_page`, avec un `scroll_metadata()` par défaut) reflète l'interface
  Laravel du même nom pour un paginateur que cette crate ne connaît pas ;
  `LengthAwarePaginator`, `Paginator` et `CursorPaginator` l'implémentent
  maintenant au lieu de construire `ScrollMetadata` à la main. Les champs
  `.match_on(...)` d'une prop de scroll émettent désormais aussi dans
  `matchPropsOn`, à l'image du `resolveMergeMatchingKeys` de Laravel
  (`Response.php:641-652`), qui replie le `matchesOn()` d'un `ScrollProp`
  comme celui de n'importe quelle autre prop de fusion - l'entrée de
  correspondance se cale là où la prop fusionne réellement, `<key>` sans
  enveloppe ou `<key>.<wrap_key>` sous `.scroll_wrap(...)`.
- **`Prop::merge_with_path`, `match_on` multi-champs et props de fusion
  adossées à un résolveur.** `Prop::merge_with_path(path)` fusionne un champ
  imbriqué à l'intérieur de la valeur d'une prop au lieu de la prop entière -
  `Prop::eager(v).merge().merge_with_path("data")` émet
  `mergeProps: ["<key>.data"]`, et une prop qui fusionne par chemin ne fusionne
  jamais aussi sa racine ; `.deep_merge()` l'ignore, puisqu'une fusion profonde
  descend déjà dans chaque champ. `Prop::match_on` prend maintenant un champ ou
  plusieurs en un seul appel (`match_on(["id", "slug"])`), en plus du chaînage
  `match_on("id").match_on("slug")` que la composition de `Prop` supportait
  déjà. `InertiaResponse::merge_lazy` / `merge_lazy_with` ajoutent les
  homologues de `.merge` / `.merge_with` adossés à un résolveur, à l'image de
  l'`Inertia::merge(fn () => ...)` de Laravel.
- **Les `only`/`except` du rechargement partiel comprennent la notation
  pointée.** `X-Inertia-Partial-Data: user.name` restreint la prop `user` à
  `{ name: ... }` au lieu d'exiger la valeur entière ou rien ;
  `X-Inertia-Partial-Except: user.email` élague ce seul champ et laisse le
  reste de `user` en place. `except` l'emporte sur un chemin que les deux
  en-têtes nomment, une entrée nue signifie toujours la prop entière, et un
  chemin imbriqué inconnu ou de type incompatible disparaît en silence sans
  toucher à ses voisins. Les props `Always` ne sont pas affectées - elles
  partent toujours entières.
- **Imbrication de props par clé pointée.** `.with("user.name", value)` (et
  toute autre méthode qui attache une prop, eager ou résolue) s'imbrique
  désormais dans `props.user` au lieu d'envoyer une clé littérale
  `"user.name"`, à l'image du dépaquetage `resolveArrayableProperties` de
  Laravel fondé sur `Arr::set`. Deux appels qui partagent un préfixe -
  `.with("user.name", …)` puis `.with("user.age", …)` - s'accumulent en un seul
  objet ; une clé sans point n'est pas affectée. Les clés du registre partagé
  `App::inertia_share*` s'imbriquent de la même façon sur le réseau. Le
  dépaquetage ne touche jamais qu'aux *clés* de props de premier niveau - il ne
  descend jamais dans la valeur d'une prop, si bien qu'un sac d'`errors` de
  validation garde tous les noms de champs pointés qu'il porte en interne.
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`.** Les
  `Inertia::getShared` / `Inertia::flushShared` de Laravel, qui lisent et
  vident le registre statique de partage (`App::inertia_share` / `_lazy` /
  `_once`). `inertia_shared` supporte la même notation pointée
  qu'`inertia_share` du côté lecture ; il renvoie `None` pour un partage lazy
  ou once (il n'y a pas de requête contre laquelle le résoudre) et pour une clé
  non enregistrée. `flush_inertia_shared` ne vide que le registre statique - un
  fournisseur de trait enregistré via `App::register_inertia_shared` n'est pas
  touché, comme chez Laravel (il n'y a pas d'état par requête à vider là-bas).
- **`InertiaResponse::always_with(key, resolver)`.** L'homologue à résolveur
  asynchrone de `.always(key, value)`, pour une prop toujours incluse assez
  coûteuse pour valoir une résolution paresseuse - c'est
  l'`Inertia::always(fn () => …)` de Laravel (`AlwaysProp` accepte n'importe
  quelle valeur, closures comprises).
- **`InertiaSharedData::share` reçoit désormais le nom du composant de page**,
  si bien qu'un fournisseur peut faire varier sa sortie selon la page - le
  `RenderContext` de Laravel. Voir Mise à niveau.
- **Composition de props Inertia.** Une `Prop` porte désormais des flags
  orthogonaux au lieu d'être l'une de neuf variantes fermées, si bien qu'une
  seule prop peut être deferred *et* fusionnable, fusionnable *et* mise en
  cache, ou optionnelle *et* mise en cache - les combinaisons que le protocole
  Inertia 3 attend et qu'une énumération fermée ne pouvait pas épeler.
  Construisez-en une avec `Prop::eager` / `Prop::lazy` / `Prop::from_resolver`
  / `Prop::absent`, chaînez `.always()`, `.optional()`, `.defer()`, `.group()`,
  `.rescue()`, `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on()`,
  `.once()`, `.as_key()`, `.until()`, `.fresh()`, `.scroll()`, et attachez-la
  avec le nouveau `InertiaResponse::prop(key, prop)`. Une prop
  `defer().merge()` est annoncée sous `deferredProps` au premier rendu et
  arrive sous `mergeProps` à la requête suivante. Les nouveaux types
  `MergeMode` et `Visibility` décrivent les flags ; chaque raccourci de
  constructeur existant (`.with`, `.always`, `.lazy`, `.optional`, `.defer`,
  `.merge*`, `.once*`) est inchangé.
- **Mise en pause / reprise de file.** `Queue::pause(connection, queue)` /
  `resume` / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` /
  `paused_queues(connection, &queues)`, adossés au `Cache` de la même façon que
  le signal de redémarrage - `resume_all` ne lève pas une pause par file, comme
  chez Laravel. La porte de réclamation du worker se place juste avant chaque
  pop, si bien qu'un job en vol se termine toujours ; une pause globale
  court-circuite le filtrage `--queue=...` de la même façon que le
  `pausedQueues` de Laravel, et une pause par file ne prend effet que sur un
  worker démarré avec une liste `--queue=...` explicite. Nouvelles commandes
  CLI `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]` (alias
  `queue:continue`), plus `QUEUE_PAUSABLE=false` pour qu'un opérateur désactive
  la fonctionnalité - un worker non pausable ignore les signaux de pause, et
  `queue:pause` lui-même refuse de s'exécuter. Nouveaux événements :
  `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed`.
- **`suprnova::testing::TestResponse`** - un wrapper fluide, de la forme du
  `TestResponse` de Laravel, autour du triplet `(status, headers, body)` que
  tout harnais de test HTTP produit déjà : `assert_status`, `assert_ok`,
  `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`,
  `assert_see`, `assert_header`, `assert_cookie` et (moyennant
  `.with_session_store(...)`) `assert_session_has`. Chaque assertion renvoie
  `&Self` et panique en cas d'échec, le même contrat qu'`expect!`. Rien de la
  façon dont un test pilote une requête n'a à changer.
- **`suprnova new` crée un point d'entrée SSR.** Chaque starter (Svelte, React,
  Vue) livre désormais `frontend/src/ssr.{ts,tsx}` et un script npm
  `build:ssr` (`vite build --ssr`), câblé sur son propre répertoire de sortie
  (`frontend/bootstrap/ssr/`) pour que le bundle SSR n'entre jamais en
  collision avec le build client dans `public/assets/`.
- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`.**
  La passerelle SSR peut maintenant vérifier que le bundle construit existe sur
  disque avant de dispatcher un rendu, à l'image de la configuration
  `ensure_bundle_exists` de Laravel - un worker qui n'a jamais été démarré, ou
  un bundle qui n'a jamais été construit, échoue tôt au lieu de payer
  `ssr_timeout` sur une connexion qui n'avait aucune chance d'aboutir. Activez
  avec `.ssr_bundle_path(...)` ; contrairement au `BundleDetector` de Laravel,
  le chemin n'est jamais détecté automatiquement, si bien que les
  configurations SSR existantes (et les tests) qui n'en définissent pas ne sont
  pas affectées.
- **Les échecs de validation sur une visite Inertia redirigent désormais en
  arrière au lieu de renvoyer un JSON `422`.** `Inertia::install` enregistre un
  quatrième middleware, `InertiaValidationRedirectMiddleware`, qui transforme
  un `422` de validation sur une requête `X-Inertia` en un `303` vers la page
  du formulaire, avec les erreurs en flash - si bien que `useForm().errors` se
  remplit sans code de handler. Le client Inertia considère toute réponse sans
  en-tête `X-Inertia` comme non-Inertia et affiche sa modale d'erreur, si bien
  que l'ancien `422` ne pouvait jamais atteindre `form.errors`. Les requêtes
  non-Inertia conservent l'enveloppe `422`, les essais à blanc de Precognition
  ne sont pas touchés, et `X-Inertia-Error-Bag` cantonne le sac mis en flash.
  La cible de la redirection est le `Referer` same-origin, puis l'URL
  précédente de la session, puis le chemin de la requête elle-même passé par ce
  même assainisseur, avec repli sur `/` si même celui-ci échoue - jamais cru
  mot pour mot.
- **`InertiaConfig::with_all_errors(bool)`** - conserve chaque message de
  validation par champ au lieu de réduire au premier. Reflète le
  `Inertia\Middleware::$withAllErrors` de Laravel.
- **`suprnova::testing::AssertableInertia`** - des assertions fluides, de la
  forme de l'`AssertableInertia` de Laravel, sur un objet page Inertia, analysé
  soit depuis une réponse JSON `X-Inertia`, soit depuis l'élément
  `<script data-page="app">` embarqué dans la coquille HTML d'une navigation
  dure : `component`, `url`, `version`, `prop`, `has`, `missing`, `where_`,
  `count`, `has_flash`. Construisez-en une depuis un `HttpResponse` avec
  `AssertableInertia::from_response`, ou depuis un `TestResponse` avec le
  nouveau `TestResponse::assert_inertia()`. `reload_only`, `reload_except` et
  `load_deferred_props` rejouent un rechargement partiel contre une closure
  `with_reload(...)` fournie par l'appelant - les tests HTTP de Suprnova
  traversent une vraie socket, il n'y a donc pas de client de test in-process
  unique à coder en dur.
- **`Cookie::queue`/`queued`/`unqueue`/`expire`.** Un pot à cookies
  task-local - le `CookieJar` de Laravel - permet à n'importe quel code de
  mettre un cookie en file pour la prochaine réponse sortante sans détenir un
  `HttpResponse` auquel l'attacher : un écouteur d'événement, un service lié au
  conteneur, un middleware en amont du handler. Adossé au même emplacement par
  requête qu'`Auth::login_remember` utilise déjà pour porter le cookie « se
  souvenir de moi » au-delà de la frontière du handler ;
  `SessionMiddleware` le vide sur la réponse à côté du cookie de session.
  `Cookie::expire(name, path, domain)` met en file un cookie de suppression
  construit avec `Cookie::forget_with`. Exige `SessionMiddleware` dans la
  chaîne de middlewares de la route - en dehors, les quatre appels sont sans
  effet en silence, comme le comportement d'`App::flash` hors d'une portée de
  flash.
- **`HttpResponse::event_stream(stream, end)` et
  `HttpResponse::stream_json(stream)`.** Les `ResponseFactory::eventStream` /
  `streamJson` de Laravel, et exactement les formes réseau que les
  `useEventStream` / `useJsonStream` de `@laravel/stream-{react,vue,svelte}`
  attendent. `event_stream` encadre un `Stream<Item = sse::StreamedEvent>` en
  `event: update` par élément, à moins que l'élément ne nomme son propre
  événement, encode en JSON tout payload non textuel, et ajoute une trame
  terminale configurable (`EndSignal::default()` vaut `data: </stream>` ;
  `EndSignal::None` l'omet). `stream_json` diffuse n'importe quel
  `Stream<Item = impl Serialize>` comme un unique tableau JSON vidé au fil de
  l'eau. Les deux sont bâtis sur le pipeline de corps `sse`/`stream_bytes`
  existant, et partagent donc son comportement d'annulation et d'isolation des
  paniques avec le reste du framework.
- **`suprnova serve` relance un processus de développement qui a planté au lieu
  d'abattre toute la session.** Backoff exponentiel entre les tentatives -
  200 ms, doublé à chaque plantage consécutif, plafonné à 5 s, revenant au
  plancher dès qu'un processus est resté debout 30 s. `--no-restart` s'y
  soustrait et restaure le comportement précédent. `--restart-tries <N>` (par
  défaut `5`, comme le `--restart-tries=5` de Laravel) cesse de réessayer un
  processus après ce nombre de plantages consécutifs au lieu de réessayer
  indéfiniment, en imprimant un message actionnable et en laissant tourner les
  autres processus - et la session elle-même. `--timestamps` préfixe chaque
  ligne relayée par `HH:MM:SS`. Un nouveau tableau `[[serve.process]]` dans
  `Suprnova.toml` permet à un projet de déclarer ses propres processus de
  développement - le `DevCommands::register` de Laravel - pour les faire
  tourner aux côtés du backend et du frontend, chacun avec son propre préfixe
  `[name]` et une couleur optionnelle ; une clé inconnue ou un `name`/`command`
  vide dans une entrée est maintenant une erreur d'analyse dure au lieu d'être
  ignorée en silence ou de devenir plus tard un échec de spawn opaque. `--json`
  émet à la place un objet JSON par ligne (NDJSON) sur stdout - événements de
  démarrage de processus, de sortie, de fin, de redémarrage planifié, de
  redémarrage réussi, d'abandon, de types régénérés et d'arrêt, y compris les
  avis de régénération du watcher de fichiers et l'avis d'arrêt du handler de
  `Ctrl+C`, qui restent tous deux désormais hors de stdout sous `--json`
  également - pour le scripting et les pipelines de journaux ; le combiner avec
  `--timestamps` est inoffensif mais redondant, puisque chaque événement porte
  déjà son propre horodatage.
- **`RequestBuilder::retry_when(predicate)`.** Un prédicat consulté avant
  chaque réessai que la politique intégrée (`.retry(...)` /
  `.retry_non_idempotent(...)`) aurait sinon fait, qui reçoit un
  `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }`.
  Il se compose avec la politique plutôt que de la remplacer : `false` oppose
  son veto à un réessai que la politique aurait fait ; il ne peut jamais en
  forcer un au-delà de `max_attempts`, ni un que la politique n'aurait pas
  tenté autrement (un statut 4xx, ou une méthode non idempotente sans
  `retry_non_idempotent`).
- **`#[model(touches = [...])]` touche désormais vraiment.** Après qu'un enfant
  a été créé, sauvegardé, mis à jour ou supprimé, chaque propriétaire
  `BelongsTo` nommé dans la liste reçoit un
  `UPDATE <owner> SET updated_at = ? WHERE <key> = ?`, sur le même exécuteur
  que l'écriture qui l'a déclenché - à l'intérieur d'un `DB::transaction`, le
  touch rejoint donc cette transaction et est annulé avec elle. Un propriétaire
  dont le modèle porte `timestamps = false` est ignoré : ni écrit ni erreur
  (Laravel 13.25 a comblé la même lacune). Les propriétaires atteints par une
  clé étrangère `NULL`, ainsi que les propriétaires en suppression logicielle,
  sont ignorés eux aussi. Une entrée `touches` qui ne nomme pas une relation
  `BelongsTo` déclarée est désormais une erreur de compilation ; les
  propriétaires polymorphes ne sont pas encore supportés.
- **`without_touching_on::<M, _, _>(fut)`** - le
  `Model::withoutTouchingOn([M::class], $cb)` de Laravel. Supprime à la fois
  `m.touch()` et toute cascade de propriétaire visant `M`, tandis que les
  propriétaires d'autres types continuent d'être touchés. Les portées
  s'imbriquent, et le `without_touching` existant supprime désormais la cascade
  de propriétaires en plus des appels directs à `touch()`.
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - le `touchOwners()`
  de Laravel, pour quand vous avez écrit la ligne enfant par un chemin que le
  framework ne possède pas.
- **Règles de validation en forme de valeur : `ArrayKeys` et `Distinct`.** Un
  nouveau trait `ValueRule` (`passes(&self, value: &serde_json::Value)`) se
  place aux côtés de `Rule`, en partageant le même contrat de message par clé.
  `rules::ArrayKeys(&[...])` rejette un objet JSON portant une clé hors de la
  liste autorisée (l'`array:keys` de Laravel, #60918) ;
  `rules::Distinct { ignore_case, strict }` rejette un tableau JSON contenant
  un élément répété (le `distinct` de Laravel). Les lignes de `validate!`
  acceptent l'un ou l'autre type de règle dans la même liste de champs - le
  dispatch est automatique, choisi selon le trait que la règle implémente et
  non par une nouvelle syntaxe de ligne.
- **`Job::delay()`** - un job peut déclarer un délai par défaut
  (`fn delay() -> Option<Duration>`, `None` par défaut), honoré par
  `Queue::push` et `Queue::bulk` : `available_at` devient `now + delay` au lieu
  de `now`. Un délai explicite au site d'appel l'emporte toujours -
  `Queue::push_later(job, at)` et `Queue::later(delay, job)` utilisent
  l'horodatage de l'appelant mot pour mot et ne consultent jamais
  `Job::delay()`.
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`.**
  Une notification mise en file (`Notify::queue`) porte désormais ses propres
  valeurs par défaut de réglage de file sur chaque push de
  `SendNotificationJob` par canal, via la primitive `EnvelopeOverrides`
  qu'utilise `Mail::on_queue` - `fail_on_timeout(&self) == true` met en lettre
  morte au premier dépassement de délai au lieu de réessayer, à l'image de
  l'attribut de notification `#[FailOnTimeout]` de Laravel (#61072). Les cinq
  reprennent par défaut les valeurs `Job` existantes de `SendNotificationJob`,
  si bien qu'une notification qui ne redéfinit rien n'est pas affectée.
- **`Mail::on_queue` / `Mail::on_connection` + `Queue::push_with`/`later_with`.**
  Un mailable mis en file se route désormais lui-même avec
  `Mail::to(..).on_queue("emails").queue(mailable)`, ou par défaut via
  `Mailable::queue(&self)`. Les deux l'emportent sur toute `Queue::route`
  enregistrée pour le job et sur les `Job::queue()`/`Job::connection()` du job
  lui-même - la nouvelle primitive `EnvelopeOverrides` qui les porte
  (`Queue::push_with(job, overrides)` / `Queue::later_with(delay, job, overrides)`)
  couvre aussi le timeout, l'échec sur dépassement de délai, le nombre maximal
  de tentatives et le backoff pour un push. Les instantanés de mise en file de
  `MailFake` portent désormais la `queue` résolue, avec `queued_on(...)` /
  `assert_queued_on(name, queue)` pour l'affirmer.
- **`Application::http_bootstrap(f)`** - un hook d'amorçage réservé à HTTP. Il
  tourne après `bootstrap` et uniquement sur le chemin `serve` / `web:run`, si
  bien que les workers de file, de planification et de workflow, ainsi que le
  binaire console, ne l'exécutent jamais. Les images de conteneur des workers
  et de la console n'ont plus besoin d'un manifeste frontend construit pour
  s'amorcer : `Inertia::install` échoue de façon fermée en production quand il
  manque, et cette vérification ne tourne désormais que sur un processus qui
  sert effectivement du HTTP.
- **`Router::inertia(path, component, props)`** - le `Route::inertia` de
  Laravel, pour une page statique dont le handler tiendrait en une ligne.
  Enregistre `GET` (HEAD y retombe) et renvoie un `RouteBuilder`, si bien que
  la route peut être nommée et recevoir des middlewares. `Router::view` est
  conservé comme alias.
- **Options d'envoi SES v2.** Le transport SES émet désormais `TenantName`,
  `ConfigurationSetName` et `ListManagementOptions` sur `SendEmail`. Chacune a
  une valeur par défaut au niveau du transport
  (`SesMailTransport::tenant_name` / `configuration_set_name` /
  `list_management`) et un remplacement par en-tête et par message
  (`X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`,
  `X-SES-LIST-MANAGEMENT-OPTIONS`), l'en-tête l'emportant. Les en-têtes sont
  consommés à la construction de la requête et ne sont jamais rendus dans le
  message.
- **`without_cookies` sur chaque constructeur de réponse.** `HttpResponse`,
  `Response` (via `ResponseExt`), `Redirect` et `RedirectRouteBuilder` font
  tous expirer une liste de cookies en un seul appel, et `Redirect`
  /`RedirectRouteBuilder` ont gagné le `without_cookie` à nom unique qui leur
  manquait. Le nouveau `Cookie::forget_with(name, path, domain)` construit un
  cookie de suppression cantonné au chemin et au domaine avec lesquels
  l'original avait été posé - un simple `forget` n'efface jamais un cookie posé
  hors de `/`.
- **`Queue::fake()` estampille un identifiant d'enveloppe sur chaque push
  capturé.** `pushed_with_id::<J>()` renvoie des paires `(job, id)`, et le fake
  dispatche désormais la même paire `JobQueueing` / `JobQueued` qu'un push de
  driver réel - en portant cet identifiant - si bien qu'un test peut corréler
  un push capturé avec ce que ses écouteurs ont vu. Les helpers de fake
  existants sont inchangés.
- **Événement de file `UniqueJobSkipped`.** `Queue::push_unique` dispatche
  désormais `queue::events::UniqueJobSkipped { job_name, unique_id, connection }`
  quand il supprime un doublon, si bien qu'une déduplication est observable au
  lieu d'être silencieuse. La valeur de retour de l'appel est inchangée
  (`Ok(false)`).
- **`model_keys()` sur le constructeur de requêtes et sur les collections.**
  `User::query().model_keys().await?` renvoie la clé primaire de chaque ligne
  correspondante sans hydrater le moindre modèle, en projetant la clé qualifiée
  par la table (`users.id`) pour que la requête survive à une jointure.
  `Collection::model_keys()` est le pendant déjà hydraté.
  `#[suprnova::model]` déclare désormais aussi le type Rust de la clé comme
  `EloquentModel::Key`, si bien que les deux renvoient le type que `key_type`
  nomme plutôt qu'un turbofish choisi par l'appelant.

### Corrigé

- **Les suppressions logicielles PostgreSQL utilisent désormais des
  placeholders conscients du backend, et les écritures d'horodatage générées
  honorent les casts déclarés.** `delete()` et `restore()` rendent les
  placeholders ordinaux PostgreSQL au lieu des placeholders `?` de MySQL et
  SQLite. Les écritures générées de création, mise à jour, sauvegarde, touch et
  suppression logicielle convertissent aussi les horodatages via le type de
  stockage `Cast` déclaré de chaque champ, si bien que les colonnes natives
  `TIMESTAMPTZ` ne reçoivent plus de valeurs textuelles. Merci à
  [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) d'avoir signalé
  les deux défauts et soumis un correctif dans la
  [PR #3](https://github.com/eas4ai/suprnova/pull/3).
- **Les exécutions par défaut du gate sur l'espace de travail et sur Magnetar
  n'exigent plus de services PostgreSQL ou MySQL vivants.** Les suites de
  comportement propres à un backend sont des tests de qualification explicites
  et ignorés, qui échouent toujours quand on les invoque délibérément sans leur
  base de données configurée. Les tests de simple accessibilité et les
  exigences permanentes d'environnement du gate ont été retirés, si bien que
  des changements sans rapport ne paient plus une installation de base de
  données externe à chaque exécution de vérification.

- **`PartialFilter::narrow` est maintenant `pub`.** Ses quatre prédicats
  homologues (`should_include`, `should_include_eager`,
  `should_include_optional` et le type lui-même) étaient déjà publics, mais la
  passe de restriction qui rend correcte la réponse `true` de
  `should_include_eager` - élaguer une valeur résolue jusqu'aux seuls chemins
  pointés qu'une entrée `only`/`except` a réellement demandés - était
  `pub(crate)`. Un appelant qui bâtissait un traitement personnalisé du
  rechargement partiel par-dessus `PartialFilter` n'avait aucun moyen public de
  reproduire cette restriction et aurait livré une valeur entière sous une
  entrée `only` pointée, alors même que `should_include_eager` signalait la clé
  comme incluse.
- **Le `QueuedSnapshot` de `MailFake` peut désormais affirmer sur
  `.on_connection(...)`.** `Queue::fake()` avait gagné
  `assert_pushed_on_connection` à la vague 3, aux côtés
  d'`assert_pushed_on_queue` ; `Mail::fake()` n'avait reçu que la moitié
  « file », si bien qu'un mailable mis en file avec un remplacement de
  connexion était résolu et appliqué au dispatch réel, mais impossible à
  affirmer à travers le fake. Les nouveaux `QueuedSnapshot::connection`,
  `MailFake::queued_on_connection` et `MailFake::assert_queued_on_connection`
  comblent la lacune, en reprenant la forme d'`assert_queued_on`.
- **Une prop partagée pointée était inatteignable par une entrée `only` nue.**
  `App::inertia_share("auth.user", …)` suivi de
  `router.reload({ only: ['auth'] })` renvoyait `props: {"errors":{}}` - le
  partage disparaissait purement et simplement. Le registre stocke `auth.user`
  comme une seule clé littérale, et la passe de dépaquetage `Arr::set` ne
  l'imbrique qu'une fois toutes les props résolues, si bien que la porte du
  rechargement partiel voyait la clé encore plate et ne l'appariait ni à `auth`
  ni à quoi que ce soit d'autre. Les entrées `only`/`except` sont désormais
  symétriques : une entrée peut nommer exactement la clé d'une prop, un chemin
  *à l'intérieur* de celle-ci (`user.name`, qui restreint), ou un **ancêtre**
  de celle-ci (`auth` contre la clé `auth.user`, ce qui envoie la prop
  entière, parce que l'appelant a demandé toute la racine). Un
  `except: ['auth']` nu supprime chaque clé de prop en dessous de lui, de la
  même façon qu'`Arr::forget` supprime tout le sous-arbre dans le sac déjà
  imbriqué de Laravel. Le préfixe doit s'arrêter sur une frontière de segment,
  si bien qu'une prop `authAgent.user` sans rapport n'est touchée par aucune
  des deux listes. Laravel ne rencontre jamais cela parce qu'`Inertia::share`
  exécute `Arr::set` au moment du partage ; le registre de Suprnova ne le peut
  pas, puisqu'un partage lazy n'a aucune valeur à imbriquer tant que la requête
  ne l'a pas résolue.
- **Un champ `#[data(lazy(deferred))]` contournait la liste blanche
  `?include=`.** Le chemin de résolution tagué par propriétaire dans
  `resolve_props` sélectionnait les props avec `Prop::is_lazy()`, qui est faux
  pour tout ce qui porte un flag - et un champ deferred est
  `Visibility::Deferred`. Le champ se résolvait donc par le chemin de prop
  ordinaire, où aucune vérification de l'ensemble d'include n'existe, et
  partait vers tout client qui envoyait la requête de suivi deferred, que la
  requête ait ou non explicitement inclus le champ.
  `Prop::resolve_with_owner` filtre maintenant chaque prop taguée par
  propriétaire et adossée à un résolveur, flags ou pas, et `resolve_props`
  exécute ce filtre avant tout autre bloc : un champ hors de `?include=` est
  supprimé en entier (pas de valeur, pas d'annonce `deferredProps`), et un
  champ nommé par `?include=` mais hors de la liste blanche du DTO lève son
  `400` avant que `X-Inertia-Partial-Data` ne puisse l'absorber. Ce n'est pas
  une régression - le code d'avant la vague 4 filtrait sur la variante d'énum
  `Prop::Lazy`, à laquelle un `Prop::Defer` échouait également - mais c'était
  un vrai trou dans les deux cas.
- **`deferredProps` était réannoncé sur un rechargement partiel apparié.** Un
  partiel qui nommait une clé deferred annonçait quand même au client toutes
  les *autres* clés deferred, que celui-ci allait alors rechercher de nouveau,
  et encore au partiel suivant. Le `resolveDeferredProps` de Laravel renvoie
  `[]` dès l'instant où la requête est partielle, avant d'inspecter la moindre
  prop (`Response.php:661-663`) ; le bloc est maintenant supprimé en entier sur
  tout partiel apparié. Un rechargement partiel visant un autre composant est,
  pour cette porte comme pour toutes les autres, une visite standard : ses
  annonces ne sont donc pas affectées.
- **Le sac `errors` filtrait différemment selon l'origine des erreurs.** Le sac
  mis en flash par la session est déposé avant la boucle de résolution et aucun
  filtre de rechargement partiel ne pouvait l'atteindre, tandis que le
  `.with("errors", …)` propre à un handler passait par les portes ordinaires -
  si bien qu'`only: ['errors.email']` envoyait le sac déposé entier mais un sac
  de handler à un seul champ, et qu'`only: ['users']` remplaçait le sac du
  handler par celui qui avait été déposé au lieu de laisser la clé tranquille.
  Les deux chemins traitent désormais `errors` comme toujours visible, à
  l'image du middleware de Laravel, qui le partage avec `Inertia::always(...)`
  et réinjecte la valeur brute via `resolveAlways` après la reconstruction
  `only`/`except`. C'est la forme dont le client a besoin : il replie une
  réponse partielle avec `{...current.props, ...response.props}`, si bien qu'un
  objet `errors` vide efface les messages déjà à l'écran là où un objet non
  filtré les laisse corrects. Un flag de visibilité explicite sur la clé
  l'emporte toujours : `.prop("errors", Prop::eager(…).optional())` se comporte
  donc de façon optionnelle.
- **`Queue::fake()` peut désormais observer les `EnvelopeOverrides` par push.**
  Un job poussé via `Queue::push_with`/`Queue::later_with` était indiscernable
  d'un simple `Queue::push` sous le fake - `FakePush` ne portait que le payload
  et `available_at`, si bien que le remplacement ne quittait jamais la façade
  et que rien ne pouvait affirmer qu'un test avait dispatché vers la bonne file
  ou la bonne connexion. Le nouveau
  `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>`
  renvoie chaque push capturé apparié à ce qu'il a déclaré ;
  `assert_pushed_on_queue::<J>(queue)` et
  `assert_pushed_on_connection::<J>(connection)` couvrent le cas courant à un
  seul champ, en miroir de `MailFake::assert_queued_on`. Tous les autres points
  d'entrée (`push`, `push_later`, `bulk`, `push_unique`, les dispatchers de
  chaîne et de lot) ne prennent toujours aucun remplacement et enregistrent
  `EnvelopeOverrides::default()`, si bien qu'un push simple se lit sous le fake
  exactement comme « aucun remplacement déclaré ».
- **Un worker SSR bloqué en plein corps de réponse pouvait suspendre un rendu
  pour toujours.** `SsrConfig::timeout` ne bornait que l'attente des en-têtes de
  réponse ; une fois les en-têtes arrivés, la lecture du corps n'avait pas de
  délai propre, si bien qu'un worker qui acceptait la connexion, envoyait les
  en-têtes, puis cessait d'envoyer des données laissait la requête suspendue
  au-delà du délai configuré au lieu de retomber sur le CSR (ou d'échouer, sous
  `ssr_throw_on_error`). Les deux phases partagent maintenant une seule
  échéance, si bien que le délai configuré borne tout l'appel SSR, comme sa
  propre documentation le promettait déjà.
- **Les cookies mis en file - y compris le cookie « se souvenir de moi » que
  pose `Auth::login_remember` - étaient supprimés en silence sur trois chemins
  internes de `SessionMiddleware` qui échouent de façon fermée.** Un échec de
  lecture de
  session, un échec d'écriture de session et un échec de chiffrement du cookie
  de session renvoyaient chacun un `500` synthétisé directement, en
  court-circuitant la vidange des cookies en attente qui tourne à la fin de
  `handle`. Tout ce qui avait été mis en file via `Cookie::queue` lors de cette
  requête - y compris une ligne de jeton « se souvenir de moi » déjà validée en
  base de données - n'atteignait jamais le client comme en-tête `Set-Cookie`.
  Les trois chemins vident maintenant les cookies en attente avant de revenir,
  comme une erreur renvoyée par un handler ou une redirection. Cela ne couvre
  pas une panique non rattrapée, à l'image des cookies mis en file de Laravel,
  qui sont eux aussi perdus dans ce cas.
- **`Queue::push_unique` honore désormais `Job::delay()`, comme `Queue::push`,
  `Queue::push_with` et `Queue::bulk`.** Il calculait auparavant `available_at`
  directement depuis `Utc::now()`, si bien qu'un job qui déclarait un délai par
  défaut (`fn delay() -> Option<Duration>`) était dispatché immédiatement quand
  on le poussait via `push_unique`, au lieu de l'être après ce délai.
  `Queue::push_unique_later` et `Queue::later_unique` ne sont pas affectés -
  ils prennent déjà un horodatage ou un délai explicite de l'appelant et ne
  consultent jamais `Job::delay()`, la règle même que suivent
  `push_later`/`later`.

### Modifié

- **La branche de développement courante utilise SeaORM 2.0 et exige Rust
  1.94.0.** Suprnova préserve les formes source de son Eloquent, de son
  `#[model]`, de ses migrations et de sa façade de base de données. Les
  applications qui appellent SeaORM directement doivent importer `ExprTrait`
  pour les méthodes d'expression SeaQuery et utiliser les méthodes de connexion
  `*_raw` explicites pour les valeurs `Statement` préconstruites. SeaQuery est
  désormais en 1.0, et le driver vectoriel MariaDB direct utilise SQLx 0.9. Les
  bases de données existantes n'exigent aucune migration de données
  applicative ; les schémas PostgreSQL neufs conservent des clés primaires
  adossées à des séquences serial.
- **Trois dépendances inutilisées de plus ont été retirées.**
  `pretty_assertions` et `qrcode` quittent la crate du framework (`totp-rs`
  porte déjà la feature `qr`, si bien que le provisionnement de QR code pour
  l'enrôlement à deux facteurs n'est pas affecté), et `notify-debouncer-mini`
  quitte la CLI (`notify` lui-même reste - les watchers de `serve` et de
  `generate-types` l'utilisent directement). Les trois ont été confirmées
  inutilisées par `cargo-udeps` plus une recherche sur toutes les sources qui
  couvre les doc tests.
- **`suprnova-macros` ne dépend plus de `serde` ni de
  `serde_derive_internals`.** Ni l'une ni l'autre n'était utilisée : les
  chemins `::serde::Serialize` que les macros émettent se résolvent dans la
  crate en aval, pas dans la crate de macros elle-même. Aucun effet sur le code
  généré.
- **Le `match_on` de `MergeStrategy` porte désormais plus d'un nom de champ.**
  `Append`, `Prepend` et `Deep` s'élargissent chacun de
  `match_on: Option<String>` à `match_on: Option<Vec<String>>`, si bien que
  `InertiaResponse::merge_with` / `merge_lazy_with` peuvent dédupliquer sur
  plusieurs champs de la même façon que
  `.prop(key, Prop::eager(v).match_on([...]))` le pouvait déjà - avant cela,
  les raccourcis du constructeur de réponse étaient strictement moins
  expressifs que la construction directe d'une `Prop`. Voir Mise à niveau.
- **Les props de scroll émettent désormais des sémantiques de `reset` et de
  fusion identiques à celles de Laravel.** `scrollProps[key].reset` vaut `true`
  exactement quand le client a nommé `key` dans `X-Inertia-Reset`, à l'image du
  `resolveScrollProps` de Laravel - et non `true` à chaque visite dépourvue
  d'en-tête `X-Inertia-Infinite-Scroll-Merge-Intent`, comme auparavant. Une
  prop de scroll porte aussi désormais des métadonnées de fusion sans
  condition, avec l'ajout en fin comme défaut : une visite fraîche (aucun
  en-tête du tout) émet `reset: false` plus une entrée `mergeProps`, là où elle
  émettait auparavant `reset: true` et aucune métadonnée de fusion. Une clé
  présente dans `X-Inertia-Reset` est exclue de `mergeProps` / `prependProps`
  pour cette réponse, la même exclusion dont une prop de fusion ordinaire
  bénéficiait déjà.
- **`ssr:check` vérifie désormais que la route `GET /health` du worker SSR
  répond en 2xx**, au lieu de seulement confirmer que quelque chose a accepté
  une connexion TCP. Chaque worker `@inertiajs/{vue3,react,svelte}/server`
  répond à `/health` d'origine, si bien que cela n'a rien exigé du côté
  worker - c'est l'équivalent de l'`Inertia\Ssr\HttpGateway::isHealthy()` de
  Laravel.
- **La prop Inertia `errors` porte désormais une chaîne par champ, et non un
  tableau.** Un sac de validation mis en flash par la session se rend comme
  `{ email: "The email field is required." }` plutôt que
  `{ email: ["The email field is required."] }`, ce qui correspond au défaut de
  Laravel et à l'`ErrorValue = string` d'Inertia lui-même.
  `InertiaConfig::with_all_errors(true)` restaure la forme en tableau. Une prop
  `errors` qu'un handler définit lui-même est transmise intacte, et le flash de
  session (`Redirect::with_errors`, `session.pull_errors_flash()`) stocke
  toujours des tableaux - seule la prop de page rendue change.
- **`Model::TOUCHES` a quitté une constante inhérente pour `EloquentModel`.**
  La cascade de touch du parent vit sur un défaut du trait `Model`, et un
  défaut de trait ne peut pas lire une constante inhérente. `Comment::TOUCHES`
  se résout toujours - il faut maintenant `use suprnova::EloquentModel;` dans
  la portée. Les modèles sans attribut `touches` prennent le défaut vide du
  trait.
- **`RelationEntry` a gagné `related_updated_at_column`.** Tout ce qui
  construit un `RelationEntry` à la main a besoin du champ supplémentaire ;
  rien dans l'arbre ne le fait, la macro les émet tous.
- **`Router::view` rejette désormais les props qui ne sont pas un objet JSON.**
  Il les ignorait auparavant en silence, en enregistrant une route qui rendait
  un sac de props vide sans aucun diagnostic. `null` reste accepté comme
  « aucune prop » ; `Router::try_inertia` est la forme faillible.
- **La version d'actifs Inertia vaut désormais par défaut un hachage du
  manifeste de build Vite** au lieu du littéral `"1.0"`, si bien qu'un
  déploiement invalide les clients de longue durée sans que personne n'ait à
  penser à incrémenter une chaîne. `InertiaConfig::manifest_path(...)` repointe
  le résolveur avec lui ; un `.version(...)` / `.version_with(...)` explicite
  l'emporte toujours. Sans manifeste sur disque - en développement local - la
  version retombe sur `"1.0"`, ce que toute application voyait auparavant :
  rien ne change donc tant que vous ne construisez pas. Le nouveau
  `VersionResolver::from_manifest(path)` expose le résolveur directement.

### Obsolète

- **`Cookie::read_encrypted` est désormais le lecteur hérité v1 uniquement.**
  Le code qui frappe avec `Cookie::encrypted` et lit avec `read_encrypted`
  échoue à l'exécution sur la première valeur écrite après cette release ;
  passez à `read_encrypted_for(name, wire)`. Les points d'entrée
  `CryptPurpose::Cookie` sans contexte sont également remplacés. Les deux
  suppressions sont prévues pour la 1.4.0.

### Mise à niveau
- **Les avertissements de déchiffrement de cookie ont désormais deux axes
  indépendants.** Un avertissement `KeyOrigin::Previous(index)` signifie qu'il
  faut rechiffrer la valeur sous l'`APP_KEY` courante et ne retirer cette clé
  précédente qu'une fois la traîne de rotation épuisée ; un avertissement
  `AadVersion::Legacy` signifie qu'il faut réémettre le cookie via l'API liée
  au nom avant la suppression du repli en 1.4.0. Une valeur peut signaler les
  deux.
- **`SESSION_COOKIE_PREFIX` est en opt-in.** Ne déployez `__Host-` qu'avec
  HTTPS, `SESSION_SECURE=true`, `SESSION_PATH=/` et aucun `SESSION_DOMAIN` ;
  les scaffolds HTTP locaux le laissent vide. Le `with_session_config` de
  `CsrfMiddleware` conserve le nom littéral `XSRF-TOKEN` ; utilisez
  `.xsrf_cookie_name("__Host-XSRF-TOKEN")` quand un client est configuré pour
  ce nom distinct.
- **`DecryptOrigin` est désormais une struct `#[non_exhaustive]` à deux axes.**
  Lisez ses champs `key` et `aad` indépendamment et gardez une stratégie de
  `match` compatible avec un joker pour les énumérations `KeyOrigin` /
  `AadVersion`.
- **`SessionConfig` et `CookieOptions` sont désormais `#[non_exhaustive]`.**
  Les littéraux de struct et les mises à jour fonctionnelles d'enregistrement
  dans le code applicatif doivent passer à `Type::default()` suivi
  d'affectations de champs publics ou de méthodes de constructeur.

- **`FrameworkError` est désormais `#[non_exhaustive]`.** Un `match` dessus
  dans votre propre code a besoin d'un bras joker. C'est la dernière release où
  l'ajout d'une variante aurait été un changement cassant.
- **Le champ `match_on` de `MergeStrategy::Append`/`Prepend`/`Deep` est
  désormais `Option<Vec<String>>`, et non `Option<String>`.** Un site d'appel
  qui construit directement la forme littérale de struct -
  `MergeStrategy::Append { match_on: Some("id".into()) }` - ne compile plus ;
  enveloppez le nom de champ dans un `Vec` : `Some(vec!["id".into()])`.
  `match_on: None` n'est pas affecté et n'a besoin d'aucun changement.
- **Un rechargement partiel apparié n'émet plus `deferredProps`.** Le code qui
  lit `page.deferredProps` sur une réponse de rechargement partiel - un
  composant de chargement deferred personnalisé, un instantané de test, une
  assertion de bout en bout - trouvera désormais la clé absente là où elle
  listait auparavant les props deferred que la requête n'avait pas nommées.
  Lisez les annonces sur la visite initiale (non partielle), là où Laravel les
  place et là où le client officiel les lit.
- **Une entrée `except` nue supprime désormais les clés de props pointées en
  dessous d'elle.** `X-Inertia-Partial-Except: auth` laissait auparavant dans
  la réponse une prop enregistrée sous `auth.user`, parce que la porte
  comparait des clés entières. Elle est supprimée maintenant. Si une page
  comptait sur une entrée `except` nue pour n'élaguer que la clé exacte, nommez
  la clé exacte (`except: ['auth.user']`) ou restreignez plutôt avec un chemin
  pointé.
- **`errors` ignore `only`/`except`.** Un rechargement partiel qui filtrait une
  prop `.with("errors", …)` fournie par le handler, ou qui la restreignait avec
  une entrée pointée, l'envoie désormais entière. Les tests qui affirment un
  objet `errors` tranché ou vide sur un rechargement partiel ont besoin d'une
  mise à jour. Pour garder délibérément le sac hors d'une réponse, marquez-le -
  `.prop("errors", Prop::eager(…).optional())` - plutôt que de compter sur les
  listes de rechargement partiel.
- **`Prop::resolve_with_owner` filtre aussi les props marquées.** Il résolvait
  auparavant toute prop qui n'était pas `Prop::is_lazy()` - une valeur eager
  *ou* un résolveur portant un flag - sans consulter l'ensemble d'include. Il
  filtre désormais chaque prop adossée à un résolveur et ne laisse passer sans
  filtre qu'une valeur déjà matérialisée. Un champ `#[data(lazy(deferred))]` a
  par conséquent besoin de `?include=<field>` sur la requête avant d'être
  résolu ou annoncé, comme toute autre saveur de lazy. Ajoutez le champ à la
  liste `?include=` de la requête, ou retirez l'attribut `lazy(...)` s'il
  n'était pas censé être en opt-in.
- **Le `reset` d'une prop de scroll ne suit plus l'en-tête d'intention de
  fusion.** Le code qui lit `page.scrollProps[key].reset` directement - un
  composant de défilement infini personnalisé, un instantané de test - verra
  `reset: false` (plus une entrée `mergeProps`) sur une simple revisite qui
  lisait auparavant `reset: true` et ne portait aucune métadonnée de fusion. Le
  composant officiel `<InfiniteScroll>` ne se comporte différemment que sur une
  simple revisite : il écoute `reset` sur chaque événement `success` du
  `router`, et pas seulement sur un `router.reload()` explicite, si bien qu'une
  revisite normale n'efface plus son état accumulé à moins que le serveur n'ait
  réellement nommé la clé dans `X-Inertia-Reset`, ce qui correspond à Laravel.
  Envoyez explicitement `X-Inertia-Reset: <key>` partout où l'ancien
  comportement « toute visite sans ajout en fin ni en tête réinitialise » était
  utilisé.
- **`Prop::match_on` prend `impl MatchOnFields`, et non `impl Into<String>`.**
  La nouvelle borne est ce qui permet à un appel de nommer plusieurs champs
  (`match_on(["id", "slug"])`), et sa liste d'implémentations est
  délibérément fermée : `&str`, `String`, `[T; N]` et `Vec<T>` seulement. Une
  implémentation générale sur `IntoIterator` n'est pas disponible : la
  cohérence la rejette face aux implémentations pour `&str` et `String`,
  puisque rien n'empêche ces types de gagner plus tard une implémentation
  d'`IntoIterator`. Trois types d'arguments qui compilaient avant ne compilent
  plus : `&String`, `Cow<'_, str>` et `Box<str>`. Passez plutôt un `&str` au
  site d'appel - `match_on(name.as_str())` pour un `&String`,
  `match_on(name.as_ref())` pour un `Cow<'_, str>`, `match_on(&*name)` pour un
  `Box<str>`.
- **Une entrée `only`/`except` pointée restreint désormais sa prop de premier
  niveau au lieu de l'exclure entièrement.** Avant ce correctif,
  `X-Inertia-Partial-Data: user.name` faisait chercher à
  `should_include_eager` une entrée `"user"` en correspondance exacte, n'en
  trouvait aucune, et laissait tomber en silence la prop `user` entière - un
  client qui demandait un champ de `user` n'obtenait rien. Tout composant de
  page frontend qui se trouvait compter sur cette lacune (en traitant un
  `router.reload({ only: [...] })` pointé comme équivalent à l'omission de la
  clé) reçoit désormais `{ user: { name: ... } }` à la place. Aucun changement
  de code n'est requis - c'est déjà ce que le protocole Inertia v3 spécifie
  comme sens du contrat requête/réponse. Le même correctif s'applique à
  `should_include_optional`, et son effet est opérationnellement plus grand :
  une entrée `only` pointée (`permissions.read`) compte désormais comme une
  demande explicite de la clé de premier niveau d'une prop `Optional` ou
  `Defer`, ce qui exigeait auparavant une entrée nue (`permissions`) pour se
  déclencher du tout. Une requête qui sautait entièrement le résolveur de cette
  prop l'exécute désormais - si le résolveur touche une base de données ou un
  service externe, un client qui envoie déjà des requêtes de rechargement
  partiel pointées commence à émettre ce travail sur des requêtes qui n'en
  faisaient aucun auparavant. Surveillez le volume d'appels aux résolveurs
  après la montée de version si votre application a des props
  `Optional`/`Defer` avec du trafic de rechargement partiel pointé.
- **`InertiaSharedData::share` prend désormais le nom du composant de page.**
  Ajoutez un paramètre `component: &str` après `req` :
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```
  Ignorez-le (`_component`) si votre fournisseur n'a pas besoin de varier selon
  la page - le `RenderContext` de Laravel porte le même appariement
  (`component`, `request`) pour
  `ProvidesInertiaProperties::toInertiaProperties`.
- **`Prop` est une struct, et non une énumération.** Ses variantes ont disparu ;
  construisez et lisez les props par des méthodes :
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (`Prop::lazy(closure)` est
    inchangé)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()` ;
    `matches!(prop, Prop::EagerNone)` -> `prop.is_absent()`
  Les structs de payload `DeferConfig`, `MergeConfig`, `OnceConfig` et
  `ScrollConfig` sont retirées - leurs champs sont désormais des flags sur
  `Prop`. `Prop::is_deferred()` est renommé `Prop::has_resolver()`, ce qu'il a
  toujours voulu dire. `DeferOptions`, `OnceOptions`, `MergeStrategy`,
  `ScrollMetadata` et chaque méthode du constructeur `InertiaResponse` sont
  inchangés, si bien qu'une application qui n'utilise que le constructeur de
  réponse n'a besoin d'aucune modification. Les applications qui construisent
  des props à la main - typiquement une implémentation d'`InertiaSharedData` -
  ont besoin des renommages ci-dessus.

- **Ce correctif protège les sessions que vous avez déjà, et pas seulement les
  requêtes à venir.** La montée de version suffit : un cookie de session écrit
  par une release antérieure peut porter une `_previous.url` qui n'a jamais été
  assainie, et `SessionData::previous_url()` la jette désormais à la lecture,
  la première fois que cette session est utilisée après la montée de version,
  au lieu de lui faire confiance parce qu'elle est déjà stockée. Vous n'avez
  pas besoin d'invalider les sessions existantes, de migrer la table des
  sessions ni de forcer une reconnexion. Une requête dont le chemin a l'air
  relatif au protocole (`//host`) ne met pas non plus à jour l'URL précédente
  enregistrée à l'avenir - si la route `fallback!` de votre application (ou
  toute route répondant 200 atteignable par un chemin inhabituel) comptait
  légitimement sur un tel chemin pour devenir la cible de `Redirect::back()`,
  ce ne sera plus le cas. Dans un cas comme dans l'autre, la valeur précédente
  et sûre de la session est plutôt laissée en place (ou le repli propre à
  `Redirect::back(fallback)` l'emporte, si rien de sûr n'a jamais été
  enregistré). Aucun changement de code n'est nécessaire, sauf si vous
  dépendiez du cas limite exact que ceci ferme - qui était déjà un risque de
  redirection ouverte.
- **Retirez le `[0]` de chaque liaison `errors.<field>` dans vos pages.** Avec
  la nouvelle forme par défaut, `errors.email` est une chaîne, si bien que
  `errors.email[0]` rend son premier caractère au lieu du message. Changez en
  même temps le type TypeScript de `string[]` à `string`. Si vous préférez ne
  pas toucher à vos pages, positionnez `InertiaConfig::with_all_errors(true)`
  sur la configuration que vous passez à `Inertia::install` et ajoutez
  l'augmentation de module `errorValueType: string[]` pour `@inertiajs/core`.
  Les frontends de démarrage livrent la nouvelle forme.
- **Un handler qui codait à la main la redirection en arrière après un échec de
  validation peut la supprimer.** Le pont est automatique désormais ; un
  handler qui redirige encore lui-même continue de fonctionner, parce que le
  middleware n'agit que sur un `422` qui porte un objet `errors` peuplé.
- **Un enfant de `suprnova serve` qui plante est désormais relancé au lieu de
  mettre fin à la session.** Si vous comptiez sur un plantage pour arrêter
  `suprnova serve` net (une vérification de fumée en CI, un script qui traite
  la sortie comme « quelque chose ne va pas »), passez `--no-restart` pour
  restaurer ce comportement à l'identique. Les réessais sont aussi bornés par
  défaut : un processus qui plante 5 fois de suite cesse d'être réessayé
  (relevez la limite avec `--restart-tries`, ou utilisez `--no-restart` pour le
  comportement d'origine « un plantage et c'est fini »).
- **`Model::TOUCHES` n'est plus une constante inhérente.** Le code qui lisait
  `Comment::TOUCHES` directement a besoin de `use suprnova::EloquentModel;` (ou
  `suprnova::eloquent::EloquentModel`) dans la portée - la constante a déménagé
  là pour que la cascade de touch du parent, un défaut du trait `Model`, puisse
  la lire. Un `grep -rn TOUCHES` sur votre application trouve chaque site
  d'appel ; la plupart des applications n'en ont aucun, puisque la constante ne
  faisait auparavant rien à l'exécution.
- **`RelationEntry` a gagné un champ.** Seul le code qui construit un
  `RelationEntry` à la main a besoin d'un changement - ajoutez
  `related_updated_at_column` au littéral. Les enregistrements de relations
  générés par macro que le framework livre l'émettent déjà, si bien qu'une
  application ordinaire qui ne fait que déclarer des relations via
  `#[suprnova::model]` n'est pas affectée.
- **`Router::view` avec des props qui ne sont pas un objet panique désormais à
  l'amorçage.** Il enregistrait auparavant en silence avec un sac de props
  vide ; `view` délègue à `Router::inertia`, qui exige un objet (ou `null`) et
  panique sinon. Si un appel à `view` peut porter des props qui ne sont pas un
  objet, passez à `Router::try_inertia` et traitez l'`Err` - sinon rien ne
  change pour vous.
- **Le défaut de version par manifeste Inertia peut changer votre chaîne de
  version dès qu'un build existe.** Une application ou un test qui code en dur
  `X-Inertia-Version: 1.0` ne continue de fonctionner que jusqu'à ce qu'un
  manifeste Vite apparaisse sur disque ; dès que c'est le cas, la version
  devient le hachage du manifeste. S'il vous faut l'ancienne constante, lisez-la
  vous-même depuis `VersionResolver::from_manifest(path)` ou épinglez
  `.version(...)` explicitement. Attendez-vous à ce que le premier déploiement
  après la montée de version force un cycle de rechargement complet de page
  pour les clients déjà connectés - une seule fois, et c'est tout l'objet du
  changement. La valeur de repli sans manifeste est exportée comme
  `suprnova::MANIFEST_VERSION_FALLBACK`, si bien que vous n'avez plus jamais
  besoin de coder `"1.0"` en dur.
- **Sortez l'enregistrement d'`Inertia::install` et de `global_middleware!` de
  `bootstrap::register`.** Mettez-les dans une nouvelle fonction et passez-la à
  `.http_bootstrap(...)` à la place - la nouvelle forme du scaffold est une
  fonction synchrone `register_http_stack()` appelée comme
  `.http_bootstrap(|| async { bootstrap::register_http_stack() })`. Les
  applications qui sautent cette étape gardent le comportement d'aujourd'hui,
  échec d'amorçage du worker sur un manifeste frontend manquant compris.

## 1.2.4 - 2026-08-18

### Sécurité

- **Le secret de contournement du mode de maintenance est comparé en temps constant.** `MaintenanceMiddleware` comparait l'URL secrète avec une simple comparaison de chaînes, qui s'arrête au premier octet différent. Comme le secret est un identifiant au porteur transporté dans le chemin de la requête, cette différence de temps indiquait à un attaquant la longueur du préfixe qu'il avait deviné correctement. La comparaison s'exécute désormais sur toute la longueur en octets via `subtle::ConstantTimeEq`, et ne court-circuite que sur une différence de longueur - la même forme que la comparaison du cookie de contournement à côté d'elle.
- **`rules::Url` rejette désormais les URI de script.** La règle acceptait tout schéma que `url::Url` pouvait analyser, y compris `javascript:` et `vbscript:`, si bien qu'une URL validée pouvait quand même servir de puits d'exécution de script une fois rendue dans un `href`. Elle applique désormais la forme de la règle `url` de Laravel (`^(PROTOCOLS)://HOST` de `Illuminate\Support\Str::isUrl`) : le schéma doit figurer dans la liste blanche de Laravel, être suivi de `://`, **et** être suivi d'un hôte non vide - le groupe hôte de Laravel n'a pas de `?`, donc un hôte absent ou vide ne correspond jamais, même avec un schéma listé. La liste des schémas et l'exigence `://` + hôte sont celles de Laravel mot pour mot ; l'hôte lui-même est analysé par la crate `url` plutôt que par la regex de Laravel, si bien que quelques cas limites diffèrent encore - un port hors plage est rejeté ici et accepté là-bas, et les hôtes IDN se normalisent différemment. Le nouveau `Url::protocols(&[...])` reflète `url:http,https` de Laravel ; `HttpUrl` n'est désormais que du sucre et conserve son propre message. **Changement de comportement :** une URL avec un schéma non listé qui validait auparavant échoue désormais - nommez le schéma avec `Url::protocols(&["myapp"])` si vous vouliez l'accepter. Deux autres changements de comportement : `mailto:`, `data:` et `tel:` sont nommément sur la liste blanche de Laravel mais ne portent pas de composante d'autorité, donc ils échouent désormais ; et les chemins de la forme `file:///etc/passwd` - `scheme://` avec rien entre les deux derniers slashes - échouent désormais aussi, puisqu'une chaîne vide n'est pas non plus un hôte. Les deux découlent de la règle `://` + hôte de Laravel elle-même.
- **Les réponses Inertia annoncent désormais `Vary: X-Inertia` partout.** L'en-tête n'était défini que sur les réponses de l'objet de page lui-même. Les redirections, les 404, les 422 et les réponses statiques n'en portaient aucun, si bien qu'un cache partagé indexé uniquement par l'URL pouvait servir l'objet de page JSON à une navigation complète du navigateur, ou le shell HTML à un XHR Inertia. Le nouveau `InertiaHeadersMiddleware` - enregistré par `Inertia::install` comme le plus externe des trois - le fixe sur chaque réponse et transforme un `200` vide lors d'une visite Inertia en un `303` de retour au lieu d'une réponse que le client rejette comme non Inertia. `InertiaVersionMiddleware` re-flashe maintenant la session avant son `409`, si bien qu'une erreur flashée survit au `GET` de page complète suivant du client.
- **Trois correctifs sur les réponses Inertia.** `InertiaResponse::location_for(&req, url)` retourne `409` + `X-Inertia-Location` pour un XHR Inertia et un simple `302` + `Location` pour une navigation dure, si bien qu'un rebond OAuth ou SSO amorcé hors du SPA ne se termine plus en cul-de-sac sur un `409` sans corps. La variante `location(url)` existante conserve sa forme toujours-`409`. `App::clear_history()` flashe le flag d'effacement de l'historique dans la session, si bien qu'il survit à la redirection de déconnexion et atterrit sur la page qui est réellement rendue - la `.clear_history()` par réponse ne marquait que la redirection que le navigateur jette, laissant l'historique chiffré de la session précédente déchiffrable. Et une prop `once` n'est désormais ignorée que lors d'une visite Inertia complète : un `router.reload({ only: ['stats'] })` explicite la réévalue, au lieu de ne rien renvoyer.
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
  correctement les deux signaux et son socket TCP est désormais
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
  sauter, ce pourquoi la v0.5.10 et la v0.6.1-v0.6.3 sont tag-only et
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
