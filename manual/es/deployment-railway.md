# Desplegar en Railway

[Railway](https://railway.app) es un PaaS basado en Git que construye
tu Dockerfile y lo ejecuta sobre infraestructura gestionada. Combínalo
con el Postgres y el Redis gestionados de Railway y tendrás una pila
de producción de Suprnova completa, sin servidores que cuidar. Esta
receta lleva una app recién generada con andamiaje desde `suprnova new`
hasta una URL en producción.

## Prerrequisitos

- Una [cuenta de Railway](https://railway.app)
- Un proyecto Suprnova subido a GitHub, GitLab o Bitbucket
- Un `Dockerfile` y un `.dockerignore` en la raíz del repo, generados
  con:
  ```bash
  suprnova docker:init
  ```
- Una `APP_KEY` generada que puedas pegar en las variables de Railway:
  ```bash
  suprnova key:generate --show
  ```

`suprnova` solo hace falta en local - Railway construye el Dockerfile
por sí mismo. El crate del framework se obtiene desde git como una
dependencia normal de cargo durante la construcción.

## Aprovisiona el proyecto

1. Abre el [panel de Railway](https://railway.app/dashboard), haz clic
   en **New Project** y elige **Deploy from GitHub repo**.
2. Elige el repositorio. Railway detecta el `Dockerfile` e inicia la
   primera construcción automáticamente.
3. Mientras se construye, añade una base de datos: **New** →
   **Database** → **Add PostgreSQL**. Railway expone `DATABASE_URL`
   como variable de referencia del proyecto.
4. Opcionalmente añade Redis de la misma forma (**New** → **Database**
   → **Redis**) si tu app usa el driver de caché, sesión, cola o
   límite de velocidad de Redis. Railway expone la URL de conexión
   como `REDIS_URL`.

## Conecta las variables

Abre el servicio web, ve a **Variables** y añade la configuración de
producción. Usa la sintaxis de referencia `${{ }}` de Railway para
extraer las URLs de los servicios de base de datos, de modo que las
rotaciones no exijan volver a pegarlas.

```env
APP_ENV=production
APP_KEY=<paste the output of `suprnova key:generate --show`>
SERVER_HOST=0.0.0.0
SERVER_PORT=8765
DATABASE_URL=${{ Postgres.DATABASE_URL }}
REDIS_URL=${{ Redis.REDIS_URL }}
```

Algunas cosas que conviene saber:

- **`APP_KEY` es obligatoria en entornos que no son de desarrollo.**
  Suprnova falla cerrado en el arranque cuando `APP_ENV != local|dev|test`
  y `APP_KEY` no existe o es mal formada. El servidor registra un
  mensaje de remediación y sale con código distinto de cero - Railway
  marcará el despliegue como fallido. Genera la clave con
  `suprnova key:generate --show`.
- **`SERVER_HOST=0.0.0.0` es obligatorio.** Railway enruta el tráfico
  a través de la interfaz de red del contenedor; enlazar a
  `127.0.0.1` (el valor predeterminado local) parecerá una conexión
  rechazada.
- **`SERVER_PORT` coincide con `EXPOSE` en el Dockerfile.** El
  Dockerfile generado expone el 8765. Railway lo asigna a una URL
  pública automáticamente.

## Construye y despliega

Railway construye en cada push a la rama conectada. El Dockerfile
generado por `docker:init` hace lo siguiente:

1. **Etapa 1 - Frontend.** Ejecuta `npm ci` y `npm run build` en
   `frontend/`. La salida de Vite queda en `frontend/dist/`.
2. **Etapa 2 - Backend.** Ejecuta `cargo build --release` contra tu
   workspace; las capas de dependencias en caché mantienen rápidas
   las construcciones iterativas.
3. **Etapa 3 - Runtime.** Una imagen `debian:bookworm-slim` con
   `ca-certificates` + `libssl3`, un `appuser` no root y el binario
   `./app` ya compilado. El `CMD` predeterminado es `./app`, que
   ejecuta `serve` con auto-migración.

La primera construcción suele tardar varios minutos (caché de Rust en
frío); las construcciones siguientes son mucho más rápidas gracias al
cacheo por capas de Docker.

## Añade un servicio de planificador

Si tu app usa horarios `#[derive(Task)]`, el planificador necesita su
propio proceso de larga duración. Añade un segundo servicio desde el
mismo repo:

1. **New** → **GitHub Repo** → elige el mismo repositorio.
2. Nómbralo `scheduler` para que sea fácil de identificar en el panel.
3. En **Settings** → **Deploy**, establece el **Custom Start Command**
   en:
   ```bash
   ./app schedule:work
   ```
4. Copia las mismas variables (especialmente `APP_KEY` y las
   referencias a la base de datos) para que el worker lea la misma
   configuración que el servicio web.

`schedule:work` es un bucle daemon - se despierta una vez por minuto,
consulta el planificador en busca de tareas pendientes y las ejecuta a
través del mismo arranque que el servidor HTTP. Ver [Consola](console.md)
y el capítulo del planificador para el contrato.

Ejecuta exactamente una instancia del planificador. Varios procesos
`schedule:work` se coordinan mediante bloqueos respaldados por caché,
pero lo esperado por defecto es un único worker.

### Por qué Suprnova diverge

Un despliegue de Laravel en Forge o Vapor suele conectar un servidor
web (php-fpm + nginx), un worker de cola (`php artisan queue:work`) y
una entrada de cron que invoca `schedule:run` cada minuto. Tres
componentes, tres superficies de despliegue.

Suprnova compila cada rol en el mismo binario. La especificación del
servicio de Railway es `./app` para el rol web y `./app schedule:work`
para el planificador - misma imagen, mismo arranque, distinto argv. No
hay contenedor php-fpm separado, ni imagen de worker separada, ni cron
en el host. Añade `./app queue:work` como tercer servicio si tienes
jobs en cola y tendrás toda la topología de Laravel en tres servicios
de Railway a partir de un único Dockerfile.

## Verificación de salud y `railway.json`

Para más control sobre el despliegue, haz commit de un `railway.json`
en la raíz del repo. Railway lo detecta automáticamente.

```json
{
  "$schema": "https://railway.app/railway.schema.json",
  "build": {
    "builder": "DOCKERFILE",
    "dockerfilePath": "Dockerfile"
  },
  "deploy": {
    "startCommand": "./app",
    "healthcheckPath": "/_suprnova/health/live",
    "healthcheckTimeout": 300,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10
  }
}
```

Suprnova incluye puntos de conexión de salud integrados que se
cortocircuitan antes de la cadena de middleware - devuelven un estado
JSON 200 sin pasar por auth, CSRF ni límite de velocidad. El prefijo
`/_suprnova/` está reservado para que nunca colisionen con tus rutas.

`healthcheckPath` arriba apunta a `/_suprnova/health/live`, que no
toca nada. Ese emparejamiento es deliberado: este servicio está
configurado con `"restartPolicyType": "ON_FAILURE"`, así que lo que
sea que compruebe la verificación de salud es un disparador de
reinicio. Apuntarla a la base de datos - vía `/_suprnova/health/ready`
o el `/_suprnova/health?db=true` más antiguo - hace que un fallo
puntual de la base de datos reinicie cada réplica justo en el momento
en que la base de datos menos puede permitirse una avalancha de
reconexiones. Comprueba la base de datos desde una verificación de
preparación separada o desde tu monitoreo, no desde la ruta que
reinicia el proceso. Ver [Usa la sonda correcta para la pregunta
correcta](deployment.md#use-the-right-probe-for-the-right-question).

Ambas rutas antiguas siguen funcionando, así que un servicio de
Railway ya existente no necesita cambios; las rutas con nombre son
simplemente más claras.

## Dominios personalizados y TLS

1. En el servicio web, abre **Settings** → **Networking**.
2. Haz clic en **Generate Domain** para un subdominio
   `*.up.railway.app`, o en **Custom Domain** para apuntar tu propio
   nombre de host al servicio.
3. Actualiza el DNS como te indique Railway (un `CNAME` para
   subdominios, un ANAME/ALIAS para dominios raíz).

Railway aprovisiona y renueva certificados de Let's Encrypt tanto para
los dominios generados como para los personalizados.

## Migraciones en CI/CD

El `CMD ["./app"]` predeterminado ejecuta migraciones en el arranque,
lo cual está bien para despliegues de instancia única. Para
configuraciones multi-réplica, desacopla el paso de migración:

1. Añade un **pre-deploy hook** de una sola vez que ejecute
   `./app migrate` contra la base de datos de producción antes de que
   arranquen las réplicas nuevas.
2. Cambia el comando de inicio en runtime a `./app serve --no-migrate`
   para que las réplicas no compitan entre sí.

El runner de migraciones es idempotente - incluso si no divides los
pasos, ejecutar migraciones en cada arranque es seguro entre réplicas.
La división existe para que puedas fallar el despliegue pronto ante
una migración mala sin mantener el rollout abierto.

## Logs, métricas, reversiones

La pestaña del servicio web expone:

- **Deployments** - cada construcción en orden cronológico; el menú
  de tres puntos en un despliegue exitoso anterior es la ruta de
  reversión con un clic
- **Logs** - salida de `tracing` del contenedor, con campos de log
  estructurados (`request_id`, `route`, `status`) listos para los
  filtros del visor de logs
- **Metrics** - CPU, memoria, E/S de red; útil para dimensionar la
  instancia hacia arriba o hacia abajo

## Solución de problemas

**Falla `cargo build --release`.** Reprodúcelo en local con
`docker build -t myapp .`. La causa más común es un miembro del
workspace que compila en tu máquina pero falta en el repo - el
Dockerfile copia primero `Cargo.toml` y `Cargo.lock`, así que los
crates faltantes fallan de forma estrepitosa.

**La app devuelve "connection refused".** Comprueba que
`SERVER_HOST=0.0.0.0` esté establecido en el servicio. El valor
predeterminado es `127.0.0.1`, al que Railway no puede enrutar.

**La app arranca y luego sale con un error de clave.** `APP_KEY` no
está establecida o está mal formada. El framework se niega a
arrancar en producción sin ella; vuelve a pegar la salida de
`suprnova key:generate --show` en las variables del servicio.

**Las migraciones fallan en el arranque.** Revisa los logs en busca
del error SQL subyacente. Las causas más comunes son una
`DATABASE_URL` sin establecer (verifica que la referencia
`${{ Postgres.DATABASE_URL }}` se resolviera) o una migración
ejecutada contra una línea base obsoleta (`./app migrate:status`
informa qué está aplicado y dónde).

**El planificador nunca se dispara.** Verifica que el comando de
inicio sea exactamente `./app schedule:work` (no `schedule:run`, que
ejecuta las tareas pendientes una vez y sale). `schedule:list` desde
un despliegue de una sola vez confirma que tus tareas están
registradas.

## Siguiente

- [Descripción general de despliegue](deployment.md) - el modelo de
  binario unificado que ejecutan tus servicios de Railway
- [CLI de Docker](cli-docker.md) - lo que `docker:init` y
  `docker:compose` generan realmente
- [Configuración](configuration.md) - carga de `.env`, config
  tipada, claves requeridas
- [Consola](console.md) - `schedule:work`, `queue:work`,
  `workflow:work`, y el resto de la CLI unificada
- [Desplegar en Digital Ocean](deployment-digital-ocean.md) - la
  misma receta en un PaaS distinto
