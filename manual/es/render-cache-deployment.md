# RenderCache Despliegue

Un proceso que cachea para sí mismo no necesita más que memoria. Varios
procesos detrás de un balanceador de carga necesitan ponerse de acuerdo
sobre qué hay almacenado, sobre quién puede reconstruir una entrada y sobre
cuándo algo dejó de ser cierto, y necesitan ponerse de acuerdo sin que
ninguno de ellos pueda convencer a los demás de que un contenido obsoleto
está vigente. RenderCache responde a eso con **perfiles**: un perfil nombra
qué proveedores construye un proceso, y nada más cambia. Las declaraciones
de ruta, las políticas, las claves, el recolector, el flujo del middleware
y el cosido son idénticos en todos los perfiles, y ningún tipo de cara a la
aplicación difiere entre ellos.

Este capítulo es cómo eliges y conectas uno. Los tres perfiles y qué aporta
cada uno, las variables de entorno que la configuración del framework lee
de verdad, la migración que tu aplicación tiene que listar antes de que un
perfil compartido arranque, dónde va la instalación en tu bootstrap, y qué
prometen y qué no prometen los niveles. La aplicación de dogfood de este
repositorio ejecuta el perfil embebido por defecto y
`the_database_profile_serves_a_hit_through_the_sql_stores`, en
`app/tests/live_render_cache.rs`, la arranca sobre el perfil de base de
datos.

## Tres perfiles

| Perfil | Entradas de L1 | Liderazgo de reconstrucción | Registros de instancia de Live |
|---|---|---|---|
| `embedded` (por defecto) | un archivo por clave, o ninguno | en proceso | en proceso |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`, `suprnova_live_promotions` |
| `redis` | un hash de Redis por clave | un hash de Redis por clave, más un contador de tokens | un hash de Redis por registro |

Lo que **no** se mueve entre ellos es la verdad de las generaciones. El
libro mayor de generaciones respaldado por base de datos es la autoridad en
todos los perfiles: sea cual sea el nivel que entregó los bytes, la
vigencia se demuestra contra la base de datos, releyéndola en el acierto
bajo `CoherenceMode::Authority`, o mediante un lease de validación
concedido a partir de una lectura anterior de ella bajo
`CoherenceMode::Lease`. Eso es lo que mantiene a un acelerador como
acelerador: Redis puede perder todo lo que contiene sin que nada obsoleto
quede demostrado como vigente, porque nada de lo que contiene Redis
demuestra vigencia en primer lugar.

Elige según lo que necesites compartir de verdad:

- **`embedded`** para un solo proceso, y para varios procesos a los que les
  vale con guardar cada uno su propia copia. Establece
  `RENDER_CACHE_L1_DIR` y cada proceso gana un nivel de archivo que
  sobrevive a su propio reinicio.
- **`database`** cuando varios nodos deban compartir entradas almacenadas y
  elegir un líder de reconstrucción por clave, y prefieras no añadir otra
  pieza móvil al despliegue.
- **`redis`** cuando la latencia del nivel compartido importe más que su
  durabilidad, con la base de datos sosteniendo por debajo la verdad de las
  generaciones.

## Las variables de entorno

`RenderCacheConfig::from_env` lee estas, en
`framework/src/render_cache/config.rs`:

| Variable | Por defecto | Significado |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | cualquier cosa salvo `false` o `0`; `false` convierte `RenderCache::install` en una operación nula |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database` o `redis`; fija las dos filas de abajo |
| `RENDER_CACHE_L1` | la del perfil | `disabled`, `file`, `database` o `redis` |
| `RENDER_CACHE_COORDINATOR` | el del perfil | `local`, `database` o `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4096 | techo de entradas en proceso |
| `RENDER_CACHE_L0_BYTES` | 128 MiB | techo de bytes en proceso |
| `RENDER_CACHE_L1_DIR` | sin establecer | el directorio del nivel de archivo; bajo `embedded`, establecerla es lo que enciende L1 |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | el directorio entero para el nivel de archivo, una entrada para los niveles de base de datos y de Redis |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, luego `redis://127.0.0.1:6379` | dónde se conectan ambos niveles de caché de Redis |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | el espacio de nombres de claves bajo el que escriben ambos niveles de caché de Redis |
| `RENDER_CACHE_LEASE_MS` | 30.000 | vida del lease de reconstrucción |
| `RENDER_CACHE_MAX_WAITERS` | 128 | techo de peticiones en espera en proceso |
| `RENDER_CACHE_FAILURE` | `open` | `open` sirve la ruta sin cachear ante un fallo de proveedor, `closed` responde `503` |
| `APP_BUILD_ID` | la versión del paquete de la aplicación (ver abajo) | da a cada entrada el espacio de nombres del build que la produjo |

