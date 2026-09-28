# Guía de contribución

Suprnova es de código abierto bajo la licencia MIT, y la contribución
más valiosa es un **buen informe**. El proyecto no acepta pull
requests: el framework está escrito de principio a fin por los
mantenedores, y todo cambio llega a través de ellos para que toda la
superficie mantenga una sola forma. Es una postura deliberada y
permanente - no una fase previa a la 1.0.

MIT significa que nunca necesitas permiso para llevar el código más
allá por tu cuenta: **haz fork libremente**. Un fork que crece en su
propia dirección es un resultado saludable, no una rivalidad.

Lo que eso significa en la práctica:

- **Informes de bugs** - bienvenidos, vía
  [GitHub issues](https://github.com/eas4ai/suprnova/issues).
- **Solicitudes de características** - bienvenidas, vía issues.
  Describe el caso de uso, no la implementación; a menudo ya existe
  una forma planificada (normalmente el equivalente en Laravel).
- **Bugs de documentación** - bienvenidos, vía issues. Si un capítulo
  dice que una API existe y no la encuentras, eso es un bug de
  documentación - indica qué capítulo y qué esperabas.
- **Problemas de seguridad** - en privado, por correo (ver más abajo).
  Nunca como issues públicos.
- **Pull requests** - no se aceptan. Los PR se cierran con un enlace a
  este capítulo; abre un issue en su lugar para que la corrección
  pueda llegar en origen, o haz fork y lleva el cambio tú mismo.

## Presentar un informe de bug que se corrija rápido

El estándar de oro es una reproducción desde un proyecto recién
generado con andamiaje:

```bash
suprnova new repro-app --frontend vue --no-interaction
# …el cambio más pequeño que muestre el bug…
```

Incluye:

1. **Qué hiciste** - los comandos y el código, reducidos al mínimo
2. **Qué esperabas** - una frase
3. **Qué pasó en su lugar** - la salida o el error real, pegado
   textualmente
4. **Versiones** - el tag del framework (`suprnova --version`, o el
   `tag =` en tu `Cargo.toml`) y tu versión de Rust (`rustc --version`)

Una prueba que falla es incluso mejor que la prosa. Si puedes expresar
el bug como una prueba contra el framework, pégala en el issue - por
lo general se convertirá en la prueba de regresión con la que llega la
corrección.

## Compilar desde el código fuente (para investigar un informe)

No necesitas esto para *presentar* un issue, pero reproducirlo contra
el workspace a menudo afina un informe:

```bash
git clone https://github.com/eas4ai/suprnova.git
cd suprnova
cargo check --workspace          # verifica los tipos de todo
cargo test --workspace           # ejecuta la suite completa (~3400 pruebas)
```

La disposición del workspace: `framework/` (el crate `suprnova`),
`suprnova-cli/` (el binario `suprnova`), `suprnova-macros/` (macros
proc), `app/` (app interna para dogfooding), `crates/` (adaptadores de
pagos y web-push), y `manual/` (este manual).

## El estándar que debe cumplir el código

No son reglas para colaboradores - pero conocer el estándar te ayuda a
calibrar los informes (un pánico desde código de biblioteca, una
prueba de modo de fallo ausente, o una API que obliga a usar
`unwrap()` siempre vale la pena reportar):

- **Solo implementaciones completas.** Sin TODOs, sin andamiaje
  parcial. Una corrección llega con la prueba de regresión que la
  fija.
- **El código de superficie pública devuelve `Result`, no hace
  pánico.** Donde se publica un nombre infalible al estilo Laravel, se
  publica junto con él una contraparte `try_*`.
- **Ningún `unsafe` fuera del arranque del entorno.** El framework
  tiene exactamente dos bloques `unsafe` en código que no es de
  pruebas, ambos en `config/env.rs::load_dotenv`, ambos envolviendo
  `std::env::set_var` / `remove_var` - que se volvieron `unsafe` en la
  edición 2024 - y ambos con una nota SAFETY para la invariante de
  hilo único en tiempo de arranque de la que dependen. Todo lo demás
  es solo para pruebas. Un `unsafe` nuevo en cualquier otro sitio
  necesita una justificación escrita en la revisión, y un `unsafe` en
  un driver, handler, o expansión de macro no se aceptará.
- **`cargo fmt` y clippy sin denegar todas las advertencias son canónicos.**

Ver [Modelo de errores](error-model.md) para el contrato de errores
completo.

## Seguridad

Reporta los problemas de seguridad en privado a
**shawn@eas4ai.com** (el mantenedor del proyecto). Confirmaremos la
recepción en unos pocos días, trabajaremos la corrección en una rama
privada y coordinaremos la divulgación contigo.

No presentes problemas de seguridad como issues públicos de GitHub hasta
que se haya publicado una corrección.

### Avisos de dependencias

`cargo audit` se ejecuta en la puerta de release. Si un aviso no tiene
corrección disponible y el código vulnerable no es alcanzable en un build
por defecto, puede añadirse a la lista de exclusiones de la auditoría -
pero cada entrada necesita tres cosas, y sin ellas la puerta falla:

```toml
# OWNER: name <email>
# EXPIRES: YYYY-MM-DD
"RUSTSEC-XXXX-XXXX",
```

- un **propietario**, para que la excepción pertenezca a alguien;
- una **fecha de caducidad**, tras la cual la puerta se niega a
  ejecutarse hasta que la entrada se renueve con un motivo declarado o
  se elimine;
- un **argumento de alcanzabilidad escrito** - qué ruta la arrastra, y
  por qué un build por defecto no la enlaza.

Las afirmaciones de alcanzabilidad se verifican, no se asumen. Si tu
argumento es "esto está detrás de una feature desactivada por defecto",
la puerta de release resuelve los árboles de dependencias reales y
afirma que el crate está ausente del árbol por defecto y presente en el
de la feature activada. Una excepción cuya justificación nada verifica
deja de ser cierta en silencio la primera vez que alguien añade una
dependencia.

Una exclusión es una decisión de publicar un problema conocido. Debería
leerse como tal.

## Licencia

MIT, con atribución al proyecto
[Kit](https://github.com/dayemsiddiqui/kit) del que hicimos fork.
