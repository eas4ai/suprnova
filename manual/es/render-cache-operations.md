# RenderCache Operaciones

Una caché que no puedes ver es una caché en la que no puedes confiar.
RenderCache responde a dos preguntas del operador de forma directa y sin
imprimir jamás una página almacenada: **¿qué tiene este nodo bajo esta
clave, y sigue vigente?** y **¿cómo hago que todo se detenga?** Responde a
una tercera, «¿se está sirviendo esta ruta desde una copia almacenada,
siquiera?», mediante la telemetría y la cabecera `Age` en lugar de mediante
un comando, porque esa pregunta va de tráfico y no de una entrada. Hay dos
comandos de consola, seis contadores de telemetría, un barrido de disco
acotado y una palanca de emergencia.

Este capítulo es la superficie operativa: los comandos, exactamente qué
imprimen y qué pueden ver; los contadores y sus conjuntos cerrados de
desenlaces; cómo el nivel de archivo recupera disco; cómo probar una ruta
cacheada de modo que la prueba demuestre el cacheo y no la mera respuesta;
qué hacer cuando algo va mal, incluido el procedimiento multinodo que
necesita una restauración de base de datos; y cómo se mide el rendimiento
de la propia caché y cuánto valen honestamente esos números. Los ejemplos
de comandos son los que
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch`
ejecuta a través del propio punto de entrada de consola de este repositorio
en `app/tests/live_render_cache.rs`.

## Los dos comandos de consola

Ambos son comandos ocultos, registrados por el framework y alcanzables a
través del binario `console` de tu proyecto como cualquier otro. Ninguno
imprime jamás un cuerpo almacenado ni una identidad de dependencia en
bruto.

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** informa de la forma de una entrada
almacenada: su clase de representación, sus `body_bytes`, sus demás
metadatos y el epoch de autoridad actual a su lado, de modo que puedas
saber si la entrada que estás mirando sigue siendo autoridad vigente o ya
ha caducado por debajo. Imprime `no entry (current epoch: {epoch})` cuando
la clave no nombra nada que pueda ver, y falla, no informa de éxito, ante
una clave que no puede analizar o sin runtime instalado.

**Lee la L0 en proceso de este proceso y nada más.** `RenderCache::inspect`
busca la clave solo en L0; nunca consulta el nivel L1. En el perfil de base
de datos o de Redis eso importa: una entrada que está viva en
`suprnova_render_entries` o en Redis, publicada por otro nodo o por este
antes de un reinicio, imprime `no entry` aquí salvo que este proceso la
haya servido desde que arrancó. Lee el informe como «lo que este nodo tiene
en memoria», nunca como «lo que el despliegue tiene almacenado». Lo mismo
vale para `RenderCache::store_inspection`, que informa de la ocupación de
L0 y del epoch actual.

La clave es el texto que usa la propia búsqueda: `rk1.` más 43 caracteres
base64url, que es lo que pueden mostrar el logging y la telemetría de tu
aplicación. No es un segundo hash de nada, así que una clave que tiene un
operador nombra exactamente una entrada.

Esa afirmación de ausencia de cuerpo está comprobada, no meramente
enunciada. La prueba toma el documento que se sirvió realmente, lo parte en
líneas y exige que **toda** línea no trivial del mismo esté ausente de lo
que imprimió el informe de inspección.

**`render-cache:epoch-advance`** es la invalidación de emergencia. Avanza
el epoch de autoridad e imprime `epoch advanced to {epoch}`. Como el epoch
está incorporado en toda clave de búsqueda, esto pone las entradas
almacenadas fuera de alcance sin nada que enumerar y nada que borrar. La
prueba asevera la línea impresa y después la consecuencia que importa:
tras el comando, la ruta vuelve a renderizar.

**En el nodo que lo ejecuta**, el efecto es inmediato: el comando descarta
el lease de epoch de ese proceso y vacía su nivel en proceso, así que su
siguiente petición deriva claves bajo el nuevo epoch y no encuentra nada.
(Esa última cláusula se sostiene mientras el epoch solo avance, que es el
caso ordinario; después de una restauración de base de datos el valor
avanzado puede ser uno que el despliegue ya haya usado, así que consulta
«Restaurar la base de datos» más abajo.) **En cualquier otro nodo**, el
libro mayor se ha movido pero ese proceso sigue teniendo su viejo epoch
arrendado y su propia L0, y se pone al día en su siguiente lectura de
autoridad: de inmediato bajo `CoherenceMode::Authority`, y hasta
`max_age_ms` después bajo `CoherenceMode::Lease`. Ejecuta el comando en
cada nodo, o reinicia los demás. «Restaurar la base de datos», más abajo,
tiene el procedimiento completo y las pruebas que hay detrás.

Recurre a él cuando algo va mal con el contenido cacheado y no puedes
esperar a que las entradas individuales expiren, y después de un job que
cambió lo que muestran las páginas cacheadas (consulta «huecos conocidos»
en [RenderCache Generaciones](render-cache-generations.md)).

## Cambios de permisos

`RenderCache::bump_permission_version().await?` es la única llamada de
invalidación que hace una aplicación a mano, y en realidad no es un comando
de operaciones: pertenece al camino de código que cambia lo que un usuario
con sesión iniciada tiene permitido hacer. Avanza una generación persistida
que observa todo render con clave por principal, sobrevive a un reinicio, y
se une a la transacción en la que se ejecuta el cambio de rol cuando existe
una. Sin ella, un usuario cuyos permisos acaban de cambiar sigue
coincidiendo con lo que se cacheó bajo su anterior conjunto de permisos.

## Telemetría

Seis nombres de contador cerrados, y nada en ninguno de ellos nombra un
nivel, un proveedor o un backend:

| Contador | Atributo |
|---|---|
| `suprnova.render_cache.lookups` | `outcome` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | ninguno |
| `suprnova.render_cache.rebuilds` | ninguno |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |

`lookups` y `hits` llevan el mismo conjunto cerrado de ocho desenlaces:

- `l0`, `l1` - una entrada fresca servida desde el nivel en proceso o desde
  el compartido.
- `conditional` - un acierto fresco cuyo `If-None-Match` coincidió,
  respondido con `304`.
- `stale` - una entrada obsoleta-servible servida de inmediato, o el
  repliegue obsoleto-ante-error después de que fallara una reconstrucción
  en primer plano.
- `miss` - no se encontró nada, una reconstrucción obsoleta-ante-error en
  curso, o una entrada muerta.
- `bypass` - un parámetro de query no declarado, una dimensión de varianza
  declarada que no se puede resolver, o una lista de espera agotada.
- `moved` - la relectura posterior al render encontró que una dependencia o
  el epoch habían cambiado; la candidata se descartó, nunca se publicó.
- `declined` - el render no era almacenable: elegibilidad, un informe de
  observación desbordado, una clasificación `Uncacheable`, una regla de
  documento Live, o una cota.

`hits` se incrementa solo para `l0`, `l1`, `conditional` y `stale`.
`publications` cuenta solo un store que responde «publicado», nunca un
intento vallado o rechazado. `rebuilds` cuenta uno por cada reconstrucción
en segundo plano lanzada.

Los dos contadores de cosido llevan sus propios conjuntos: `assembled` y
`fail_document` para los ensamblajes; `rendered`, `omitted`, `fallback` y
`failed` para los slots.

**Una tasa alta de `declined` es la señal por la que vale la pena alertar.**
Significa que rutas que incluiste están renderizando y sirviendo
correctamente sin ser almacenadas nunca, y la respuesta tiene el mismo
aspecto en ambos casos. La comprobación local más rápida son dos peticiones
seguidas: si la segunda no lleva cabecera `Age`, no se almacenó nada.

## Higiene de disco

Solo el nivel de archivo necesita barrido, y en su mayor parte se barre
solo.

`FileRenderStore` almacena un archivo por clave, plano bajo
`RENDER_CACHE_L1_DIR`. Una entrada está muerta cuando su antigüedad desde
la publicación alcanza la retención con la que se publicó, o cuando su
epoch de valla es más antiguo que el epoch actual. La retención sale del
mismo borde muerto consciente de la clase que usa la comprobación de
frescura en vivo, así que el archivo de una entrada privada se retira antes
que el de una pública y un barrido nunca puede discrepar de una
comprobación de frescura sobre si una entrada está verdaderamente muerta.

`sweep` elimina como mucho 64 entradas por llamada, primero las de
publicación más antigua, y devuelve si quedan más. Se ejecuta
automáticamente en cada 256.ª publicación, así que un directorio sano no
necesita atención. `RenderCache::sweep()` lo provoca explícitamente cuando
quieras, y un atasco mayor que el límite de una llamada se drena a lo largo
de disparos posteriores en lugar de bloquear en un solo escaneo largo.

Dos cosas que el barrido no es:

- **Un avance de epoch no toca L1.** Vacía L0 de plano, porque eso es
  memoria en proceso sin nada contra lo que reconciliar, y deja en disco
  todo archivo anterior al epoch hasta que un barrido lo recupera. Eso es
  higiene de disco, no una cuestión de corrección: los archivos ya son
  inalcanzables por búsqueda.
- **El nivel de base de datos no tiene barrido automático**, y solo se
  recupera mediante `RenderCache::sweep()`. El nivel de Redis no necesita
  ninguno: cada entrada que almacena lleva una expiración y Redis recupera
  los bytes por su cuenta.

La publicación es segura ante caídas. Escribe un archivo temporal, le hace
fsync, lo renombra sobre el destino y hace fsync del directorio padre, de
modo que un lector solo llega a ver el archivo completo anterior o el
archivo completo nuevo. Al abrir, el store elimina cualquier archivo
temporal sobrante y cualquier archivo que falle su comprobación de marco,
tratando una escritura rota como algo que se cura solo en lugar de como una
entrada envenenada para siempre.

## Probar una ruta cacheada

Una prueba que asevera que una ruta cacheada responde correctamente pasa
tanto si la respuesta salió del store como si salió de un render nuevo.
Toda afirmación hay que hacerla contra algo que solo pueda producir una
entrada almacenada sirviéndose de verdad. Cuatro patrones hacen eso, y las
propias pruebas de dogfood de este repositorio usan los cuatro:
`app/tests/live_render_cache.rs` con el harness de
`app/tests/live_support/mod.rs`.

**1. Cuenta los renders del lado del handler de la caché.** Registra un
middleware contador *después* de `RenderCache::install`. El registro añade
al final, así que aterriza más cerca del handler que
`RenderCacheMiddleware`, y una petición que responde la caché vuelve antes
de llamarlo:

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// After the install, so it only sees requests the cache forwarded.
render_counter::register();
```

