# suprnova serve

`suprnova serve` exécute votre backend et le serveur de dev Vite
ensemble, avec rechargement à chaud des deux côtés, plus régénération
automatique des types TypeScript chaque fois que vous touchez une
struct `#[derive(InertiaProps)]`. C'est la seule commande que vous
gardez ouverte dans un terminal pendant que vous construisez.

```bash
suprnova serve
```

Les deux processus déversent leur stdout dans le même terminal avec
des préfixes colorés `[backend]` et `[frontend]` pour que vous
puissiez distinguer qui dit quoi. `Ctrl+C` les arrête tous les deux
proprement.

## Utilisation

```bash
suprnova serve [OPTIONS]
```

| Option | Par défaut | Description |
|---|---|---|
| `-p, --port <PORT>` | `8765` (CLI) / `$SERVER_PORT` (env) | Port HTTP du backend |
| `--frontend-port <PORT>` | `5765` (CLI) / `$VITE_PORT` (env) | Port du serveur de dev Vite |
| `--backend-only` | `false` | Ignore le serveur de dev Vite |
| `--frontend-only` | `false` | Ignore le backend, exécute seulement Vite |
| `--skip-types` | `false` | Ne régénère pas les types TypeScript sur les changements Rust |
| `--no-restart` | `false` | Ne relance pas un processus de dev en échec - ferme toute la session à la place (l'ancien comportement) |
| `--restart-tries <N>` | `5` | Abandonne les relances d'un processus après ce nombre de crashs consécutifs. Ignoré avec `--no-restart`, qui termine déjà la session au premier crash. |
| `--timestamps` | `false` | Préfixe chaque ligne de sortie avec une heure `HH:MM:SS` |
| `--json` | `false` | Émet sur stdout un objet JSON par ligne (NDJSON) au lieu du texte préfixé - voir [Sortie JSON](#sortie-json). Le combiner avec `--timestamps` n'est pas une erreur ; `--timestamps` n'a alors aucun effet supplémentaire, puisque chaque événement porte déjà son propre horodatage. |
Les flags CLI priment sur les variables d'environnement, qui priment
sur les valeurs par défaut intégrées. Un `.env` scaffoldé est livré
avec `SERVER_PORT=8765` et `VITE_PORT=5765` ; vous verrez ces valeurs
utilisées sauf si vous les redéfinissez avec `--port`.

## Exemples

### Par défaut - les deux serveurs

```bash
suprnova serve
```

Sortie :

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

Ouvrez `http://127.0.0.1:8765` dans votre navigateur. Le backend sert
la coque HTML d'Inertia et fait suivre les requêtes d'assets vers
Vite, si bien que vous n'avez pas besoin de visiter directement l'URL
de Vite.

### Ports personnalisés

```bash
suprnova serve --port 3000 --frontend-port 3001
```

Ou définissez-les dans `.env` et exécutez sans flags :

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### Backend seul

```bash
suprnova serve --backend-only
```

Utile pour travailler sur un projet API uniquement, ou quand votre
frontend s'exécute déjà dans un autre terminal (ou une autre machine,
ou un aperçu déployé).

### Frontend seul

```bash
suprnova serve --frontend-only
```

Utile pour travailler sur l'UI sans payer le coût d'une recompilation
Rust à chaque sauvegarde, ou quand le backend s'exécute dans un autre
shell (ou dans Docker).

### Projet API uniquement

Un projet scaffoldé avec `suprnova new --api` n'a pas de répertoire
`frontend/`. Lancez `serve` exactement comme vous le feriez partout
ailleurs :

```bash
suprnova serve
```

`serve` ne voit aucun `frontend/package.json`, saute le panneau Vite et
la génération TypeScript qui l'alimente, et exécute le backend.
`--frontend-only` reste une erreur sur un tel projet : il réclame le
seul panneau qui n'existe pas.

### Ignorer la génération de types

```bash
suprnova serve --skip-types
```

Désactive le surveilleur de régénération TypeScript. Utilisez ceci
quand vous gérez `frontend/src/types/inertia-props.ts` à la main, ou
quand vous travaillez loin de tout code Inertia et voulez une sortie
plus silencieuse.

## Ce que fait réellement la commande

Quand vous exécutez `suprnova serve`, le CLI :

1. Charge `.env` depuis le répertoire courant.
2. Résout les ports backend et frontend (flag CLI → variable d'env → défaut).
3. Vérifie que vous êtes dans un projet Suprnova - `Cargo.toml` doit exister
   (sauf `--frontend-only`), et `--frontend-only` exige un répertoire
   `frontend/` doté d'un `package.json`. Un projet qui n'en a pas est servi
   en backend seul plutôt que refusé.
4. Régénère les types TypeScript à partir de toute struct
   `#[derive(InertiaProps)]` trouvée dans `src/`, en les écrivant dans
   `frontend/src/types/inertia-props.ts`. Ignoré quand le projet n'a pas de
   frontend.
5. Installe `cargo-watch` via `cargo install --locked --version "^8.5"
   cargo-watch` si elle n'est pas déjà sur le PATH (une seule fois, avec un
   avis "Installing..."). Ignoré sous `--frontend-only`. La version est bornée
   parce que `serve` pilote `cargo watch -x`, dont le sens n'est pas garanti
   d'un bump de version majeure à l'autre ; `--locked` construit l'arbre de
   dépendances publié par cargo-watch plutôt que de le réanalyser au moment de
   l'installation. Une commande qui installe un logiciel comme effet de bord du
   démarrage d'un serveur de développement ne devrait pas en plus choisir les
   versions à votre place.
6. Exécute `npm install` dans `frontend/` si `node_modules` n'existe pas encore.
   Ignoré sous `--backend-only`, et quand le projet n'a pas de frontend.
7. Lance `cargo watch` pour le backend, restreint par `-w` aux chemins à
   partir desquels le serveur est réellement construit : `src/`, `cmd/`,
   `Cargo.toml`, `Cargo.lock`, `.env` et `lang/`. C'est dans `cmd/` que le
   scaffold full-stack place le `main.rs` du binaire serveur ; le scaffold
   `--api` le place dans `src/` et n'a pas de `cmd/`. Chaque chemin n'est
   passé que s'il existe, parce que cargo-watch refuse de démarrer sur un
   chemin `-w` inexistant - un projet qui n'a pas encore été construit n'a
   pas de `Cargo.lock`, et celui-ci est pris en compte au `serve` suivant.

   `--no-vcs-ignores` les accompagne. cargo-watch applique votre
   `.gitignore` aux racines `-w` nommées explicitement, et pas seulement à
   son propre parcours du projet, et le scaffold place `.env` dans le
   `.gitignore` - sans ce flag, `-w .env` ne surveille donc rien du tout.
   Il ne peut pas élargir ce qui redémarre le backend, parce que `-w` a
   déjà restreint cela aux six chemins ci-dessus, et les seules choses
   ignorées par git à l'intérieur sont `.env` et (avec `--api`)
   `Cargo.lock`, toutes deux surveillées à dessein. `target/`,
   `node_modules` et le reste se trouvent de toute façon hors de chaque
   racine surveillée.

   Sur un projet full-stack scaffoldé, l'invocation complète est
   `cargo watch --no-vcs-ignores -w src -w cmd -w Cargo.toml -w Cargo.lock
   -w .env -w lang -x 'run --bin <package-name>'`. Les modifications du
   frontend et les `frontend/src/types/*.ts` générés sont hors de cette
   portée : ils ne redémarrent donc jamais le backend.
8. Lance `npm run dev` dans `frontend/` pour Vite, ce qui donne le HMR pour les
   composants Svelte/React/Vue et les classes Tailwind. Ignoré sous
   `--backend-only`, et quand le projet n'a pas de frontend.
9. Lance chaque processus supplémentaire déclaré dans le `Suprnova.toml` du
   projet (voir [Processus de dev supplémentaires](#processus-de-dev-supplémentaires)
   ci-dessous), chacun avec son préfixe `[name]` - workers de file d'attente,
   tailers de logs, tout autre processus que vous auriez autrement à jongler
   dans un autre terminal.
10. Démarre un surveilleur de fichiers sur `src/` qui réexécute le générateur
    de types chaque fois qu'un fichier `.rs` change, une fois que la salve de
    sauvegardes s'est tue pendant 500 ms. Seuls les vrais changements
    comptent - une création, une écriture ou une suppression. Les lectures,
    non, ce qui a son importance parce que le générateur lit à chaque
    exécution tous les fichiers `.rs` de l'arbre qu'il surveille. Ignoré
    quand le projet n'a pas de frontend, comme la génération de types au
    démarrage à l'étape 4. Le debounce se déclenche en fin de salve, si
    bien qu'une salve - `cargo fmt`, formatage à la sauvegarde sur
    plusieurs fichiers, un changement de branche - se fond en exactement
    une régénération qui s'exécute *après* la dernière écriture, plutôt
    qu'une régénération qui se déclencherait dès le premier fichier et
    manquerait le reste. Une régénération n'écrit le fichier que lorsque
    le TypeScript émis diffère de ce qui s'y trouve déjà, et le
    surveilleur ne signale que ce qu'il a écrit : une modification qui ne
    change la forme d'aucune prop n'affiche rien et n'émet aucun
    événement `types_regenerated`. Un silence après une sauvegarde
    signifie que votre modification n'a pas changé les types générés.
11. Redirige le stdout/stderr de chaque processus enfant vers votre terminal
    avec un préfixe `[name]` (`[backend]`, `[frontend]`, ou le nom configuré du
    processus), éventuellement horodaté avec `--timestamps` - ou, avec
    `--json`, sous forme d'événements NDJSON à la place (voir [Sortie JSON](#sortie-json)
    ci-dessous).

`Ctrl+C` signale au gestionnaire de processus de positionner son flag d'arrêt,
de tuer chaque processus enfant, et de quitter. Si un processus enfant quitte
de lui-même - une erreur de compilation Rust trop grave pour que `cargo watch`
s'en remette, un processus Vite en panne, ou un processus défini dans
`Suprnova.toml` qui a échoué - il est relancé après un bref backoff
(200 ms, doublé à chaque crash consécutif, plafonné à 5 s ; un processus qui
reste en vie 30 s remet la montée à zéro) au lieu de fermer la session. Passez
`--no-restart` pour retrouver l'ancien comportement : la sortie d'un seul
processus enfant ferme immédiatement toute la session.

Un processus qui boucle sur des crashs ne sera pas relancé indéfiniment :
`--restart-tries` (par défaut `5`) limite le nombre de crashs consécutifs qu'il
essaie avant d'abandonner ce processus - une nouvelle fenêtre de 30 s de
stabilité remet le compteur à zéro, comme le délai de backoff. L'abandon affiche
un message actionnable et cesse de réessayer *uniquement* ce processus ; les
autres (et la session elle-même) continuent de tourner, ce qui correspond au
comportement par défaut de `concurrently --restart-tries=5` côté Laravel. Voir
[Dépannage](#un-processus-boucle-sur-les-crashs).


### Pourquoi Suprnova diverge

Les utilisateurs de Laravel exécutent typiquement `php artisan serve`
pour le backend et `npm run dev` dans un autre terminal, et la
plupart des équipes masquent cette scission en deux terminaux avec un
`Procfile` et `foreman`/`overmind`. Suprnova livre ce multiplexeur
comme une commande CLI de première classe. Vous obtenez un seul
terminal, un seul `Ctrl+C`, un amorçage automatique de la chaîne
d'outils (`cargo-watch`, `npm install`), et un pont Inertia typé qui
régénère `frontend/src/types/inertia-props.ts` à la volée pour que
vos composants Svelte/React/Vue voient toujours la forme de props
courante sans synchronisation manuelle des types.

La commande `dev` de Laravel propose aussi les modes `--tabs` et `--stream`,
qui rendent tous deux la sortie via une petite TUI Node
(`@laravel/multiplex`). Suprnova ne fournit pas cette TUI : la sortie préfixée
dans un seul terminal est la norme dans l'écosystème des outils de développement
Rust (`cargo watch`, `bacon`, `just`), et un registre de processus doté de
préfixes colorés fournit déjà le signal « quel processus a dit cela » qu'offre
une TUI. Le travail sous-jacent de `--stream`  -  un flux d'événements temps réel
scriptable  -  est fourni par `--json` (voir [Sortie JSON](#sortie-json)) ; la
TUI à plusieurs panneaux de `--tabs` est le refus délibéré, non une lacune  -
un second modèle d'interaction et une seconde bibliothèque à maintenir sur tous
les terminaux pour un problème que cette page résout déjà. Voir la ligne
correspondante dans [Parité](parity.md#what-we-won-t-ship-and-why).

## Rechargement à chaud

**Backend.** `cargo watch` est la boucle, restreinte aux chemins à
partir desquels le serveur est construit. Elle recompile et redémarre
sur un changement sous `src/` ou `cmd/`, dans `Cargo.toml`,
`Cargo.lock` ou `.env`, ou sous `lang/` - `.env` n'est lu qu'une fois
par `Config::init` au démarrage, et les catalogues Fluent qu'une fois
à l'amorçage, si bien qu'un changement de l'un ou de l'autre ne prend
effet qu'au redémarrage. `.env` est surveillé grâce à
`--no-vcs-ignores`, sans lequel votre `.gitignore` le masquerait au
surveilleur. Sauvegarder un composant, ou régénérer
`frontend/src/types/inertia-props.ts`, se situe hors de cette portée
et laisse le backend en marche. Les recompilations à froid après avoir
touché une crate lourde peuvent prendre plusieurs secondes ; les
changements incrémentaux dans un seul fichier sont généralement
inférieurs à la seconde.

**Frontend.** Le HMR de Vite injecte les changements de composants
sur place sans rechargement complet, en préservant l'état des
composants. Les classes Tailwind se mettent à jour en direct via le
surveilleur Tailwind v4.

**Types TypeScript.** Chaque fois qu'un fichier `.rs` change, le
surveilleur de types réexécute le générateur. Si de nouvelles structs
`#[derive(InertiaProps)]` apparaissent (ou que des existantes changent
de forme), le `frontend/src/types/inertia-props.ts` régénéré déclenche
le HMR de Vite pour le composant qui les importe. Quand le TypeScript
émis est identique octet pour octet à ce qui se trouve déjà sur le
disque, le fichier est laissé intact et le surveilleur ne dit rien :
une régénération qui n'a rien changé n'est donc pas un changement
auquel quoi que ce soit en aval doive réagir - ni Vite, ni le
surveilleur du backend, ni ce qui lit `--json`.

## Processus de dev supplémentaires

`suprnova serve` exécute toujours le backend et Vite, mais la plupart des
projets ont plus de deux choses à garder en vie - un worker de file d'attente,
un tailer de logs, un mail-catcher. Déclarez-les dans un `Suprnova.toml` à la
racine du projet et `serve` les lance, les préfixe, et les relance
automatiquement aux côtés du backend et du frontend :

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

Chaque entrée a besoin de `name` et `command` ; `args` vaut par défaut vide,
`color` vaut par défaut l'une des couleurs verte/jaune/bleue/blanche assignées
dans l'ordre de déclaration (ou choisissez l'une des huit couleurs nommées de
`console` - black, red, green, yellow, blue, magenta, cyan, white). Les noms
doivent être uniques. `Suprnova.toml` est entièrement optionnel ; un projet qui
n'en a pas tourne exactement comme avant.

### Pourquoi Suprnova diverge

Laravel enregistre des processus de `dev` supplémentaires depuis PHP -
`DevCommands::register($command, $name)`, typiquement dans le `boot()` d'un
service provider - parce que `php artisan dev` lance un multiplexeur depuis le
même processus qui a déjà amorcé l'application. `suprnova serve` est un binaire
séparé de votre application ; il ne lie ni n'exécute jamais votre code Rust, et
ne fait qu'appeler `cargo watch` et `npm`. Il n'y a pas d'amorçage d'application
à crocheter, donc l'enregistrement doit être des données que le CLI lit plutôt
qu'un appel que votre code effectue - d'où `Suprnova.toml` au lieu d'une API
`DevProcesses::register()`.

## Sortie JSON

Passez `--json` et `suprnova serve` écrit un objet JSON par ligne (NDJSON) sur
stdout au lieu du texte coloré préfixé par `[name]` - rien d'autre n'atterrit
sur stdout tant qu'il est actif, si bien que vous pouvez le pipez directement
dans `jq` ou tout autre consommateur JSON orienté ligne. Chaque ligne porte un
champ `type` :

| `type` | Champs | Signification |
|---|---|---|
| `started` | `ts`, `name`, `pid` | Un processus (backend, frontend, ou une entrée `Suprnova.toml`) a été lancé pour la première fois. |
| `output` | `ts`, `name`, `stream` (`"stdout"` ou `"stderr"`), `line` | Une ligne de sortie d'un enfant, portée comme champ au lieu d'être recrachée brute. |
| `exited` | `ts`, `name`, `code` (nullable) | Un processus s'est arrêté. `code` vaut `null` s'il a été tué par un signal plutôt que de rendre un statut. |
| `restart_scheduled` | `ts`, `name`, `delay_ms` | Un processus en crash sera relancé après `delay_ms` (voir la séquence de backoff ci-dessus). |
| `restart_succeeded` | `ts`, `name`, `pid` | Une relance planifiée a réussi ; le processus tourne de nouveau sous un nouveau PID. |
| `gave_up` | `ts`, `name`, `tries` | Le processus a crashé `tries` fois de suite (`--restart-tries`) et `serve` a cessé d'essayer. La session, et tous les autres processus, continuent de tourner. |
| `types_regenerated` | `ts`, `artifact` (`"inertia_props"` ou `"lang_keys"`), `count` | Le surveilleur de fichiers a réécrit un artefact TypeScript après un changement `.rs` / `.ftl`. N'est émis que lorsque le fichier généré a réellement changé : une modification `.rs` qui laisse le TypeScript émis identique octet pour octet n'écrit rien et n'émet rien, si bien qu'un événement signifie toujours que le fichier sur le disque est désormais différent. `count` est le nombre de structs (ou d'ids de message) dans le fichier réécrit, pas le nombre de ceux qui ont changé. |
| `shutdown` | `ts` | La session est en train de s'arrêter. Toujours la dernière ligne. |

Par exemple, un crash Vite et sa relance ressemblent à ceci :

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` se compose avec `--timestamps` au lieu de s'y opposer : les combiner
n'est pas une erreur, mais `--timestamps` n'a alors aucun effet supplémentaire,
puisque chaque événement porte déjà son propre champ `ts`.

Cette sortie est destinée à être consommée par des outils machine - les noms de
champ et les valeurs de `type` ne changeront pas sans note dans le journal des
modifications. Traitez un `type` inconnu ou un champ supplémentaire inattendu
comme quelque chose à ignorer, pas comme une erreur, afin qu'une version
future puisse étendre le schéma sans casser votre consommateur.

## Dépannage

### Port déjà utilisé

```text
[backend] Error: Address already in use (os error 98)
```

Trouvez et tuez le processus, ou choisissez un autre port :

```bash
lsof -i :8765
kill -9 <pid>

# ou
suprnova serve --port 8081
```

### L'installation de `cargo-watch` échoue

Le CLI exécute `cargo install cargo-watch` si `cargo-watch` n'est pas
déjà sur le PATH. Si cette installation échoue (pas de réseau,
environnement restreint), installez-le manuellement une fois :

```bash
cargo install cargo-watch
```

Après cela, `suprnova serve` le trouvera et n'essaiera plus de
l'installer.

### Dépendances frontend bloquées

Si `npm install` échoue en cours de bootstrap, corrigez la cause
(registre npm accessible, espace disque, lockfile en bon état) et
exécutez-le manuellement :

```bash
cd frontend && npm install
```

Puis relancez `suprnova serve`. Le CLI n'exécute automatiquement
`npm install` que lorsque `node_modules` est manquant, si bien qu'une
installation manuelle réussie lui permet d'ignorer cette étape.

### La régénération de types ne détecte pas les changements

Le surveilleur sonde toutes les 2 secondes (en utilisant `notify` avec
un intervalle de sondage - choisi pour la fiabilité multiplateforme
plutôt que les bizarreries d'inotify) et debounce la régénération à
une fois toutes les 500 ms. Si un changement n'apparaît pas :

- Vérifiez que le fichier est sous `src/` (le surveilleur ne descend
  pas récursivement dans `crates/`, `cmd/`, ou `migrations/`).
- Vérifiez que la struct a bien `#[derive(InertiaProps)]`.
- Redémarrez `suprnova serve` et surveillez le message de démarrage
  `Generated N type(s)` - si vous voyez `No InertiaProps structs
  found`, le scanner n'a rien trouvé à émettre.

### Un processus boucle sur les crashs

Si un enfant  -  backend, frontend, ou une entrée de `Suprnova.toml`  -  ne peut
pas démarrer (mauvais code, binaire manquant, conflit de port), il est relancé
selon la séquence de backoff décrite ci-dessus au lieu de s'arrêter. Regardez
les lignes `[name]` juste avant chaque avis « respawning in …ms » pour trouver
l'erreur réelle (un `error[E…]` de rustc, une ENOENT, ou ce que l'enfant a
imprimé). Corrigez la cause ; la prochaine tentative de relance la prendra
automatiquement en compte. Pour arrêter les tentatives et voir une seule fois
l'échec, relancez avec `--no-restart` : la session se ferme alors au premier
crash, comme le faisait `suprnova serve` avant l'existence de cette option.

Après `--restart-tries` (par défaut `5`) crashs consécutifs, `serve` cesse de
relancer ce processus seul et affiche un message qui le nomme :

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

Les autres processus et la session elle-même continuent de tourner. Corrigez
la cause puis relancez `suprnova serve` pour ramener le processus abandonné ;
vous n'avez pas besoin de redémarrer toute la session.

## Suivant

- [Installation](installation.md) - installer le CLI sur votre
  machine
- [Démarrage rapide](quickstart.md) - une procédure complète pour
  votre première application
- [Structure des répertoires](structure.md) - ce que `suprnova new` a
  scaffoldé
- [Générateurs de code](cli-generators.md) - `make:controller`,
  `make:action`, etc.
- [Console](console.md) - le binaire `cargo run --bin console` par
  projet
