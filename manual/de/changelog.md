# Änderungsprotokoll

Ein lesbares Protokoll pro Version dessen, was sich in Suprnova
geändert hat. Jeder Versionsabschnitt ist der Freigabe-Datensatz dieser
Version. Eine Version wird freigegeben, wenn ihr Versions-Commit und
der passende `v<version>`-Tag atomar gepusht werden. Neueste zuerst.

## 1.2.0 - 2026-08-05

### Hinzugefügt

- **Das Handbuch erscheint in sieben Sprachen.** `manual/es/`,
  `manual/fr/`, `manual/de/`, `manual/pt-BR/`, `manual/ja/` und
  `manual/zh-Hans/` tragen jeweils das vollständige Handbuch mit 104
  Kapiteln - jedes Kapitel, das Inhaltsverzeichnis und dieses
  Änderungsprotokoll - übersetzt aus der englischen Quelle. Englisch
  bleibt kanonisch: Kapitelstruktur, Codeblöcke, Bezeichner, CLI-Befehle
  und Umgebungsvariablen werden Byte für Byte identisch zur Quelle
  gehalten, sodass ein übersetztes Kapitel dem Englischen nie
  widersprechen kann, was das Framework tut - es sagt es nur in der
  Sprache des Lesers.

  Die Übersetzungen wurden für suprnova.app erstellt und geprüft, das
  dieses Handbuch als sein `/docs` rendert. Jeder Abschnitt trägt dort
  ein Prüfregister: Urteile werden gegen Inhalts-Hashes sowohl des
  Englischen als auch der Übersetzung festgehalten, zwei unabhängige
  Prüfer müssen die exakten Bytes freigeben, damit ein Abschnitt als
  freigegeben zählt, und Glossare je Sprache halten die
  Terminologie-Entscheidungen fest (welche Begriffe englisch bleiben,
  welche das native Wort nehmen, und warum). Korrekturen sind in beiden
  Repositories willkommen - eine Korrektur hier erreicht die Website
  bei ihrer nächsten Synchronisation.

## 1.1.0 - 2026-08-02

### Hinzugefügt

- **Fallback-Ketten pro Locale.** `LocalizationConfig` bekommt `parents`
  (`APP_LOCALE_PARENTS`, kommagetrennte `child=parent`-Paare, oder den
  verkettbaren `.parent(child, parent)`-Builder): Ein Locale kann von
  einem konfigurierten Geschwister-Locale erben, bevor es weiter auf
  das globale `fallback_locale` zurückfällt - `pt-PT` von `pt-BR`,
  `en-AU` von `en-GB`, und so weiter, transitiv.
  `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has` laufen alle die
  Kette ab, aktuelles Locale zuerst, sodass das für jeden
  `Translator`-Treiber funktioniert, nicht nur den mitgelieferten. Ein
  fehlerhaftes Paar, ein ungültiges Locale, ein doppelt benanntes Kind
  oder ein Zyklus (auch ein Locale, das sich selbst als Eltern-Locale
  nennt) scheitert beim Laden der Konfiguration sichtbar, statt zur
  Laufzeit der Anfrage zu degradieren.

  Ausgelieferte Kataloge bleiben vorab kettenabgeflacht:
  `FluentTranslator` baut jetzt den Katalog jedes Locale unter
  `/_suprnova/lang/<locale>.ftl` als Fold - zuunterst der eingebettete
  Framework-Katalog für `en`/`en-*`-Locales, dann die konfigurierte
  Eltern-Kette des Locale, dann seine eigenen `*.ftl`-Dateien -, sodass
  ein verkettetes Locale weiterhin eine einzige in sich geschlossene
  Datei bleibt, die der Browser einmal abruft, ohne dass der Client
  etwas von der Kette wissen muss. Das Abflachen deckt nur
  konfigurierte Eltern ab; das abschließende `fallback_locale` bleibt
  ein Fallback auf Ebene der `Lang`-Facade und wird nicht in die
  ausgelieferten Bytes eingebacken.

  Das macht Delta-artige Kataloge praktikabel: Ein `lang/pt-PT/`-
  Verzeichnis kann nur die Handvoll Strings enthalten, die sich
  tatsächlich von `lang/pt-BR/` unterscheiden, statt eines
  vollständigen doppelten Katalogs. Der Merge, der das möglich macht,
  arbeitet auf Fluent-AST-Ebene - der Wert eines Kindes ersetzt den des
  Elternteils, Attribute mergen nach Namen (ein Override, der ein
  Attribut nicht erwähnt, verliert es nicht mehr), Select-Ausdrücke
  werden als Ganzes ersetzt (CLDR-Pluralkategorien sind
  Locale-abhängig, daher ist ein Merge Variante für Variante nicht
  kohärent), und reine Kind-Einträge werden angehängt. Den
  vollständigen Vertrag finden Sie im neuen Abschnitt
  „Fallback-Ketten“ in `manual/localization.md`.

### Geändert

- **`LocalizationConfig` hat das Feld `parents` bekommen.**
  `from_env()` und der Builder sind nicht betroffen; eine Konstruktion
  per Struktur-Literal (Tests, die eine `LocalizationConfig` von Hand
  bauen) braucht ein Feld mehr.
- **Der Text ausgelieferter Kataloge wird jetzt für jedes Locale vom
  Serializer normalisiert**, und das Mergen mehrerer Dateien innerhalb
  eines Locale (mehrere `.ftl`-Dateien in einem Locale-Verzeichnis)
  läuft jetzt über denselben Merge auf AST-Ebene wie Eltern-Ketten,
  statt über ein einfaches Bundle-Überschreiben. Aufgelöste
  Übersetzungen bleiben unverändert, bis auf die zwei strikten
  Verbesserungen unten; die zugrunde liegenden Bytes rotieren
  trotzdem - `ETag`/`?v=<hash>` rotiert einmalig beim Upgrade. Die
  Verbesserungen: Ein Override verwirft nicht mehr still die
  Attribute, die er nicht erwähnt, und ein reiner Attribut-Override
  streicht nicht mehr den eigenen Wert der Nachricht (vorher ein
  Fehler oder eine Fallback-Auflösung; jetzt löst er zum Wert des
  früheren Overrides auf).

## 1.0.0 - 2026-08-02

### Hinzugefügt

