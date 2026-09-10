# RenderCache

RenderCache almacena una copia con seguridad demostrada de la respuesta de
una ruta GET o HEAD y sirve la siguiente petición equivalente a partir de
ella sin ejecutar tu handler en absoluto. Incluyes rutas y grupos de forma
explícita; todo lo demás sigue funcionando exactamente igual que hoy. Una
ruta que nunca incluyes queda intacta. Una ruta que sí incluyes sigue
renderizando y sirviendo correctamente incluso cuando nada en esa petición
concreta resulta ser seguro de cachear - simplemente nunca se almacena, y
puedes averiguar por qué.

Este capítulo cubre cómo habilitar la caché, incluir rutas y grupos,
declarar la varianza, leer las cabeceras de respuesta que añade, las
razones por las que se rechaza un render, el control operativo y en qué se
diferencia de `suprnova::Cache`.

## Los capítulos

Este es el primero de cinco. Léelos en orden la primera vez; después, cada
uno responde una pregunta por su cuenta.

| Capítulo | Responde |
|---|---|
| RenderCache (este) | ¿Cómo lo activo e incluyo una ruta? |
| [Representaciones](render-cache-representations.md) | ¿Qué se almacena en realidad, y bajo qué clave? |
| [Generaciones](render-cache-generations.md) | ¿Cuándo deja de estar vigente una copia almacenada? |
| [Despliegue](render-cache-deployment.md) | ¿Cómo comparten varios nodos una misma caché? |
| [Operaciones](render-cache-operations.md) | ¿Cómo la inspecciono, la pruebo, la mido y la apago? |

## Habilitar la caché

Dos variables de entorno importan para empezar:

- `RENDER_CACHE_ENABLED` - `true` salvo que se establezca en `false` o `0`.
  Con ella deshabilitada, toda petición evita RenderCache por completo; no
  se busca nada y no se almacena nada.
- `RENDER_CACHE_L1_DIR` - sin establecer por defecto, lo que significa que
  no hay nivel en disco. Establécela a un directorio que el proceso pueda
  crear y en el que pueda escribir, y las representaciones almacenadas
  sobreviven a un reinicio del proceso en un segundo nivel respaldado por
  archivo.

Un puñado de otras variables ajustan los valores por defecto:
`RENDER_CACHE_L0_ENTRIES` (4096) y `RENDER_CACHE_L0_BYTES` (128 MiB) acotan
el nivel en proceso; `RENDER_CACHE_L1_BYTES` (1 GiB) acota el nivel de
archivo; `RENDER_CACHE_FAILURE` (`open` por defecto, o `closed`) decide si
un problema del store o de la base de datos sirve la ruta sin cachear o
rechaza la petición; `APP_BUILD_ID` da a cada entrada cacheada el espacio
de nombres del build que la produjo. Establécela explícitamente a algo que
cambie en cada despliegue: su valor por defecto es una versión de crate
incrustada en la compilación, que no cambia. Consulta
[RenderCache Despliegue](render-cache-deployment.md).

`RENDER_CACHE_PROFILE` (`embedded` por defecto, o `database` o `redis`)
elige si el segundo nivel y el coordinador de reconstrucción están en este
proceso o se comparten con todos los demás nodos. Un perfil compartido
necesita además una migración que tu aplicación liste. Ambas cosas son el
tema del capítulo [Despliegue](render-cache-deployment.md), junto con la
tabla completa de variables.

## Incluir una ruta o un grupo

