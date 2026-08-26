# Änderungsprotokoll

Ein lesbares Protokoll pro Version dessen, was sich in Suprnova
geändert hat. Jeder Versionsabschnitt ist der Freigabe-Datensatz dieser
Version. Eine Version wird freigegeben, wenn ihr Versions-Commit und
der passende `v<version>`-Tag atomar gepusht werden. Neueste zuerst.

## 1.3.4 - 2026-08-25

### Hinzugefügt

- **Read-Through-Disks nehmen ein `copy`-Flag entgegen und lösen `copy` /
  `rename` über die Fallback-Grenze hinweg auf.** Setzen Sie `copy: false`
  auf `ReadThroughConfig`, um Fallback-Treffer auszuliefern, ohne sie
  durchzuschreiben; das macht die Disk zu einem transparenten Overlay und
  verengt jeden Abruf auf den angefragten Bereich. `copy` und `rename`
  streamen jetzt eine Quelle, die nur auf dem Fallback liegt, zum
  primären Ziel hinüber; ein `rename` löscht außerdem die Quelle auf dem
  Fallback, sodass ein späteres Lesen das verschobene Objekt nicht wieder
  auferstehen lassen kann. Bedingungen reisen auf diesem Streaming-Pfad
  mit: `if_not_exists` verweigert weiterhin ein bestehendes Ziel, die
  Quellversion einer Kopie wählt aus, welches Objekt der Fallback
  herausgibt, und das `if_match` einer Kopie wird mit `Unsupported`
  abgelehnt, statt stillschweigend fallen gelassen zu werden. Ein
  Transfer, der unterwegs scheitert, entfernt nur ein Ziel, das er selbst
  angelegt hat, er kann also kein Objekt zerstören, das bereits da war.
- **Entprellte Jobs und entprellte Queued Listener.**
  `Job::debounce_for()` fasst einen Schwall von Dispatches zu einem Lauf
  zusammen, ein Fenster nach dem jüngsten, und trägt die neueste Payload.
  Es ist das Spiegelbild von `push_unique`, das den ersten Dispatch behält
  und den Rest unterdrückt. `Job::max_debounce_wait()` verhindert, dass
  ein durchgehender Schwall die Arbeit ewig aufschiebt, und
  `Job::debounce_id(&self)` grenzt das Fenster pro Entität ein, sodass
  zwanzig Aktualisierungen an einer Bestellung zusammenfallen, ohne die
  einer anderen Bestellung zu berühren.
  `Queue::push_debounced(job, DebounceOptions)` setzt das Fenster an der
  Aufrufstelle, und `DebouncedListener::new(window, build).keyed_by(...)`
  entprellt einen Event-Listener mit einem aus dem Event abgeleiteten
  Schlüssel - ein schlichter `QueuedListener` beachtet ein Fenster, das
  der Job selbst deklariert, ohnehin. Jeder Dispatch wird weiterhin
  eingereiht; das Zusammenfassen wird im Worker entschieden, der ein
  überholtes Envelope bestätigt und `JobDebounced` ausgibt. Das Entprellen
  ist Fail-open: Ein abgelaufenes oder verdrängtes Fenster lässt den Job
  laufen, statt ihn zu verwerfen. Jeder tatsächliche Lauf
  startet ein frisches Fenster für die maximale Wartezeit, ein Schwall
  misst seine maximale Wartezeit also immer ab seinem eigenen ersten
  Dispatch, statt die des vorherigen Schwalls zu erben. Ein Job kann nicht
  zugleich `debounce_for` und `unique_id` deklarieren, und Chains und
  Batches lehnen einen entprellten Job ab - ein überholtes Glied ließe den
  Rest seiner Chain stranden, und ein überholter Batch-Job hielte den
  Zähler der ausstehenden Jobs für immer über null. Das Envelope trägt
  dafür zwei zusätzliche Felder und lässt das Wire-Format für jeden nicht
  entprellten Push byteidentisch.

- **`Storage::register_read_through` komponiert zwei Disks zu einer
  Read-Through-Disk.** Lesezugriffe und Metadaten lösen zuerst gegen die
  primäre Disk auf und fallen auf die zweite zurück; alles, was auf dem
  Fallback gefunden wird, wird auf die primäre Disk durchgeschrieben,
  sodass eine Speichermigration unter echtem Verkehr fertig wird.
  Schreibzugriffe und Auflistungen bleiben auf der primären Disk, und ein
  Löschen entfernt das Objekt von beiden Disks. Setzen Sie
  `throw_on_promotion_failure`, wenn ein fehlgeschlagenes Hochziehen
  sichtbar werden muss, statt zu einem Fallback-Lesen abzusinken. Ein
  Hochziehen wird atomar veröffentlicht, kein Leser kann also ein halb
  geschriebenes Objekt sehen, und es trägt Content-Type, Cache-Control,
  Content-Disposition, Content-Encoding und die Nutzer-Metadaten des
  Fallback-Objekts mit hinüber. Ein versioniertes oder bedingtes Lesen
  wird mit unveränderter Bedingung durchgereicht und ausgeliefert, ohne
  hochgezogen zu werden.
- **`Queue::forward` leitet eine ganze Queue namentlich um.** Wo
  `Queue::route` nach Job-Typ geschlüsselt ist, ist
  `Queue::forward("default", "high")` nach Queue-Namen geschlüsselt - der
  Hebel, um einen Pool stillzulegen, einen Rückstau aufzunehmen oder
  Arbeit von einem Pool wegzuholen, den Sie gleich abschalten, ohne einen
  einzigen Job oder eine einzige Route anzufassen. Es greift auf beiden
  Seiten: Neue Pushes, die auf `default` aufgelöst haben, landen auf
  `high`, *und* ein mit `--queue=default` gestarteter Worker leert `high`,
  sodass das Ziel keine Arbeit sammeln kann, die niemand beansprucht.
  `default` weiterzuleiten fängt Jobs ein, die keine Queue benannt haben.
  Eine Weiterleitung ist ein einzelner Nachschlag, nie eine Kette; ein
  Tausch (`a -> b` bei zusätzlich registriertem `b -> a`) oder eine
  längere Rotation ist deshalb ein stimmiger Pool-Tausch und keine
  Schleife - genau wie bei Laravel, dessen Resolver derselbe einzelne
  Nachschlag ist. Das Pausieren wird weiterhin auf den Namen ausgewertet,
  mit denen ein Worker gestartet wurde,
  `Queue::pause(&connection, "default")` stoppt diesen Worker also auch
  dann, wenn `default` weitergeleitet wird.
  `Queue::forward_on(from, to, connection)` beschränkt eine Weiterleitung
  auf einen Connection-Namen, verglichen mit dem Connection-Namen dieses
  Prozesses statt mit der vom Job deklarierten Connection, sodass beide
  Hälften der Umleitung am selben Wert hängen. `Queue::forward_for(from)`
  liest eine Weiterleitung zurück, und `Queue::try_forward` ist das
  fehlbare Geschwister. Die Aufrufe zur Inspektion
  (`Queue::pending_jobs` und seine Geschwister) folgen einer Weiterleitung
  absichtlich nicht, sodass ein Rückstau, der auf einer weitergeleiteten
  Queue zurückgeblieben ist, sichtbar bleibt.

- **Lesende Redis-Befehle wiederholen einen transienten Fehlschlag, statt
  ihn nach außen zu geben.** Der Connection-Manager hat sich im
  Hintergrund ohnehin schon neu verbunden, aber der Befehl, der den toten
  Socket erwischt hat, ließ Ihren Aufruf trotzdem scheitern. `GET`,
  `EXISTS`, die `SCAN`- und `SSCAN`-Seiten hinter `Cache::flush` /
  `Cache::flush_tags`, die Lesezugriffe `XLEN` / `ZCARD` / `XPENDING` des
  Queue-Treibers und die `Retry-After`-Berechnung der Ratenbegrenzung
  wiederholen nun einmal nach einer kurzen Pause.
  `REDIS_COMMAND_RETRIES` fügt darüber hinaus weitere Wiederholungen
  hinzu, gedeckelt bei 10. Rechnen Sie die Wiederholung in Sekunden statt
  in Millisekunden: Der zweite Versuch wartet auf die Ersatzverbindung und
  kostet damit das gesamte Verbindungs- und Antwortbudget des Treibers,
  und ein in ein Timeout gelaufener Befehl zählt ebenso als transient wie
  ein abgerissener. Schreibzugriffe wiederholen bei keiner Einstellung:
  Ein transienter Fehler bedeutet, dass die Verbindung ausgefallen ist,
  nicht, dass der Server den Befehl abgelehnt hätte, ein `SET`, ein
  `INCR`, ein Sperrerwerb, ein Treffer der Ratenbegrenzung oder ein
  Queue-Pop könnte also zweimal laufen. Fehlermeldungen sind unverändert,
  alles, was darauf abgleicht, funktioniert also weiter.
- **Ein pausierter Worker sagt Ihnen jetzt, dass er pausiert ist.**
  `queue:work` gibt eine Zeile pro Übergang aus -
  `2026-08-25 14:03:11 Queue billing PAUSED`, und `RESUMED` auf dem
  Rückweg - und der Worker gibt `WorkerQueuePaused` /
  `WorkerQueueResumed` aus, sodass Sie dasselbe Signal in Ihr eigenes
  Alerting leiten können. Das ist das Paar auf der Worker-Seite; die
  bestehenden `QueuePaused` / `QueueResumed` feuern in dem Prozess, der
  `queue:pause` ausgeführt hat, und das ist nie der Worker, ein Worker,
  der still wurde, weil jemand seine Queue pausiert hat, war bislang
  also nicht von einem hängenden zu unterscheiden. Jedes Event feuert
  einmal pro Übergang, nicht einmal pro Poll. Ihr Feld `queue` ist
  optional: Ein ohne `--queue` gestarteter Worker leert alles und hat
  unter `pause_all` keine Queue-Namen zu melden, er meldet daher `None`,
  statt einen Namen zu erfinden, auf den ein Listener abgleichen könnte.
- **`?include=`-Pfade sind auf fünf Segmente gedeckelt, und
  `max_relationship_depth` verschiebt die Obergrenze.** Ein zyklischer
  Relationsgraph macht aus `?include=author.posts.author.posts...` ein
  Fan-out, das ein Client steuert, begrenzt nur durch den Query-String.
  Pfade werden nun beim Parsen abgeschnitten; rufen Sie
  `suprnova::max_relationship_depth(n)` in `bootstrap::register()` auf, um
  die Grenze zu ändern, oder übergeben Sie `0`, um Includes abzuschalten.
- **`Gt`, `Gte`, `Lt` und `Lte` vergleichen ein Feld mit einer Zahl oder
  mit einem anderen Feld.** `CompareWith` benennt Operand und Maß in einem
  Wert: `Number` für ein Literal, `NumericField` für ein numerisches
  Geschwisterfeld und `LengthField` für ein nach Zeichenzahl verglichenes
  Geschwisterfeld. Ein Operand, den die Regel nicht messen kann, lässt das
  Feld scheitern, statt zu panicken.
- **Drei Zugehörigkeitsregeln kommen zum eingebauten Satz hinzu:
  `InArray`, `Contains` und `DoesntContain`.** `InArray` prüft einen Wert
  gegen die Liste eines anderen Feldes, und Sie übergeben die Liste
  direkt, statt das Feld in einem Regel-String zu benennen. `Contains` und
  `DoesntContain` laufen über ein JSON-Array und treffen einen Parameter
  nur gegen ein String-Element, `1` und `"1"` bleiben also verschieden.
- **Der Datenbank-Pool hat jetzt Stellschrauben für die Lebendigkeit.**
  `DB_IDLE_TIMEOUT`, `DB_MAX_LIFETIME`, `DB_ACQUIRE_TIMEOUT`,
  `DB_TEST_BEFORE_ACQUIRE` und `DB_PING_AFTER_IDLE` steuern, wann der Pool
  eine Verbindung schließt, recycelt und anpingt, mit passenden Settern
  auf `DatabaseConfig::builder()`. Jede ist standardmäßig ungesetzt, der
  Pool eines bestehenden Deployments verhält sich also exakt wie zuvor.
  Nutzen Sie sie, wenn ein NAT-Gateway oder eine Firewall untätige
  Verbindungen verwirft: sqlx bietet keine Entsprechung zu libpqs
  `keepalives_*`, das Recyceln im Pool ist also der Mechanismus.
- **`db:seed <Class>` meldet seinen Fortschritt.** Ein gezielter Lauf gibt
  vor dem Seeder eine `RUNNING`-Zeile und danach eine `DONE`-Zeile mit den
  verstrichenen Millisekunden aus. Ein bloßes `db:seed` bleibt still. Der
  Formatierer `suprnova::two_column_detail` steht auch Ihren eigenen
  `#[command]`-Handlern zur Verfügung.
- **Many-to-many-Relationen filtern jetzt nach Pivot-Spalten.**
  `where_pivot`, `where_pivot_op`, `where_pivot_in`, `where_pivot_not_in`,
  `where_pivot_null`, `where_pivot_not_null`, `where_pivot_between`,
  `where_pivot_not_between`, `where_pivot_group` und ihre
  `or_`-Zwillinge schränken `get`, `first` und `count` auf
  `BelongsToMany`, `MorphToMany` und `MorphedByMany` ein.
  `where_pivot_group` nimmt eine Closure entgegen und rendert eine
  geklammerte Gruppe, bleibt also innerhalb eines folgenden
  `or_where_pivot` atomar. Pivot-Filter gelten nur für Lesezugriffe:
  `attach`, `attach_with`, `detach` und `sync` geben einen Fehler zurück,
  solange einer gesetzt ist, und Eager Loading trägt sie nicht mit.
- **`where_binary` vergleicht Spaltenwerte Byte für Byte.** Die Familie
  (`where_binary`, `or_where_binary`, `where_not_binary`,
  `or_where_not_binary`) wird auf `Builder<M>` ausgeliefert, und
  `where_binary` und `where_not_binary` werden auf `DB::table(...)`
  ausgeliefert. MySQL und MariaDB geben `= binary` aus; Postgres und
  SQLite geben beim Rendern der Query einen Fehler zurück, statt auf
  einen kollationsabhängigen Treffer zurückzufallen.
- **`Builder::try_to_sql_with_bindings_for` rendert SQL für einen Dialekt,
  ohne zu panicken.** Es ist das fehlbare Geschwister von
  `to_sql_with_bindings_for`, für die Fälle, in denen ein Builder
  berechtigterweise nicht für ein Backend rendern kann.
- **`Model::refresh_for_update` lädt eine Zeile unter einer
  `FOR UPDATE`-Sperre neu.** Rufen Sie es innerhalb einer Transaktion auf,
  wenn Sie den aktuellen Zustand der Zeile und die exklusive Sperre in
  einem Statement brauchen. SQLite hat keine Sperren auf Zeilenebene, die
  Sperrklausel ist dort also wirkungslos.
- **`Builder::or_where_key` und `Builder::or_where_key_not` fügen
  Primärschlüssel-Filter als Disjunktion hinzu.** Beide falten sich
  genauso in die vorangehende `WHERE`-Klausel wie `or_where`, und beide
  bringen die Aliase `or_filter_key` und `or_filter_key_not` mit.
- **`Builder::in_order_of` sortiert Zeilen in eine ausdrückliche
  Reihenfolge.** Übergeben Sie eine Spalte und die Werte in der
  gewünschten Reihenfolge; Zeilen, deren Wert nicht in der Liste steht,
  sortieren zuletzt. Die Werte werden als Parameter gebunden, sie dürfen
  also aus Anfragedaten stammen.

### Behoben

- **Das Bypass-Cookie des Wartungsmodus läuft jetzt serverseitig ab.** Die
  TTL von 12 Stunden war ein `max-age`, das der Browser durchsetzte, ein
  abgefangenes Cookie funktionierte also weiter, bis Sie das Secret
  rotiert haben. Die verschlüsselte Payload trägt die Frist nun mit, und
  jede Anfrage prüft sie erneut.
