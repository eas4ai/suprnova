# Comandos de programación

Superficie de CLI para el planificador de tareas por minuto. Los tres
subcomandos `schedule:*` delegan en el despacho `Application::run()`
del binario de tu aplicación, así que ven la misma configuración, los
mismos servicios, observadores, y oyentes que ve un handler de
solicitud. El modelo completo del planificador - el trait `Task`, la
API fluida de cron, `without_overlapping`, `run_in_background` - vive
en [Programación de tareas](scheduling.md); este capítulo es la
referencia de operador para los comandos en sí.

## Cómo se ejecutan los comandos

`suprnova schedule:run`, `suprnova schedule:work`, y `suprnova
schedule:list` son envoltorios delgados que invocan `cargo run --
schedule:<subcommand>` contra el proyecto en el directorio actual.
Los mismos subcomandos también son alcanzables directamente en el
binario de la aplicación en producción:

```bash
# En desarrollo (desde la raíz del proyecto, build desde fuente):
suprnova schedule:run

# En producción (binario en el PATH):
/usr/local/bin/myapp schedule:run
```

Los drivers de runtime (Cache, Queue, RateLimit, Mail) y tu
`bootstrap_fn` arrancan antes de que se ejecute cualquier tarea, así
que una tarea programada puede resolver servicios desde el
contenedor exactamente igual que un controlador - consulta [Arranque
de la aplicación](bootstrap.md).

Debes conectar el planificador al builder de la aplicación para que
los subcomandos encuentren alguna tarea:

```rust
// cmd/main.rs (starter de backend) o src/main.rs (starter de API)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- el hook del planificador
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` conecta esto automáticamente; si
construyes la cadena a mano, añade tú mismo la llamada
`.schedule(...)`.

## schedule:run

Evalúa cada tarea registrada una vez y ejecuta las que su expresión
cron coincide con el minuto actual. Diseñado para ser invocado por el
cron del sistema cada minuto. Sale con código distinto de cero si
alguna tarea falló; sale con cero (con `No tasks were due.`) si nada
estaba vencido este minuto.

```bash
suprnova schedule:run
```

### Salida de ejemplo

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

Cuando una tarea devuelve un error, su línea se prefija con `✗` y se
añade el mensaje de error:

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

Cuando ninguna tarea está vencida este minuto:

```
Running due scheduled tasks...
No tasks were due.
```

### Entrada de crontab

Una única entrada ejecuta el planificador cada minuto. El binario de
la aplicación evalúa por sí mismo todas las tareas vencidas, así que
esta es la única línea de crontab que necesita un host de producción:

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