Nada se cachea hasta que tú lo decidas. `Router::try_render_cache` incluye
un patrón de ruta ya registrado; `Router::try_render_cache_group` incluye
toda ruta bajo un prefijo de path. Ambos reciben una política construida
con `RenderCachePolicy::builder`:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)`
establece cuánto tiempo una representación está fresca, y después dos
ventanas medidas desde ese borde de frescura: hasta dónde más allá de él
puede seguir sirviéndose la copia almacenada mientras se ejecuta una
reconstrucción en segundo plano, y hasta dónde más allá de él puede
servirse la copia almacenada si una reconstrucción en primer plano falla
del todo. Las dos ventanas no se acumulan; consulta
[RenderCache Representaciones](render-cache-representations.md).

`RepresentationClass` va de la compartición más amplia a la más
restringida: `PublicShared` (una representación para todo el que coincida
con la varianza declarada), `PublicShellStitched` (un documento Live cuyo
shell compartido se almacena una sola vez y cuyas islas se vuelven a montar
para quien esté preguntando; consulta
[Representaciones](render-cache-representations.md)),
`PrivateCached` (una representación por cada visitante con sesión iniciada
o por cada tenant), y `Uncacheable`.

Un patrón de ruta debe estar ya registrado antes de incluirlo, y debes
terminar de incluir rutas y grupos **antes** de llamar a
`RenderCache::install` (más abajo) - el paso de instalación lee lo que se
haya registrado hasta ese momento.

Una política a nivel de ruta también puede ser un parche que estrecha la
de su grupo contenedor, usando `PolicyPatch` en lugar de una
`RenderCachePolicy` completa: hereda todo lo que declaró el grupo y solo
puede hacerlo más estricto (una ventana de frescura más corta, una clase
más restrictiva), nunca más amplio. Sacar una ruta por completo de un
grupo cacheado es un `PolicyPatch` que fija la clase en `Uncacheable`.

Termina de conectar RenderCache con una línea, después de todo registro de
middleware que establezca el locale, la sesión o la identidad con ámbito
de petición (RenderCache los lee para construir su clave de búsqueda, así
que necesita ejecutarse después de lo que sea que los configure):

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## Declarar la varianza

Por defecto, una representación cacheada solo varía por el patrón de ruta,
los parámetros de path y el build de la aplicación. Cualquier otra cosa de
la que dependa realmente la salida de tu handler debe declararse, mediante
dos mecanismos:

- **Parámetros de query.** `.query(QueryPolicy::declared(["page", "sort"]))`
  nombra los parámetros de query que distinguen representaciones; cualquier
  otro parámetro de query presente en una petición evita la caché para esa
  petición en lugar de ser ignorado silenciosamente.
- **Dimensiones de varianza**, añadidas una a una con `.vary(dimension)`:
  - `VarianceDimension::Locale` particiona por el locale negociado.
  - `VarianceDimension::Host` particiona por el host de la petición, cuando
    tu despliegue hace que más de un host sea relevante.
  - `VarianceDimension::Tenant` particiona por el tenant actual como
    material de clave opaco; una ruta cuyo handler llegue a leer el tenant
    debe declararlo.
  - `VarianceDimension::Principal` particiona por el visitante con sesión
    iniciada como material de clave opaco, vinculado a una versión de
    permisos (ver "Epoch, permisos e inspección" más abajo); una ruta
    `PrivateCached` debe declarar `Principal` o `Tenant` (o ambos), o no
    logra construirse en absoluto.
- **`Media` y `Encoding`**, declarados juntos con su propio conjunto cerrado:
  `.vary_media(NegotiatedPolicy::declared(["text/html", "application/json"], "text/html")?)`
  y
  `.vary_encoding(NegotiatedPolicy::declared(["identity", "gzip"], "identity")?)`.
  El `.vary(VarianceDimension::Media)` desnudo (o `::Encoding`) se rechaza en
  `build`/`apply`: a diferencia de cualquier otra dimensión, estas dos
  negocian contra un conjunto que solo la ruta puede nombrar, así que no hay
  nada por lo que indexar sin él.

  La negociación lee la cabecera `Accept` de la petición (para `Media`) o
  `Accept-Encoding` (para `Encoding`), la compara contra el conjunto
  declarado, y añade la cabecera de petición correspondiente a `Vary`. Está
  ponderada por `q`: el miembro declarado con la mayor calidad gana, y en caso
  de igual calidad se conserva el orden de izquierda a derecha propio de la
  cabecera, de modo que el candidato listado primero gana el empate. Un
  wildcard (`*/*`, `type/*`, un `*` a secas) se compara como un token literal,
  no se expande contra el conjunto, así que prácticamente nunca coincide con
  un valor declarado real. Una cabecera ausente, un valor que no nombra nada
  del conjunto declarado, o una cabecera de la que esto no puede sacar sentido -
  un `q=0`, una calidad fuera de rango o imposible de analizar, sintaxis
  basura - resuelve al valor por defecto declarado en lugar de crear una
  variante o hacer fallar la petición. Dos valores negociados distintos son
  dos claves distintas; el mismo valor negociado, sin importar cómo se haya
  escrito o ponderado en el cable, es siempre la única representación
  almacenada para él.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` y un
`VarianceDimension::Application(name)` personalizado existen en el tipo
pero no tienen resolutor en esta versión: una ruta que declare uno de ellos
evita la caché en toda petición, silenciosamente, en lugar de fallar al
construirse. No los declares todavía.

## Leer las cabeceras de la respuesta

Un acierto servido lleva `ETag` (un validador fuerte que tu cliente puede
devolver como `If-None-Match` para un `304`), `Cache-Control`, `Vary` y
`Age` (segundos enteros desde que se publicó la representación, y la señal
local más rápida de que una respuesta salió del store y no de tu handler).
Una respuesta servida más allá de su intervalo de frescura lleva además
`Warning: 110 - "Response is Stale"`. Cada una de las cinco está definida,
junto con los valores que las rutas de dogfood tienen comprobado que
envían, en
[RenderCache Representaciones](render-cache-representations.md).

## Por qué un render nunca se almacena

Estar incluida no es una garantía. Dos comprobaciones independientes se
ejecutan después de cada render, y cualquiera de las dos puede rechazar el
almacenamiento sin que la petición falle - la respuesta que recibes es
idéntica en ambos casos, simplemente nunca se convierte en una entrada de
caché:

**Elegibilidad** rechaza de plano una respuesta que no sea un `200` simple
a un `GET` o `HEAD`, que transmita su cuerpo en streaming, que fije una
cookie, o que lleve una cabecera hop-by-hop o de trazado. Esto es casi
siempre accidental (una redirección, una página de error, una respuesta
que resulta tocar `Set-Cookie`) más que algo en torno a lo cual necesites
diseñar.

**Clasificación** rechaza según lo que tu handler realmente hizo mientras
se ejecutaba, en términos que reconocerás:

- **Leíste un valor de sesión.** Cualquier lectura de la sesión actual (a
  través de `session()`, `session_mut`, o una cookie de sesión) fuerza el
  render a `Uncacheable`, de forma permanente, sin importar qué varianza
  declare la ruta. Lo único que esto *no* cubre es la identidad propia del
  visitante con sesión iniciada. `Auth::id()` la lee de la sesión cuando
  nada anterior en la petición la resolvió, y esa lectura se clasifica como
  lectura de identidad, no como lectura de sesión - así que un inicio de
  sesión ordinario respaldado por cookie es exactamente para lo que sirve
  una ruta `PrivateCached` que declara varianza `Principal`, y recurrir al
  id del visitante no vuelve la página no cacheable a escondidas. Todo otro
  valor de la sesión sí lo hace. Dos consecuencias que conviene conocer:
  una petición anónima a una ruta así se cachea bajo la clave `Anonymous`,
  porque el render no resolvió identidad alguna, no observó material de
  principal, y la clave lo dice - un visitante con sesión iniciada deriva
  una clave `Private` que nunca alcanza esa entrada; y el identificador
  propio de un guard con nombre es material de principal exactamente igual
  que el del guard por defecto.
- **Leíste una identidad, en una ruta que no declara `Principal`.** Leer
  al usuario con sesión iniciada estrecha la clase a `PrivateCached`; si
  la varianza declarada de la ruta no incluye `Principal`, no hay forma de
  asignar clave a la entrada por visitante, así que se rechaza en lugar de
  compartirse.
- **Tradujiste (o lo hizo tu motor de vistas) sin declarar `Locale`.**
  Cualquier lectura del locale negociado necesita una dimensión `Locale`
  declarada, o el render se rechaza. El shell de documento de toda página
  Inertia lee el locale para fijar `<html lang>`, tenga o no relación el
  idioma con los propios datos de la página - así que una ruta Inertia
  necesita `Locale` declarado para poder cachear alguna vez, incluso una
  sin contenido traducido propio.
- **Comprobaste la autorización.** Una decisión se juzga por lo que leyó su
  propia evaluación. Un gate cuyo cuerpo lee solo el tenant - por ejemplo a
  través de `suprnova::live::current_tenant()` - se clasifica solo bajo
  `Tenant` y cachea en una ruta con clave por `Tenant`. Un gate que lee un
  dato por usuario, o que no lee nada que RenderCache pueda ver, sigue
  necesitando `Principal` declarado: un cuerpo que decidió a partir de su
  argumento `user` sin pasar por un accesor instrumentado es indistinguible
  de uno que decidió a partir de una constante, y la lectura segura de eso
  es la conservadora.
- **Un modelo detrás de la página lleva un global scope que lee estado por
  petición.** Declara de qué depende el scope. Un `GlobalScope` que
  devuelve `ScopeDependency::Constant` no registra nada y no cuesta
  aciertos de caché. El valor por defecto, `ScopeDependency::PerRequest`,
  exige que el `apply` del scope lea ese estado a través de un accesor
  instrumentado - `suprnova::live::current_tenant()`, `Auth::id()`,
  `Lang::locale()`. Un scope por petición cuya evaluación no lee ninguno de
  ellos estrecha el render a `Uncacheable` y se nombra a sí mismo en el
  rechazo, así que un filtro de tenant invisible te cuesta la caché a ti en
  lugar de costarles a tus visitantes las filas de otros.
- **Leíste un valor de configuración secreto, o un contexto de petición no
  declarado.** Ambos fuerzan `Uncacheable`. La dependencia de una
  respuesta de una cabecera de petición ordinaria, o de `Config::get`, es
  completamente invisible para RenderCache - no puede rechazar lo que no
  puede ver, así que declarar la varianza correspondiente depende de ti.
- **Ejecutaste SQL en bruto a través de `DB::select`, `DB::select_one`,
  `DB::scalar`, o `DB::select_on`.** El framework no puede nombrar las
  tablas que lee una sentencia en bruto, así que el render nunca se
  almacena; aun así se sirve. Las lecturas a través de `DB::table(..)`
  conocen su tabla y se cachean con normalidad, y lo mismo ocurre con
  `Auth::user()`, que se resuelve por esa vía.
  Las propias comprobaciones de rol y permiso de RBAC del framework nombran
  las cinco tablas que leen - `roles`, `permissions`, `role_permissions`,
  `model_roles` y `model_permissions` - así que una ruta cacheada que
  evalúa una se observa con precisión y se cachea con normalidad.
- **La escritura la hizo un worker de cola, una tarea programada, o un
  comando de consola.** Ya no hace falta nada especial. Todo proceso cuya
  configuración habilita RenderCache y cuya base de datos tiene la
  migración de RenderCache avanza generaciones, así que tal escritura
  invalida exactamente lo mismo que la misma escritura invalida en el
  servidor, y `RenderCache::bump_permission_version()` funciona desde
  cualquiera de ellos. Un proceso con `RENDER_CACHE_ENABLED=false`, o uno
  cuya base de datos no tiene la migración, no avanza nada y no emite
  ningún SQL de RenderCache.

En PostgreSQL el render se ejecuta en una transacción `REPEATABLE READ` para
que lo que leyó y las generaciones que registró concuerden; el handler de
una ruta cacheada que actualiza una fila que otra transacción cambió después
de que empezara el render ve un fallo de serialización. Diseña las rutas
cacheadas como rutas de lectura. Un handler que sí escribe dentro de la
transacción del render igual avanza generaciones, pero compite con
escritores concurrentes por las mismas filas y puede ver el fallo de
serialización anterior.

Una escritura hecha fuera de cualquier transacción (`model.save()` por su
cuenta) confirma primero y avanza sus generaciones en una transacción
inmediatamente posterior, así que el momento entre ambas es «datos
nuevos, generación antigua»: una reconstrucción extra, nunca contenido
obsoleto.

Nada de esto necesita herramientas especiales para verse en la práctica:
el comando oculto `render-cache:inspect` (más abajo) muestra si siquiera
existe la entrada de una ruta, o simplemente puedes probar dos peticiones
seguidas y comprobar si la segunda lleva una cabecera `Age`.

## Una ruta que cachea

Una página pública de listado sin contenido específico por visitante:

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

registrada e incluida:

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` nunca toca la sesión, el visitante con sesión iniciada, ni el
locale, así que la primera petición renderiza y publica; toda petición
durante los siguientes cinco minutos se sirve desde esa copia almacenada
con una cabecera `Age`, un `304` para un cliente que ya la tiene, y
`Cache-Control: public, max-age=300, s-maxage=300` para cualquier CDN por
delante.

## Una ruta que se rechaza

La misma forma de página, pero el handler lee la sesión para mostrar un
flash:

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

incluida exactamente de la misma manera que arriba. Toda petición sigue
renderizando y sirviendo la página correcta - flash incluido - pero nada
se almacena jamás: la lectura de sesión estrecha la clase a `Uncacheable`
antes de que RenderCache siquiera llegue a la comprobación de
elegibilidad, así que una segunda petición a la misma URL renderiza de
nuevo desde cero en lugar de volver con una cabecera `Age`. La solución,
si esta página está pensada para cachear, es dejar de leer la sesión en
la ruta cacheada (renderiza el flash a partir de un parámetro de query o
de una respuesta pequeña separada en su lugar) - no existe ninguna
declaración de varianza que haga cacheable una lectura de sesión, porque
una lectura de sesión significa que la respuesta depende de algo por lo
que ninguna clave podría particionar con seguridad.

## Epoch, permisos e inspección

- **`RenderCache::bump_permission_version().await?`** - llama a esto cada
  vez que una acción de la aplicación cambie lo que un usuario con sesión
  iniciada tiene permitido hacer (un cambio de rol, una concesión o
  revocación de permiso). Avanza una generación persistida que observa todo
  render con clave por principal. La generación sobrevive a un reinicio, y
  la llamada se une a la transacción en la que se ejecuta el cambio de rol,
  cuando existe una. Sin esta llamada, un usuario cuyos permisos acaban de
  cambiar sigue coincidiendo con lo que se cacheó bajo su anterior conjunto
  de permisos.
- **`RenderCache::advance_epoch()`**, o el comando oculto
  `render-cache:epoch-advance` - una invalidación de emergencia. El epoch
  está incorporado en la propia clave de búsqueda, así que avanzarlo pone
  las entradas almacenadas fuera de alcance sin nada que enumerar y nada
  que borrar. En el proceso que lo ejecuta el efecto es inmediato: descarta
  el lease de epoch de ese proceso y vacía su nivel en proceso en ese mismo
  instante. Otro nodo se pone al día en su siguiente lectura de autoridad,
  y su nivel respaldado por archivo conserva sus archivos antiguos hasta
  que un barrido los recupera - el automático de cada 256.ª publicación, o
  un `RenderCache::sweep()` explícito -, lo cual es higiene de disco y no
  una cuestión de corrección. Recurre a esto cuando algo va mal con el
  contenido cacheado y no puedes esperar a que las entradas individuales
  expiren; con más de un nodo, consulta
  [RenderCache Operaciones](render-cache-operations.md).
- **El comando oculto `render-cache:inspect <key>`** informa de los
  metadatos de una entrada almacenada (nunca de su cuerpo) mediante el
  texto de clave que tus logs de aplicación o tu telemetría pueden
  mostrar, junto con el epoch actual, de modo que puedas saber si lo que
  estás viendo sigue siendo autoridad vigente o ya ha caducado por debajo.
  Busca la clave solo en el nivel en proceso del proceso en ejecución, nunca
  en el compartido, así que en un perfil `database` o `redis` informa de que
  no hay entrada para una clave que este nodo no haya servido él mismo.

## RenderCache frente a `suprnova::Cache`

`suprnova::Cache` es un store clave-valor que llamas explícitamente: tú
eliges la clave, tú eliges qué almacenar, tú eliges cuándo invalidarlo
(`Cache::put`, `Cache::get`, `Cache::remember`, `Cache::forget`). Funciona
para cualquier dato que tu código decida que vale la pena cachear, en
cualquier backend que configures (memoria o Redis).

RenderCache no es un store de propósito general, y nunca lo llamas desde
tu handler. Cachea respuestas HTTP completas, la clave se deriva
automáticamente de la ruta y su varianza declarada, y la invalidación se
basa en generaciones: una escritura de base de datos ordinaria a través
del ORM o del constructor de consultas avanza las generaciones de las que
dependía el render, y la entrada se recalcula la próxima vez que se
solicita en lugar de borrarse a mano; un render que leyó SQL en bruto
nunca llega a almacenarse, así que no hay nada que recalcular. Recurre a
`suprnova::Cache` cuando
tengas un valor específico que quieras calcular una vez y reutilizar;
recurre a RenderCache cuando tengas una ruta completa cuya respuesta es
cara de renderizar y segura de compartir.

### Por qué Suprnova diverge

Laravel no tiene equivalente en el propio framework. El cacheo de
respuestas es un paquete que añades, envuelve la ruta en un middleware que
almacena la respuesta renderizada bajo una clave que tú compones, y todo lo
demás a partir de ahí es cosa tuya: qué rutas son seguras de cachear, qué
hace diferentes a dos visitantes, y cuándo una página almacenada deja de
ser cierta. El framework no sabe que una página se cacheó, así que no puede
decirte cuándo cachearla fue un error.

RenderCache forma parte del framework exactamente por esa razón. Ve
ocurrir el render, así que puede registrar lo que leyó el handler,
compararlo con lo que declaró la ruta, y negarse a almacenar una respuesta
cuya seguridad no puede justificar - en silencio, sin cambiar lo que se le
sirve al visitante. Incluir una ruta es una declaración que el framework
te hace cumplir después, en lugar de una promesa que te haces a ti mismo.
El coste es que algunas rutas que te gustaría cachear se rechazan y tienes
que averiguar por qué; el beneficio es que las que sí se almacenan quedaron
demostradas como seguras de almacenar, una vez, por el proceso que las
renderizó.

## Siguiente

- [RenderCache Representaciones](render-cache-representations.md) - qué se
  almacena en realidad, bajo qué clave y en qué capas
- [RenderCache Generaciones](render-cache-generations.md) - cómo una copia
  almacenada deja de estar vigente
- [Caché](cache.md) - el store clave-valor explícito con el que este
  capítulo contrasta
- [Live](live.md) - los documentos de los que se recorta una representación
  cosida
