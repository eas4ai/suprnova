# RenderCache Generaciones

La mayoría de las cachés expiran. RenderCache también expira, pero la
expiración es la red de seguridad y no el mecanismo. El mecanismo es una
**generación**: cada pieza de datos que leyó un render tiene un contador en
la base de datos, el render almacena los contadores que vio, y una
escritura avanza el contador de lo que haya cambiado. Una representación
almacenada está vigente cuando los contadores que vio siguen coincidiendo
con los contadores que la base de datos tiene ahora. Tú no escribes ninguna
regla de invalidación para tus propios datos, porque un `model.save()`
corriente ya lo es.

Este capítulo trata de esa maquinaria desde fuera: de qué se registra que
depende un render, cuán gruesas son en realidad esas dependencias, qué no
puede ver el framework y por tanto no puede invalidar, cómo se paga la
comprobación de coherencia en un acierto, qué petición reconstruye cuando
varias quieren la misma entrada a la vez, y qué se le sirve a un visitante
en la ventana entre «ya no está vigente» y «reconstruida». Toda afirmación
de más abajo está sujeta por una prueba con nombre o por una medición
registrada en el repositorio; los ejemplos de dogfood son rutas de
`app/src/live/mod.rs` demostradas por `app/tests/live_render_cache.rs`.

## De qué se registra que depende un render

Mientras se ejecuta un render, un recolector con ámbito de petición
registra cada dependencia que puede nombrar: la lectura de una tabla, la
lectura de un registro por clave primaria, una clase de consulta, una
relación, una identidad de configuración, una característica, un locale,
una ruta, y una identidad `Broad` siempre presente que observa toda
representación. Las lecturas a través del ORM y del constructor de
consultas se registran solas; tú no escribes nada.

`/live/todos` es el patrón entero en un solo handler:

```rust
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView { count: titles.len(), titles })
    }
    .await;
    result.map_err(failed)
}
```

`Todo::all()` registra la tabla `todos`. Nada más en el handler ni en la
plantilla lee la sesión, el visitante con sesión iniciada o una traducción,
que es lo que permite que la ruta siga siendo siquiera una representación
compartida.

## Una escritura corriente es la invalidación

`an_orm_write_invalidates_the_todos_document_through_generations` recorre
el ciclo entero a través de la aplicación en ejecución:

1. El primer `GET /live/todos` renderiza y publica.
2. El segundo es un acierto: no llega nunca al handler, no lleva `Warning`
   y no programa nada.
3. Un `POST /todos/random` escribe una fila, a través de la propia ruta de
   la aplicación, con la sesión y el token CSRF que enviaría un navegador.
4. El siguiente `GET` se sirve con `Warning: 110 - "Response is Stale"` y
   programa exactamente una reconstrucción en segundo plano. Sus cinco
   minutos frescos apenas han empezado, así que la generación avanzada de
   la tabla `todos` es lo único que puede explicar cualquiera de las dos
   cosas.
5. Esa reconstrucción se ejecuta de verdad: la prueba espera sobre el
   contador de renders (una barrera de estado, no una espera temporizada)
   hasta que ha ocurrido un render que la propia prueba no despachó.
6. La fila escrita está de verdad en el listado. Este es un paso
   **separado** y deliberadamente no es una aseveración sobre la salida
   propia de la reconstrucción en segundo plano: la prueba tira L0 primero
   y vuelve a renderizar, porque la publicación de la reconstrucción
   aterriza en un momento que nada alcanzable desde la aplicación hace
   observable, así que aseverar sobre la petición que resultara atraparla
   sería una carrera.
7. Y la ruta se asienta de vuelta en un acierto simple contra la entrada
   republicada.

En esa secuencia no se nombró ninguna clave de caché. Una escritura del ORM
dentro de un `DB::transaction` avanza sus generaciones dentro de esa misma
transacción, así que una escritura revertida no avanza nada en absoluto.

## Qué tan estrecha es la invalidación

Esto es lo más importante que hay que saber antes de dimensionar una ruta
cacheada.