La diferencia entre dos lecturas de `render_counter::renders()` es entonces
el número de renders que la caché no evitó, y nada más, a diferencia de
cuerpos idénticos o de una cabecera `Age`, que tienen ambos explicaciones
honestas ajenas a la caché. Toda aseveración de acierto de
`an_orm_write_invalidates_the_todos_document_through_generations` descansa
sobre ella. Espera un render que no despachaste (una reconstrucción en
segundo plano) con la propia barrera del contador,
`wait_until_renders_at_least`, nunca con una espera temporizada.

**La excepción es una ruta `PublicShellStitched`, y no es una excepción
pequeña.** Un acierto cosido se reenvía deliberadamente por toda la cadena
de la ruta: su guarda de autorización tiene que ejecutarse otra vez, y solo
el middleware de finalización de Live al final de esa cadena sirve el
acierto. Un middleware contador registrado globalmente después de la
instalación queda fuera de la cadena propia de la ruta, así que se alcanza
en un acierto cosido exactamente igual que en un fallo. En una ruta así el
contador no puede sostener en absoluto la afirmación de «no se ejecutó
ningún handler».

Asevera en su lugar sobre lo que guarda el store, sobre de qué está hecho el
documento servido y sobre qué antigüedad tiene, que es lo que hace
`the_dashboard_is_stitched_per_principal_from_one_shared_shell`: la entrada
almacenada es `EntryKind::Composite` con el número de slots esperado
(`inspect_route_for_test`), los documentos de dos principales difieren en sus
etiquetas de isla y en ningún otro sitio, y la respuesta al segundo principal
informa de un `Age` de los segundos enteros transcurridos desde que se
publicó el shell.

