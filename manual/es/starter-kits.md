# Kits de inicio

Los kits de inicio son aplicaciones Suprnova preconfiguradas que bifurcas y
despliegas. Cada uno conecta los controladores, rutas, migraciones, páginas del
frontend y pruebas para una superficie de producto completa - así que comienzas
desde una app en funcionamiento, no desde un andamiaje vacío.

Hoy se envían dos kits, modelados según la línea de Laravel. Elige el que más se
acerque a lo que estás construyendo y personalízalo desde allí.

## Nebula - autenticación (nivel Breeze)

**Repositorio: [github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

El kit de autenticación completa mínimo - el equivalente de Breeze de Suprnova. Todo
lo que necesitas para cuentas y nada que no necesites:

- Registro con verificación de correo electrónico
- Inicio de sesión con recordarme
- Restablecimiento de contraseña con respuestas anti-enumeración
- Gestión de perfil - actualizar correo electrónico y contraseña, eliminar cuenta
- Un frontend Inertia 3 + Svelte 5 personalizado (oscuro por defecto), con el
  menú de usuario autenticado conectado

Nebula envía dos suites de pruebas: lógica de autenticación a nivel de fachada y una
suite HTTP a nivel de conexión que ejecuta rutas reales, sesiones, viajes redondos
CSRF y compuertas guest/auth/verified a través de un socket loopback.

Recurre a Nebula cuando desees una base de gestión de cuentas limpia para construir
tu propio producto encima.

## Pulsar - sitio de producto y comunidad

**Repositorio: [github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

Un sitio completo de herramienta para desarrolladores / empresa SaaS en Vue 3.5 +
Vuetify. Todo en la historia de autenticación de Nebula, más las superficies que un
sitio de producto real necesita:

- Página de destino de marketing y panel de control del usuario
- Un pipeline de documentación Markdown (`docs:build`) con búsqueda y una
  tabla de contenidos generada
- Un sistema de blog / artículos con feed RSS
- Perfiles de miembros públicos
- Taxonomía - temas, etiquetas y categorías
- Control de acceso basado en roles: roles, permisos y compuertas
- Superficies de administrador y moderación para contenido y miembros

Pulsar es el kit fuente para productos posteriores como `suprnova.app`. Recurre
a él cuando estés desplegando un sitio de producto con docs, blog y comunidad de
miembros - no solo autenticación.

## ¿Cuál kit?

| Quieres… | Comienza con |
|---|---|
| Cuentas y un lugar para construir | **Nebula** |
| Un sitio de producto completo - destino, docs, blog, comunidad, RBAC | **Pulsar** |
| Un backend solo API (autenticación de token, sin frontend) | `suprnova new my-api --api` |

Ambos kits rastrean el framework como una dependencia git y se ejecutan en la
misma pila que ya conoces - consulta el README de cada repositorio para la
configuración. Se planean más kits; vigila los
[lanzamientos](https://github.com/eas4ai/suprnova/releases) o abre un
problema si hay uno que quieras.

## Lo que te da el andamiaje por defecto

Si ningún kit se ajusta, `suprnova new my-app --frontend svelte` (o `react`, o
`vue`) ya envía un flujo de autenticación en funcionamiento - inicio de sesión,
registro, cierre de sesión, autenticación de sesión con el middleware
`authenticate`, protección CSRF y una ruta protegida `/dashboard` - en cualquiera
de los tres frontends (Svelte 5, React 19, Vue 3.5) con Tailwind v4 e Inertia v3.
Consulta [Instalación](installation.md) para la salida del andamiaje e
[Inicio rápido](quickstart.md) para el recorrido de los primeros cinco minutos.

Para servicios solo API, `suprnova new my-api --api` envía la misma pila de
backend con autenticación basada en token en lugar de sesiones y sin frontend.

## Contribuir un kit de inicio

¿Has construido algo reutilizable sobre Suprnova y quieres enviarlo como un kit
canónico? Consulta [Contribuciones](contributions.md). Estamos felices de tomar
una implementación real y redondearlo en un kit genérico.
