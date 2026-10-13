# Benannte HTTPS Dev URLs (`suprnova dev:tls`)

Standardmäßig liefert `suprnova serve` Ihr Backend über ein einfaches
`http://127.0.0.1:8765` aus. Das reicht für die meiste Entwicklung -
aber manche Browser-Funktionen laufen nur über HTTPS auf einem
benannten Host:

- **Passkeys / WebAuthn** - benötigen einen sicheren Kontext und einen
  stabilen Origin.
- **`Secure`-Cookies** und **`SameSite=None`** - werden nur über HTTPS
  gesetzt.
- **Service Worker** - registrieren sich nur über HTTPS (oder
  `localhost`).
- **OAuth/OIDC-Redirect-URIs** - Provider lehnen bloße IP/Port-Hosts
  oft ab.

[portless](https://portless.sh) gibt jeder lokalen App eine stabile
`https://<name>.localhost`-URL hinter einem einzigen TLS-Proxy auf
Port 443. `suprnova dev:tls` verdrahtet Suprnova mit portless und
vertraut - das ist der Teil, den man leicht falsch macht - der
lokalen CA von portless in **jedem Zertifikatsspeicher jedes Browsers
auf Ihrem Rechner**, ohne sudo unter Linux.

> **Rein optional.** portless ist nie erforderlich. `suprnova serve`
> funktioniert auch ohne installiertes portless. Sie aktivieren es
> beim Scaffolden (`suprnova new <name> --with-portless`) oder indem
> Sie `portless.json` später hinzufügen. Wenn Sie `dev:tls` nie
> ausführen, kommen Sie nie mit portless in Berührung.

## portless installieren

portless ist ein Node-Tool:

```bash
npm install -g portless
```

Installieren Sie danach einmalig dessen dauerhaften 443-Proxy (das
ist ein Schritt auf Systemebene, der sudo braucht und zu portless
gehört, nicht zu Suprnova):

```bash
portless service install
```

## Pro Projekt

Es gibt zwei Wege, ein Projekt einzubinden.

**Mit dem Flag scaffolden** - schreibt `portless.json` von Anfang an:

```bash
suprnova new myapp --frontend svelte --with-portless
```

Das erzeugt eine `portless.json` im Projekt-Root:

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` ist der feste `SERVER_PORT` Ihres Backends. Er sagt
portless, dass die App einen bekannten Port bindet (statt dass
portless über `$PORT` selbst einen zuweist), sodass die benannte URL
direkt dorthin routet.

**Zu einem bestehenden Projekt hinzufügen** - schreiben Sie dieselbe
`portless.json` von Hand (oder führen Sie `portless alias myapp 8765`
aus), mit Ihrem `SERVER_PORT`.

Führen Sie dann auf **jedem Rechner**, der die App ausführen wird,
die einmalige Vertrauens- und Routen-Registrierung durch:

```bash
cd myapp
suprnova dev:tls
```

Das:

1. Prüft, ob `portless` in Ihrem PATH liegt.
2. Löst den Namen auf (`--name`, sonst der `[package].name` aus
   `Cargo.toml`) und den Port (`--port`, sonst `SERVER_PORT`, sonst
   `8765`).
3. Registriert die Route `myapp.localhost → 127.0.0.1:8765`
   (überspringen mit `--no-alias`).
4. Vertraut der CA von portless in den Zertifikatsspeichern Ihrer
   Browser.
5. Gibt die nächsten Schritte aus.

Flags:

| Flag | Wirkung |
|---|---|
| `--name <name>` | Überschreibt den URL-Namen. Standard: der Package-Name aus `Cargo.toml`. |
| `--port <port>` / `-p` | Überschreibt den gerouteten Port. Standard: `SERVER_PORT`, sonst `8765`. |
| `--no-alias` | Vertraut nur der CA; rührt die portless-Route nicht an. |
| `--yes` | Überspringt die Bestätigung vor der Änderung Ihrer Zertifikatsspeicher. Wird ignoriert, wenn sich der Fingerprint der CA seit dem letzten Lauf geändert hat - das fragt immer nach. |

### Warum Schritt 4 zuerst nachfragt

Einer CA zu vertrauen bedeutet, dass jedes Zertifikat, das sie
signiert, von Ihrem Browser akzeptiert wird - stillschweigend, für
jede Site. Das ist einen bewussten Tastendruck wert.

Die CA wird ausschließlich aus dem eigenen Zustand von portless
aufgelöst, nie aus etwas, das das Projektverzeichnis beeinflussen
kann - ein ausgechecktes Repo kann `dev:tls` nicht auf eine CA seiner
Wahl lenken. Der Befehl gibt den Fingerprint aus, dem er gleich
vertrauen wird, und wartet auf Ihre Bestätigung. Weicht der
Fingerprint von dem zuvor vertrauten ab, fragt er selbst unter
`--yes` nach: Eine geänderte CA ist entweder eine Neuinstallation von
portless oder etwas, das Sie sich ansehen sollten - und nur Sie
können das unterscheiden.

## Ausführen

```bash
suprnova serve
```

Öffnen Sie `https://myapp.localhost`.

Das Backend bindet standardmäßig `8765`; der Vite-Dev-Server läuft
begleitend auf `5765` über `http://localhost`. Eine Seite, die vom
HTTPS-Origin ausgeliefert wird, kann auf `http://localhost`-Assets
verweisen, weil Browser `localhost` als sicheren Kontext behandeln -
das gilt **nicht** als Mixed Content.

> **Hot Module Reload über HTTPS ist Best-Effort.** Vites
> HMR-Websocket verbindet sich zurück zum Dev-Server; ob das über den
> HTTPS-Origin sauber funktioniert, hängt von Ihren
> Vite-/Browser-Versionen ab. Wenn Live-Updates unter `https://`
> aufhören zu funktionieren, weisen Sie Vite über die
> Umgebungsvariable `INERTIA_VITE_DEV_SERVER` einen
> HTTPS-Dev-Server-Origin zu. Seitenaufrufe und der Rest des Ablaufs
> bleiben davon unberührt.

## Mehrere Apps

portless besitzt Port 443 und multiplext nach Subdomain. Registrieren
Sie jede App mit ihrem eigenen Namen und Port:

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

Binden Sie 443 nie direkt aus einer App heraus - das ist die Aufgabe
von portless.

## Fehlerbehebung

**`ERR_CERT_AUTHORITY_INVALID` nach dem Ausführen von `dev:tls`.**
Ihr Browser wurde nicht vollständig neu gestartet. Browser lesen
ihren Zertifikatsspeicher nur einmal beim Start; ein Reload des Tabs
reicht nicht. Geben Sie `chrome://restart` ein (oder beenden Sie den
Browser vollständig und starten Sie ihn neu).

**`502 Bad Gateway`.** Der Proxy läuft, aber Ihr Backend nicht.
Führen Sie `suprnova serve` im Projektverzeichnis aus.

**`portless trust` meldet „A terminal is required to
authenticate“.** Das ist der eigene Befehl von portless, der für
`sudo` ein echtes TTY braucht. `suprnova dev:tls` umgeht das unter
Linux vollständig: Es installiert die CA direkt in die NSS-Speicher
Ihrer Browser, die kein sudo benötigen.

**Ein Flatpak-Browser bleibt weiterhin nicht vertrauenswürdig.**
Flatpak-Browser halten ihre NSS-Datenbank unter
`~/.var/app/<id>/.pki/nssdb`. `dev:tls` deckt das ab - führen Sie es
erneut aus und starten Sie diesen Browser vollständig neu.

**`certutil: command not found`.** Installieren Sie die NSS-Tools:

| Distro | Befehl |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`portless CA not found at ~/.portless/ca.pem`.** portless erzeugt
seine CA, wenn der Proxy zum ersten Mal läuft. Starten Sie ihn
einmal (`systemctl start portless`, oder `portless proxy start`),
und führen Sie dann `suprnova dev:tls` erneut aus.

## Hinweise zur Plattform

Der Browser-NSS-Pfad oben ist der Linux-Mechanismus. Unter **macOS**
und **Windows** lesen Browser die Keychain bzw. den
Zertifikatsspeicher des Betriebssystems, daher delegiert `dev:tls`
das CA-Vertrauen an `portless trust`, das genau diese nativen
Speicher anspricht.
