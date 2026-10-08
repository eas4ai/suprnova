# RenderCache Representaciones

Una ruta cacheada no almacena «una página». Almacena una
**representación**: una respuesta concreta, bajo una clave de búsqueda, en
una o más capas de almacenamiento, con suficientes metadatos a su lado para
responder a una petición condicional y para demostrar después que sigue
vigente. Dos visitantes obtienen los mismos bytes almacenados solo cuando
la clave que derivan es la misma clave, y la clave se deriva de lo que
declaró la ruta, nunca de lo que el handler resultó hacer.

Este capítulo trata de esa cosa almacenada. Qué formas puede adoptar una
representación (`Complete` y `Composite`), qué entra en su clave, en qué
capas se escribe, el `ETag`, el `Cache-Control`, el `Vary`, el `Age` y el
`Warning` que lleva un acierto servido, los cuatro estados de frescura en
los que puede estar, cómo responde a `If-None-Match` y a `HEAD`, y qué
almacenan realmente `PrivateCached` y `PublicShellStitched`. *Por qué* una
representación abandona la banda fresca (una escritura, un avance de epoch)
es el tema del próximo capítulo; aquí basta con que las bandas existan y con
que una representación esté en una de ellas. Todo ejemplo de más abajo es una
ruta de la aplicación de dogfood de este repositorio
(`app/src/live/mod.rs`) y está demostrado por una prueba con nombre en
`app/tests/live_render_cache.rs`.

## Dos formas de entrada

Una entrada almacenada es de uno de dos tipos.

- **`Complete`** es una respuesta terminada: un estado, un conjunto de
  cabeceras reproducibles y un búfer de cuerpo. Servirla no copia nada y no
  ejecuta nada. Toda ruta `PublicShared` y `PrivateCached` almacena esta
  forma.
- **`Composite`** es un **shell** compartido con huecos tipados recortados
  en él, más un grafo de segmentos que dice qué vuelve a cada hueco. Solo
  `RepresentationClass::PublicShellStitched` almacena esta forma, y solo un
  documento Live produce una.

La clase que declaras en la política decide qué forma es siquiera
alcanzable. `/live/public` y `/live/todos` declaran ambas `PublicShared`;
`the_database_profile_serves_a_hit_through_the_sql_stores` vuelve a leer
del store la entrada publicada de `/live/todos` y asevera que es una de
tipo `EntryKind::Complete`, y
`the_public_document_is_a_hit_whose_seed_still_promotes` lee la de
`/live/public` a través de `RenderCache::inspect_route_for_test` y asevera
la clase bajo la que se almacenó. Eso importa, porque «se almacenó» y «se
rechazó en silencio» producen la misma respuesta: la afirmación hay que
hacerla contra la entrada, no contra lo que ve el visitante.

## La clave de búsqueda

La clave que deriva una petición se construye a partir del patrón de ruta,
sus parámetros de path, los parámetros de query que declaró la política, el
valor resuelto de cada dimensión de varianza declarada, el id de build de
la aplicación (`APP_BUILD_ID`) y el epoch de autoridad actual. Nada más. Un
parámetro de query que llega en la petición pero al que `QueryPolicy::declared`
no da nombre evita la caché para esa petición en lugar de caer
silenciosamente de la clave, porque descartarlo serviría la página
equivocada a quien la envió.