Una lectura puntual por clave primaria que devuelve una fila registra la
identidad de esa **fila** y la identidad de **escritura sin clave** de la
tabla, no la tabla misma. `Model::find`, `Model::find_or_fail`, y
`Model::find_many` observan cada uno una identidad de registro por fila
hidratada y una identidad de escritura sin clave junto a ellas, así que una
escritura a nivel de fila en otro punto de la tabla deja la entrada
vigente, mientras que un `update_all` o `delete_all` masivo, una escritura
por `DB::table(..)`, o una sentencia en bruto sobre la tabla la sigue
alcanzando. Una lectura puntual que no devuelve fila observa la tabla en su
lugar, porque insertar la fila que falta es lo que cambiaría la respuesta.

Toda otra lectura tiene granularidad de tabla: `Model::all`, cada terminal
de `Builder`, y cada carga de relación registran la tabla entera, así que
cualquier escritura en esa tabla invalida toda entrada cacheada que haya
leído de ella. Eso es seguro (solo puede invalidar de más, nunca de menos)
y está medido en lugar de supuesto. La carga de trabajo de tormenta de
invalidación de `framework/benches/render_cache_workloads.rs` publica 64
claves sobre 12 identidades de registro, provoca 1.000 escrituras, y
registra la propagación que observó en
`crates/suprnova-live/benchmarks/render-cache-workloads-v1.json` (abreviado;
el objeto registrado lleva además los campos de ráfaga, barrido, acierto,
reconstrucción, sentencia y latencia):

```json
"invalidation_storm": {
  "keys": 64,
  "identities": 12,
  "writes": 1000,
  "every_write_invalidates_every_key": false,
  "point_read_invalidation_ratio": 0.09375,
  "rebuilds_per_write": 1.28,
  "final_bodies_coherent": true
}
```

`every_write_invalidates_every_key` es `false` porque una lectura puntual ya
no depende de toda su tabla; `point_read_invalidation_ratio` es el número
que la reemplazó como lo que vale la pena vigilar.

Diseña contando con ello. Una ruta cacheada respaldada por una tabla en la
que tu aplicación escribe constantemente reconstruirá constantemente, diga
lo que diga su ventana de frescura. Una ruta cacheada respaldada por una
tabla que cambia cuando un editor publica algo se quedará quieta durante
horas. Si necesitas una granularidad más fina que la tabla, la respuesta
honesta hoy es que no la tienes.

## Lo que el framework no puede ver

Una dependencia que no se puede nombrar no se puede invalidar, y el
framework es deliberado sobre cuáles de esas se niega a almacenar y cuáles
deja pasar.

**Rechazadas de plano.** El SQL en bruto a través de `DB::select`,
`DB::select_one`, `DB::scalar` o `DB::select_on` no puede nombrar las
tablas que leyó su sentencia, así que el render se marca como no observable
y nunca se almacena. La respuesta se sigue sirviendo, correctamente, cada
vez.
Las propias comprobaciones de rol y permiso de RBAC del framework nombran
las cinco tablas que leen - `roles`, `permissions`, `role_permissions`,
`model_roles` y `model_permissions` - así que una ruta cacheada que evalúa
una se observa con precisión y se cachea con normalidad.
Las lecturas a través de `DB::table(..)` conocen su tabla y se cachean con
normalidad.

**Invisibles, y responsabilidad tuya.** Una cabecera de petición leída a
través de `Request::header`, una llamada a `Config::get` y un global scope
de Eloquent que filtra una consulta a partir de su propio estado por
petición cambian todos ellos lo que produce un render sin que el recolector
vea nada. Declara la dimensión de varianza correspondiente en una ruta así;
nada aquí puede detectar la omisión por ti.

**Indicadores de característica.** Una lectura de un indicador que la tabla
`features` contiene - en cualquier clave de scope, incluido el valor por
defecto global - observa la generación propia de ese indicador.
`DatabaseEvaluator::set_flag` la avanza después de que el nuevo valor es
visible para los lectores, y `DatabaseEvaluator::reload()` la avanza para
cada indicador cuyas reglas almacenadas hayan cambiado, y le dice al
evaluador cacheado cuáles fueron esos. Un indicador que la tabla no
contiene no registra nada: ese render dependía del valor por defecto
compilado en `is_enabled!`, no de estado almacenado.