Eso último es la prueba sobre la que se sostiene el test, y es el comprobante
local de servicio desde el store en su forma exacta. La cabecera `Age` por
sí sola es la señal débil de la que este capítulo advertía más arriba, porque
un render también pone una: a cero. El número no es débil: un render publica
su respuesta y su entrada en el mismo instante, así que una respuesta
renderizada informa de cero por lejos que haya avanzado el reloj, mientras
que un ensamblaje informa de la antigüedad del shell del que se ensambló. El
test mueve un reloj ajustable, bien dentro de la ventana de frescura de la
ruta, para que lo que lee sea exacto y no incidental.

La respuesta lleva además `Cache-Control: private, no-store`, pero léelo por
lo que es: la directiva que lleva una ruta con slots de esta clase, fijada
tanto en el render que publica el shell como en cada ensamblaje posterior,
porque sigue lo que contienen los bytes y no la ruta que los produjo. «Con
slots» es la expresión clave: un `Composite` sin slots conserva en su lugar
el `max-age` privado de la clase, así que la directiva dice algo de una ruta
con islas y nada de una sin ellas. Esa prueba asevera
`renders() == before + 1` en un acierto, y dice en su propia nota por qué esa
es la lectura honesta y no un fallo.

**2. Vuelve a leer la entrada.** Dos llamadas de la fachada son API pública
corriente: `RenderCache::store_inspection()` informa de la ocupación de L0,
los bytes y el epoch actual, y `RenderCache::inspect(key_text)` informa de
los metadatos sin cuerpo de una entrada. Junto a ellas, el framework expone
costuras de prueba ocultas, marcadas `#[doc(hidden)]` y nombradas
`_for_test` para que nada las confunda con API de aplicación:

| Costura | Qué le da a una prueba |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | el texto de clave que deriva el middleware **en el epoch 1**, el valor que siembra la migración |
| `RenderCache::key_for_route_at_epoch_for_test(pattern, params, login, epoch)` | lo mismo, bajo un epoch que tú nombras |
| `RenderCache::inspect_route_for_test(pattern)` | la entrada L0 de esa clave de epoch 1: clase, tipo, estado, `body_bytes`, slots |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | lo mismo, pero sacado del nivel L1 configurado |
| `RenderCache::clear_l0_for_test()` | vacía L0 y deja en paz L1, el epoch y el coordinador |

El epoch importa porque forma parte de la clave.
`key_for_route_for_test` fija el epoch 1 en el código, así que una prueba
que ha avanzado el epoch, en este nodo o, a través del libro mayor, en
otro, debe nombrar el nuevo con `key_for_route_at_epoch_for_test` o buscará
una clave bajo la que no se publicó nada.

`the_public_document_is_a_hit_whose_seed_still_promotes` usa
`store_inspection` e `inspect_route_for_test` para aseverar que la entrada
existe y está almacenada bajo la clase declarada;
`the_database_profile_serves_a_hit_through_the_sql_stores` usa
`inspect_l1_for_test` y luego `clear_l0_for_test`, que es la única manera
de demostrar que una petición posterior salió de L1 y no de la memoria.

**3. Mueve el reloj en lugar de esperar.** El reloj que lee el runtime se
puede establecer en un `RenderCacheConfig` y nunca mediante `from_env`, así
que una prueba que necesita una banda de frescura instala el suyo:

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Bound to its own name first: passing `Arc::clone(&clock)` inline leaves
// the compiler inferring the trait object as the clone's return type.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... install through the application's own configuration seam, then:
clock.advance_ms(300_001);
```

`AdjustableTestClock` viene de `suprnova::live::testing`, y `unix_now_ms`
es la propia lectura de reloj de pared del harness, de modo que un reloj
ajustable arranca donde está el del sistema y no en un origen de tiempos
con el que el resto del proceso no estaría de acuerdo. (Eso es un cero de
reloj, no el epoch de autoridad que este capítulo entiende por lo demás con
esa palabra.) El harness envuelve la pareja como `setup_app_with_clock` y
`advance_clock_ms`, el segundo de los cuales entra en pánico en lugar de no
hacer nada en silencio cuando el arranque tomó el reloj del sistema.
`stale_service_is_marked_and_rebuilt_in_the_background` es la prueba.

**4. Cuenta sentencias SQL.** Una caché que se saltó el handler pero aun
así consultó la base de datos en cada acierto satisface todo contador del
lado del handler y sigue costando un viaje de ida y vuelta.
`DbConnection::observe_statements_for_test` apunta el callback de métricas
de SeaORM a un contador tuyo, y ve las sentencias del pool y de toda
transacción iniciada desde él:

```rust
// Immediately after connecting, before the connection is cloned or bound
// into the container: installing needs sole ownership of the pool, and the
// call reports `false` rather than counting nothing silently.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