- **Lokalisierung.** Message-Kataloge in `lang/<locale>/*.ftl`
  ([Fluent](https://projectfluent.org)), eine `Lang`-Facade mit dem
  `__!("key", name: value)`-Makro, Locale-Erkennung pro Anfrage
  (`LocaleMiddleware`: Session → Cookie → `Accept-Language` →
  `APP_LOCALE`), und Locale-bewusste Formatierung für Zahlen, Währung,
  Daten, Uhrzeiten, Listen und relative Zeiten über ICU4X.
  `manual/localization.md` ist das Kapitel.

  Die eingebauten Validierungsregeln hören auf, Englisch fest zu
  verdrahten. Jede liefert eine Meldung mit Schlüssel
  (`validation-min` plus ihre Argumente und ein englischer Fallback),
  die einmalig an der Serialisierungsgrenze übersetzt wird - eine
  spanische App bekommt also spanische Validierungsfehler, indem
  `lang/es/validation.ftl` hinzugefügt wird, ohne Wrappen von Regeln
  und ohne geforkte Kopie der Meldungen des Frameworks. Feldnamen
  werden über ein `field-<name>`-Lookup in menschenlesbare Form
  gebracht. `Rule::passes` (und `ContextualRule` / `AsyncRule`) geben
  jetzt `Result<(), ValidationMessage>` zurück; der Rumpf
  `Err("…".into())` einer eigenen Regel kompiliert weiterhin und
  rendert weiterhin wörtlich, aber die Signatur in Ihrer `impl`
  braucht den neuen Typ.

  Der Browser bekommt dieselben Bytes, die der Server aufgelöst hat:
  Der gemergte Katalog wird unter `/_suprnova/lang/<locale>.ftl` mit
  einem ETag und einer unveränderlichen `?v=<hash>`-Form ausgeliefert,
  die drei Starter-Kits parsen ihn mit `@fluent/bundle`, und
  `suprnova generate-types` gibt eine `MessageKey`-Union aus, sodass
  das Umbenennen einer Message den TypeScript-Compiler auf jede
  Aufrufstelle zeigen lässt.

  Fluent statt PHP-Arrays im Laravel-Stil, weil ein Format sowohl
  Server als auch Browser bedienen muss, und weil
  CLDR-Pluralkategorien das sind, was Russisch, Polnisch und Arabisch
  richtig hinbekommt - die Ganzzahl-Bereiche von `trans_choice` können
  das nicht, weshalb es hier kein `trans_choice` gibt. Hinter einem
  standardmäßig aktivierten `localization`-Feature;
  `--no-default-features` kompiliert weiterhin und validiert
  weiterhin, mit den eingebetteten englischen Fallbacks.

- **`IntoInertiaScroll` für `Paginator`.** Der Trait war für
  `LengthAwarePaginator` und `CursorPaginator` implementiert, aber
  nicht für den einfachen Paginator, sodass `simple_paginate`-Ergebnisse
  `Inertia::paginate` überhaupt nicht füttern konnten - obwohl die
  eigenen Moduldocs von `simple.rs` genau dorthin als
  URL-Erzeugungspfad zeigen. Das ließ OFFSET-paginierten
  Inertia-Collections nur die Wahl zwischen einem `COUNT(*)` pro
  Anfrage und dem Handrollen der Scroll-Metadaten. `next_page` kommt
  aus der Overflow-Probe von `LIMIT n+1`, statt aus einer berechneten
  letzten Seite - dafür gibt es ja keine Gesamtzahl, aus der sich eine
  berechnen ließe.

### Behoben

- **`suprnova generate-types` gab bei jedem Lauf eine andere Datei
  aus.** Die topologische Sortierung befüllte ihre Arbeitswarteschlange,
  indem sie über eine `HashMap` iterierte, und Rust randomisiert die
  Hash-Iterationsreihenfolge pro Prozess, sodass aufeinanderfolgende
  Läufe dieselben Interfaces unterschiedlich ordneten. Die Ausgabe ist
  ein eingechecktes Artefakt, also erzeugte jeder Lauf einen Diff - und
  eine generierte Datei, die sich grundlos laufend ändert, ist eine,
  die irgendwann niemand mehr neu generiert, wonach sie im Stillen
  aufhört, den Rust-Code zu beschreiben, den zu beschreiben sie
  vorgibt. Der Verzeichnis-Durchlauf ist jetzt ebenfalls sortiert,
  sodass die Ausgabe auch nicht mehr von der Dateisystem-Reihenfolge
  abhängt. Zwei Läufe über dieselbe Quelle sind jetzt Byte für Byte
  identisch.

- **`topological_sort` tat das Gegenteil seines Doc-Kommentars**,
  indem es Abhängige vor Abhängigkeiten ausgab. Harmlos - ein
  TypeScript-Interface darf eines referenzieren, das später in
  derselben Datei deklariert wird -, weshalb der Kommentar korrigiert
  wurde statt der Reihenfolge, was eine versionierte Datei ohne Nutzen
  durcheinandergewirbelt hätte.

## 0.9.1 - 2026-08-01

Drei Defekte, alle gefunden, indem die Dogfood-App unter einem
containerisierten Harness lief, statt durch Lesen des Codes. Jeder von
ihnen ist unsichtbar für eine Testsuite, die nie einen Prozess so
stoppt, wie Produktion ihn stoppt.

Sie verstärken sich in einer bestimmten Reihenfolge: Ein Rolling
Deploy schickt einem Worker mitten im Job ein SIGKILL (der erste), und
dieser Job nimmt dann einen Reclaim-Pfad, der den Versuch nie
mitgezählt hat (der zweite).

### Behoben

- **`schedule:work`, `queue:work` und `workflow:work` ignorierten
  SIGTERM.** Jeder selektierte allein auf `tokio::signal::ctrl_c()`,
  was einen SIGINT-Handler installiert - sodass SIGTERM nirgends im
  Prozess einen Handler hatte, und SIGTERM ist, was `docker stop`,
  Coolify, systemd und Kubernetes senden. Alle drei hatten hinter
  jenem `select!` bereits einen sorgfältig begrenzten Drain; keiner
  davon war je unter einem Supervisor gelaufen. Vor dem Fix gemessen:
  Ein `docker stop` auf einem `queue:work`-Container verbrauchte sein
  gesamtes 40s-Grace-Fenster und beendete sich mit 137, wobei der
  In-Flight-Job zerstört wurde. Als PID 1 - was ein Container
  ausführt - verwirft der Kernel ein unbehandeltes SIGTERM rundweg,
  sodass der Prozess nicht schlecht starb; er starb überhaupt nicht,
  bis SIGKILL kam. `Server::run` behandelte beide Signale bereits
  korrekt, und sein Listener wird jetzt geteilt, was auch ein Fenster
  für verpasste Signale in der Schleife des Schedulers schließt.

- **Ein Job, der seinen Worker tötete, konnte nie zum Dead-Letter
  werden.** Ein Job, dessen *Handler* fehlschlägt, wird genackt und
  sein Versuch gezählt, sodass er nach `max_tries` zum Dead-Letter
  wird. Ein Job, der *seinen Worker tötet* - OOM, Abort, Segfault,
  oder das SIGKILL von oben - schließt nichts ab; seine Reservierung
  läuft einfach ab, und jeder Treiber pflegte ihn Byte-identisch
  erneut zuzustellen. So ein Job ist unsterblich: Er tötet jeden
  Worker, der ihn beansprucht, kommt unverändert zurück und tötet den
  nächsten, solange irgendetwas Worker neu startet. Alle drei Treiber
  verbuchen den Versuch jetzt dort, wo sie erfahren, dass ein Worker
  gestorben ist, weil ein Wechsel von `QUEUE_DRIVER` nicht ändern
  darf, ob sich ein vergiftender Job stoppen lässt. `attempts`
  bedeutet jetzt „Zustellungen an einen Worker“ statt
  „Handler-Fehlschläge“ - dokumentiert in `manual/queues.md`, weil
  auch ein aus unabhängigen Gründen verlorener Worker einen Versuch
  verbrennt.

- **… und der erschöpfte Job wird jetzt zum Dead-Letter, bevor er
  dispatcht wird.** Den Versuch zu zählen war nötig, aber nicht
  ausreichend. Jede Dead-Letter-Entscheidung lebte im Abschluss-Pfad
  des Workers, der voraussetzt, dass der Handler zurückkehrt - sodass
  sie genau für die Jobs nie lief, die nicht zurückkehren konnten. Mit
  dem Treiber-Fix allein stieg der Zähler (gemessen: 0 → 1 → 2 über
  drei getötete Worker), und nichts reagierte darauf. Das Budget ist
  jetzt aufgebraucht, bevor der Handler läuft. Nur gefunden, weil das
  Container-Experiment erneut lief, nachdem der erste Fix korrekt
  aussah.

- **Die Daemons hatten keinen Tracing-Subscriber.** `serve` bekommt
  einen von `init_telemetry`; `queue:work`, `schedule:work`,
  `schedule:run` und `workflow:work` kommen über einen anderen
  Boot-Pfad und bekamen keinen, sodass jede `tracing::`-Zeile, die sie
  ausgeben, ins Leere ging und `LOG_LEVEL` für sie wirkungslos war.
  Das ist das meiste von dem, was sie zu sagen haben - ein Worker, der
  einen Job zum Dead-Letter macht, ein Scheduler, der einen verpassten
  Tick überspringt, eine Sperre, die er nicht freigeben konnte. In
  einem Container war die einzige sichtbare Ausgabe das Start-Banner,
  und der Prozess wirkte untätig, während er all das tat. Zwei der
  Defekte dieses Release waren unsichtbar, bis das behoben war.

- **Ein Dead-Letter ohne gebundenen Failed-Jobs-Store war eine stille
  Löschung.** Der Persist-Schritt saß in einem
  `if let Some(store) = ..`, sodass der Zweig ohne Store nicht
  gematcht wurde und die Ausführung zum Ack durchfiel - leiser als der
  Fehlerpfad direkt darüber, der wenigstens die Reservierung intakt
  lässt. Ein fehlender Store wurde als erfolgreicher behandelt als ein
  kaputter. Er protokolliert jetzt die vollständige Envelope auf
  ERROR, denn genau das ist es, was `queue:retry` erneut pusht: der
  Unterschied zwischen Arbeit, die von Hand wiederherstellbar ist, und
  Arbeit, die aufgehört hat zu existieren.

- **`QUEUE_DRIVER=database` bindet jetzt einen Failed-Jobs-Store.**
  `failed_jobs` ist Teil des Vertrags dieses Treibers - `queue:retry`
  liest ihn, und `Queue::retry_failed` kann ohne ihn nicht
  funktionieren -, aber `bootstrap_from_env` verdrahtete den Treiber
  und ließ den Store ungesetzt, sodass eine datenbankgestützte
  Warteschlange ins Leere zum Dead-Letter wurde, sofern die App nicht
  von Hand einen band. Konfigurierbar über `QUEUE_FAILED_DB_TABLE`.
  Nur für diesen Treiber: `memory` ist konstruktionsbedingt
  vergänglich, und `redis` hat keine Tabelle, in die geschrieben
  werden könnte.

- **Die Redis-Reclaim-Latenz folgt jetzt `--visibility-timeout`.** Das
  Flag setzt die Idle-Schwelle von XAUTOCLAIM, aber eine separate Uhr
  bestimmt, wie oft ein Consumer nachsieht, und der Treiber ließ sie
  beim 30s-Standard von sea-streamer - sodass `--visibility-timeout 5`
  in Wirklichkeit „bis zu 35 Sekunden“ bedeutete. Das Intervall folgt
  jetzt dem konfigurierten Timeout, geklammert auf 1s..=30s, sodass
  ein kurzes Timeout nicht zu einem XAUTOCLAIM-Sturm werden kann und
  ein langes Reclaim höchstens schneller macht als zuvor.

### Hinzugefügt

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - führt
  einen geplanten Task über Replicas hinweg genau einmal pro fälligem
  Tick aus. Ohne das wählt nichts einen Leader für einen Tick: Jeder
  `schedule:work`-Prozess wertet den Zeitplan unabhängig aus, und bei
  drei Replicas wurde gemessen, dass jeder fällige Task dreimal lief,
  jede Minute, ohne Varianz. Ein nächtlicher Abrechnungsjob auf drei
  Replicas hat jeden Kunden dreimal abgerechnet.

  `without_overlapping()` deckt das nicht ab und kann es auch nicht:
  Seine Sperre ist auf den Task geschlüsselt und wird freigegeben,
  wenn der Handler zurückkehrt, sodass ein schneller Task sie
  freigibt, bevor eine zweite Replica nachsieht. `on_one_server`
  schlüsselt auf den Task *und den Tick* und hält die Sperre über den
  Handler hinaus, sodass sie per TTL abläuft. Die zwei lassen sich
  kombinieren.

  Opt-in, passend zu Laravel. Weicht von Laravel darin ab, dass es
  geschlossen fehlschlägt: Die Wahl ist nur so geteilt wie der Cache
  dahinter, sodass ein Produktions-Boot mit `CACHE_DRIVER=memory` und
  einem Single-Server-Task verweigert wird, wobei die betroffenen
  Tasks benannt werden, mit
  `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` für Deployments, die
  wirklich nur einen Scheduler betreiben.

### Geändert

- `manual/deployment.md` sagt nicht mehr, dass „genau einen
  `schedule:work`-Prozess ausführen“ die einzige Option ist, und
  bekommt einen neuen Abschnitt **Sauber stoppen**, der die
  Grace-Fenster pro Subsystem behandelt, wie man die
  Beendigungs-Grace-Zeit einer Plattform darüber hinaus dimensioniert,
  und warum PID 1 einen fehlenden Signal-Handler schlimmer macht, als
  es klingt.

## 0.9.0 - 2026-07-31

### Sicherheit

- **Auth-Ausstellung ließ sich nur pro Aufrufer drosseln, nie pro
  Empfänger.** Ein adress-geschlüsseltes Limit beantwortet „stellt
  ein Client zu viele Anfragen“; es kann nicht beantworten „wird ein
  Postfach geflutet“. Ein Angreifer, verteilt über ein Botnet oder ein
  einzelnes IPv6-`/64`, blieb unter jedem Pro-IP-Budget, während er
  das Postfach eines einzigen Opfers mit Passwort-Reset-Mails füllte,
  und nichts im Framework konnte das Limit ausdrücken, das das
  gestoppt hätte - eine Key-Funktion konnte Pfad, Header und
  Query-String lesen, aber keinen formularkodierten Body, sodass die
  Adresse genau auf der Route unsichtbar war, die sie trägt.

  `identity_key` schlüsselt einen Bucket auf das Konto, auf das
  eingewirkt wird. Es liest zuerst den Query-String und dann einen
  gepufferten Formular-Body, sodass eine einzige Key-Funktion beide
  Formen abdeckt; der Wert wird getrimmt und kleingeschrieben, weil
  `Alice@Example.com` dasselbe Postfach erreicht wie
  `alice@example.com` und ein Limit, das sich durch Umschalt-Taste
  umgehen lässt, kein Limit ist; und er wird gehasht, weil ein
  Rate-Limit-Backend häufig ein gemeinsam genutztes Redis mit
  schwächerer Zugriffskontrolle als die primäre Datenbank ist.

  Zwei neue Middleware-Builder unterstützen das. `key_reads_body(cap)`
  puffert den Body vor dem Schlüsseln - opt-in, weil Puffern Arbeit
  ist, die ein nicht authentifizierter Aufrufer Sie machen lassen
  kann, und ein Body über der Obergrenze wird mit 413 abgelehnt, statt
  ungeschlüsselt durchgelassen zu werden. `only_when(pred)`
  überspringt einen Limiter komplett für Anfragen, zu denen er nichts
  zu sagen hat, was verhindert, dass ein gestapeltes
  Pro-Empfänger-Budget stillschweigend zum bindenden Limit auf Routen
  wird, die niemanden nennen.

  Die Dogfood-App stapelt jetzt beide auf ihrer Ausstellungsgruppe: 10
  pro 5 Minuten pro Adresse, 3 pro 15 Minuten pro Empfänger.

Eine Durchsicht von Toriis Session-, Passwort-, OAuth- und
Passkey-Pfaden förderte acht Defekte zutage, alle behoben im
gepinnten Fork (`suprnova-torii-rs` `968b0be`).

- **Abgelaufene Sessions ließen sich zurück ins Leben erneuern.** Das
  `refresh` des SeaORM-Session-Repositorys hatte kein Ablauf-Prädikat
  und verlängerte `expires_at` bedingungslos, und
  `OpaqueSessionProvider::refresh_session` übersprang die
  `is_expired()`-Prüfung, die `get_session` durchführt. Ein über
  seinen Ablauf hinaus gehaltenes Token ließ sich unbegrenzt erneuern.
  Auf beiden Schichten behoben. Über Suprnovas eigene Oberfläche nicht
  erreichbar - weder `Torii` noch das Framework legt Session-Refresh
  offen -, aber es ist öffentliche API beider Crates.
- **Das Login-Formular verriet per Timing, welche Konten
  existieren.** Die Authentifizierung kehrte zurück, sobald die
  E-Mail nicht traf, und übersprang Argon2 komplett: gemessen bei
  54 µs für eine unbekannte Adresse gegenüber 719 ms für ein falsches
  Passwort - eine ~13.000-fache Lücke, über ein Netzwerk hinweg
  lesbar. Beide Fehlerpfade verifizieren jetzt gegen einen Dummy-Hash,
  sodass sie gleich viel kosten. Dieser Defekt *war* über Suprnovas
  Passwort-Login erreichbar.
- **Der JWT-Claim `iss` wurde geschrieben, aber nie geprüft.** Das
  Algorithmus-Pinning war bereits korrekt - `alg: none` und eine
  HS-/RS-Verwechslung waren nie möglich -, aber der Issuer war reine
  Dekoration, sodass zwei Dienste mit gemeinsamem Signierschlüssel
  gegenseitig ihre Sessions akzeptiert hätten. Jetzt erzwungen, wenn
  ein Issuer konfiguriert ist.
- **Ein Single-Use-PKCE-Verifier ließ sich zweimal beanspruchen.**
  Der Verbrauch war ein Read gefolgt von einem Delete, sodass zwei
  OAuth-Callbacks für denselben `csrf_state` beide lesen konnten,
  bevor eines der Deletes griff. Jetzt in einer einzigen Operation
  beansprucht - `DELETE ... RETURNING` auf Postgres, ein
  Primärschlüssel-Delete, dessen Anzahl betroffener Zeilen auf
  SeaORM den Gewinner bestimmt.
- **Abgelaufene Sessions wurden als aktiv aufgeführt.**
  `find_by_user_id` hatte keinen Ablauf-Filter, und abgelaufene
  Zeilen überleben, bis ein Cleanup läuft, sodass ein Bildschirm
  „Geräte, auf denen Sie angemeldet sind“ Nutzern tote Sessions zum
  Widerrufen anbot, während er nichts über die lebende sagte.
- **Ein Passkey-Lookup hieß `authenticate`.** Toriis
  `PasskeyService::authenticate_credential` nahm eine Credential-ID
  und lieferte den besitzenden Benutzer, und `PasskeyAuth::authenticate`
  prägte daraus eine Session. Torii speichert Passkeys - es trägt
  keine WebAuthn-Abhängigkeit und kann eine Assertion nicht
  verifizieren, sodass diese Aufrufe nur bewiesen, dass der Aufrufer
  eine Credential-ID kannte: ein Wert, den der Browser im Klartext
  sendet und den `allowCredentials` jedem in die Hand drückt, der
  eine Ceremony starten kann. Umbenannt in `find_user_by_credential`
  und `create_session_for_verified_credential`, beide dokumentieren,
  dass Verifikation die Aufgabe des Aufrufers ist. Über Suprnova
  nicht erreichbar, das `webauthn-rs` selbst steuert (siehe
  `torii_integration::passkey`) und Torii nur für die
  Credential-Speicherung erreicht.
- **Eine WebAuthn-Challenge ließ sich über ihre gesamte TTL hinweg
  wiederholen.** Kein Backend verbrauchte eine Challenge beim Lesen,
  und das SeaORM-`get_challenge` ignorierte `expires_at` sogar
  vollständig und lieferte abgelaufene Challenges als lebend zurück.
  Lesevorgänge schließen jetzt auf beiden Backends abgelaufene Zeilen
  aus, und ein neues `take_challenge` beansprucht eine Challenge genau
  einmal - dieselbe Delete-entscheidet-den-Gewinner-Form wie beim
  PKCE-Fix.

### Breaking Changes

- **Azure Blob Storage und Google Cloud Storage wurden hinter die
  neuen Features `filesystem-azure` und `filesystem-gcs` gesperrt.**
  `Storage::register_azblob`, `register_azblob_with`, `register_gcs`,
  `register_gcs_with`, `AzBlobConfig` und `GcsConfig` existieren nicht
  mehr, sofern Sie das passende Feature nicht aktivieren. Wenn Sie
  eines der beiden Backends nutzen, fügen Sie es Ihrer Abhängigkeit
  hinzu:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  Sie bekommen einen Compile-Fehler, der das fehlende Element nennt,
  keinen Laufzeitfehler.

  Beide opendal-Service-Crates ziehen `rsa` nach, das
  RUSTSEC-2023-0071 (den Marvin-Timing-Angriff) trägt, ohne dass es
  dafür upstream ein Fix-Release gäbe. Sie waren die einzigen Crates,
  die `reqsign-core/jwt` aktivierten, das Feature, hinter dem das
  optionale `rsa` von `reqsign-core` steckt, sodass ein Sperren sie
  alle drei opendal-Pfade dorthin auf einmal kappt. `rsa` ist jetzt
  *vermeidbar*: `--no-default-features --features
  filesystem,database-postgres` löst ohne es auf und hat das
  Storage-Subsystem trotzdem noch. Vorher konnte keine
  Feature-Kombination es abwerfen und dabei überhaupt Storage
  behalten.

  Ein Standard-Build trägt `rsa` weiterhin - `database-mysql` ist ein
  Default-Feature, und `sqlx-mysql 0.8.6` hängt nicht-optional davon
  ab -, daher bleibt die Audit-Ausnahme offen. S3 ist bewusst *nicht*
  gesperrt: `reqsign-aws-v4` nimmt `reqsign-core` ohne `jwt`, sodass
  der S3-Treiber nie einen Pfad dorthin beigetragen hat, und eine
  Sperrung würde das meistgenutzte Cloud-Backend brechen, ohne etwas
  zu entfernen.

### Hinzugefügt

- **`suprnova --version`**, mit `-v` zusätzlich zu claps
  Standard-`-V`. Eine CLI nach ihrer Version zu fragen, mit dem Flag,
  das jede andere CLI verwendet, sollte keinen Usage-Fehler ausgeben.

### Behoben

- **Zwei Redis-Operationen hatten keine Obergrenze.** Die Tag-Leerung
  des Caches las die gesamte Mitgliedermenge eines Tags mit
  `SMEMBERS` und löschte Schlüssel für Schlüssel, sodass ein Tag mit
  großer Mitgliederzahl die Verbindung blockierte und ein
  gleichzeitiger Schreibvorgang zwischen dem Lesen und dem Löschen
  verloren gehen konnte; Tags sind jetzt generationsbasiert, werden
  atomar geleert und mit einem begrenzten `SSCAN` gescannt. Der
  Beförderungsdurchlauf der verzögerten Warteschlange verschob jeden
  fälligen Job in einem einzigen unbegrenzten `ZRANGEBYSCORE`, sodass
  ein Rückstau, der gemeinsam fällig wurde, ein einziges gewaltiges
  Skript erzeugte; er befördert jetzt in Batches.
- **Zwei Shutdown-Drains warteten ewig.** `schedule:work` bei Ctrl-C
  und der Workflow-Worker nach einer Cancellation warteten beide ohne
  Deadline auf jeden In-Flight-Task, sodass ein Task, der nie
  zurückkehrte, den Prozess bis zu `SIGKILL` offenhielt - ein Operator
  sieht dann einen Daemon, der „nicht aufhört“. Beide warten jetzt
  eine begrenzte Grace-Zeit, brechen dann den Rest ab und melden die
  Anzahl.
- **Der Version-Pin-Sweep des Release erkannte nur eine der beiden
  Pin-Syntaxen**, sodass jede Datei mit einer Zeile
  `cargo install --tag vX.Y.Z` und ohne Dependency-Snippet nie
  entdeckt wurde. `suprnova-cli/README.md` hatte Lesern drei Releases
  lang geraten, v0.6.0 zu installieren; `manual/cli.md` und
  `manual/cli-new.md` standen bei v0.7.2; `manual/installation.md`
  trug beide Formen, wobei eine hochgezogen wurde und die andere
  einfror. Entdeckung und Neuschreiben lesen jetzt aus einer einzigen
  Muster-Tabelle, und die Regeln einer Datei leiten sich aus ihrem
  Inhalt ab.
- **`cargo doc` scheiterte bei jedem Build mit `filesystem`, aber
  ohne `testing`** - sieben Intra-Doc-Links von `Storage::fake`
  konnten nicht aufgelöst werden, und `lib.rs` verbietet kaputte
  Links. `testing` ist ein Default-Feature, daher hatte kein
  Gate-Schritt diese Kombination je gebaut; `check-feature-matrix.sh`
  tut das jetzt.
- **Toriis Migrationen ließen sich nicht über ihr eigenes Schema
  hinweg replayen**, sodass eine Datenbank, die es ohne die
  Tracking-Tabelle `torii_migrations` hielt - wiederhergestellt aus
  einem Dump, der sie ausließ, oder von Hand migriert -, nicht unter
  Verwaltung gebracht werden konnte. Jedes `Table::create()` trug
  `.if_not_exists()`; keiner der 19 Aufrufe von `Index::create()` tat
  das, ebenso wenig das Alter `ADD COLUMN locked_at`, sodass der
  Replay durch die Tabellen segelte und beim ersten `CREATE INDEX`
  starb. Behoben im gepinnten Fork (`suprnova-torii-rs` `a0f956d`)
  über `has_index` / `has_column` statt `IF NOT EXISTS`, was sea-query
  für MySQL still verwirft - der syntaktische Fix hätte einen Build
  mit Default-Features kaputt zurückgelassen.
- **Eine fehlgeschlagene Torii-Migration brach den Prozess ab, statt
  einen Fehler zurückzugeben.** `SeaORMStorage::migrate` entpackte den
  Migrator per Unwrap und gab bedingungslos `Ok(())` zurück, sodass
  `init_torii`s Abbildung des Fehlschlags auf einen `FrameworkError`
  unerreichbarer Code war.
- **Die eigene `users`-Tabelle einer App unterdrückte Toriis
  stillschweigend**, weil `.if_not_exists()` nicht zwischen „schon
  meine“ und „schon die eines anderen“ unterscheiden kann. Die
  Migration meldete Erfolg, und die Authentifizierung scheiterte
  später an einer fehlenden Spalte - der Grund, warum der
  `--api`-Starter seine Tabelle `app_users` nennt. Toriis Migration
  warnt jetzt zum Migrationszeitpunkt, wenn eine bestehende
  `users`-Tabelle benötigte Spalten vermissen lässt, und nennt die
  Spalten und die Abhilfe. Es bleibt eine Warnung statt eines harten
  Fehlschlags, damit bestehende Deployments weiter booten.
- **Die Deployment-Anleitungen für Railway und DigitalOcean richteten
  den Plattform-Health-Check auf einen Pfad, der Postgres abfragen
  konnte.** Beide Plattformen starten den Container neu, wenn dieser
  Check fehlschlägt, sodass das Befolgen des Rats aus einem kurzen
  Datenbank-Ausfall eine Restart-Schleife über jede Replica machte.
  Beide nutzen jetzt `/_suprnova/health/live`, wobei die Datenbank von
  Hand über die Console abgefragt wird. Die Legacy-Pfade lösen
  weiterhin auf; an bereits Bereitgestelltem muss nichts geändert
  werden.

## 0.8.0 - 2026-07-30

Nachbesserung nach einem externen Red-Team-Audit. Das Audit lieferte
19 P1-Befunde und ein NO-GO-Urteil für 1.0; dieses Release schließt
**alle neunzehn**, plus eine Reihe von Defekten, die beim Beheben
gefunden wurden und die das Audit nicht benannt hatte.

Mehrere Fixes verwandeln eine stille Fehlkonfiguration absichtlich in
einen verweigerten Boot. Lesen Sie **Upgrade** vor dem Deployment -
eine Produktions-App, die bisher klaglos lief, startet womöglich
nicht mehr.

### Upgrade

Drei Konfigurationen, die früher mit einer Warnung (oder klaglos)
booteten, schlagen jetzt in Produktion geschlossen fehl. Jeder Fehler
nennt die Variable, die ihn freischaltet, und jede hat einen
expliziten Override für das Deployment, bei dem das Risiko
tatsächlich nicht besteht.

- **Ein nicht zustellender Mail-Treiber.** `MAIL_DRIVER` ungesetzt,
  `log`, `memory`, oder ein nicht erkannter Wert lösten alle zu einem
  Transport auf, der Mail rendert und verwirft - sodass
  Passwort-Resets Erfolg meldeten, während nichts versendet wurde.
  Override: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **Klartext-SMTP.** Drei der vier Credential-Kombinationen landeten
  auf einem unverschlüsselten Transport, und der Fall mit beiden
  ungesetzt protokollierte eine Warnung und versendete trotzdem.
  Override: `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **Der In-Memory-Rate-Limiter.** Seine Buckets leben auf dem Heap
  eines Prozesses, sodass hinter N Replicas jedes Kontingent
  eigentlich N-fach ist und jedes Deploy sie zurücksetzt. Zeigen Sie
  `RATE_LIMIT_DRIVER` auf `redis`, oder setzen Sie
  `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true`, wenn Sie wirklich nur
  einen Prozess betreiben. Ein *nicht erkannter* Treiber-Wert schlägt
  aus demselben Grund fehl, weil er auf memory zurückfiel -
  `RATE_LIMIT_DRIVER=Redis`, großgeschrieben, ist der Fall, der am
  ehesten in Produktion landet, weil er konfiguriert aussieht.

Entwicklung, Testing und Staging sind in allen drei Fällen
unverändert. Staging ist absichtlich nicht gesperrt: Es dort hart
scheitern zu lassen, drängt Teams dazu, den Override global zu
setzen, was die Prüfung genau dort entschärft, wo es zählt.

Zwei Verhaltensänderungen, die keine Boot-Fehlschläge sind:

- **`fill` und `first_or_new` weisen fehlerhafte Werte zurück.** Ein
  Wert, der sich nicht in den Typ seines Feldes dekodieren ließ, wurde
  früher zum `Default` dieses Feldes und meldete `Ok` -
  `fill(attrs!{ age: "abc" })` setzte `age = 0` und meldete Erfolg. Es
  liefert jetzt einen `ValidationError`, der das Feld nennt, und
  lässt das Modell unverändert. Unbekannte Spalten werden weiterhin
  still übersprungen (Laravel-Parität), und numerisches Widening
  funktioniert weiterhin.
- **`/_suprnova/health?db=true` liefert den Treiber-Fehler nicht mehr
  zurück.** Das Detail wandert ins Log; der Body behält
  `"database": "error"`. Debug-Builds enthalten es weiterhin.
  Dashboards, die `status` / `database` parsen, sind nicht betroffen.
- **`url::signature_has_not_expired` verlangt jetzt eine gültige
  Signatur**, und ist deprecated. Sie antwortete früher `true` für
  eine gefälschte URL - eine schlechte Signatur ist nicht
  „abgelaufen“, weil sie nie ein Ablaufdatum hatte, das sie verpassen
  konnte -, sodass jeder Handler, der sich allein darauf absicherte,
  Fälschungen akzeptierte. Sie ist jetzt identisch zu
  `has_valid_signature`. Wenn Sie sie genutzt haben, um *abgelaufen*
  von *ungültig* zu unterscheiden (um „fordern Sie einen frischen
  Link an“ statt eines 403 zu rendern), wechseln Sie zu
  `url::signature_verdict`, das alle drei Zustände liefert. Das weicht
  absichtlich von Laravels `URL::signatureHasNotExpired` ab.

Zwei Ergänzungen, die nur dann etwas von Ihnen brauchen, wenn Sie
opt-in gehen:

- **`QueueDriver` hat `settle` und `release` bekommen**, beide mit
  Default-Implementierungen, sodass bestehende
  Treiber-Implementierungen unverändert weiterkompilieren.
  Implementieren Sie `settle`, wenn Ihr Backend einen
  Folge-Schreibvorgang und eine Bestätigung in einer Transaktion
  committen kann; implementieren Sie `release`, wenn es eine
  reservierte Nachricht an Ort und Stelle erneut einreihen kann.
- **Batch-Buchführung kann jetzt dauerhaft sein.**
  `DatabaseBatchRepository` braucht zwei neue Tabellen, `job_batches`
  und `job_batch_settlements` - fügen Sie sie Ihren Migrationen hinzu,
  wie bei `jobs` und `failed_jobs`. Das Schema steht in
  `manual/queues.md`. Nichts ändert sich, wenn Sie bei
  `MemoryBatchRepository` bleiben.

### Sicherheit

- **Slowloris (SEC-07).** Der Header-Read-Timeout von hyper war mit
  30s dokumentiert, aber wirkungslos - er aktiviert sich erst, wenn
  ein Timer am Connection-Builder installiert ist, und das war er
  nicht. Ein Client konnte eine Verbindung, und ein
  `SERVER_MAX_CONNECTIONS`-Permit, unbegrenzt halten. Jetzt aktiviert
  und konfigurierbar über `SERVER_HEADER_READ_TIMEOUT`.
- **Multipart-Uploads (SEC-05).** Die Obergrenze galt für einzelne
  Part-Payloads, aber nicht für den rohen Stream, sodass ein Body das
  Limit in Summe überschreiten konnte. Jetzt am Stream begrenzt.
- **Webhook-HMAC mit leerem Schlüssel (SEC-08).** Beide
  Zahlungs-Adapter akzeptierten ein leeres Secret, das alles
  verifiziert. Auf beiden jetzt abgewiesen.
- **Paddle-Signaturparsing (P2-11).** Ein `paddle-signature` mit
  ungerader Länge oder Nicht-Hex-Zeichen erreichte das gepinnte SDK
  und paniekte darin. Jetzt zuerst validiert: Eine fehlerhafte
  Signatur ist ein 401.
- **Passkey-Registrierung und Reset-Tokens (SEC-01, SEC-02).**
  Anonyme Registrierung gegen eine bestehende E-Mail,
  Nicht-Eigentümer-Registrierung und Eigentümer-Registrierung ohne
  kürzliche Reauth werden jeweils mit unterschiedlichen Status
  abgewiesen. Ein Passwort-Login stempelt jetzt das Reauth-Fenster.
- **`dev:tls` (SEC-10).** Ein Projekt konnte die CA wählen, der der
  Befehl vertraut.
- **Generiertes Docker Compose (P2-12).** Veröffentlichte Postgres und
  Redis auf allen Interfaces, mit in diesem Repository eingecheckten
  Credentials. Jetzt an Loopback gebunden, mit pro Scaffold
  generierten Passwörtern, `.env` mit 0600 geschrieben, und
  symlink-verlinkte Ziele abgewiesen.
- **Health-Endpunkt (P2-01, CI-05).** Er entschied per
  `query.contains("db=true")` - ein Substring-Test - ob die Datenbank
  abgefragt wird, sodass auch `?nodb=true` die Probe auslöste. Jetzt
  korrekt geparst. Der 503 bettet den Treiber-Fehler nicht mehr ein,
  der Hosts, Ports, Schemas und Versionen nannte.
- **Drosselung der Credential-Ausstellung (P2-02).** Die vier
  Auth-Ausstellungs-Routen in der Referenz-App trugen überhaupt kein
  Rate-Limit, und die eine Route, die eines hatte, schlüsselte ihren
  Bucket auf den rohen `x-forwarded-for`-Header - den jeder Client
  pro Anfrage variieren kann, um einen frischen Bucket zu bekommen.
  Beide behoben; das Ausstellungs-Budget wird über die vier Routen
  geteilt, sodass das Rotieren zwischen ihnen es nicht vervielfacht.
- **Ein redeliverter Chain-Schritt puschte seinen Nachfolger erneut
  unter einer neuen ID (DATA-02b, teilweise).** Der Abschluss pusht
  den nächsten Chain-Link absichtlich *vor* dem Acken: Zuerst zu
  acken würde bedeuten, dass ein Crash in diesem Fenster die Chain
  dauerhaft verliert, und ein Duplikat ist wiederherstellbar, wo ein
  stiller Verlust es nicht ist. Aber die Envelope des Nachfolgers
  bekam bei jedem Push eine frische `Uuid::new_v4()`, sodass das
  durch diesen Tausch erzeugte Duplikat von einem legitimen neuen
  Schritt nicht zu unterscheiden war - für den Treiber, für eine
  Outbox und für den Handler.

  Der letzte Punkt ist der eigentliche Preis. Der Zustellungsvertrag
  des Frameworks ist At-least-once, und seine Antwort auf Duplikate
  lautet „Handler müssen idempotent sein“ - aber ein auf `env.id`
  geschlüsselter Handler, dem einzigen Identifier, den er bekommt,
  konnte diesen Vertrag für einen verketteten Job nicht erfüllen,
  weil das Duplikat jedes Mal unter einer neuen ID ankam. Der Vertrag
  war konstruktionsbedingt unerfüllbar.

  Die ID des Nachfolgers ist jetzt eine UUIDv5, abgeleitet von der
  seines Vorgängers, die über die eigenen Redeliveries dieses
  Vorgängers hinweg stabil bleibt. Ein redeliverter Schritt puscht
  erneut die ID, die er zuvor gepusht hat. Keine Schema-Änderung,
  kein neues Feld, keine neue Abhängigkeit.

  Das macht das Duplikat **erkennbar**, das Primitiv, das dem Rest
  von DATA-02b fehlte. Es macht den Push nicht atomar mit dem Ack
  (dafür braucht es die Outbox), und noch weist nichts das Duplikat
  auf dem Weg herein zurück. Beide bleiben offen.
- **Signierte URLs verifizierten eine URL und führten eine andere aus
  (SEC-04).** Die kanonische Form fasste Query-Paare in eine Map
  zusammen, sodass ein wiederholter Schlüssel nur seinen **letzten**
  Wert behielt - während `Request::query_param` den **ersten**
  lieferte. Ein legitim signiertes `?user=victim` ließ sich also mit
  der unangetasteten ursprünglichen Signatur als
  `?user=attacker&user=victim` wiedereinspielen: Die Verifikation
  kanonisierte über `victim` und ließ durch, der Handler handelte an
  `attacker`.

  Die kanonische Form trägt jetzt jedes Paar, sortiert nach
  `(key, value)`, sodass die Signatur die exakte Multimenge der
  Parameter abdeckt - jeden Wert hinzuzufügen, zu entfernen oder zu
  ersetzen bricht den HMAC. Ein wiederholtes `signature` oder
  `expires` wird rundweg abgewiesen, da zweimal eines davon keine
  nicht willkürliche Antwort darauf übrig lässt, welches gilt.

  `Request::query_param` löst einen wiederholten Schlüssel jetzt auf
  seinen letzten Wert auf, passend zu `query_params` und
  `Context::query_param`; es war der einzige der drei, der abweichend
  war, und diese Abweichung war die andere Hälfte des Defekts.
  **Bestehende signierte Links funktionieren weiter** - ohne
  wiederholte Schlüssel sind die Payload-Bytes unverändert, was ein
  Test festnagelt, weil eine Änderung der kanonischen Form, die
  stillschweigend jeden ausstehenden Passwort-Reset-Link entwertet
  hätte, schlimmer wäre als der Bug.

  Sechs Regressionstests, darunter beide Angriffsreihenfolgen, ein
  legitim wiederholter Schlüssel, der weiterhin signieren und
  verifizieren muss, und die Umsortierungs-Garantie. *Nicht*
  geändert: `signature_has_not_expired` meldet eine gefälschte
  Signatur weiterhin als „nicht abgelaufen“. Das ist Laravels
  Verhalten, wurde absichtlich als Dokumentations-Fix festgelegt und
  hat einen eigenen Test, der es gegen eine gutgemeinte „Korrektur“
  festnagelt.
- **RBAC unter Postgres.** Gegen ein echtes Postgres verifiziert,
  nicht nur gegen SQLite allein.
- **Vier RustSec-Advisories beseitigt, nicht erneuert.** Der
  Pinecone-Treiber wurde gegen Pinecones REST-API neu geschrieben,
  wobei `pinecone-sdk 0.1.2` wegfiel - dessen neuestes Release vom
  2024-09-06 stammt - und mit ihm `tonic 0.11 → rustls 0.22 →
  rustls-webpki 0.102` sowie RUSTSEC-2026-0049 / -0098 / -0099 /
  -0104. Alle vier waren upstream in `rustls-webpki >= 0.103.13`
  behoben, was dieser Workspace für seine anderen TLS-Nutzer bereits
  auflöste; eine verwaiste Crate hielt den Baum auf der verwundbaren
  Linie. `.cargo/audit.toml` ist von fünf Ignores auf einen
  gesunken. Siehe **Geändert** für das, was das für die API des
  Treibers bedeutet.
- **Audit-Ausnahmen laufen jetzt ab.** Jeder Eintrag in
  `.cargo/audit.toml` trägt einen `OWNER` und ein `EXPIRES`-Datum,
  und `scripts/check-audit.sh` lässt das Release-Gate bei einem
  fehlenden Owner, einem fehlenden oder nicht parsbaren Datum oder
  einem abgelaufenen scheitern. `cargo audit` kennt kein abgelaufenes
  Ignore, sodass eines, „vorübergehend“ hinzugefügt, so lange
  stehenblieb, bis jemand die Datei erneut las. Der verbleibende
  Eintrag (RUSTSEC-2023-0071, `rsa`, das überhaupt kein Fix-Release
  hat) ist mit Owner und Datum versehen.
- **Erreichbarkeits-Behauptungen werden geprüft, nicht nur
  aufgestellt.** `scripts/check-feature-matrix.sh` löst echte
  Abhängigkeitsbäume auf und stellt sicher, dass kein Build -
  einschließlich `--all-features`, was `cargo audit` tatsächlich
  liest - `pinecone-sdk`, `rustls-webpki 0.102.x` oder `tonic 0.11.x`
  enthält. Eine Ausnahme, die durch einen Kommentar begründet wird,
  den nichts verifiziert, hört auf, wahr zu sein, sobald jemand eine
  Abhängigkeit hinzufügt.

### Behoben

- **Jedes Release auf einer datenbankgestützten Warteschlange war
  stillschweigend ein No-op.** `JobOutcome::Released` - eine belegte
  `WithoutOverlapping`-Sperre, ein Rate-Limiter-Backoff - war als
  „Kopie pushen, dann das Original acken“ implementiert. Die
  Envelope-ID ist der Primärschlüssel der `jobs`-Tabelle, sodass die
  Kopie mit der Zeile kollidierte, die noch die lebende Reservierung
  hielt, und der Push mit `UNIQUE constraint failed: jobs.id`
  scheiterte. Der Worker lehnte daraufhin korrekt das Acken ab,
  sodass die angeforderte Verzögerung nie angewendet wurde, kein
  `JobReleased`-Event feuerte, und der Job einfach parkte, bis der
  Ablauf der Sichtbarkeit ihn erneut zustellte. Releases sind jetzt
  ein einziger Treiber-Aufruf, an Ort und Stelle erledigt.
- **Ein teilweiser Batch-Dispatch verwaiste die Jobs, die er bereits
  eingereiht hatte (DATA-02).** Als ein `driver.push` mitten in der
  Schleife fehlschlug, löschte `PendingBatch::dispatch` die
  Batch-Zeile - aber die bereits in der Warteschlange befindlichen
  Envelopes trugen weiterhin den Stempel dieser Batch-ID, sodass jede
  von ihnen gegen einen nicht mehr existierenden Batch abschloss und
  bei jeder Zustellung für immer `Err(batch not found)` zurückgab.
  Der Batch wird jetzt stattdessen abgeschlossen: Nicht dispatchte
  Jobs werden als Fehlschläge verbucht, und der Batch wird
  abgebrochen, sodass die eingereihten normal abschließen und die
  Abschluss-Callbacks trotzdem feuern.
- **Nichts testete, dass `url::has_valid_signature` eine gefälschte
  URL zurückweist.** Gefunden beim Verifizieren des SEC-04-Fixes: Die
  gesamte Framework-Suite war grün, obwohl die primäre
  Signed-URL-Absicherung umgeschrieben war, um jede Signatur zu
  akzeptieren.
- **Eine gescaffoldete App konnte weder ihre Datenbank migrieren noch
  ihr Image bauen (REL-01b).** Keines der beiden Scaffolds
  deklarierte `default-run`, sodass alle neun CLI-Wrapper, die zu
  `cargo run` ausshellen, bei einem frischen Projekt scheiterten. Das
  generierte Dockerfile hatte fünf unabhängige Defekte - ein
  fehlendes Lockfile-COPY, `npm ci` ohne Lock, eine Cache-Stufe, die
  eines von zwei deklarierten Binaries stubbte, einen Frontend-Build,
  kopiert von einem Pfad, den vite nie anlegt, und ein fehlendes
  `frontend/src/pages`-Copy, das `inertia_response!` zur Compile-Zeit
  validiert. Das Image eines Standard-Scaffolds konnte nicht bauen.
- **`docker:init` gab für jeden Projekttyp dasselbe Dockerfile aus.**
  Bei einem `--api`-Projekt scheiterte dessen erste Instruktion,
  `COPY frontend/package.json`, rundweg. API-Projekte bekommen jetzt
  ein frontend-freies Dockerfile.
- **SQL-Platzhalter (DATA-01).** Werden jetzt pro Backend gerendert,
  statt einen Dialekt anzunehmen.
- **Warteschlangen-Abschluss (DATA-02a, P2-06c).** Folge-Aktionen
  schließen ab, bevor die Reservierung geackt wird, und ein
  Sperr-Freigabe-Fehler verwandelt einen bereits erfolgreichen Job
  nicht mehr in eine Wiederholung.
- **Ein abgebrochener Batch feuerte `Catch`, nie `Then`.**
- **`Builder::clone` verwarf den Eager-Load-Plan stillschweigend
  (P2-09a).** `User::query().with("posts")`, überall geklont -
  Paginierung, `count()`, jeder klonende Scope - lieferte Zeilen ohne
  Relationen und ohne Fehler.
- **Presence-Rosters verloren Mitglieder (P2-08).** Der Roster wurde
  vor dem Abonnieren als Snapshot erfasst, sodass jeder, der in
  diesem Fenster beitrat, dauerhaft in keinem von beiden erschien.
- **Pinecone serialisierte jede Index-Beschaffung (P2-14).** Die
  Schreibsperre wurde über zwei Netzwerk-Round-Trips hinweg gehalten,
  und `tokio`s faires `RwLock` bedeutete, dass ein kalter Index jeden
  warmen blockierte.
- **Der Type-Watcher verwarf Bursts (P2-13).** Leading-Edge-Debounce
  regenerierte bei der ersten Datei eines Bursts und verwarf den Rest
  ohne einen abschließenden Lauf, sodass das letzte Speichern nie
  wirksam wurde.
- **`ssr:check` konnte hängen bleiben und versuchte nur eine Adresse
  (P2-13).** DNS lief vollständig außerhalb des Timeouts, und nur die
  erste aufgelöste Adresse wurde versucht - sodass ein Host mit einem
  AAAA-Eintrag und ohne IPv6-Route den Worker als down meldete,
  während er auf v4 lauschte.
- **`suprnova serve` installierte `cargo-watch` ungepinnt (P2-13).**
  Jetzt `--locked` mit einer Major-Version-Grenze.
- **Der Release-Bumper schrieb fünf READMEs um und sonst nichts.**
  Vier Manual-Kapitel und ein öffentlicher Doc-Kommentar pinnten
  Tags, die kein Release je aktualisierte - der Doc-Kommentar war
  zwei Releases veraltet. Die Entdeckung ersetzt jetzt die von Hand
  gepflegte Liste, und der Smoke-Test greppt den hochgezogenen Baum
  unabhängig, statt dem eigenen Verify-Schritt des Bumpers zu
  vertrauen.
- **`db:sync` behandelte das Datenbankschema als vertrauenswürdige
  Eingabe (CLI-01).**
- **`migrate:fresh` ist jetzt hinter `--force` plus einer getippten
  Bestätigung gesperrt (CLI-02)**, sowohl in der App-Binary als auch
  in der CLI.
- **Der `log`-Mail-Treiber protokolliert jetzt die ganze Nachricht**,
  wie Laravel es tut, und schreibt in Produktion keine Bearer-Links
  mehr ins Log.

### Hinzugefügt

- **Atomarer terminaler Abschluss (`QueueDriver::settle`, DATA-02).**
  Der Chain-Nachfolger und die Bestätigung committen jetzt gemeinsam
  auf `DatabaseQueueDriver`, was das Fenster schließt, in dem ein
  Crash zwischen beiden entweder den Rest einer Chain verlor oder
  ihren nächsten Schritt zweimal ausführte. Das auf die Reservierung
  geschlüsselte Delete dient zugleich als Fencing: Ein Worker, dessen
  Sichtbarkeit mitten im Lauf ablief, committet nichts und meldet
  `Settled::Stale`, sodass er keine Arbeit für eine Nachricht
  einreihen kann, die jetzt ein anderer Consumer besitzt. Treiber,
  die das nicht können, antworten `Settled::Unsupported` und behalten
  die dokumentierte Push-vor-Ack-Reihenfolge bei.
- **`DatabaseBatchRepository` (DATA-02).** Die Batch-Buchführung
  übersteht einen Neustart, und `pending_jobs`/`failed_jobs` werden
  aus Abschluss-Zeilen abgeleitet, geschlüsselt auf
  `(batch_id, job_id)`, statt gespeichert und dekrementiert zu
  werden - sodass ein redeliverter Job einen Batch nicht auf
  „abgeschlossen“ treiben kann, während seine anderen Jobs noch
  laufen, und die Absicherung hält prozessübergreifend statt nur
  innerhalb eines Prozesses.
- **`/_suprnova/health/live` und `/_suprnova/health/ready`.**
  Liveness rührt nichts an; Readiness prüft Abhängigkeiten. Eine
  Datenbank-Prüfung in eine Liveness-Probe zu verdrahten macht aus
  einem kurzen Datenbank-Ausfall einen Rolling Restart jeder
  Replica - wozu der bisherige einzelne Endpunkt einlud.
  `/_suprnova/health` funktioniert weiterhin genau wie dokumentiert.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Optionales gemeinsames Secret
  für die Readiness-Probe, in konstanter Zeit verglichen. Ohne es
  antwortet Readiness mit 404 - nicht zu unterscheiden von einem
  ungerouteten Pfad, weil es *das* 404 des Routers selbst ist.
  Standardmäßig ungesetzt, damit bestehende Probes weiterlaufen.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, mit `ssl`
  und `null` als Laravel-kompatible Aliase akzeptiert. Ungesetzt
  leitet es sich aus den Credentials ab und reproduziert exakt das
  bisherige Verhalten. Das macht auch implizites TLS auf Port 465
  erreichbar: Der Transport unterstützte es, aber keine Kombination
  von Umgebungsvariablen konnte es auswählen.
- **`SERVER_MAX_CONNECTIONS` und `SERVER_HEADER_READ_TIMEOUT`**
  dokumentiert in `manual/env-vars.md`, wo sie zuvor vollständig
  gefehlt hatten.

### Geändert

Das Fazit des Audits selbst war, dass das Gate in 470s durchlief und
keinen der 19 P1s fing. Der Großteil der Testarbeit dieses Release
zielt darauf.

- **Postgres läuft im Gate.** Zwölf Tests über sechs Dateien waren
  nie gelaufen. Zwei von ihnen zielten, wie sich herausstellte, mit
  `DROP TABLE` auf irgendein Postgres unter `localhost:5432` als
  Standard, und keiner von beiden hatte je `Crypt` initialisiert,
  sodass beide beim ersten Lauf fehlschlugen.
- **Scaffold-Assertions lesen die Bytes, die ein Nutzer bekommt**,
  nach der Substitution, statt der Template-Quelle. Gefunden: ein
  API-Projekt, das einen Doc-Kommentar auslieferte, der eine
  Datenbank wörtlich `{package_name}` nannte, und eine
  `.env.example`, die fünf Mail-Keys bewarb, die das Framework nie
  liest.
- **Fehler-Injektion für die Warteschlange.** ACK-Verlust,
  Redelivery, Lease-Ablauf und Teil-Dispatch werden von einem
  Decorator gesteuert, der eine benannte Operation beim benannten
  Aufruf fehlschlagen lässt, sodass jeder Fall deterministisch ist
  statt eines Sleep-Race.
- **Zahlungs-Adapter haben jetzt Negativtests.** Stripes `verify()`
  war nie mit einer *gültigen* Signatur durchgespielt worden, sodass
  jeder Ablehnungspfad, der davon abhängt, den HMAC-Vergleich zu
  erreichen, unbewiesen war.
- **Der Pinecone-Treiber spricht REST.** *Breaking, hinter dem
  standardmäßig deaktivierten Feature `vector-pinecone`.* Die
  Motivation steht unter **Sicherheit**; die Oberflächenänderungen
  sind:
  - `client()` ist weg - es gibt kein `PineconeClient` mehr. An
    seine Stelle treten `control_plane_get`, `control_plane_post` und
    `data_plane_post`, die *jeden* Pinecone-Endpunkt mit Ihren
    eigenen Request- und Response-Typen über den authentifizierten,
    host-aufgelösten Transport des Treibers erreichen. Das ist
    strikt mehr Reichweite, als der alte Direktzugriff hatte.
  - `json_to_metadata` → `metadata_from_json`, und Metadaten sind
    jetzt `serde_json::Map` statt `prost_types::Struct`.
    `decode_match_fields` → `decode_match`, nimmt jetzt ein
    `PineconeMatch`. `namespace()` liefert `&str`.
  - Neu: `with_control_plane`, `with_api_version`, `with_index_host`
    (pinnt einen bekannten Host und überspringt den
    Control-Plane-Round-Trip), `index_host`, sowie die Wire-Typen
    `PineconeVector` / `PineconeMatch`.
  - `from_env` liest weiterhin `PINECONE_API_KEY` und
    `PINECONE_CONTROLLER_HOST`, und jetzt auch `PINECONE_API_VERSION`.
  - Die REST-API-Version ist gepinnt, nicht schwimmend - `2025-04`,
    die Version, gegen die die Request- und Response-Formen des
    Treibers geschrieben wurden.
  - Nichts serialisiert mehr. Der alte Treiber cachte einen `Index`
    pro Name hinter einem `tokio::Mutex`, weil `pinecone-sdk` ihn nur
    hinter `&mut self` freigab; der neue cacht einen Host-String und
    teilt sich den Connection-Pool von `reqwest`.
  - Ein von der Control Plane gelernter Host wird immer über `https`
    kontaktiert, unabhängig davon, welches Schema die Response trägt.
  - `Debug` ist von Hand implementiert, mit geschwärztem API-Key,
    sodass ein `#[derive(Debug)]` auf einer Struktur, die einen
    Treiber hält, ihn nicht ausdrucken kann.
- **Wire-Vertrags-Tests für Pinecone.** Die Live-Integrationstests
  brauchen einen `PINECONE_API_KEY` und können daher nicht im Gate
  laufen - was die Feldnamen eines REST-Rewrites (`topK`,
  `includeMetadata`, `vectorCount`) auf nichts ruhen ließ. Dreizehn
  Tests steuern den Treiber jetzt gegen einen lokalen
  `wiremock`-Fake und assertieren die exakte Methode, den Pfad, die
  Header und den JSON-Body, den er aufs Wire legt, sowie dass ein
  Nicht-2xx nie als Ergebnis dekodiert wird und dass eine
  Fehlermeldung nie den API-Key trägt. Sie nageln den Treiber auf
  Pinecones *dokumentierten* Vertrag fest; nur die
  `#[ignore]`-Tests können bestätigen, dass die Dokumentation zum
  Live-Dienst passt.

## 0.7.2 - 2026-07-28

### Behoben

- **`generate-types` löst verschachtelte Prop-Strukturen ohne
  Derives auf.** Der Generator von 0.7.1 degradierte jedes Prop-Feld,
  dessen Typ nicht `InertiaProps`/`Data` derivte, zu `unknown` -
  sodass ein erneuter Lauf des Generators (oder der `suprnova
  serve`-Watcher) über ein Projekt mit eingechecktem Types-File echte
  Interfaces wie `Array<AdminArticleRow>` durch `unknown` ersetzte
  und die Typprüfung in der ganzen App brach. Einfache Strukturen,
  die irgendwo in `src/` definiert sind, lösen jetzt zu ihren echten
  Interfaces auf, transitiv von den Prop-Wurzeln aus; `unknown` (mit
  einer Warnung) bleibt Typen vorbehalten, die das Projekt
  tatsächlich nicht definiert - externe Crate-Typen, Enums,
  Tuple-Strukturen.

### Geändert

- **Die Generierung von `routes.ts` ist jetzt opt-in.**
  `generate-types` legt `frontend/src/types/routes.ts` nicht mehr
  ungefragt in jedes Projekt; übergeben Sie `--routes`, um sie zu
  generieren.

- **Frontend-Starter-Abhängigkeiten aufgefrischt.** Neue Scaffolds
  von `suprnova new` pinnen jetzt aktuelle Versionen: Vite ^8.1.5,
  Tailwind CSS ^4.3.3, Svelte ^5.56.8 (vite-plugin-svelte ^7.2.0,
  svelte-check ^4.7.4), React ^19.2.8 (plugin-react ^6.0.4), Vue
  ^3.5.40 (plugin-vue ^6.0.8, vue-tsc ^3.3.8), und `@types/node` ^24
  (die Node-24-LTS-Typenlinie). TypeScript bleibt absichtlich bei
  ^6.0.3: Das ist das neueste 6.x, und der Peer-Bereich von
  svelte-check (`^5 || ^6`) lässt TypeScript 7 noch nicht zu. Alle
  drei Starter wurden Ende-zu-Ende verifiziert (`npm install` +
  `npm run build`) gegen den aufgefrischten Satz.

## 0.7.1 - 2026-07-27

Ein Defekt-Fix-Durchlauf über das Queue-Routing von 0.7.0, aus einer
vollständigen Post-Release-Durchsicht.

### Behoben

- **Verkettete Jobs verlieren ihre deklarierte Warteschlange nicht
  mehr.** `ChainLink` erfasste `max_tries`, `timeout` und `backoff`
  eines Jobs beim Bau der Chain, aber nicht dessen `Job::queue()`,
  sodass ein Job, der bei direktem Push auf seiner deklarierten
  Warteschlange landete, beim Dispatch als Teil einer Chain auf
  `default` landete - die „Job“-Stufe der Auflösungsreihenfolge Route
  → Job → Default verschwand stillschweigend für Chains. Die
  deklarierte Warteschlange wird jetzt auf dem Link erfasst und genau
  wie bei einem direkten Push aufgelöst. Vor diesem Release
  geschriebene Chain-Payloads dekodieren unverändert
  (`serde(default)`), und ein Link ohne deklarierte Warteschlange
  serialisiert Byte-identisch zu dem, was 0.7.0 schrieb.
- **Failed-Job-Datensätze tragen die Warteschlange, auf der der Job
  starb.** Der Dead-Letter-Pfad des Workers verdrahtete
  `queue = "default"` fest in jeden `FailedJob`-Datensatz, sodass
  Fehlschläge eines gerouteten Jobs für einen Operator unsichtbar
  waren, der den Failed-Store nach dem besitzenden Pool filterte. Der
  Datensatz trägt jetzt die Warteschlange der Envelope (`default` für
  ungeroutete Jobs).
- **Der 0.7.0-Upgrade-Hinweis untertrieb bei der `jobs`-Migration.**
  Er lautete „ungefilterte Worker sind nicht betroffen und brauchen
  keine Migration“, aber `DatabaseQueueDriver::push` nennt die
  Spalte `queue` in seinem `INSERT`, unabhängig davon, ob der Job
  geroutet ist - eine 0.7.0-Binary gegen eine unmigrierte Tabelle
  scheitert bei **jedem Push**, gefiltert oder nicht. Der Abschnitt
  zu 0.7.0 unten und `manual/queues.md` sind korrigiert: Auf dem
  Datenbank-Treiber ist das `ALTER TABLE` für jedes Deployment
  erforderlich, und es muss laufen, bevor Binaries rollen (ältere
  Binaries listen ihre Spalten explizit auf, daher ist zuerst zu
  migrieren sicher).

- **Das README bewirbt kein `#[job]`-Makro mehr.** Ein solches Makro
  existiert nicht - Jobs implementieren den `Job`-Trait. Die
  Warteschlangen-Zeile beschreibt jetzt die echte Oberfläche,
  einschließlich des Queue-Routings von 0.7.0.

### Geändert

- **Der Release-Pfad hebt jetzt README-Versionsreferenzen an.**
  `bump-workspace-version.py` schreibt den gepinnten Install-Tag des
  README, das Beispiel des Distributionsmodells und die MSRV-Zeile
  atomar zusammen mit den Manifesten um, und ein umformuliertes
  README, das nicht mehr zu einem Muster passt, lässt das Release
  sichtbar scheitern. Das README hatte v0.6.0 beworben, seit v0.7.0
  auslieferte, weil nichts im Release-Pfad es berührte.
- **Connection-Routing ist als reine Namensauflösung dokumentiert.**
  `Job::connection()` und das Connection-Feld von `Queue::route`
  lösen den Connection-*Namen* auf, der auf den Lifecycle-Events
  `JobQueueing` / `JobQueued` mitgeführt wird; ein einziger
  prozessglobaler Treiber empfängt weiterhin jeden Push, sodass sie
  keinen anderen Treiber auswählen. Der Rustdoc und
  `manual/queues.md` implizierten zuvor eine Treiber-Auswahl, die es
  nicht gibt. Die Warteschlangen-Dimension ist nicht betroffen - sie
  wird end-to-end respektiert. Pro-Connection-Treiber bleiben
  zukünftige Arbeit.
- `ChainLink` hat ein öffentliches Feld `queue: Option<String>`
  bekommen, was die Struktur-Literal-Konstruktion von Chain-Links
  bricht. Über `ChainLink::from_job` gebaute Links - der normale
  Weg - sind nicht betroffen.

### Upgrade

Wer von ≤ 0.6.x auf dem Datenbank-Queue-Treiber kommt, wendet die
0.7.0-Migration unten **vor** dem Rollen der Binaries an; sie ist für
jedes Deployment auf diesem Treiber erforderlich, nicht nur für
solche, die `--queue` nutzen. 0.7.1 selbst braucht keine Migration.

## 0.7.0 - 2026-07-26

### Sicherheit

- **`ammonia` auf 4.1.4 aktualisiert (RUSTSEC-2026-0213).** Versionen
  bis einschließlich 4.1.3 erlauben XSS über die SVG-Animationstags
  `animate` und `set`. `ammonia` ist der Sanitizer am Ende von
  Suprnovas Markdown-Pipeline (`comrak` → `syntect` → `ammonia`),
  also war jede App exponiert, die nutzergeliefertes Markdown über
  `content` rendert. Das Advisory wurde am 2026-07-21 veröffentlicht -
  nachdem v0.6.5 auslieferte -, daher **ist jedes Release bis
  einschließlich v0.6.5 betroffen**. Der Fix ist, das Framework zu
  aktualisieren; keine Änderungen am Anwendungscode sind erforderlich.

### Hinzugefügt

- **Queue-Routing.** Jobs lassen sich an eine bestimmte Warteschlange
  und Connection dispatchen, und Worker lassen sich bestimmten
  Warteschlangen widmen - die Oberfläche von Laravel 13s
  `Queue::route(...)`, typisiert. Ein Job nennt sein eigenes Zuhause
  mit `Job::queue()` / `Job::connection()`; ein Betreiber
  überschreibt das zentral mit
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` in
  `bootstrap::register()`, ohne den Job zu bearbeiten. Die Auflösung
  ist Route, dann Job, dann globaler Standard, und ein `None`-Feld in
  einer Route verschiebt sich, statt zu leeren. `queue:work
  --queue=billing,default` leert nur diese Warteschlangen.
  Ungeroutete Jobs gehören zu `default`, sodass sie nie stranden.
  Verkettete Jobs lösen Routen nach Namen auf, da ein Chain-Link
  seinen Job typgelöscht speichert.
- **`QueueDriver::pop_from`.** Filternder Pop, mit einer
  Default-Implementierung, die einen Filter, den sie nicht einhalten
  kann, **zurückweist**, statt still jede Warteschlange zu leeren -
  ein Worker, dem gesagt wird, `billing` zu leeren, der aber
  stillschweigend alles leert, ist von einem funktionierenden
  Deployment nicht zu unterscheiden, bis der falsche Pool die
  falschen Jobs frisst. Die Memory- und Datenbank-Treiber filtern
  nativ. Eigene Treiber kompilieren weiter und erben den lauten
  Standard.
- **Das Schema der `jobs`-Tabelle dokumentiert.** `manual/queues.md`
  trägt jetzt die DDL, die `DatabaseQueueDriver` tatsächlich
  erwartet, was vorher nur durch Lesen des SQL des Treibers
  herauszufinden war.
- **Inertias `serverHead`-Option dokumentiert.**
  Server-getriebene `<head>`-Elemente (Inertia 3.5.0) brauchen keine
  Framework-Unterstützung: Der Client liest sie aus einer
  gewöhnlichen Prop, sodass jeder Handler sie bereits liefern kann.
  Siehe `manual/frontend-inertia-responses.md`.

### Geändert

- `Envelope` hat ein Feld `queue: Option<String>` bekommen. Es ist
  `serde(default)` und wird bei Abwesenheit übersprungen, sodass eine
  ungeroutete Envelope Byte-identisch zu dem serialisiert, was
  frühere Versionen schrieben - der eingefrorene Wire-Format-Test
  besteht unverändert, es gibt keinen `schema_version`-Bump, und
  Flotten mit gemischten Versionen interoperieren während eines
  Rolling Upgrade.
- `WorkerConfig` hat ein Feld `queues: Vec<String>` bekommen (leer =
  alles leeren, das bisherige Verhalten).
- `ROADMAP.md` entfernt. Ihre Design-Prinzipien leben in
  `manual/introduction.md`, die Arbeitsvereinbarung in
  `manual/contributions.md`, und das Deployment- und
  Scale-out-Material in `manual/deployment.md`; die
  Ausgeliefert/Geplant-Checklisten waren veraltet. `README.md`s
  Verweis darauf für „die Beziehung zum Upstream“ war bereits ins
  Leere gegangen - diese Zuordnung lebt in `LICENSE`.
- Scaffold-Frontends pinnen `@inertiajs/{svelte,react,vue3}` jetzt
  auf `^3.6.1` (von `^3.4.0`). Der Bereich 3.4.0 → 3.6.1 ist nur
  clientseitig - geprüft gegen das vorgelagerte Änderungsprotokoll
  und den `Page`-Vertrag in `packages/core/src/types.ts`, jeder
  `X-Inertia-*`-Header, den der 3.6.1-Client sendet, wurde bereits
  behandelt.
- `scripts/release.sh` veröffentlicht das GitHub-Release jetzt
  selbst, mit Notizen aus dem Abschnitt des Änderungsprotokolls der
  Version. Vorher war das ein manueller „nächster Schritt“, der
  übersprungen wurde, weshalb v0.5.10 und v0.6.1–v0.6.3 nur getaggt
  sind und die Releases-Seite auf einer veralteten Version saß.
  Preflight läuft vor dem Gate, sodass ein fehlendes `gh` oder ein
  fehlender Abschnitt im Änderungsprotokoll in Sekunden fehlschlägt,
  und das Veröffentlichen wird automatisch übersprungen, sofern
  `origin` nicht GitHub ist.

### Upgrade

Bestehende `jobs`-Tabellen auf dem Datenbank-Queue-Treiber
**müssen** die neue Spalte hinzufügen - `push` nennt sie in seinem
`INSERT`, unabhängig davon, ob der Job geroutet ist, sodass eine
unmigrierte Tabelle bei jedem Push scheitert. Zuerst migrieren, dann
Binaries rollen (ältere Binaries listen ihre Spalten explizit auf und
ignorieren die neue, daher ist diese Reihenfolge sicher):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Korrigiert in 0.7.1 - dieser Hinweis behauptete ursprünglich, dass
ungefilterte Deployments keine Migration bräuchten.)*

## 0.6.5 - 2026-07-21

### Hinzugefügt

- **Gehosteter Einmalzahlungs-Checkout im Stripe-Adapter.**
  `Checkout::start_session` mit `SessionMode::OneOff` und nicht
  leeren `price_refs` legt jetzt eine gehostete Checkout-Session an
  (`mode=payment`, ein Line-Item pro Price-Ref,
  `allow_promotion_codes=true`) und liefert
  `SessionPayload::StripeCheckoutRedirect`. Der reine
  `amount_hint`-Elements-Pfad ist unverändert; die beiden Formen
  werden pro Anfrage gewählt.
- **Unterstützung für Stripe Managed Payments (Merchant of
  Record).** `StripeProvider::with_managed_payments(true)` - oder
  `STRIPE_MANAGED_PAYMENTS=true` in `from_env()` - sendet
  `managed_payments[enabled]=true` beim Anlegen einer gehosteten
  Einmalzahlungs-Session. Standardmäßig aus; das Feld wird komplett
  weggelassen, sodass nicht eingeschriebene Konten nicht betroffen
  sind.
- **`Checkout::session_status`.** Neue Trait-Methode (Standard:
  `PaymentError::NotSupported`), die den provider-seitigen Zustand
  einer Session als neuen neutralen `CheckoutSessionState` meldet
  (`Open` / `Complete { paid, payment_ref, amount_total }` /
  `Expired`). Die Stripe-Implementierung bildet
  `GET /v1/checkout/sessions/{id}` ab; `payment_ref` trägt die
  PaymentIntent-ID der Session zur Korrelation mit der
  Mirror-Tabelle. Das ist das serverseitige Verifikations-Primitiv
  für Redirect-Rückkehrseiten und Reconciliation-Durchläufe.
- **`Promotions`-Capability-Trait.** `create_promotion_code` prägt
  einen kundengebundenen, optional ablaufenden, einlösungsbegrenzten
  Code aus einem vorher angelegten Coupon. Abgefragt über das neue
  `PaymentProvider::as_promotions()` (Standard `None`).
  Implementiert für Stripe (`POST /v1/promotion_codes`) und den Mock.
- **`MockPaymentProvider`-Erweiterungen für das Obige.** Zeichnet
  jede `start_session`-Anfrage auf (`recorded_sessions()`), skriptet
  `session_status` pro Session-ID (`script_session_status()` - nicht
  skriptete bekannte Sessions melden `Open`, unbekannte IDs
  `NotFound`), und implementiert `Promotions` mit aufgezeichneten
  Anfragen (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

### Behoben

- **Eloquent-Aggregate dekodieren konsistent über
  Datenbank-Backends hinweg.** Generierte `count`-, `sum`-, `avg`-,
  `min`- und `max`-Ausdrücke nutzen jetzt einen stabilen internen
  Ergebnis-Alias. PostgreSQL liefert keine falschen Nullen oder
  `None` mehr, weil sein Treiber Aggregat-Spalten anders benennt als
  SQLite, und Fehler durch fehlende Spalten oder inkompatible Typen
  propagieren jetzt, statt still auf einen Default zu fallen.
- **Massenlöschungen können keine vom Aufrufer gelieferten
  Tabellenausdrücke verwenden.** Ausführbares Lösch-SQL leitet sein
  Ziel immer vom validierten statischen `M::TABLE` des Modells ab.
  Das alte öffentliche Renderer-Argument bleibt quellkompatibel, kann
  das Lösch-Ziel aber nicht mehr umleiten oder injizieren.

## 0.6.3 - 2026-07-15

### Hinzugefügt

- **Typisierte rohe Reads können auf der gepinnten Connection einer
  Transaktion bleiben.** `Transaction::backend()` legt das aktive
  Backend offen, und `Transaction::query_all(Statement)` führt
  typisiertes Aggregat- oder eigenes SQL über die Transaktion aus,
  unter Erhalt der `QueryExecuted`-Instrumentierung. Anwendungen
  brauchen keinen Query auf Pool-Ebene oder privaten
  Executor-Zugriff mehr, wenn eine sperr-gebundene Entscheidung von
  berechneten Ergebnis-Spalten abhängt.

## 0.6.2 - 2026-07-15

### Behoben

- **Gebundene rohe Prädikate sind Backend-neutral.** Eloquent
  `filter_raw` und `where_raw` akzeptieren jetzt portable
  `?`-Bind-Marker auf jedem Datenbank-Backend;
  PostgreSQL-Rendering rebased sie auf monotone `$N`-Positionen über
  vorangehende Prädikate, Relationship-Subqueries, HAVING-Klauseln
  und UNION-Arme hinweg. Bestehende nummerierte
  PostgreSQL-Fragmente werden nach ihrer lokalen Marker-Reihenfolge
  normalisiert, während gemischte Stile und nicht passende
  Bind-Anzahlen die Validierung vor jeder I/O scheitern lassen. Der
  SQL-bewusste Scanner erhält Fragezeichen innerhalb gequoteter
  Strings, Identifiern, Kommentaren und Dollar-gequoteten Bodies;
  `??` gibt in einem gebundenen rohen Fragment einen literalen
  Fragezeichen-Operator aus.

## 0.6.1 - 2026-07-15

### Hinzugefügt

- **Beobachtbares überwachtes Session-Cleanup.**
  `SessionMiddleware::install` nutzt den konfigurierbaren
  `SESSION_GC_INTERVAL`-Takt (standardmäßig eine Stunde), während
  `session_gc_metrics()` prozesslokale Zeitstempel für Lauf, Erfolg,
  Fehlschlag, entfernte Zeilen und letztes Ergebnis für geschützte
  Operations-Oberflächen offenlegt.
- **Begrenzte Touches für gleitende Sessions.**
  `SESSION_TOUCH_INTERVAL` steuert den minimalen Takt für
  Aktivitäts-Schreibvorgänge (standardmäßig fünf Minuten) und ist auf
  die Hälfte der Session-Lebensdauer gedeckelt, sodass aktive
  Sessions nicht zwischen zwei Touches ablaufen können.

### Behoben

- **Zustandsfreie Anfragen erzeugen keine dauerhaften Sessions
  mehr.** Anfragen ohne ein gültiges Session-Cookie führen weder
  einen Session-Store-Read noch -Write aus und bekommen kein
  Session-Cookie, sofern die Verarbeitung keinen Zustand erzeugt.
  Bestehende saubere Sessions vermeiden bedingungslose Upserts und
  Cookie-Churn, Legacy-Cookies migrieren bei ihrer nächsten Anfrage,
  und Cookies, deren zugrunde liegende Zeilen abgelaufen sind, werden
  bereinigt, ohne leere Sessions neu anzulegen.

## 0.6.0 - 2026-07-10

### Hinzugefügt

- **Opt-in-Framework-Subsysteme mit abwärtskompatiblen Defaults.**
  Filesystem-Storage, die Datenbank-Treiber SQLite/Postgres/MySQL,
  der MariaDB-Vector-Treiber und Web Push haben jetzt explizite
  Cargo-Features. Bestehende Default-Builds behalten alle diese
  Fähigkeiten, während `default-features = false`-Konsumenten null
  Treiber oder nur die Storage-/Datenbank-/Vector-/Push-Oberfläche
  wählen können, die sie nutzen. Die ausführbare Feature-Matrix
  verifiziert Null-Treiber-, Einzel-Treiber-, Nation-X-Minimal-,
  Default- und All-Feature-Profile.
- **Import roher P-256-VAPID-Private-Keys.** `VapidKey::from_bytes`
  akzeptiert einen validierten 32-Byte-Big-Endian-P-256-Skalar neben
  dem bestehenden PKCS#8-PEM-Import-/Export-Pfad.

### Geändert

- **VAPID-JWTs werden direkt mit P-256 signiert.** Web Push
  serialisiert jetzt Header/Claims nach RFC 8292 ES256 und signiert
  sie mit `p256`, wodurch die generische JWT-Abhängigkeit entfällt,
  während generierte Keys, PEM-Round-Trips, Public-Key-Kodierung und
  die 24-Stunden-Lebensdauergrenze erhalten bleiben.
- **Auffrischung der Sicherheits-Abhängigkeiten.** Verwundbare
  Framework-Abhängigkeiten aktualisiert, darunter bcrypt und ammonia,
  und die aktivierten Features von Comrak eingeengt, bei Erhalt der
  Syntax-Hervorhebung.
- **Rust 1.91.1 ist die MSRV des Release.** Jedes Workspace-Paket
  deklariert dieselbe `rust-version`, generierte Dockerfiles pinnen
  das passende Builder-Image, und das vollständige Release-Gate
  kompiliert das unterstützte Filesystem-Profil mit exakt dem
  Rust-1.91.1-Toolchain.
- **OpenDAL-0.58-Sicherheits-Pin.** Das Filesystem-Feature pinnt den
  Commit `88717391eb72c9839d3f8e79fccad9f22fc3a1b4` von
  `eas4ai/opendal`, einen minimalen Fork, der exakt auf dem
  offiziellen Apache-OpenDAL-Commit
  `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` basiert. Der Fork ändert
  nur die von OpenDAL-Core plus S3, GCS und Azure Blob genutzten
  Reqsign-Deklarationen, sodass nachgelagerte Konsumenten den
  offiziellen Apache-Reqsign-Commit
  `b49cd2996b9d2d9944e84481f8835ff55b188b97` und `quick-xml` 0.41.0
  auflösen. Ein Fork ist nötig, weil die Root-Cargo-Patches eines
  Abhängigkeits-Repositorys nicht an Konsumenten weitergereicht
  werden; der veröffentlichte Graph könnte sonst das verwundbare
  `quick-xml` 0.38/0.40 wiederherstellen.

### Behoben

- **Atomare Release-Versions-Metadaten.** Das Release-Bumping
  aktualisiert jetzt `workspace.package.version` und jede
  versionierte interne Pfad-Abhängigkeit in einer validierten
  Operation, staged jedes betroffene Manifest, und beweist einen
  temporären `0.6.0`-Workspace mit `cargo check --workspace` vor dem
  Release. Release-Versionen werden als striktes SemVer 2.0
  validiert, einschließlich der Regel gegen führende Nullen bei
  numerischen Prerelease-Kennungen. Versionsunabhängige, wegwerfbare
  Bare-Remote-Smoketests leiten ein späteres Patch-Release sowohl
  aus der aktuellen Quelle als auch aus einer bereits
  `0.6.0`-Quelle ab, weisen staged/unstaged/untracked Release-Bäume
  vor dem Gate zurück, beweisen, dass die atomare
  Commit-/Tag-Veröffentlichung beide Refs zurückrollt, wenn ein Tag
  abgelehnt wird, und beweisen die normale Release-Sequenz, ohne das
  echte Remote anzufassen. Release-Versionen müssen nach
  SemVer-Rangfolge steigen, einschließlich Prerelease-Übergängen.
  Smoke-Build-Artefakte bleiben immer innerhalb ihres temporären
  Workspace und ignorieren jedes aufrufende `CARGO_TARGET_DIR`.
- **Rustdoc deckt jede unterstützte Feature-Grenze ab.** Das
  OAuth-Modul verlinkt zum öffentlichen `OAuthAuth::complete`, und
  die ausführbare Matrix baut Null-Treiber-, Default- und
  All-Feature-Rustdoc ohne Abhängigkeiten.
- **Filesystem-Stream-Validierung ist Session-gebunden.** Lokale
  Filesystem-Writer, -Lister und -Kopierer lösen ihre Pfade auf und
  schränken sie einmal vor dem ersten I/O ein, statt einmal pro
  Chunk/Item, während aktivierte Close-/Abort-Operationen immer das
  Backend zum Aufräumen erreichen. Bestehende Traversal- und
  Symlink-Eingrenzung bleiben für ein vertrauenswürdiges Filesystem
  durchgesetzt; Canonicalize-dann-Open-Prüfungen eliminieren keine
  Races gegen einen Principal, der den Baum gleichzeitig mutiert.

### Sicherheit

- **Das Release-Gate schlägt geschlossen fehl.** `release.sh`
  delegiert an das kanonische Voll-Gate, bevor Manifeste bearbeitet
  oder Commits/Tags erstellt werden; dieses Gate führt immer
  `cargo audit` aus, behandelt ein fehlendes `cargo-audit`-Binary als
  Fehler und stoppt bei jedem Audit-Fehlschlag. Es baut und auditiert
  außerdem einen isolierten nachgelagerten Filesystem-Konsumenten,
  wobei es exakte OpenDAL-/Reqsign-Quell-Revisionen und kein
  `quick-xml` unter 0.41 assertiert. Keine neuen Advisory-Ignores
  wurden hinzugefügt.

## 0.5.10 - 2026-07-03

### Behoben

- **`generate-types` verwirft selbstreferenzierende Strukturen nicht
  mehr.** Eine Struktur mit einem Feld, das auf ihren eigenen Typ
  verweist (ein Baumknoten mit `children: Vec<Self>`, z. B. eine
  Threaded-Comment-Ansicht), erzeugte eine Selbstkante im
  Typ-Abhängigkeitsgraphen, die ihren Eingangsgrad über null hielt,
  sodass Kahns topologische Sortierung sie nie ausgab - was jedes
  Interface, das auf sie verwies, mit einem baumelnden Typnamen
  zurückließ, der bei `svelte-check`/`tsc` scheiterte. Selbstkanten
  werden jetzt vor dem Sortieren entfernt, und jede in einem
  Referenz-Zyklus gefangene Struktur (wechselseitige Rekursion) wird
  in beliebiger Reihenfolge statt verworfen ausgegeben, da
  TS-Interfaces sich unabhängig von der Deklarationsreihenfolge
  aufeinander beziehen dürfen.

## 0.5.9 - 2026-07-01

### Hinzugefügt

- **`MAIL_FROM_NAME` - optionaler Anzeigename auf
  Auth-Flow-E-Mails.** Die Mailables für E-Mail-Verifizierung,
  Passwort-Reset und Passwort-geändert rendern ihren `From`-Header
  jetzt als `"Name <address>"`, wenn `MAIL_FROM_NAME` gesetzt ist
  (gelesen zum Sendezeitpunkt, damit es den Serde-Round-Trip der
  Warteschlange übersteht). `MAIL_FROM` bleibt eine reine Adresse;
  `MAIL_FROM_NAME` ungesetzt oder leer zu lassen behält das bisherige
  Verhalten mit reiner Adresse bei. Keine Änderung an irgendeiner
  Aufrufstelle - die Mailables lesen die Env-Var selbst.

## 0.5.8 - 2026-06-30

### Behoben

- **Die Route-Helfer von `generate-types` sind immer gültiges
  TypeScript.** Wenn sich mehrere Routen in einem Modul einen
  Handler teilen (z. B. eine `static_files::serve`-Whitelist, die
  viele Favicon-/Asset-URLs abbildet), behielt die erste den
  Handler-Namen, und der Rest bekam einen vom Routenpfad abgeleiteten
  Key - aber der Pfad war nur teilweise saniert (`/ { } -` → `_`),
  sodass eine Dateiendung einen `.` in den Key durchsickern ließ:
  `favicon_16x16.png: (...) => ...`. Das ist Member-Zugriff, kein
  Property-Name, sodass `tsc`/`svelte-check` das generierte
  `routes.ts` zurückwies. Abgeleitete Keys werden jetzt zu legalen
  Identifiern saniert - jedes nicht-alphanumerische Zeichen wird zu
  `_`, und eine führende Ziffer wird mit einem Präfix versehen -
  sodass `favicon-16x16.png` → `favicon_16x16_png` und `2fa.json` →
  `_2fa_json` wird. Eindeutige Handler-Namen bleiben unberührt.

## 0.5.7 - 2026-06-30

### Behoben

- **`generate-types` gibt keine baumelnden Typ-Referenzen mehr aus.**
  Ein Prop-Feld, dessen Typ eine Struktur ist, die nicht
  `InertiaProps`/`Data` derivt (oder ein externer Typ, den der
  Generator nicht sehen kann), wurde als bloßer Identifier
  ausgegeben - z. B. `user: UserInfo` -, was TypeScript erzeugte,
  das bei `tsc`/`svelte-check` scheiterte, weil dieses Interface nie
  geschrieben wird. Solche Referenzen degradieren jetzt zu `unknown`
  (`user: unknown`; `Vec<T>` → `Array<unknown>`; `Option<T>` →
  `unknown | null`), sodass die generierte Ausgabe immer die
  Typprüfung besteht, und `generate-types` gibt eine Warnung aus, die
  den nicht aufgelösten Typ und das Feld nennt, das ihn referenziert,
  samt dem Fix (`InertiaProps`/`Data` darauf derivieren). Generische
  Parameter und aufgelöste verschachtelte InertiaProps-/Data-Typen
  sind nicht betroffen.

## 0.5.6 - 2026-06-29

### Geändert

- **Anmeldung mit Apple: RS256-JWKS-Verifikation.**
  `suprnova-apple-rs` auf v0.3.1 angehoben - Apple-ID-Tokens werden
  jetzt gegen Apples veröffentlichte JWKS (RS256) verifiziert, statt
  strukturell vertraut zu werden.

## 0.5.5 - 2026-06-28

### Hinzugefügt

- **`MagicLink`-Token-Zweck.** Neue `MagicLink`-Variante auf dem
  Auth-Flow-Enum `TokenPurpose`, für passwortlose
  Magic-Link-Anmeldetokens.

## 0.5.4 - 2026-06-28

### Geändert

- **Komponierbarer OAuth-Abschluss.** Den generischen
  OAuth-Abschluss aufgeteilt in `verify_oauth_identity` (verifizieren +
  Identität auflösen) und ein schlankes `complete`, sodass Apps
  eine OAuth-Identität verifizieren können, ohne die vollständigen
  Session-Abschluss-Seiteneffekte auszulösen.

## 0.5.3 - 2026-06-28

### Behoben

- **Korrekte Workspace-Versions-Metadaten.** v0.5.2 wurde getaggt
  und gepusht, bevor sein `Cargo.toml`-Versions-Bump gestaged war,
  sodass der gepushte v0.5.2-Tag weiterhin `version = "0.5.1"` liest.
  v0.5.3 schneidet das Release mit der korrekten Workspace-Version
  neu - keine Code-Änderung (die OAuth-Aufteilung von v0.5.2 ist
  nicht betroffen).

## 0.5.2 - 2026-06-28

### Geändert

- **Komponierbarer Apple-Abschluss.** Den Apple-Sign-In-Abschluss
  aufgeteilt in `verify_apple_identity` + ein schlankes
  `complete_apple`, spiegelnd zur generischen OAuth-Aufteilung.
  (Hinweis: Der gepushte v0.5.2-Tag trägt ein veraltetes Versionsfeld
  `0.5.1` - behoben in v0.5.3.)

## 0.5.1 - 2026-06-28

### Geändert

- **Apple-Crate umbenannt.** Die Apple-Abhängigkeit auf das
  umbenannte Repository `suprnova-apple-rs` umgebogen.

## 0.5.0 - 2026-06-28

### Hinzugefügt

- **Anmeldung mit Apple.** OAuth-Token-Austausch +
  ID-Token-Verifikation + User-Upsert für Apple; Apples
  Well-known-Endpunkte und der `form_post`-Response-Modus;
  Apple-spezifische Felder auf `OAuthProviderConfig`; `AppleKeyPair`
  re-exportiert, damit Apps Apple Sign-In ohne direkte
  `apple`-Abhängigkeit konfigurieren.

### Behoben

- PKCE-Parameter aus der Apple-Autorisierungs-URL weggelassen (Apple
  weist die Anfrage zurück, wenn sie vorhanden sind).

### Abhängigkeiten

- Den Magic-Auth-Fix von `torii` konsumiert; `apple-rs` v0.3.0
  hinzugefügt.

## 0.4.1 - 2026-06-26

### Performance

- `MiddlewareChain` vorab dimensioniert, um Pro-Anfrage-Reallokationen
  von `Vec` zu eliminieren.

### Behoben

- Den Pfad der `down`-Datei des Wartungsmodus kollisionssicher
  gemacht unter parallelen Testläufen.

### Docs

- Die Doc-Beispiele des Frameworks compile-geprüft (`ignore` →
  `no_run`); die Distributions-Hinweise mit den getaggten
  GitHub-Releases abgeglichen; den gesamten `docs/`-Baum ignoriert.

## 0.4.0 - 2026-06-22

### Geändert

- **Distribution ist Git-verfolgt; Sie pinnen nicht auf Tags.**
  Gescaffoldete Apps hängen von `suprnova = { git =
  "…/suprnova.git" }` ab und verfolgen den Default-Branch; Updates
  werden mit `cargo update -p suprnova` gezogen. Versionen werden als
  getaggte GitHub-Releases (`v0.4.0`, …) für das Änderungsprotokoll
  veröffentlicht, aber `Cargo.lock` pinnt bereits den exakt
  aufgelösten Commit - sodass Builds reproduzierbar bleiben, ohne von
  Hand einen `tag` oder `rev` zu pinnen. Die Installations-Docs
  stellen Commit-Pinning nicht mehr als Update-Pfad dar.

## 0.3.0 - 2026-06-21

### Hinzugefügt

- **Query-Instrumentierung für Eloquent-Reads** - `Builder::get`,
  `Model::find`, `find_many` und `all` geben jetzt `QueryExecuted`
  aus, sodass Modell-SELECTs und Eager-Load-Queries in `DB::listen`
  und dem In-Memory-Query-Log neben Writes und rohen Queries
  auftauchen. Fügt das instrumentierte Read-Terminal
  `ExecutorChoice::statement_all` hinzu.
- **Resource-Route-Autorisierung** -
  `ResourceRoutes::authorize_resource::<U, R>()` hängt die
  konventionelle Fähigkeits-Prüfung als Pro-Route-Middleware an jede
  generierte Resource-Route (Parität zu Laravels
  `authorizeResource`). Die Aktion-zu-Fähigkeit-Abbildung ist
  `index`/`show` → `view`, `create`/`store` → `create`,
  `edit`/`update` → `update`, `destroy` → `delete`. Ein Aufruf
  sichert die ganze Sieben-Aktionen-Oberfläche per Gate ab, statt
  sich darauf zu verlassen, dass jeder Controller-Körper an ein
  `Gate::authorize` denkt.
- **Atomarer Rate-Limit-Hit** - `RateLimiter::hit_and_check(key, max,
  decay)` inkrementiert ein festes Fenster und prüft es in einem
  einzigen Round-Trip, und liefert, ob der Bucket jetzt über seinem
  Limit liegt (`i64::MAX` bedeutet unbegrenzt).
- **Zeitkonstanter Vergleichs-Helfer** - `constant_time_eq(a, b)`
  (subtle-gestützt) für die Webhook-Signatur-Verifikation; die Docs
  von `WebhookHandler::verify` verlangen jetzt einen zeitkonstanten
  Digest-Vergleich.
- **Inertia-Client auf 3.4.0** - die Svelte-/React-/Vue-Scaffolds
  pinnen jetzt `@inertiajs/{svelte,react,vue3}` auf `^3.4.0` (von
  `3.1.1`), und nehmen `router.poll`-Modi, dynamisches `usePoll`,
  `Inertia.once`, den InfiniteScroll-Cancel-Fix und awaited
  Form-`onSuccess` mit. Der Server gibt bereits die vollständige
  3.4.0-Page-Objekt- und Header-Oberfläche aus (Once-Props, die
  Prepend-/Deep-Merge-Scroll-Familie, `matchPropsOn`,
  rescued/geteilte Props), das ist also ein
  Client-Aktualitäts-Bump ohne Protokolländerung.
- **Optionale Verbindungs-Obergrenze** - `SERVER_MAX_CONNECTIONS`
  (und das programmatische `Server::max_connections(n)`) begrenzt
  gleichzeitig aktive Verbindungen mit einem Semaphor auf der
  Accept-Schleife und übt Backpressure auf TCP-Ebene aus. Ungesetzt -
  oder `0` - lässt Verbindungen unbegrenzt (der Standard,
  unverändert). Ein Rückhalt zum Kombinieren mit einem
  Reverse-Proxy und `LimitNOFILE`, kein Ersatz für vorgelagertes
  Rate-Limiting.
- **Redirect-Folgen abwählen** - `RequestBuilder::no_redirects()`
  leitet eine Anfrage durch einen nicht folgenden HTTP-Client, sodass
  ein `3xx` unverändert zurückgegeben wird, statt verfolgt zu werden.
  Verwenden Sie es, wenn die Anfrage-URL von nicht vertrauenswürdiger
  Eingabe beeinflusst wird, um einen Redirect-basierten SSRF-Vektor
  zu schließen (ein feindlicher Endpunkt, der auf einen internen Host
  oder einen Cloud-Metadaten-Host umleitet). Der Standard-Client
  folgt weiterhin Redirects, passend zur allgemeinen
  Client-Konvention.

### Sicherheit

- **Resource-Routen** schlagen beim typgelöschten Downcast der
  Autorisierungs-Registry geschlossen fehl statt zu paniken, und
  `authorize_resource`-Ablehnungen / nicht authentifizierte Anfragen
  werden abgewiesen, bevor der Handler läuft.
- **Der Rate-Limiter** schließt ein Fixed-Window-Check-then-Hit-Race,
  indem er atomar inkrementiert und vergleicht (`hit_and_check`).
- **Die Queue-Middleware `RateLimited`** lässt Jobs jetzt über
  dieses atomare `hit_and_check` zu, statt über ein getrenntes Paar
  `too_many_attempts` + `hit`, sodass nicht mehr alle nebenläufigen
  Worker die Budget-Prüfung bestehen können, bevor auch nur einer von
  ihnen inkrementiert, und über `max_attempts` hinaus zulassen.
- **Upload-Validatoren** (`mimetypes` / `mime`) schnüffeln jetzt den
  Content der hochgeladenen Bytes, statt dem clientseitig gelieferten
  `Content-Type` zu vertrauen.
- **Der Filesystem-Pfad-Schutz** kanonisiert Pfade, um
  Symlink-Traversal aus der Storage-Wurzel heraus zu fangen, über die
  vorherigen lexikalischen `../`-/absoluten/UNC-Prüfungen hinaus.
- **Auth** schließt ein Timing-Orakel beim passwortlosen Login - ein
  passendes, aber passwortloses Konto, dem ein Passwort übergeben
  wird, durchläuft jetzt eine Verifikation mit festen Kosten, über
  sowohl den Eloquent- als auch den Datenbank-User-Provider hinweg -
  und `dummy_verify` steuert den konfigurierten Hasher, sodass der
  Pfad für nicht passende Nutzer zeitkonstant ist.
- **Eloquent** validiert Spalten-Identifier auf den
  Projektionspfaden `pluck` / `value` / `pluck_keyed` / `sole_value`
  und `sum` / `avg` / `min` / `max`.
- **Zahlungen** - der Verifizierer des Mock-Providers schlägt
  außerhalb einer Entwicklungsumgebung geschlossen fehl, und
  Webhook-Quell-IPs lösen jetzt über `TrustedProxiesConfig`
  (`req.ip()`) auf, statt über einen rohen `X-Forwarded-For`-Header.
- **Der Filesystem-Pfad-Schutz** läuft jetzt bis zum nächsten
  *existierenden* Vorfahren hoch, wenn ein Schreibziel noch nicht
  existiert, und schließt damit eine Symlink-Flucht, bei der ein
  platzierter Zwischen-Symlink mit fehlendem unmittelbarem Elternteil
  am Schutz vorbeischlüpfte.
- **`DB::init_with`** validiert die Umgebung vor dem Verbinden
  (passend zu `DB::init`), sodass der Dev-SQLite-Fallback über
  diesen Einstiegspunkt nicht mehr still in Produktion booten kann.
- **Das Ausliefern statischer Dateien** weist Dotfiles zurück
  (`.env`, `.git/config`, `.htpasswd`, jedes mit `.` beginnende
  Segment), nicht nur `.`/`..`-Traversal.
- **Zahlungs-Webhooks** serialisieren nebenläufige Wiederholungen
  desselben unverarbeiteten Events mit einer `FOR UPDATE`-Sperre plus
  erneuter Prüfung, und behandeln Unique-Verletzungen auf der
  Mirror-Tabelle als harmlos-bereits-angewendet;
  `payments_subscription_items` bekommt ein
  `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** setzt den Modell-Diskriminator standardmäßig auf den
  vollqualifizierten Typnamen, sodass zwei authentifizierbare Typen
  mit gemeinsamem Blattnamen nicht mehr gegenseitig ihre
  Rollen/Berechtigungen erben können.
- **`invalidate_session()`** rotiert jetzt die Session-ID (statt nur
  zu leeren), was eine Session-Fixation-Lücke schließt; die
  Queue-Middleware `WithoutOverlapping` gibt ihre Cache-Sperre auch
  frei, wenn der Job paniekt.
- **Mail-Provider** deckeln Error-Response-Body-Reads (8 KiB),
  passend zum Web-Push-Client, sodass ein feindlicher Endpunkt nicht
  den Speicher des Senders treiben kann.
- **Web Push** deaktiviert das HTTP-Redirect-Folgen am
  Standard-Client, sodass ein angreiferbeeinflusster Push-Endpunkt
  einen Notification-POST nicht mehr per `3xx` auf einen internen
  Host oder Cloud-Metadaten-Host umleiten kann (SSRF). Ein Redirect
  taucht jetzt als abgelehnter Push auf, statt als still verfolgte
  Anfrage.
- **Der Stripe-Adapter** schwärzt in `Debug` das
  Webhook-Signing-Secret *und* druckt einen Platzhalter für den
  `stripe::Client` (der den API-Secret-Key in seinem Auth-Header
  trägt), sodass keines der beiden Secrets über ein `{:?}` von
  `StripeProvider` ins Log gelangen kann, unabhängig vom eigenen
  `Debug` des vorgelagerten Clients.
- **Der Stripe-Adapter** weist in `from_env` vorhandene, aber leere
  Credentials zurück und schlägt geschlossen fehl, statt einen
  Client mit leerem (und damit fälschbarem) Webhook-HMAC-Secret zu
  bauen.
- **Die OAuth-E-Mail-Verifikation** schlägt für nicht erkannte
  Provider geschlossen fehl: Ein Userinfo-Payload, der ein `email`,
  aber kein `email_verified`-Flag trägt, gilt nicht mehr als
  verifiziert. Ein unbekannter Provider muss jetzt
  `email_verified: true` behaupten oder einen
  Verified-Emails-Endpunkt offenlegen, was einen
  Account-Verknüpfungs-/Übernahme-Vektor für Apps schließt, die
  Konten über E-Mail schlüsseln. Google (nur explizites `true`) und
  GitHub (verifiziert per `/user`-Vertrag) sind unverändert.

### Behoben

- **Verschachteltes Eager Loading** (`with(["posts.comments"])`) ist
  jetzt eine konstante Anzahl von Queries - das letzte Segment lädt
  in einer gebündelten IN-Query über alle Eltern hinweg, statt einer
  Query pro Elternteil (N+1).
- **`where_has`/`where_doesnt_have`** qualifizieren Closure-Spalten
  jetzt mit der Ziel-Tabelle, sodass eine Spalte, die sowohl auf
  Pivot als auch auf dem Ziel existiert, bei Many-to-many-Relationen
  keinen Ambiguous-Column-Fehler mehr erzeugt.
- **Soft-Delete-`delete`/`force_delete`/`touch` und
  Factory-`persist`** respektieren jetzt das
  `#[model(connection = "…")]`-Routing eines Modells (passend zu
  `restore` und den anderen Schreibpfaden), statt auf den primären
  Pool zurückzufallen.
- **JSON:API-`Maybe::Missing`** nutzt jetzt einen
  nicht-kollidierbaren Wire-Sentinel, sodass Nutzerdaten in der Form
  `{"__missing__": true}` nicht mehr still entfernt werden.
- **Eingereihte Notifications** respektieren jetzt `should_send`
  (Pro-Kanal-Veto) und `after_sending`, erneut geprüft auf dem
  Worker - vorher tat das nur der synchrone Pfad.
- **Released Jobs** pushen die Wiederholungs-Kopie, bevor das
  Original geackt wird, sodass ein vorübergehender
  Treiber-Push-Fehler den Job nicht mehr verliert.
- **Paddle-Adjustment-(Rückerstattungs-)Webhooks** schlüsseln das
  Mirror-Update jetzt auf die referenzierte Transaktions-ID und lesen
  Beträge aus `data.totals`, statt eine Nullbetrags-Zeile unter der
  Adjustment-ID einzufügen.
- **SQLite-URLs** mit Query-String (`sqlite://db.sqlite?mode=rwc`)
  bauen jetzt eine gültige Single-Query-Connection-URL und einen
  sauberen Dateinamen auf der Platte.
- **HTTP** klammert `Accept`-`q`-Werte jetzt auf `[0,1]` und erzwingt
  `max_body_bytes` eines `FormRequest` auch, wenn der Body
  vorgepuffert war; die **WebSocket**-Konfiguration weist
  `max_missed_pings < 2` zurück (1 schloss jede Verbindung bei ihrem
  ersten Ping).
- **Cron** nutzt jetzt ODER-Semantik für Tag-des-Monats und
  Wochentag, wenn beide eingeschränkt sind (Parität zu Vixie/POSIX);
  Markdown-`plain_text`/Auszüge erhalten absichtlich mit Leerzeichen
  gesetzte Interpunktion; `CachedEvaluator` begrenzt jetzt sein
  Cache-Wachstum; `SupervisorRegistry::start_all` spawnt bei einem
  zweiten Aufruf nicht mehr doppelt; der Test-Container erholt sich
  an Ort und Stelle von einer vergifteten Sperre.
- **Der Neustart-Backoff des Supervisors** setzt sich jetzt auf den
  100-ms-Boden zurück, nachdem ein Lauf mindestens die 60-s-Grenze
  oben gehalten hat, sodass ein Daemon, der lange gesund lief und
  dann beendet, prompt neu startet, statt einen Backoff zu erben, der
  während eines früheren Fehlschlag-Ausbruchs gestiegen war. Eine
  Absturzschleife, deren Läufe die Schwelle nie erreichen, rampt
  weiterhin auf die Grenze hoch, sodass der Reset einen flatternden
  Supervisor nie verdeckt.
- Veraltete Docs korrigiert zu `filter_op` (Operatoren sind
  Allowlist-validiert), signierten URLs (nicht Byte-kompatibel mit
  Laravels absoluten Standard-Signaturen), `UniqueIdKind::is_valid`
  (ein Aufrufer-Helfer, nicht automatisch in `find` verdrahtet), und
  der Identifier-Längengrenze (128, nicht 64).

### Dokumentation

- Resource-Route-Autorisierung (`authorize_resource`) in den
  Routing- und Autorisierungs-Kapiteln dokumentiert, sowie den
  atomaren `hit_and_check`-Zähler im Rate-Limiting-Kapitel.

## 0.2.0 - 2026-06-21

Fügt rollenbasierte Zugriffskontrolle, eine
Markdown-Content-/Docs-Rendering-Pipeline und natives Ausliefern
statischer Dateien hinzu.

### Hinzugefügt

- **Tier-2-RBAC** - Trait `HasRoles`; Rollen + Berechtigungen mit
  einem `role_has_permissions`-Join; `PermissionMiddleware` /
  `RoleMiddleware` (beide fail-closed / default-deny); die Migration
  `CreateRbacTables`; und die Helfer `create_role` /
  `create_permission` / `give_permission_to_role`.
- **Content-Rendering** - Markdown-Rendering und eine
  Docs-Build-Pipeline: `MarkdownRenderer`, `build_docs`,
  `DocsCatalog` / `DocsChapter`, Heading-Extraktion und
  `slugify_heading`. Gerendertes HTML wird sanitisiert (comrak +
  syntect + ammonia).
- **Natives Ausliefern statischer Dateien** -
  `StaticFiles::public()`-Fallback-Handler zum Ausliefern eines
  `public/`-Verzeichnisses an der Web-Wurzel, ersetzt handgerollte
  Pro-Asset-Whitelist-Controller in Apps.

### Behoben

- Frisch generierte Apps erben jetzt einen Kompatibilitäts-Pin
  `time = 0.3.47` auf Framework-Ebene, was Kohärenz-Konflikte von
  Rust 1.96 mit `time 0.3.48` in frischen
  Scaffold-Abhängigkeitsauflösungen vermeidet.

### Dokumentation

- Die zwei ausgelieferten Starter-Kits dokumentiert - **Nebula**
  (Auth auf Breeze-Niveau) und **Pulsar** (Produktsite + Community) -
  über Manual, README und Roadmap hinweg; die Roadmap um die
  ausgelieferte Oberfläche herum umstrukturiert; und
  Versionsreferenzen in der gesamten Dokumentation abgeglichen.

## 0.1.0 - 2026-06-10

Das initiale Suprnova-Release. Suprnova ist ein Laravel-inspiriertes
Web-Framework für Rust, geforkt von Kit und in eine eigene Richtung
weiterentwickelt. Das heutige Paritätsziel ist Laravel 13.x.

Dieses Release nutzt das Git-Distributionsmodell: Framework-Konsumenten
hängen von
`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`
ab, und die CLI installiert sich mit `cargo install --git`.

### Hinzugefügt

#### HTTP, Routing und Middleware

- `Router` mit Routen-Gruppen, Präfixen, Parameter-Constraints,
  benannten Routen
- Compile-Zeit-validierte Routen-Registrierung über das
  `routes!`-Makro
- Resource-Routing (`Router::resource`), erzeugt die sieben
  Standard-Routen
- Signierte URLs (freie Funktionen `url::signed_route` /
  `url::temporary_signed_route`, plus `Redirect::signed_route` /
  `Redirect::temporary_signed_route`)
- Redirect-Helfer - `Redirect::to`, `Redirect::back`,
  `Redirect::route`, `Redirect::with_input`, `Redirect::with_errors`,
  `with_flash`
- Middleware-Trait mit globalen, Gruppen- und Pro-Route-Schichten
- Eingebaute Middleware - CORS, CSRF, Session, Request-Timeout,
  Request-ID, Throttle / Login-Throttle, Signed-URL-Verify,
  Authenticated, Email-Verified, Brute-Force
- Abort-Helfer (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - öffentlicher Adapter, um eine
  einzelne Hyper-Anfrage gegen einen Router + eine Middleware-Chain
  zu bedienen

#### Inertia.js-Frontend-Brücke

- `#[derive(InertiaProps)]` mit TypeScript-Typ-Emission
- `inertia_response!`-Makro mit Compile-Zeit-Komponentenvalidierung
- Drei erstklassige Starter-Frontends - **Svelte 5** (Runes an),
  **React 19**, **Vue 3.5** - alle auf Inertia 3.1.1 + Vite 8 +
  Tailwind v4
- Partial Reloads (`only` / `except`), Deferred Props, persistentes
  Layout, verschlüsselte History, Scroll-Erhalt
- `Inertia::paginate(component, key, paginator)` für die
  Paginator-→-Inertia-Prop-Verdrahtung

#### ORM im Eloquent-Stil (über SeaORM)

- Attribut-Makro `#[suprnova::model]`, das in einem Schritt eine
  SeaORM-Entity und die nutzerseitige Eloquent-Struktur ausgibt
- Vollständiger `Model`-Trait - `create`, `find`, `find_or_fail`,
  `find_many`, `all`, `query`, `save`, `update`, `delete`,
  `force_delete`, `refresh`, `fresh`, `replicate`, `replicate_into`,
  `increment`/`decrement`, `destroy`, `is`/`is_not`,
  `to_array`/`to_json`
- Fillable-/Guarded-Massenzuweisung mit `Attrs`-Envelope
- 22 Attribut-Casts - Booleans, Integers, Floats, Daten, Enums,
  Hashed, Encrypted, JSON, Collections, Geld, Datetime mit Zeitzone
- Accessors / Mutators über `#[suprnova::model]`
- Auto-Zeitstempel (`created_at`, `updated_at`)
- Soft Deletes (`deleted_at`) mit `force_delete`, `restore`,
  `trashed`, `only_trashed`, `with_trashed`
- Elf Relations-Arten - `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Pro-Familie Morph-Enums + Morph-Registry mit
  `APP_KEY_PREVIOUS`-Rotation
- Eager Loading über `.with(...)`, `.with_count(...)`,
  `.load_missing(...)`
- Korrelierte EXISTS-Engine für `has` / `where_has`
- Sechzehn Lifecycle-Events (retrieving, retrieved, creating,
  created, updating, updated, saving, saved, deleting, deleted,
  restoring, restored, force-deleting, force-deleted, replicating,
  trashed)
- `Observer<M>`-Trait mit Pro-Methode-Auto-Registrierung über
  Inventory
- Lokale Scopes über `#[scopes(M)]`, globale Scopes über
  `GlobalScope`
- `Collection<M>`-Laravel-Oberfläche - `pluck`, `key_by`,
  `group_by`, `where_in`, `first_where`, `contains_where`,
  `partition`, usw.
- Drei Paginatoren - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - alle serialisieren zu JSON in Laravel-Form
- `chunk` / `lazy` / `cursor` für Bulk-Zeilen-Iteration ohne OOM
- `lock_for_update` / `shared_lock` Zeilen-Sperren
- `DB::table(...)`-Query-Builder mit `DynamicRow` für Ad-hoc-Queries
- `DB::transaction(...)` mit Savepoints, Retry-bei-Deadlock,
  Multi-Connection-Read-/Write-Split
- `DB::listen(...)` + Events `QueryExecuted` / `TransactionBegan` /
  `TransactionCommitted` / `TransactionRolledBack`
- `Prunable`-Trait + Console-Befehl `model:prune`
- Query-Helfer-Methoden `dump` / `dd`
- `#[model(unique_id="...")]` für UUID-/ULID-Primärschlüssel

#### Auth

- `Authenticatable`-Trait + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Mehrere benannte Guards (Web-Session, API-Token)
- E-Mail-Verifizierungs-Flow - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, signierte Verifizierungs-URLs,
  `EmailVerificationMail`
- Passwort-Reset-Flow - `PasswordReset`, gedrosselte Tokens,
  `PasswordChangedMail`, Event `PasswordResetLinkSent`
- Zwei-Faktor-TOTP - Registrieren, Verifizieren, Recovery-Codes,
  Replay-Schutz
- Brute-Force-/Login-Throttle - geschlüsselt auf IP + Identifier,
  `LoginThrottleMiddleware`
- Remember-me-Cookies mit stabilen opaken Tokens
- Sechs Auth-Events - `LoginAttempted`, `LoggedIn`, `Authenticated`,
  `LoggedOut`, `PasswordResetLinkSent`, `EmailVerified`
- Browser-Sessions, gestützt auf den Torii-Fork unter
  `github.com/eas4ai/suprnova-torii-rs`

#### Autorisierung

- `Gate`-Facade - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (synchrone + asynchrone Varianten)
- `#[policy(Model)]`-Makro für Policy-Registrierung
- Resource-Route-Auto-Autorisierung

#### Zahlungen

- Provider-agnostische Fünf-Trait-Oberfläche - `Checkout`,
  `Payment`, `Subscription`, `CustomerStore`, `WebhookHandler`
- `PaymentProvider`-Dachtrait + Capability-Abfrage über
  `as_payment()`
- DB-Mirror - `customers`, `subscriptions`, `subscription_items`,
  `payments`, `refunds`, `payment_webhook_events` (UNIQUE für
  Idempotenz)
- Flow-getaggtes Enum `SessionPayload` (einmalig vs. Abonnement)
- Zwei Referenz-Adapter als Workspace-Crates -
  `suprnova-payments-stripe` (Gateway, vollständige
  `Payment`-Implementierung), `suprnova-payments-paddle` (Merchant of
  Record, keine `Payment`-Implementierung)
- Mock-Provider für Tests

#### Warteschlange, Jobs, Batches, Chains

- `Job`-Trait - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Treiber - `sync`, `null`, `redis`, `database`
- `JobMiddleware`-Trait - sechs eingebaute Middleware
- Batches und Chains - `Queue::batch(jobs).dispatch()`, fluenter
  Chain-Builder, Cancellation, Fortschritts-Tracking
- Failed-Jobs-Store mit Replay
- Worker mit Graceful Shutdown, konfigurierbarer Nebenläufigkeit,
  Panic-Recovery über `catch_unwind`, Abschluss-Metriken
- Zwölf Queue-Events, die Queueing, Processing, Fehlschlag, Release
  und Worker-Lifecycle abdecken

#### Broadcasting und WebSockets

- `ws!()`-Makro + `Router::ws` für typisierte
  WebSocket-Endpunkte
- `WsSocket`-Sink-/Stream-Split
- Auto-Restart-Supervisoren über den `Supervisor`-Trait
- `BroadcastHub` mit `Channel`-, `Private`-, `Presence`-Kanälen
- JSON-Envelope-Protokoll, Presence Join/Leave/Here, konfigurierbare
  Presence-TTL mit Crash-Recovery
- `Broadcastable`-Brücke zum `EventDispatcher`
- Close-on-no-pong-Herzschlag mit konfigurierbarem
  WS_TASKS-Drain
- Pro-Route-WebSocket-Middleware
- 1-MiB-/64-KiB-sicherere Defaults + Factory `WsConfig::generous()`
- Origin-Policy + 1011-Close-bei-Protokollverletzung

#### Benachrichtigungen und Mail

- `Notification`-Trait + `Notify::send(recipient,
  notification).await`
- Mailable + Markdown-Template-Rendering
- Database-/Mail-/Broadcast-/Web-Push-Kanäle
- VAPID-Signierung + RFC-8291-ECE-Payload-Verschlüsselung (über
  `suprnova-web-push`)
- VAPID-Subject-Validierung, Retry-After-Parsing, 8-KiB-Obergrenze
  für Rejection-Bodies
- Notifiable-Trait für Empfänger-Typisierung

#### Ereignisse

- Typisierter Event-Dispatcher - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Abbrechbare saving-/updating-Events (liefern
  `EventResult::cancel`)
- Queueable Listener

#### Dateisystem

- `Storage::disk("name")` mit Multi-Treiber-Unterstützung - Local,
  S3, Azure, GCS über OpenDAL
- Move, Copy, Exists, Size, Mime, Last-Modified, Prepend/Append
- Streaming-Uploads und -Downloads

#### Cache

- `Cache::store("name")` + Treiber-Registrierung
- Treiber - Memory, Redis (mit begrenztem Connect-Timeout),
  Database, File
- `remember`, `forever`, `tags`, atomares Increment/Decrement,
  Sperren

#### Vector-DB

- `VectorDriver`-Trait mit vier Treibern - In-Memory, Qdrant
  (UUID-5-ID-Mapping), Pinecone (native String-IDs),
  MariaDB-native `VECTOR(N)` + HNSW-Indizes (11.7+)
- Cosine-/Dot-/Euklidische Distanz

#### Console-Binary und CLI

- Projekteigene `console`-Binary - Rust-Analogon zu `php artisan`,
  führt nutzerdefinierte Befehle über
  `#[suprnova::console::command]` aus
- `#[derive(Command)]` für typisierte Argumente
- `suprnova`-CLI - `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`,
  `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- `--version`-Flag
- Scaffold-Templates für Backend- + API-Starter über drei
  Frontends hinweg

#### Feature Flags

- `DatabaseEvaluator` mit Snapshot-Laden
- `CachedEvaluator` mit TTL
- `FeatureMiddleware`-Extractor
- Admin-CRUD-Oberfläche
- `FeatureSync`-Trait für Sub-Sekunden-Propagation über Prozesse
  hinweg

#### Zeitplan

- Cron-Ausdrucks-Parser
- `Schedule::task(...)` mit komponierbaren Prädikaten
- Single-Server-Sperren, Overlap-Prävention, Dispatch-Tracking
- Console-Befehl `schedule:run`

#### Validierung

- Integration von `validator` 0.20
- Makros `#[request]` + `#[derive(FormRequest)]`
- `#[form_request(max_body_bytes = N)]` Pro-Formular-Größenobergrenze
- `#[form_request(custom_hooks)]` Opt-out für nutzergeschriebenes
  `impl FormRequest`
- Lifecycle-Hooks - `authorize`, `after_validation`,
  `after_validation_async`

#### Datenbank-Treiber

- SeaORM-gestützte Unterstützung für SQLite, Postgres, MySQL,
  MariaDB
- URL-basierte Treiber-Erkennung
- Migrationssystem + `migrate`, `migrate:rollback`,
  `migrate:status`, `migrate:fresh`, `migrate:refresh`

#### HTTP-Client

- `Http`-Facade - `get` / `post` / `put` / `patch` / `delete`,
  liefert einen `RequestBuilder`; `.send().await` erzeugt eine
  `ClientResponse`
- rustls-TLS, 30s Standard-Timeout, User-Agent
  `suprnova/<version>`
- Verkettbare Methoden `json` / `form` / `body` / `header` /
  `bearer_token` / `basic_auth` / `timeout`
- `RequestBuilder::retry(max_attempts, base_backoff)` - exponentieller
  Backoff für transiente Fehlschläge und 5xx; respektiert
  `Retry-After`
- Test-Guard `Http::fake(|| async { ... }).await` mit
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Verschlüsselung

- Statische `Crypt`-Facade + `EncryptionKey` (`crypto::*`);
  AES-256-GCM mit 12-Byte-Zufalls-Nonces
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- `CryptPurpose`-AAD-Bindung, verhindert Cross-Protocol-Replay
- `APP_KEY_PREVIOUS`-Rotation
- CLI-Befehl `suprnova key:generate` zum Prägen frischer Keys

#### Testen

- Asynchrones Test-Makro `#[suprnova_test]`
- `TestDatabase::fresh::<Migrator>()` mit parallel-sicheren
  Instanzen
- `TestContainer::bind` für Pro-Test-Mocks
- HTTP-Test-Helfer - `Test::get`, `Test::post`, JSON / Form /
  Multipart
- Fakes für Queue / Mail / Notification / Event
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Geändert

- Auth-Verifizierung und Passwort-Reset-Flows laufen jetzt über den
  konfigurierten User-Provider statt über Torii-Interna.
- Generierte Apps müssen jetzt `get_auth_password` implementieren;
  gescaffoldete Beispiele scheitern jetzt sichtbar, statt den Login
  immer still fehlschlagen zu lassen.
- Das lokale Release-Gate ist jetzt in `scripts/release.sh`
  verdrahtet, und das Repo enthält einen erzwungenen Pre-push-Hook
  für fmt, clippy, Tests, Docs und Feature-Builds.
- Die Dokumentation der gescaffoldeten Dev-Ports wurde auf die
  aktuellen Backend-/Frontend-Defaults (`8765` / `5765`)
  aktualisiert, mit dokumentiertem `dev:tls` und `--with-portless`.
- `MAIL_FROM` wird jetzt validiert, bevor Verifizierungs- oder
  Reset-Tokens ausgestellt werden, was verwaiste Auth-Flow-Zeilen bei
  ungültiger Mail-Konfiguration vermeidet.

### Behoben

- Drift des React-Scaffold-Templates vom veröffentlichten Starter.
- Root-Routen-Gruppen erzeugen keine doppelten `//`-Pfade mehr.
- Literal-Pfad-Redirects dispatchen jetzt über den beabsichtigten
  Routing-Pfad.
- Broadcasting-Fanout-Tests behandeln jetzt `track`-/`untrack`-Ergebnisse.
- Der `log`-Mail-Treiber gibt jetzt den gerenderten Text-Body aus,
  sodass Verifizierungs- und Passwort-Reset-Links in lokalen
  Entwicklungs-Logs auftauchen.
- Die Passwort-Reset-Abdeckung nagelt das Revocation-Verhalten für
  Session und Remember-me fest.

### Hinweise

- **Distributionsmodell**: durchgängig Git-basiert.
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`;
  CLI über `cargo install --git`. Nichts wird auf crates.io
  veröffentlicht.