La clave es texto que un operador puede tener en la mano:
`RenderCache::key_for_route_for_test`, en
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch`,
asevera que empieza por `rk1.`, y `render-cache:inspect` toma exactamente
ese texto.

Como el epoch forma parte de la clave, un avance de epoch no tiene que
encontrar ni borrar nada. Toda entrada previamente almacenada simplemente
deja de ser alcanzable por búsqueda ordinaria en la siguiente petición. Ese
es el mecanismo en el que se apoya la invalidación de emergencia del
capítulo [Operaciones](render-cache-operations.md).

## En qué capas escribe una política

Hay dos capas de almacenamiento. **L0** es memoria en proceso, acotada por
`RENDER_CACHE_L0_ENTRIES` y `RENDER_CACHE_L0_BYTES`. **L1** es lo que el
perfil de despliegue configure - un directorio de archivos, una tabla de
base de datos o Redis - y la comparte todo proceso que apunte a ella.

El constructor de políticas almacena **solo en L0** salvo que digas otra
cosa: `StorageLayers::l0_only()` es el valor por defecto. Una ruta que vale
la pena poner en el nivel compartido lo declara:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

Esa es la declaración de `/live/todos` en `app/src/live/mod.rs`. Es el
único documento de esa aplicación cuyos bytes puede compartir todo nodo,
así que es el único que declara `l0_and_l1()`. Bajo el perfil embebido,
donde L1 está deshabilitada salvo que `RENDER_CACHE_L1_DIR` nombre un
directorio, declarar la capa no cambia nada; bajo el perfil de base de
datos la entrada aterriza en `suprnova_render_entries` y un segundo proceso
la encuentra ahí.

`the_database_profile_serves_a_hit_through_the_sql_stores` es la prueba.
Arranca la aplicación sobre los proveedores del perfil de base de datos,
lee la entrada publicada directamente de L1 bajo la mismísima clave que
derivó el middleware, luego vacía L0 y vuelve a preguntar - y la segunda
petición se sigue respondiendo sin que se ejecute el handler. Un acierto en
memoria tendría un aspecto idéntico desde el lado del cliente, y por eso la
prueba recurre al store.

Elige las capas por ruta y no globalmente. L1 cuesta un viaje de ida y
vuelta en un fallo que L0 por sí sola no cuesta, y una entrada que solo un
nodo va a pedir alguna vez no merece ponerse donde todo nodo pueda verla.

## Los metadatos que lleva un acierto servido

Cinco campos de respuesta describen una representación servida, y aquí es
donde quedan definidos; los demás capítulos los usan sin volver a
enunciarlos.

| Campo | Qué dice |
|---|---|
| `ETag` | Un validador fuerte sobre exactamente los bytes enviados. Un cliente puede devolverlo como `If-None-Match`. |
| `Cache-Control` | `private` para toda clase por defecto. Una ruta `PublicShared` que establece `SharedCachePolicy::SMaxAge` obtiene además `public` y `s-maxage`, que es la única manera de invitar alguna vez a un proxy compartido a conservar los bytes. Un documento `Composite` con al menos una isla es `private, no-store`, tanto si se ensambló en un acierto como si lo produjo el render que publicó el shell. |
| `Vary` | Derivada de las dimensiones de varianza declaradas que impliquen una cabecera de petición: `Locale` implica `Accept-Language`, `Media` implica `Accept`, `Encoding` implica `Accept-Encoding`. Una dimensión que no implique ninguna no añade nada. Los nombres se emiten ordenados por nombre de cabecera, no en el orden en que declaraste las dimensiones. |
| `Age` | Segundos enteros desde que se publicó la representación. Su presencia es la prueba local más simple de que una respuesta salió del store. |
| `Warning` | `110 - "Response is Stale"`, y solo en una respuesta servida más allá de su intervalo de frescura. |

La correspondencia entre dimensión y cabecera es
`VarianceDimension::vary_header`, en
`crates/suprnova-live/src/render_cache/variance.rs`. Dos pruebas del motor
demuestran las mitades `Locale` y `Encoding` de esa correspondencia y el
valor de cabecera unido:
`a_descriptor_orders_dimensions_and_bounds_values`
(`crates/suprnova-live/tests/render_cache_variance.rs`) asevera que un
descriptor que lleva ambas informa `["Accept-Encoding", "Accept-Language"]`, y
`cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
(`crates/suprnova-live/tests/render_cache_coherence.rs`) asevera que ese
mismo par emite `Accept-Encoding, Accept-Language` y que un descriptor sin
ninguna dimensión que implique cabecera no emite `Vary` en absoluto. Que
`Media` implique `Accept` está documentado a partir del código; ninguna
prueba de aquí lo empareja.

