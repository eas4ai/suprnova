# Descripción general de despliegue

Una aplicación Suprnova se compila en un único binario independiente que
posee el servidor web, el ejecutor de migraciones, el planificador y el
worker de colas. Desplegar consiste en "copiar el binario, establecer
cuatro variables de entorno, ejecutarlo". Este capítulo explica cuáles son
esas cuatro variables, qué hacen los subcomandos del binario en producción
y cómo el punto de conexión de salud integrado se integra con la sonda de
actividad de una plataforma. Los tutoriales específicos de la plataforma se
encuentran en [Railway](deployment-railway.md),
[Digital Ocean](deployment-digital-ocean.md) y
[Hetzner](deployment-hetzner.md).

## El único binario

Tu aplicación se compila en un binario con una superficie de subcomandos clap:

```bash
./app                       # serve (por defecto) - automigra y luego HTTP
./app serve                 # serve explícito, con automigración
./app serve --no-migrate    # sirve sin ejecutar las migraciones
./app web:run               # alias de serve

./app migrate               # aplica las migraciones pendientes y sale
./app migrate:status        # muestra el estado de las migraciones
./app migrate:rollback [N]  # revierte las últimas N migraciones (1 por defecto)
./app migrate:fresh         # elimina todas las tablas y vuelve a migrar - en producción
                            # esto exige --force Y una confirmación escrita en una
                            # terminal interactiva; ver cli-migrations.md

./app schedule:work         # demonio del planificador - despierta cada minuto
./app schedule:run          # ejecuta las tareas vencidas una vez y sale
./app schedule:list         # imprime todas las tareas registradas
./app queue:work            # demonio del worker de colas
./app workflow:work         # demonio del worker de flujos de trabajo

./app down [--secret …] [--retry …] [--except …] [--message …]
./app up                    # sale del modo mantenimiento
```

Un único binario significa una imagen Docker, un artefacto CI, un despliegue
a verificar. La misma imagen ejecuta el servicio web, el planificador, el
worker de colas y el worker de flujos de trabajo - ejecutas un
subcomando diferente para cada uno.

## Cuatro variables de entorno de producción

Suprnova falla cerrado en el arranque si el entorno de producción está mal
configurado. El conjunto mínimo para desplegar:

| Variable | Qué hace | Modo de fallo |
|---|---|---|
| `APP_ENV` | Selecciona el entorno (`production`, `staging`, etc.). | Por defecto es `local` si no se establece - tu aplicación se ejecuta en modo desarrollo en producción. |
| `APP_KEY` | Clave base64 AES-256 de 32 bytes para `Crypt`, sesiones, cookies y cursores de paginación. | El arranque devuelve un error tipado y sale con código distinto de cero cuando `APP_ENV` no es local/dev/test y `APP_KEY` no existe o es mal formada. |
| `APP_URL` | URL absoluta canónica de tu aplicación (`https://app.example.com`). | Por defecto es `http://localhost:8765`; las URLs firmadas, redirecciones, enlaces de correo y URLs de Inertia absolutas utilizan esta. |
| `DATABASE_URL` | URL de conexión para tu base de datos relacional. | El arranque se niega a iniciar cuando `APP_ENV` es `production` o `staging` y `DATABASE_URL` no se establece - la alternativa SQLite de desarrollo se rechaza explícitamente. |

Genera `APP_KEY` una vez con la CLI:

```bash
suprnova key:generate           # escribe APP_KEY=… en ./.env
suprnova key:generate --show    # imprime la clave para $(…)
```

Para rotación de claves, consulta [Cifrado](encryption.md) -
`APP_KEY_PREVIOUS` (o la compatible con Laravel `APP_PREVIOUS_KEYS`)
toma una lista separada por comas de claves antiguas para alternativa de
solo descifrado.

Más allá de las cuatro variables requeridas, controles de producción comunes:

| Variable | Predeterminado | Notas |
|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | Usa `0.0.0.0` en contenedores. |
| `SERVER_PORT` | `8765` | Coincide con el puerto esperado de tu plataforma. |
| `APP_DEBUG` | derivado del entorno | `false` en producción/staging/entornos personalizados. Establécelo explícitamente si quieres errores explícitos en staging. |
| `SERVER_MAX_BODY_SIZE` | predeterminado por handler | Límite de cuerpo de solicitud en todo el proceso. |
| `SERVER_MAX_CONNECTIONS` | sin establecer (sin límites) | Límite en conexiones TCP activas concurrentes. Ver a continuación. |
| `SERVER_HEALTH_READINESS_TOKEN` | sin establecer (preparación pública) | Secreto compartido requerido para alcanzar la sonda de preparación. Ver [Verificación de salud](#verificación-de-salud). |
| `DB_MAX_CONNECTIONS` | `10` | Tamaño del grupo. |
| `REDIS_URL` | sin establecer | Requerido si has configurado los drivers de caché/cola/sesión de Redis. |

La tabla completa se encuentra en [Variables de entorno](env-vars.md).

## Base de datos recomendada: MariaDB

Suprnova admite SQLite, PostgreSQL, MySQL y MariaDB como backends
relacionales de primera clase. La recomendación es específica del entorno:

- **Desarrollo.** SQLite. El generador de andamiaje escribe
  `DATABASE_URL=sqlite://./database.db` para que `suprnova serve` funcione
  sin necesidad de configuración de base de datos.
- **Producción.** MariaDB. Coloca lo que de otro modo serían tres
  servicios separados (relacional + vector + caché KV) en un único motor,
  con tablas versionadas por el sistema para auditoría si las necesitas.

```bash
# .env.production
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production
```

Utiliza el esquema `mysql://` - el driver MySQL de SeaORM maneja
MariaDB de forma nativa, y el `MariaDbVectorDriver` de Suprnova
(`VECTOR(N)` + HNSW) se conecta directamente para cargas de trabajo de
vectores.

Los otros backends relacionales también son de primera clase:

```bash
# PostgreSQL
DATABASE_URL=postgres://app_user:secret@db.internal:5432/app_production

# MySQL
DATABASE_URL=mysql://app_user:secret@db.internal:3306/app_production

# SQLite (para despliegues diminutos de instancia única)
DATABASE_URL=sqlite:///var/lib/myapp/data.db
```

### Por qué Suprnova diverge

Los valores predeterminados de Laravel impulsan nuevos proyectos hacia
PostgreSQL porque PHP + PostgreSQL es el camino bien trillado. Suprnova
elige la base de datos que proporciona la postura de producción de motor
único más limpia para una aplicación Rust. `VECTOR(N)` de MariaDB (11.7+),
Columnas dinámicas y tablas versionadas por el sistema significan que un
producto pequeño a mediano puede enviar búsqueda, KV y auditoría sin añadir
Redis, OpenSearch o pgvector. PostgreSQL sigue siendo totalmente compatible
(la matriz de pruebas del framework se ejecuta contra los tres backends
relacionales), pero nuestra documentación de despliegue comienza con el motor
que minimiza las partes móviles. Ver [Almacenamiento de vectores](vector.md)
y [Base de datos](database.md) para las superficies específicas del backend.

## Construcción de una imagen de producción

El generador de andamiaje proporciona un generador para un Dockerfile
multietapa:

```bash
suprnova docker:init
```

Esto escribe un `Dockerfile` con tres etapas:

1. **Construcción de frontend** - `node:20-alpine`, ejecuta `npm ci && npm run build`
   en tu aplicación `frontend/` Inertia (Svelte 5, React 19 o Vue 3.5
   según tu opción de andamiaje).
2. **Construcción de backend** - `rust:1.94.0-slim-bookworm`, compila tu crate en
   modo de lanzamiento con almacenamiento en caché de dependencias.
3. **Tiempo de ejecución** - `debian:bookworm-slim`, copia el binario compilado
   y la salida de Vite, se ejecuta como `appuser` sin raíz, expone el puerto 8765
   y ejecuta `CMD ["./app"]` (el servidor auto-migrando).

La rama `main` actual usa SeaORM 2.0, SeaQuery 1.0 y SQLx 0.9. Las aplicaciones que llaman directamente a SeaORM deben importar `ExprTrait` para los métodos de expresión de SeaQuery y usar métodos de conexión `*_raw` explícitos para valores `Statement` preconstruidos. La actualización de dependencias no requiere ninguna migración de datos de la aplicación.

Construye y ejecuta localmente para verificar antes de impulsar:

```bash
docker build -t myapp .

# Con un archivo de entorno
docker run --rm -p 8765:8765 --env-file .env.production myapp

# O con variables explícitas (las cuatro requeridas)
docker run --rm -p 8765:8765 \
  -e APP_ENV=production \
  -e APP_KEY=$APP_KEY \
  -e APP_URL=https://app.example.com \
  -e DATABASE_URL=mysql://user:pass@host:3306/app \
  myapp
```

Nunca confirmes `.env.production` (o cualquier archivo que contenga `APP_KEY` o
`DATABASE_URL`) en tu repositorio. Usa el almacén de secretos de tu plataforma
y lee los valores en tiempo de despliegue.

## Migraciones en el arranque

El comando predeterminado `./app` (y `./app serve` explícito) aplica cualquier
migración pendiente antes de vincularse al socket. Las dos implicaciones prácticas:

- **Seguro con múltiples instancias.** El ejecutor de migraciones de SeaORM toma
  un bloqueo asesor a nivel de base de datos; el pod más lento espera, los otros
  proceden una vez que se hace. No necesitas un paso separado "migrar-luego-desplegar"
  para lanzamientos de rutina.
- **Migración fallida = despliegue fallido.** Si una migración da error, el
  proceso sale con código distinto de cero antes de que el servidor se vincule.
  La sonda de salud de la plataforma (ver a continuación) reporta el pod como
  no saludable y la implementación se detiene. Corrígelo hacia adelante enviando
  una migración correctiva en el próximo lanzamiento.

Para tuberías de CI que quieran limitar el despliegue en una migración exitosa
antes de que cualquier pod acepte tráfico, ejecuta migraciones de una sola vez:

```bash
docker run --rm myapp ./app migrate
# … luego realiza el despliegue real
docker run myapp ./app serve --no-migrate
```

`--no-migrate` omite la fase de auto-migración pero sigue arrancando el servidor
normalmente.

## Workers como servicios separados

Los sistemas de planificador, cola y flujo de trabajo tienen cada uno su propio
subcomando de demonio. En producción, ejecútalos como procesos separados contra
la misma imagen, compartiendo el mismo entorno:

```bash
docker run myapp ./app schedule:work    # una instancia - ver a continuación
docker run myapp ./app queue:work       # escala a N instancias
docker run myapp ./app workflow:work    # escala a N instancias
```

Dos reglas para interiorizar:

- **Ejecuta exactamente un proceso `schedule:work`, o marca tus tareas
  `.on_one_server()`.** Las réplicas del planificador no se coordinan por
  defecto: cada una evalúa el calendario de forma independiente, por lo que
  tres réplicas ejecutan cada tarea vencida tres veces. `replicas: 1` es la
  respuesta simple; `.on_one_server()` elige una réplica por tick contra una
  caché compartida y es lo que deseas si el planificador tiene que tener alta
  disponibilidad. Ver [Programación](scheduling.md#running-on-one-server).
- **Los workers de cola y flujo de trabajo se escalan horizontalmente.**
  Ambos extraen trabajo de un almacén compartido y utilizan tiempos de espera
  de visibilidad o bloqueos a nivel de fila para coordinarse; agregar pods
  añade rendimiento. `./app queue:work --max-jobs N` hace que el worker
  salga después de N trabajos para que un supervisor pueda rotar el proceso -
  útil para despliegues de lanzamiento-en-reinicio.

Ver [Colas](queues.md), [Programación](scheduling.md) y
[Flujos de trabajo](workflows.md) para los detalles por subsistema.

## Detención limpia

Cada proceso Suprnova de larga duración - el servidor y los tres demonios -
se drena en **SIGTERM** así como en SIGINT. SIGTERM es lo que envían
`docker stop`, Coolify, systemd y Kubernetes; SIGINT es lo que envía Ctrl-C.
Ambos toman el mismo camino: dejar de aceptar nuevos trabajos, terminar lo
que está en vuelo dentro de un periodo de gracia acotado, salir con `0`.

Las ventanas de gracia son por subsistema y están acotadas a propósito - un
cliente lento o una tarea larga no deben poder mantener un proceso vivo
indefinidamente:

| Proceso | Espera | Gracia |
|---|---|---|
| `serve` | conexiones HTTP en vuelo | 5s |
| `queue:work` | que el trabajo en vuelo se resuelva | hasta que regresa el trabajo |
| `schedule:work` | tareas `.run_in_background()` | 30s |
| `workflow:work` | pasos de flujo de trabajo en vuelo | hasta que regresan |

**Establece el periodo de gracia de terminación de tu plataforma por encima
de estos.** Docker por defecto a 10 segundos, Kubernetes a 30. Si la ventana
de la plataforma es más corta que lo que tarda el trabajo, envía SIGKILL y
vuelves a perder trabajos en vuelo:

```yaml
# docker compose
services:
  worker:
    command: ["app", "queue:work"]
    stop_grace_period: 60s
```

```yaml
# kubernetes
spec:
  terminationGracePeriodSeconds: 60
```

**Un trabajo eliminado en vuelo no se pierde, pero sí cuesta un intento.** Su
reserva caduca y otro worker lo reclama, cobrando un intento para que un
trabajo que mata sistemáticamente su worker pueda seguir siendo enviado
a fallidos en lugar de circular para siempre. Ver
[Colas](queues.md#what-counts-as-an-attempt).

**PID 1 es una restricción real.** Un punto de entrada de contenedor se
ejecuta como PID 1, y el kernel no aplica disposiciones de señal predeterminadas
a PID 1 - un proceso sin handler SIGTERM no muere en SIGTERM, lo ignora hasta
que la plataforma se rinde y envía SIGKILL. Suprnova instala el handler, así
que `CMD ["app", "queue:work"]` es correcto como está escrito y no se requiere
un shim `tini`.

## Verificación de salud

Suprnova expone tres rutas de salud integradas. El prefijo `_suprnova/` está
reservado para que tus propias rutas nunca puedan colisionar con ellas.

| Ruta | Toca | Usar para |
|---|---|---|
| `/_suprnova/health/live` | nada | Actividad. Responde 200 mientras el proceso pueda servir una solicitud. |
| `/_suprnova/health/ready` | la base de datos | Preparación. 503 cuando una dependencia es inalcanzable. |
| `/_suprnova/health` | nada, o la base de datos con `?db=true` | El punto de conexión original. Se comporta como cualquiera de los anteriores. |

```bash
curl http://localhost:8765/_suprnova/health/live
# 200 {"status":"ok","timestamp":"2026-05-30T12:34:56+00:00"}

curl http://localhost:8765/_suprnova/health/ready
# Saludable: 200 {"status":"ok","timestamp":"…","database":"connected"}
# Degradado: 503 {"status":"degraded","timestamp":"…","database":"error"}
```

`/_suprnova/health` y `/_suprnova/health?db=true` siguen funcionando exactamente
como antes, y nada de lo que ya hayas desplegado necesita cambios - la
[guía Hetzner](deployment-hetzner.md) todavía las nombra para comprobaciones
únicas, al igual que tus propias especificaciones. Las rutas nombradas son más
claras, así que prefierelas en la configuración nueva; las guías
[Railway](deployment-railway.md), [DigitalOcean](deployment-digital-ocean.md)
y [Docker](cli-docker.md) las usan.

### Usa la sonda correcta para la pregunta correcta

Apunta la actividad a `/live` y la preparación a `/ready`. La distinción
importa más de lo que parece: una sonda **actividad** fallida reinicia el pod,
mientras que una sonda **preparación** fallida solo lo saca del equilibrador
de carga. Incorpora una comprobación de base de datos en la actividad y un
problema en la base de datos reinicia cada réplica que tienes - en el momento
exacto en que la base de datos puede menos permitirse un rebaño de reconexiones.

```yaml
livenessProbe:
  httpGet:
    path: /_suprnova/health/live
    port: 8765
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
```

El punto de conexión se cortocircuita antes de la cadena de middleware por lo
que se mantiene responsivo incluso si un middleware se bloquea o el middleware
de ID de solicitud está rechazando tráfico.

### Las respuestas degradadas no llevan detalle del driver

El cuerpo 503 reporta `"database":"error"` y nada más. El mensaje del
driver propio - que nombra hosts, puertos, nombres de base de datos y
esquema y versiones del servidor, y para algunos errores de configuración la
URL de conexión - va al registro en nivel `error!`, donde un operador puede
leerlo y un extraño no puede. En compilaciones de depuración también se
incluye en el cuerpo como `database_error`, por lo que la depuración local
no se ve afectada.

### Cierre de la preparación

La preparación ejecuta un viaje de ida y vuelta a la base de datos para quien
la solicite. Si el punto de conexión es accesible desde internet, establece un
secreto compartido:

```bash
SERVER_HEALTH_READINESS_TOKEN=<a long random string>
```

Las sondas deben entonces enviarlo como encabezado:

```bash
curl -H "X-Suprnova-Health-Token: $SERVER_HEALTH_READINESS_TOKEN" \
  http://localhost:8765/_suprnova/health/ready
```

```yaml
readinessProbe:
  httpGet:
    path: /_suprnova/health/ready
    port: 8765
    httpHeaders:
      - name: X-Suprnova-Health-Token
        value: <the same value>
```

Sin el encabezado, la preparación responde **404** - la misma respuesta que
cualquier ruta que no existe, por lo que el punto de conexión es invisible en
lugar de simplemente cerrado. La actividad se mantiene pública de cualquier
forma, por lo que no tienes que poner el secreto en cada manifiesto para
mantener tu señal de reinicio-en-cuelgue.

Sin establecer es el predeterminado, y la preparación es pública. Eso es
deliberado: las configuraciones que este manual y el generador de andamiaje
generan todos llaman `?db=true` sin un encabezado, y establecer por defecto
como cerrado las rompería.

## Modo de mantenimiento

Para ejecutar una migración destructiva o poner el tráfico en reposo
durante un incidente:

```bash
./app down --secret abc123 \
           --retry 60 \
           --message "Deploying - back in a few minutes" \
           --except /webhooks/stripe

./app up
```

`down` escribe un marcador de mantenimiento que el middleware lee en
cada solicitud. Las solicitudes reciben un 503 (configurable vía
`--status`) con el mensaje indicado, salvo las rutas de `--except` y
cualquier solicitud que incluya el secreto. `up` elimina el marcador.

El secreto es una credencial bearer: a quien visite `/<secret>` se le
emite una cookie de bypass de 12 horas. Tanto la coincidencia de la URL
como la de la cookie son comparaciones en tiempo constante, así que la
temporización de la respuesta no le revela a quien sondea qué longitud
de prefijo acertó. Prefiere `--with-secret`, que acuña uno por ti (16
bytes aleatorios, 32 caracteres hexadecimales) e imprime la URL de
bypass, antes que elegir una cadena memorable para `--secret` - y trátalo
como cualquier otra credencial en tus notas del incidente.

## Escalado

### Web

La escalabilidad horizontal es la historia predeterminada: cada pod ejecuta
`./app`, comparte `DATABASE_URL` y se conecta al mismo Redis (si has
configurado caché/cola/sesión respaldados por Redis). La auto-migración es
segura debido al bloqueo asesor anterior. Las sesiones adhesivas no son
requeridas - el estado de la sesión vive en tu driver de sesión
(base de datos o Redis), no en la memoria del proceso.

### Workers

- **Planificador.** Exactamente una instancia, siempre.
- **Cola.** Escala horizontalmente. Si has dividido el trabajo entre múltiples
  colas nombradas, ejecuta un worker por cola (o pasa filtros de cola
  específicos del driver - ver [Colas](queues.md)).
- **Flujo de trabajo.** Escala horizontalmente; la reclamación a nivel de fila
  y el latido coordinan los workers.

## Límite de conexión (`SERVER_MAX_CONNECTIONS`)

Por defecto, el servidor acepta un número ilimitado de conexiones TCP
concurrentes. En la mayoría de despliegues, un proxy inverso (nginx, Caddy,
Traefik) o el equilibrador de carga de la plataforma proporciona la primera
línea de defensa. Si quieres un tope duro dentro del proceso mismo - para
evitar que un único grupo de cliente mal comportado agote descriptores de
archivo - establece `SERVER_MAX_CONNECTIONS`:

```bash
# .env.production - limita conexiones concurrentes a 1024
SERVER_MAX_CONNECTIONS=1024
```

Cuando se alcanza el límite, el **bucle de aceptación se bloquea** (contrapresión
en el nivel TCP) hasta que se cierre una conexión existente; el apretón de manos
pendiente permanece en el atraso de aceptación del kernel. El permiso se mantiene
durante toda la vida útil de cada conexión y se libera en el momento en que la
conexión termina, por lo que las ranuras se renuevan rápidamente.

Reglas prácticas:

- **Sin establecer (predeterminado = sin límites).** Correcto si tienes un proxy
  inverso aplicando su propio límite de conexión, o si se ejecuta detrás de una
  PaaS que gestiona la concurrencia para ti.
- **Establece a un valor concreto** si el proceso se ejecuta directamente en
  internet o deseas defensa en profundidad independientemente de la configuración
  del proxy. Un punto de partida típico es 2 × tus usuarios concurrentes de pico
  esperados, ajustado hacia arriba para conexiones de larga duración (WebSocket, SSE).
- **Empareja con `LimitNOFILE`** (systemd) o `ulimit -n` para que el límite de
  descriptor de archivo del sistema operativo no se convierta en el tope sorpresa.
  Cada conexión HTTP cuesta un descriptor de archivo; añade tu tamaño de grupo de
  base de datos y algunas docenas para las tareas del sistema operativo.
- **Esto es un tope, no un reemplazo para la limitación de velocidad ascendente.**
  `SERVER_MAX_CONNECTIONS` detiene la acumulación desenfrenada; tu proxy inverso
  o middleware `rate_limit` debería manejar la limitación por cliente o por IP.

Los valores en blanco, no analizables o cero se tratan silenciosamente como no
establecidos para que un error tipográfico no impida que el servidor se inicie.

## Tutoriales específicos de plataforma

La receta anterior se adapta a cada PaaS o VPS moderna. Los próximos tres
capítulos te guían a través de los detalles específicos:

| Plataforma | Estilo | Tutorial |
|---|---|---|
| Railway | PaaS con auto-despliegue desde git | [Desplegar en Railway](deployment-railway.md) |
| Digital Ocean | App Platform (PaaS) o Droplets (VPS) | [Desplegar en Digital Ocean](deployment-digital-ocean.md) |
| Hetzner | VPS con systemd + Caddy | [Desplegar en Hetzner](deployment-hetzner.md) |

## Siguiente

- [Variables de entorno](env-vars.md) - cada variable de entorno que lee el marco
- [Cifrado](encryption.md) - `APP_KEY`, rotación de claves, qué está cifrado
- [Configuración](configuration.md) - secciones de configuración tipadas construidas sobre env
- [Base de datos](database.md) - selección de driver, ajuste de grupos, división de múltiples conexiones
- [Colas](queues.md) - escalado de workers y drivers de colas