Al callback no se le dice nada sobre la sentencia (ni texto SQL ni valores
enlazados) porque un recuento es todo el objetivo.
`framework/tests/render_cache/bypass.rs` está escrito enteramente sobre
este patrón: `a_lease_mode_hit_runs_nothing_and_issues_no_statement`
mantiene un acierto en modo de lease en cero sentencias,
`an_authority_mode_hit_issues_exactly_one_statement` mantiene un acierto en
modo de autoridad en una, y `the_epoch_is_read_once_at_first_use` mide dos
fallos uno contra otro para mostrar que el epoch cuesta una lectura por
runtime.

Dos hábitos que conviene mantener. Arranca el harness a través de la propia
costura de configuración de tu aplicación y no a través de un router
construido a mano, de modo que la prueba instale las mismas rutas,
políticas y ordenación de middleware que instala el servidor. Y no añadas
nunca una espera temporizada: cada barrera de más arriba es una barrera de
estado sobre un contador, que es lo que hace que estas pruebas sean
reproducibles en lugar de inestables.

## Cuando algo va mal

- **Una página muestra contenido que sabes que es viejo.** Comprueba si la
  ruta está almacenando siquiera (dos peticiones, busca `Age`). Si lo está,
  y la escritura que debería haberla invalidado vino de un worker de cola,
  una tarea programada o un comando de consola, esa escritura no avanzó
  nada: ejecuta `render-cache:epoch-advance` (por nodo; ver el último
  punto).
- **Una página que esperabas que cacheara nunca lleva cabecera `Age`.**
  Está siendo rechazada, no fallando. Repasa la lista de clasificación de
  [RenderCache](render-cache.md): una lectura de sesión, una lectura de
  identidad en una ruta sin varianza `Principal`, una lectura de locale sin
  varianza `Locale`, una comprobación de autorización, o una lectura de SQL
  en bruto.
- **Un backend está inalcanzable.** `RENDER_CACHE_FAILURE` decide: `open`
  (el valor por defecto) sirve la ruta sin cachear, `closed` responde un
  `503` escueto. Un backend que falta en el arranque detiene el arranque en
  su lugar, con una frase que nombra la migración o la variable que hay que
  arreglar.
- **Redis se vació o se reinició.** Las entradas fallan y se vuelven a
  renderizar. Nada obsoleto puede demostrarse vigente: la vigencia se
  demuestra contra el libro mayor de generaciones de la base de datos,
  nunca contra el nivel que tenía los bytes.
- **Un líder de reconstrucción murió a mitad de reconstrucción.** Su lease
  se toma en cuanto el tiempo del store pasa la expiración, y la propia
  publicación del antiguo líder queda vallada fuera en lugar de correr una
  carrera con la nueva. No publica nada; la respuesta de su petición se
  sigue sirviendo.
- **Un archivo de L1 quedó roto por una caída o un disco lleno.** Nada lo
  sirve. Cada archivo lleva un digest sobre su propio marco, así que un
  archivo truncado o alterado falla esa comprobación y es un fallo; el
  store lo elimina, junto con cualquier archivo temporal sobrante, la
  próxima vez que abre. Aquí una escritura rota se cura sola en lugar de
  ser una entrada envenenada para siempre.
- **La base de datos se restauró desde una copia de seguridad.** Esto tiene
  un procedimiento en lugar de una frase; consulta «Restaurar la base de
  datos» más abajo.
- **Necesitas que todo desaparezca, ya.** `render-cache:epoch-advance`. Con
  más de un nodo, ejecútalo en cada uno, o reinicia aquellos en los que no
  lo ejecutaste: el avance mueve el epoch del libro mayor para todo el
  despliegue, pero vacía L0 y descarta el epoch arrendado solo en el
  proceso que lo ejecutó. El procedimiento de restauración de más abajo
  detalla por qué.

## Restaurar la base de datos

El libro mayor de generaciones es la autoridad contra la que se demuestra
cada acierto, así que restaurar la base de datos cambia lo que significa
«vigente» para toda entrada ya almacenada. Tres hechos del código deciden
qué hace a continuación una entrada almacenada, y ninguno de ellos es «se
descarta en silencio».