**El lado de escritura.** Todo proceso cuya configuración habilita
RenderCache y cuya base de datos tiene la migración de RenderCache avanza
generaciones, así que una escritura hecha por un worker de cola, una tarea
programada o un comando de consola invalida exactamente lo mismo que la
misma escritura invalida en el servidor, y
`RenderCache::bump_permission_version()` funciona desde cualquiera de
ellos. Un proceso con `RENDER_CACHE_ENABLED=false`, o uno cuya base de
datos no tiene la migración, no avanza nada y no emite ningún SQL de
RenderCache. Consulta
[RenderCache Operaciones](render-cache-operations.md).

## Lo que cuesta un acierto

La comprobación de coherencia es lo que convierte «tenemos bytes» en «estos
bytes están vigentes», y es el único trabajo que hace un acierto.

Un acierto no ejecuta **ningún handler, ninguna consulta del ORM, ninguna
plantilla y ningún serializador**, y no copia bytes de cuerpo: los bytes
que el servidor escribe en el socket son los bytes que tiene el store,
demostrado por dirección y no por valor en
`framework/tests/render_cache/bypass.rs`. Lo que queda es la lectura de
base de datos que demuestra la vigencia, y con cuánta frecuencia pagas por
ella es el `CoherenceMode` de la política:

| Modo | Sentencias SQL por acierto en caliente | En qué confía |
|---|---|---|
| `Authority` (por defecto) | exactamente 1 | el libro mayor, releído en cada acierto |
| `Lease { max_age_ms }` | 0 | un lease de validación concedido localmente, hasta que expira |

`an_authority_mode_hit_issues_exactly_one_statement` mantiene el modo de
autoridad en un viaje de ida y vuelta: las generaciones observadas y el
epoch de autoridad se leen juntos en un solo `UNION ALL`, no como dos
lecturas. `a_lease_mode_hit_runs_nothing_and_issues_no_statement` mantiene
el modo de lease en cero, porque el epoch bajo el que se derivó la clave se
arrienda junto con las generaciones en lugar de leerse por petición.

El epoch en sí se lee una vez por proceso, no una vez por petición.
`the_epoch_is_read_once_at_first_use` mide el primer fallo de un runtime
recién arrancado contra otro por lo demás idéntico y encuentra que el
primero paga exactamente una sentencia más: la única lectura de autoridad
que llena el lease de epoch. Toda petición posterior no paga nada por él.

## Cuando el epoch se mueve

`render-cache:epoch-advance` es la invalidación de emergencia, y el epoch
está incorporado en toda clave de búsqueda, así que lo que ocurre a
continuación depende de dónde estés parado:

- **En el nodo que ejecutó el comando**, la siguiente petición ya ve el
  nuevo epoch. L0 se vacía de plano en ese mismo instante, y la caché queda
  invalidada de inmediato.
- **En otro nodo**, una ruta en modo `Authority` se entera en su siguiente
  acierto. Una ruta en modo `Lease` se entera en su siguiente relectura de
  autoridad, que es como mucho `max_age_ms` después.

Una ruta con una ventana obsoleta-servible sirve una vez la entrada movida
bajo `Warning` mientras la reconstrucción se ejecuta por detrás de la
petición; una ruta sin ella reconstruye en primer plano y quien pidió
espera. Esa diferencia es toda la razón para declarar una ventana
obsoleta-servible, y aplica a cualquier movimiento, no solo a un avance de
epoch.