- **`suprnova serve` startet ein Projekt ohne Frontend.** Ein mit
  `suprnova new --api` gescaffoldetes Projekt hat kein
  `frontend/`-Verzeichnis, und `serve` lehnte es mit „No frontend
  directory found. Are you in a Suprnova project directory?“ ab, sofern
  Sie nicht `--backend-only` übergaben. Es überspringt nun den
  Vite-Bereich und die TypeScript-Generierung, die ihn speist, und startet
  das Backend. `--frontend-only` scheitert bei einem solchen Projekt
  weiterhin, mit einer Meldung, die sagt, warum.

### Upgrade

- **Bypass-Cookies, die vor dieser Veröffentlichung ausgestellt wurden,
  funktionieren nicht mehr.** Die Payload des Cookies hat sich vom bloßen
  Secret zu einem versiegelten Objekt `{ secret, expires_at }` geändert,
  und eine Payload ohne Frist wird abgelehnt. Rufen Sie die Secret-URL
  nach dem Upgrade einmal auf, um ein neues Cookie zu bekommen. Sonst
  ändert sich nichts: `down`, `up`, `--secret` und `--with-secret`
  verhalten sich alle wie zuvor.
- **Ein Include-Pfad, der länger als fünf Segmente ist, gibt jetzt seine
  ersten fünf Relationen zurück statt aller.** Nichts außerhalb der
  Allowlist einer Resource war je erreichbar, keine Antwort gewinnt also
  Daten hinzu; ein tiefer Pfad verliert sein Ende. Ein Statuscode ändert
  sich damit: Ein Pfad, dessen zu tiefes Ende eine Relation benennt, die
  die Resource nicht erlaubt, wird abgeschnitten, bevor irgendetwas
  validiert, er gibt also jetzt `200` mit den überlebenden Segmenten
  zurück, wo der vollständige Pfad früher `400` zurückgab - passen Sie
  jeden Client und jeden Test an, der auf diese Ablehnung assertiert.
  Heben Sie die Obergrenze mit `suprnova::max_relationship_depth(n)` an,
  wenn Ihre API längere Pfade dokumentiert.
- **`DatabaseConfig` hat fünf öffentliche Felder bekommen.** Code, der
  eine solche Struktur über ein Strukturliteral baut, kompiliert nicht
  mehr. Nutzen Sie `DatabaseConfig::from_env()` oder
  `DatabaseConfig::builder()`; beide füllen die neuen Felder mit den
  Standardwerten, die das heutige Pool-Verhalten bewahren.

## 1.3.3 - 2026-08-25

### Hinzugefügt