**Una entrada movida no se retiene automáticamente.** La comparación de
coherencia (`CoherenceCheck::compare`) es una desigualdad en *cualquiera*
de las dos direcciones, así que una entrada almacenada cuyas generaciones
observadas difieren de las del libro mayor restaurado es un movimiento
hacia donde sea que fueran los números. Pero un movimiento no es una
negativa a servir: el middleware evalúa una entrada movida con una
antigüedad efectiva de al menos su intervalo fresco (`freshness_state` en
`framework/src/render_cache/middleware.rs`), y en una ruta que declara una
ventana obsoleta-servible eso la deja en la banda obsoleta-servible. **Al
visitante se le sirve una vez la copia previa a la restauración, bajo
`Warning`, mientras la reconstrucción se ejecuta por detrás de la
petición.** Ese es el mismo traspaso que describe
[RenderCache Generaciones](render-cache-generations.md), y el paso 4 de
`an_orm_write_invalidates_the_todos_document_through_generations` lo
asevera. Una ruta `PrivateCached` nunca hace esto, porque su borde muerto
es su borde fresco, y tampoco lo hace una ruta que no declaró ventana
obsoleta-servible; ambas reconstruyen en primer plano.

**Un avance de epoch es por proceso.** `RenderCache::advance_epoch` avanza
el epoch del libro mayor, luego descarta el lease de epoch de *este*
proceso y vacía la L0 de *este* proceso. Sus nodos hermanos conservan
ambas cosas: sus entradas de L0 y el epoch previo a la restauración que
tienen arrendado. Cada uno se entera en su siguiente lectura de autoridad,
de inmediato en su siguiente acierto bajo `CoherenceMode::Authority` y
hasta `max_age_ms` después bajo `CoherenceMode::Lease`, que es exactamente
lo que miden las tres pruebas `an_epoch_advanced_by_another_node_*` de
`framework/tests/render_cache/middleware.rs`. Hasta entonces un hermano
puede servir una entrada previa a la restauración, y en una ruta
obsoleta-servible puede servirla bajo `Warning` como arriba.

**El nivel compartido no lo barre un cambio de epoch por sí solo.** El
barrido del nivel de archivo elimina una entrada cuando su retención ha
transcurrido *o* su epoch de valla es `<` que el actual. Si restaurar la
copia de seguridad bajó el epoch del libro mayor por debajo de valores bajo
los que el despliegue ya había publicado entradas, esas entradas llevan un
epoch de valla que ahora es *mayor* que el actual, así que esa cláusula no
las recupera; en su lugar esperan a que se agote su retención. El nivel de
base de datos solo lo barre un `RenderCache::sweep()` explícito. El nivel
de Redis se recupera solo, pero según el calendario del propio Redis: cada
hash de entrada se almacena bajo `<RENDER_CACHE_REDIS_PREFIX>entry:<key>`
(prefijo por defecto `suprnova_render:`) con un `PEXPIRE` fijado a partir
de la retención de la entrada, así que esperar a que se agote la retención
más larga que hayas declarado es la opción pasiva.

Así que el procedimiento, en orden:

1. **Ejecuta `render-cache:epoch-advance` una vez**, antes de que el
   despliegue restaurado sirva. Falla ruidosamente en lugar de informar de
   éxito cuando falta el singleton de epoch, que es además cómo te enteras
   de que la migración no volvió con los datos.
2. **Vacía el nivel L1 compartido.** Borra el contenido del directorio del
   nivel de archivo, `DELETE FROM suprnova_render_entries`, o borra las
   claves de Redis que coincidan con `<prefix>entry:*`, según el nivel que
   configure el perfil. Haz esto en lugar de esperar un barrido, por la
   razón de más arriba.
3. **Cubre la L0 de cada nodo, con el tráfico todavía cortado.** El avance
   solo vació el nodo que lo ejecutó, así que hasta que este paso esté
   hecho un hermano sin cubrir todavía puede servir una vez una entrada
   previa a la restauración, y por eso el tráfico sigue cortado hasta aquí
   y no hasta el paso 2. O reinicias los demás nodos (un proceso nuevo
   tiene una L0 vacía y ningún epoch arrendado, así que su primera petición
   lee la autoridad restaurada) o ejecutas `render-cache:epoch-advance` en
   cada uno de ellos, lo cual vacía la L0 de cada uno según se ejecuta. La
   segunda opción sube el epoch del libro mayor una vez por nodo, lo que no
   cuesta nada: a partir de ahí el epoch solo avanza, y todos los nodos
   acaban leyendo el último valor. Ambas son seguras; el reinicio es el más
   sencillo de razonar, y es el único que no necesita aritmética de modo
   `Lease`.

Los pasos 2 y 3 son lo que hace que el paso 1 sea completo en lugar de
parcial. Sáltatelos y, en una ruta con una ventana obsoleta-servible, una
representación previa a la restauración todavía puede servirse una vez:
correctamente marcada con `Warning`, y reconstruida justo después, pero
servida.