Si estás ejecutando `schedule:run` desde el cron del sistema en más
de un host (o junto a un demonio `schedule:work`), las tareas
marcadas con `.without_overlapping()` necesitan un backend de Cache
configurado (`CACHE_DRIVER=redis` es la opción de calidad de
producción) para coordinarse entre procesos - consulta [Prevenir el
solapamiento](scheduling.md#preventing-overlapping) para la
semántica del bloqueo.

## schedule:work

Ejecuta el planificador como un demonio de larga duración. El primer
tick se alinea al siguiente límite de minuto, y luego el bucle evalúa
las tareas vencidas una vez por minuto hasta que recibe `SIGINT`
(Ctrl-C) o `SIGTERM`. Al apagar, cualquier tarea `run_in_background`
que todavía esté en curso se espera antes de salir, para que no se
derriben a mitad de escritura.

```bash
suprnova schedule:work
```

### Salida de ejemplo

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

Cada tick es silencioso - solo se registran los fallos. Al apagar:

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### Casos de uso

- **Desarrollo.** No se necesita crontab - inicia el demonio en una
  terminal y observa cómo hace tick.
- **Docker.** Úsalo como el proceso principal del contenedor cuando
  quieras que una imagen cumpla el rol de planificador.
- **Systemd.** Gestiónalo como una unidad de larga duración (consulta
  la [unidad de systemd](#unidad-de-systemd) más abajo).

### Unidad de systemd

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` hace que el demonio vuelva a levantarse si se cae;
`RestartSec=5` aplica antirrebote a un bucle de caídas. Porque el
límite de pánico del framework atrapa las tareas que entran en pánico
y las convierte en `FrameworkError`, una sola tarea defectuosa no
debería derribar al demonio - `Restart=always` es para el fallo raro
de todo el proceso (OOM, que mate al padre).

## schedule:list

Imprime cada tarea registrada con su expresión cron y su descripción.

```bash
suprnova schedule:list
```

### Salida de ejemplo

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
  heartbeat [* * * * *]
```

Las tareas que tienen un `.description(...)` encadenado en el builder
incluyen la descripción después de la expresión cron; las tareas sin
descripción muestran solo el cron.

Cuando no hay nada registrado (falta la llamada al builder
`.schedule(...)`, o `schedule::register` es un no-op):

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## Generar una tarea

El framework distribuye un generador que crea la tarea, la conecta al
proyecto, y añade la llamada al planificador a tu `main.rs`:

```bash
suprnova make:task CleanupLogs
```

Esto:

1. Se crea `src/tasks/cleanup_logs_task.rs` (un stub `Task` funcional
   que registra su propia duración)
2. Se crea `src/tasks/mod.rs` (reexportando `CleanupLogsTask`) si
   todavía no existe
3. Se crea `src/schedule.rs` (con una función `register(&mut
   Schedule)`) si todavía no existe
4. Se declaran `pub mod schedule;` y `pub mod tasks;` en `src/lib.rs`
5. Se añade `.schedule(<crate>::schedule::register)` a la cadena
   `Application` en `cmd/main.rs` (o `src/main.rs` para el starter de
   API)

Los pasos 2 a 5 son idempotentes, así que volver a ejecutar
`make:task` repara el cableado que se eliminó a mano. Consulta
[Generadores](cli-generators.md) para la familia `make:*` más amplia.

Después de generarlo, registra la tarea en `src/schedule.rs`:

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

La API fluida del builder (`.daily()`, `.cron(...)`,
`.without_overlapping()`, `.run_in_background()`, modificadores
específicos de día) se cubre completamente en [Programación de
tareas](scheduling.md).

## Códigos de salida

| Comando | Sale con cero | Sale con código distinto de cero |
|---|---|---|
| `schedule:run` | cada tarea vencida devolvió `Ok(())`, o ninguna tarea estaba vencida | al menos una tarea devolvió `Err(_)` o entró en pánico |
| `schedule:work` | apagado limpio vía `SIGINT` / `SIGTERM` (el envoltorio trata el código de salida 130 como un Ctrl-C limpio) | fallo de bootstrap, o el proceso del demonio abortó |
| `schedule:list` | el listado tuvo éxito (incluyendo el mensaje de "no hay tareas registradas") | la aplicación no logró arrancar |

Los fallos de tareas en segundo plano dentro de `schedule:work` se
registran en stderr pero no hacen salir al demonio - el límite
`catch_unwind` del `JoinSet` los hace emerger como `FrameworkError` y
el bucle de tick continúa.

### Por qué Suprnova diverge

El `schedule:run` de Laravel es el único punto de entrada de primera
clase; la forma de demonio (`schedule:work`) es un backport para
hosts sin crontab. PHP no tiene un proceso de larga duración, así que
cada minuto es un runtime nuevo que tiene que rearrancar el
framework, el contenedor, y cada vinculación de servicio.

En Suprnova el demonio es de primera clase. `schedule:work` se
ejecuta dentro del mismo runtime de Tokio que sirve HTTP, así que:

- **Las tareas en segundo plano se combinan con el bucle de ticks.**
  Una tarea `.run_in_background()` se lanza dentro de un `JoinSet`;
  el bucle hace emerger las que ya terminaron antes del siguiente
  tick, y drena el resto al apagarse. Laravel lanza un proceso hijo
  por cada tarea en segundo plano.
- **El apagado ordenado drena el trabajo en curso.** Ctrl-C /
  SIGTERM deja que las tareas en línea terminen su llamada actual, y
  espera cada lanzamiento en segundo plano antes de salir. Laravel
  depende de que el SO mate al hijo de cron.
- **El costo de arranque se paga una sola vez.** El contenedor, los
  drivers, y tu `bootstrap_fn` arrancan al iniciar el demonio, no en
  cada tick. `schedule:run` todavía paga el costo de arranque por
  invocación (es un subcomando de una sola vez), pero la ruta del
  demonio es donde el modelo de runtime da sus frutos.

`schedule:run` sigue funcionando (y es la opción correcta cuando el
cron del sistema ya es la fuente de verdad del operador). Elige el
que se ajuste a la forma de tu despliegue - ambos comparten las
mismas definiciones de tarea.

## Siguiente

- [Programación de tareas](scheduling.md) - el trait `Task`, la API
  fluida de cron, `without_overlapping`, `run_in_background`, y la
  deduplicación en el mismo minuto
- [Generadores](cli-generators.md) - la familia completa `make:*`,
  incluyendo `make:task`
- [Consola](console.md) - `#[command]` para tareas de operador de una
  sola vez (no programadas)
- [Cola](queues.md) - para trabajo que debería recoger un worker en
  lugar de marcar el tick de un reloj
- [Arranque de la aplicación](bootstrap.md) - cómo se conecta
  `.schedule(...)` al builder, y qué pueden resolver las tareas desde
  el contenedor