El perfil es un atajo, no un candado. `RENDER_CACHE_L1` y
`RENDER_CACHE_COORDINATOR` sobrescriben cada una su propia mitad, así que
un despliegue que quiera sus entradas en la base de datos pero sus leases
de reconstrucción en proceso dice exactamente eso en lugar de elegir el
perfil entero más cercano.

Una variable con un conjunto cerrado de valores aceptados que se establece
a algo fuera de él hace fallar el arranque con un mensaje que nombra la
variable. El valor rechazado nunca se repite en ese mensaje, porque un
valor de entorno puede llevar un secreto.

**Establece `APP_BUILD_ID` explícitamente, una vez por despliegue.** Se
mezcla en toda clave de búsqueda, así que cambiarla es lo que impide que un
build nuevo sirva entradas que publicó el anterior. Si falta la variable,
`RenderCacheConfig::from_env` recurre a la propia versión del paquete de tu
aplicación: `#[suprnova::main]` registra `CARGO_PKG_VERSION` a partir de la
compilación del propio crate de la aplicación, en el momento en que carga
el entorno, y ese valor registrado es a lo que recurre el valor por
defecto aquí. Solo un binario que nunca expande `#[suprnova::main]` recurre
más lejos todavía, a la versión del propio crate del **framework** -
nombrada así porque de otro modo es fácil confundirla con la de la
aplicación. En cualquier caso el valor solo se mueve cuando alguien sube un
número de versión, y una versión de paquete rara vez cambia por despliegue:
un despliegue que cambia una plantilla, una traducción o un handler sin
subir la versión conserva el mismo id de build y puede servir entradas que
publicó el build anterior. Establécela a algo que cambie cada vez que
publicas: un id de commit o un identificador de versión:

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

Una instalación que nunca lee el entorno establece el mismo valor en código
con `RenderCacheConfig::with_build_id`, que sobrescribe lo que sea que
eligiera `from_env` - un `APP_BUILD_ID` explícito incluido - para una
aplicación que deriva su propio identificador por despliegue de forma
programática.

