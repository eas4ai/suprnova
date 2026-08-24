# Variables de entorno

Esta es la lista auditada de cada variable de entorno que el framework
Suprnova lee en tiempo de ejecución, agrupada por el subsistema que la
consulta. Cada entrada se ha validado contra el código fuente del
framework - los valores por defecto, los tipos y el comportamiento
reflejan lo que el código realmente hace, no lo que el `.env` inicial
suele traer.

La lista también cubre las variables que lee el binario de la CLI
`suprnova` (servidor de desarrollo, worker de SSR), ya que aparecen en
el `.env` inicial y quien lea la documentación las buscará aquí.

Consulta [Configuración](configuration.md) para las reglas de carga
(`.env` → `.env.<entorno>` → env del proceso), los ayudantes `env*`
(`env`, `env_required`, `env_optional`), y el patrón de registro
tipado `Config::*`.

## Convenciones

- **Por defecto** - el valor que usa el framework cuando la variable
  no está establecida. `none` significa que no hay valor por defecto;
  el framework o falla en el arranque, o recae en un valor por
  defecto propio de la feature (p. ej. el driver `Memory`), o trata
  el valor como `None`.
- **Tipo** - el tipo de Rust al que se analiza la variable. Los
  valores `bool` aceptan `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`
  (sin distinguir mayúsculas de minúsculas). Los valores fuera de
  rango o no analizables para los controles tipados del framework se
  acotan (workflow), se registran con `warn!` y luego se usa el valor
  por defecto (`env()` / `env_optional()`, permisivos), o hacen
  fallar el arranque (`try_from_env`, estricto).
- **Obligatoria** - `boot` significa que el framework se niega a
  arrancar sin ella en los entornos indicados. `driver` significa que
  solo es obligatoria cuando se selecciona el driver del que depende
  (p. ej. `MAIL_SES_REGION` es irrelevante salvo que
  `MAIL_DRIVER=ses`). Todo lo demás es opcional.

Donde un `.env` inicial trae una clave que el framework nunca lee
(`MAIL_FROM_ADDRESS`, `FILESYSTEM_DISK`), se indica al final de este
capítulo.

## Aplicación