Tres de los valores de respuesta se aseveran contra la aplicación en
ejecución: `the_public_document_is_a_hit_whose_seed_still_promotes` lee
`private, max-age=300` en `/live/public` y exige una cabecera `Age` en la
segunda petición;
`the_private_document_is_cached_per_principal_and_never_crosses` lee
`private, max-age=60` en `/live/me`;
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` lee
`private, no-store` en el panel, tanto en el render que publica su shell como
en el acierto ensamblado posterior, porque ese valor sigue lo que contienen
los bytes y no la ruta de código que los produjo.

## Los cuatro estados de frescura

Todo acierto se resuelve a exactamente uno de cuatro estados antes de que
se sirva nada. `FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)`
los establece. **Las dos ventanas de obsolescencia se miden ambas desde el
final del intervalo fresco, no se apilan una tras otra** - este es el
detalle con el que tropieza la gente:

| Estado | Antigüedad desde la publicación | Qué recibe el visitante |
|---|---|---|
| Fresca | por debajo de `fresh_ms` | los bytes almacenados, sin `Warning` |
| Obsoleta-servible | más allá de `fresh_ms` por menos de `stale_servable_ms` | los bytes almacenados de inmediato, bajo `Warning`, con una reconstrucción acotada lanzada por detrás de la petición |
| Obsoleta-ante-error | más allá de `fresh_ms` por al menos `stale_servable_ms`, y por menos de `stale_on_error_ms` | una reconstrucción en primer plano; los bytes almacenados bajo `Warning` solo si esa reconstrucción falla ella misma |
| Muerta | más allá de `fresh_ms` por la mayor de las dos ventanas o más | nada; la petición renderiza |

`/live/todos` declara `FreshnessPolicy::new(300_000, 60_000, 300_000)`, así
que es fresca durante cinco minutos, obsoleta-servible durante el sexto,
obsoleta-ante-error hasta los diez minutos, y muerta a partir de ahí.

Dos reglas se imponen sobre las bandas. Una representación `PrivateCached`
**nunca** se sirve obsoleta: pasado su intervalo fresco está muerta, y por
eso `/live/me` declara `FreshnessPolicy::new(60_000, 0, 0)` - una banda
obsoleta ahí se leería como una promesa que la caché no cumple. Y un
documento almacenado de semilla pública cuyo plazo de promoción ha vencido
está muerto digan lo que digan sus intervalos, porque una semilla pasada de
plazo no puede volver a promoverse nunca.

`stale_service_is_marked_and_rebuilt_in_the_background` lleva `/live/todos`
al otro lado de la primera frontera sobre un reloj controlado y asevera el
cuerpo servido, `Warning: 110 - "Response is Stale"` y `Age: 300`. Qué es
lo que *provoca* que una representación abandone pronto la banda fresca -
una escritura, un avance de epoch - es el tema de
[RenderCache Generaciones](render-cache-generations.md).

## Peticiones condicionales y HEAD

Un cliente que devuelve un `ETag` servido como `If-None-Match` obtiene un
`304` sin cuerpo, y un `HEAD` obtiene las cabeceras sin cuerpo. Ninguno de
los dos llega a tu handler:

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` asevera
las tres cosas contra la aplicación en ejecución, incluido que el contador
de renders no se mueve a lo largo de las dos últimas.

Una excepción, y es deliberada: una respuesta `Composite` nunca responde
`304`. Cada ensamblaje es una representación distinta - identidades de isla
nuevas, un nonce de bootstrap nuevo donde el documento tiene uno -, así que
un `304` le diría al cliente que empareje el cuerpo que ya tiene con
cabeceras acuñadas para esta petición. El `ETag` de una respuesta
ensamblada sigue siendo fuerte sobre exactamente los bytes que se enviaron;
simplemente no coincide nunca con una petición posterior. El paso 7 de
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` devuelve
directamente un validador servido y asevera un `200` con un `ETag`
distinto.

## Una representación que pertenece a una sola persona

`RepresentationClass::PrivateCached` almacena una representación por cada
visitante con sesión iniciada. Se rechaza en tiempo de construcción salvo
que la política declare además varianza `Principal` o `Tenant`, de modo que
la pareja no puede separarse por accidente:

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

El handler que hay detrás es uno corriente. Resuelve al visitante con
sesión iniciada y renderiza su nombre:

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

No se conecta nada extra para que eso cachee. La ruta lleva el mismo
`AuthMiddleware::redirect_to("/login")` que lleva el panel, así que un
visitante anónimo es redirigido antes de que se ejecute el handler, y el
principal en sí se resuelve dentro del render. Leer de la sesión la
identidad del visitante con sesión iniciada se clasifica como **lectura de
identidad**, no como lectura de sesión, así que el render se estrecha a
`PrivateCached`, la clave lleva material opaco por principal, y ambos
concuerdan.

`the_private_document_is_cached_per_principal_and_never_crosses` inicia
sesión con dos visitantes, acierta dos veces con cada uno sin render, y
asevera que cada cuerpo nombra a su propia persona y no a la otra; un
tercer visitante renderiza, porque no comparte nada con ninguno de los dos.
El `Cache-Control` servido es `private, max-age=60`, así que a ningún proxy
compartido se le ofrecen nunca los bytes. La misma prueba muestra la otra
mitad del trato: el render resuelve su principal a través del proveedor que
lee la tabla `users`, así que sembrar un tercer visitante invalida toda
entrada `/live/me` almacenada, y la siguiente petición de cada una
reconstruye. Eso es la invalidación con granularidad de tabla haciendo
exactamente lo que describe
[Generaciones](render-cache-generations.md).

Dos consecuencias de esa clasificación merecen conocerse antes de que
declares la clase:

- Una petición **anónima** a una ruta `PrivateCached` con varianza
  `Principal` se cachea bajo la clave `Anonymous`. El render no resolvió
  identidad alguna, así que no se observó material de principal, la clave
  dice `Anonymous`, y ambos concuerdan. Un visitante con sesión iniciada
  deriva una clave `Private` que nunca puede alcanzar esa entrada. Esto
  aplica cuando tal petición renderiza de verdad un `200`, cosa que
  `/live/me` nunca hace: su redirección al login responde un `302`, y un
  `302` lo rechaza la elegibilidad antes de que se consulte nada de esto.
  La prueba del framework que sí llega hasta ahí es
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  en `framework/tests/render_cache/middleware.rs`.
- El identificador de un **guard con nombre** es material de principal
  exactamente igual que el del guard por defecto. Leerlo registra una
  lectura de principal y, cuando hay un id, el valor.

Y una regla que no se ha movido: una ruta que lee el principal *sin*
declarar varianza `Principal` se rechaza para el almacenamiento. No hay
forma de asignar clave a una entrada así por visitante, así que nunca se
almacena en lugar de compartirse. Todo otro valor de sesión sigue forzando
`Uncacheable`; consulta la lista de clasificación en
[RenderCache](render-cache.md).

## Un shell con huecos

`RepresentationClass::PublicShellStitched` es para un documento Live cuyo
marco es el mismo para todo el mundo y cuyas islas no lo son. La entrada
almacenada guarda el shell solo. El marcado de ninguna isla ligada a
identidad ni ningún snapshot firmado están nunca dentro de los bytes
almacenados; cada acierto vuelve a montar cada isla para quien esté
preguntando, bajo autoridad derivada para esa petición.

El panel de este repositorio es esa ruta:

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` asevera qué
compra eso y qué cuesta. La entrada almacenada es un
`EntryKind::Composite` con tres slots, uno por cada isla ligada a
identidad. Un segundo principal se responde desde ese shell, y los dos
documentos difieren **solo** en sus etiquetas de isla: la prueba quita de
cada uno las tres etiquetas de isla y compara lo que queda, byte a byte. La
propia redirección al login de la ruta se sigue ejecutando en cada acierto:
un acierto cosido se reenvía por toda la cadena de middleware de la ruta
antes de servir nada, así que un visitante anónimo recibe la redirección,
nunca un documento ensamblado.

