# Lokalisierung

Lokalisierung in Suprnova ist ein Modul mit vier Gesichtern:
Message-Kataloge auf dem Server, Validierungsfehler, die bereits
übersetzt ankommen, dieselben Katalog-Bytes, die an den Browser
gehen, und locale-bewusste Zahlen-, Datums- und
Listenformatierung. Das Message-Format ist
[Fluent](https://projectfluent.org) - Mozillas `.ftl`, dasselbe, das
Firefox ausliefert - und das gesamte Subsystem ist standardmäßig
aktiv, hinter dem `localization`-Feature.

Die kürzestmögliche Tour. Schreiben Sie einen Katalog:

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

Verwenden Sie ihn aus einem Handler heraus:

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

Eine Anfrage mit `Accept-Language: es` bekommt die spanische
Zeichenkette, weil `LocaleMiddleware` das Locale aufgelöst hat,
bevor Ihr Handler lief. Sonst ändert sich im Handler nichts - kein
Locale-Parameter wird durchgereicht, kein `&Translator` in der
Signatur.

## Warum Lokalisierung

Drei Gründe, warum das eine Framework-Angelegenheit ist statt einer
Crate, die Sie sich aussuchen:

- **Validierungsmeldungen sind die Zeichenketten des Frameworks,
  nicht Ihre.** „The email field is required.“ wird tief in
  `Rule::passes` ausgegeben, weit weg von jedem Code, den Sie
  besitzen. Wenn das Framework keine Übersetzungsnaht trägt,
  liefert eine spanische App englische Validierungsfehler aus -
  oder Sie umschließen jede Regel von Hand. Suprnovas eingebaute
  Regeln geben *keyed* Messages zurück; Sie übersetzen sie, indem
  Sie eine `.ftl`-Datei hineinlegen, und fassen die Regeln nie an.
- **Der Browser braucht dieselben Zeichenketten.** Eine
  Inertia-App rendert die Hälfte ihres Texts in Rust und die
  Hälfte in Svelte/React/Vue. Zwei Übersetzungssysteme bedeuten
  zwei Dateiformate, zwei Review-Workflows und zwei Gelegenheiten
  für denselben Satz, auseinanderzudriften. Suprnova liefert genau
  den Katalog aus, den der Server unter
  `/_suprnova/lang/<locale>.ftl` aufgelöst hat, und die
  Starter-Kits parsen ihn mit `@fluent/bundle` - ein Satz Dateien,
  eine Wahrheitsquelle.
- **Plurale und Formate sind CLDR-Daten, keine
  String-Konkatenation.** Englisch hat zwei Pluralkategorien,
  Russisch und Polnisch vier, Arabisch sechs. Eine Zahl ist
  `1,234.56` in `en-US` und `1.234,56` in `de-DE`. Fluent wählt
  anhand von CLDR-Pluralkategorien aus, und ICU4X übernimmt die
  Formatierung, sodass Sie keins von beidem pro Locale von Hand
  bauen müssen.

Das Feature auszuschalten (`--no-default-features`) wird
unterstützt: Das Lokalisierungsmodul wird nicht kompiliert, und die
Validierung rendert ihre eingebetteten englischen
Fallback-Strings. Sonst ändert sich an der Form nichts.

## Dateilayout

Kataloge leben unter `lang/`, ein Verzeichnis pro Locale:

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

Die Regeln:

- **Ein Verzeichnisname ist ein BCP-47-Locale** - `en`, `en-GB`,
  `pt-BR`, `zh-Hans`. Ein Verzeichnis, dessen Name sich nicht
  parsen lässt, wird mit einem `warn!` übersprungen, statt den
  Boot fehlschlagen zu lassen.
- **Jede `.ftl`-Datei in einem Locale-Verzeichnis fließt in einen
  einzigen Katalog ein**, in sortierter Dateinamen-Reihenfolge.
  Teilen Sie sie nach Belieben nach Feature auf (`auth.ftl`,
  `billing.ftl`, `emails.ftl`) - Message-IDs sind innerhalb des
  Locale global, also dürfen `auth.ftl` und `billing.ftl` nicht
  dieselbe ID definieren.
- **Der eigene englische Validierungskatalog des Frameworks lädt
  zuerst**, in das Bundle jedes Locale. Ihre Dateien laden
  darüber, und eine spätere Definition gewinnt. Das ist der
  gesamte Override-Mechanismus: Definieren Sie `validation-min` in
  `lang/es/validation.ftl`, und das spanische Bundle nutzt Ihre.
- **Die Wurzel ist `lang_path()`** - `<APP_BASE_PATH>/lang`.
  Setzen Sie `APP_BASE_PATH`, wenn die Binary von einem anderen
  Ort als dem Projekt-Root aus läuft (eine systemd-Unit, ein
  Container mit einem anderen `WorkingDirectory`), oder rufen Sie
  `use_lang_path("…")` auf, um nur das `lang`-Verzeichnis zu
  verschieben. Siehe [Umgebungsvariablen](env-vars.md).
- **Ein fehlendes `lang/`-Verzeichnis ist kein Fehler.** Eine
  frische App muss booten, also kommt der Translator mit dem
  eingebetteten englischen Katalog hoch und mit sonst nichts. Eine
  *fehlerhafte* `.ftl`-Datei ist eine andere Geschichte:
  Parse-Fehler lassen den Boot fehlschlagen und nennen die Datei
  und woran der Parser sich gestört hat, weil ein stillschweigend
  halb geladener Katalog schlimmer ist als ein gestoppter Prozess.
- **In `local` und `development` machen Kataloge Hot-Reload.**
  Jede Anfrage prüft `lang/` per `stat` und parst nur neu, wenn
  sich tatsächlich etwas geändert hat, sodass das Bearbeiten einer
  `.ftl`-Datei beim nächsten Refresh sichtbar wird. Produktion
  prüft nie erneut; Kataloge werden einmal beim Boot gelesen.

## FTL in fünf Minuten

Fluent ist ein kleines Format. Dieser Abschnitt ist alles, was Sie
für eine typische App brauchen.

**Messages** sind `id = value`-Paare. IDs sind per Konvention
kebab-case (die des Frameworks selbst sind es), Werte laufen bis
zum Zeilenende, und eingerückte Fortsetzungszeilen werden
zusammengefügt:

```ftl
# Ein Kommentar. Der Nachricht darunter zugeordnet.
sign-in = Anmelden
password-hint =
    Verwenden Sie mindestens 12 Zeichen. Eine Passphrase aus ein
    paar gewöhnlichen Wörtern schlägt eine kurze Zeichenfolge aus
    Symbolen.
```

**Argumente** sind `{ $name }`-Placeables. Sie liefern sie beim
Aufruf; fehlende Argumente sind ein Fehler, keine leere
Zeichenkette (`Lang::get` fällt dann durch seine Kette durch -
siehe [Die `Lang`-Facade](#die-lang-facade)):

```ftl
greeting = Hallo, { $name }!
invoice-line = { $qty } × { $item }
```

**Terms** beginnen mit `-`, sind privat zum Katalog, und
existieren, damit ein Markenname oder eine wiederholte Phrase an
einem Ort lebt:

```ftl
-product-name = Suprnova
about = Über { -product-name }
footer = © 2026 { -product-name }. Alle Rechte vorbehalten.
```

**Selektoren** sind Fluents Bedingung. Der Selektor-Wert wird
gegen Varianten-Schlüssel abgeglichen; genau eine Variante wird
mit `*` als Standard markiert:

```ftl
cart-summary =
    { $count ->
        [0] Ihr Warenkorb ist leer.
        [one] Ein Artikel in Ihrem Warenkorb.
       *[other] { $count } Artikel in Ihrem Warenkorb.
    }
```

`[0]` matcht die literale Zahl null. `[one]` und `[other]` sind
**CLDR-Pluralkategorien**, aufgelöst für das Locale des Bundles -
und genau da verdient sich Fluent seinen Platz. Englisch hat zwei
Kategorien; Russisch hat vier, und ein russischer Übersetzer
schreibt alle vier, ohne dass Sie eine Zeile Rust ändern:

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

CLDR weist `1`, `21`, `31` `one` zu; `2`-`4`, `22`-`24` `few`; `0`,
`5`-`20`, `25`-`30` `many`; und Brüche `other`. Derselbe Aufruf
`__!("unread-messages", count: 22)` rendert korrekt in Englisch,
Russisch, Polnisch und Arabisch, weil die Kategorie-Auswahl Daten
sind, kein Code.

**Setzen Sie das `*` immer auf `other`.** Es ist die eine
Kategorie, die CLDR für jedes Locale definiert, also ist es die
einzige Variante, die garantiert existiert - und der Standard ist
das, worauf ein nicht passender Selektor-Wert durchfällt,
einschließlich jeder Nicht-Ganzzahl-Anzahl. `*[many]` (oder jede
andere Kategorie) als Standard zu markieren, schickt Brüche zu
Text, der für ganze Zahlen geschrieben wurde.

> **Übergeben Sie Zählwerte als Zahlen.**
> `__!("unread-messages", count: 3)` sendet eine JSON-Zahl und
> wählt eine Pluralkategorie aus. `count: "3"` sendet eine
> Zeichenkette, die nur mit einem literalen Varianten-Schlüssel
> matchen kann - sie landet auf Ihrem `*[other]`-Standard. Das ist
> die eine FTL-Falle, die man sich merken sollte.

**Funktionen** werden innerhalb von Placeables aufgerufen. Zwei
sind registriert: `NUMBER()` (Fluents eingebaute) und `DATETIME()`
(Suprnovas):

```ftl
score = Ihr Punktestand ist { NUMBER($points) } von { NUMBER($total) }.
published = Veröffentlicht { DATETIME($when, dateStyle: "medium") }
```

Siehe [Locale-bewusste Formatierung](#locale-bewusste-formatierung)
für beide.

**Eine bewusste Einschränkung:** Suprnova löst nur flache
Message-*Werte* auf. Fluents Attribut-Syntax (`login .placeholder =
…`) wird geparst, ist aber nicht über `Lang::get` adressierbar,
behalten Sie also eine ID pro Zeichenkette: `login-placeholder`,
nicht `login.placeholder`. IDs sind ein flacher Namensraum pro
Locale - versehen Sie sie mit einem Präfix (`auth-login-title`,
`billing-invoice-due`), statt nach einer Hierarchie zu greifen, die
der Resolver nicht hat.

## Die `Lang`-Facade

`Lang` ist der serverseitige Einstiegspunkt. Jede Methode liest
das **aktuelle Locale**, das die Middleware für diese Anfrage
gebunden hat.

| Methode | Gibt zurück | Hinweise |
|---|---|---|
| `Lang::get(key)` | `String` | Unfehlbar. Läuft die Fallback-Kette, gibt dann den Schlüssel selbst zurück |
| `Lang::get_with(key, args)` | `String` | Dasselbe, mit Argumenten |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | Meldet Fehler, statt zu degradieren |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | Dasselbe, mit Argumenten |
| `Lang::has(key)` | `bool` | Ob der Schlüssel für das aktuelle Locale auflöst, oder irgendwo entlang seiner Fallback-Kette |
| `Lang::locale()` | `Locale` | Das aktuelle Locale |
| `Lang::set_locale(locale)` | `()` | Ändert es für den Rest dieser Anfrage |
| `Lang::available_locales()` | `Vec<Locale>` | Jedes Locale mit geladenem Katalog |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // Nur manche Locales liefern den Banner-Text aus.
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` ist eine geordnete Map von `String` auf
`serde_json::Value`, beide an der Crate-Wurzel re-exportiert.
Fluent-Argumente sind Zeichenketten und Zahlen; andere JSON-Formen
werden zu Zeichenketten.

### Die Fallback-Kette

`Lang::get` schlägt nie fehl, und es gibt nie eine leere
Zeichenkette zurück. In dieser Reihenfolge:

1. Der Katalog des **aktuellen Locale**.
2. Seine **konfigurierten Fallback-Eltern** (siehe
   [Fallback-Ketten](#fallback-ketten)), transitiv durchlaufen,
   falls welche konfiguriert sind - `pt-PT` vor `pt-BR` vor dem,
   was `pt-BR` selbst als Elternteil nennt, und so weiter.
3. Der Katalog des **Fallback-Locale** (`APP_FALLBACK_LOCALE`,
   Standard `en`), außer es ist schon früher in dieser Kette
   aufgetaucht.
4. Der **Schlüssel selbst**, plus ein `tracing::warn!` pro
   fehlendem `(locale, key)`-Paar - einmal, nicht einmal pro
   Anfrage, damit ein fehlender Schlüssel in einem Hot-Path Ihre
   Logs nicht ertränkt.

Schritt 4 ist, warum eine fehlende Übersetzung `checkout-submit` im
Button rendert statt eines leeren Buttons: eine sichtbar falsche
Zeichenkette ist ein Bug-Report, der nur darauf wartet zu
passieren, während eine leere ein Rätsel ist.

Wenn Sie es lieber wissen als degradieren wollen, nutzen Sie die
`try_*`-Geschwister. Sie laufen die Schritte 1 bis 3 und geben
`Err` zurück, statt Schritt 4 auszuführen:

```rust
use suprnova::Lang;

// Ein fehlender Schlüssel hier bedeutet eine kaputte Mail - lassen
// Sie den Job fehlschlagen, senden Sie keine Nachricht mit einem
// rohen Schlüssel in der Betreffzeile.
let subject = Lang::try_get("invoice-paid-subject")?;
```

### Das `__!`-Makro

`__!` ist die Laravel-Muskelgedächtnis-Abkürzung. Ohne Argumente
ruft es `Lang::get` auf; mit benannten Argumenten baut es ein
`TranslateArgs` und ruft `Lang::get_with` auf:

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

Argument-Werte sind alles, was sich in einen `serde_json::Value`
konvertiert - `&str`, `String`, Integer, Floats, `bool`. Das Makro
wird an der Crate-Wurzel exportiert, also funktioniert
`suprnova::__!("welcome-back")` ohne den Import, wenn Sie `__`
lieber nicht in den Scope holen wollen.

## Fallback-Ketten

`APP_FALLBACK_LOCALE` ist ein globales Netz unter jedem Locale.
Manchmal reicht das nicht: Europäisches Portugiesisch und
brasilianisches Portugiesisch teilen sich fast alles und weichen
bei einer Handvoll Wörter voneinander ab (`ficheiro`/`arquivo`,
`utilizador`/`usuário`, `tu`/`você`), und zwei vollständige
Kataloge zu pflegen bedeutet, dass jede neue Zeichenkette zweimal
geschrieben werden muss. Ein **Fallback-Elternteil** lässt `pt-PT`
von `pt-BR` erben, bevor `pt-BR` weiter zurück auf das globale
`fallback_locale` fällt - sodass `lang/pt-PT/` nur die
Zeichenketten halten muss, die tatsächlich anders sind.

### Fallback-Eltern konfigurieren

Eine Umgebungsvariable, kommagetrennte `child=parent`-Paare:

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Oder der Builder, ein Aufruf pro Paar, verkettbar:

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

Beide Wege füttern dieselbe Map (`LocalizationConfig::parents`),
und beide werden beim Boot validiert, nicht zur Anfragezeit:

- Ein Paar ohne `=`, oder ein leeres Kind oder Elternteil, ist ein
  fehlerhafter `APP_LOCALE_PARENTS`-Eintrag - der Boot schlägt
  fehl und nennt das kaputte Segment.
- Ein Locale, das auf einer der beiden Seiten des Paars als BCP-47
  ungültig ist, schlägt auf dieselbe Weise fehl.
- Dasselbe Kind zweimal zu nennen ist mehrdeutige Config, nicht
  „letzter gewinnt“ - der Boot schlägt fehl und nennt das doppelte
  Kind.
- **Ein Zyklus lässt den Boot fehlschlagen.** Der Fehler
  buchstabiert den Zyklus aus: zwei Locales, die sich gegenseitig
  nennen (`pt-PT=pt-BR,pt-BR=pt-PT`), erzeugen
  `` `pt-PT` -> `pt-BR` -> `pt-PT` ``. Ein Locale, das sich selbst
  als eigenes Elternteil nennt (`pt-PT=pt-PT`), ist derselbe Fall
  im Kleinen - `` `pt-PT` -> `pt-PT` ``. (Zwei Codepfade werfen
  diesen Fehler: das Parsen von `APP_LOCALE_PARENTS` - sodass jede
  App, deren Config über `LocalizationConfig::from_env()` läuft,
  schon beim Config-Laden fehlschlägt - und der Katalog-Load von
  `FluentTranslator`, der eine zyklische, programmatisch mit
  `.parent(...)` gebaute Map abfängt. Nur eine App, die ihre
  Config vollständig von Hand baut *und* ihren eigenen
  `Translator` in `bootstrap_fn` bindet, umgeht beide; der Walk von
  `Lang` ist unabhängig abgesichert und terminiert dort trotzdem
  sicher, bekommt nur nicht den lauten Boot-Zeit-Fehler.)

Das `.parent(child, parent)` des Builders ist „letzter Schreiber
gewinnt“ für ein wiederholtes Kind - ein späterer Aufruf, der einen
früheren überschreibt, ist nur ein späterer Override, nicht der
mehrdeutige Eingabefall, gegen den sich `APP_LOCALE_PARENTS`
absichert.

### Auflösungsreihenfolge

Eine Kette kann mehr als einen Hop lang sein: `pt-PT` nennt
`pt-BR` als sein Elternteil, und `pt-BR` kann wiederum ein eigenes
Elternteil nennen. `Lang::get` / `try_get` / `get_with` /
`try_get_with` / `has` durchlaufen das Ganze alle, aktuelles
Locale zuerst:

1. Der Katalog des **aktuellen Locale**.
2. Sein **konfiguriertes Elternteil**, dann das konfigurierte
   Elternteil *dieses* Locale, transitiv, bis ein Locale ohne
   konfiguriertes Elternteil erreicht ist.
3. Das globale **`fallback_locale`** (`APP_FALLBACK_LOCALE`),
   außer es ist schon früher in der Kette aufgetaucht -
   einschließlich des üblichen Falls, in dem es einfach das
   aktuelle Locale selbst ist (der `en`/`en`-Standard).

`Lang::get` / `Lang::get_with` fallen durch bis zum Schlüssel
selbst, wenn nichts in der Kette ihn auflöst, genau wie [Die
Fallback-Kette](#die-fallback-kette) es beschreibt; `Lang::try_get`
/ `Lang::try_get_with` geben `Err` zurück, und `Lang::has` gibt
`false` zurück. Dieser Walk läuft innerhalb der `Lang`-Facade
selbst, also funktioniert er für **jeden** `Translator` - den
mitgelieferten `FluentTranslator`, oder einen Treiber, den Sie
schreiben.

### Ein lauffähiges Beispiel

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

// Eine Anfrage, die zu `pt-PT` aufgelöst wurde.
assert_eq!(__!("file-label"), "Ficheiro");                    // pt-PTs eigener Override
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // von pt-BR geerbt
);
```

`lang/pt-PT/` definiert `welcome` nie - es braucht es nicht.
`file-label` ist ein echter Ein-Wort-Unterschied zwischen den
beiden Katalogen, also ist es die einzige ID, die eine Datei
bekommt.

### Ausgelieferte Kataloge sind geflacht

Der Endpunkt `/_suprnova/lang/pt-PT.ftl` (siehe [Der
Katalog-Endpunkt](#der-katalog-endpunkt)) verlangt vom Browser nie,
zu wissen, dass `pt-BR` existiert. `FluentTranslator` merged die
gesamte Kette beim Ladezeitpunkt vorab in eine Ressource pro
Locale - der eingebettete Framework-Katalog ganz unten für
`en`-/`en-*`-Locales, dann die konfigurierte Elternkette, dann die
eigenen Dateien des Locale - und liefert *das* aus, bereits
geflacht. Rufen Sie `pt-PT.ftl` ab, und die Antwort trägt sowohl
`welcome` als auch `file-label`, in einer Anfrage, ohne
clientseitige Kettenlogik. `?v=<hash>` benennt weiterhin eine
einzige unveränderliche Ressource; der Hash deckt jetzt einfach
auch Zeichenketten ab, die aus `pt-BR` hereingezogen wurden.

**Flachen deckt nur konfigurierte Eltern ab** - es reicht nie über
sie hinaus zu `fallback_locale`. Der ausgelieferte Katalog von
`pt-PT` enthält die Zeichenketten von `pt-BR`, weil `pt-BR` ein
*konfiguriertes Elternteil* ist; er enthält nicht die
Zeichenketten von `en`, nur weil `en` zufällig das globale
Fallback ist. Das `fallback`-Feld von `LocaleShare` nennt immer
das terminale `fallback_locale`, davon unberührt - es sagt dem
Frontend, wo der Walk auf Facade-Ebene von `Lang` letztlich landen
würde, nicht, was schon in der Datei steckt, die es gerade
abgerufen hat.

### Merge-Regeln für Delta-Dateien

Ein Kind-Katalog merged über sein Elternteil **auf Ebene des
Fluent-AST**, nicht durch textuelle Konkatenation und nicht durch
Shadowing ganzer Messages. Die Override-Einheit ist das *Pattern*,
also:

- **Ein Kind-Wert ersetzt den Wert des Elternteils**, an der
  Position des Elternteils in der Datei.
- **Ein Kind-Eintrag mit Attributen, aber ohne Wert, behält den
  Wert des Elternteils.** `.placeholder` neu zu übersetzen
  erfordert nicht, den eigenen Text der Message zu wiederholen.
- **Attribute mergen nach Namen.** Ein gleichnamiges
  Kind-Attribut ersetzt das des Elternteils, an Ort und Stelle;
  ein nur-beim-Kind-vorhandenes Attribut hängt sich nach dem des
  Elternteils an. **Attribute, die das Kind nicht erwähnt,
  überleben vom Elternteil** - den Wert einer Message zu
  überschreiben lässt niemals stillschweigend ihr `.placeholder`
  oder `.aria-label` fallen.
- **Select-Ausdrücke werden als Ganzes ersetzt, nie
  Variante-für-Variante.** Die Varianten eines Selektors sind an
  die CLDR-Pluralkategorien eines Locale gebunden; weil diese
  Kategorien locale-abhängig sind, könnte das Zusammenstückeln
  einer Variante vom Elternteil und einer anderen vom Kind einen
  Selektor produzieren, hinter dem die Grammatik keines einzigen
  Locale steckt. Ein Kind, das einen Selektor überhaupt
  überschreibt, muss jede Variante liefern, die es will.
- **Kommentare auf einem überschriebenen Eintrag bleiben die des
  Elternteils.** Der Kommentar dokumentiert die ID, und die
  Override-Einheit ist das Pattern, nicht der Kommentar.
- **Nur-beim-Kind-vorhandene Einträge hängen sich am Ende an**, in
  der eigenen Reihenfolge des Kindes, Kommentare eingeschlossen -
  eine ID, die `pt-BR` nie definiert hat, ist kein „Override“ von
  irgendetwas.

Terms (`-brand`) folgen derselben Regel, mit einer Einschränkung:
Der Wert eines Terms ist in der Fluent-Syntax nie optional, also
gilt der obige Fall „Attribute-aber-kein-Wert-behält-den-Eltern-
Wert“ nur für Messages - ein Kind-Term liefert immer einen Wert,
und dieser Wert gewinnt immer. Attribut-Merge-nach-Namen,
Ganz-Pattern-Ersetzung für den Wert und
Eltern-gewinnt-bei-Kommentaren gelten für Terms genau wie für
Messages. Terms werden in ihrem eigenen Namensraum verfolgt -
`-brand` zu überschreiben kann niemals eine Message shadowen, die
ebenfalls `brand` heißt.

### Warum Suprnova abweicht

Laravel 13 hat genau ein Fallback: den einzigen globalen
Config-Wert `fallback_locale`, konsultiert, wenn dem Array des
aktuellen Locale ein Schlüssel fehlt. Es gibt kein Konzept, in dem
ein Locale von einem Geschwister-Locale erbt - `pt_PT.php` und
`pt_BR.php` sind zwei unabhängige Arrays, und eine `pt_PT`-App
dupliziert entweder alles, was `pt_BR` bereits übersetzt hat, oder
liefert ohne es aus.

Suprnovas Elternketten sind die Rust-seitige Erweiterung: ein
Zwischenschritt zwischen „diesem Locale“ und „dem globalen
Fallback“, konfiguriert pro Locale statt einmal global. Der
Tradeoff, den wir nicht eingehen wollten, ist, diese Komplexität
auf den Browser abzuwälzen - ein kettenbewusstes Frontend müsste
`pt-PT.ftl` abrufen, feststellen, dass es unvollständig ist, auch
`pt-BR.ftl` abrufen, und sie clientseitig in JavaScript mergen,
mit Regeln, die exakt zu denen des Servers passen müssten.
Stattdessen bedeutet Flachen beim Ladezeitpunkt, dass der
ausgelieferte Katalog immer eine vollständige, in sich
geschlossene Datei ist - derselbe Vertrag, den das Frontend schon
hatte, bevor es Elternketten gab, sodass `@fluent/bundle` und die
Kit-Wrapper null Änderungen brauchten, um dieses Feature zu
unterstützen.

## Locale-Erkennung

`LocaleMiddleware` löst pro Anfrage ein Locale auf und bindet es
für die Dauer des Handlers. Die Kette ist config-gesteuert, und
**der erste Treffer gewinnt**:

1. **Session** - der Schlüssel `locale` in der Session, falls die
   [Session-Middleware](session.md) lief und der Wert ein
   verfügbares Locale benennt. Hier lebt „Nutzer hat Español in
   den Einstellungen gewählt“.
2. **Cookie** - das Cookie `locale`. Überlebt den Logout, sodass
   eine vor dem Anmelden getroffene Sprachwahl nicht verloren
   geht.
3. **`Accept-Language`** - ausgehandelt gegen
   `available_locales()` mit `fluent-langneg`, unter Beachtung
   der q-Werte. `fr-CH, es;q=0.8, en;q=0.5` gegen die Kataloge
   `en` + `es` löst zu `es` auf.
4. **`APP_LOCALE`** - der konfigurierte Standard, wenn oben nichts
   getroffen hat.

Ein Kandidat, der sich nicht parsen lässt, oder ein Locale ohne
Katalog benennt, wird **übersprungen, nicht abgelehnt**. Ein
Nutzer mit einem veralteten `locale=zz`-Cookie sieht die
Standardsprache, kein 500er. Ein kaputter `Accept-Language`-Header
macht dasselbe. Angreiferkontrollierte Eingabe erreicht diese
Kette bei jeder Anfrage; sie darf nie mehr können, als eine
Sprache auszuwählen.

Verdrahten Sie es in `bootstrap.rs`, **nach** der
Session-Middleware, da Schritt 1 die Session liest:

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // Löst das Locale auf und bindet es für die Anfrage.
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // Gibt dem Frontend sein Locale + die Katalog-URL auf jeder Inertia-Seite mit.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` liest `LocalizationConfig::from_env()`;
`LocaleMiddleware::new(config)` nimmt eine, die Sie selbst gebaut
haben. Eine gescaffoldete App hat beide Zeilen schon.

### Das Locale mitten in der Anfrage wechseln

`Lang::set_locale` ist Laravels `App::setLocale` - es schreibt das
Locale der aktuellen Anfrage ab diesem Punkt neu:

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// Der Nutzer hat gerade in einem Einstellungsformular die Sprache gewechselt.
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // diese Anfrage
    session_mut(|s| s.put("locale", choice));       // jede Anfrage danach
    Ok(())
}
```

Beachten Sie die zwei Hälften: `set_locale` betrifft *diese*
Anfrage (sodass die Flash-Message des Redirects schon auf
Spanisch ist), und das Session-Schreiben ist das, was die
Erkennungskette bei der *nächsten* liest.

### Außerhalb einer Anfrage

Konsolenbefehle, Queue-Worker und geplante Tasks haben keine
Anfrage und keine Middleware. Dort schreibt `Lang::set_locale`
einen prozessglobalen Override, den `Lang::locale()` konsultiert,
bevor es auf `APP_LOCALE` zurückfällt:

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // Die gespeicherte Präferenz jedes Nutzers, für die Dauer seiner E-Mail.
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

Weil dieser Override prozessweit statt task-lokal ist, setzen Sie
ihn wie oben am Anfang jeder Arbeitseinheit - verlassen Sie sich
nicht darauf, dass er über ein `.await` hinweg unverändert bleibt,
mit dem sich ein anderer Task verschachteln könnte.

## Konfiguration

Drei Umgebungsvariablen. `APP_LOCALE` und `APP_FALLBACK_LOCALE`
haben beide `en` als Standard; `APP_LOCALE_PARENTS` ist
standardmäßig leer - keine Pro-Locale-Overrides, nur
`fallback_locale` greift:

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Alles andere ist Code, auf `LocalizationConfig`. Es registriert
sich wie jede andere typisierte Config - in Ihrem
`config::register_all`, das vor dem Boot läuft:

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // siehe die Abweichungs-Notiz
        .detection(vec![Detect::Session, Detect::Header])   // Cookie ignorieren
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // siehe Fallback-Ketten
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - überschreiben
  `APP_LOCALE` und `APP_FALLBACK_LOCALE` aus dem Code. Ein
  fehlerhafter Wert an beiden Stellen lässt den Boot fehlschlagen,
  statt stillschweigend zu `en` zu werden.
- `use_isolating` - Unicode-Isolationsmarken um Interpolationen.
  Standardmäßig aus; schalten Sie es ein, wenn Sie ein
  RTL-Locale ausliefern.
- `detection` - die Kette, in Reihenfolge. `Detect::Cookie`
  wegzulassen bedeutet, dass eine Sprachwahl nur in der Session
  lebt; `Detect::Header` wegzulassen bedeutet, dass die Präferenz
  des Browsers völlig ignoriert wird.
- `session_key` / `cookie_name` - benennen die beiden Lookups um.
- `parents` - Pro-Locale-Fallback-Eltern (`child -> parent`),
  durchlaufen vor `fallback_locale`, wenn ein Schlüssel im
  Katalog des Kindes fehlt; gleiche Form wie
  `APP_LOCALE_PARENTS`. Fügen Sie eins mit `.parent(child,
  parent)` hinzu - verkettbar, letzter Schreiber gewinnt bei
  einem wiederholten Kind. Siehe
  [Fallback-Ketten](#fallback-ketten) für den vollständigen
  Vertrag (Boot-Zeit-Validierung, Auflösungsreihenfolge, Flachen
  des ausgelieferten Katalogs).

Der Boot bindet ein `Arc<dyn Translator>` in den Container. Wenn
Ihre App schon eins gebunden hat, lässt das Framework es in Ruhe -
so ersetzen Sie einen eigenen Translator, ohne irgendetwas zu
forken:

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

`Translator` ist die Erweiterungsnaht: `translate`, `has`,
`available_locales`, `catalog`, `reload`. Ein Treiber wird
ausgeliefert (`FluentTranslator`), und ein neues Backend ist ein
neuer Treiber - kein Fork der Oberfläche.

## Übersetzte Validierungsmeldungen

Jede eingebaute Regel gibt eine **keyed** Message zurück: einen
Katalog-Schlüssel, die Argumente, die die Message braucht, und ein
englisches Fallback. Übersetzung passiert einmal, an der
Serialisierungsgrenze - `ValidationErrors::to_json` und die
Inertia-Error-Bag - nie innerhalb der Regel. Regeln bleiben pur,
und das gesamte Subsystem kompiliert sich weg.

Die Schlüssel folgen einer Konvention:

| Form | Beispiel | Wofür |
|---|---|---|
| `validation-<rule>` | `validation-min`, `validation-required-if` | Eine pro eingebauter Regel, kebab-case |
| `field-<name>` | `field-email` | Ein menschenlesbarer Name für ein Feld |
| `validation-invalid-data` | - | Das übergeordnete Banner „The given data was invalid.“ |

Um sie zu übersetzen, definieren Sie die IDs, die Ihnen wichtig
sind, in einer beliebigen `.ftl`-Datei unter dem Ziel-Locale:

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` ist immer verfügbar. Die eigenen Parameter jeder Regel
werden unter den Namen übergeben, die sie im englischen Katalog
des Frameworks tragen - `$min`, `$max`, `$other`, `$value` - und
`framework/src/localization/catalogs/en/validation.ftl` ist die
kanonische Liste der IDs und Argumente. Kopieren Sie sich die IDs
heraus, die Sie brauchen; Sie müssen nie alle davon überschreiben.

Überschreiben funktioniert pro Locale und pro Schlüssel.
`validation-min` in `lang/en/validation.ftl` zu definieren, ersetzt
den englischen Wortlaut des Frameworks für diese eine Regel und
lässt den Rest in Ruhe.

### Feldnamen

Einen rohen Spaltennamen zu interpolieren produziert „The
email_address field is required.“ Die Konvention `field-<name>`
behebt das:

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

Vor dem Rendern schlägt der Translator `field-<name>` für das
aktuelle Locale nach. Ein Treffer wird als `$field` übergeben; ein
Fehltreffer fällt zurück auf den Feldnamen mit Unterstrichen, die
zu Leerzeichen werden. Die Datei oben wird also nur für die Namen
gebraucht, die sich schlecht humanisieren.

### Eigene Regeln

`Rule::passes` gibt `Result<(), ValidationMessage>` zurück. Eine
keyed Message nimmt an der Übersetzung teil:

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

Eine reine Zeichenkette funktioniert weiterhin und ist die
richtige Antwort für eine Message, die immer nur in einer Sprache
existieren wird:

```rust
Err("must start with acct_".into())   // keyless: wörtlich gerendert
```

Keyless Messages überspringen die Übersetzung komplett, was dafür
sorgt, dass bestehende eigene Regeln weiter genau wie vorher
kompilieren und sich verhalten.

### Der Derive-Flow

Fehler von `#[derive(Validate)]` sind auch keyed. Der Fehlercode
der `validator`-Crate wird zu `validation-<code>`, mit
Unterstrichen, die zu Bindestrichen werden, und jeder Parameter,
den der Validator anhängt, wird zu einem Message-Argument - mit
zwei reservierten Ausnahmen, `value` und `other`, die immer
verworfen werden. Beide tragen den tatsächlichen *Wert* eines
Felds statt Metadaten über die Regel: `value` ist die
zurückgespiegelte geprüfte Eingabe, und `other` (gesetzt von
`must_match`, der kanonischen Passwort-Bestätigungsregel) ist der
Wert des Nachbarfelds. Keins von beiden wird je an den Katalog
übergeben, sodass kein `.ftl`-Override - wie auch immer er
`validation-must-match` formuliert - ein eingereichtes Geheimnis
in einen 422-Response-Body interpolieren kann. Ein
`#[validate(email)]`-Fehlschlag löst also `validation-email` genau
wie die handgeschriebene Regel auf, und ein Locale, das eins
übersetzt, übersetzt beide.

## Das Frontend

Der Browser bekommt dieselben Bytes, die der Server aufgelöst hat.
Nichts wird neu übersetzt, neu exportiert, oder von Hand synchron
gehalten.

### Der Katalog-Endpunkt

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 wenn If-None-Match übereinstimmt
GET /_suprnova/lang/zz.ftl              → 404 (kein solcher Katalog)
```

Der Body ist der gemergte Katalog für dieses Locale -
Framework-Messages zuerst, dann seine konfigurierte
Fallback-Elternkette, falls vorhanden (siehe
[Fallback-Ketten](#fallback-ketten)), dann Ihre Dateien in
Lade-Reihenfolge. `ETag` ist der Content-Hash. Fragen Sie mit
`?v=` nach einem bestimmten Hash, und die Antwort ist für immer
unveränderlich-cachebar, weil diese URL nur je eine Sache
bedeuten kann; fragen Sie ohne, bekommen Sie stattdessen
Revalidierung. Wie `/_suprnova/health` ist der Pfad von der
Middleware-Kette ausgenommen: Er muss antworten, bevor ein Locale
aufgelöst wurde, und er trägt keine Nutzerdaten.

### Die Shared Prop

`LocaleShare` ist ein `InertiaSharedData`, das das Framework
ausliefert. In `bootstrap.rs` registriert (siehe
[Locale-Erkennung](#locale-erkennung)), fügt es jeder
Inertia-Seite eine Prop hinzu:

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

`catalog` ist `null`, wenn kein Translator gebunden ist - die
Share lässt ein Seiten-Rendering nie fehlschlagen.

### Die Kit-Wrapper

Jedes Starter-Kit liefert einen ~100-Zeilen-Wrapper aus, der
diese Prop liest, den Katalog einmal abruft, ein
`@fluent/bundle`-Bundle baut, und `t()` exponiert. Rufen Sie
`initLang` einmal in Ihrem Inertia-Einstiegspunkt auf
(gescaffoldete Apps tun das schon):

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* … unverändert … */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

Dann, in Komponenten:

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

Zahlen- und Datumsformatierung auf dem Client nutzt das
eingebaute `Intl` des Browsers - keine ICU-Daten werden an den
Browser ausgeliefert.

### Typisierte Message-Keys

`suprnova generate-types` parst `lang/<default locale>/*.ftl` und
gibt eine Union jeder Message-ID neben den Page-Props-Typen aus:

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

Die Wrapper typisieren `t(key: MessageKey, …)`, das ist also
dasselbe Versprechen wie
[`inertia-props.ts`](frontend-typescript-types.md): Eine Message
in Rust umbenennen, neu generieren, und der TypeScript-Compiler
zeigt auf jede Aufrufstelle, die noch die alte ID benutzt.
`suprnova serve` beobachtet `lang/` neben `src/`, sodass sich die
Datei neu generiert, während Sie Kataloge bearbeiten.

Ein Projekt ohne `lang/`-Verzeichnis und ohne Message-IDs bekommt
**keine Datei** - eine App, die nicht lokalisiert ist, sieht kein
neues Artefakt erscheinen.

## Locale-bewusste Formatierung

Sieben Funktionen auf `Lang`, alle ICU4X-gestützt, alle lesen das
aktuelle Locale, alle mit `try_*`-Geschwistern, die
`Result<String, FrameworkError>` zurückgeben statt zu degradieren:

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

Die Style-Enums: `DateStyle { Full, Long, Medium, Short }`,
`TimeStyle { Medium, Short }`, `ListStyle { And, Or, Unit }`,
`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`.
`Lang::relative` nimmt einen vorzeichenbehafteten Betrag - negativ
ist die Vergangenheit („3 days ago“), positiv die Zukunft („in 3
days“).

> Die exakte Ausgabe kommt aus den CLDR-Daten, die in ICU4X
> eingebacken sind, und kann sich über ein ICU-Upgrade hinweg
> ändern, besonders bei Daten und Währungen. Assertieren Sie in
> Ihren eigenen Tests auf Form und Locale-Unterschiedlichkeit
> (`de != en`, enthält `2026`), statt auf exakte Bytes.

### Formatierung innerhalb einer Nachricht

Zwei Funktionen sind aus FTL aufrufbar:

```ftl
order-total = Ihr Gesamtbetrag ist { NUMBER($amount, maximumFractionDigits: 2) }.
published = Veröffentlicht { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` ist Fluents eingebaute, explizit registriert, und gibt
Ihnen Kontrolle über Nachkommastellen innerhalb der Nachricht.
`DATETIME()` ist Suprnovas: `$value` akzeptiert eine
ISO-8601-Zeichenkette oder Epoch-Millisekunden, und `dateStyle` /
`timeStyle` nehmen dieselben Namen wie die Rust-Enums, klein
geschrieben. Ein Wert, den sie nicht parsen kann, läuft wörtlich
durch mit einem `warn!` - eine Fluent-Funktion kann keinen Fehler
zurückgeben, und eine gerenderte Seite mit einem seltsam
aussehenden Datum schlägt einen 500er.

Wenn Sie ICU4Xs volle Formatierung wollen statt dessen, was eine
Fluent-Funktion bietet, formatieren Sie in Rust und übergeben Sie
die fertige Zeichenkette:

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## Ihre Übersetzungen testen

Zwei Helfer erledigen die Arbeit: `use_lang_path` zeigt den
Loader auf ein Fixture-Verzeichnis, und `scope_locale` pinnt das
aktuelle Locale für die Dauer eines Future.

Die hermetische Form - einen Translator über einem
Fixture-Verzeichnis bauen und ihn in einem test-scoped Container
binden - ist das, was die eigenen Tests des Frameworks benutzen,
weil sie keinen prozessglobalen Zustand berührt und parallele
Testausführung übersteht:

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

`use_lang_path` ist das richtige Werkzeug, wenn der Test die echte
Anwendung bootet und Sie die *ganze* App auf Fixtures zeigen
lassen wollen:

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // … App booten; `lang_path("")` löst jetzt in das Fixture-Verzeichnis auf.
}
```

Es schreibt einen prozessglobalen Pfad-Override, behandeln Sie es
also als Pro-Binary-Einstellung statt als etwas, worüber zwei
parallele Tests uneins sein können.

Die Erkennung selbst - die Kette
Session/Cookie/`Accept-Language` - lohnt es sich, durch die echte
Pipeline zu testen statt durch direktes Aufrufen der Middleware,
weil die interessanten Fälle um Header-Parsing gehen und darum,
welche Quelle gewinnt. Mounten Sie
eine Route, deren Handler `__!("welcome")` zurückgibt,
registrieren Sie `LocaleMiddleware` in der `MiddlewareRegistry`,
und steuern Sie sie mit dem Loopback-Harness aus
[HTTP-Tests](http-tests.md) an, indem Sie `Accept-Language: fr,
es;q=0.8` senden und auf den spanischen Body assertieren. Die
Fälle, die es wert sind, festgenagelt zu werden: ein Header
handelt aus, ein Cookie schlägt einen Header, ein nicht
verfügbares Locale wird übersprungen statt einen Fehler zu
werfen, und ein fehlerhafter Header gibt trotzdem 200 zurück.

Siehe [Testen](testing.md) für `TestContainer::scope`, wenn Ihr
Test auf einer Multi-Thread-Runtime läuft - der thread-lokale
`fake()`-Guard oben übersteht es nicht, wenn ein Future zwischen
Workern wandert.

### Warum Suprnova abweicht

**FTL-Dateien, keine PHP-Arrays.** Laravel hat zwei Formate -
verschachtelte Arrays in `lang/en/messages.php`, plus flaches
JSON in `lang/en.json` für string-keyed Übersetzungen - und
keins von beiden ist von einem Browser ladbar, noch drückt es
Plural-Auswahl in der Datei aus: die lebt in der
Pipe-und-Bereich-Konvention von `trans_choice` innerhalb der
Zeichenkette. Fluent gibt uns ein Format, das Server und Client
beide parsen, was macht, dass „das Frontend zeigt dieselbe
Zeichenkette, die der Validator produziert hat“ eine Eigenschaft
des Designs ist statt einer Konvention, die Sie pflegen. Es
kostet Sie eine neue Syntax zu lernen (dieses Kapitel ist das
meiste davon) und einen Tooling-Wechsel: Poedit kann `.ftl` nicht
bearbeiten, während Crowdin, Weblate, Lokalise und Pontoon es
können. Es kostet auch das gepunktete Namensraum-Konzept -
`trans('messages.welcome')` hat kein Äquivalent, weil IDs ein
flacher Namensraum pro Locale sind. Nutzen Sie stattdessen ein
Präfix.

**Kein `trans_choice`.** Laravel wählt eine Pluralform mit
pipe-getrennten Zeichenketten und expliziten Bereichen:

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

Zählen Sie jetzt auf Polnisch bis 22. CLDR setzt 22 in die
Kategorie `few` - `22 pliki` - aber `[5,*]` verschluckt es und
produziert `22 plików`. Derselbe Bruch passiert bei 32, 42, 102,
und in Russisch, Arabisch, Tschechisch, Litauisch und Walisisch,
jeweils an ihren eigenen Stellen. Integer-Bereiche können
Pluralregeln nicht ausdrücken, weil Pluralregeln nicht von
Bereichen handeln; sie handeln von der letzten Ziffer, den letzten
zwei Ziffern, und in manchen Sprachen davon, ob der Wert überhaupt
eine Ganzzahl ist. Fluent wählt direkt anhand der CLDR-Kategorie
aus, sodass `$count` ein gewöhnliches Argument ist, und der
*Übersetzer* - die Person, die die Sprache kennt - schreibt alle
vier Kategorien des Polnischen:

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` ist 1; `few` ist 2-4, 22-24, 32-34, 102-104; `many` ist 0,
5-21, 25-31; `other` fängt die Brüche (`1,5 pliku`) ab und trägt
den Standard-Marker, gemäß der Regel oben.

Laravels bereichslose Form (`plik|pliki|plików`) macht es besser -
sie konsultiert einen Index pro Sprache und wählt das *n*-te
Segment - aber dieser Index ist eine handgepflegte Tabelle statt
CLDR-Daten, er bietet Polnisch drei Segmente, wo CLDR vier
Kategorien definiert, die Segmente sind positionell ohne
Kategorienamen zum Nachprüfen, und er kann nur je nach der Anzahl
auswählen.

Was der zweite Vorteil ist, der gratis mit abfällt: Ein
Fluent-Selektor kann auf *jedem* Argument umschalten, nicht nur
auf einer Anzahl. Geschlecht, Plan-Stufe und Verbindungsstatus
wählen auf dieselbe Weise, und keins davon brauchte eine neue
Facade-Methode.

**Isolationsmarken sind standardmäßig aus.** Fluent umschließt
normalerweise jede Interpolation mit U+2068 (FIRST STRONG
ISOLATE) und U+2069 (POP DIRECTIONAL ISOLATE), sodass ein
Rechts-nach-links-Wert, eingebettet in einen
Links-nach-rechts-Satz, in der richtigen Reihenfolge rendert.
Korrekt - und unsichtbar, was bedeutet, dass jedes
`assert_eq!("Hello Ada", …)` in einer rein englischen App an zwei
Zeichen scheitert, die niemand im Diff sehen kann. Wir stellen sie
standardmäßig aus und machen das Einschalten zu einem einzigen
Aufruf:

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**Schalten Sie sie ein, wenn Sie ein RTL-Locale ausliefern** -
Arabisch, Hebräisch, Persisch, Urdu - oder jedes Locale, in dem
nutzergelieferte Werte Schriftsysteme innerhalb eines Satzes
mischen. Aktualisieren Sie dann Ihre Assertions, um gegen
Zeichenketten zu vergleichen, die die Marken tragen, oder
streichen Sie sie im Assertion-Helfer heraus. Der Standard
optimiert für den üblichen Fall; der korrekte Fall ist eine Zeile
entfernt, und dieser Absatz ist die Erinnerung, sie zu nehmen.

## Nächste Schritte

- [Validierung](validation.md) - Regeln, das `validate!`-Makro,
  und woher `ValidationMessage` kommt
- [TypeScript Types](frontend-typescript-types.md) -
  `generate-types`, `inertia-props.ts` und `lang-keys.ts`
- [Middleware](middleware.md) - `LocaleMiddleware` gegenüber dem
  Rest der globalen Kette ordnen
- [Sitzungen](session.md) - der Store, den der erste
  Erkennungsschritt liest
- [Umgebungsvariablen](env-vars.md) - `APP_LOCALE`,
  `APP_FALLBACK_LOCALE`, `APP_LOCALE_PARENTS`, `APP_BASE_PATH`
- [Testen](testing.md) - `TestContainer`, `#[suprnova_test]`, und
  hermetische DI-Overrides
