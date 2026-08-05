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

### Ignorer la génération de types

```bash
suprnova serve --skip-types
```

Désactive le surveilleur de régénération TypeScript. Utilisez ceci
quand vous gérez `frontend/src/types/inertia-props.ts` à la main, ou
quand vous travaillez loin de tout code Inertia et voulez une sortie
plus silencieuse.

## Ce qu'il fait réellement

Quand vous exécutez `suprnova serve`, le CLI :

1. Charge `.env` depuis le répertoire courant.
2. Résout les ports backend et frontend (flag CLI → variable d'env →
   défaut).
3. Vérifie que vous êtes dans un projet Suprnova - `Cargo.toml` doit
   exister (sauf `--frontend-only`) et un répertoire `frontend/` doit
   exister (sauf `--backend-only`).
4. Régénère les types TypeScript à partir de toute struct
   `#[derive(InertiaProps)]` trouvée dans `src/`, en les écrivant dans
   `frontend/src/types/inertia-props.ts`.
5. Installe `cargo-watch` via `cargo install --locked --version "^8.5"
   cargo-watch` si elle n'est pas déjà sur le PATH (une seule fois,
   avec un avis "Installing..."). Ignoré sous `--frontend-only`.
   La version est bornée parce que `serve` pilote `cargo watch -x`,
   dont le sens n'est pas garanti d'un bump de version majeure à
   l'autre ; `--locked` construit l'arbre de dépendances que
   cargo-watch a publié plutôt que de le réanalyser au moment de
   l'installation. Une commande qui installe un logiciel comme effet
   de bord du démarrage d'un serveur de dev ne devrait pas en plus
   choisir les versions à votre place.
6. Exécute `npm install` dans `frontend/` si `node_modules` n'existe
   pas encore. Ignoré sous `--backend-only`.
7. Lance `cargo watch -x 'run --bin <package-name>'` pour le backend.
   `cargo-watch` réexécute le binaire chaque fois qu'un fichier `.rs`
   change.
8. Lance `npm run dev` dans `frontend/` pour Vite, ce qui vous donne
   le HMR pour les composants Svelte/React/Vue et les classes
   Tailwind.
9. Démarre un surveilleur de fichiers sur `src/` qui réexécute le
   générateur de types chaque fois qu'un fichier `.rs` change, une
   fois que la salve de sauvegardes s'est tue pendant 500 ms. Le
   debounce se déclenche en fin de salve, si bien qu'une salve -
   `cargo fmt`, formatage à la sauvegarde sur plusieurs fichiers, un
   changement de branche - se fond en exactement une régénération qui
   s'exécute *après* la dernière écriture, plutôt qu'une régénération
   qui se déclencherait dès le premier fichier et manquerait le
   reste.
10. Redirige le stdout/stderr des deux processus enfants vers votre
    terminal avec les préfixes `[backend]` et `[frontend]`.

`Ctrl+C` signale au gestionnaire de processus de positionner son flag
d'arrêt, de tuer les deux processus enfants, et de quitter. Si l'un
des deux processus quitte de lui-même - généralement à cause d'une
erreur de compilation Rust trop grave pour que `cargo watch` s'en
remette, ou d'un conflit de port - le gestionnaire de processus
traite cela comme un signal d'arrêt et arrête l'autre.

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

## Rechargement à chaud

**Backend.** `cargo watch -x 'run --bin <package>'` est la boucle.
Elle recompile et redémarre le serveur à chaque changement `.rs` dans
le projet. Les recompilations à froid après avoir touché une crate
lourde peuvent prendre plusieurs secondes ; les changements
incrémentaux dans un seul fichier sont généralement inférieurs à la
seconde.

**Frontend.** Le HMR de Vite injecte les changements de composants
sur place sans rechargement complet, en préservant l'état des
composants. Les classes Tailwind se mettent à jour en direct via le
surveilleur Tailwind v4.

**Types TypeScript.** Chaque fois qu'un fichier `.rs` change, le
surveilleur de types réexécute le générateur. Si de nouvelles structs
`#[derive(InertiaProps)]` apparaissent (ou que des existantes changent
de forme), le `frontend/src/types/inertia-props.ts` régénéré déclenche
le HMR de Vite pour le composant qui les importe.

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

### Le backend quitte silencieusement juste après le démarrage

Quand l'un des processus enfants quitte, le gestionnaire de processus
arrête aussi l'autre. Si le backend est mort avec une erreur de
compilation, les lignes `[backend]` juste au-dessus du message
"Servers stopped." afficheront le `error[E…]` de rustc. Corrigez
l'erreur de compilation et relancez.

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