- **Failover-Queue-Connection.** `FailoverQueueDriver` umschließt eine
  geordnete Liste von Connections: Ein Push, den die erste ablehnt, wird auf
  der nächsten wiederholt, und so weiter die Liste hinunter. Verdrahten Sie
  ihn aus der Umgebung mit `QUEUE_DRIVER=failover` plus
  `QUEUE_FAILOVER_CONNECTIONS=redis,database` (jeder Eintrag liest die
  Variablen seines eigenen Treibers, ein `database`-Eintrag braucht also
  weiterhin zuerst `DB::init()` und bringt weiterhin seinen Speicher für
  fehlgeschlagene Jobs mit), oder bauen Sie ihn direkt mit
  `FailoverQueueDriver::new(vec![(label, driver), ...])`. Nur Schreibzugriffe
  fallen durch: `push` und `bulk_push` laufen die Liste ab, während `pop`,
  `pop_from`, `ack`, `nack`, `release`, `settle`, `clear`, alle vier Zähler
  und alle drei Auflistungen zur Inspektion an die erste Connection und an
  keine andere delegieren, denn ein Reservierungstoken hat nur für den
  Treiber Bedeutung, der es ausgestellt hat. Die betriebliche Folge ist
  dokumentiert statt übertüncht: Ein Worker auf der Failover-Connection leert
  nur die primäre, alles, was auf einen Fallback ausgewichen ist, braucht
  also einen eigenen Worker. `bulk_push` schiebt jedes Envelope einzeln,
  statt einen Batch weiterzureichen, was sowohl das eigene `available_at`
  jedes Envelopes bewahrt (Laravel #60950) als auch verhindert, dass ein
  Batch, den die primäre Connection halb angenommen hat, geschlossen erneut
  auf den Fallback geschoben wird. Eine Ablehnung löst
  `queue::events::QueueFailedOver { connection, job_name, exception }` aus,
  flankengesteuert: Eine Connection meldet sich einmal, wenn sie in den
  Fehlerzustand geht, und bleibt still, bis ein späterer Push auf ihr gelingt
  und sie neu scharf stellt; ein Ausfall erzeugt also einen einzigen Alarm
  statt einen pro Dispatch. Wenn jede Connection ablehnt, gibt der Push den
  Fehler der letzten Connection zurück. Eine leere Connection-Liste, ein
  fehlendes oder leeres `QUEUE_FAILOVER_CONNECTIONS`, ein verschachtelter
  `failover`-Eintrag und ein Eintrag, der einen nicht existierenden Treiber
  benennt, sind allesamt Boot-Fehler - das Verhalten „warnen und auf Memory
  zurückfallen“ bleibt bei `QUEUE_DRIVER` selbst, wo ein Tippfehler kein
  flüchtiges Backend in eine dauerhafte Kette einfügen kann.
- **API zur Queue-Inspektion.** `Queue::pending_jobs(queue)` / `delayed_jobs`
  / `reserved_jobs` listen die tatsächlichen Envelopes hinter den vorhandenen
  Zählern `pending_size`/`delayed_size`/`reserved_size` auf, als
  `InspectedJob`-DTOs (`id`, `queue`, `name`, `attempts`, `payload`,
  `created_at`) - spiegelt Laravels `InspectedJob`. Ein einzelner
  `Option<&str>`-Filter für die Queue fasst Laravels Paar
  `pendingJobs($queue)` / `allPendingJobs()` (und die Entsprechungen
  `delayedJobs`/`reservedJobs`) zu jeweils einem Aufruf zusammen. Der
  Standard des `QueueDriver`-Traits ist ein ehrliches `Err` - nicht Laravels
  Beanstalkd/SQS-Standard mit leerer Collection, der sich wie „nichts in der
  Queue“ liest, auch wenn dort ganz offensichtlich etwas liegt -, sodass ein
  Treiber, der die Inspektion nicht implementiert hat, das auch sagt;
  `sync`/`null` überschreiben mit `Ok(vec![])`, weil das für sie wirklich die
  Wahrheit ist. Der Memory-, der Datenbank- und der Redis-Treiber
  implementieren alle die vollständige Auflistung: Der Speicher des
  Memory-Treibers für verzögerte Jobs wechselte von einer bloßen
  `DelayQueue<Envelope>` (die sich nicht iterieren lässt) zu einer
  `DelayQueue<Uuid>` plus einer nach ID geschlüsselten Map; der
  Datenbank-Treiber verwendet die exakten Prädikate der Zähler erneut, dazu
  `ORDER BY available_at`, und eine Zeile, deren `envelope_json` sich nicht
  dekodieren lässt, wird weiterhin aufgelistet (`id: None`,
  `payload: {"unparseable": true}`) statt fallen gelassen, sodass eine
  vergiftende Zeile einem Betreiber nicht den Blick auf den Rest der Queue
  verstellt; Redis' `reserved_jobs` ist auf die prozessinternen
  Reservierungen dieses Konsumenten begrenzt (dokumentiert), und
  `pending_jobs` scannt den Stream in Stapeln über `XRANGE`. `Queue::fake()`
  hat passende Helfer `pending_jobs()`/`delayed_jobs()` bekommen, die
  aufgezeichnete Pushes projizieren, wobei `attempts` immer `0` und
  `created_at` immer `None` ist.
- **Dispatch nach dem Commit.** `Job::after_commit()` hält einen Push zurück,
  bis die umgebende `DB::transaction` committet, sodass ein Worker in einem
  anderen Prozess nie ein Envelope poppen kann, das Zeilen beschreibt, die
  die Transaktion noch nicht dauerhaft gemacht hat. Der ganze Push wartet,
  nicht nur der Schreibvorgang des Treibers: Der Aufbau des Envelopes,
  `JobQueueing` und `JobQueued` passieren alle zum Commit-Zeitpunkt, sodass
  nie ein Listener von einem Job erfährt, den ein Rollback anschließend
  verwirft. Ein Rollback verwirft den Push vollständig; außerhalb einer
  Transaktion passiert der Push sofort, und genau das erlaubt es einem
  Job-Typ, das Opt-in zu erklären, ohne dass jede Dispatch-Stelle wissen
  müsste, ob ihr Codepfad transaktional ist. Pro Dispatch sticht
  `EnvelopeOverrides::after_commit` den Job aus: `Some(true)` (mit der
  Kurzform `Queue::push_after_commit(job)`) verzögert einen Job, der sich
  nicht angemeldet hat, und `Some(false)` ist Laravels `beforeCommit()`. Ein
  verzögerter `Queue::push` löst `Job::delay()` erneut gegen den Commit statt
  gegen den Push auf, während `Queue::push_later` / `later` / `later_with`
  den absoluten Zeitstempel des Aufrufers unverändert durchreichen.
  `Queue::push_unique` nimmt seine Dedupe-Sperre sofort, auch wenn das
  Envelope verzögert wird, sodass ein Duplikat innerhalb derselben
  Transaktion trotzdem unterdrückt wird, und ein Rollback gibt diese Sperre
  eigentümerbezogen frei. `Queue::bulk` verzögert als Einheit.
  `Queue::fake()` zeichnet einen Push sofort auf, Verzögerung inklusive,
  passend zu Laravels `Bus::fake`. Ein manuelles `DB::begin_transaction`
  verzögert nie - es installiert keine umgebende Transaktion, es gibt also
  keinen Commit, an den sich ein Callback hängen ließe. Jedes Ende, das den
  Commit nicht landen lässt, kompensiert auf dieselbe Weise, einschließlich
  eines `COMMIT`, den die Datenbank verweigert, und eines geleakten
  `TxHandle`, der einen blockiert; und `Transaction::rollback_to` zählt für
  den Bereich, den es abwickelt, ebenfalls dazu: Ein innerhalb eines
  Savepoints verzögerter Push wird verworfen, wenn dieser Savepoint
  zurückgerollt wird, und seine Sperre wird genau dann freigegeben, während
  alles vor dem Savepoint Registrierte unangetastet bleibt. Gequeuete Mails,
  Benachrichtigungen, Batches und Chains verzögern noch nicht.
- **Jobs mit Eindeutigkeit bis zur Verarbeitung.**
  `Job::unique_until_processing()` gibt die Eindeutigkeitssperre frei, wenn
  die Verarbeitung beginnt - nach dem Middleware-Durchlauf des Jobs,
  unmittelbar bevor der Handler läuft -, statt sie über das volle
  `unique_for`-Fenster zu halten; genau das wollen Sie, wenn die Sperre dazu
  da ist, in der Queue liegende Duplikate zusammenzufassen, und nicht dazu,
  die Ausführung zu serialisieren. Ein Job, den eine Middleware zurück auf
  die Queue legt, behält seine Sperre, denn er hat die Verarbeitung nicht
  begonnen; ein Job, den eine Middleware löscht oder ins Dead-Letter
  schickt, gibt seine Sperre ab. Die Freigabe ist eigentümerbezogen:
  `Queue::push_unique` vermerkt das Eigentümer-Token der Cache-Sperre auf dem
  Envelope (`Envelope::unique_lock_owner`, ein additives Feld, das das
  eingefrorene Wire-Format für jeden nicht eindeutigen Push byteidentisch
  lässt), und der Worker gibt mit diesem Token frei, sodass ein erneut
  zugestellter Versuch niemals eine Sperre gewaltsam freigeben kann, die
  inzwischen ein neuerer Dispatch hält. Die tragende
  Idempotenz-Oberfläche ist ebenfalls öffentlich:
  `Idempotency::commit_on_success_owned` reicht dem Rumpf den
  Sperreigentümer und gibt ihn zurück, und
  `Idempotency::release_owned(key, owner)` gibt eigentümerbezogen frei und
  meldet `Ok(false)` statt eines Fehlers, wenn die Sperre fehlt oder jemand
  anderes sie hält. Schlichte `unique_id`-Jobs sind unverändert und lassen
  weiterhin das `unique_for`-TTL das Dedupe-Fenster sein.
- **`Gate::default_denial_response` passt die Standardform einer bloßen
  Ablehnung an.** Spiegelt Laravels `Gate::defaultDenialResponse($response)`.
  Einmal gesetzt - typischerweise in `bootstrap::register()` - formt es genau
  zwei Ausgänge um: ein bloßes `false` (ein Bool-Gate - `Gate::define` /
  `Gate::define_async`, einschließlich einer `#[policy]`-Methode, die `bool`
  zurückgibt - oder ein `before`/`after`-Hook, der sich für `false`
  entschieden hat) und eine Auswertung, über die überhaupt nichts anderes
  entschieden hat (eine undefinierte Fähigkeit, zu der auch kein Hook eine
  Meinung hatte). All das fiel früher auf ein bloßes `Response::deny()` (ein 403)
  zusammen; jetzt taucht es als die `Response` auf, die der Standard
  trägt, zum Beispiel `Response::deny_as_not_found()` für ein 404, das die
  Existenz einer Ressource anwendungsweit verbirgt statt Gate für Gate. Der
  Standard gilt nur für ein bloßes `false` - ein mit `define_with` /
  `define_async_with` registriertes Gate hat bereits die `Response`
  zurückgegeben, die es wollte, und die geht immer unangetastet durch
  `Gate::inspect`, passend zu Laravels eigener Regel, dass der Standard nie
  ein zurückgegebenes `Response`-Objekt ersetzt. Ein Standard in der Form
  `Response::allow()` wird abgelehnt (protokolliert, ignoriert), statt
  stillschweigend jedes Bool-Gate auf „erlaubt“ zu drehen - im
  Doc-Kommentar von `Gate::default_denial_response` steht die eine Stelle, an
  der das bewusst von Laravel abweicht, das keine solche Absicherung hat.
- **Die Validierungsregel-Familie `Password` wird ausgeliefert,
  einschließlich der Prüfung `uncompromised()` gegen Have I Been Pwned.**
  `Password::min(n)` und die Stärke-Builder (`.max()`, `.letters()`,
  `.mixed_case()`, `.numbers()`, `.symbols()`) portieren die
  Regex-Ausdrücke von Laravels `Password`-Regel wörtlich - ein einfaches
  Leerzeichen erfüllt `.symbols()`, passend zu Laravels Trennzeichenklasse
  `\p{Z}`. `.uncompromised()` (oder `.uncompromised_with_threshold(n)`)
  prüft das Passwort über die k-Anonymitäts-Range-API von Have I Been Pwned:
  Nur die ersten 5 Zeichen des SHA-1-Hashes des Passworts verlassen jemals
  den Prozess, und ein Netzwerkfehler, ein Timeout oder eine
  Nicht-2xx-Antwort lässt das Passwort durchgehen, statt Anmeldungen zu
  blockieren - genau wie Laravels `NotPwnedVerifier`. Weil diese Prüfung ein
  HTTP-Round-Trip ist, ist `Password` die eine eingebaute Regel, die sowohl
  `Rule` (nur Stärke, für synchrone `validate!`-Zeilen) als auch `AsyncRule`
  (Stärke, dann die HIBP-Prüfung, für `after_validation_async`)
  implementiert - den synchronen Pfad auf einem `Password` aufzurufen, das
  mit `uncompromised()` konfiguriert ist, ist ein lauter, an Entwickler
  gerichteter Fehler und kein stilles Überspringen.
  `Password::defaults_with(...)` setzt den prozessweiten Standard, den
  `Password::defaults()` zurückgibt. Neue Umgebungsvariable
  `HIBP_TIMEOUT_SECS` (Standard 30s). `Http::fake_response_text(...)` ist das
  neue Geschwister für rohe Rümpfe zu `fake_response(...)`, für Tests gegen
  Upstream-APIs mit `text/plain` wie die von HIBP.
- **Eine geplante Task kann jetzt die Zeitzone benennen, in der ihr
  Cron-Ausdruck gelesen wird, und `schedule:list` kann den ganzen Zeitplan in
  jeder Zone darstellen.** `.timezone(chrono_tz::Tz)` legt eine einzelne Task
  fest, `.try_timezone("Area/City")` ist das fehlbare Geschwister für einen
  Zonennamen, den es erst zur Laufzeit gibt, und `Schedule::timezone(tz)`
  setzt einen Standard für jede danach registrierte Task. Für eine Task, die
  keine Zone festlegt, ändert sich nichts: Sie wird weiterhin gegen die
  lokale Zone des Prozesses ausgewertet. Eine festgelegte Zone wirkt sich nur
  auf die Fälligkeit aus - der Scheduler tickt weiterhin einmal pro
  Prozessminute, und das Dedup-Gate für dieselbe Minute bleibt unangetastet.
  Beachten Sie, dass eine Zone mit Sommerzeit manche Uhrzeitminuten doppelt
  und andere gar nicht stattfinden lässt, sodass eine auf eine solche Minute
  festgelegte Task zweimal laufen oder übersprungen werden kann; das Kapitel
  zur Task-Planung trägt die vollständige Warnung. `schedule:list` hat eine
  Option `--timezone` und zwei Spalten bekommen: die Zone, in der ein
  ausgegebener Ausdruck geschrieben ist, und die nächste Minute, in der die
  Task feuert. Der Ausdruck einer festgelegten Task wird in die Zone der
  Auflistung umgeschrieben und dabei auf mehrere Zeilen aufgeteilt, wenn er
  dort über Mitternacht reicht; er bleibt genau so stehen, wie er geschrieben
  wurde, wenn eine getreue Umschreibung unmöglich ist - über einen
  Sommerzeitwechsel hinweg, wenn ein Tageswechsel einen eingeschränkten Tag
  des Monats und Tag der Woche gemeinsam verschieben müsste, oder wenn sie
  entscheiden müsste, wie lang der Februar ist. `chrono_tz::Tz` ist von der
  Crate-Wurzel re-exportiert, konsumierende Apps nehmen `chrono-tz` also
  nicht in ihre eigene `Cargo.toml` auf.
- **Ein Laravel-förmiges Bild-Subsystem, in `suprnova::media` hinter dem
  standardmäßig aktiven `media`-Feature.**
  `Image::from_bytes/from_path/from_disk/from_upload/from_stream` baut eine
  Lazy-Pipeline - `resize`, `scale`, `crop`, `cover`, `contain`, `rotate` um
  jeden beliebigen Winkel, `flip_vertically`/`flip_horizontally`, `blur`,
  `sharpen`, `grayscale`, `to_format`, `quality` -, abgeschlossen mit
  `to_bytes`, `to_response`, `save`, `store`, `dimensions`, `mime_type` oder
  `dominant_color`. Liest und schreibt PNG, JPEG, WebP, GIF und BMP; die
  AVIF-Ausgabe ist zurückgestellt, bis der hauseigene AV1-Encoder
  veröffentlicht ist, und ist dann eine neue `OutputFormat`-Variante und
  sonst keine Änderung. Wie bei Laravels Aufteilung in `gd`/`imagick` gibt es
  zwei Treiber: `IMAGE_DRIVER=oxideav` (der Standard) läuft auf der
  Pure-Rust-Codec-Familie [OxideAV](https://github.com/OxideAV) ohne native
  Bibliothek und ohne irgendetwas zu installieren, und `IMAGE_DRIVER=magick`
  ruft ein auf dem Host installiertes ImageMagick 7 auf, für breitere
  Eingabeunterstützung einschließlich HEIC. Die Dekodier-Limits
  (`IMAGE_MAX_DIMENSION`, `IMAGE_MAX_ALLOC_BYTES`) werden gegen den Header
  der Eingabe selbst geprüft, bevor irgendetwas alloziert wird -
  einschließlich des inneren Bitstreams eines erweiterten WebP, dessen
  unverbindliche Leinwandgröße sich nicht dazu nutzen lässt, ein größeres
  Frame am Gate vorbeizuschmuggeln -, und alle Pixelarbeit läuft auf einem
  blockierenden Thread. Der `magick`-Treiber legt den Eingabe-Coder
  namentlich fest, statt ImageMagick einen aus den Bytes wählen zu lassen,
  und begrenzt jeden Aufruf mit `IMAGE_MAGICK_TIMEOUT_SECS`. `ImageDriver`
  ist die Trait-Grenze für alles Weitere. Das Modul heißt `media`, weil die
  OxideAV-gestützten Audio- und Video-Oberflächen daneben leben werden.
  [Bilder](../images.md)
- **Das WebP-Gate trägt eine feste, nicht konfigurierbare Grenze.** Ein WebP
  deklariert seine echte dekodierte Größe in seinem innersten
  Bitstream-Chunk, also durchläuft das Framework den Container, um sie zu
  finden; dieser Durchlauf besucht höchstens 4096 Chunks pro Ebene und folgt
  zwei Ebenen der Verschachtelung, und eine Datei jenseits von einem der
  beiden wird abgelehnt statt vermessen. Eine Zahl aus einem unfertigen
  Durchlauf zu melden wäre ein Gate, um das genug Füll-Chunks herumgehen
  könnten. Keine `IMAGE_MAX_*`-Variable wirkt darauf, und der Fehler sagt das
  auch so. Eine Animation mit 300 Frames ist nicht betroffen; eine mit 4100
  wird abgelehnt. [Bilder](../images.md#one-bound-is-not-configurable)

- **OAuth lässt sich jetzt installieren, ohne die bestehende Passwort- und
  Session-Autorität einer Anwendung zu ersetzen.** `MagnetarOAuthOnlyConfig`
  und `init_magnetar_oauth_only` installieren die Standard-Zeremonie und die
  Provider-Engine und lassen die Slots für Passwort und Passkey leer.
  Anwendungen mit einer bestehenden `users`-Tabelle können
  `verify_oauth_identity` aufrufen, den verifizierten Provider-Subject selbst
  zuordnen und ihre normale Framework-Session aufbauen.

### Geändert

- **`DB::transaction` kann jetzt nach einem erfolgreichen Commit `Err`
  zurückgeben**, wenn ein After-Commit-Callback fehlschlägt: Die Meldung
  lautet `after-commit callback failed (the transaction itself committed): …`,
  der Rückgabewert des Closures geht verloren, seine Schreibvorgänge nicht.
  `DB::transaction_with_attempts` wiederholt diesen Fehler nie, so
  deadlock-förmig die Meldung des Callbacks auch klingen mag - ein Closure
  erneut auszuführen, dessen Schreibvorgänge bereits dauerhaft sind, würde
  sie zweimal anwenden.
- **Neuer Schlüssel im Validierungskatalog:
  `validation-password-unverifiable`.** Ein eigener `UncompromisedVerifier`,
  der `Err` zurückgibt, setzt seinen eigenen Fehlertext nicht mehr wörtlich
  in den 422-Rumpf. Dieser Text wird stattdessen auf `error` protokolliert,
  und die Antwort trägt diesen Schlüssel, der als „The { $field } could not
  be checked against known data leaks. Please try again.“ gerendert wird -
  die Prüfung hat nicht stattgefunden, was nicht dasselbe ist, wie dass das
  Passwort schlecht wäre, und Infrastrukturdetails gehören nicht in eine
  Antwort an den Client. Eine App, die ihren eigenen Validierungskatalog
  ausliefert, muss den Schlüssel ergänzen, sonst sehen ihre Nutzer den
  eingebauten englischen Fallback.
- **Der Upload-Validator `Image` heißt jetzt `ImageFile`.**
  `suprnova::Image` ist der neue Typ für die Bildmanipulations-Pipeline,
  passend zu `Illuminate\Image\Image`, und die Upload-Regel über Magic Bytes
  übernimmt den Namen, den Laravel derselben Regelklasse gibt:
  `Illuminate\Validation\Rules\ImageFile`. Die Migration ist eine Zeile pro
  Verwendungsstelle: Aus `UploadedFile<(Image, MaxSize<N>)>` wird
  `UploadedFile<(ImageFile, MaxSize<N>)>`. Pre-1.0-Churn, den das
  Distributionsmodell über Git-Tags abfängt.

### Entfernt

- **Die ungenutzte direkte Abhängigkeit `image` ist weg.** Sie war eine
  Basisabhängigkeit ohne eine einzige Verwendungsstelle im gesamten
  Workspace und zog JPEG-, PNG-, WebP- und GIF-Codecs für nichts herein; sie
  fallen zu lassen entfernt `gif`, `image-webp`, `zune-jpeg`, `color_quant`
  und `weezl` aus dem Baum. Das Crate selbst taucht weiterhin transitiv auf,
  nur mit seinem `png`-Feature, hinter der QR-Code-Darstellung von `totp-rs`.
  Das neue Bild-Subsystem baut stattdessen auf den OxideAV-Crates hinter dem
  `media`-Feature auf.

### Behoben

- **OAuth zu installieren zwingt Provider-gestützte Anwendungen nicht mehr in
  die Web-Binding-Validierung von Magnetar.** Der vollständige
  `init_magnetar`-Pfad bleibt atomar und unverändert. Der reine OAuth-Pfad
  reserviert die Engine-Slots während der Konstruktion, veröffentlicht nur
  OAuth und schlägt fehl, statt zwei Authentifizierungsautoritäten zu
  vermischen.

### Upgrade

- **`Image` ist jetzt ein anderer Typ; der Upload-Validator heißt
  `ImageFile`.** Quellcode-brechend für alle, die die Upload-Regel über Magic
  Bytes nutzen. Benennen Sie sie an jeder Verwendungsstelle um: Aus
  `UploadedFile<(Image, MaxSize<N>)>` wird
  `UploadedFile<(ImageFile, MaxSize<N>)>`. `suprnova::Image` löst weiterhin
  auf, ist jetzt aber der Typ für die Bildmanipulations-Pipeline, sodass eine
  vergessene Umbenennung nicht mehr kompiliert, statt still das Verhalten zu
  ändern.
- **`EnvelopeOverrides` hat ein öffentliches Feld
  `after_commit: Option<bool>` bekommen.** Jede Konstruktion in diesem
  Repository und in den gescaffoldeten Templates verwendet
  `..Default::default()` und braucht daher keine Änderung. Code, der ein
  `EnvelopeOverrides` mit einem erschöpfenden Struktur-Literal baut, muss das
  neue Feld benennen; `after_commit: None` behält das heutige Verhalten bei,
  nämlich sich auf `Job::after_commit()` zu verlassen. Sonst ändert sich
  nichts: `after_commit()` ist standardmäßig `false`, es beginnt also kein
  bestehender Job, auf einen Commit zu warten, auf den er vorher nicht
  gewartet hat.
- **`Envelope` hat ein öffentliches Feld
  `unique_lock_owner: Option<String>` bekommen.** Das Wire-Format ist
  unverändert - das Feld ist `#[serde(default)]` und wird bei `None`
  ausgelassen, sodass Envelopes in beiden Richtungen byteidentisch
  durchlaufen und `schema_version` bei 2 bleibt -, aber jeder Code, der ein
  `Envelope` mit einem Struktur-Literal baut, muss es jetzt benennen. Fügen
  Sie `unique_lock_owner: None` hinzu, sofern Sie nicht absichtlich eine
  Eindeutigkeitssperre über den Push hinweg tragen. Code, der Envelopes nur
  liest oder sie über `Queue::push` und dessen Geschwister baut, braucht
  keine Änderung.

- Verwenden Sie `init_magnetar_oauth_only` statt `init_magnetar`, wenn die
  Anwendung Benutzer, Passwörter, Framework-Sessions und den
  Remember-Me-Zustand bereits selbst besitzt. Callbacks im reinen
  OAuth-Modus verwenden `verify_oauth_identity`; vollständige
  Magnetar-Anwendungen verwenden weiterhin `complete`.

## 1.3.2 - 2026-08-25

> The v1.3.2 release notes are intentionally kept in English to preserve the complete normative record.

### Hinzugefügt

- **OAuth providers can now be registered through `MagnetarConfig::oauth`.** Suprnova re-exports the `OAuthProvider` contract, all five first-party provider and configuration types, and the HTTP, revocation, abuse-limiter, authorization, and auto-link types an application needs. Custom providers no longer require a direct `suprnova-magnetar` dependency or a hand-retained `MagnetarHostEngine`.

- **A production OAuth transport and framework limiter adapter now ship at the crate root.** `ReqwestOAuthTransport` implements token, userinfo, and revocation I/O with redirects disabled by default, a 30-second timeout, a default `User-Agent`, and a 1 MiB response cap. `FrameworkAbuseLimiter` reuses the configured `RateLimiterDriver`; apps no longer hand-write either adapter.

### Behoben

- **`init_magnetar` now publishes OAuth with password and passkey services as one reserved installation.** The OAuth service is built before publication, and all three engine slots remain hidden while the reservation is active. A failed or duplicate OAuth configuration cannot leave password and passkey state visible without the configured OAuth registry.

- **Custom providers can supply userinfo headers.** `OAuthProvider::userinfo_headers` is merged with the host-owned bearer header, enabling requirements such as GitHub's `User-Agent` and media-type `Accept` headers without allowing a provider to replace `Authorization`.

### Upgrade

- **The Magnetar cutover in `4faaa933` removed Torii's OAuth installation path without wiring its replacement into the default initializer.** The old workaround required constructing a custom host engine, calling `oauth_service`, and installing the adapter separately. Replace that workaround with `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` and one `init_magnetar` call.

- **GitHub community providers must handle verified email explicitly.** GitHub `/user` usually omits non-public email, while the verified primary address requires `/user/emails`. Return `email: None` to use the email-completion ceremony, or point `userinfo_endpoint` at a host adapter that combines both responses; never treat a public but unverified address as ownership.

## 1.3.1 - 2026-08-24

> The v1.3.1 release notes are intentionally kept in English to preserve the complete normative record.

### Behoben

- **Provider-backed applications can reset verified users again.** When no Magnetar engine is installed, `PasswordReset` uses an explicitly reset-capable `UserProvider` and framework `auth_flow_tokens` for already verified accounts. `EloquentUserProvider<M>` opts in when `M` implements `MustVerifyEmail + CanResetPassword`; no `app_users` migration is required.
- **The published framework line now contains both post-release repair sets.** The translated 1.3.0 changelog layout and headings, CJK wrapping, localized anchors, glossary terms, and prose punctuation are reconciled instead of split across divergent local and remote branches.
- **Post-tag CLI and Magnetar hardening is included.** Development-process cleanup uses the completed process-group fallback, and the local qualification contracts cover the released refs and plugin-SDK SQLite lanes.

### Sicherheit

- **The provider fallback never treats password reset as first mailbox proof.** Unknown and unverified addresses receive the same no-mail response. Install Magnetar when an unverified account must prove mailbox ownership through reset so credential cleanup, auth-epoch advancement, and revocation remain atomic. Provider fallback completion reports framework session and remember revocation failures through `PasswordResetOutcome`.

### Upgrade

- **Move every `v1.3.0` Git dependency to `v1.3.1`.** Applications with their own `users` table keep their configured `UserProvider`; they do not initialize the default `app_users` engine merely to reset an already verified account. Applications that use Magnetar credentials or unverified-account first proof continue to initialize Magnetar.

## 1.3.0 - 2026-08-24

### Sicherheit

- **Magnetar begrenzt Mutationen von Anmeldedaten und Sessions jetzt auf den authentifizierten Akteur und die Auth-Epoche des Kontos.** Schreibvorgänge für Passwort, Passkey, verknüpfte Konten, Zwei-Faktor, opake Sessions, JWT, Remember, OAuth und Geräteautorisierung weisen veraltete oder widerrufene Akteure zurück. Der erste erfolgreiche Nachweis einer verifizierten E-Mail durch Passwort-Reset, Magic Link oder OAuth für ein nicht verifiziertes Konto erhöht die Epoche und entfernt provisorische Anmeldedaten, Sessions, Remember-Zustand und eine Squatter-TOTP-Registrierung atomar. Verifizierte Konten behalten während des Passwort-Resets legitime Anmeldedaten. Die E-Mail-Verifizierung erfordert den authentifizierten Token-Eigentümer, und OAuth verknüpft ein vorhandenes nicht verifiziertes Konto niemals allein anhand der E-Mail-Adresse automatisch.

- **Eine protokollrelative `_previous.url` kann über `Redirect::back()` weder beim Schreiben noch beim Lesen mehr einen off-origin Open Redirect erzeugen.** `SessionMiddleware` persistiert keine protokollrelative aktuelle URL mehr: Der Schreibvorgang nutzt denselben Sanitizer, den `InertiaValidationRedirectMiddleware` für seine `Referer`-Prüfung verwendet, und ein Request-Pfad in der Form `//host` (oder mit einem ASCII-Steuerbyte) wird nie gespeichert. Ohne dies könnte die `fallback!`-Route einer Anwendung (das Standardmuster für die Inertia-/SPA-App-Shell, bei dem jeder nicht passende Pfad mit `200` antwortet) mit `GET //evil.test/anything` diesen Pfad unverändert persistieren. `SessionData::previous_url()` wendet dieselbe Prüfung jetzt auch bei jedem **Lesen** an, sodass ein Session-Cookie, das ein Upgrade von einer Version vor diesem Fix überlebt hat und bereits einen rohen, unsanitierten Wert enthält, den kein Schreibvorgang im aktuellen Prozess je erzeugt hat, sich zu „nichts gespeichert“ selbst heilt, statt vertraut zu werden. Zusammen können weder ein altes vergiftetes Cookie noch eine neue bösartige Anfrage `Redirect::back()`, `Redirect::refresh()` oder `url::previous()` ein off-origin `Location` übergeben. Scheitert ein Wert an einer der beiden Prüfungen, wird er als nicht vorhanden behandelt, statt durch einen synthetisierten Wert ersetzt zu werden; daher wird eine tatsächlich gute vorherige URL nie überschrieben.

- **Die `Referer`-Prüfung der Inertia-Validierungs-Redirect-Brücke schließt zwei weitere Same-Origin-Bypässe.** Das `303`-Ziel von `InertiaValidationRedirectMiddleware` wies zuvor nur einen `Referer` zurück, der mit dem wörtlichen Präfix `//` oder `/\` beginnt. Ein Wert wie `Referer: /<TAB>/evil.test` schlüpfte durch, weil der WHATWG-URL-Parser ASCII-Tabulatoren und Zeilenumbrüche vor dem Vergleichen der Origins aus dem gesamten String entfernt, sodass ein Browser dies als `//evil.test` liest und dem `303` off-origin folgt. Die Prüfung weist nun jedes ASCII-Steuerbyte (C0 oder DEL) überall im Kandidaten zurück, nicht nur in den beiden genannten Präfixen. Außerdem wurde das letzte Fallback - der Pfad der fehlgeschlagenen Anfrage selbst, der verwendet wird, wenn weder `Referer` noch die vorherige URL der Session nutzbar ist - nie sanitisiert: Ein origin-form-HTTP-Request-Target darf syntaktisch mit `//` beginnen, sodass ein roher Client oder ein nicht normalisierender Proxy auch das „sichere letzte Fallback“ zu einem off-origin Redirect machen konnte. Beide Pfade teilen jetzt eine root-relative Prüfung und fallen auf `/` zurück, wenn selbst der Pfad der Anfrage sie nicht besteht.

- **Cookie-Chiffrat ist jetzt mit kontextbezogenem v2-AAD an seinen logischen Cookie-Namen gebunden.** `Cookie::encrypted` / `Cookie::read_encrypted_for` verhindern, dass ein für einen Cookie-Slot erzeugter Wert in einem anderen Slot entschlüsselt wird, während die Bindung an den logischen Namen einen späteren Wechsel des Wire-Präfixes `__Host-` / `__Secure-` sicher hält. Das versionslose Kompatibilitätsfenster versucht v2 über den gesamten Schlüsselbund und danach v1 über den gesamten Bund, sodass vorhandene Cookies den Rollout überstehen; der v1-Fallback bewahrt die alte Replay-Schwäche bis zu seiner geplanten Entfernung in 1.4.0.

- **Präfixe für Session- und Remember-me-Cookies werden beim Boot validiert und zur Renderzeit erzwungen.** `SESSION_COOKIE_PREFIX=__Host-` erfordert `Secure`, `Path=/` und keine `Domain`; `__Secure-` erfordert `Secure`. Ungültige Boot-Kombinationen schlagen vor dem Bereitstellen fehl, und der Renderer schreibt ungültige präfixierte Header um, statt Browser sie stillschweigend verwerfen zu lassen.

### Hinzugefügt

- **Die Suprnova-Authentifizierung läuft jetzt auf der internen Magnetar-Engine.** Die frameworkeigene `Auth`-Fassade erhält bestehende Aufrufstellen für Passwörter, Magic Links, Passkeys, OAuth, Bearer, Sperren, Sessions und Zwei-Faktor bei, während die Torii-Abhängigkeit entfernt wird. Die Standard-Engine installiert Passwort-/Session- und Passkey-Adapter atomar, speichert Zustellungs-Leases für den Lebenszyklus in der Anwendungsdatenbank und teilt die kanonischen `i64`-`app_users`-Identitäten der Anwendung.

- **Ein formbewusster Migrations-Runner für Authentifizierung deckt jetzt Quellen aus Torii, Suprnova Web und Suprnova API ab.** Probeläufe binden eine stabile Plan-ID an dauerhafte Zeilen- und Schema-Fingerprints sowie Zielidentitätsentscheidungen. Die Anwendung verwendet transaktionale Importe, Wiederholungs-Ledger, formbesessene Bereinigung und Kollisionsverweigerung. MySQL verwendet einen durch eine Schreibbarriere geschützten Shadow-Swap mit Pre-Copy-Journalen, Zeilen- und Schema-Parität, fortsetzbaren Umbenennungen und bereinigungserhaltender Wiederherstellung.

- **`MAIL_DRIVER=file` schreibt eine RFC-5322-Datei `.eml` pro Nachricht** nach `MAIL_FILE_PATH` (Standard `storage_path("mail")`; ein relativer Wert wird im Basisverzeichnis der Anwendung verankert, nicht im CWD des Prozesses), sodass lokale Mail in einem Mail-Client geöffnet werden kann, statt aus einer Log-Zeile gelesen zu werden. Die Datei enthält dieselbe Header-Obermenge wie SMTP, einschließlich `X-Priority`, `Importance`, `X-Tag`, `X-Metadata-*` und `Return-Path`. Wie `log` und `memory` stellt sie nicht zu: Ein Produktions-Boot weist sie zurück, sofern nicht `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` gesetzt ist.

- **`FrameworkError::External` trägt jetzt den Fehler, den es umschließt.** `FrameworkError::from_external(e)` und `FrameworkError::from_external_with("saving user", e)` halten den ursprünglichen Fehler als `std::error::Error`-Quelle erreichbar, statt ihn zu einem String einzuschmelzen. `FrameworkError::external_source()` gibt ihn für Downcasting zurück; verwenden Sie dies statt `source()`, das das gemeinsam genutzte `Arc`-Handle liefert. Beide Konstruktoren werden HTTP 500 zugeordnet.

- **5xx-Logs rendern jetzt die vollständige Fehlerquellenkette.** `render_error_chain` durchläuft `source()` und ist in die Framework-Error-Log-Zeile, die Nutzlast des Events `ErrorOccurred` und das unter `APP_DEBUG=true` ausgegebene Feld `debug_message` eingebunden. Client-seitige Response-Bodys bleiben unverändert, und 5xx-Bodys bleiben bereinigt.

- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` / `try_scroll_wrapped`.** Verschachteln Sie die Merge-Anweisung einer Scroll-Prop unter `<key>.<wrap_key>` statt unter dem nackten Schlüssel - `mergeProps: ["users.data"]` statt `["users"]` - für einen Wert, der selbst ein Envelope ist (`{ data: [...], meta: {...} }`). Laravels `ScrollProp` verschachtelt bedingungslos unter `"data"`; Suprnovas eingebaute Paginatoren geben ein nacktes Zeilen-Array zurück, daher ist dies ein Opt-in statt eines Standards, den jeder Aufrufer umgehen muss. Das neue Trait `ProvidesScrollMetadata` (`page_name` / `previous_page` / `next_page` / `current_page`, mit einem Standard-`scroll_metadata()`) spiegelt Laravels gleichnamiges Interface für einen Paginator, den dieses Crate nicht kennt; `LengthAwarePaginator`, `Paginator` und `CursorPaginator` implementieren es jetzt, statt `ScrollMetadata` von Hand zu bauen. Die `.match_on(...)`-Felder einer Scroll-Prop geben jetzt ebenfalls in `matchPropsOn` aus und entsprechen damit Laravels `resolveMergeMatchingKeys` (`Response.php:641-652`), das `matchesOn()` einer `ScrollProp` genauso einbindet wie jede andere Merge-Prop - der Match-Eintrag richtet sich danach, wo die Prop tatsächlich merged, unverpackt unter `<key>` oder unter `.scroll_wrap(...)` unter `<key>.<wrap_key>`.

- **`Prop::merge_with_path`, mehrfeldiges `match_on` und resolvergestützte Merge-Props.** `Prop::merge_with_path(path)` merged ein verschachteltes Feld im Wert einer Prop statt der ganzen Prop - `Prop::eager(v).merge().merge_with_path("data")` gibt `mergeProps: ["<key>.data"]` aus, und eine pfad-mergende Prop merged nie zusätzlich ihre Wurzel; `.deep_merge()` ignoriert sie, weil ein Deep Merge ohnehin jedes Feld rekursiv durchläuft. `Prop::match_on` nimmt jetzt ein Feld oder mehrere in einem Aufruf (`match_on(["id", "slug"])`) zusätzlich zur bereits unterstützten verketteten `Prop`-Komposition `match_on("id").match_on("slug")`. `InertiaResponse::merge_lazy` / `merge_lazy_with` ergänzen die resolvergestützten Geschwister von `.merge` / `.merge_with` und entsprechen Laravels `Inertia::merge(fn () => ...)`.

- **Partial-Reload-`only`/`except` verstehen Dot-Notation.** `X-Inertia-Partial-Data: user.name` begrenzt die Prop `user` auf `{ name: ... }`, statt den gesamten Wert oder nichts zu verlangen; `X-Inertia-Partial-Except: user.email` entfernt nur dieses Feld und lässt den Rest von `user` erhalten. `except` gewinnt, wenn beide Header einen Pfad nennen, ein nackter Eintrag meint weiterhin die gesamte Prop und ein unbekannter oder im Typ nicht passender verschachtelter Pfad wird stillschweigend verworfen, ohne seine Geschwister zu berühren. `Always`-Props sind nicht betroffen - sie werden immer vollständig geliefert.

- **Verschachtelung von Dot-Key-Props.** `.with("user.name", value)` (und jede andere Prop-anheftende Methode, eager oder aufgelöst) verschachtelt jetzt in `props.user`, statt einen wörtlichen Schlüssel `"user.name"` zu liefern, entsprechend Laravels `Arr::set`-basierter Entpackung von `resolveArrayableProperties`. Zwei Aufrufe mit demselben Präfix - `.with("user.name", …)` und dann `.with("user.age", …)` - sammeln sich in einem Objekt; ein Schlüssel ohne Punkt bleibt unverändert. Schlüssel des Shared-Registrys `App::inertia_share*` verschachteln sich auf dem Wire genauso. Die Entpackung berührt nur Prop-*Schlüssel* der obersten Ebene - sie steigt nie in den Wert einer Prop hinab, sodass ein Validierungs-`errors`-Bag seine intern getragenen gepunkteten Feldnamen behält.

- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`.** Laravels `Inertia::getShared` / `Inertia::flushShared` zum Lesen und Leeren des statischen Share-Registrys (`App::inertia_share` / `_lazy` / `_once`). `inertia_shared` unterstützt auf der Leseseite dieselbe Dot-Notation wie `inertia_share`; es gibt für einen lazy- oder once-Share `None` zurück (es gibt keinen Request, gegen den einer aufgelöst werden könnte) sowie für einen nicht registrierten Schlüssel. `flush_inertia_shared` leert nur das statische Registry; ein über `App::register_inertia_shared` registrierter Trait-Provider bleibt unberührt, entsprechend Laravel (dort gibt es keinen pro-Request-Zustand zum Leeren).

- **`InertiaResponse::always_with(key, resolver)`.** Das async-resolvergestützte Geschwister von `.always(key, value)` für eine stets eingeschlossene Prop, deren lazy Auflösung kostspielig genug ist - Laravels `Inertia::always(fn () => …)` (`AlwaysProp` akzeptiert jeden Wert, einschließlich Closures).

- **`InertiaSharedData::share` erhält jetzt den Namen der Seitenkomponente**, damit ein Provider seine Ausgabe nach Seite variieren kann - Laravels `RenderContext`. Siehe Upgrade.

- **Inertia-Prop-Komposition.** Eine `Prop` trägt jetzt orthogonale Flags, statt eine von neun abgeschlossenen Varianten zu sein, sodass eine einzelne Prop deferred *und* mergebar, mergebar *und* gecacht oder optional *und* gecacht sein kann - die Kombinationen, die das Inertia-3-Protokoll erwartet und die ein geschlossenes Enum nicht ausdrücken konnte. Bauen Sie eine mit `Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent`, verketten Sie `.always()`, `.optional()`, `.defer()`, `.group()`, `.rescue()`, `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on()`, `.once()`, `.as_key()`, `.until()`, `.fresh()`, `.scroll()`, und hängen Sie sie mit dem neuen `InertiaResponse::prop(key, prop)` an. Eine `defer().merge()`-Prop wird beim ersten Render unter `deferredProps` angekündigt und kommt beim Folge-Request unter `mergeProps` an. Neue Typen `MergeMode` und `Visibility` beschreiben die Flags; jede vorhandene Builder-Abkürzung (`.with`, `.always`, `.lazy`, `.optional`, `.defer`, `.merge*`, `.once*`) bleibt unverändert.

- **Queue anhalten / fortsetzen.** `Queue::pause(connection, queue)` / `resume` / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` / `paused_queues(connection, &queues)`, über `Cache` gesichert wie das Restart-Signal - `resume_all` löscht keine pro-Queue-Pause, entsprechend Laravel. Das Claim-Gate des Workers sitzt unmittelbar vor jedem Pop, daher wird ein laufender Job immer fertig; eine globale Pause schließt die Filterung durch `--queue=...` genauso kurz wie Laravels `pausedQueues`, und eine pro-Queue-Pause wirkt nur auf einen Worker, der mit einer expliziten `--queue=...`-Liste gestartet wurde. Neue CLI-Befehle `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]` (Alias `queue:continue`) sowie `QUEUE_PAUSABLE=false`, damit ein Operator das Feature deaktivieren kann - ein nicht pausierbarer Worker ignoriert Pause-Signale und `queue:pause` verweigert selbst die Ausführung. Neue Events: `QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed`.

- **`suprnova::testing::TestResponse`** - ein flüssiger, wie Laravels `TestResponse` geformter Wrapper um das Triple `(status, headers, body)`, das jede HTTP-Test-Harness bereits erzeugt: `assert_status`, `assert_ok`, `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`, `assert_see`, `assert_header`, `assert_cookie` und (mit `.with_session_store(...)`) `assert_session_has`. Jede Assertion gibt `&Self` zurück und panickt bei Fehlschlag, derselbe Vertrag wie `expect!`. Nichts daran, wie ein Test einen Request ausführt, muss sich ändern.

- **`suprnova new` erzeugt einen SSR-Einstieg.** Jeder Starter (Svelte, React, Vue) liefert jetzt `frontend/src/ssr.{ts,tsx}` und ein npm-Skript `build:ssr` (`vite build --ssr`), verdrahtet mit seinem eigenen Ausgabeverzeichnis (`frontend/bootstrap/ssr/`), damit das SSR-Bundle nie mit dem Client-Build in `public/assets/` kollidiert.

- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`.** Das SSR-Gateway kann jetzt vor dem Dispatch eines Renderings prüfen, ob das gebaute Bundle auf dem Datenträger existiert, entsprechend Laravels `ensure_bundle_exists`-Konfiguration - ein Worker, der nie gestartet wurde, oder ein Bundle, das nie gebaut wurde, schlägt schnell fehl, statt `ssr_timeout` für eine Verbindung zu bezahlen, die niemals erfolgreich werden konnte. Aktivieren Sie dies mit `.ssr_bundle_path(...)`; anders als Laravels `BundleDetector` wird der Pfad nie automatisch erkannt, daher bleiben bestehende SSR-Konfigurationen (und Tests), die keinen setzen, unberührt.

- **Validierungsfehler bei einem Inertia-Visit leiten jetzt zurück, statt `422` JSON zurückzugeben.** `Inertia::install` registriert eine vierte Middleware, `InertiaValidationRedirectMiddleware`, die einen Validierungs-`422` bei einem `X-Inertia`-Request in ein `303` zur Formularseite mit geflashten Fehlern wandelt - dadurch füllt sich `useForm().errors` ohne Handler-Code. Der Inertia-Client behandelt jede Response ohne `X-Inertia`-Header als Nicht-Inertia und zeigt sein Fehler-Modal, sodass der alte `422` niemals `form.errors` erreichen konnte. Nicht-Inertia-Requests behalten das `422`-Envelope, Precognition-Probeläufe bleiben unberührt und `X-Inertia-Error-Bag` begrenzt den geflashten Bag. Das Redirect-Ziel ist der same-origin `Referer`, dann die vorherige URL der Session, dann der durch denselben Sanitizer geführte Pfad der Anfrage selbst; es fällt auf `/` zurück, wenn auch dieser fehlschlägt - niemals wörtlich vertraut.

- **`InertiaConfig::with_all_errors(bool)`** - behält jede Validierungsnachricht pro Feld, statt auf die erste zu reduzieren. Entspricht Laravels `Inertia\Middleware::$withAllErrors`.

- **`suprnova::testing::AssertableInertia`** - flüssige, wie Laravels `AssertableInertia` geformte Assertions über ein Inertia-Seitenobjekt, geparst entweder aus einer `X-Inertia`-JSON-Response oder dem eingebetteten `<script data-page="app">`-Element einer HTML-Shell für Hard-Navigation: `component`, `url`, `version`, `prop`, `has`, `missing`, `where_`, `count`, `has_flash`. Bauen Sie eines aus einer `HttpResponse` mit `AssertableInertia::from_response` oder aus einer `TestResponse` mit dem neuen `TestResponse::assert_inertia()`. `reload_only`, `reload_except` und `load_deferred_props` spielen einen Partial Reload gegen eine vom Aufrufer gelieferte `with_reload(...)`-Closure erneut ab - Suprnovas HTTP-Tests durchqueren einen echten Socket, daher gibt es keinen einzelnen In-Process-Test-Client, gegen den hartcodiert werden könnte.

- **`Cookie::queue`/`queued`/`unqueue`/`expire`.** Ein task-lokales Cookie-Jar - Laravels `CookieJar` - lässt beliebigen Code ein Cookie für die nächste ausgehende Response einreihen, ohne eine `HttpResponse` festzuhalten, an die es angehängt werden kann: einen Event Listener, einen containergebundenen Service, Middleware vor dem Handler. Es nutzt denselben pro-Request-Slot, den `Auth::login_remember` bereits verwendet, um das Remember-me-Cookie über die Handler-Grenze zu tragen; `SessionMiddleware` leert es neben dem Session-Cookie in die Response. `Cookie::expire(name, path, domain)` reiht ein mit `Cookie::forget_with` gebautes Lösch-Cookie ein. Erfordert `SessionMiddleware` in der Middleware-Chain der Route - außerhalb davon sind alle vier Aufrufe ein stiller No-op, entsprechend dem Verhalten von `App::flash` außerhalb eines Flash-Scopes.

- **`HttpResponse::event_stream(stream, end)` und `HttpResponse::stream_json(stream)`.** Laravels `ResponseFactory::eventStream` / `streamJson` und die exakten Wire-Formen, die `@laravel/stream-{react,vue,svelte}` mit `useEventStream` / `useJsonStream` erwarten. `event_stream` rahmt einen `Stream<Item = sse::StreamedEvent>` standardmäßig als `event: update` pro Item ein, sofern das Item sein Event nicht selbst benennt, JSON-kodiert jede nicht-stringförmige Nutzlast und hängt einen konfigurierbaren Terminal-Frame an (`EndSignal::default()` ist `data: </stream>`; `EndSignal::None` lässt ihn weg). `stream_json` streamt jedes `Stream<Item = impl Serialize>` als ein inkrementell geflushtes JSON-Array. Beide bauen auf der bestehenden Body-Pipeline `sse`/`stream_bytes` auf und teilen daher ihr Verhalten bei Abbruch und Panic-Isolation mit dem Rest des Frameworks.

- **`suprnova serve` startet einen abgestürzten Dev-Prozess neu, statt die ganze Session zu beenden.** Exponentieller Backoff zwischen Versuchen - 200ms, bei jedem aufeinanderfolgenden Absturz verdoppelt, auf 5s begrenzt und auf den Boden zurückgesetzt, sobald ein Prozess 30s gelaufen ist. `--no-restart` optiert aus und stellt das frühere Verhalten wieder her. `--restart-tries <N>` (Standard `5`, entsprechend Laravels `--restart-tries=5`) beendet die Wiederholung eines Prozesses nach so vielen aufeinanderfolgenden Abstürzen, statt ewig zu wiederholen, druckt eine handlungsfähige Meldung und lässt die anderen Prozesse - und die Session selbst - laufen. `--timestamps` stellt jeder weitergeleiteten Zeile `HH:MM:SS` voran. Ein neues Array `[[serve.process]]` in `Suprnova.toml` lässt ein Projekt eigene Dev-Prozesse deklarieren - Laravels `DevCommands::register` - die neben Backend und Frontend laufen, jeder mit seinem eigenen Präfix `[name]` und optionaler Farbe; ein unbekannter Schlüssel oder ein leerer `name`/`command` in einem Eintrag ist jetzt ein harter Parse-Fehler, statt still ignoriert oder später ein undurchsichtiger Spawn-Fehler zu werden. `--json` gibt stattdessen ein JSON-Objekt pro Zeile (NDJSON) auf stdout aus - Events für Prozessstart, Ausgabe, Ende, restart-scheduled, restart-succeeded, gave-up, types-regenerated und shutdown, einschließlich der eigenen Regenerierungsnotizen des File-Watchers und der Shutdown-Notiz des `Ctrl+C`-Handlers, die unter `--json` nun ebenfalls von stdout fernbleiben - für Scripting und Log-Pipelines; die Kombination mit `--timestamps` ist harmlos, aber redundant, weil jedes Event bereits seinen eigenen Timestamp trägt.

- **`RequestBuilder::retry_when(predicate)`.** Ein Prädikat, das vor jeder Wiederholung abgefragt wird, die die eingebaute Richtlinie (`.retry(...)` / `.retry_non_idempotent(...)`) sonst ausführen würde, und `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }` erhält. Es ergänzt die Richtlinie, statt sie zu ersetzen: `false` verhindert eine Wiederholung, die die Richtlinie ausgeführt hätte; es kann niemals eine über `max_attempts` hinaus oder eine Wiederholung erzwingen, die die Richtlinie sonst nicht versuchen würde (einen 4xx-Status oder eine nicht idempotente Methode ohne `retry_non_idempotent`).

- **`#[model(touches = [...])]` führt jetzt tatsächlich Touches aus.** Nachdem ein Child erstellt, gespeichert, aktualisiert oder gelöscht wurde, erhält jeder in der Liste genannte `BelongsTo`-Owner ein `UPDATE <owner> SET updated_at = ? WHERE <key> = ?` auf demselben Executor wie der Schreibvorgang, der ihn ausgelöst hat - daher tritt der Touch innerhalb einer `DB::transaction` dieser Transaktion bei und wird mit ihr zurückgerollt. Ein Owner, dessen Model `timestamps = false` hat, wird übersprungen, nicht geschrieben und löst keinen Fehler aus (Laravel 13.25 schloss dieselbe Lücke). Owner, die über einen `NULL`-Foreign Key erreicht werden, und soft-gelöschte Owner werden ebenfalls übersprungen. Ein `touches`-Eintrag, der keine deklarierte `BelongsTo`-Relation benennt, ist jetzt ein Compilerfehler; polymorphe Owner werden noch nicht unterstützt.

- **`without_touching_on::<M, _, _>(fut)`** - Laravels `Model::withoutTouchingOn([M::class], $cb)`. Unterdrückt sowohl `m.touch()` als auch jede auf `M` zielende Owner-Kaskade, während Owner anderer Typen weiter erhöht werden. Scopes verschachteln sich, und das vorhandene `without_touching` unterdrückt jetzt neben direkten `touch()`-Aufrufen ebenfalls die Owner-Kaskade.

- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - Laravels `touchOwners()`, für den Fall, dass Sie die Child-Zeile über einen Pfad geschrieben haben, den das Framework nicht besitzt.

- **Wertgeformte Validierungsregeln: `ArrayKeys` und `Distinct`.** Ein neues Trait `ValueRule` (`passes(&self, value: &serde_json::Value)`) steht neben `Rule` und teilt denselben Vertrag für schlüsselbezogene Meldungen. `rules::ArrayKeys(&[...])` weist ein JSON-Objekt zurück, das einen Schlüssel außerhalb der erlaubten Liste trägt (Laravels `array:keys`, #60918); `rules::Distinct { ignore_case, strict }` weist ein JSON-Array mit einem wiederholten Element zurück (Laravels `distinct`). `validate!`-Zeilen akzeptieren beide Arten von Regeln in derselben Feldliste - der Dispatch erfolgt automatisch, gewählt nach dem Trait, das die Regel implementiert, nicht durch eine neue Zeilensyntax.

- **`Job::delay()`.** Jobs können eine Standardverzögerung deklarieren (`fn delay() -> Option<Duration>`, Standard `None`), die `Queue::push` und `Queue::bulk` berücksichtigen: `available_at` wird `now + delay` statt `now`. Eine explizite Verzögerung an der Aufrufstelle gewinnt weiterhin - `Queue::push_later(job, at)` und `Queue::later(delay, job)` verwenden den Timestamp des Aufrufers unverändert und konsultieren `Job::delay()` nie.

- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`.** Eine eingereihte Notification (`Notify::queue`) trägt jetzt ihre eigenen Queue-Tuning-Standards über das Primitive `EnvelopeOverrides`, das `Mail::on_queue` verwendet, auf jeden Push von `SendNotificationJob` pro Channel - `fail_on_timeout(&self) == true` legt beim ersten Timeout in die Dead Letter Queue, statt erneut zu versuchen, entsprechend Laravels Notification-Attribut `#[FailOnTimeout]` (#61072). Alle fünf verwenden standardmäßig die vorhandenen `Job`-Standards von `SendNotificationJob`, daher bleibt eine Notification ohne Überschreibung unberührt.

- **`Mail::on_queue` / `Mail::on_connection` + `Queue::push_with`/`later_with`.** Ein eingereihtes Mailable routet sich jetzt selbst mit `Mail::to(..).on_queue("emails").queue(mailable)` oder erhält Standards über `Mailable::queue(&self)`. Beide haben Vorrang vor jeder für den Job registrierten `Queue::route` und vor `Job::queue()`/`Job::connection()` des Jobs selbst. Das neue Primitive `EnvelopeOverrides` dahinter (`Queue::push_with(job, overrides)` / `Queue::later_with(delay, job, overrides)`) deckt zudem Timeout, Fail-on-Timeout, Max-Tries und Backoff für einen Push ab. Die eingereihten Snapshots von `MailFake` tragen jetzt die aufgelöste `queue`, mit `queued_on(...)` / `assert_queued_on(name, queue)` zum Assertieren.

- **`Application::http_bootstrap(f)`.** Ein HTTP-only-Boot-Hook. Er läuft nach `bootstrap` und nur auf dem Pfad `serve` / `web:run`, sodass Queue-, Schedule- und Workflow-Worker sowie das Konsolen-Binary ihn nie ausführen. Container-Images für Worker und Konsole brauchen zum Booten kein gebautes Frontend-Manifest mehr: `Inertia::install` schlägt in Produktion Fail-Closed fehl, wenn es fehlt, und diese Prüfung läuft jetzt nur in einem Prozess, der tatsächlich HTTP bereitstellt.

- **`Router::inertia(path, component, props)`.** Laravels `Route::inertia` für eine statische Seite, deren Handler eine Zeile wäre. Registriert `GET` (HEAD fällt darauf durch) und gibt einen `RouteBuilder` zurück, sodass die Route benannt werden und Middleware erhalten kann. `Router::view` bleibt als Alias erhalten.

- **SES-v2-Sendeoptionen.** Der SES-Transport gibt jetzt `TenantName`, `ConfigurationSetName` und `ListManagementOptions` bei `SendEmail` aus. Jede hat einen Standard auf Transportebene (`SesMailTransport::tenant_name` / `configuration_set_name` / `list_management`) und eine Header-Überschreibung pro Nachricht (`X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`, `X-SES-LIST-MANAGEMENT-OPTIONS`), wobei der Header gewinnt. Die Header werden beim Erstellen der Anfrage verbraucht und nie in die Nachricht gerendert.

- **`without_cookies` auf jedem Response-Builder.** `HttpResponse`, `Response` (über `ResponseExt`), `Redirect` und `RedirectRouteBuilder` lassen alle eine Liste von Cookies in einem Aufruf ablaufen, und `Redirect` / `RedirectRouteBuilder` erhielten das ihnen fehlende `without_cookie` für einen einzelnen Namen. Das neue `Cookie::forget_with(name, path, domain)` baut ein Lösch-Cookie, das auf den Pfad und die Domain begrenzt ist, mit denen das Original gesetzt wurde - ein einfaches `forget` löscht nie ein Cookie, das außerhalb von `/` gesetzt wurde.

- **`Queue::fake()` versieht jedes erfasste Push mit einer Envelope-ID.** `pushed_with_id::<J>()` gibt Paare `(job, id)` zurück, und das Fake dispatcht jetzt dasselbe Paar `JobQueueing` / `JobQueued`, das ein echter Driver-Push dispatcht - mit dieser ID -, sodass ein Test einen erfassten Push mit dem korrelieren kann, was seine Listener sahen. Bestehende Fake-Helfer bleiben unverändert.

- **Queue-Event `UniqueJobSkipped`.** `Queue::push_unique` dispatcht jetzt `queue::events::UniqueJobSkipped { job_name, unique_id, connection }`, wenn es ein Duplikat unterdrückt, sodass eine Deduplizierung beobachtbar statt still ist. Der Rückgabewert des Aufrufs bleibt unverändert (`Ok(false)`).

- **`model_keys()` auf dem Query Builder und auf Collections.** `User::query().model_keys().await?` gibt den Primärschlüssel jeder passenden Zeile zurück, ohne eine einzige Zeile zu hydrieren, und projiziert den tabellenqualifizierten Schlüssel (`users.id`), sodass die Query einen Join übersteht. `Collection::model_keys()` ist das bereits hydrierte Gegenstück. `#[suprnova::model]` deklariert jetzt außerdem den Rust-Typ des Schlüssels als `EloquentModel::Key`, sodass beide den durch `key_type` benannten Typ zurückgeben statt eines vom Aufrufer gewählten Turbofish.

### Behoben

- **PostgreSQL-Soft-Deletes verwenden jetzt Backend-bewusste Platzhalter, und generierte Timestamp-Schreibvorgänge berücksichtigen deklarierte Casts.** `delete()` und `restore()` rendern ordinale PostgreSQL-Platzhalter statt der `?`-Platzhalter von MySQL und SQLite. Generierte Schreibvorgänge für create, update, save, touch und soft-delete konvertieren Timestamps außerdem über den für jedes Feld deklarierten `Cast`-Speichertyp, sodass native `TIMESTAMPTZ`-Spalten keine Textwerte mehr erhalten. Danke an [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v), der beide Defekte gemeldet und einen Fix in [PR #3](https://github.com/eas4ai/suprnova/pull/3) eingereicht hat.

- **Standardmäßige Workspace- und Magnetar-Gate-Läufe erfordern keine laufenden PostgreSQL- oder MySQL-Dienste mehr.** Backend-spezifische Verhaltenssuiten sind explizite, ignorierte Qualifikationstests, die weiterhin fehlschlagen, wenn sie bewusst ohne ihre konfigurierte Datenbank aufgerufen werden. Reine Erreichbarkeitstests und dauerhafte Gate-Umgebungsanforderungen wurden entfernt, sodass unabhängige Änderungen nicht bei jedem Verifizierungslauf für externes Datenbank-Setup bezahlen.

- **`PartialFilter::narrow` ist jetzt `pub`.** Seine vier Geschwister-Prädikate (`should_include`, `should_include_eager`, `should_include_optional` und der Typ selbst) waren bereits öffentlich, aber der Narrowing-Pass, der die `true`-Antwort von `should_include_eager` korrekt macht - das Kürzen eines aufgelösten Werts auf die gepunkteten Pfade, nach denen ein `only`-/`except`-Eintrag tatsächlich gefragt hat - war `pub(crate)`. Ein Aufrufer, der auf `PartialFilter` eine eigene Partial-Reload-Behandlung aufbaut, hatte keine öffentliche Möglichkeit, dieses Narrowing zu reproduzieren, und lieferte bei einem gepunkteten `only`-Eintrag einen Wert vollständig, obwohl `should_include_eager` den Schlüssel als eingeschlossen meldete.

- **`QueuedSnapshot` von `MailFake` kann jetzt auf `.on_connection(...)` assertieren.** `Queue::fake()` erhielt in Wave 3 `assert_pushed_on_connection` neben `assert_pushed_on_queue`; `Mail::fake()` erhielt nur die Queue-Hälfte, sodass ein mit einer Connection-Überschreibung eingereihtes Mailable beim echten Dispatch aufgelöst und angewendet, aber über das Fake nicht assertierbar war. Neues `QueuedSnapshot::connection`, `MailFake::queued_on_connection` und `MailFake::assert_queued_on_connection` schließen die Lücke und spiegeln die Form von `assert_queued_on`.

- **Eine gepunktete Shared-Prop war über einen nackten `only`-Eintrag nicht erreichbar.** Auf `App::inertia_share("auth.user", …)` gefolgt von `router.reload({ only: ['auth'] })` antwortete `props: {"errors":{}}` - der Share verschwand vollständig. Das Registry speichert `auth.user` als einen wörtlichen Schlüssel und der Entpack-Pass `Arr::set` verschachtelt ihn erst, nachdem jede Prop aufgelöst wurde; deshalb sah das Partial-Reload-Gate den noch flachen Schlüssel und verglich ihn weder mit `auth` noch mit etwas anderem. `only`-/`except`-Einträge sind jetzt symmetrisch: Ein Eintrag kann den Schlüssel einer Prop genau benennen, einen Pfad *in* ihr (`user.name`, was begrenzt) oder einen **Vorfahren** von ihr (`auth` gegen den Schlüssel `auth.user`, was die Prop vollständig liefert, weil der Aufrufer nach der ganzen Wurzel fragte). Ein nacktes `except: ['auth']` verwirft jeden Prop-Schlüssel darunter genauso, wie `Arr::forget` im bereits verschachtelten Bag den gesamten Teilbaum verwirft. Das Präfix muss an einer Segmentgrenze enden, sodass eine unabhängige Prop `authAgent.user` von keiner Liste berührt wird. Laravel trifft dies nie, weil `Inertia::share` `Arr::set` zur Share-Zeit ausführt; Suprnovas Registry kann dies nicht, weil ein lazy Share keinen Wert zum Verschachteln hat, bis der Request ihn auflöst.

- **Ein Feld `#[data(lazy(deferred))]` umging die Allowlist `?include=`.** Der owner-getaggte Auflösungspfad in `resolve_props` wählte Props mit `Prop::is_lazy()`, was für alles mit einem Flag falsch ist - und ein deferred-Feld ist `Visibility::Deferred`. Das Feld löste daher über den gewöhnlichen Prop-Pfad auf, wo keine Include-Set-Prüfung existiert, und wurde an jeden Client geliefert, der den deferred Folge-Request sandte, unabhängig davon, ob der Request das Feld opt-in einschloss. `Prop::resolve_with_owner` begrenzt jetzt jede resolvergestützte owner-getaggte Prop, Flags hin oder her, und `resolve_props` führt dieses Gate vor jedem anderen Block aus: Ein Feld außerhalb von `?include=` wird vollständig verworfen (kein Wert, keine Ankündigung unter `deferredProps`), und ein durch `?include=` benanntes Feld außerhalb der Allowlist des DTO löst seinen `400` aus, bevor `X-Inertia-Partial-Data` ihn absorbieren kann. Keine Regression - der Code vor Wave 4 begrenzte auf die Enum-Variante `Prop::Lazy`, was auch für `Prop::Defer` fehlschlug -, aber in jedem Fall eine reale Lücke.

- **`deferredProps` wurde bei einem passenden Partial Reload erneut angekündigt.** Ein Partial, der einen deferred-Schlüssel nannte, kündigte dem Client weiterhin jeden *anderen* deferred-Schlüssel an, den dieser dann erneut holte, und beim nächsten Partial wieder. Laravels `resolveDeferredProps` gibt `[]` zurück, sobald der Request partial ist, bevor es eine einzige Prop inspiziert (`Response.php:661-663`); der Block wird jetzt bei jedem passenden Partial vollständig verworfen. Ein auf eine andere Komponente zielender Partial Reload ist für dieses Gate wie für jedes andere ein Standard-Visit, daher bleiben seine Ankündigungen unberührt.

- **Der `errors`-Bag wurde je nach Herkunft der Fehler unterschiedlich gefiltert.** Der aus der Session geflashte Bag wird vor der Resolve-Schleife gesetzt, und kein Partial-Reload-Filter konnte ihn erreichen, während ein eigenes `.with("errors", …)` eines Handlers durch die gewöhnlichen Gates lief. Daher lieferte `only: ['errors.email']` den gesamten gesetzten Bag, aber einen einfeldrigen Handler-Bag, und `only: ['users']` ersetzte den Handler-Bag durch den gesetzten Bag, statt den Schlüssel unangetastet zu lassen. Beide Pfade behandeln `errors` jetzt als immer sichtbar, entsprechend Laravels Middleware, die ihn als `Inertia::always(...)` teilt und den Rohwert nach dem Neuaufbau durch `only`/`except` über `resolveAlways` erneut injiziert. Der Client benötigt diese Form: Er faltet eine Partial-Response mit `{...current.props, ...response.props}` ein, sodass ein leeres `errors`-Objekt bereits sichtbare Meldungen löscht, ein ungefiltertes Objekt sie hingegen korrekt belässt. Ein explizites Sichtbarkeits-Flag auf dem Schlüssel gewinnt weiterhin, sodass `.prop("errors", Prop::eager(…).optional())` sich optional verhält.

- **`Queue::fake()` kann nun `EnvelopeOverrides` pro Push beobachten.** Ein über `Queue::push_with`/`Queue::later_with` geschobener Job war unter dem Fake von einem gewöhnlichen `Queue::push` nicht zu unterscheiden - `FakePush` trug nur die Payload und `available_at`, sodass die Überschreibung die Fassade nie verließ und nichts testen konnte, ob ein Dispatch die richtige Queue oder Connection verwendete. Das neue `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` gibt jeden erfassten Push gepaart mit seiner Deklaration zurück; `assert_pushed_on_queue::<J>(queue)` und `assert_pushed_on_connection::<J>(connection)` decken den üblichen Einfeld-Fall ab und entsprechen `MailFake::assert_queued_on`. Jeder andere Einstiegspunkt (`push`, `push_later`, `bulk`, `push_unique`, die Chain-/Batch-Dispatcher) nimmt weiterhin keine Überschreibungen und zeichnet `EnvelopeOverrides::default()` auf, sodass ein einfacher Push im Fake genau als „keine Überschreibung deklariert“ gelesen wird.

- **Ein SSR-Worker, der mitten im Response-Body stehen blieb, konnte ein Rendering endlos aufhalten.** `SsrConfig::timeout` begrenzte nur das Warten auf Response-Header; nach deren Eintreffen hatte das Lesen des Bodys kein eigenes Timeout, sodass ein Worker, der die Verbindung annahm, Header sandte und dann keine Daten mehr sandte, den Request über das konfigurierte Timeout hinaus hängen ließ, statt auf CSR zurückzufallen (oder unter `ssr_throw_on_error` einen Fehler zu erzeugen). Beide Phasen teilen jetzt eine Deadline, sodass das konfigurierte Timeout den gesamten SSR-Aufruf begrenzt, wie es die eigene Dokumentation bereits versprach.

- **Eingereihte Cookies - einschließlich des Remember-me-Cookies, das `Auth::login_remember` setzt - wurden auf drei internen Fail-Closed-Pfaden in `SessionMiddleware` stillschweigend verworfen.** Ein Session-Lesefehler, ein Session-Schreibfehler und ein Fehler bei der Session-Cookie-Verschlüsselung gaben jeweils direkt ein synthetisiertes `500` zurück und umgingen den Pending-Cookie-Drain, der am Ende von `handle` läuft. Alles, was in diesem Request über `Cookie::queue` eingereiht wurde - einschließlich einer bereits in die Datenbank committeten Remember-me-Token-Zeile - erreichte den Client nie als `Set-Cookie`-Header. Alle drei Pfade leeren Pending-Cookies jetzt vor der Rückkehr genauso wie ein vom Handler zurückgegebener Fehler oder Redirect. Dies deckt keine ungefangene Panic ab, entsprechend Laravels eigenen eingereihten Cookies, die bei einer solchen verloren gehen.

- **`Queue::push_unique` berücksichtigt jetzt `Job::delay()`, entsprechend `Queue::push`, `Queue::push_with` und `Queue::bulk`.** Es berechnete zuvor `available_at` direkt aus `Utc::now()`, sodass ein Job mit deklarierter Standardverzögerung (`fn delay() -> Option<Duration>`) bei einem Push durch `push_unique` sofort statt nach dieser Verzögerung dispatcht wurde. `Queue::push_unique_later` und `Queue::later_unique` bleiben unberührt - sie nehmen bereits einen expliziten Timestamp oder eine Verzögerung vom Aufrufer und konsultieren `Job::delay()` nie, dieselbe Regel wie bei `push_later`/`later`.

### Geändert

- **Der aktuelle Entwicklungs-Branch verwendet SeaORM 2.0 und erfordert Rust 1.94.0.** Suprnova erhält seine Quellformen für Eloquent, `#[model]`, Migrationen und die Datenbank-Fassade. Anwendungen, die SeaORM direkt aufrufen, müssen `ExprTrait` für SeaQuery-Ausdrucksmethoden importieren und explizite `*_raw`-Connection-Methoden für vorgebaute `Statement`-Werte verwenden. SeaQuery ist jetzt 1.0, und der direkte MariaDB-Vektortreiber verwendet SQLx 0.9. Bestehende Datenbanken erfordern keine Migration von Anwendungsdaten; frische PostgreSQL-Schemas behalten serial-gestützte Primärschlüssel.

- **Drei weitere ungenutzte Abhängigkeiten entfernt.** `pretty_assertions` und `qrcode` verlassen das Framework-Crate (`totp-rs` trägt das Feature `qr` bereits, daher bleibt die QR-Bereitstellung für Zwei-Faktor-Registrierung unberührt), und `notify-debouncer-mini` verlässt die CLI (`notify` selbst bleibt - die Watcher `serve` und `generate-types` verwenden es direkt). Alle drei wurden durch `cargo-udeps` sowie eine quellweite Suche mit Doc-Tests als ungenutzt bestätigt.

- **`suprnova-macros` hängt nicht mehr von `serde` oder `serde_derive_internals` ab.** Keines wurde verwendet: Die Pfade `::serde::Serialize`, die die Makros ausgeben, werden im Downstream-Crate aufgelöst, nicht im Macro-Crate selbst. Keine Auswirkung auf generierten Code.

- **`match_on` von `MergeStrategy` trägt jetzt mehr als einen Feldnamen.** `Append`, `Prepend` und `Deep` weiten jeweils von `match_on: Option<String>` auf `match_on: Option<Vec<String>>`, sodass `InertiaResponse::merge_with` / `merge_lazy_with` auf mehreren Feldern deduplizieren können, wie `.prop(key, Prop::eager(v).match_on([...]))` es bereits konnte - zuvor waren die Abkürzungen des Response-Builders weniger ausdrucksstark als das direkte Bauen einer `Prop`. Siehe Upgrade.

- **Scroll-Props geben jetzt Laravel-identisches `reset`- und Merge-Verhalten aus.** `scrollProps[key].reset` ist genau dann `true`, wenn der Client `key` in `X-Inertia-Reset` nannte, entsprechend Laravels `resolveScrollProps` - nicht wie zuvor bei jedem Visit ohne Header `X-Inertia-Infinite-Scroll-Merge-Intent`. Eine Scroll-Prop trägt jetzt außerdem unbedingt Merge-Metadaten, standardmäßig append: Ein frischer Visit (gar keine Header) gibt `reset: false` plus einen Eintrag in `mergeProps` aus, während er zuvor `reset: true` und keine Merge-Metadaten ausgab. Ein Schlüssel in `X-Inertia-Reset` wird für diese Response aus `mergeProps` / `prependProps` ausgeschlossen, dieselbe Ausnahme, die eine reguläre Merge-Prop bereits hatte.

- **`ssr:check` prüft jetzt, dass die Route `GET /health` des SSR-Workers 2xx antwortet**, statt nur zu bestätigen, dass etwas eine TCP-Verbindung annahm. Jeder `@inertiajs/{vue3,react,svelte}/server`-Worker antwortet standardmäßig auf `/health`, daher erforderte dies keine Änderung auf Worker-Seite - entspricht Laravels `Inertia\Ssr\HttpGateway::isHealthy()`.

- **Die Inertia-Prop `errors` trägt jetzt einen String pro Feld, kein Array.** Ein aus der Session geflashter Validierungs-Bag rendert als `{ email: "The email field is required." }` statt `{ email: ["The email field is required."] }`, entsprechend Laravels Standard und dem `ErrorValue = string` von Inertia. `InertiaConfig::with_all_errors(true)` stellt die Array-Form wieder her. Eine Prop `errors`, die ein Handler selbst setzt, wird unverändert durchgereicht, und der Session-Flash (`Redirect::with_errors`, `session.pull_errors_flash()`) speichert weiterhin Arrays - nur die gerenderte Seiten-Prop ändert sich.

- **`Model::TOUCHES` wanderte von einer inhärenten Konstante zu `EloquentModel`.** Die Parent-Touch-Kaskade lebt in einem Standard eines `Model`-Traits, und ein Trait-Standard kann keine inhärente Konstante lesen. `Comment::TOUCHES` löst sich weiterhin auf - es benötigt jetzt `use suprnova::EloquentModel;` im Scope. Models ohne `touches`-Attribut erhalten den leeren Standard des Traits.

- **`RelationEntry` erhielt `related_updated_at_column`.** Alles, was ein `RelationEntry` von Hand konstruiert, benötigt das zusätzliche Feld; nichts im Tree tut dies, das Makro erzeugt sie alle.

- **`Router::view` weist jetzt Props zurück, die kein JSON-Objekt sind.** Zuvor ignorierte es sie stillschweigend und registrierte eine Route, die ohne Diagnose einen leeren Prop-Bag renderte. `null` wird weiter als „keine Props“ akzeptiert; `Router::try_inertia` ist die fehlbare Form.

- **Die Inertia-Asset-Version ist jetzt standardmäßig ein Hash des Vite-Build-Manifests** statt des wörtlichen `"1.0"`, sodass ein Deployment lang lebende Clients invalidiert, ohne dass jemand daran denken muss, einen String zu erhöhen. `InertiaConfig::manifest_path(...)` richtet damit auch den Resolver neu aus; ein explizites `.version(...)` / `.version_with(...)` gewinnt weiterhin. Ohne Manifest auf dem Datenträger - lokale Entwicklung - fällt die Version auf `"1.0"` zurück, was jede Anwendung zuvor sah, daher ändert sich nichts, bis Sie bauen. Das neue `VersionResolver::from_manifest(path)` legt den Resolver direkt offen.

### Veraltet

- **`Cookie::read_encrypted` ist jetzt der nur-v1-Legacy-Reader.** Code, der mit `Cookie::encrypted` erzeugt und mit `read_encrypted` liest, schlägt beim ersten nach diesem Release geschriebenen Wert zur Laufzeit fehl; wechseln Sie zu `read_encrypted_for(name, wire)`. Die nicht kontextbezogenen Einstiegspunkte `CryptPurpose::Cookie` sind ebenfalls ersetzt. Beide Entfernungen sind für 1.4.0 geplant.

### Upgrade

- **Cookie-Entschlüsselungswarnungen haben jetzt zwei unabhängige Achsen.** Eine Warnung `KeyOrigin::Previous(index)` bedeutet, den Wert unter dem aktuellen `APP_KEY` erneut zu verschlüsseln und diesen vorherigen Schlüssel erst zu entfernen, wenn der Rotation-Tail verschwunden ist; eine Warnung `AadVersion::Legacy` bedeutet, das Cookie vor der Entfernung des Fallbacks in 1.4.0 über die namensgebundene API erneut auszustellen. Ein Wert kann beides melden.

- **`SESSION_COOKIE_PREFIX` ist ein Opt-in.** Stellen Sie `__Host-` nur mit HTTPS, `SESSION_SECURE=true`, `SESSION_PATH=/` und ohne `SESSION_DOMAIN` bereit; lokale HTTP-Scaffolds lassen es leer. `CsrfMiddleware` mit `with_session_config` behält den wörtlichen Namen `XSRF-TOKEN`; verwenden Sie `.xsrf_cookie_name("__Host-XSRF-TOKEN")`, wenn ein Client für diesen separaten Namen konfiguriert ist.

- **`DecryptOrigin` ist jetzt eine zweiachsige Struktur `#[non_exhaustive]`.** Lesen Sie ihre Felder `key` und `aad` unabhängig und behalten Sie eine wildcard-kompatible Match-Strategie für die Enums `KeyOrigin` / `AadVersion` bei.

- **`SessionConfig` und `CookieOptions` sind jetzt `#[non_exhaustive]`.** Struktur-Literale und funktionale Record-Updates im Anwendungscode müssen zu `Type::default()` gefolgt von Zuweisungen öffentlicher Felder oder Builder-Methoden wechseln.

- **`FrameworkError` ist jetzt `#[non_exhaustive]`.** Ein `match` darauf in Ihrem eigenen Code benötigt einen Wildcard-Arm. Dies ist das letzte Release, in dem das Hinzufügen einer Variante eine Breaking Change gewesen wäre.

- **Das Feld `match_on` von `MergeStrategy::Append`/`Prepend`/`Deep` ist jetzt `Option<Vec<String>>`, nicht `Option<String>`.** Eine Aufrufstelle, die die Struktur-Literal-Form direkt konstruiert - `MergeStrategy::Append { match_on: Some("id".into()) }` - kompiliert nicht mehr; verpacken Sie den Feldnamen in ein `Vec`: `Some(vec!["id".into()])`. `match_on: None` ist nicht betroffen und benötigt keine Änderung.

- **Ein passender Partial Reload gibt `deferredProps` nicht mehr aus.** Code, der `page.deferredProps` aus einer Partial-Reload-Response liest - eine benutzerdefinierte Deferred-Loading-Komponente, ein Test-Snapshot, eine End-to-End-Assertion - findet den Schlüssel jetzt nicht mehr vor, wo er zuvor die deferred Props auflistete, die der Request nicht nannte. Lesen Sie die Ankündigungen beim initialen (nicht partialen) Visit, dort platziert Laravel sie und dort liest der offizielle Client sie.

- **Ein nackter `except`-Eintrag verwirft jetzt gepunktete Prop-Schlüssel darunter.** `X-Inertia-Partial-Except: auth` ließ zuvor eine unter `auth.user` registrierte Prop in der Response, weil das Gate ganze Schlüssel verglich. Sie wird jetzt verworfen. Wenn eine Seite darauf vertraute, dass ein nackter `except`-Eintrag nur den exakten Schlüssel reduziert, benennen Sie den exakten Schlüssel (`except: ['auth.user']`) oder begrenzen Sie stattdessen mit einem gepunkteten Pfad.

- **`errors` ignoriert `only`/`except`.** Ein Partial Reload, der eine vom Handler gelieferte Prop `.with("errors", …)` herausfilterte oder mit einem gepunkteten Eintrag begrenzte, liefert sie jetzt vollständig. Tests, die ein geschnittenes oder leeres Objekt `errors` bei einem Partial Reload assertieren, müssen aktualisiert werden. Um den Bag bewusst aus einer Response herauszuhalten, markieren Sie ihn - `.prop("errors", Prop::eager(…).optional())` - statt sich auf die Partial-Reload-Listen zu verlassen.

- **`Prop::resolve_with_owner` begrenzt auch geflaggte Props.** Es löste zuvor jede Prop auf, die nicht `Prop::is_lazy()` war - einen eager Wert *oder* einen Resolver mit Flag -, ohne das Include-Set zu konsultieren. Es begrenzt jetzt jede resolvergestützte Prop und lässt nur einen bereits materialisierten Wert unbegrenzt passieren. Ein Feld `#[data(lazy(deferred))]` benötigt daher `?include=<field>` im Request, bevor es aufgelöst oder angekündigt wird, wie jeder andere lazy Geschmackszustand. Fügen Sie das Feld zur `?include=`-Liste des Requests hinzu oder entfernen Sie das Attribut `lazy(...)`, wenn es nie Opt-in sein sollte.

- **`reset` einer Scroll-Prop folgt nicht mehr dem Merge-Intent-Header.** Code, der `page.scrollProps[key].reset` direkt liest - eine eigene Infinite-Scroll-Komponente, ein Test-Snapshot - sieht bei einem einfachen Revisit jetzt `reset: false` (plus einen Eintrag in `mergeProps`), wo zuvor `reset: true` ohne Merge-Metadaten stand. Die offizielle Komponente `<InfiniteScroll>` verhält sich nur bei einem einfachen Revisit anders: Sie lauscht auf `reset` bei jedem `success`-Event von `router`, nicht nur bei einem expliziten `router.reload()`, daher löscht ein normaler Revisit den gesammelten Zustand nicht mehr, sofern der Server den Schlüssel nicht tatsächlich in `X-Inertia-Reset` nannte, was Laravel entspricht. Senden Sie `X-Inertia-Reset: <key>` überall explizit, wo Sie sich auf das frühere Verhalten „jeder nicht-append/prepend-Visit setzt zurück“ verlassen haben.

- **`Prop::match_on` nimmt `impl MatchOnFields`, nicht `impl Into<String>`.** Die neue Bound erlaubt einem Aufruf, mehrere Felder zu benennen (`match_on(["id", "slug"])`), und ihre Impl-Liste ist bewusst abgeschlossen - nur `&str`, `String`, `[T; N]` und `Vec<T>`. Eine Blanket-Impl über `IntoIterator` ist nicht verfügbar: Coherence weist sie gegen die `&str`- und `String`-Impls zurück, da nichts verhindert, dass diese Typen später ein `IntoIterator`-Impl erhalten. Drei Argumenttypen, die zuvor kompilierten, tun dies nicht mehr: `&String`, `Cow<'_, str>` und `Box<str>`. Übergeben Sie stattdessen ein `&str` an der Aufrufstelle - `match_on(name.as_str())` für ein `&String`, `match_on(name.as_ref())` für ein `Cow<'_, str>`, `match_on(&*name)` für ein `Box<str>`.

- **Ein gepunkteter `only`-/`except`-Eintrag begrenzt seine Top-Level-Prop jetzt, statt sie vollständig auszuschließen.** Vor diesem Fix ließ `X-Inertia-Partial-Data: user.name` `should_include_eager` nach einem exakten Eintrag `"user"` suchen, fand keinen und verwarf die ganze Prop `user` stillschweigend - ein Client, der ein Feld von `user` anforderte, erhielt nichts. Jede Frontend-Seitenkomponente, die sich zufällig auf diese Lücke verließ (und einen gepunkteten `router.reload({ only: [...] })` als Auslassen des Schlüssels behandelte), erhält jetzt stattdessen `{ user: { name: ... } }`. Keine Code-Änderungen sind erforderlich - das Inertia-v3-Protokoll spezifiziert bereits diese Bedeutung des Request-/Response-Vertrags. Derselbe Fix gilt für `should_include_optional` und wirkt operativ stärker: Ein gepunkteter `only`-Eintrag (`permissions.read`) zählt jetzt als explizite Anfrage für den Top-Level-Schlüssel einer `Optional`- oder `Defer`-Prop, die zuvor einen nackten Eintrag (`permissions`) benötigte, um überhaupt auszulösen. Ein Request, der den Resolver dieser Prop zuvor vollständig übersprang, führt ihn nun aus. Trifft der Resolver eine Datenbank oder einen externen Service, beginnt ein Client, der bereits gepunktete Partial-Reload-Requests sendet, auf Requests Arbeit auszuführen, die zuvor keine erzeugten. Beobachten Sie nach dem Upgrade das Resolver-Aufrufvolumen, wenn Ihre App `Optional`-/`Defer`-Props mit gepunktetem Partial-Reload-Traffic hat.

- **`InertiaSharedData::share` nimmt jetzt den Namen der Seitenkomponente.** Fügen Sie nach `req` einen Parameter `component: &str` hinzu:
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```

  Ignorieren Sie ihn (`_component`), wenn Ihr Provider nicht nach Seite variieren muss - Laravels `RenderContext` trägt dieselbe Paarung (`component`, `request`) für `ProvidesInertiaProperties::toInertiaProperties`.

- **`Prop` ist eine Struktur, kein Enum.** Seine Varianten sind verschwunden; konstruieren und lesen Sie Props über Methoden:
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (`Prop::lazy(closure)` ist unverändert)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` -> `prop.is_absent()`
  Die Payload-Strukturen `DeferConfig`, `MergeConfig`, `OnceConfig` und `ScrollConfig` sind entfernt - ihre Felder sind jetzt Flags auf `Prop`. `Prop::is_deferred()` wird in `Prop::has_resolver()` umbenannt, was es immer schon bedeutete. `DeferOptions`, `OnceOptions`, `MergeStrategy`, `ScrollMetadata` und jede Builder-Methode von `InertiaResponse` bleiben unverändert, daher braucht eine App, die nur den Response-Builder verwendet, keine Änderungen. Apps, die Props von Hand bauen - typischerweise eine `InertiaSharedData`-Implementierung -, benötigen die obigen Umbenennungen.

- **Dieser Fix schützt bereits vorhandene Sessions, nicht nur Requests ab jetzt.** Das Upgrade allein genügt: Ein von einem früheren Release geschriebenes Session-Cookie kann eine nie sanitierte `_previous.url` tragen, und `SessionData::previous_url()` verwirft sie jetzt beim ersten Lesen nach dem Upgrade, statt ihr zu vertrauen, weil sie bereits gespeichert ist. Sie müssen bestehende Sessions nicht invalidieren, die Session-Tabelle nicht migrieren und keinen erneuten Login erzwingen. Ein Request, dessen Pfad protokollrelativ aussieht (`//host`), aktualisiert die gespeicherte vorherige URL künftig ebenfalls nicht mehr. Wenn die `fallback!`-Route Ihrer App (oder irgendeine mit `200` antwortende Route, die über einen ungewöhnlichen Pfad erreichbar ist) jemals rechtmäßig darauf vertraute, dass ein solcher Pfad zum Ziel von `Redirect::back()` wird, tut sie dies nicht mehr. In jedem Fall bleibt stattdessen der vorherige sichere Wert in der Session erhalten (oder das eigene Fallback von `Redirect::back(fallback)` gewinnt, falls nie etwas Sicheres gespeichert war). Keine Code-Änderung ist erforderlich, sofern Sie nicht vom exakten Randfall abhingen, den dies schließt und der bereits ein Open-Redirect-Risiko war.

- **Entfernen Sie das `[0]` aus jeder Bindung `errors.<field>` in Ihren Seiten.** Bei der neuen Standardform ist `errors.email` ein String; `errors.email[0]` rendert daher sein erstes Zeichen statt der Meldung. Ändern Sie zugleich den TypeScript-Typ von `string[]` zu `string`. Wenn Sie Ihre Seiten lieber nicht anfassen möchten, setzen Sie `InertiaConfig::with_all_errors(true)` auf der Konfiguration, die Sie an `Inertia::install` übergeben, und fügen Sie die Modul-Augmentation `errorValueType: string[]` für `@inertiajs/core` hinzu. Die Starter-Frontends liefern die neue Form aus.

- **Ein Handler, der den Redirect-zurück nach einem Validierungsfehler von Hand gebaut hat, kann ihn löschen.** Die Brücke ist jetzt automatisch; ein Handler, der weiterhin selbst redirectet, funktioniert weiter, weil die Middleware nur auf einen `422` mit einem gefüllten Objekt `errors` reagiert.

- **Ein abgestürztes Child von `suprnova serve` startet jetzt neu, statt die Session zu beenden.** Wenn Sie sich darauf verlassen haben, dass ein Absturz `suprnova serve` vollständig beendet (ein CI-Smoke-Check, ein Skript, das Ende als „etwas stimmt nicht“ behandelt), übergeben Sie `--no-restart`, um dieses Verhalten genau wiederherzustellen. Wiederholungen sind außerdem standardmäßig begrenzt: Ein Prozess, der fünfmal hintereinander abstürzt, wird nicht weiter wiederholt (erhöhen Sie die Grenze mit `--restart-tries` oder verwenden Sie `--no-restart` für das ursprüngliche Verhalten „ein Absturz und fertig“).

- **`Model::TOUCHES` ist keine inhärente Konstante mehr.** Code, der `Comment::TOUCHES` direkt gelesen hat, benötigt `use suprnova::EloquentModel;` (oder `suprnova::eloquent::EloquentModel`) im Scope - die Konstante zog dorthin, damit die Parent-Touch-Kaskade, ein `Model`-Trait-Standard, sie lesen kann. Ein `grep -rn TOUCHES` über Ihre App findet jede Aufrufstelle; die meisten Apps haben keine, weil die Konstante zur Laufzeit zuvor nichts tat.

- **`RelationEntry` erhielt ein Feld.** Nur Code, der ein `RelationEntry` von Hand konstruiert, benötigt eine Änderung - fügen Sie `related_updated_at_column` zum Literal hinzu. Die vom Framework ausgelieferten macro-generierten Relationsregistrierungen geben es bereits aus, daher ist eine gewöhnliche App, die nichts weiter tut als Relationen über `#[suprnova::model]` zu deklarieren, nicht betroffen.

- **`Router::view` mit Nicht-Objekt-Props panickt jetzt beim Boot.** Es registrierte zuvor stillschweigend mit einem leeren Prop-Bag; `view` delegiert an `Router::inertia`, das ein Objekt (oder `null`) erfordert und andernfalls panickt. Kann ein `view`-Aufruf Nicht-Objekt-Props tragen, wechseln Sie zu `Router::try_inertia` und behandeln Sie das `Err` - ansonsten ändert sich für Sie nichts.

- **Der Standard für das Inertia-Version-Manifest kann Ihren Versionsstring ändern, sobald ein Build existiert.** Eine App oder ein Test, der `X-Inertia-Version: 1.0` hartcodiert, funktioniert nur, bis ein Vite-Manifest auf dem Datenträger erscheint; sobald eines erscheint, wird die Version stattdessen der Manifest-Hash. Benötigen Sie die alte Konstante, lesen Sie sie selbst aus `VersionResolver::from_manifest(path)` oder pinnen Sie `.version(...)` explizit. Rechnen Sie damit, dass das erste Deployment nach dem Upgrade für bereits verbundene Clients einen vollständigen Page-Reload-Zyklus erzwingt - einmalig und Zweck der Änderung. Der Fallback-Wert ohne Manifest wird als `suprnova::MANIFEST_VERSION_FALLBACK` exportiert, daher müssen Sie `"1.0"` nie wieder hartcodieren.

- **Verschieben Sie die Registrierung von `Inertia::install` und `global_middleware!` aus `bootstrap::register`.** Legen Sie sie in eine neue Funktion und übergeben Sie diese stattdessen an `.http_bootstrap(...)` - die neue Form des Scaffolds ist ein synchrones `register_http_stack()`, aufgerufen als `.http_bootstrap(|| async { bootstrap::register_http_stack() })`. Apps, die dies überspringen, behalten das heutige Verhalten bei, einschließlich des Worker-Boot-Fehlers bei fehlendem Frontend-Manifest.

## 1.2.4 - 2026-08-18

### Sicherheit

- **Das Bypass-Secret des Wartungsmodus wird in konstanter Zeit
  verglichen.** `MaintenanceMiddleware` matchte die Secret-URL mit einem
  einfachen String-Vergleich, der beim ersten abweichenden Byte
  zurückkehrt. Da das Secret ein im Anfragepfad mitgeführtes
  Bearer-Credential ist, verriet dieser Zeitunterschied einem Angreifer,
  wie lang das von ihm korrekt geratene Präfix war. Der Vergleich läuft
  jetzt über die volle Bytelänge via `subtle::ConstantTimeEq` und bricht
  nur bei abweichender Länge vorzeitig ab - dieselbe Form wie der
  Bypass-Cookie-Vergleich daneben.

- **`rules::Url` lehnt jetzt Skript-URIs ab.** Die Regel akzeptierte
  jedes Schema, das `url::Url` parsen konnte, `javascript:` und
  `vbscript:` eingeschlossen, sodass eine validierte URL beim Rendern in
  ein `href` immer noch eine Senke für Skript-Ausführung sein konnte. Sie
  wendet jetzt die Form von Laravels `url`-Regel an (das Muster
  `^(PROTOCOLS)://HOST` aus `Illuminate\Support\Str::isUrl`): Das Schema
  muss auf Laravels Allowlist stehen, von `://` gefolgt werden **und**
  von einem nicht leeren Host - Laravels Host-Gruppe hat kein `?`, sodass
  ein fehlender oder leerer Host selbst mit gelistetem Schema nie matcht.
  Die Schema-Liste und die Anforderung aus `://` plus Host sind wörtlich
  Laravels; der Host selbst wird vom `url`-Crate geparst statt von
  Laravels Regex, sodass sich ein paar Randfälle weiterhin
  unterscheiden - ein Port außerhalb des gültigen Bereichs wird hier
  abgelehnt und dort akzeptiert, und IDN-Hosts normalisieren
  unterschiedlich. Das neue
  `Url::protocols(&[...])` spiegelt Laravels `url:http,https`; `HttpUrl`
  ist jetzt wörtlich Zucker dafür und behält seine eigene Meldung.
  **Verhaltensänderung:** Eine URL mit einem nicht gelisteten Schema, die
  früher validierte, schlägt jetzt fehl - benennen Sie das Schema mit
  `Url::protocols(&["myapp"])`, falls Sie es akzeptieren wollten. Zwei
  weitere Verhaltensänderungen: `mailto:`, `data:` und `tel:` stehen
  namentlich auf Laravels Allowlist, tragen aber keine
  Authority-Komponente und schlagen daher jetzt fehl; und Pfade der Form
  `file:///etc/passwd` - `scheme://` mit nichts zwischen den letzten
  beiden Schrägstrichen - schlagen ebenfalls fehl, denn eine leere
  Zeichenkette ist auch kein Host. Beides folgt aus Laravels eigener
  Regel aus `://` plus Host.

- **Inertia-Responses geben jetzt überall `Vary: X-Inertia` an.** Der
  Header wurde nur auf den Page-Objekt-Responses selbst gesetzt.
  Redirects, 404er, 422er und statische Responses trugen keinen, sodass
  ein geteilter Cache, der allein auf die URL geschlüsselt ist, einer
  harten Browser-Navigation das JSON-Page-Objekt ausliefern konnte oder
  einem Inertia-XHR die HTML-Shell. Die neue `InertiaHeadersMiddleware` -
  von `Inertia::install` als äußerste der drei registriert - setzt ihn
  auf jeder Response und verwandelt eine leere `200` bei einem
  Inertia-Besuch in ein `303` zurück, statt in eine Response, die der
  Client als nicht-Inertia ablehnt. `InertiaVersionMiddleware` flasht
  jetzt vor ihrer `409` die Session erneut, sodass ein geflashter Fehler
  das darauf folgende vollständige Seiten-GET des Clients überlebt.

- **Drei Fixes an Inertia-Responses.**
  `InertiaResponse::location_for(&req, url)` liefert `409` +
  `X-Inertia-Location` für ein Inertia-XHR und ein einfaches `302` + `Location` für eine harte Navigation, sodass ein außerhalb der SPA
  begonnener OAuth- oder SSO-Bounce nicht mehr in einer `409` ohne Body
  endet. Das bestehende `location(url)` behält seine Form mit immer
  `409`. Das neue `App::clear_history()` flasht das History-Clear-Flag in
  die Session, sodass es den Logout-Redirect überlebt und auf der Seite
  landet, die tatsächlich rendert - das Per-Response-`.clear_history()`
  markierte nur den Redirect, den der Browser wegwirft, und ließ die
  verschlüsselte History der vorherigen Session entschlüsselbar. Und eine
  `once`-Prop wird jetzt nur bei einem vollständigen Inertia-Besuch
  übersprungen: Ein explizites `router.reload({ only: ['stats'] })` löst
  sie erneut auf, statt nichts zurückzugeben.

- **Der SES-Transport sendet jetzt eigene Message-Header.** `Mail::to(..)
  .header("List-Unsubscribe", ...)` und `Mailable::headers()` wurden unter
  `MAIL_DRIVER=ses` still verworfen: Der `Content.Simple`-Request-Body
  hatte kein `Headers`-Feld, und der Raw-MIME-Builder las
  `OutgoingMessage::headers` nie, obwohl jeder andere Transport sie
  weiterreicht. Beide SES-Pfade tragen sie jetzt - `Headers` als
  `{Name, Value}`-Liste von SES v2, Raw-MIME als echte Header-Zeilen -,
  sodass Abmeldelinks, Threading-Header und Routing-Hinweise einen
  Treiberwechsel überleben. Header-Namen werden auf beiden Pfaden vorab
  validiert - CR, LF und NUL (die Injection-Bytes, die der
  Mailgun-Transport bereits ablehnt) und alles, was kein gültiger
  RFC-5322-Feldname ist (Leerzeichen, Doppelpunkte, Nicht-ASCII) -, sodass
  das Anhängen einer Datei nie ändert, ob eine Nachricht akzeptiert wird.

### Behoben

- **Verschachtelte Validierungsfehlschläge erreichen jetzt den
  422-Body.** Fehlschläge von `#[validate(nested)]` an einer
  verschachtelten Struktur oder an einem Element eines validierten
  `Vec<T>` gingen zwischen Validator und Response verloren: Die Anfrage
  wurde korrekt mit 422 abgelehnt, aber die `errors`-Map kam leer zurück,
  sodass keine Meldung gerendert wurde und der Client nicht erkennen
  konnte, welches Feld fehlerhaft war. Verschachtelte Fehlschläge werden
  jetzt neben den Fehlschlägen der obersten Ebene in Laravels
  Punktnotation abgeflacht - `address.street`, `items.1.name`,
  `order.items.2.sku`.

- **Das `url` des Inertia-Page-Objekts behält den Query-String.**
  `page.url` war nur der Anfragepfad, sodass der Client für einen Besuch
  auf `/users?page=2&sort=name` `/users` verzeichnete. Jede
  Vor-/Zurück-Navigation und jedes `router.reload()` spielte die Seite
  dann ohne ihren Pagination-Cursor, ihre Sortierung und ihre Filter
  erneut ab. Es ist jetzt Pfad plus Query - dieselbe Ableitung, die
  `InertiaVersionMiddleware` bereits für `X-Inertia-Location` verwendete,
  sodass die beiden standardmäßig Byte für Byte übereinstimmen. Das neue
  `InertiaConfig::url_resolver(...)` überschreibt, wie das *Page-Objekt*
  die Seite benennt (Laravels `Inertia::resolveUrlUsing`); der
  Versions-Bounce benennt weiterhin die URL, die ankam, denn das ist die
  URL, die der Browser holen muss.

- **`Inertia::install` wendet seine Config jetzt auf jede Response an.**
  Die an `Inertia::install` übergebene Config wurde für drei Felder
  gelesen und dann verworfen, sodass jede ohne explizites
  `.with_config(...)` gebaute `InertiaResponse` aus
  `InertiaConfig::default()` renderte. Eine mit `--frontend react` per
  Scaffold erzeugte App lieferte den Svelte-Einstiegspunkt und keine
  React-Refresh-Präambel aus, sofern `SUPRNOVA_FRONTEND` nicht in der
  Umgebung gesetzt war; auf der Config aktiviertes SSR erreichte nie eine
  Response; und die Asset-Version des Page-Objekts kam aus einer anderen
  Config als der Resolver der Versions-Middleware. Die installierte
  Config wird jetzt auf der Inertia-Registry des Containers vorgehalten
  und ist das, womit `InertiaResponse::new` startet. Ein
  Per-Response-`.with_config(...)` überschreibt weiterhin, Apps, die
  `Inertia::install` nie aufrufen, sind unverändert, und eine
  fehlgeschlagene (geschlossen fehlschlagende) Installation hält nichts
  vor. Als Nebeneffekt wird das Produktions-Vite-Manifest jetzt einmal
  pro Prozess statt einmal pro Response geparst.

- **Per Scaffold erzeugte Apps installieren jetzt die
  Inertia-Protokoll-Middlewares.** Die von `suprnova new` geschriebene
  `bootstrap.rs` registrierte die Session-, Locale-, CSRF- und
  Include-Middlewares, rief aber nie `Inertia::install` auf, sodass eine
  generierte App weder `InertiaVersionMiddleware` noch
  `Inertia303Middleware` hatte: Einem Browser, der noch das vorherige
  Bundle ausführte, wurde nach einem Deploy nie gesagt, dass er neu laden
  soll, und ein `PUT`/`PATCH`/`DELETE` mit Redirect blieb auf einer
  `302`, der der Client mit dem ursprünglichen Verb folgen konnte. Der
  Aufruf landet jetzt nach `SessionMiddleware` - wo das erneute
  Session-Flashen der Versions-Middleware funktioniert - mit einer
  benannten `INERTIA_VERSION`-Konstante, die bei Asset-Änderungen zu
  erhöhen ist, und er pinnt das Frontend, mit dem das Projekt generiert
  wurde (`.frontend(Frontend::React)` für `--frontend react`), sodass die
  HTML-Shell den Vite-Einstiegspunkt dieses Frameworks lädt, statt auf
  den von Svelte zurückzufallen. Die generierte `.env` setzt jetzt
  passend `SUPRNOVA_FRONTEND`. Der `--api`-Starter ist unverändert; er
  hat kein Frontend.

- **`Queue::push_unique` meldet einen eingereihten Job nicht mehr als
  übersprungen.** Der Rückgabewert wurde mit
  `matches!(outcome, Idempotent::Fresh(()))` berechnet, was
  `Idempotent::FreshUnfenced` zu `false` faltete - den Ausgang, bei dem
  die Envelope *zwar* gepusht wurde, das Dedupe-Lease aber mitten im Push
  verloren ging. Aufrufern, die auf diesem Boolean verzweigten, wurde
  gesagt, ein Job, der gleich laufen würde, sei als Duplikat unterdrückt
  worden. Alle drei Ausgänge werden jetzt erschöpfend gematcht: Ein
  verlorenes Lease liefert `true` mit einem `warn`, das den Job und
  seinen Unique-Key benennt, und nur ein echtes Duplikat liefert `false`.
  `push_unique_later` und `later_unique` teilen den Pfad und sind mit ihm
  behoben.

### Geändert

- **Die Parity-Baseline ist auf Laravel 13.25.0 umgezogen.** Die Release
  Notes zu 13.23.0, 13.24.0 und 13.25.0 wurden Punkt für Punkt auf die
  eigene Oberfläche des Frameworks zurückverfolgt. Alles, was einen
  Suprnova-Codepfad erreichte, ist in diesem Release entweder behoben
  oder hat in [`manual/parity.md`](../parity.md) eine Zeile, die mit
  `not yet` oder `by design no` markiert ist.

### Upgrade

Zwei Änderungen können eine laufende App verändern, ohne dass Sie auf
Ihrer Seite Code ändern.

- **Einstellungen auf der Config, die Sie an `Inertia::install`
  übergeben, wirken jetzt.** Sie wurden für drei Felder gelesen und
  verworfen. Wenn Ihre Install-Config `.ssr(...)` setzt, ist SSR jetzt
  an: Starten Sie den Worker (`suprnova ssr:start`), bevor Sie deployen,
  oder entfernen Sie den `.ssr(...)`-Aufruf. Auch `.entry_point`,
  `.assets_base_url`, `.default_title` und `.encrypt_history(...)`, die
  dort gesetzt werden, erreichen jetzt die Seite.

- **`rules::Url` lehnt mehr ab.** Werte, die früher durchgingen und es
  jetzt nicht mehr tun: jedes Schema außerhalb von Laravels Allowlist,
  darunter `javascript:` und `vbscript:`; `mailto:`, `data:` und `tel:`,
  die auf der Allowlist stehen, aber keinen `://`-Host tragen; und
  `scheme://` mit leerem Host, etwa `file:///path`. Wenn Sie ein Schema
  akzeptieren wollten, benennen Sie es: `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Behoben

- **Datetime-Casts lesen jetzt datenbanknativen `CURRENT_TIMESTAMP`-Text.**
  `AsDateTime`, `AsImmutableDateTime` und `AsOptionalDateTime` schreiben
  weiterhin kanonisches RFC-3339, akzeptieren beim Lesen aber auch den
  zeitzonenbehafteten PostgreSQL-Text und zeitzonenfreie SQLite-/MySQL-Werte.
  Zeitzonenfreie Werte werden gemäß dem UTC-Timestamp-Vertrag des Frameworks
  als UTC interpretiert.

## 1.2.2 - 2026-08-14

### Behoben

- **Nullable Nicht-Text-Werte funktionieren unter PostgreSQL jetzt bei allen
  attributbasierten Schreibvorgängen.** Typisierte `Builder::update_all` und
  `Builder::upsert`, modelllose `DB::table().insert/update` sowie zusätzliche
  Attribute in Viele-zu-viele-Pivots geben explizite JSON-Nullwerte als SQL
  `NULL` aus und binden weiterhin jeden Nicht-Null-Wert. Dadurch bleibt der Typ
  der Zielspalte erhalten, statt einen als Text typisierten Nullparameter zu
  senden, den PostgreSQL für bigint-, integer-, boolean-, timestamp- und andere
  Nicht-Text-Spalten ablehnt. Mehrzeilige Upserts lehnen jetzt außerdem
  fehlende oder zusätzliche Spalten ab, statt eine fehlerhaft geformte Zeile
  stillschweigend in null umzuwandeln. Automatische Zeitstempel von
  Viele-zu-viele-Pivots werden als typisierte UTC-Datumswerte statt als Text
  gebunden.

### Sicherheit

- **Das Release-Gate unterscheidet jetzt im gesamten Workspace zwischen
  ruhenden Lockfile-Metadaten und kompilierten Abhängigkeiten.** Cargo
  verzeichnet die ungenutzte optionale rkyv 0.7-Kompatibilitätsabhängigkeit
  von rust_decimal in `Cargo.lock`; das Gate weist jetzt nach, dass weder rkyv
  noch dessen Derive-Crate von irgendeinem Workspace-Mitglied, Feature, Target
  oder einer Abhängigkeitskante erreichbar ist. Die zugehörige
  RustSec-Ausnahme ist zugewiesen, läuft am 2026-11-14 ab und muss entfernt
  werden, sobald rust_decimal diese veraltete optionale Abhängigkeit nicht mehr
  verzeichnet.

## 1.2.1 - 2026-08-09

### Geändert

- **Suprnova ist von der GitHub-Organisation `entrepeneur4lyf` zu `eas4ai`
  umgezogen.** Repository-URLs
  in Paketmetadaten, Dokumentation, Abhängigkeitsbeispielen und
  Scaffold-Vorlagen verwenden jetzt `github.com/eas4ai`. Neue Projekte verwenden
  außerdem die überwachte Autorenadresse `shawn@eas4ai.com`. Diese Version
  änderte kein Runtime-Verhalten.

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
  übersprungen wurde, weshalb v0.5.10 und v0.6.1-v0.6.3 nur getaggt
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
