# Localisation

La localisation dans Suprnova est un seul module à quatre facettes : les
catalogues de messages côté serveur, les erreurs de validation qui
arrivent déjà traduites, les *mêmes* octets de catalogue remis au
navigateur, et le formatage des nombres, des dates et des listes sensible
à la locale. Le format des messages est
[Fluent](https://projectfluent.org) - le `.ftl` de Mozilla, celui que
Firefox utilise - et tout le sous-système est activé par défaut derrière
la feature `localization`.

Le tour le plus court possible. Écrivez un catalogue :

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

Utilisez-le depuis un handler :

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

Une requête avec `Accept-Language: es` obtient la chaîne espagnole, parce
que `LocaleMiddleware` a résolu la locale avant que votre handler ne
s'exécute. Rien d'autre ne change dans le handler - pas de paramètre de
locale à threader, pas de `&Translator` dans la signature.

## Pourquoi la localisation

Trois raisons pour lesquelles c'est une préoccupation du framework plutôt
qu'une crate que vous choisissez :

- **Les messages de validation sont les chaînes du framework, pas les
  vôtres.** « The email field is required. » est émis au fond de
  `Rule::passes`, loin de tout code que vous possédez. À moins que le
  framework ne porte une couture de traduction, une app espagnole livre
  des erreurs de validation en anglais - ou vous enveloppez chaque règle
  à la main. Les règles intégrées de Suprnova retournent des messages
  *à clé* ; vous les traduisez en déposant un fichier `.ftl`, sans jamais
  toucher aux règles.
- **Le navigateur a besoin des mêmes chaînes.** Une app Inertia rend la
  moitié de son texte en Rust et l'autre moitié en Svelte/React/Vue.
  Deux systèmes de traduction, c'est deux formats de fichier, deux
  workflows de revue, et deux occasions pour la même phrase de diverger.
  Suprnova sert exactement le catalogue que le serveur a résolu depuis
  `/_suprnova/lang/<locale>.ftl`, et les starter kits le parsent avec
  `@fluent/bundle` - un seul jeu de fichiers, une seule source de vérité.
- **Les pluriels et les formats sont des données CLDR, pas de la
  concaténation de chaînes.** L'anglais a deux catégories de pluriel, le
  russe et le polonais quatre, l'arabe six. Un nombre s'écrit
  `1,234.56` en `en-US` et `1.234,56` en `de-DE`. Fluent sélectionne
  selon les catégories de pluriel CLDR et ICU4X fait le formatage, si
  bien qu'aucun des deux n'est quelque chose que vous roulez vous-même à
  la main par locale.

Désactiver la feature (`--no-default-features`) est pris en charge : le
module de localisation ne compile pas, et la validation affiche ses
chaînes de repli anglaises intégrées. Rien d'autre ne change de forme.

## Disposition des fichiers

Les catalogues vivent sous `lang/`, un répertoire par locale :

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

Les règles :

- **Un nom de répertoire est une locale BCP-47** - `en`, `en-GB`,
  `pt-BR`, `zh-Hans`. Un répertoire dont le nom ne se parse pas est
  ignoré avec un `warn!` plutôt que de faire échouer l'amorçage.
- **Chaque `.ftl` d'un répertoire de locale fusionne dans un seul
  catalogue**, dans l'ordre alphabétique des noms de fichier. Découpez
  par fonctionnalité (`auth.ftl`, `billing.ftl`, `emails.ftl`) autant que
  vous voulez - les ids de message sont globaux au sein de la locale,
  donc `auth.ftl` et `billing.ftl` ne doivent pas définir le même id.
- **Le propre catalogue de validation anglais du framework se charge en
  premier**, dans le bundle de chaque locale. Vos fichiers se chargent
  par-dessus, et une définition plus tardive l'emporte. C'est tout le
  mécanisme de remplacement : définissez `validation-min` dans
  `lang/es/validation.ftl` et le bundle espagnol utilise le vôtre.
- **La racine est `lang_path()`** - `<APP_BASE_PATH>/lang`. Définissez
  `APP_BASE_PATH` quand le binaire tourne depuis un endroit autre que la
  racine du projet (une unité systemd, un container avec un
  `WorkingDirectory` différent), ou appelez `use_lang_path("…")` pour ne
  déplacer que le répertoire `lang`. Voir [Variables
  d'environnement](env-vars.md).
- **Un répertoire `lang/` manquant n'est pas une erreur.** Une app
  fraîche doit démarrer, donc le traducteur démarre avec le seul
  catalogue anglais intégré. Un `.ftl` *malformé* est une autre
  histoire : les erreurs de parsing font échouer l'amorçage, en nommant
  le fichier et ce à quoi le parseur s'est opposé, parce qu'un catalogue
  silencieusement à moitié chargé est pire qu'un processus arrêté.
- **En `local` et `development`, les catalogues se rechargent à chaud.**
  Chaque requête fait un stat sur `lang/` et ne reparse que si quelque
  chose a réellement changé, si bien qu'éditer un `.ftl` apparaît au
  prochain rafraîchissement. La production ne refait jamais de stat ;
  les catalogues sont lus une seule fois au démarrage.

## FTL en cinq minutes

Fluent est un petit format. Cette section contient tout ce dont vous avez
besoin pour une app typique.

Les **messages** sont des paires `id = valeur`. Les ids sont en
kebab-case par convention (ceux du framework le sont), les valeurs
courent jusqu'à la fin de la ligne, et les lignes de continuation
indentées sont jointes :

```ftl
# Un commentaire. Rattaché au message ci-dessous.
sign-in = Se connecter
password-hint =
    Utilisez au moins 12 caractères. Une phrase de passe de
    quelques mots ordinaires bat une courte chaîne de symboles.
```

Les **arguments** sont des placeables `{ $name }`. Vous les fournissez
au moment de l'appel ; un argument manquant est une erreur, pas une
chaîne vide (`Lang::get` se replie alors sur sa chaîne - voir [La
façade `Lang`](#la-façade-lang)) :

```ftl
greeting = Bonjour, { $name } !
invoice-line = { $qty } × { $item }
```

Les **termes** commencent par `-`, sont privés au catalogue, et existent
pour qu'un nom de marque ou une phrase répétée vive à un seul endroit :

```ftl
-product-name = Suprnova
about = À propos de { -product-name }
footer = © 2026 { -product-name }. Tous droits réservés.
```

Les **sélecteurs** sont le conditionnel de Fluent. La valeur du
sélecteur est comparée à des clés de variante ; exactement une variante
est marquée par défaut avec `*` :

```ftl
cart-summary =
    { $count ->
        [0] Votre panier est vide.
        [one] Un article dans votre panier.
       *[other] { $count } articles dans votre panier.
    }
```

`[0]` correspond au nombre littéral zéro. `[one]` et `[other]` sont des
**catégories de pluriel CLDR**, résolues pour la locale du bundle -
c'est là que Fluent gagne sa place. L'anglais a deux catégories ; le
russe en a quatre, et un traducteur russe les écrit toutes les quatre
sans que vous ayez à changer une ligne de Rust :

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

CLDR affecte `1`, `21`, `31` à `one` ; `2`-`4`, `22`-`24` à `few` ; `0`,
`5`-`20`, `25`-`30` à `many` ; et les fractions à `other`. Le même appel
`__!("unread-messages", count: 22)` s'affiche correctement en anglais,
en russe, en polonais et en arabe, parce que la sélection de catégorie
est une donnée, pas du code.

**Mettez toujours le `*` sur `other`.** C'est la seule catégorie que
CLDR définit pour chaque locale, donc c'est la seule variante garantie
d'exister - et c'est vers le défaut que se replie une valeur de
sélecteur non appariée, y compris tout compte non entier. Marquer
`*[many]` (ou toute autre catégorie) comme défaut envoie les fractions
vers un texte écrit pour des nombres entiers.

> **Passez les comptes en tant que nombres.** `__!("unread-messages",
> count: 3)` envoie un nombre JSON et sélectionne une catégorie de
> pluriel. `count: "3"` envoie une chaîne, qui ne peut correspondre
> qu'à une clé de variante littérale - elle atterrira sur votre défaut
> `*[other]`. C'est le seul piège FTL qui mérite d'être mémorisé.

Les **fonctions** s'appellent à l'intérieur des placeables. Deux sont
enregistrées : `NUMBER()` (celle intégrée à Fluent) et `DATETIME()`
(celle de Suprnova) :

```ftl
score = Votre score est { NUMBER($points) } sur { NUMBER($total) }.
published = Publié le { DATETIME($when, dateStyle: "medium") }
```

Voir [Formatage sensible à la locale](#formatage-sensible-à-la-locale)
pour les deux.

**Une limitation délibérée :** Suprnova ne résout que des *valeurs* de
message à plat. La syntaxe d'attribut de Fluent (`login .placeholder =
…`) se parse mais n'est pas adressable via `Lang::get`, donc gardez un
id par chaîne : `login-placeholder`, pas `login.placeholder`. Les ids
forment un espace de noms à plat par locale - préfixez-les
(`auth-login-title`, `billing-invoice-due`) plutôt que de chercher une
hiérarchie que le résolveur n'a pas.

## La façade `Lang`

`Lang` est le point d'entrée côté serveur. Chaque méthode lit la
**locale courante**, celle que le middleware a liée pour cette requête.

| Méthode | Retourne | Notes |
|---|---|---|
| `Lang::get(key)` | `String` | Infaillible. Parcourt la chaîne de repli, puis retourne la clé elle-même |
| `Lang::get_with(key, args)` | `String` | Pareil, avec des arguments |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | Renvoie une erreur plutôt que de se dégrader |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | Pareil, avec des arguments |
| `Lang::has(key)` | `bool` | Si la clé se résout pour la locale courante, ou n'importe où le long de sa chaîne de repli |
| `Lang::locale()` | `Locale` | La locale courante |
| `Lang::set_locale(locale)` | `()` | La change pour le reste de cette requête |
| `Lang::available_locales()` | `Vec<Locale>` | Chaque locale avec un catalogue chargé |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // Seules certaines locales livrent ce texte de bannière.
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` est une map ordonnée de `String` vers
`serde_json::Value`, tous deux ré-exportés depuis la racine de la crate.
Les arguments Fluent sont des chaînes et des nombres ; les autres formes
JSON sont converties en chaîne.

### La chaîne de repli

`Lang::get` n'échoue jamais, et ne retourne jamais de chaîne vide. Dans
l'ordre :

1. Le catalogue de la **locale courante**.
2. Ses **parents de repli configurés** (voir [Chaînes de
   repli](#chaînes-de-repli)), parcourus transitivement, s'il y en a de
   configurés - `pt-PT` avant `pt-BR` avant ce que `pt-BR` lui-même
   nomme comme parent, et ainsi de suite.
3. Le catalogue de la **locale de repli** (`APP_FALLBACK_LOCALE`, `en`
   par défaut), sauf si elle est déjà apparue plus tôt dans cette
   chaîne.
4. La **clé elle-même**, plus un `tracing::warn!` par paire `(locale,
   clé)` manquante - une seule fois, pas une fois par requête, pour
   qu'une clé manquante dans un hot path ne noie pas vos logs.

L'étape 4 est la raison pour laquelle une traduction manquante affiche
`checkout-submit` dans le bouton plutôt qu'un bouton vide : une chaîne
visiblement fausse est un rapport de bug qui n'attend qu'à arriver,
alors qu'une chaîne vide est un mystère.

Quand vous préférez savoir plutôt que vous dégrader, utilisez les
homologues `try_*`. Ils exécutent les étapes 1 à 3 et retournent `Err`
plutôt que de faire l'étape 4 :

```rust
use suprnova::Lang;

// Une clé manquante ici signifie un e-mail cassé - faites échouer le
// job, n'envoyez pas un message avec une clé brute dans la ligne d'objet.
let subject = Lang::try_get("invoice-paid-subject")?;
```

### La macro `__!`

`__!` est le raccourci pour la mémoire musculaire venue de Laravel. Sans
arguments, elle appelle `Lang::get` ; avec des arguments nommés, elle
construit un `TranslateArgs` et appelle `Lang::get_with` :

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

Les valeurs d'argument sont tout ce qui se convertit en
`serde_json::Value` - `&str`, `String`, entiers, flottants, `bool`. La
macro est exportée à la racine de la crate, donc
`suprnova::__!("welcome-back")` fonctionne sans l'import quand vous
préférez ne pas amener `__` dans le scope.

## Chaînes de repli

`APP_FALLBACK_LOCALE` est un seul filet global sous chaque locale.
Parfois ça ne suffit pas : le portugais européen et le portugais
brésilien partagent presque tout et divergent sur une poignée de mots
(`ficheiro`/`arquivo`, `utilizador`/`usuário`, `tu`/`você`), et
maintenir deux catalogues complets signifie que chaque nouvelle chaîne
doit être écrite deux fois. Un **parent de repli** permet à `pt-PT`
d'hériter de `pt-BR` avant que `pt-BR` ne se replie plus loin sur le
`fallback_locale` global - donc `lang/pt-PT/` n'a besoin de contenir que
les chaînes qui sont réellement différentes.

### Configurer les parents

Une variable d'environnement, des paires `child=parent` séparées par
des virgules :

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Ou le builder, un appel par paire, chaînable :

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

Les deux chemins alimentent la même map (`LocalizationConfig::parents`),
et tous deux sont validés à l'amorçage, pas au moment de la requête :

- Une paire sans `=`, ou avec un enfant ou un parent vide, est une
  entrée `APP_LOCALE_PARENTS` malformée - l'amorçage échoue en nommant
  le segment fautif.
- Une locale invalide en BCP-47 d'un côté ou de l'autre de la paire
  échoue de la même façon.
- Nommer le même enfant deux fois est une config ambiguë, pas un
  « dernier gagne » - l'amorçage échoue en nommant l'enfant dupliqué.
- **Un cycle fait échouer l'amorçage.** L'erreur détaille le cycle :
  deux locales se nommant l'une l'autre (`pt-PT=pt-BR,pt-BR=pt-PT`)
  produit `` `pt-PT` -> `pt-BR` -> `pt-PT` ``. Une locale se nommant
  elle-même comme son propre parent (`pt-PT=pt-PT`) est le même cas en
  miniature - `` `pt-PT` -> `pt-PT` ``. (Deux chemins de code lèvent
  cette erreur : le parsing de `APP_LOCALE_PARENTS` - donc toute app
  dont la config passe par `LocalizationConfig::from_env()` échoue au
  chargement de la config - et le chargement de catalogue de
  `FluentTranslator`, qui attrape une map cyclique construite par
  programme avec `.parent(...)`. Seule une app qui construit sa config
  entièrement à la main *et* lie son propre `Translator` personnalisé
  dans `bootstrap_fn` évite les deux ; le parcours de `Lang` est gardé
  indépendamment et termine quand même en sécurité dans ce cas, il
  n'obtient simplement pas l'erreur bruyante au moment de l'amorçage.)

Le `.parent(child, parent)` du builder est « dernière écriture gagne »
pour un enfant répété - un appel plus tardif qui écrase un appel plus
ancien n'est qu'un remplacement plus tardif, pas le cas d'entrée
ambiguë contre lequel `APP_LOCALE_PARENTS` se protège.

### Ordre de résolution

Une chaîne peut faire plus d'un saut : `pt-PT` nomme `pt-BR` comme son
parent, et `pt-BR` peut à son tour nommer son propre parent.
`Lang::get` / `try_get` / `get_with` / `try_get_with` / `has`
parcourent tous l'ensemble, la locale courante en premier :

1. Le catalogue de la **locale courante**.
2. Son **parent configuré**, puis le parent configuré de *cette*
   locale, transitivement, jusqu'à atteindre une locale sans parent
   configuré.
3. Le **`fallback_locale`** global (`APP_FALLBACK_LOCALE`), sauf s'il
   est déjà apparu plus tôt dans la chaîne - y compris le cas courant
   où c'est simplement la locale courante elle-même (le défaut
   `en`/`en`).

`Lang::get` / `Lang::get_with` se replient sur la clé elle-même si rien
dans la chaîne ne la résout, exactement comme le décrit [La chaîne de
repli](#la-chaîne-de-repli) ; `Lang::try_get` / `Lang::try_get_with`
retournent `Err`, et `Lang::has` retourne `false`. Ce parcours
s'exécute à l'intérieur de la façade `Lang` elle-même, donc il
fonctionne pour **n'importe quel** `Translator` - le `FluentTranslator`
fourni, ou un driver que vous écrivez.

### Un exemple exécutable

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// Une requête résolue vers `pt-PT`.
assert_eq!(__!("file-label"), "Ficheiro");                    // le remplacement propre à pt-PT
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // hérité de pt-BR
);
```

`lang/pt-PT/` ne définit jamais `welcome` - elle n'en a pas besoin.
`file-label` est une vraie différence d'un mot entre les deux
catalogues, donc c'est le seul id qui obtient un fichier.

### Les catalogues servis sont aplatis

Le point de terminaison `/_suprnova/lang/pt-PT.ftl` (voir [Le point de
terminaison du catalogue](#le-point-de-terminaison-du-catalogue)) ne
demande jamais au navigateur de savoir que `pt-BR` existe.
`FluentTranslator` pré-fusionne toute la chaîne en une seule ressource
par locale au moment du chargement - le catalogue du framework intégré
tout en bas pour les locales `en`/`en-*`, puis la chaîne de parents
configurée, puis les propres fichiers de la locale - et sert *cela*,
déjà aplati. Récupérez `pt-PT.ftl` et la réponse porte à la fois
`welcome` et `file-label`, en une seule requête, sans aucune logique de
chaîne côté client. `?v=<hash>` nomme toujours une ressource immuable
unique ; le hash couvre simplement maintenant aussi les chaînes tirées
de `pt-BR`.

**L'aplatissement ne couvre que les parents configurés** - il n'atteint
jamais au-delà d'eux jusqu'au `fallback_locale`. Le catalogue servi de
`pt-PT` inclut les chaînes de `pt-BR` parce que `pt-BR` est un *parent
configuré* ; il n'inclut pas les chaînes de `en` simplement parce que
`en` se trouve être le repli global. Le champ `fallback` de
`LocaleShare` nomme toujours le `fallback_locale` terminal, non affecté
par tout ceci - il indique au frontend où le parcours au niveau de la
façade de `Lang` finirait par atterrir, pas ce qui se trouve déjà dans
le fichier qu'il vient de récupérer.

### Règles de fusion des fichiers delta

Un catalogue enfant fusionne par-dessus son parent **au niveau de l'AST
Fluent**, pas par concaténation textuelle et pas par occultation de
message entier. L'unité de remplacement est le *pattern*, donc :

- **La valeur d'un enfant remplace la valeur du parent**, à la position
  du parent dans le fichier.
- **Une entrée enfant avec des attributs mais sans valeur conserve la
  valeur du parent.** Retraduire `.placeholder` ne demande pas de
  répéter le texte propre du message.
- **Les attributs fusionnent par nom.** Un attribut enfant de même nom
  remplace celui du parent, en place ; un attribut propre à l'enfant
  s'ajoute après celui du parent. **Les attributs que l'enfant ne
  mentionne pas survivent depuis le parent** - remplacer la valeur d'un
  message ne fait jamais tomber silencieusement son `.placeholder` ou
  son `.aria-label`.
- **Les expressions de sélection se remplacent en bloc, jamais variante
  par variante.** Les variantes d'un sélecteur sont indexées sur les
  catégories de pluriel CLDR d'une locale ; comme ces catégories
  dépendent de la locale, épisser une variante du parent et une autre de
  l'enfant pourrait produire un sélecteur sans la grammaire d'aucune
  locale unique derrière lui. Un enfant qui remplace un sélecteur, quel
  qu'il soit, doit fournir toutes les variantes qu'il veut.
- **Les commentaires sur une entrée remplacée restent ceux du parent.**
  Le commentaire documente l'id, et l'unité de remplacement est le
  pattern, pas le commentaire.
- **Les entrées propres à l'enfant s'ajoutent à la fin**, dans l'ordre
  propre de l'enfant, commentaires compris - un id que `pt-BR` n'a
  jamais défini n'est le « remplacement » de rien.

Les termes (`-brand`) suivent la règle identique, avec une restriction :
la valeur d'un terme n'est jamais optionnelle en syntaxe Fluent, donc le
cas « attributs mais pas de valeur conserve la valeur du parent »
ci-dessus ne s'applique qu'aux messages - un terme enfant fournit
toujours une valeur, et cette valeur l'emporte toujours. La fusion des
attributs par nom, le remplacement du pattern entier pour la valeur, et
les commentaires qui restent ceux du parent s'appliquent tous aux
termes exactement comme aux messages. Les termes sont suivis dans leur
propre espace de noms - remplacer `-brand` ne peut jamais occulter un
message aussi nommé `brand`.

### Pourquoi Suprnova diverge

Laravel 13 n'a qu'un seul repli : la valeur de config globale unique
`fallback_locale`, consultée quand le tableau de la locale courante n'a
pas une clé. Il n'existe pas de notion d'une locale héritant d'une
locale sœur - `pt_PT.php` et `pt_BR.php` sont deux tableaux sans
rapport, et une app `pt_PT` soit duplique tout ce que `pt_BR` a déjà
traduit, soit s'en passe.

Les chaînes de parents de Suprnova sont l'extension côté Rust : une
étape intermédiaire entre « cette locale » et « le repli global »,
configurée par locale plutôt qu'une seule fois globalement. Le
compromis que nous n'avons pas voulu faire, c'est de pousser cette
complexité sur le navigateur - un frontend conscient de la chaîne
devrait récupérer `pt-PT.ftl`, découvrir qu'il est incomplet, récupérer
aussi `pt-BR.ftl`, et les fusionner côté client en JavaScript, avec des
règles qui devraient correspondre exactement à celles du serveur.
Aplatir au moment du chargement signifie à la place que le catalogue
servi est toujours un fichier complet et autonome - le même contrat que
le frontend avait déjà avant que les chaînes de parents n'existent,
donc `@fluent/bundle` et les wrappers de kit n'ont eu besoin d'aucun
changement pour prendre en charge cette fonctionnalité.

## Détection de la locale

`LocaleMiddleware` résout une locale par requête et la lie pour la
durée du handler. La chaîne est pilotée par la config et **le premier
coup gagne** :

1. **Session** - la clé `locale` dans la session, si le [middleware de
   session](session.md) a tourné et que la valeur nomme une locale
   disponible. C'est là que vit « l'utilisateur a choisi Español dans
   les paramètres ».
2. **Cookie** - le cookie `locale`. Survit à la déconnexion, donc un
   choix de langue fait avant de se connecter n'est pas perdu.
3. **`Accept-Language`** - négocié contre `available_locales()` avec
   `fluent-langneg`, en respectant les q-values. `fr-CH, es;q=0.8,
   en;q=0.5` contre les catalogues `en` + `es` se résout en `es`.
4. **`APP_LOCALE`** - le défaut configuré, quand rien au-dessus n'a
   fait mouche.

Un candidat qui ne se parse pas, ou qui nomme une locale sans
catalogue, est **ignoré, pas rejeté**. Un utilisateur avec un cookie
`locale=zz` périmé voit la langue par défaut, pas un 500. Un en-tête
`Accept-Language` invalide fait de même. Une entrée contrôlée par un
attaquant atteint cette chaîne à chaque requête ; elle ne doit jamais
pouvoir faire plus que choisir une langue.

Câblez-le dans `bootstrap.rs`, **après** le middleware de session,
puisque l'étape 1 lit la session :

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // Résout la locale et la lie pour la requête.
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // Remet au frontend sa locale + l'URL du catalogue sur chaque page Inertia.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` lit `LocalizationConfig::from_env()` ;
`LocaleMiddleware::new(config)` en prend une que vous avez construite
vous-même. Une app scaffoldée a déjà les deux lignes.

Enregistrez-le **avant** `Inertia::install` également, si l'application
nomme une [page d'erreur Inertia](frontend-inertia-responses.md#error-pages).
Cette page est rendue par un middleware au retour, une fois que tout ce
qui est enregistré à l'intérieur a rendu la main - une portée de locale
ouverte à l'intérieur de la couche Inertia a donc déjà disparu à ce
moment-là, et chaque page d'erreur s'afficherait dans la locale par
défaut. Session à l'extérieur, locale au milieu, Inertia à l'intérieur :
c'est l'ordre qu'utilise le scaffold.

### Changer de locale en cours de requête

`Lang::set_locale` est le `App::setLocale` de Laravel - il réécrit la
locale de la requête courante à partir de ce point :

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// L'utilisateur vient de changer de langue dans un formulaire de paramètres.
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // cette requête
    session_mut(|s| s.put("locale", choice));       // chaque requête suivante
    Ok(())
}
```

Notez les deux moitiés : `set_locale` affecte *cette* requête (donc le
message flash de la redirection est déjà en espagnol), et l'écriture en
session est ce que la chaîne de détection lit à la *prochaine*.

### En dehors d'une requête

Les commandes console, les workers de queue et les tâches planifiées
n'ont ni requête ni middleware. Là, `Lang::set_locale` écrit un
remplacement global au processus que `Lang::locale()` consulte avant de
se replier sur `APP_LOCALE` :

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // La préférence stockée de chaque utilisateur, pour la durée de son e-mail.
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

Comme ce remplacement est global au processus plutôt que local à la
tâche, définissez-le en haut de chaque unité de travail comme
ci-dessus - ne comptez pas sur le fait qu'il reste inchangé à travers
un `.await` avec lequel une autre tâche pourrait s'entrelacer.

## Configuration

Trois variables d'environnement. `APP_LOCALE` et `APP_FALLBACK_LOCALE`
ont toutes deux `en` par défaut ; `APP_LOCALE_PARENTS` est vide par
défaut - pas de remplacement par locale, seul `fallback_locale`
s'applique :

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Tout le reste est du code, sur `LocalizationConfig`. Elle s'enregistre
comme toute autre config typée - dans votre `config::register_all`, qui
s'exécute avant le démarrage :

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // voir la note de divergence
        .detection(vec![Detect::Session, Detect::Header])   // ignore le cookie
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // voir Chaînes de repli
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - remplace `APP_LOCALE` et
  `APP_FALLBACK_LOCALE` depuis le code. Une valeur malformée à l'un ou
  l'autre endroit fait échouer le démarrage plutôt que de silencieusement
  devenir `en`.
- `use_isolating` - marques d'isolation Unicode autour des
  interpolations. Désactivé par défaut ; activez-le quand vous livrez
  une locale RTL.
- `detection` - la chaîne, dans l'ordre. Retirer `Detect::Cookie`
  signifie qu'un choix de langue ne vit que dans la session ; retirer
  `Detect::Header` signifie que la préférence du navigateur est
  entièrement ignorée.
- `session_key` / `cookie_name` - renomme les deux lookups.
- `parents` - parents de repli par locale (`enfant -> parent`),
  parcourus avant `fallback_locale` quand une clé manque dans le
  catalogue de l'enfant ; même forme que `APP_LOCALE_PARENTS`.
  Ajoutez-en un avec `.parent(child, parent)` - chaînable, dernière
  écriture gagne pour un enfant répété. Voir [Chaînes de
  repli](#chaînes-de-repli) pour le contrat complet (validation au
  démarrage, ordre de résolution, aplatissement du catalogue servi).

Le démarrage lie un `Arc<dyn Translator>` dans le conteneur. Si votre
app en a déjà lié un, le framework le laisse tranquille - c'est comme
ça que vous substituez votre propre traducteur sans rien forker :

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` est la couture d'extension : `translate`, `has`,
`available_locales`, `catalog`, `reload`. Un seul driver est livré
(`FluentTranslator`), et un nouveau backend est un nouveau driver - pas
un fork de la surface.

## Messages de validation traduits

Chaque règle intégrée retourne un message **à clé** : une clé de
catalogue, les arguments dont le message a besoin, et un repli anglais.
La traduction se produit une seule fois, à la frontière de
sérialisation - `ValidationErrors::to_json` et le sac d'erreurs
Inertia - jamais à l'intérieur de la règle. Les règles restent pures,
et tout le sous-système se compile en dehors.

Les clés suivent une convention :

| Forme | Exemple | Utilisé pour |
|---|---|---|
| `validation-<rule>` | `validation-min`, `validation-required-if` | Une par règle intégrée, en kebab-case |
| `field-<name>` | `field-email` | Un nom humain pour un champ |
| `validation-invalid-data` | - | La bannière de haut niveau « The given data was invalid. » |

Pour les traduire, définissez les ids qui vous intéressent dans
n'importe quel fichier `.ftl` sous la locale cible :

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` est toujours disponible. Les propres paramètres de chaque
règle sont passés sous les noms qu'ils portent dans le catalogue
anglais du framework - `$min`, `$max`, `$other`, `$value` - et
`framework/src/localization/catalogs/en/validation.ftl` est la liste
canonique des ids et arguments. Copiez-en les ids dont vous avez
besoin ; vous n'avez jamais à tous les remplacer.

Le remplacement fonctionne par locale et par clé. Définir
`validation-min` dans `lang/en/validation.ftl` remplace la formulation
anglaise du framework pour cette seule règle et laisse le reste
tranquille.

### Noms de champ

Interpoler un nom de colonne brut produit « The email_address field is
required. » La convention `field-<name>` corrige ça :

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

Avant le rendu, le traducteur cherche `field-<name>` pour la locale
courante. Une correspondance est passée comme `$field` ; une absence se
replie sur le nom du champ avec les underscores transformés en espaces.
Donc le fichier ci-dessus n'est nécessaire que pour les noms qui
s'humanisent mal.

### Règles personnalisées

`Rule::passes` retourne `Result<(), ValidationMessage>`. Un message à
clé participe à la traduction :

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

Une simple chaîne fonctionne toujours, et c'est la bonne réponse pour un
message qui n'existera jamais que dans une seule langue :

```rust
Err("must start with acct_".into())   // sans clé : rendu tel quel
```

Les messages sans clé sautent entièrement la traduction, ce qui est ce
qui permet aux règles personnalisées existantes de continuer à
compiler et à se comporter exactement comme avant.

### Le flux du derive

Les erreurs de `#[derive(Validate)]` sont aussi à clé. Le code d'erreur
de la crate `validator` devient `validation-<code>` avec les
underscores transformés en tirets, et chaque paramètre que le
validateur attache devient un argument de message - à deux exceptions
réservées près, `value` et `other`, qui sont toujours écartés. Les deux
portent la *valeur* réelle d'un champ plutôt que des métadonnées sur la
règle : `value` est l'entrée testée renvoyée en écho, et `other` (défini
par `must_match`, la règle canonique de confirmation de mot de passe)
est la valeur du champ jumeau. Aucun des deux n'est jamais remis au
catalogue, donc aucun remplacement `.ftl` - quelle que soit la façon
dont il formule `validation-must-match` - ne peut interpoler un secret
soumis dans un corps de réponse 422. Donc un échec `#[validate(email)]`
résout `validation-email` comme le fait la règle écrite à la main, et
une locale qui en traduit une traduit les deux.

## Le frontend

Le navigateur reçoit les mêmes octets que ceux résolus par le serveur.
Rien n'est retraduit, ré-exporté, ou maintenu synchronisé à la main.

### Le point de terminaison du catalogue

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 quand If-None-Match correspond
GET /_suprnova/lang/zz.ftl              → 404 (catalogue inexistant)
```

Le corps est le catalogue fusionné pour cette locale - les messages du
framework d'abord, puis sa chaîne de parents de repli configurée s'il y
en a une (voir [Chaînes de repli](#chaînes-de-repli)), puis vos
fichiers dans l'ordre de chargement. `ETag` est le hash du contenu.
Demandez un hash précis avec `?v=` et la réponse est cacheable de façon
immuable pour toujours, parce que cette URL ne peut jamais vouloir dire
qu'une seule chose ; demandez sans et vous obtenez une revalidation à
la place. Comme `/_suprnova/health`, le chemin est exempté de la chaîne
de middleware : il doit répondre avant qu'une locale n'ait été résolue,
et il ne porte aucune donnée utilisateur.

### La prop partagée

`LocaleShare` est un `InertiaSharedData` que le framework livre.
Enregistrée dans `bootstrap.rs` (voir [Détection de la
locale](#détection-de-la-locale)), elle ajoute une prop à chaque page
Inertia :

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

`catalog` vaut `null` quand aucun traducteur n'est lié - le partage ne
fait jamais échouer le rendu d'une page.

### Les wrappers de kit

Chaque starter kit livre un wrapper d'environ 100 lignes qui lit cette
prop, récupère le catalogue une fois, construit un bundle
`@fluent/bundle`, et expose `t()`. Appelez `initLang` une fois dans
votre point d'entrée Inertia (les apps scaffoldées le font déjà) :

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* … inchangé … */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

Ensuite, dans les composants :

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

Le formatage des nombres et des dates côté client utilise l'`Intl`
intégré du navigateur - aucune donnée ICU n'est livrée au navigateur.

### Clés de message typées

`suprnova generate-types` parse `lang/<default locale>/*.ftl` et émet
une union de chaque id de message aux côtés des types de props de
page :

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

Les wrappers typent `t(key: MessageKey, …)`, donc c'est la même
promesse que [`inertia-props.ts`](frontend-typescript-types.md) :
renommez un message en Rust, régénérez, et le compilateur TypeScript
pointe vers chaque site d'appel qui utilise encore l'ancien id.
`suprnova serve` surveille `lang/` aux côtés de `src/`, donc le fichier
se régénère à mesure que vous éditez les catalogues.

Un projet sans répertoire `lang/` et sans id de message n'obtient
**aucun fichier** - une app qui n'est pas localisée ne voit apparaître
aucun nouvel artefact.

## Formatage sensible à la locale

Sept fonctions sur `Lang`, toutes adossées à ICU4X, toutes lisant la
locale courante, toutes avec des homologues `try_*` qui retournent
`Result<String, FrameworkError>` plutôt que de se dégrader :

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

Les enums de style : `DateStyle { Full, Long, Medium, Short }`,
`TimeStyle { Medium, Short }`, `ListStyle { And, Or, Unit }`,
`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`.
`Lang::relative` prend un montant signé - négatif pour le passé
(« 3 days ago »), positif pour le futur (« in 3 days »).

> La sortie exacte provient des données CLDR intégrées à ICU4X et peut
> changer lors d'une mise à niveau d'ICU, en particulier pour les dates
> et les devises. Dans vos propres tests, affirmez sur la forme et la
> distinction entre locales (`de != en`, contient `2026`) plutôt que
> sur les octets exacts.

### Formater à l'intérieur d'un message

Deux fonctions sont appelables depuis FTL :

```ftl
order-total = Votre total est { NUMBER($amount, maximumFractionDigits: 2) }.
published = Publié le { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` est la fonction intégrée de Fluent, enregistrée
explicitement, et vous donne le contrôle des chiffres de fraction à
l'intérieur du message. `DATETIME()` est celle de Suprnova : `$value`
accepte une chaîne ISO-8601 ou des millisecondes epoch, et `dateStyle`
/ `timeStyle` prennent les mêmes noms que les enums Rust, en
minuscules. Une valeur qu'elle ne peut pas parser passe telle quelle
avec un `warn!` - une fonction Fluent ne peut pas retourner d'erreur,
et une page rendue avec une date à l'air bizarre vaut mieux qu'un 500.

Quand vous voulez le formatage complet d'ICU4X plutôt que ce qu'expose
une fonction Fluent, formatez en Rust et passez la chaîne finie :

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## Tester vos traductions

Deux aides font le travail : `use_lang_path` pointe le loader vers un
répertoire de fixtures, et `scope_locale` épingle la locale courante
pour la durée d'un future.

La forme hermétique - construire un traducteur sur un répertoire de
fixtures et le lier dans un conteneur cantonné au test - est ce
qu'utilisent les propres tests du framework, parce qu'elle ne touche
aucun état global au processus et survit à l'exécution parallèle des
tests :

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

`use_lang_path` est le bon outil quand le test démarre la vraie
application et que vous voulez que *toute* l'app pointe vers des
fixtures :

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // … démarrez l'app ; `lang_path("")` résout désormais vers le répertoire de fixtures.
}
```

Elle écrit un remplacement de chemin global au processus, donc
traitez-la comme un réglage par binaire plutôt que comme quelque chose
sur lequel deux tests parallèles peuvent être en désaccord.

La détection elle-même - la chaîne session/cookie/`Accept-Language` -
mérite d'être testée à travers le vrai pipeline plutôt qu'en appelant
le middleware directement, parce que les cas intéressants concernent le
parsing d'en-tête et la question de quelle source gagne. Montez une
route dont le handler retourne `__!("welcome")`, enregistrez
`LocaleMiddleware` dans le `MiddlewareRegistry`, et pilotez-la avec le
harnais loopback de [HTTP Tests](http-tests.md), en envoyant
`Accept-Language: fr, es;q=0.8` et en affirmant sur le corps espagnol.
Les cas qui méritent d'être épinglés : un en-tête négocie, un cookie
bat un en-tête, une locale indisponible est ignorée plutôt que de
générer une erreur, et un en-tête malformé retourne quand même 200.

Voir [Tests](testing.md) pour `TestContainer::scope` quand votre test
tourne sur un runtime multi-thread - la garde `fake()` locale au thread
ci-dessus ne survit pas à un future qui migre entre workers.

### Pourquoi Suprnova diverge

**Des fichiers FTL, pas des tableaux PHP.** Laravel a deux formats -
des tableaux imbriqués dans `lang/en/messages.php`, plus du JSON à plat
dans `lang/en.json` pour les traductions à clé de chaîne - et aucun des
deux n'est chargeable par un navigateur, ni n'exprime la sélection de
pluriel dans le fichier : ça vit dans la convention de pipes et de
plages de `trans_choice`, à l'intérieur de la chaîne. Fluent nous donne
un seul format que le serveur et le client parsent tous les deux, ce
qui est ce qui fait de « le frontend affiche la même chaîne que celle
produite par le validateur » une propriété de la conception plutôt
qu'une convention que vous maintenez. Ça vous coûte une nouvelle
syntaxe à apprendre (ce chapitre en est l'essentiel) et un changement
d'outillage : Poedit ne peut pas éditer `.ftl`, alors que Crowdin,
Weblate, Lokalise et Pontoon le peuvent. Ça coûte aussi l'espace de
noms à points - `trans('messages.welcome')` n'a pas d'équivalent, parce
que les ids forment un espace de noms à plat par locale. Préfixez à la
place.

**Pas de `trans_choice`.** Laravel sélectionne une forme de pluriel
avec des chaînes séparées par des pipes et des plages explicites :

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

Maintenant comptez jusqu'à 22 en polonais. CLDR place 22 dans la
catégorie `few` - `22 pliki` - mais `[5,*]` l'avale et produit `22
plików`. La même cassure se produit à 32, 42, 102, et en russe, en
arabe, en tchèque, en lituanien et en gallois, chacun à ses propres
endroits. Les plages d'entiers ne peuvent pas exprimer les règles de
pluriel, parce que les règles de pluriel ne parlent pas de plages ;
elles parlent du dernier chiffre, des deux derniers chiffres, et dans
certaines langues, de si la valeur est un entier du tout. Fluent
sélectionne directement sur la catégorie CLDR, donc `$count` est un
argument ordinaire et c'est le *traducteur* - la personne qui connaît
la langue - qui écrit les quatre catégories du polonais :

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` c'est 1 ; `few` c'est 2-4, 22-24, 32-34, 102-104 ; `many` c'est 0,
5-21, 25-31 ; `other` attrape les fractions (`1,5 pliku`) et porte le
marqueur par défaut, selon la règle ci-dessus.

La forme sans plage de Laravel (`plik|pliki|plików`) fait mieux - elle
consulte un index par langue et choisit le *n*-ième segment - mais cet
index est une table maintenue à la main plutôt que des données CLDR,
elle offre au polonais trois segments là où CLDR définit quatre
catégories, les segments sont positionnels sans nom de catégorie à
examiner, et elle ne peut jamais sélectionner que sur le compte.

Ce qui est le second bénéfice, qui tombe gratuitement : un sélecteur
Fluent peut basculer sur *n'importe quel* argument, pas seulement un
compte. Le genre, le palier de plan, et l'état de connexion se
sélectionnent de la même façon, et aucun d'eux n'a eu besoin d'une
nouvelle méthode de façade.

**Les marques d'isolation sont désactivées par défaut.** Fluent
enveloppe normalement chaque interpolation dans U+2068 (FIRST STRONG
ISOLATE) et U+2069 (POP DIRECTIONAL ISOLATE), pour qu'une valeur de
droite à gauche intégrée dans une phrase de gauche à droite se rende
dans le bon ordre. Correct - et invisible, ce qui signifie que chaque
`assert_eq!("Hello Ada", …)` dans une app anglais-seulement échoue avec
deux caractères que personne ne peut voir dans le diff. Nous les
désactivons par défaut et rendons leur activation un seul appel :

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**Activez-les quand vous livrez une locale RTL** - arabe, hébreu,
persan, ourdou - ou toute locale où des valeurs fournies par
l'utilisateur mélangent des écritures à l'intérieur d'une phrase.
Mettez ensuite à jour vos assertions pour comparer contre des chaînes
qui portent les marques, ou retirez-les dans l'aide d'assertion. Le
défaut optimise pour le cas courant ; le cas correct est à une ligne de
distance et ce paragraphe est le rappel de la prendre.

## Suivant

- [Validation](validation.md) - les règles, la macro `validate!`, et
  d'où vient `ValidationMessage`
- [Types TS](frontend-typescript-types.md) - `generate-types`,
  `inertia-props.ts`, et `lang-keys.ts`
- [Middleware](middleware.md) - ordonner `LocaleMiddleware` par rapport
  au reste de la chaîne globale
- [Session](session.md) - le magasin que la première étape de
  détection lit
- [Variables d'environnement](env-vars.md) - `APP_LOCALE`,
  `APP_FALLBACK_LOCALE`, `APP_LOCALE_PARENTS`, `APP_BASE_PATH`
- [Tests](testing.md) - `TestContainer`, `#[suprnova_test]`, et les
  remplacements de DI hermétiques