## Medirlo

RenderCache trae dos benchmarks, y son **herramientas bajo demanda, nunca
pasos de la puerta**:

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

Eso ejecuta el bench del motor (`render_cache_budget`, las mediciones de
acierto en caliente y de ensamblaje de compuestos con un asignador que
cuenta), luego el bench de carga de trabajo del framework
(`render_cache_workloads`, la misma ruta a través de todo el middleware) y
luego la prueba de contrato sobre los resultados registrados en el
repositorio. Ambos están fijados a `SUPRNOVA_LIVE_S1_CPUSET`.

Una ejecución completa necesita un PostgreSQL desechable (`PG_TEST_URL`) y
un Redis desechable (`REDIS_TEST_URL`), porque el contrato de resultados
registrados exige los tres perfiles registrados. **Una ejecución parcial
debe redirigir ambos archivos de resultados** con
`SUPRNOVA_LIVE_BENCH_RESULT` y `SUPRNOVA_LIVE_WORKLOADS_RESULT` bajo
`benchmarks/local/`; sin eso sobrescribe los resultados registrados con un
archivo más corto y después falla su propio contrato.

Los números registrados, de
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` y
`render-cache-workloads-v1.json`:

| Medición | Valor |
|---|---|
| Trabajo del motor para un acierto L0 `Complete` fresco, p95 | 0,76 microsegundos |
| Asignaciones de heap, acierto fresco | 3 |
| Asignaciones de heap, acierto condicional `304` | 3 |
| Asignaciones de heap, acierto acotado por un plazo de semilla | 4 |
| Copias de cuerpo en cualquiera de ellos | ninguna; el búfer se comparte |
| La misma ruta a través del middleware, lado servidor, p95 | 14,4 microsegundos |
| La misma petición sobre un viaje HTTP de ida y vuelta por loopback, p95 | 109 microsegundos |
| Sentencias SQL por acierto en caliente (modo de lease) | 0 |

Las cifras del middleware son para un cuerpo de 65.536 bytes cuyo render
leyó 12 filas, registradas como 14 identidades de dependencia observadas.

**Léelas como exploratorias, no como evidencia cualificada.** Todo
resultado registrado en el repositorio lleva
`"classification": "local_exploratory"` y `"s1_requirements_met": false`:
se produjeron en una estación de trabajo de desarrollo con una CPU
compartida, un gobernador `powersave` y proveedores por loopback. Sirven
para detectar una regresión de todo un orden de magnitud y para nada más
fino. Un número es evidencia cualificada solo cuando se produjo en el
runner dedicado con su atestación establecida, y estos no.

### Por qué Suprnova diverge

Los paquetes de cacheo de respuestas de Laravel dejan las operaciones al
store de caché que hay debajo. Inspeccionar una entrada significa encontrar
su clave a mano y leer el valor, que es la página renderizada, así que
mirarla significa imprimir el HTML de alguien en una terminal, e invalidar
todo significa vaciar un store que también contiene tus sesiones, tus
límites de velocidad y tu cola. La observabilidad es lo que el driver del
store emita por casualidad.

Suprnova le da a la caché su propia superficie operativa, deliberadamente
estrecha. La inspección carece de cuerpo por construcción, de modo que un
operador puede confirmar que una entrada existe, bajo qué clase está
almacenada y cuán grande es, sin que se le muestre nunca su contenido. La
invalidación es una subida de epoch que no cuesta nada aplicar y que toca
solo esta caché: tus sesiones y tu cola no están en el radio de la
explosión. La telemetría es un conjunto cerrado de seis contadores con
conjuntos de atributos cerrados, que es lo que hace que un panel sobre
ellos sea estable entre versiones en lugar de un conjunto de cadenas que se
va a la deriva. El intercambio es que no hay comando de «borra esta clave»:
las palancas son de solo lectura por entrada, o de alcance de epoch.

## Siguiente

- [RenderCache](render-cache.md) - las declaraciones sobre las que operan
  estos comandos
- [Observabilidad](observability.md) - dónde se exportan los contadores de
  más arriba
- [Pruebas](testing.md) - las convenciones de prueba del entorno en el que
  se insertan los patrones de más arriba
- [Despliegue](deployment.md) - la lista de verificación de producción que
  los rodea
