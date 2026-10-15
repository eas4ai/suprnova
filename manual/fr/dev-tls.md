# URLs HTTPS nommées de développement (`suprnova dev:tls`)

Par défaut, `suprnova serve` sert votre backend sur une simple
`http://127.0.0.1:8765`. Cela convient à la plupart des besoins de
développement - mais certaines fonctionnalités du navigateur ne
fonctionnent qu'en HTTPS, sur un hôte nommé :

- **Passkeys / WebAuthn** - nécessitent un contexte sécurisé et une
  origine stable.
- **Cookies `Secure`** et **`SameSite=None`** - ne se positionnent
  qu'en HTTPS.
- **Service Workers** - ne s'enregistrent qu'en HTTPS (ou en
  `localhost`).
- **URI de redirection OAuth/OIDC** - les fournisseurs rejettent
  souvent les hôtes en IP/port bruts.

[portless](https://portless.sh) donne à chaque application locale une
URL stable `https://<name>.localhost`, derrière un unique proxy TLS
sur le port 443. `suprnova dev:tls` relie Suprnova à portless et -
c'est la partie facile à mal comprendre - fait confiance au CA local
de portless dans **chaque magasin de certificats de navigateur sur
votre machine**, sans sudo sous Linux.

> **Strictement opt-in.** portless n'est jamais requis. `suprnova
> serve` fonctionne sans portless installé. Vous y optez en
> scaffoldant (`suprnova new <name> --with-portless`) ou en ajoutant
> `portless.json` plus tard. Si vous n'exécutez jamais `dev:tls`, vous
> ne touchez jamais à portless.

## Installer portless

portless est un outil Node :

```bash
npm install -g portless
```

Puis installez une fois son proxy 443 permanent (une étape de niveau
système, qui exige sudo et qui relève de portless, pas de Suprnova) :

```bash
portless service install
```

## Par projet

Vous avez deux façons d'embarquer un projet.

**Scaffolder avec le flag** - écrit `portless.json` dès le départ :

```bash
suprnova new myapp --frontend svelte --with-portless
```

Cela émet un `portless.json` à la racine du projet :

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` est le `SERVER_PORT` fixe de votre backend. Cela indique à
portless que l'app se lie à un port connu (plutôt que portless n'en
attribue un via `$PORT`), si bien que l'URL nommée route directement
vers lui.

**L'ajouter à un projet existant** - écrivez ce même `portless.json` à
la main (ou lancez `portless alias myapp 8765`), en utilisant votre
`SERVER_PORT`.

Puis, sur **chaque machine** qui exécutera l'app, faites l'inscription
unique de confiance + de route :

```bash
cd myapp
suprnova dev:tls
```

Cela :

1. Vérifie que `portless` est sur votre PATH.
2. Résout le nom (`--name`, sinon `[package].name` de `Cargo.toml`) et
   le port (`--port`, sinon `SERVER_PORT`, sinon `8765`).
3. Enregistre la route `myapp.localhost → 127.0.0.1:8765` (à sauter
   avec `--no-alias`).
4. Fait confiance au CA de portless dans les magasins de certificats
   de vos navigateurs.
5. Affiche les étapes suivantes.

Flags :

| Flag | Effet |
|---|---|
| `--name <name>` | Remplace le nom de l'URL. Défaut : le nom du package `Cargo.toml`. |
| `--port <port>` / `-p` | Remplace le port routé. Défaut : `SERVER_PORT`, sinon `8765`. |
| `--no-alias` | Ne fait confiance qu'au CA ; ne touche pas à la route portless. |
| `--yes` | Ignore la confirmation avant de modifier vos magasins de certificats. Ignoré si l'empreinte du CA a changé depuis la dernière exécution - dans ce cas elle est toujours demandée. |

### Pourquoi l'étape 4 demande d'abord confirmation

Faire confiance à un CA signifie que chaque certificat qu'il signe est
accepté par votre navigateur, silencieusement, pour chaque site. Cela
vaut bien une frappe de clavier délibérée.

Le CA n'est résolu qu'à partir du propre état de portless, jamais de
ce que le répertoire du projet peut influencer - un dépôt cloné ne
peut pas pointer `dev:tls` vers un CA de son choix. La commande
affiche l'empreinte à laquelle elle est sur le point de faire
confiance, et attend votre confirmation. Si l'empreinte diffère de
celle approuvée précédemment, elle demande même sous `--yes` : un CA
modifié est soit une réinstallation de portless, soit quelque chose
que vous voulez examiner, et seul vous pouvez le dire.

## Lancer

```bash
suprnova serve
```

Ouvrez `https://myapp.localhost`.

Le backend se lie à `8765` par défaut ; le serveur de développement
Vite roule en parallèle sur `5765` en `http://localhost`. Une page
servie depuis l'origine HTTPS peut référencer des assets
`http://localhost`, car les navigateurs traitent `localhost` comme un
contexte sécurisé - ce n'est **pas** bloqué comme contenu mixte.

> **Le rechargement à chaud (HMR) en HTTPS est du best-effort.** Le
> websocket HMR de Vite se reconnecte au serveur de développement ;
> que cela réussisse proprement sur l'origine HTTPS dépend de vos
> versions de Vite/navigateur. Si les mises à jour en direct
> s'arrêtent sous `https://`, pointez Vite vers une origine de serveur
> de développement HTTPS via la variable d'environnement
> `INERTIA_VITE_DEV_SERVER`. Le chargement des pages et le reste du
> flux ne sont pas affectés.

## Plusieurs applications

portless possède le port 443 et multiplexe par sous-domaine.
Enregistrez chaque application avec son propre nom et son propre
port :

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

Ne liez jamais le port 443 directement depuis une application - c'est
le travail de portless.

## Dépannage

**`ERR_CERT_AUTHORITY_INVALID` après avoir lancé `dev:tls`.** Votre
navigateur n'a pas été complètement redémarré. Les navigateurs lisent
leur magasin de certificats une seule fois au lancement ; recharger un
onglet ne suffit pas. Tapez `chrome://restart` (ou quittez et relancez
complètement).

**`502 Bad Gateway`.** Le proxy est actif mais pas votre backend.
Lancez `suprnova serve` dans le répertoire du projet.

**`portless trust` dit « A terminal is required to authenticate ».**
C'est la propre commande de portless qui a besoin d'un vrai TTY pour
`sudo`. `suprnova dev:tls` la contourne entièrement sous Linux : elle
installe le CA directement dans les magasins NSS de vos navigateurs,
qui n'ont besoin d'aucun sudo.

**Un navigateur Flatpak reste non approuvé.** Les navigateurs Flatpak
gardent leur base NSS sous `~/.var/app/<id>/.pki/nssdb`. `dev:tls`
couvre ces cas - relancez-le et redémarrez complètement ce navigateur.

**`certutil: command not found`.** Installez les outils NSS :

| Distro | Commande |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`portless CA not found at ~/.portless/ca.pem`.** portless génère son
CA quand le proxy tourne pour la première fois. Démarrez-le une fois
(`systemctl start portless`, ou `portless proxy start`), puis relancez
`suprnova dev:tls`.

## Notes par plateforme

Le chemin NSS de navigateur ci-dessus est le mécanisme Linux. Sur
**macOS** et **Windows**, les navigateurs lisent le trousseau du
système / magasin de certificats, si bien que `dev:tls` délègue la
confiance du CA à `portless trust`, qui cible ces magasins natifs.