Sea cual sea el valor que establezcas, se espera que el binario de
producción que lo lee esté construido en la [forma de build de
producción](deployment.md#production-build-shape) de Suprnova: con las
características por defecto apagadas y `testing` reservado solo para
`cargo test`.

El propio libro mayor de instancias de Live se configura aparte, porque es
autoridad de Live y no almacenamiento de la caché: `LIVE_LEDGER_DRIVER`
(`memory`, `database` o `redis`), `LIVE_REDIS_URL` y `LIVE_REDIS_PREFIX`.
Un despliegue puede ejecutar la caché en un nivel y el libro mayor en otro.

## La migración que tu aplicación debe listar

El esquema de RenderCache es propiedad del framework y lo aplica la
aplicación. Tu `Migrator` lo lista, de modo que `suprnova migrate`
aprovisiona las tablas junto a las tuyas:

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

Eso es `app/src/migrations/mod.rs` literalmente, y las dos no son
intercambiables:

- **`Migration`** crea las tres tablas `suprnova_render_*` que contienen la
  verdad duradera de las generaciones: las generaciones actuales, un
  registro de cambios de solo anexado y el epoch de autoridad. Todo perfil
  la necesita, incluido `embedded`, porque la verdad de las generaciones
  nunca se muda a un nivel de caché.
- **`TierMigration`** crea las cuatro tablas que leen el store L1 de base
  de datos y el coordinador de reconstrucción de base de datos. Solo la
  necesita un perfil que las alcance, pero `RenderCache::install` se niega
  a arrancar el perfil de base de datos sin ellas, así que listarla es lo
  que hace de `RENDER_CACHE_PROFILE=database` una elección de configuración
  que tu aplicación puede tomar de verdad.

Una aplicación que establece `RENDER_CACHE_ENABLED=false` no necesita
cargar con ninguna de las dos: la instalación devuelve el router intacto,
no sondea nada, no ensambla ningún runtime, no registra ningún middleware y
deja sin instrumentar el lado de escritura, de modo que nada paga por una
caché que está apagada.

## Instalarlo

`RenderCache::install` es asíncrono, porque sondea las tablas y hace ping a
cada endpoint de Redis distinto que la configuración usaría antes de
ensamblar nada. `Application::try_routes_async` es el gancho que lo aloja.
Esto es `app/src/live/mod.rs`, y la separación en dos funciones merece la
pena copiarse:

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` es la mitad interna síncrona: registra las rutas Live reservadas,
las rutas de documento y todas las políticas de caché, y no instala ningún
middleware. `cmd/main.rs` alcanza `routes_with_render_cache` a través de
`Application::try_routes_async`, y el servidor del escenario de navegador
en `app/examples/live_dogfood_host.rs` lo espera directamente.

La costura de configuración que hay debajo no es decoración. Una prueba que
necesita un perfil distinto o un reloj que pueda mover no tiene otra
entrada, e importa que instale *las mismas* rutas, políticas y ordenación
de middleware que instala el servidor, difiriendo solo en la configuración
que pasó. Los dos arranques de dogfood pasan por ella:
`the_database_profile_serves_a_hit_through_the_sql_stores` pasa una
configuración del perfil de base de datos, y
`stale_service_is_marked_and_rebuilt_in_the_background` pasa una que lleva
un reloj ajustable. Consulta «Probar una ruta cacheada» en
[RenderCache Operaciones](render-cache-operations.md).

Dos reglas de orden, ambas responsabilidad de quien llama:

1. Toda ruta y todo grupo deben incluirse **antes** de `install`, que lee
   lo que se haya registrado hasta ese punto.
2. `install` añade al final de la cadena global de middleware, así que debe
   ejecutarse **después** del middleware de sesión, locale e identidad cuyo
   estado con ámbito de petición lee el middleware de la caché mientras
   deriva una clave de búsqueda.

La instalación falla cerrado, mediante dos sondeos. Comprueba que las
tablas que la configuración alcanzaría existen, y hace ping a cada endpoint
de Redis distinto que la configuración usaría, una vez por endpoint. Que
falle cualquiera de los dos detiene el arranque con una frase accionable
que nombra la migración o la variable que hay que arreglar, de modo que
nunca se sirve nada contra una tabla que falta o un endpoint que no
responde. (El propio libro mayor de instancias de Live se sondea aparte,
por `Server::run`, antes de servir petición alguna.)

## Elegir dónde viven las entradas de una ruta

El perfil decide qué *es* L1; la política decide qué rutas la usan. El
constructor almacena solo en L0 salvo que una ruta declare otra cosa, así
que un nivel compartido se puebla a propósito:

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` demuestra el
viaje completo de punta a punta sobre el perfil de base de datos: la
entrada publicada se vuelve a leer del store SQL bajo la clave que derivó
el middleware, luego se vacía L0 y la siguiente petición se sigue
respondiendo sin render. Consulta
[RenderCache Representaciones](render-cache-representations.md) para
decidir ruta por ruta.

## Una suite de conformidad, todos los proveedores

Todo store responde a la misma suite.
`framework/tests/render_cache/store_conformance.rs` ejecuta los escenarios
de proveedor del motor, escritos únicamente contra el trait `RenderStore`,
sobre la L1 respaldada por archivo, la L1 SQL en SQLite, PostgreSQL y
MySQL, y la L1 de Redis, y el store en proceso responde a la misma suite en
el crate del motor. PostgreSQL, MySQL y Redis se ejecutan mediante pruebas
ignoradas que `scripts/check-postgres.sh`, `scripts/check-mysql.sh` y
`scripts/check-redis.sh` seleccionan por nombre contra servidores reales.
Aquí un proveedor no está «soportado» porque exista; está soportado porque
pasa las mismas palabras que todos los demás.

## Lo que prometen los niveles, y lo que no

- **Sin espera entre nodos.** Una clave que otro nodo ya está
  reconstruyendo se elude: este nodo renderiza y no publica nada. Se acepta
  un cómputo duplicado y acotado entre nodos; dos publicaciones aceptadas
  no, y la propia valla de publicación del store es lo que prohíbe la
  segunda.
- **Un backend perdido es un fallo, nunca una respuesta equivocada.** Una
  expulsión, una expiración o un reinicio de Redis hacen que las entradas
  fallen y que las instancias falten. La comprobación de coherencia contra
  el libro mayor de generaciones de la base de datos se ejecuta en cada
  acierto, sirviera quien sirviera los bytes.
- **Los bytes manipulados son un fallo.** Los bytes de una entrada en una
  fila o en un hash son un marco de códec firmado, así que un valor roto,
  truncado o alterado falla su comprobación de integridad y se trata como
  un fallo en lugar de servirse.
- **Los niveles de base de datos y de Redis no expulsan para hacer sitio.**
  `RENDER_CACHE_L1_BYTES` acota ahí una entrada, no la tabla ni el espacio
  de claves; el crecimiento se acota mediante la retención en su lugar.
  Solo el nivel de archivo acota un directorio entero, porque es el único
  dueño de ese directorio.
- **Los adaptadores de Redis apuntan a una única instancia de Redis 7 o
  posterior.** Redis Cluster se rechaza: los scripts tocan claves que no
  declaran, y leen el reloj del store con `TIME` dentro de un script.
- **MySQL necesita 8.0.19 o posterior** para clasificar con precisión las
  claves duplicadas. MySQL y MariaDB más antiguos informan de una colisión
  que este build no atribuirá a una tabla, así que degrada a un error de
  proveedor no disponible, la dirección segura, y de todos modos nada se
  concede dos veces.
- **Toda decisión de expiración entre nodos se toma sobre el reloj del
  backend**, leído dentro de la operación que actúa sobre él. Un nodo cuyo
  reloj va adelantado no puede ni extender un lease, ni ocultar una entrada
  viva a sus pares, ni declarar transcurrido el registro de un par.

### Por qué Suprnova diverge

Los paquetes de cacheo de respuestas de Laravel heredan el store de caché
que ya configuraste, así que «desplegar la caché entre varios nodos»
significa apuntar `CACHE_STORE` a Redis y confiar en que lo que haya ahí
dentro sea correcto. No hay noción separada de quién puede reconstruir una
entrada, ni valla que impida que dos workers publiquen bytes en conflicto
para la misma clave, ni, lo más determinante, autoridad alguna por debajo
del store. Si Redis tiene una página, la página se sirve; si Redis se
vacía, todo se recalcula. El store *es* la verdad.

Suprnova separa ambas cosas a propósito. El nivel compartido contiene bytes
y nada más; la base de datos contiene la verdad de las generaciones en
todos los perfiles, y es contra ella contra la que se comprueba un acierto.
Por eso perder Redis aquí cuesta latencia en lugar de corrección, por eso
una reconstrucción se arrienda y una publicación se valla en lugar de
correr una carrera, y por eso las mismas declaraciones de ruta se ejecutan
sin cambios desde un `cargo run` en un portátil hasta una flota coordinada
por base de datos. El coste es una migración que tu aplicación tiene que
listar y una lectura de base de datos en la ruta del acierto que una caché
clave-valor simple no paga: una sentencia, o cero bajo un lease de
validación. Consulta
[RenderCache Generaciones](render-cache-generations.md) para ver qué compra
esa lectura.

## Siguiente

- [RenderCache Operaciones](render-cache-operations.md) - los comandos de
  consola, la telemetría, la higiene de disco y qué hacer cuando algo va
  mal
- [Despliegue](deployment.md) - la lista de verificación de producción que
  lo rodea
- [Migraciones](migrations.md) - cómo se aplica la lista del `Migrator` de
  más arriba