A un documento cosido con al menos un slot se le envía
`Cache-Control: private, no-store`, tanto en el render que publica el shell
como en cada ensamblaje posterior. Contiene las islas de un principal bajo
autoridad vuelta a derivar para una petición, y un `max-age` dejaría que un
perfil de navegador compartido se las reprodujera a quien se sentara
después; qué ruta produjo los bytes no cambia lo que hay en ellos. Un
`Composite` sin slots no lleva byte alguno específico de un
principal, solo un nonce por petición, así que conserva el `max-age` privado
de la clase igual que cualquier otra representación privada;
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit`, en
`framework/tests/render_cache/stitch.rs`, lo asevera. En cualquiera de los
dos casos la clase rechaza `SharedCachePolicy::SMaxAge` en tiempo de
construcción de la política, así que a ningún proxy compartido se le ofrecen
nunca los bytes.

Dos límites que conviene conocer: la clase solo tiene sentido en una ruta
cuya cadena termina en el middleware de finalización de Live, así que úsala
con `LiveDocument::render` y con nada más, y una entrada cosida nunca la
sirve el repliegue obsoleto-ante-error y nunca dispara una reconstrucción
en segundo plano. El capítulo
[Generaciones](render-cache-generations.md) dice qué significa lo segundo
en la práctica.

### Por qué Suprnova diverge

Laravel no tiene modelo alguno de representación del lado del servidor. Sus
paquetes de cacheo de respuestas almacenan la salida renderizada de una
ruta bajo una clave que tú mismo compones - típicamente la URL, a veces la
URL más un sufijo escrito a mano para el usuario con sesión iniciada - y la
devuelven en la siguiente petición. Hay una única forma de cosa almacenada,
siempre es un cuerpo terminado, y que dos visitantes la compartan es una
propiedad de la cadena de texto que construiste.

Suprnova hace de la clave una declaración y de la forma una consecuencia.
Tú nombras la clase y las dimensiones de varianza; el framework deriva la
clave, rechaza `PrivateCached` sin una dimensión que particione, compara lo
que el render observó realmente con lo que la clave dijo realmente, y se
niega a almacenar el render cuando ambas cosas discrepan. Y como existe
`PublicShellStitched`, una página que es un 95 por ciento compartida y un 5
por ciento privada no tiene que elegir entre no cachear nada y cachear algo
que no debería: la parte compartida se almacena una vez y la parte privada
se vuelve a renderizar por petición, sin que los bytes privados entren
nunca en el store.

## Siguiente

- [RenderCache Generaciones](render-cache-generations.md) - cómo una
  representación almacenada deja de estar vigente, y qué ocurre después
- [RenderCache](render-cache.md) - declarar políticas y varianza, y las
  razones por las que un render nunca se almacena
- [Live](live.md) - las islas para las que un shell cosido tiene huecos
