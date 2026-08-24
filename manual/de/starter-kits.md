# Starter-Kits

Starter-Kits sind einsatzbereite Suprnova-Anwendungen, die Sie forken und einsetzen. Jedes verdrahtet die Controller, Routen, Migrationen, Frontend-Seiten und Tests für eine komplette Produktoberfläche - Sie starten also von einer laufenden App, nicht von einem leeren Scaffold.

Heute werden zwei Kits versendet, modelliert nach Laravels Abstammung. Wählen Sie das Kit aus, das Ihren Anforderungen am nächsten kommt, und passen Sie es anschließend an.

## Nebula - Authentifizierung (Breeze-Ebene)

**Repository: [github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

Das minimale Full-Auth Kit - Suprnovas Breeze-Äquivalent. Alles, was Sie für Konten brauchen, und nichts, was Sie nicht brauchen:

- Registrierung mit E-Mail-Verifizierung
- Anmeldung mit Remember-Me
- Passwort-Zurücksetzen mit Anti-Enumeration-Responses
- Profilmanagement - E-Mail und Passwort aktualisieren, Konto löschen
- Ein markiertes Inertia 3 + Svelte 5 Frontend (standardmäßig dunkel), mit dem angemeldeten Benutzermenü verdrahtet

Nebula versendet zwei Test-Suites: Facade-Level Auth-Logik und eine Wire-Level HTTP-Suite, die echte Routen, Sessions, CSRF-Umlaufbahnen und die Guest/Auth/Verified-Gates über einen Loopback-Socket treibt.

Greifen Sie zu Nebula, wenn Sie eine saubere Kontoverwaltungsgrundlage haben möchten, auf der Sie Ihr eigenes Produkt aufbauen.

## Pulsar - Produktsite und Community

**Repository: [github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

Eine komplette Developer-Tool/SaaS-Unternehmenssite auf Vue 3.5 + Vuetify. Alles aus Nebulas Auth-Geschichte, plus die Oberflächen, die eine echte Produktsite braucht:

- Marketing-Landingpage und ein Benutzer-Dashboard
- Eine Markdown-Dokumentations-Pipeline (`docs:build`) mit Suche und einer generierten Inhaltsübersicht
- Ein Blog/Artikel-System mit einem RSS-Feed
- Öffentliche Memberprofile
- Taxonomie - Themen, Tags und Kategorien
- Rollenbasierte Zugriffskontrolle: Rollen, Berechtigungen und Gates
- Admin- und Moderationsoberflächen für Inhalte und Mitglieder

Pulsar ist das Source-Kit für Downstream-Produkte wie `suprnova.app`. Greifen Sie dazu, wenn Sie eine Produktsite mit Docs, einem Blog und einer Member-Community einsetzen - nicht nur Authentifizierung.

## Welches Kit?

| Sie möchten… | Beginnen Sie mit |
|---|---|
| Konten und einen Ort zum Bauen | **Nebula** |
| Eine vollständige Produktsite - Landing, Docs, Blog, Community, RBAC | **Pulsar** |
| Ein API-only Backend (Token-Auth, kein Frontend) | `suprnova new my-api --api` |

Beide Kits verfolgen das Framework als Git-Abhängigkeit und laufen auf demselben Stack, den Sie bereits kennen - siehe die README jedes Repos für die Einrichtung. Weitere Kits sind geplant; beobachten Sie die [Releases](https://github.com/eas4ai/suprnova/releases) oder öffnen Sie einen Issue, wenn es eines gibt, das Sie möchten.

## Was das Standard-Scaffold bietet

Wenn keines der Kits passt, versendet `suprnova new my-app --frontend svelte` (oder `react`, oder `vue`) bereits einen funktionierenden Authentifizierungsfluss - Anmeldung, Registrierung, Abmeldung, Session-Authentifizierung mit der `authenticate`-Middleware, CSRF-Schutz und eine geschützte `/dashboard`-Route - auf einem der drei Frontends (Svelte 5, React 19, Vue 3.5) mit Tailwind v4 und Inertia v3. Siehe [Installation](installation.md) für die Scaffold-Ausgabe und [Schnellstart](quickstart.md) für die Walkthrough der ersten fünf Minuten.

Für API-only-Services initialisiert `suprnova new my-api --api` Magnetar,
installiert Bearer-Session-Middleware und legt ohne Frontend die
Passwortregistrierung und Anmeldung gegen die kanonische Tabelle
`app_users` an.

## Zu einem Starter-Kit beitragen

Etwas Wiederverwendbares auf Suprnova aufgebaut und möchten es als kanonisches Kit aufwärts einreichen? Siehe [Leitfaden für Beiträge](contributions.md). Wir freuen uns, eine echte Implementierung zu nehmen und sie zu einem generischen Kit zu machen.