La familia `APP_*` es la identidad y la raíz criptográfica del
framework. Son las variables que toda app de Suprnova establece; el
resto del archivo se vuelve relevante a medida que activas
subsistemas.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | Nombre de la aplicación. Se usa como emisor de TOTP (2FA), como realm de `WWW-Authenticate` en HTTP Basic, en el branding del asunto del correo, y en campos del registro estructurado. |
| `APP_ENV` | `local` | `String` | Impulsa `Environment::detect()` y la búsqueda de `.env.<sufijo>`. Alias reconocidos (sin distinguir mayúsculas de minúsculas): `local`, `development`/`dev`, `staging`/`stage`/`stg`, `production`/`prod`, `testing`/`test`. Cualquier otro valor se conserva como `Environment::Custom(...)` con su capitalización original. |
| `APP_DEBUG` | según el entorno (ver Obligatoria) | `bool` | Páginas de error detalladas + registros adicionales. Por defecto es `true` en `local`/`development`/`testing` y `false` en todo lo demás (incluidos `staging`, `production`, y cualquier entorno personalizado no reconocido). Un valor explícito siempre gana; un valor no analizable recae en el valor por defecto según el entorno con un `warn!`. La variante estricta `try_from_env` aborta el arranque ante un fallo de análisis. |
| `APP_URL` | `"http://localhost:8765"` (AppConfig) / `"http://localhost"` (fallback de URL) | `String` | URL base para la generación de URLs absolutas, las URLs firmadas y las redirecciones de Inertia. Las barras finales se recortan al leer. |
| `APP_KEY` | ninguno - obligatoria fuera de dev | `String` (base64-url sin padding, 32 bytes) | Clave AES-256-GCM para `Crypt`, sesiones cifradas, cursores de paginación, URLs firmadas y cualquier otra ruta de cifrado en reposo. El arranque **falla cerrado** cuando falta o está malformada fuera de `local`/`development`/`testing`. Genérala con `suprnova key:generate`. |
| `APP_KEY_PREVIOUS` | ninguno | `String` (claves base64 separadas por comas, máx. 8) | Claves anteriores separadas por comas, usadas durante la rotación. `Crypt::decrypt` prueba primero la `APP_KEY` actual, y luego cada entrada en orden. Tope duro de 8 entradas - `crypto::MAX_PREVIOUS_KEYS`. Una entrada a medio rotar que no se puede decodificar aborta el arranque. Consulta [Cifrado](encryption.md#key-rotation). |
| `APP_PREVIOUS_KEYS` | ninguno | `String` (alias de `APP_KEY_PREVIOUS`) | Alias aceptado por compatibilidad con Laravel, para que un `.env` de Laravel colocado en un despliegue de Suprnova siga desencriptando datos heredados sin fallar. Cuando ambas están establecidas con valores distintos, gana `APP_KEY_PREVIOUS` con un `warn!` que expone el duplicado; los valores idénticos se aceptan en silencio. |
| `APP_BASE_PATH` | directorio de trabajo actual | `Path` | Directorio raíz que usa el resolutor de rutas para `config/`, `database/`, `public/`, `storage/`, `resources/`, `lang/`. Útil cuando ejecutas el binario desde un CWD distinto al de la raíz del proyecto (p. ej. una unidad de systemd cuyo `WorkingDirectory=` no apunta al proyecto). Recae en el CWD, y luego en `.` si el CWD no está disponible. |
| `APP_TRUSTED_PROXIES` | ninguno - lista de permitidos vacía | `String` (IPs separadas por comas) | Direcciones de par TCP cuyos encabezados `X-Forwarded-*` / `X-Real-IP` pueden creerse en `Request::ip()` y en los accesores de host / scheme / puerto. **Vacía por defecto, así que los encabezados de proxy se ignoran y el par TCP siempre gana** - lee la nota de abajo antes de desplegar detrás de un proxy. Una entrada no analizable hace fallar el arranque (`try_from_env`). |
| `AUTH_GUARD` | `"web"` | `String` | Nombre del guard por defecto que lee `Auth::*`. Refleja a Laravel - solo el guard por defecto es seleccionable por entorno; los guards con nombre viven en código vía `AuthConfig::guard(name, …)`. |

Dos variables `APP_*` más - `APP_LOCALE` y `APP_FALLBACK_LOCALE` - las
lee el subsistema de localización en lugar de `AppConfig`, así que se
listan más abajo bajo **Localización**.

### Detrás de un proxy inverso, establece `APP_TRUSTED_PROXIES`

Ignorar los encabezados de proxy es el valor por defecto seguro -
`X-Forwarded-For` lo proporciona quien llama, y confiar en él sin
condiciones le permite a cualquiera reclamar cualquier dirección. Pero
en el momento en que un proxy terminador está delante de ti (nginx,
Traefik, un ALB, Cloudflare), el par TCP es *el proxy*, en cada
solicitud, y dejar esto sin establecer no solo pierde la dirección del
cliente:

- **Los límites de velocidad por IP colapsan en un único cubo.** La
  clave por defecto de `ThrottleRequestsMiddleware` es
  `request.ip()`, así que `ThrottleRequestsMiddleware::with(20, 1,
  "login")` deja de significar "20 intentos de login por cliente por
  minuto" y empieza a significar 20 *en total, entre todos*. Eso es a
  la vez más débil (sin presupuesto por atacante) y activamente
  peligroso: un único llamador puede gastar la cuota y bloquear a
  cada usuario legítimo del formulario de login. Consulta [Limitación
  de velocidad](rate-limiting.md).
- `Request::host()`, `scheme()` y `port()` recaen en la conexión en
  lugar de en `X-Forwarded-Host` / `-Proto` / `-Port`, así que las
  URLs absolutas generadas pueden nombrar la dirección y el esquema
  internos en lugar de los públicos.

Lista las direcciones desde las que te alcanzan los saltos del proxy -
no la del cliente:

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

Nada detecta esto por ti: una app detrás de un proxy con la variable
sin establecer se ve saludable, responde correctamente, y limita la
velocidad de todo el mundo en silencio como si fuera un único usuario.

### Matriz de obligatoriedad de `APP_KEY`

| Entorno | `APP_KEY` obligatoria al arrancar |
|---|---|
| `local` | no (genera una clave efímera si falta) |
| `development` | no |
| `testing` | no |
| `staging` | sí - el arranque sale con código distinto de cero y un mensaje de remediación |
| `production` | sí |
| `Custom(...)` | sí - cualquier cosa que no esté en la lista segura se trata como producción para esta comprobación |

## Servidor

El listener HTTP y los límites del cuerpo de la
solicitud.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | Dirección de enlace. Ponla en `0.0.0.0` para exponerte fuera de la interfaz de loopback (p. ej. en contenedores). |
| `SERVER_PORT` | `8765` | `u16` | Puerto de enlace. El análisis permisivo advierte y usa el valor por defecto; la variante estricta `try_from_env` aborta el arranque ante una errata. |
| `SERVER_MAX_BODY_SIZE` | `8388608` (8 MiB) | `usize` (bytes) | Tamaño máximo del cuerpo de la solicitud, global para el proceso. Los overrides por `FormRequest::max_body_bytes` siguen aplicándose en endpoints individuales. El valor configurado se conecta al tope global durante `Server::from_config`. |
| `SERVER_MAX_CONNECTIONS` | sin establecer (ilimitado) | `usize` | Tope de conexiones TCP activas de forma concurrente. Sin establecer significa sin tope. Un valor cero o no analizable recae en un `10000` finito con una advertencia en lugar de volver en silencio a ilimitado - un límite mal escrito sigue siendo una petición de límite. |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64` (segundos) | Plazo para leer la cabecera completa de una solicitud. La mitigación de slowloris. El cero se trata como inválido, no como "deshabilitar", y recae en el valor por defecto. No se aplica a las conexiones de WebSocket/SSE ya establecidas. |
| `SERVER_HEALTH_READINESS_TOKEN` | sin establecer (la preparación es pública) | `String` | Secreto compartido obligatorio para alcanzar `/_suprnova/health/ready` y `/_suprnova/health?db=true`, enviado como `X-Suprnova-Health-Token`. Sin él esas rutas responden 404, indistinguible de cualquier ruta no enrutada; la actividad se mantiene pública. Consulta [Despliegue](deployment.md#health-check). |

## Base de datos

URL de conexión y ajuste del pool de sqlx. `DATABASE_URL` es
obligatoria para cualquier subcomando que toque la base de datos
(`migrate*`, `db:sync`, `db:seed`, `queue:work` con
`QUEUE_DRIVER=database`, `workflow:work`, el store de sesión en BD) y
para `serve` cuando la app tiene migraciones registradas.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `DATABASE_URL` | ninguno - obligatoria si existen migraciones | `String` | URL de conexión. El esquema selecciona el driver: `sqlite://path`, `postgres://...` / `postgresql://...`, `mysql://...`, `mariadb://...`. El framework crea automáticamente el directorio padre para las rutas de SQLite. `serve` se salta la conexión a la base de datos por completo cuando el `Migrator` configurado no tiene migraciones. |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | Tope del pool de sqlx. |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | Piso del pool de sqlx (se mantiene caliente). |
| `DB_CONNECT_TIMEOUT` | `30` (segundos) | `u32` | Cuánto esperará sqlx una conexión inicial antes de fallar con un error. |
| `DB_LOGGING` | `false` | `bool` | Cuando es cierto, sqlx registra cada statement (úsalo con moderación en producción - es ruidoso). |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | Cuando es cierto, una automigración fallida durante el arranque de `serve` se registra pero no aborta. Por defecto es fail-closed: el arranque sale con código distinto de cero en lugar de iniciar contra un esquema parcialmente migrado. Pasa `--no-migrate` para saltarte la automigración por completo. |

## Sesión

Atributos de cookie y vida útil del subsistema de sesión. Nota que
`SESSION_SECURE` es `true` por defecto - segura para producción por
defecto; desactívala solo para desarrollo local por HTTP.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `SESSION_LIFETIME` | `120` (minutos) | `u64` | Vida útil de la sesión en minutos. Se analiza vía `env_optional`; recae en silencio si no es analizable. |
| `SESSION_TOUCH_INTERVAL` | `300` (segundos) | `u64` | Cadencia mínima de persistencia de la expiración deslizante. La aplicación en tiempo de ejecución la acota a la mitad de la vida útil de la sesión. |
| `SESSION_GC_INTERVAL` | `3600` (segundos) | `u64` | Cadencia del recolector supervisado de sesiones expiradas que instala `SessionMiddleware::install`. |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | Nombre de la cookie de sesión. |
| `SESSION_PATH` | `"/"` | `String` | Atributo `Path=` de la cookie. |
| `SESSION_DOMAIN` | sin establecer | `String` | Atributo `Domain=` de la cookie. Déjala sin establecer para cookies solo de host (el valor por defecto más seguro para la mayoría de apps). |
| `SESSION_SECURE` | `true` | `bool` | Atributo `Secure` de la cookie. Por defecto es `true`; ponla en `false` solo en desarrollo local por HTTP. `cookie_http_only` siempre es `true` y no se puede configurar por entorno. |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | Atributo `SameSite`. Acepta `Strict`, `Lax`, `None` (sin distinguir mayúsculas de minúsculas). |
| `SESSION_COOKIE_PREFIX` | sin establecer | `String` (`__Host-` / `__Secure-`) | Prefijo aplicado a los nombres wire de sesión y remember-me. `Config::init` valida el valor y sus restricciones de `SESSION_DOMAIN` / `SESSION_PATH` durante el arranque; las combinaciones inválidas fallan antes de servir. |
| `SESSION_PARTITIONED` | `false` | `bool` | Emite el atributo de cookie `Partitioned` / CHIPS para cookies aisladas de terceros. |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | Cuando es cierto, omite `Max-Age` para que el navegador borre la cookie al cerrarse (semántica de cookie de sesión). |
| `SESSION_CONNECTION` | sin establecer | `String` | Conexión de BD con nombre para el store de sesión. Sin establecer significa la conexión por defecto. |
| `REMEMBER_LIFETIME` | `43200` (30 días, en minutos) | `u64` | Vida útil en minutos de la cookie/token de "recuérdame". |

## Localización

Las tres variables `APP_*` que lee el subsistema de localización.
Todo lo demás sobre él - la cadena de detección, la clave de sesión y
el nombre de cookie que consulta, las marcas de aislamiento Unicode -
es configuración a nivel de código en `LocalizationConfig`, no env.
Consulta [Localización](localization.md).

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String` (BCP-47) | Locale usado cuando la cadena de detección (sesión → cookie → `Accept-Language`) no encuentra nada. También es el locale del que `suprnova generate-types` extrae las claves de mensaje para `lang-keys.ts`. Un valor que no sea un identificador BCP-47 válido hace fallar el arranque en lugar de recaer en silencio en el valor por defecto. |
| `APP_FALLBACK_LOCALE` | `"en"` | `String` (BCP-47) | Locale consultado cuando falta una clave en el catálogo del locale actual. Una clave que falta en ambos se renderiza como la clave misma más un `warn!` de una sola vez; `Lang::try_get` devuelve `Err` en su lugar. Mismo análisis estricto que `APP_LOCALE`. |
| `APP_LOCALE_PARENTS` | ninguno - mapa vacío | `String` (pares `hijo=padre` separados por comas, BCP-47 en cada lado) | Padres de fallback por locale, consultados antes de `APP_FALLBACK_LOCALE`, p. ej. `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`. La cadena de fallback de `Lang` los recorre de forma transitiva, y `FluentTranslator` aplana la cadena de padres configurada de cada locale en su catálogo servido. Un par mal formado, un locale inválido, un hijo nombrado más de una vez, o un ciclo (incluido un locale que se nombra a sí mismo como su propio padre) hace fallar el arranque en lugar de degradarse en tiempo de solicitud. Consulta [Cadenas de fallback](localization.md#fallback-chains). |

Los catálogos en sí son archivos, no env: `lang/<locale>/*.ftl` bajo
`APP_BASE_PATH`. Un directorio `lang/` ausente no es un error - la
app arranca con el catálogo de validación en inglés incrustado en el
framework.

## Caché

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String` (`memory`/`in-memory`/`inmemory`, `redis`) | Selecciona el objetivo del arranque. Memory mantiene todo dentro del proceso; Redis exige `REDIS_URL` y hace fallar el arranque si no se puede alcanzar. Los valores desconocidos hacen fallar el arranque con un error claro. |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL de conexión a Redis (se consulta solo cuando `CACHE_DRIVER=redis`). |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | Prefijo de clave para las entradas de caché (evita colisiones en un Redis compartido). |
| `CACHE_DEFAULT_TTL` | `3600` (segundos) | `u64` | TTL por defecto en segundos. `0` significa "sin expiración". Se aplica a `Cache::put(None)` / `Cache::tags_put(None)`; `Cache::forever` y `Cache::remember_forever` siempre lo omiten. |

## Cola

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String` (`memory`, `redis`, `database`) | Backend de cola activo. Los valores desconocidos registran un `warn!` y recaen en memory. |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL de Redis (obligatoria por driver cuando `QUEUE_DRIVER=redis`). |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | Clave del Redis Stream usado para la dispersión. |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | Nombre del grupo de consumidores. |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | Nombre del consumidor dentro del grupo. Configúralo por worker para workers en paralelo. |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | Cuánto tiempo permanece invisible un job reclamado antes de que otro consumidor pueda volver a reclamarlo. Ajústalo a tu job más lento. |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | Nombre de la tabla para el driver de base de datos. Se valida como identificador SQL - un valor mal formado hace fallar el arranque, no la composición de SQL. Obligatoria por driver cuando `QUEUE_DRIVER=database`; el driver también exige que `DB::init()` se haya ejecutado antes. |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | Tabla en la que escribe el almacén de fallidos. Se vincula automáticamente cuando `QUEUE_DRIVER=database` - `queue:retry` la lee y `Queue::retry_failed` la necesita, así que la tabla forma parte del contrato de ese driver. No la usan `memory` (efímero por construcción) ni `redis` (no hay tabla en la que escribir). A diferencia de `QUEUE_DB_TABLE`, un identificador mal formado aquí **no** hace fallar el arranque: se registra con `error!` y no queda ningún store vinculado, así que los jobs enviados a fallidos se registran por completo en lugar de persistirse. Recuperable a mano, pero no con `queue:retry`. |

## Programación

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | sin establecer | tipo `bool` | Reconoce que una tarea marcada `on_one_server()` elige un líder mediante una caché **por proceso**. Esa elección es tan compartida como la caché que hay detrás, así que en producción `CACHE_DRIVER=memory` más una tarea de un solo servidor es un fallo de arranque contundente que nombra las tareas responsables, en lugar de una degradación silenciosa a "cada réplica la ejecuta". Establece esto solo donde el despliegue de verdad ejecuta un único planificador; en caso contrario, establece `CACHE_DRIVER=redis`. Consulta [Programación de tareas](scheduling.md). |

## Flujo de trabajo

El worker con estado y de larga duración `#[workflow]`. Todos los
valores se acotan a mínimos seguros en lugar de respetarse a ciegas -
un `WORKFLOW_CONCURRENCY=0` dejaría el semáforo del worker aparcado
para siempre, así que el framework advierte y acota en lugar de
aceptar una configuración obviamente rota.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | Máximo de ejecuciones de workflow concurrentes por proceso worker. Acotado a `>= 1`. |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` (ms) | `u64` | Con qué frecuencia sondea el worker en busca de workflows recién vencidos. |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` (segundos) | `u64` | Timeout de recuperación para una fila de workflow reclamada cuyo worker ha muerto. |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | Máximo de intentos por ejecución de workflow antes de marcarla como fallida. Acotado a `>= 1`. |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | Backoff lineal por intento. Acotado a `>= 0` - un backoff negativo programaría reintentos en el pasado y produciría una recuperación en bucle cerrado. |

## Correo

`MAIL_DRIVER` es **`log`** por defecto - el correo saliente se
imprime en el subscriber de tracing configurado en lugar de llegar a
la red. Cámbialo a `memory` en tests, a `file` para previsualizaciones
`.eml` que puedas abrir en un cliente de correo, y a `smtp`/`ses`/etc.
en producción. Las claves/tokens específicos del proveedor solo son
obligatorios cuando se selecciona ese driver; un valor de driver
desconocido registra un `warn!` y recae en `log`.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String` (`log`, `memory`, `file`, `smtp`, `ses`, `sendgrid`, `mailgun`, `postmark`, `resend`) | Selecciona el objetivo del arranque. |
| `MAIL_FROM` | ninguno - obligatoria para las fachadas de flujo de autenticación | `String` | Dirección de origen por defecto para las fachadas de flujo de autenticación (`EmailVerification`, `PasswordReset`, `TwoFactor`). Obligatoria para esas rutas; si falta, falla en el sitio de la llamada en lugar de recaer en silencio en un marcador de posición que rompería DMARC/SPF. |
| `MAIL_FROM_NAME` | sin establecer | `String` | Nombre para mostrar opcional para el `From` del flujo de autenticación (desde la **0.5.9**). Cuando está establecido, el encabezado se renderiza como `Name <MAIL_FROM>`; `MAIL_FROM` se queda como una dirección desnuda. Se lee en el momento del envío, así que también se aplica al correo de flujo de autenticación encolado. |

### File (`MAIL_DRIVER=file`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | Directorio donde se escribe un archivo `.eml` RFC 5322 por envío. Nunca se purga. Las rutas absolutas se usan tal cual; las relativas se anclan en el directorio base de la aplicación (consulta `APP_BASE_PATH`). |
### SMTP (`MAIL_DRIVER=smtp`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | Host de SMTP. |
| `MAIL_SMTP_PORT` | `587` | `u16` | Puerto de SMTP. |
| `MAIL_SMTP_USER` | sin establecer | `String` | Usuario de SMTP. Tanto `MAIL_SMTP_USER` **como** `MAIL_SMTP_PASS` deben estar establecidas para un transporte cifrado; sin ninguna de las dos, la conexión recae por defecto en el modo local sin cifrar. Establecer solo una de las dos advierte en el arranque. |
| `MAIL_SMTP_PASS` | sin establecer | `String` | Contraseña de SMTP. Consulta `MAIL_SMTP_USER` para el comportamiento con credenciales parciales. |
| `MAIL_SMTP_ENCRYPTION` | derivado | `starttls` \| `tls` \| `none` | Cómo se cifra la conexión. Sin establecer, se deriva de las credenciales: `starttls` cuando ambas están establecidas, `none` cuando ninguna lo está. `tls` selecciona TLS implícito (puerto 465). `ssl` y `null` se aceptan como alias compatibles con Laravel. Un valor no reconocido hace fallar el arranque en **todos** los entornos - una errata no debe degradarse a texto en claro. |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | sin establecer | tipo `bool` | Producción se niega a arrancar sobre una conexión SMTP sin cifrar. Establécela en `1`/`true`/`yes`/`on` para reconocer el texto en claro - defendible solo cuando el relay solo es alcanzable por una red privada. |

### Postmark (`MAIL_DRIVER=postmark`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | obligatoria por driver | `String` | Token de servidor de Postmark. |
| `MAIL_POSTMARK_ENDPOINT` | por defecto de Postmark | `String` | Anula el endpoint de la API (regional o servidor mock). |

### Amazon SES (`MAIL_DRIVER=ses`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | obligatoria por driver | `String` | Access key de AWS. |
| `MAIL_SES_SECRET_KEY` | obligatoria por driver | `String` | Secret key de AWS. |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | Región de AWS. |
| `MAIL_SES_ENDPOINT` | por defecto de AWS para la región | `String` | Anula el endpoint de SES (regional o servidor mock). |

### SendGrid (`MAIL_DRIVER=sendgrid`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | obligatoria por driver | `String` | API key de SendGrid. |
| `MAIL_SENDGRID_ENDPOINT` | por defecto de SendGrid | `String` | Anula el endpoint de la API. |

### Mailgun (`MAIL_DRIVER=mailgun`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | obligatoria por driver | `String` | API key de Mailgun. |
| `MAIL_MAILGUN_DOMAIN` | obligatoria por driver | `String` | Dominio de envío de Mailgun. |
| `MAIL_MAILGUN_ENDPOINT` | por defecto de Mailgun | `String` | Anula el endpoint de la API (p. ej. UE frente a EE. UU.). |

### Resend (`MAIL_DRIVER=resend`)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | obligatoria por driver | `String` | API key de Resend. |
| `MAIL_RESEND_ENDPOINT` | por defecto de Resend | `String` | Anula el endpoint de la API. |

## Limitación de velocidad

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String` (`memory`, `redis`) | Selecciona el backend del limitador de velocidad. Fuera de producción, un valor desconocido registra un `warn!` y recae en memory; **en producción, memory - incluso vía un valor desconocido - hace fallar el arranque** salvo que se establezca `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION`. |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | sin establecer | tipo `bool` | Reconoce cubos de límite de velocidad por proceso en producción. Solo es correcto si ejecutas exactamente un proceso: detrás de N réplicas, cada cuota es efectivamente N× y se reinicia en cada despliegue. |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | URL de Redis (obligatoria por driver cuando `RATE_LIMIT_DRIVER=redis`). |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Prefijo de clave en Redis. |

## Hashing

Driver de hashing de contraseñas y parámetros por algoritmo. Los
valores inválidos devuelven un `FrameworkError::param` en el primer
hash, exponiendo la mala configuración de inmediato en lugar de
recaer en silencio en un valor por defecto.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String` (`bcrypt`, `argon`/`argon2i`, `argon2id`) | Algoritmo de hashing activo. Sin distinguir mayúsculas de minúsculas. |
| `HASH_ROUNDS` | `12` | `u32` | Costo de bcrypt (rango `4..=31`). Los valores fuera de rango fallan con un error claro. |
| `HASH_MEMORY` | `65536` (64 MiB, en unidades de KiB) | `u32` | Memoria de Argon2 en KiB. Mínimo `8`. Solo para Argon. |
| `HASH_TIME` | `4` | `u32` | Tiempo / iteraciones de Argon2. Mínimo `1`. Solo para Argon. |
| `HASH_THREADS` | `1` | `u32` | Paralelismo de Argon2 (coincide con OWASP / libsodium). Mínimo `1`. Solo para Argon. |
| `HASH_VERIFY` | `false` | `bool` | Cuando es cierto, `verify()` rechaza los hashes de un algoritmo distinto a `HASH_DRIVER` (devuelve `Ok(false)`). Por defecto `false`, para que los hashes bcrypt heredados sigan verificándose tras un cambio de driver hasta que se roten. |

## Flujos de autenticación

La autenticación de dos factores usa `APP_NAME` (cubierta en
Aplicación) como el string emisor de TOTP - no hay una variable de
entorno `2FA_ISSUER` dedicada. El emisor recae en `"Suprnova"` cuando
`APP_NAME` no está establecida.

## Inertia / Frontend

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String` (`svelte`, `react`, `vue`) | Frontend activo. Sin distinguir mayúsculas de minúsculas. Impulsa `Frontend::detect_from_env()`, el punto de entrada de Vite por defecto, y el orden de búsqueda de extensión de componente de página en tiempo de compilación. Los valores desconocidos o sin establecer recaen en `svelte`. |

## Modo de mantenimiento

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String` (`file`, `cache`) | Selecciona cómo se almacena el estado de `down`/`up`. `file` escribe en la ruta de almacenamiento del framework; `cache` se apoya en el driver de caché configurado (útil cuando muchas instancias de la app deben coordinar el estado de mantenimiento). Cualquier otro valor recae en `file`. |

## Eventos

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | Tope de tareas de oyente en cola concurrentes. Los valores `<= 0` o no analizables recaen en el valor por defecto. Se aplica a `Event::queue` / oyentes en cola; los oyentes síncronos no están sujetos a este límite. |

## Registro de eventos

`LOG_FORMAT` **depende del entorno**: en producción
(`APP_ENV=production`) el valor por defecto es `json` para que sea
amigable con los agregadores de registros; en todo lo demás el valor
por defecto es `pretty` para una salida local/de desarrollo legible
para humanos. Un valor explícito siempre gana.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String` (`error`, `warn`, `info`, `debug`, `trace` - sin distinguir mayúsculas de minúsculas) | Nivel de filtro de tracing-subscriber. |
| `LOG_FORMAT` | según el entorno (`json` en producción, `pretty` en el resto) | `String` (`json`, `pretty`) | Formato de salida de tracing-subscriber. |

## Observabilidad (OpenTelemetry)

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | sin establecer (telemetría deshabilitada) | `String` | Endpoint del recolector de OTLP. Sin establecer (o en blanco), no se instalan exportadores y el framework sigue usando el subscriber estándar de `tracing`. |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | Atributo de recurso `service.name` en cada span / métrica / registro. |
| `OTEL_SERVICE_VERSION` | `CARGO_PKG_VERSION` en tiempo de compilación | `String` | Atributo de recurso `service.version`. |
| `OTEL_SDK_DISABLED` | `false` | `bool` | Interruptor de corte estándar de OTel. Cuando es cierto, no se instalan exportadores sin importar `OTEL_EXPORTER_OTLP_ENDPOINT`. |

## CLI / servidor de desarrollo

Estas las lee el binario de la CLI `suprnova` (servidor de
desarrollo, worker de SSR) en lugar del framework en tiempo de
ejecución - aparecen en el `.env` inicial o las respetan `suprnova
serve` / `suprnova ssr:*`.

| Var | Por defecto | Tipo | Propósito |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | Puerto al que se vincula Vite en `suprnova serve`. El flag de CLI `--frontend-port` lo anula. |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | Runtime bajo el que se lanza el worker de SSR (`suprnova ssr:start`). El flag de CLI `--runtime` lo anula. |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | Ruta al bundle de SSR compilado. El flag de CLI `--bundle` lo anula. |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | URL del worker de SSR para `suprnova ssr:check`. El flag de CLI `--url` lo anula. |

## Subsistemas sin variables de entorno

Unos pocos subsistemas se configuran por completo en código Rust vía
el contenedor o el registro de servicios - tienen **cero** variables
de entorno que el framework lea:

- **Sistema de archivos / almacenamiento.** Los discos se registran
  con `FilesystemRegistry::add_disk(name, driver)` en `bootstrap()`.
  No hay una variable de entorno `FILESYSTEM_DISK` (el nombre aparece
  en algunos archivos `.env` iniciales, pero el framework no la
  consulta - consulta "Variables que el framework no lee" más abajo).
- **Difusión y WebSockets.** Los canales se registran con la macro
  `ws!()` y la configuración de `BroadcastHub` en código. El driver
  en sí se apoya en lo que seleccione el `CACHE_DRIVER` configurado.
- **CORS, CSRF, Idempotencia, Timeout.** Se configuran vía structs de
  builder que se pasan a los constructores de middleware en
  `bootstrap()`. Los valores por defecto son lo bastante conservadores
  como para que una app típica nunca los toque.
- **Magnetar y OAuth.** `MagnetarConfig` se construye en el bootstrap de la aplicación. El iniciador de API lee `PASSKEY_RP_ID` y `PASSKEY_RP_ORIGIN`, pero el framework no lo hace. Los IDs y secretos de proveedores OAuth, las URL de callback, los scopes, los transportes y los valores de política se suministran programáticamente mediante el registro de proveedores de Magnetar. Las aplicaciones pueden obtener esos valores de variables de entorno o de un gestor de secretos.
- **Búsqueda vectorial, Notificaciones, Pagos, Indicadores de
  características.** Cada uno registra drivers concretos vía
  `App::bind` en `bootstrap()`. Elige tu driver en Rust; pásale las
  URLs/claves que necesite como tus propias variables de entorno.

## Variables que el framework no lee

El `.env` inicial con andamiaje lista algunas claves por comodidad de
quien escribe la app a mano, que el framework nunca consulta. Se
documentan aquí para que quien las busque no se quede con la duda:

- `MAIL_FROM_ADDRESS` - un marcador de posición al estilo Laravel que
  el framework nunca consulta. La dirección de origen real que usan
  las fachadas de flujo de autenticación es `MAIL_FROM` (cubierta en
  Correo). Tus propios tipos `Mailable` pueden leerla vía
  `env_optional` si quieres conservar el nombre de Laravel, pero nada
  en `suprnova::*` lo hace. (`MAIL_FROM_NAME` **sí** se lee desde la
  0.5.9 - consulta el capítulo de Correo - así que ya no se lista
  aquí.)
- `FILESYSTEM_DISK` - marcador de posición para el nombre del disco
  por defecto. Establece el valor por defecto en código vía
  `FilesystemRegistry::set_default(name)` en su lugar.

## Cómo se analizan los valores

Una referencia breve para las tres variantes de ayudante de env -
consulta [Configuración](configuration.md#direct-env-access) para el
tratamiento completo:

| Ayudante | Comportamiento si falta | Comportamiento si no es analizable |
|---|---|---|
| `env(key, default)` | devuelve `default` | `warn!` + devuelve `default` |
| `env_required(key)` | **entra en pánico** | **entra en pánico** |
| `env_optional(key)` | devuelve `None` | `warn!` + devuelve `None` |
| `env_strict(key)` (interno, usado por `try_from_env`) | devuelve `Ok(None)` | devuelve `Err(FrameworkError)` - el arranque aborta |

Las variantes estrictas (`AppConfig::try_from_env`,
`ServerConfig::try_from_env`) son las que llama `Config::init`, así
que una errata en `APP_DEBUG=tru` o `SERVER_PORT=80a0` aborta el
arranque con un error estructurado en lugar de recaer en silencio en
el valor por defecto. Las variantes permisivas existen para la
población más amplia de sitios de llamada (incluido `impl Default`)
donde un fallo de análisis no debe entrar en pánico.

## Overrides por entorno

El cargador lee los archivos en este orden, cada uno anulando al
anterior:

1. `.env`
2. `.env.<entorno>` (p. ej. `.env.production`, `.env.staging`,
   `.env.testing`, `.env.<personalizado>` para `APP_ENV=<personalizado>`)
3. Env del proceso

Eso significa que un despliegue de producción en contenedores puede
enviar un `.env.production` mínimo que solo anule las claves que
difieren de `.env` (nombres de driver, URLs, material de claves), y
que el env real del contenedor anule a ambos para los secretos que
nunca deberían aterrizar en un archivo con commit.

Consulta [Configuración](configuration.md#how-env-loading-works)
para el comportamiento exacto del cargador y el seguimiento de
`LOADED_KEYS` que evita que valores de `.env` obsoletos se promuevan
al nivel del "env real del sistema" a través de las recargas.

## Siguiente

- [Configuración](configuration.md) - el registro tipado de
  `Config::*`, los ayudantes `env*`, la detección de entorno
- [Despliegue](deployment.md) - qué establecer en producción
- [Cifrado](encryption.md) - la rotación de `APP_KEY` vía
  `APP_KEY_PREVIOUS`
- [Arranque de la aplicación](bootstrap.md) - dónde se establece el
  orden de arranque impulsado por env