Tres pruebas de `framework/tests/render_cache/middleware.rs` sujetan esos
caminos por su nombre:
`an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`,
`an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`,
y
`an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.

## Una reconstrucción por clave: singleflight y peticiones en espera

Cuando una entrada falta o ya no está vigente, las peticiones que llegan
por ella no renderizan todas. Se admiten a través de un **coordinador de
reconstrucción**, que elige exactamente una de ellas:

- El **líder** es la única petición que renderiza y puede publicar.
  Mantiene un lease sobre esa clave durante lo que dure su render.
- Las **peticiones en espera** son las que llegan por la misma clave
  mientras el líder lo mantiene. Esperan en proceso, y cuando el líder
  libera, vuelven a evaluar lo que hay almacenado ahora y sirven eso. Una
  petición en espera nunca se fía de la espera: si el ciclo del líder no
  logró publicar, o publicó algo que la propia comprobación de frescura de
  la petición en espera encuentra muerto, esa petición también renderiza,
  en lugar de servir lo que encontró.
  `a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh`, en
  `framework/tests/render_cache/middleware.rs`, es esa regla.
- Una petición que llega cuando ya hay `RENDER_CACHE_MAX_WAITERS` (128 por
  defecto) esperando **elude la caché**: renderiza y no publica nada, en
  lugar de hacer crecer una cola sin cota.

`concurrent_misses_render_once_and_waiters_reuse_the_publication` demuestra
el caso ordinario de punta a punta (dos fallos concurrentes, un render,
cuerpos idénticos), y `one_leader_per_key_and_fence_with_bounded_waiters`,
en `crates/suprnova-live/tests/render_cache_singleflight.rs`, demuestra la
cota directamente contra el coordinador: pasado su límite de espera, la
admisión responde `Bypass`.

Dos publicaciones para una clave nunca pueden aceptarse ambas, decidiera lo
que decidiera el coordinador. Un líder acuña un token de publicación bajo
su lease, y el store compara esa valla antes de escribir: un epoch más
antiguo, o un epoch igual con un token menor, pierde. Eso es lo que hace
seguro aceptar un *renderizado* duplicado mientras que una *publicación*
duplicada no lo es, y es la razón de que no haya espera alguna entre nodos:
una clave que otro nodo está reconstruyendo aquí se elude. Consulta
[RenderCache Despliegue](render-cache-deployment.md).

## Servir algo mientras se reconstruye

Los cuatro estados de frescura, las bandas que fija `FreshnessPolicy` y el
`Warning` y el `Age` que lleva una respuesta obsoleta están definidos en
[RenderCache Representaciones](render-cache-representations.md). Lo que
importa aquí es que un movimiento de generación mete una entrada en esas
bandas antes de tiempo: una entrada movida se evalúa con una antigüedad
efectiva de **al menos** su intervalo fresco, sea cual sea su antigüedad
real. Su antigüedad real sigue decidiendo en qué banda cae:

- Antigüedad real por debajo de `fresh_ms + stale_servable_ms`, en una ruta
  que declara una ventana obsoleta-servible: obsoleta-servible. La copia
  almacenada se sirve una vez bajo `Warning` y la reconstrucción se ejecuta
  por detrás de la petición. Ese es el paso 4 de la prueba de escritura de
  más arriba, sobre una entrada cuyos cinco minutos frescos apenas habían
  empezado.
- Antigüedad real más allá de eso, pero todavía no en el borde muerto:
  obsoleta-ante-error. La petición espera una reconstrucción en primer
  plano y ve la copia almacenada solo si esa reconstrucción falla.
- En una ruta sin ventana obsoleta-servible alguna, y en toda ruta
  `PrivateCached` (cuyo borde muerto *es* su borde fresco), un movimiento
  es Muerta: la petición reconstruye en primer plano y espera.

`stale_service_is_marked_and_rebuilt_in_the_background` muestra el mismo
traspaso provocado por el reloj en lugar de por una escritura: pasados los
300.000 milisegundos frescos de `/live/todos` y dentro de sus 60.000
obsoletos-servibles, al visitante se le entrega la copia que hay a mano
bajo `Warning: 110 - "Response is Stale"` y `Age: 300`, se programa
exactamente una reconstrucción, y esa reconstrucción se ejecuta de verdad.

El repliegue obsoleto-ante-error cubre la petición que lidera una
reconstrucción **y** a una petición en espera detrás de un líder cuya
reconstrucción falló. Ambas se responden igual: los bytes obsoletos bajo
`Warning`, en lugar del fallo. `framework/tests/render_cache/races.rs`
demuestra cada rama por separado
(`a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on`
y `a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it`),
y la segunda por reversión: quitar el repliegue de la rama que espera
convierte sus aseveraciones finales de `200` en `500`.

Las rutas cosidas son la excepción, y es una excepción deliberada. Una
entrada `Composite` nunca la sirve el repliegue obsoleto-ante-error y nunca
dispara una reconstrucción en segundo plano: servir un shell almacenado
después de una reconstrucción fallida respondería a una petición que la
propia cadena de autorización de la ruta nunca llegó a controlar, y una
reconstrucción en segundo plano no lleva nada del estado de autorización de
la petición, así que su shell sería lo que la página renderiza para nadie.
En una ruta cosida, lo que ve el cliente es el propio desenlace de la
reconstrucción fallida.

## Las rutas cacheadas son rutas de lectura

El render del líder se ejecuta dentro de una transacción de base de datos,
abierta en `REPEATABLE READ` en PostgreSQL y MySQL, de modo que las
generaciones que registra y los datos que leyó comparten una sola
instantánea. De ahí se siguen dos consecuencias.

El handler de una ruta cacheada que **escribe** compite con escritores
concurrentes por las mismas filas y, en PostgreSQL, un handler que
actualiza una fila que otra transacción cambió después de que empezara el
render ve un fallo de serialización. Diseña las rutas cacheadas como rutas
de lectura.

Una escritura hecha fuera de cualquier transacción (`model.save()` por su
cuenta) confirma su fila primero y avanza sus generaciones en una
transacción inmediatamente posterior. El momento entre ambas es «datos
nuevos, generación antigua»: cuesta una reconstrucción extra y nunca sirve
contenido obsoleto.

Por último, después de que el render termina, las dependencias observadas y
el epoch se vuelven a leer, fuera de la propia vista transaccional del
render. Cualquier cosa que se moviera durante el render descarta la
candidata en lugar de publicarla. Por eso una escritura que aterriza a
mitad de render cuesta una reconstrucción en lugar de una página
equivocada.

### Por qué Suprnova diverge

La caché de Laravel es un store clave-valor y sus paquetes de cacheo de
respuestas están construidos encima de él, así que la invalidación es algo
que escribes tú. Llamas a `Cache::forget`, o etiquetas entradas y vacías
una etiqueta, o registras un observador de modelo que limpia las claves que
crees que ese modelo alimenta. Cada una de esas cosas es una
correspondencia que mantienes a mano, y el modo de fallo es silencioso: la
página que nadie se acordó de olvidar sigue sirviéndose hasta que se agota
su TTL.

Suprnova invierte la dirección. El render registra lo que leyó, la
escritura avanza lo que cambió, y ambos se encuentran en un libro mayor de
base de datos en lugar de en tu cabeza. No hay ninguna llamada a `forget`
que olvidar. El precio es que la dependencia registrada es una tabla y no
una fila, de modo que una tabla ajetreada reconstruye a menudo lo que
depende de ella, y que las lecturas de SQL en bruto se rechazan para el
almacenamiento en lugar de cachearse con una dependencia que nadie puede
nombrar. Ambas cosas son visibles y están medidas (la propagación en la
carga de tormenta registrada en el repositorio, el rechazo en tu propia
cabecera `Age` ausente) y no una página obsoleta de la que te enteras por
un cliente.

## Siguiente

- [RenderCache Despliegue](render-cache-deployment.md) - perfiles,
  proveedores y la migración que hace duradera la verdad de las
  generaciones
- [RenderCache Representaciones](render-cache-representations.md) - qué se
  almacena en realidad, y bajo qué clave
- [Base de datos](database.md) - transacciones y aislamiento, dentro de los
  cuales se ejecutan los renders cacheados
