# URLs HTTPS de desarrollo nombradas (`suprnova dev:tls`)

Por defecto, `suprnova serve` expone tu backend en un
`http://127.0.0.1:8765` sin adornos. Eso basta para la mayoría del
desarrollo - pero algunas funciones del navegador solo funcionan por
HTTPS en un host con nombre:

- **Passkeys / WebAuthn** - requieren un contexto seguro y un origen
  estable.
- **Cookies `Secure`** y **`SameSite=None`** - solo se establecen por
  HTTPS.
- **Service workers** - solo se registran por HTTPS (o en
  `localhost`).
- **URIs de redirección OAuth/OIDC** - los proveedores suelen
  rechazar hosts con IP/puerto sin nombre.

[portless](https://portless.sh) le da a cada app local una URL
`https://<nombre>.localhost` estable, detrás de un único proxy TLS en
el puerto 443. `suprnova dev:tls` conecta Suprnova con portless y -
la parte fácil de hacer mal - confía en la CA local de portless en
**todos los almacenes de certificados de los navegadores de tu
máquina**, sin sudo en Linux.

> **Estrictamente opcional.** portless nunca es obligatorio.
> `suprnova serve` funciona sin portless instalado. Te sumas a él al
> generar el andamiaje (`suprnova new <nombre> --with-portless`) o
> añadiendo `portless.json` después. Si nunca ejecutas `dev:tls`,
> nunca tocas portless.

## Instalar portless

portless es una herramienta de Node:

```bash
npm install -g portless
```

Luego instala su proxy 443 siempre activo, una sola vez (este es un
paso a nivel de sistema, que requiere sudo y le corresponde a
portless, no a Suprnova):

```bash
portless service install
```

## Por proyecto

Tienes dos formas de sumar un proyecto.

**Generar el andamiaje con el flag** - escribe `portless.json` desde
el principio:

```bash
suprnova new myapp --frontend svelte --with-portless
```

Eso emite un `portless.json` en la raíz del proyecto:

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` es el `SERVER_PORT` fijo de tu backend. Le indica a
portless que la app se enlaza a un puerto conocido (en vez de que
portless asigne uno vía `$PORT`), de modo que la URL nombrada la
enruta directamente.

**Añadirlo a un proyecto existente** - escribe ese mismo
`portless.json` a mano (o ejecuta `portless alias myapp 8765`),
usando tu `SERVER_PORT`.

Luego, en **cada máquina** que vaya a ejecutar la app, haz el
registro único de confianza + ruta:

```bash
cd myapp
suprnova dev:tls
```

Esto:

1. Comprueba que `portless` esté en tu PATH.
2. Resuelve el nombre (`--name`, si no el `name` de
   `[package]` en `Cargo.toml`) y el puerto (`--port`, si no
   `SERVER_PORT`, si no `8765`).
3. Registra la ruta `myapp.localhost → 127.0.0.1:8765` (omítelo con
   `--no-alias`).
4. Confía en la CA de portless en los almacenes de certificados de
   tus navegadores.
5. Imprime los pasos siguientes.

Flags:

| Flag | Efecto |
|---|---|
| `--name <nombre>` | Sobrescribe el nombre de la URL. Por defecto: el nombre del paquete en `Cargo.toml`. |
| `--port <puerto>` / `-p` | Sobrescribe el puerto enrutado. Por defecto: `SERVER_PORT`, si no `8765`. |
| `--no-alias` | Solo confía en la CA; no toca la ruta de portless. |
| `--yes` | Omite la confirmación antes de modificar tus almacenes de certificados. Se ignora cuando la huella de la CA cambió desde la última ejecución - eso siempre pregunta. |

### Por qué el paso 4 pregunta primero

Confiar en una CA significa que tu navegador acepta, en silencio,
cualquier certificado que ella firme, para cualquier sitio. Eso vale
una pulsación de tecla deliberada.

La CA se resuelve solo a partir del propio estado de portless, nunca
de nada que el directorio del proyecto pueda influir - un repositorio
descargado no puede apuntar `dev:tls` hacia una CA de su elección. El
comando imprime la huella en la que está a punto de confiar y espera
tu confirmación. Si la huella difiere de la que se confió antes,
pregunta incluso con `--yes`: una CA cambiada es, o bien una
reinstalación de portless, o bien algo que quieres revisar, y solo tú
puedes saber cuál.

## Ejecutar

```bash
suprnova serve
```

Abre `https://myapp.localhost`.

El backend se enlaza a `8765` por defecto; el servidor de desarrollo
de Vite viaja aparte en `5765` por `http://localhost`. Una página
servida desde el origen HTTPS puede referenciar recursos de
`http://localhost` porque los navegadores tratan `localhost` como un
contexto seguro - **no** se bloquea como contenido mixto.

> **La recarga en caliente de módulos (HMR) por HTTPS es un mejor
> esfuerzo, sin garantías.** El websocket de HMR de Vite se conecta
> de vuelta al servidor de desarrollo; que eso funcione limpiamente
> sobre el origen HTTPS depende de tus versiones de Vite/navegador.
> Si las actualizaciones en vivo dejan de funcionar bajo `https://`,
> apunta Vite a un origen de servidor de desarrollo HTTPS con la
> variable de entorno `INERTIA_VITE_DEV_SERVER`. La carga de páginas
> y el resto del flujo no se ven afectados.

## Varias apps

portless posee el puerto 443 y multiplexa por subdominio. Registra
cada app con su propio nombre y puerto:

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

Nunca enlaces el puerto 443 directamente desde una app - ese es el
trabajo de portless.

## Solución de problemas

**`ERR_CERT_AUTHORITY_INVALID` tras ejecutar `dev:tls`.** Tu
navegador no se reinició por completo. Los navegadores leen su
almacén de certificados una sola vez, al arrancar; recargar una
pestaña no basta. Escribe `chrome://restart` (o cierra y vuelve a
abrir el navegador por completo).

**`502 Bad Gateway`.** El proxy está activo, pero tu backend no. Ejecuta
`suprnova serve` en el directorio del proyecto.

**`portless trust` dice "A terminal is required to authenticate".**
Ese es el propio comando de portless, que necesita una TTY real para
`sudo`. `suprnova dev:tls` lo evita por completo en Linux: instala la
CA directamente en los almacenes NSS de tus navegadores, que no
necesitan sudo.

**Un navegador Flatpak sigue sin confiar en ella.** Los navegadores
Flatpak guardan su base de datos NSS en
`~/.var/app/<id>/.pki/nssdb`. `dev:tls` cubre esos casos - vuelve a
ejecutarlo y reinicia ese navegador por completo.

**`certutil: command not found`.** Instala las herramientas NSS:

| Distribución | Comando |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`portless CA not found at ~/.portless/ca.pem`.** portless genera su
CA la primera vez que el proxy se ejecuta. Inícialo una vez
(`systemctl start portless`, o `portless proxy start`), y luego
vuelve a ejecutar `suprnova dev:tls`.

## Notas de plataforma

La ruta por NSS del navegador de arriba es el mecanismo de Linux. En
**macOS** y **Windows**, los navegadores leen el llavero / almacén de
certificados del sistema operativo, así que `dev:tls` delega la
confianza en la CA a `portless trust`, que apunta a esos almacenes
nativos.
