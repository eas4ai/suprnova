# Leitfaden für Beiträge

Suprnova ist Open Source unter der MIT-Lizenz, und der wertvollste
Beitrag ist ein **guter Bericht**. Das Projekt nimmt keine Pull
Requests an: Das Framework wird durchgehend von den Maintainern
verfasst, und jede Änderung läuft durch die Maintainer, damit die
gesamte Oberfläche eine Form behält. Das ist eine bewusste, dauerhafte
Haltung - keine Vor-1.0-Phase.

MIT bedeutet, dass Sie nie eine Erlaubnis brauchen, um den Code selbst
weiterzuführen: **forken Sie frei**. Ein Fork, der sich in eine eigene
Richtung entwickelt, ist ein gesundes Ergebnis, keine Rivalität.

Was das in der Praxis bedeutet:

- **Bug-Reports** - willkommen, über
  [GitHub Issues](https://github.com/eas4ai/suprnova/issues).
- **Feature-Anfragen** - willkommen, über Issues. Beschreiben Sie den
  Anwendungsfall, nicht die Umsetzung; oft gibt es bereits eine geplante
  Form (meistens das Laravel-Äquivalent).
- **Doku-Fehler** - willkommen, über Issues. Wenn ein Kapitel behauptet,
  dass eine API existiert, und Sie sie nicht finden können, ist das ein
  Doku-Fehler - nennen Sie das Kapitel und was Sie erwartet haben.
- **Sicherheitsprobleme** - privat, per E-Mail (siehe unten). Niemals
  als öffentliche Issues.
- **Pull Requests** - werden nicht angenommen. PRs werden mit einem
  Verweis auf dieses Kapitel geschlossen; öffnen Sie stattdessen ein
  Issue, damit der Fix upstream landen kann, oder forken Sie und tragen
  Sie die Änderung selbst.

## Einen Bug-Report einreichen, der schnell behoben wird

Der Goldstandard ist eine Reproduktion aus einem frischen Scaffold:

```bash
suprnova new repro-app --frontend vue --no-interaction
# …kleinste Änderung, die den Fehler zeigt…
```

Enthalten sein sollte:

1. **Was Sie getan haben** - die Befehle und der Code, auf das Minimum
   gekürzt
2. **Was Sie erwartet haben** - ein Satz
3. **Was stattdessen passiert ist** - die tatsächliche Ausgabe oder der
   Fehler, wortwörtlich eingefügt
4. **Versionen** - der Framework-Tag (`suprnova --version`, oder das
   `tag =` in Ihrer `Cargo.toml`) und Ihre Rust-Version
   (`rustc --version`)

Ein fehlschlagender Test ist noch besser als Prosa. Wenn Sie den Bug
als Test gegen das Framework ausdrücken können, fügen Sie ihn in das
Issue ein - er wird meistens zum Regressionstest, mit dem der Fix
landet.

## Aus dem Quellcode bauen (um einen Bericht zu untersuchen)

Sie brauchen das nicht, um ein Issue zu *melden*, aber die Reproduktion
gegen den Workspace schärft einen Bericht oft:

```bash
git clone https://github.com/eas4ai/suprnova.git
cd suprnova
cargo check --workspace          # alles typprüfen
cargo test --workspace           # die vollständige Suite ausführen (~3400 Tests)
```

Das Workspace-Layout: `framework/` (die `suprnova`-Crate),
`suprnova-cli/` (die `suprnova`-Binärdatei), `suprnova-macros/` (Proc
Macros), `app/` (interne Dogfood-App), `crates/` (Zahlungs- und
Web-Push-Adapter) und `manual/` (dieses Handbuch).

## Der Maßstab, an dem sich der Code messen lassen muss

Keine Contributor-Regeln - aber den Maßstab zu kennen hilft Ihnen,
Berichte zu kalibrieren (ein Panic aus Bibliothekscode, ein fehlender
Test für einen Fehlerfall, oder eine API, die `unwrap()` erzwingt, ist
immer berichtenswert):

- **Nur vollständige Implementierungen.** Keine TODOs, keine
  Teil-Scaffolds. Ein Fix landet mit dem Regressionstest, der ihn
  festnagelt.
- **Code auf der öffentlichen Oberfläche gibt `Result` zurück und
  panikt nicht.** Wo ein unfehlbarer Name im Laravel-Stil ausgeliefert
  wird, liefert ein `try_*`-Geschwister mit.
- **Kein `unsafe` außerhalb des Umgebungs-Bootstraps.** Das Framework
  hat genau zwei `unsafe`-Blöcke in Nicht-Test-Code, beide in
  `config/env.rs::load_dotenv`, beide umschließen `std::env::set_var` /
  `remove_var` - was in Edition 2024 `unsafe` wurde - und beide tragen
  einen SAFETY-Hinweis für die Boot-Zeit-Single-Thread-Invariante, auf
  die sie sich verlassen. Alles andere ist nur für Tests. Neues
  `unsafe` an jeder anderen Stelle braucht eine schriftliche
  Begründung im Review, und `unsafe` in einem Treiber, Handler oder
  einer Makro-Expansion wird nicht akzeptiert.
- **`cargo fmt` und Clippy ohne pauschales Warnungsverbot sind maßgeblich.**

Siehe [Fehlermodell](error-model.md) für den vollständigen
Fehler-Vertrag.

## Sicherheit

Melden Sie Sicherheitsprobleme privat an
**shawn@eas4ai.com** (den Projekt-Maintainer). Wir bestätigen den
Eingang innerhalb weniger Tage, arbeiten den Fix auf einem privaten
Branch und stimmen die Offenlegung mit Ihnen ab.

Reichen Sie Sicherheitsprobleme nicht als öffentliche GitHub Issues
ein, bevor ein Fix ausgeliefert ist.

### Advisories zu Abhängigkeiten

`cargo audit` läuft im Release-Gate. Wenn es für ein Advisory keinen Fix
gibt und der verwundbare Code in einem Default-Build nicht erreichbar
ist, kann es der Ignore-Liste des Audits hinzugefügt werden - aber jeder
Eintrag braucht drei Dinge, und ohne sie schlägt das Gate fehl:

```toml
# OWNER: name <email>
# EXPIRES: YYYY-MM-DD
"RUSTSEC-XXXX-XXXX",
```

- einen **Owner**, damit die Ausnahme jemandem gehört;
- ein **Ablaufdatum**, nach dem das Gate die Ausführung verweigert, bis
  der Eintrag mit einer angegebenen Begründung erneuert oder gelöscht
  wird;
- ein **schriftliches Erreichbarkeitsargument** - welcher Pfad sie
  hereinzieht und warum ein Default-Build sie nicht linkt.

Erreichbarkeitsbehauptungen werden geprüft, nicht geglaubt. Wenn Ihr
Argument lautet „das steht hinter einem standardmäßig deaktivierten
Feature“, löst das Release-Gate die echten Abhängigkeitsbäume auf und
weist nach, dass die Crate im Default-Baum fehlt und im per Opt-in
aktivierten vorhanden ist. Eine Ausnahme, deren Begründung nichts
verifiziert, hört still auf zu stimmen, sobald jemand eine
Abhängigkeit hinzufügt.

Ein Ignore ist die Entscheidung, ein bekanntes Problem auszuliefern. Es
sollte sich auch so lesen.

## Lizenz

MIT, mit Namensnennung des Upstream-Projekts
[Kit](https://github.com/dayemsiddiqui/kit), von dem wir geforkt
haben.
