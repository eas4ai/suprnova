# Validierung

Suprnova validiert Request-Eingaben auf zwei komplementären Wegen:

1. **Derive-Validierung** - `#[validate(...)]`-Attribute auf einer
   `FormRequest`-Struktur, die `extract()` automatisch ausführt. Das ist
   der alltägliche Weg, und er ist in [Anfragen](requests.md)
   beschrieben. Er behandelt Regeln pro Feld (`email`, `length`,
   `range`, …) deklarativ.
2. **Regelobjekte + das `validate!`-Makro** - schlichte Werte, die
   [`Rule`](#regelobjekte) / `ContextualRule` / `AsyncRule`
   implementieren und imperativ komponiert werden. Dazu greifen Sie,
   wenn Sie feldübergreifende Logik brauchen, Regeln, die die Datenbank
   berühren, oder Regeln, die Sie speichern und herumreichen wollen.

Beide Wege sammeln sich in derselben
[`ValidationErrors`](error-model.md)-Bag und rendern dieselbe
Laravel-/Inertia-Form `{ "message", "errors": { field: [...] } }` (HTTP
422).

## Regelobjekte

Eine Regel ist ein Wert, der eines von vier Traits implementiert:

| Trait | Form | Verwendung |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | reine Prüfung eines Wertes |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | Prüfung eines JSON-förmigen Wertes (Array/Objekt) |
| `ContextualRule` | `passes(&self, value, ctx)` | Prüfung, die Geschwisterfelder liest |
| `AsyncRule` | `async passes(&self, value)` | Prüfung, die `.await` nutzt (Datenbank, HTTP) |

Eingebaute `Rule`s: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `InArray`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`,
`AlphaDash`, `Url`, `UrlProtocols`, `HttpUrl`, `Uuid`,
[`Password`](#passwortstärke) (nur Stärkeprüfungen). Eingebaute
`ValueRule`s: `ArrayKeys`, `Distinct`, `Contains`, `DoesntContain`.
Eingebaute `ContextualRule`s: `RequiredIf`, `RequiredWith`,
`RequiredUnless`, `Same`, `Different`, `Confirmed`, `Gt`, `Gte`, `Lt`,
`Lte`. Eingebaute `AsyncRule`s: [`Unique`](#die-unique-regel) und
[`Password`](#passwortstärke) (Stärke plus die HIBP-Prüfung
`uncompromised()` - die eine eingebaute Regel, die sowohl `Rule` als auch
`AsyncRule` implementiert).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Hinweis:** `Numeric` akzeptiert eine **endliche** Zahl - `NaN`, `inf` und
> Größenordnungen, die zu Unendlich überlaufen, werden abgelehnt, obwohl
> Rusts Parser die Zeichenketten annehmen würde.

### URL-Schemata

`Url` akzeptiert einen Wert, der sich als URL parsen lässt, dessen Schema
auf Laravels Allowlist steht - derselben Liste, die
`Illuminate\Support\Str::isUrl` nutzt -, von `://` gefolgt wird **und**
darauf wiederum von einem nicht leeren Host, womit es Laravels Muster
`^(PROTOCOLS)://HOST` in der Form entspricht (Laravels Host-Gruppe hat
kein `?` - ein fehlender oder leerer Host trifft nie). Die Schema-Liste und
die Anforderung `://` plus Host sind wörtlich Laravels; der Host wird von
der `url`-Crate geparst statt von Laravels Regex, ein Port außerhalb des
gültigen Bereichs wird hier also abgelehnt, den Laravel akzeptieren würde.
Alle drei Bedingungen müssen gelten: `mailto:`, `tel:` und `data:` stehen
namentlich auf der Allowlist, tragen aber überhaupt keine
Authority-Komponente, `Url` lehnt sie also ab; und `file:///etc/passwd`
scheitert aus dem dritten Grund - es hat `://`, aber zwischen dem dritten
und dem vierten `/` steht nichts, und nichts ist kein Host. `javascript:`
und `vbscript:` werden rundheraus abgelehnt; sie stehen gar nicht erst auf
der Allowlist.

`ftp://host/x` und `ssh://host` - echte Hosts, nur eben keine
Web-Schemata - bestehen weiterhin, `Url` ist also keine Prüfung nach dem
Motto „das ist eine Webseite“, und es sagt nichts darüber aus, wohin die
URL auflöst. `javascript:` abzulehnen macht einen validierten Wert sicher
genug für ein `href`, nicht sicher genug zum Abrufen. Ein Webhook- oder
Callback-Ziel braucht weiterhin `HttpUrl` (oder Ihre eigenen Schema- und
SSRF-Prüfungen); `Url` allein deckt das nicht ab.

Für einen engeren Satz benennen Sie die Schemata, die Sie wollen:

```rust
use suprnova::{Rule, rules::Url};

// Laravels `url:http,https`
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// Dasselbe, unter einem Namen
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` **ersetzt** die Allowlist, statt sie einzuengen,
eine App kann also ihr eigenes Deep-Link-Schema (`myapp://…`) akzeptieren,
ohne dass das Framework eine Meinung dazu hätte - die Anforderung `://`
plus Host gilt auch für dieses eigene Schema. Nutzen Sie `HttpUrl` (oder
`Url::protocols(&["https"])`) für Callback-, Webhook- und
Avatar-Eingaben - ein Webhook-Ziel, das auf `ftp://internal-host/`
auflöst, parst weiterhin als `Url`, und ein `ftp:`-Ziel ist kein
Webhook-Ziel.

### Passwortstärke

`Password` prüft Länge und Stärke nach Zeichenklassen sowie optional per
`uncompromised()` gegen Have I Been Pwned - Laravels Regelobjekt
`Password`, portiert. Bauen Sie es mit `Password::min(n)` und verketten
Sie die Stärke-Builder:

```rust
use suprnova::{Password, Rule};

let rule = Password::min(8).letters().mixed_case().numbers().symbols();
Rule::passes(&rule, "Str0ng! Pass")?; // Ok(())
Rule::passes(&rule, "weak");          // Err - zu kurz, keine Ziffer, kein Symbol
```

| Builder | Verlangt | Laravel-Regex |
|---|---|---|
| `.min(n)` (über `Password::min`) | mindestens `n` Zeichen (Untergrenze 1) | Längenprüfung |
| `.max(n)` | höchstens `n` Zeichen | Längenprüfung |
| `.letters()` | mindestens einen Unicode-Buchstaben | `/\pL/u` |
| `.mixed_case()` | einen Groß- und einen Kleinbuchstaben, in beliebiger Reihenfolge | `/(\p{Ll}+.*\p{Lu})\|(\p{Lu}+.*\p{Ll})/u` |
| `.numbers()` | mindestens eine Unicode-Ziffer | `/\pN/u` |
| `.symbols()` | mindestens ein Trenn-, Symbol- oder Interpunktionszeichen - **ein einfaches Leerzeichen zählt** | `/\p{Z}\|\p{S}\|\p{P}/u` |

`Password::defaults_with(|| Password::min(12).letters().mixed_case().numbers())`,
einmal aus `bootstrap::register()` aufgerufen, setzt den prozessweiten
Standard, den `Password::defaults()` überall sonst zurückgibt - Laravels
`Password::defaults(fn () => ...)`. Ein zweiter Aufruf wird ignoriert (mit
einem `tracing::warn!`), statt die von der ersten App gewählte Richtlinie
stillschweigend zu ersetzen.

#### `uncompromised()` - weil Stärke allein nicht reicht

`.uncompromised()` (oder `.uncompromised_with_threshold(n)`) fügt eine
Prüfung gegen den Leak-Korpus von Have I Been Pwned hinzu und nutzt dessen
k-anonyme Range-API: Nur die **ersten 5 Zeichen** des in Großbuchstaben
gehaltenen SHA-1-Hashes des Passworts verlassen jemals den Prozess -
`GET https://api.pwnedpasswords.com/range/{prefix}` -, und der Abgleich
gegen den vollständigen Hash geschieht lokal, gegen die
`SUFFIX:COUNT`-Zeilen, die die API für dieses Präfix zurückgibt. Der Dienst
sieht nie das Passwort und nicht einmal dessen vollständigen Hash. Der
Vergleich mit dem Schwellenwert ist strikt (`count > threshold`), das
voreingestellte `uncompromised()` (Schwellenwert `0`) scheitert also bei
jedem einzelnen Auftreten, und ein Netzwerkfehler, ein Timeout oder eine
Antwort außerhalb von 2xx **lässt das Passwort durchgehen** - es gilt
als sauber, statt während eines Ausfalls von Have I Been Pwned jede
Registrierung zu blockieren. Das entspricht genau Laravels
`NotPwnedVerifier`.

Weil diese Prüfung ein HTTP-Round-Trip ist, braucht `uncompromised()`
`AsyncRule` und nicht das synchrone `Rule`, das die Stärkeprüfungen allein
nutzen können. Verdrahten Sie es über `after_validation_async`, nach
demselben Rezept, das [`Unique`](#die-unique-regel) nutzt:

```rust
use suprnova::{AsyncRule, FormRequest, Password, ValidationErrors, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct Register {
    pub password: String,
}

#[async_trait]
impl FormRequest for Register {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Password::defaults()
            .uncompromised()
            .check_async(&self.password, &mut errs, "password")
            .await;
        errs.into_result()
    }
}
```

Das synchrone `Rule::passes` auf einem `Password` aufzurufen, für das
`uncompromised()` gesetzt ist, ist ein **lauter Fehler** und kein stilles
Überspringen - eine Sicherheitsprüfung, die still nichts tut, ist
schlimmer als eine, die es nie gab. Die Fehlermeldung benennt
`after_validation_async` als Abhilfe.

`HIBP_TIMEOUT_SECS` (Standard `30`) steuert das Timeout der Anfrage -
siehe [Umgebungsvariablen](env-vars.md).

Ein eigener Verifier, der `Err` zurückgibt, ist ein anderer Fall als eine
fehlgeschlagene Prüfung: Sein Fehlertext wird auf `error`-Ebene
protokolliert und erreicht nie den Client, und die Antwort trägt
stattdessen den Katalogschlüssel `validation-password-unverifiable` („The
{ $field } could not be checked against known data leaks. Please try
again.“). Fügen Sie diesen Schlüssel hinzu, wenn Sie einen eigenen
Validierungskatalog ausliefern.

### Warum Suprnova abweicht: Password

- Laravels `Password` sammelt jede fehlgeschlagene Stärkeprüfung in einem
  Array. Suprnovas `Rule`-Vertrag gibt eine einzelne
  `ValidationMessage` zurück, `Rule::passes` meldet also die ERSTE
  fehlschlagende Prüfung, in der Reihenfolge min, max, gemischte
  Groß-/Kleinschreibung, Buchstaben, Symbole, Ziffern - Sie beheben eine
  nach der anderen, statt die ganze Liste vorab zu sehen.
- Laravels synchroner Validator kann `uncompromised()` direkt aufrufen;
  eine PHP-Anfrage steckt bereits in einer Ereignisschleife, die einen
  blockierenden HTTP-Aufruf verträgt. Suprnovas `Rule::passes` ist
  vertraglich synchron, es gibt darin also keinen sicheren Ort, von dem
  aus sich die HIBP-Anfrage stellen ließe. Statt die Prüfung
  stillschweigend zu überspringen - das eine nicht hinnehmbare Ergebnis
  bei einer sicherheitsrelevanten Regel - gibt Suprnovas `Rule::passes`
  einen lauten, an Entwickler gerichteten Fehler zurück, der
  `after_validation_async` als Abhilfe benennt.
- `Password::defaults_with` nimmt einen schlichten `fn`-Zeiger entgegen,
  keine Closure, sodass der konfigurierte Standard `Copy` bleibt und keine
  Heap-Allokation braucht - eine bewusste Verengung gegenüber Laravels
  `Closure`.

### Eine eigene Regel schreiben

Eine eigene Regel ist eine Unit-Struktur (oder eine datentragende) mit
einer Implementierung. Das Trait gibt Ihnen `check()` gratis dazu - es
schiebt jede Fehlermeldung unter dem benannten Feld in eine
`ValidationErrors`-Bag -, sodass sich die Regel unverändert in `validate!`
und die `after_validation`-Hooks einfügt:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// Jetzt überall nutzbar:
StartsWith("acct_").passes("acct_1234")?;
// oder, in einer validate!-Zeile:
//   stripe_id => Required, StartsWith("acct_");
```

Ein `String` wird in eine `ValidationMessage` umgewandelt, die wörtlich
gerendert wird, und mehr braucht eine einsprachige App nicht. Damit die
Meldung pro Locale übersetzt wird, geben Sie stattdessen eine
*geschlüsselte* Meldung zurück -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
und definieren die ID in `lang/<locale>/validation.ftl`. Siehe
[Lokalisierung](localization.md); dort wird auch beschrieben, wie Sie die
Meldungen der eingebauten Regeln überschreiben und wie die
Namenskonvention `field-<name>` funktioniert.

Für feldübergreifende Logik implementieren Sie stattdessen
[`ContextualRule`] - die Methode `passes` bekommt neben dem geprüften Wert
einen `&FormContext` (eine `HashMap<String, String>` der Werte von
Geschwisterfeldern). Für datenbankgestützte Prüfungen implementieren Sie
[`AsyncRule`] und nutzen es aus `after_validation_async`.

### Wertförmige Regeln

`Rule` sieht immer nur `&str`. Zwei eingebaute Regeln brauchen mehr
Struktur, als eine Zeichenkette trägt, sie implementieren daher stattdessen
`ValueRule` über `&serde_json::Value`:

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Laravels array:keys - Schlüssel außerhalb der erlaubten Menge ablehnen.
// Die aufgeführten Schlüssel müssen nicht alle vorhanden sein; eine leere
// Erlaubnisliste ist ein Programmierfehler und wird als schlüssellose
// Meldung gemeldet.
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Laravels distinct / distinct:ignore_case / distinct:strict.
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

Ein Feld, das von einer `ValueRule` validiert wird, muss selbst ein
`serde_json::Value` halten (oder ein `Option<serde_json::Value>` für eine
`?:`- oder `?=>`-Zeile) - typischerweise ein Anfragefeld, das direkt aus
dem JSON-Body kommt. `validate!`-Zeilen nehmen `Rule`s und `ValueRule`s in
derselben Feldliste an; welches Trait läuft, entscheidet sich daran,
welches der Typ der Regel implementiert, nicht an etwas, das Sie in die
Zeile schreiben.

### Zugehörigkeitsregeln

Drei Regeln beantworten „liegt dieser Wert in jener Liste?“, jede über die
Form, die sie braucht:

```rust
use suprnova::{Rule, ValueRule, rules::{Contains, DoesntContain, InArray}};

// Laravels in_array:allowed_roles.* - der Wert muss in der Liste eines
// anderen Feldes vorkommen. Übergeben Sie die Liste selbst: ein
// Vec<String>-Feld und ein &[&str]-Literal funktionieren beide.
InArray(&form.allowed_roles).passes(&form.role)?;

// Laravels contains:rust,web - das Array muss jeden aufgeführten Wert
// enthalten.
Contains(&["rust", "web"]).passes(&form.tags)?;

// Laravels doesnt_contain:banned - das Array darf keinen davon enthalten.
DoesntContain(&["banned"]).passes(&form.tags)?;
```

Jeder Vergleich ist exakt. `InArray` vergleicht Zeichenketten mit `==`,
und `Contains` und `DoesntContain` treffen einen Parameter nur gegen ein
JSON-String-Element, `["1"]` enthält also `"1"` und `[1]` nicht. Ein Wert,
der kein Array ist, scheitert bei `Contains` und `DoesntContain` rundheraus.

`Contains` und `DoesntContain` lehnen eine leere Parameterliste als
schlüssellosen Konstruktionsfehler ab, genauso wie `ArrayKeys` es tut -
eine Liste ohne Inhalt schränkt nichts ein. Ein leerer Heuhaufen bei
`InArray` ist etwas anderes: Ein Geschwisterfeld darf zur Laufzeit
berechtigterweise leer sein, der Wert scheitert dann einfach.

Die Fehlermeldung von `InArray` benennt keine Werte, denn ihre Liste kommt
aus der Anfrage, und eine Validierungsmeldung wird in einen Antwortrumpf
gerendert.

### Vergleichsregeln

`Gt`, `Gte`, `Lt` und `Lte` vergleichen ein Feld mit einer Zahl oder mit
einem anderen Feld. `CompareWith` benennt Operand und Maß gemeinsam:

```rust
use suprnova::{ContextualRule, FormContext, rules::{CompareWith, Gt, Lte}};

let mut ctx = FormContext::new();
ctx.insert("max_price".to_string(), form.max_price.clone());

// Laravels gt:0 - ein Literal als Operand, numerisch verglichen.
Gt(CompareWith::Number(0.0)).passes(&form.price, &ctx)?;

// Laravels lte:max_price - ein Geschwisterfeld, numerisch verglichen.
Lte(CompareWith::NumericField("max_price")).passes(&form.price, &ctx)?;

// Laravels gt:summary auf zwei String-Feldern - nach Zeichenzahl
// verglichen.
Gt(CompareWith::LengthField("summary")).passes(&form.body, &ctx)?;
```

Alle vier lesen Geschwisterfelder, sie sind also `ContextualRule`s, und
jede `validate!`-Zeile trägt `=> with ctx` - auch eine Zeile, deren
einziger Operand ein Literal ist und in der der Kontext ungelesen bleibt.
Übergeben Sie dort einen leeren `FormContext`.

Alles, was die Regel nicht messen kann, lässt das Feld scheitern: ein
Wert, der unter einem numerischen Vergleich keine endliche Zahl ist, ein
Geschwisterfeld, das das Formular nie gesendet hat, ein Geschwisterfeld,
das keine Zahl ist, oder ein nicht endliches Literal wie `f64::NAN`.
Nichts davon panickt, und nichts davon besteht.

### Warum Suprnova abweicht

Laravels `distinct:strict` stützt sich auf PHPs erzwingendes `==`.
JSON-Werte sind bereits typisiert, Suprnovas `strict` ändert also nur, ob
zwei *Zahlen* mit unterschiedlicher interner Darstellung (`1` gegenüber
`1.0`) als gleich gelten - es macht nie eine Zeichenkette und eine Zahl
„zum Selben“, in keinem der beiden Modi.

Laravel schreibt das andere Feld in einen Regel-String -
`in_array:allowed_roles.*` -, und der Validator globt es zur Laufzeit aus
den Anfragedaten heraus. Suprnova hat keinen Parser für Regel-Strings: Sie
geben `InArray` die Liste direkt, und der Compiler prüft, dass das Feld
existiert.

Laravel 13.27 hat `in`, `in_array` und `doesnt_contain` auf strikten
Vergleich verschärft, weil PHPs `==` `"1abc"`, `true` und `"0x1"` zu
Treffern machte. Suprnova hatte dieses Loch nie - `In` und `NotIn`
vergleichen `&str` mit `==` -, und die neuen Regeln vergleichen JSON-Werte
Variante für Variante. Laravels `contains` blieb lax; Suprnovas nicht. Der
Preis ist, dass diese Regeln kein numerisches Array prüfen können:
`Contains(&["1"])` trifft `[1]` nicht.

Laravels `gt`-Familie wählt ihr Maß zur Laufzeit: die Zahl selbst bei
Numerischem, `count()` bei Arrays, Kilobytes bei Dateien und Zeichenlänge
bei allem anderen, wobei der numerische Zweig davon abhängt, ob das Feld
außerdem `numeric` oder `integer` trägt. Suprnova schreibt das Maß
stattdessen in die Regel, denn eine Regel kann hier die anderen Regeln
ihres Feldes nicht sehen, und der Form des Wertes nachzuschnüffeln ist
genau die Zwangsumwandlungs-Gewohnheit, gegen die es diese Regeln gibt.
Zwei von Laravels vier Maßen haben überhaupt keine Entsprechung: Eine
Regel bekommt immer nur eine Zeichenkette, ein Geschwisterfeld mit einem
Array als Wert lässt sich also nicht lesen, und Uploads erreichen die
Regel-Oberfläche nie - der Multipart-Parser deckelt ihre Größe, bevor ein
Handler sie zu sehen bekommt.

## Das `validate!`-Makro

`validate!` lässt eine Kette von Regeln über die Felder einer Struktur
laufen und sammelt jeden Fehlschlag in einem einzigen
`ValidationErrors`. Es ist die idiomatische Heimat des synchronen
feldübergreifenden Hooks [`after_validation`](#feldübergreifende-hooks).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Kontextuelle Regeln lesen die Werte der Nachbarfelder aus einem
    // `FormContext`, den Sie bauen - eine Map von Feldname auf String-Wert.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // Zeile in Pflichtform
        bio         ?: Min(10), Max(500);        // optional: nur validieren, wenn Some
        card_number ?=> RequiredIf {             // bedingtes Vorhandensein (siehe unten)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Jede Zeile hat eine von drei Formen:

- **`field => Rule1, Rule2;`** - Pflichtform. Die Regeln laufen direkt
  auf `&self.field` (für `String`, `i64` oder alles, was sich auf die
  von der Regel erwartete Referenz dereferenzieren lässt) - oder bei
  einer `ValueRule` direkt auf einem `serde_json::Value`-Feld. Welchen
  Trait die jeweilige Regel verwendet, wird automatisch abgeleitet.
- **`field ?: Rule1, Rule2;`** - optional. Das Feld ist `Option<T>`; die
  Regeln laufen nur, wenn es `Some` ist, und werden bei `None`
  **vollständig übersprungen**. Das ist Laravels Semantik "falls
  vorhanden, validieren" (`sometimes`).
- **`field ?=> Rule1, Rule2;`** - bedingtes Vorhandensein. Ebenfalls für
  ein `Option<String>`-Feld, aber die Regeln laufen **auch bei `None`**
  (Abwesenheit wird als leerer String behandelt). Das ist die Zeile für
  Regeln, deren Bedingung am Vorhandensein hängt, etwa `RequiredIf` -
  die *ein fehlendes Feld durchfallen lassen* können müssen. Genau das
  kann `?:` nicht ausdrücken, weil es bei `None` überspringt.

Auf eine kontextuelle Regel folgt `=> with $ctx` (eine
`&HashMap<String, String>` mit den Werten der Nachbarfelder). Das Makro
ist **synchron** - für asynchrone Regeln nehmen Sie den
[Hook](#asynchrone-regeln-in-requests) weiter unten.

> **Warnung:** Eine häufige Falle ist es, `card_number ?: RequiredIf {...} => with ctx;`
> zu schreiben. In einer `?:`-Zeile überspringt `None` alle Regeln, sodass
> `RequiredIf` ein fehlendes Feld nie durchfallen lassen kann. Verwenden Sie
> `?=>` für jede Regel, die bei Abwesenheit feuern muss.

## Feldübergreifende Hooks

`FormRequest` führt nach den abgeleiteten Regeln pro Feld zwei
feldübergreifende Hooks aus, sowohl im normalen als auch im
Precognition-Flow. `extract()` durchläuft die Stufen der Reihe nach -
abgeleitetes `validate()`, dann `after_validation`, dann
`after_validation_async` - und **steigt bei der ersten fehlschlagenden
Stufe aus**.

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **Hinweis:** Überschriebene Hooks brauchen ein von Hand geschriebenes
> `impl FormRequest` - das `#[request]`-Attribut und
> `#[derive(FormRequest)]` geben ihre eigene (leere) Impl aus und sind damit
> nur für den häufigen Fall ohne Überschreibung gedacht.

### Asynchrone Regeln in Requests

Das `validate!`-Makro kann kein `.await` einweben, deshalb laufen
datenbankgestützte Regeln in `after_validation_async` - der letzten
Validierungsstufe, die `extract()` automatisch aufruft. Hier nehmen
[`Unique`](#die-unique-regel) und jede eigene `AsyncRule` an der
automatischen Request-Validierung teil; es ist keine Verdrahtung pro
Handler nötig.

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

Weil die asynchrone Stufe erst läuft, wenn die synchronen Stufen
bestanden sind, erreicht ein fehlerhafter Wert (eine syntaktisch
ungültige E-Mail-Adresse) nie die `Unique`-Abfrage in der Datenbank.

## Die `Unique`-Regel

`Unique` prüft, dass ein Wert in einer Tabelle noch nicht existiert.
Bauen Sie sie mit `Unique::new(table, column)` und verfeinern Sie sie
über die Fluent-API:

```rust
use suprnova::Unique;

// email muss eindeutig sein, die gerade bearbeitete Zeile ausgenommen
Unique::new("users", "email").ignore(current_user_id)

// email eindeutig *pro Tenant*, ohne Unterscheidung von Groß- und Kleinschreibung
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Builder-Methode | Wirkung |
|----------------|--------|
| `.ignore(id)` | schließt die Zeile aus, deren `id` gleich `id` ist (Fall: sich selbst bearbeiten) |
| `.ignore_with_column(col, id)` | schließt über eine Schlüsselspalte aus, die nicht `id` ist |
| `.where_eq(col, value)` | beschränkt die Prüfung auf Zeilen mit `col = value`; mehrere Aufrufe werden mit AND verknüpft |
| `.case_insensitive()` | vergleicht mit `LOWER(col) = LOWER(?)` |

Tabelle, Spalte, der Ausschlussschlüssel und jede `where_eq`-Spalte
werden gegen eine Allowlist erlaubter Bezeichner geprüft, bevor sie den
SQL-String erreichen; der geprüfte Wert und alle Werte der Einschränkung
sind gebundene Parameter.

### `Unique` ist unverbindlich - die Datenbank-Constraint ist die Garantie

`Unique` führt ein `SELECT COUNT(*)` **vor** dem Schreiben aus und trägt
damit unvermeidlich ein Race zwischen Prüfzeitpunkt und
Nutzungszeitpunkt in sich: Zwei gleichzeitige Anfragen können beide die
Prüfung bestehen und danach beide einfügen. Laravels `unique`-Regel hat
exakt dieselbe Eigenschaft. Die **einzige** echte Garantie ist eine
`UNIQUE`-Constraint (oder ein eindeutiger Index) auf der Spalte in Ihrer
Migration.

Verwenden Sie alle drei zusammen:

1. **Die unverbindliche Regel** - eine schnelle, freundliche Meldung
   "diese E-Mail-Adresse ist schon vergeben" vor dem Absenden (und damit
   Precognition das Feld validieren kann).
2. **Die `UNIQUE`-Constraint** - der maßgebliche Schutz gegen das Race.
3. **`FrameworkError::from_unique_violation`** - an der Schreibstelle:
   Die Constraint-Verletzung, die der Verlierer eines Races bekommt,
   wird auf dasselbe saubere 422 abgebildet, statt ein 500 nach außen
   dringen zu lassen:

```rust
use suprnova::FrameworkError;

// `users.email` trägt in der Migration eine UNIQUE-Constraint.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` liefert einen 422-`Validation`-Fehler, wenn der
Datenbankfehler die Verletzung einer Unique-Constraint ist, und reicht
jeden anderen Fehler unverändert durch (MySQL, Postgres und SQLite
werden alle erkannt).

## Asynchrone Autorisierung

`FormRequest::authorize(&Request) -> bool` läuft **vor** dem Parsen des
Bodys und kann unautorisierte Anfragen daher abweisen, ohne den Payload
zu lesen. Das ist bewusst synchron: Zu diesem Zeitpunkt hält die Anfrage
noch den streamenden Body, der Hook kann also kein `.await` verwenden.
Autorisierung, die die Datenbank oder eine asynchrone Policy braucht,
gehört an eine dieser Stellen und nicht in `authorize`:

- **Middleware** - läuft vor `extract()`, ist `async` und schließt kurz,
  indem sie `Err(response)` zurückgibt (siehe
  [Middleware](middleware.md)). Der richtige Ort für "darf dieser
  Benutzer diese Route überhaupt erreichen".
- **Das Gate** - rufen Sie `Gate::allows_async` /
  `Gate::authorize_async` im Handler auf, sobald Sie den
  authentifizierten Benutzer und die Ressource haben (siehe
  [Autorisierung](authorization.md)).
- **`after_validation_async`** - für eine Autorisierungsprüfung, die vom
  geparsten Request-Body abhängt: Führen Sie sie im asynchronen Hook
  neben Ihren übrigen asynchronen Regeln aus.

## Inertia-Formularübermittlungen

Ein Validierungsfehler beantwortet zwei Zielgruppen unterschiedlich. Ein
REST-Client erhält den `422` mit `{ message, errors }`. Ein Inertia-Besuch
erhält einen `303` zurück zur Formularseite, wobei die Fehler in die
Session geflasht werden, weil der Inertia-Client für jede Response, die
er nicht als Inertia-Response erkennt, ein Fehler-Modal anzeigt - ein
`422` würde `form.errors` nie befüllen.

Im Handler ändert sich nichts. Auf der Zielseite trägt jedes Feld seine
erste Meldung als String:

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

Siehe [Inertia-Responses](frontend-inertia-responses.md#validation-failures)
für Error-Bags, `with_all_errors` und das Ziel des Redirects.

## Anmerkungen zum Design

- **Teilweise Validierung.** Ein `FormRequest` deserialisiert vor der
  Validierung in eine typisierte Struktur, die Struktur *ist* also das
  Schema: Ein Feld, das fehlen darf, muss `Option<T>` sein. Genau das
  erlaubt es Precognition auch, einen unvollständigen Payload zu
  validieren - machen Sie die Felder optional, die ein Entwurf weglassen
  darf.
- **Regelmeldungen.** Eingebaute Regeln liefern Meldungen mit Schlüssel
  (`validation-min` plus seine Argumente und ein englischer Fallback),
  die an der Serialisierungsgrenze über den Katalog aufgelöst werden.
  Übersetzen oder formulieren Sie jede davon um, indem Sie dieselbe ID
  in `lang/<locale>/validation.ftl` definieren - kein Wrappen von
  Regeln. Siehe [Lokalisierung](localization.md).
- **`Min` / `Max` / `Between`** sind Regeln für die String-Länge
  (gezählt in Unicode-Skalarwerten). Für numerische Grenzen validieren
  Sie mit `#[validate(range(...))]` am Derive oder mit einer eigenen
  Regel - die Längenregeln sind keine Wertvergleiche.

## Zusammenfassung

| Aufgabe | API |
|------|-----|
| Regeln pro Feld | `#[validate(...)]` auf dem `FormRequest` (siehe Anfragen) |
| Komponierte / feldübergreifende Regeln | `validate! { self => ... }` |
| JSON-geformte Regel (Array/Objekt) | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
| Optional, "falls vorhanden" | `field ?: Rule;` |
| Bedingt erforderliches optionales Feld | `field ?=> Rule => with ctx;` |
| Asynchrone / datenbankgestützte Regel | `after_validation_async` + `AsyncRule::check_async` |
| Eindeutigkeit | `Unique::new(t, c)` + `UNIQUE`-Constraint + `from_unique_violation` |
| Asynchrone Autorisierung | Middleware / `Gate::*_async` / `after_validation_async` |

## Nächste Schritte

- [Anfragen](requests.md) - die Oberfläche aus `#[request]` /
  `#[derive(FormRequest)]`, der alltägliche Weg der abgeleiteten
  Validierung
- [Datenobjekte](data.md) - `#[derive(Data, Validate)]` für eine
  Struktur, die zugleich eingehender Request und ausgehendes DTO ist
- [Fehlermodell](error-model.md) - wie aus `ValidationErrors` der
  422-JSON-Body wird, neben jedem anderen Fehlerpfad
- [Lokalisierung](localization.md) - Regelmeldungen übersetzen, die
  `field-<name>`-Konvention und `ValidationMessage`s mit Schlüssel
- [Autorisierung](authorization.md) - `Gate`, `Policy` und wo die
  Autorisierung im Verhältnis zur Validierung hingehört
- [Middleware](middleware.md) - der richtige Ort für Prüfungen der Art
  "darf diese Anfrage überhaupt durch", die `.await` brauchen
