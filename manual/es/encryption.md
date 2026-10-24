# Cifrado

Suprnova ofrece cifrado a nivel de aplicación como una fachada global
al proceso llamada `Crypt`. Cifra strings o cualquier valor
`Serialize` bajo AES-256-GCM, con clave según tu `APP_KEY`. Recurre a
ella siempre que necesites poner algo sensible en un almacenamiento en
el que no confías del todo - una columna, una cookie, un cursor de
paginación - y necesites leerlo de vuelta intacto más adelante.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

El framework mismo usa `Crypt` para cookies cifradas, cursores de
paginación cifrados, secretos de 2FA, códigos de recuperación, y los
casts `AsEncrypted*` de Eloquent. La misma fachada está disponible para
tu código sin cableado adicional una vez que `APP_KEY` está
configurada (consulta [Configuración](configuration.md#the-env-file)).

## El formato en la red

`encrypt_string` y `encrypt` devuelven ambos base64 seguro para URL
(sin padding) sobre `nonce || ciphertext_with_tag`:

```
base64url( [nonce aleatorio de 12 bytes] || [texto cifrado] || [tag GCM de 16 bytes] )
```

Cada llamada muestrea un nonce fresco de 12 bytes desde el RNG del
sistema operativo, así que dos cifrados del mismo texto plano bajo la
misma clave producen textos cifrados distintos. No hay ningún oráculo
de padding que filtre información de longitud más allá de la del
propio texto plano.

La salida es segura de poner en query strings de URL, cuerpos JSON,
encabezados, y cookies sin codificación adicional. Un wire válido
mínimo mide 28 bytes (12 de nonce + 16 de tag) - cualquier cosa más
corta se rechaza de entrada.

## `APP_KEY` - el único secreto que importa

Suprnova lee una única clave simétrica de 32 bytes desde la variable
de entorno `APP_KEY`. El formato esperado es base64 seguro para URL,
sin padding, que decodifica a exactamente 32 bytes (43 caracteres
base64):

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Genera una con la CLI:

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

O redirígela directamente al entorno:

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### Validación en el arranque - falla en cerrado

`Server::from_config` valida `APP_KEY` **en cada arranque**, no solo en
el primero. Las reglas:

| Entorno | `APP_KEY` sin establecer | `APP_KEY` malformada |
|---|---|---|
| `local`, `development`, `testing` | Clave transitoria generada, advertencia en los logs | Error duro - falla el arranque |
| `staging`, `production`, cualquier otra cosa | Error duro - falla el arranque | Error duro - falla el arranque |

Una clave malformada **siempre** es un error duro, incluso en `local` -
es mejor que el arranque falle que enmascarar una errata. Un valor de
entorno `Custom` que el framework no reconoce (por ejemplo,
`APP_ENV=k8s`) se trata como si fuera de producción: sin `APP_KEY`, sin
arranque.

El diagnóstico señala la solución:

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - separación de dominios mediante AAD

Cada llamada a `Crypt::*` toma un `CryptPurpose`. La variante se mapea
a una etiqueta de bytes estable que se ata al tag de autenticación de
AES-GCM como Datos Asociados (AAD):

```rust
pub enum CryptPurpose {
    Cookie,            // suprnova:cookie:v1
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

La etiqueta **no** se almacena en el wire. GCM mezcla el AAD dentro del
tag de autenticación sin incluirlo en el texto cifrado, así que:

- El formato en la red no cambia - sigue siendo
  `base64(nonce || ciphertext || tag)`.
- Un wire producido bajo `CryptPurpose::Cookie` es **rechazado** por
  cualquier llamada de descifrado que suministre un propósito
  distinto. La comprobación del tag GCM falla antes de que corra
  ningún análisis posterior al descifrado.
- Añadir una nueva superficie (un futuro cifrado de payload de cola,
  una cabecera de archivo cifrada) significa añadir una nueva
  variante, no cambiar el formato en la red.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cookie, "session-id")?;

// Misma clave, mismo wire, propósito distinto - falla.
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// Mismo propósito - funciona.
let plain = Crypt::decrypt_string(CryptPurpose::Cookie, &wire)?;
```

### Por qué Suprnova diverge

El `Crypt::encryptString` de Laravel no toma un propósito. La única
`APP_KEY` se reutiliza entre cookies, URLs firmadas, tokens de
expiración firmados, y cualquier llamada de usuario a
`Crypt::encrypt`, sin separación de dominios en la capa criptográfica.
Si dos superficies llegan a aceptar texto cifrado con la misma forma
de texto plano, un valor acuñado para una superficie puede repetirse
en la otra.

Suprnova reutiliza la misma `APP_KEY` por la misma razón - los
operadores gestionan un único secreto - pero ata cada superficie a su
propia etiqueta AAD. La repetición de texto cifrado entre superficies
se rechaza en la comprobación del tag GCM, antes de que corra ningún
análisis. El costo para quien llama es un parámetro enum adicional; la
ganancia es una propiedad que el formato en la red por sí solo no
puede romper.

El sufijo `:v1` en cada etiqueta está reservado para una futura
rotación por superficie: subir `suprnova:cookie:v1` a
`suprnova:cookie:v2` invalida **solo** el texto cifrado antiguo de
cookies - deja intactos los cursores, los secretos de 2FA, y las
columnas con cast.

## AAD vinculado al nombre de la cookie (v2)

Las cookies cifradas usan una segunda generación de AAD cuando quien
llama conoce el nombre lógico de la cookie.
`Cookie::encrypted("suprnova_session", value)` vincula
`suprnova:cookie:v2:suprnova_session` al tag GCM, y
`Cookie::read_encrypted_for("suprnova_session", wire)` proporciona el
mismo contexto al regresar:

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

El nombre vinculado es lógico, no el renderizado. Por tanto, un prefijo
posterior de nombre wire `__Host-` o `__Secure-` no cambia el AAD ni
cierra la sesión de los usuarios. El prefijo es una cuestión del
navegador y de las cabeceras; el nombre de la cookie es el dominio
criptográfico.

### Ventana de compatibilidad

El formato wire no cambia y no tiene versión: sigue llevando solo el
nonce, el texto cifrado y el tag de autenticación. No hay un byte de
versión que permita al lector elegir una rama.
`decrypt_string_for` usa un descifrado de prueba a ciegas con la misma
forma que la rotación de claves: prueba el AAD v2 con contexto en todo
el anillo de claves, y después el AAD v1 sin contexto en todo el anillo.
Así, las cookies escritas antes de vincular el nombre siguen siendo
legibles mientras la rotación de `APP_KEY` también está en curso.

La ventana conserva la antigua debilidad de repetición durante toda su
duración. Una cookie v1 de un slot puede seguir repetida en otro slot
mientras exista el fallback sin contexto; el beneficio de vincular el
nombre empieza cuando se elimine ese fallback en 1.4.0. Nada elimina el
fallback automáticamente: `Crypt::encrypt_string(CryptPurpose::Cookie,
...)` sigue acuñando v1, y el punto de entrada sin contexto se sustituye,
con su eliminación prevista para 1.4.0. Traslada las escrituras de
cookies a `Cookie::encrypted` y las lecturas a `read_encrypted_for` antes
de esa fecha.

Durante la ventana existe un costo medible. Un descifrado fallido de
cookie paga dos pasadas de prueba por el anillo. El middleware de sesión
hace dos lecturas cifradas por solicitud cuando están presentes una
cookie de sesión y una cookie remember-me, de modo que una solicitud
anónima con una cookie remember obsoleta paga `2 × (1 + N)` dos veces,
donde `N` es el número de claves anteriores.

### Leer `DecryptOrigin`

`Crypt::decrypt_string_for_inner` devuelve un `DecryptOrigin` con dos
ejes independientes:

- `origin.key = KeyOrigin::Previous(index)` significa que el valor aún
  depende de `APP_KEY_PREVIOUS[index]`. Vuelve a cifrar el valor con la
  clave actual y elimina esa clave anterior solo después de que termine
  la cola de rotación.
- `origin.aad = AadVersion::Legacy` significa que el valor usó el
  fallback v1 sin contexto. Para una cookie, emítela de nuevo mediante
  la API vinculada al nombre; el fallback se eliminará en 1.4.0.

Ambos ejes pueden estar obsoletos a la vez. El lector público registra
las advertencias correspondientes sin incluir texto plano ni texto
cifrado. Trata la advertencia de clave como una tarea de limpieza de
rotación y la advertencia de AAD como una tarea de migración; coincidir
con un eje no debe ocultar el otro.

## Los dos pares de encrypt / decrypt

Hay dos formas para dos casos de uso.

### Strings - `encrypt_string` / `decrypt_string`

Para strings UTF-8:

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

La ruta de descifrado devuelve un `String` - los bytes no-UTF-8 (que un
cifrado normal no puede producir, pero que un wire corrupto o
suministrado por un atacante sí podría) emergen como un
`FrameworkError::Internal` claro.

### Cualquier cosa `Serialize` - `encrypt` / `decrypt`

Para valores estructurados, codifica a JSON y luego cifra en una sola
llamada:

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

El formato en la red es el mismo - base64 sobre
`nonce || ciphertext || tag` - la única diferencia es que el texto
plano son los bytes `serde_json` de `value` en lugar del UTF-8 de un
string. Usa esto para cualquier forma de registro: un blob de
configuración, un payload de sesión, una tupla de argumentos de cola.

### `appears_encrypted` - comprobación de forma, no de manipulación

Para middleware que necesita saltarse valores ya cifrados en el paso
de salida (igualando el comportamiento del `EncryptCookies` de
Laravel), `Crypt::appears_encrypted` hace una comprobación heurística
barata:

```rust
if Crypt::appears_encrypted(cookie_value) {
    // deja pasar - ya está envuelto
} else {
    // cifra antes de enviar
}
```

Devuelve `true` cuando la entrada decodifica como base64 seguro para
URL y la longitud decodificada es de al menos 28 bytes (nonce + tag).
Nunca llama a AES-GCM, así que **no puede** distinguir un texto cifrado
válido de bytes aleatorios con la forma correcta. Quien llame y
necesite autenticación debe llamar a `decrypt_string` / `decrypt` y
manejar el error.

## Rotación de claves - el llavero

Suprnova soporta rotación sin tiempo de inactividad mediante un
*llavero* de claves: una clave actual (usada para cada cifrado nuevo)
más una lista ordenada de claves anteriores (probadas como fallback al
descifrar). Rotas `APP_KEY` sin tener que volver a cifrar cada columna
en lock-step.

Establece `APP_KEY_PREVIOUS` con una lista de claves base64 separadas
por comas, de la más antigua a la más reciente:

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# O para una rotación multi-paso (más antigua → más reciente):
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

`APP_KEY_PREVIOUS` es el nombre canónico de Suprnova.
`APP_PREVIOUS_KEYS` se acepta como alias compatible con Laravel. Si se
establecen ambas variables, gana `APP_KEY_PREVIOUS`. Cuando sus valores
recortados difieren, el arranque registra una advertencia e ignora
`APP_PREVIOUS_KEYS`.

El cifrado **siempre** usa la clave actual. El descifrado prueba primero la
clave actual; si falla, se prueba cada clave anterior en orden. Cuando acierta
una clave anterior, `Crypt` emite un `tracing::warn!`:

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

La línea de log excluye deliberadamente tanto el texto plano como el
texto cifrado - solo viaja el hecho-de-la-rotación más una pista sobre
qué hacer. Los operadores que hagan una búsqueda de logs por
`APP_KEY_PREVIOUS` encontrarán cada columna que todavía depende de una
clave antigua.

### El tope - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` tiene un tope de 8 entradas. Una cadena de rotación
realista tiene entre 1 y 3 entradas (una rotación en curso, quizá una
rotación previa estancada que el operador no ha limpiado); 8 deja un
margen generoso. Superado el tope, el arranque **falla de forma
estrepitosa** con un diagnóstico que nombra tanto la cuenta como el
tope:

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

Un truncamiento silencioso descartaría una clave de la que el operador
todavía podría depender, dejando columnas indescifrables sin ningún
diagnóstico. El tope duro es intencional.

Las entradas vacías se toleran:
`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` se analiza como dos claves reales.
Una entrada malformada (errata, longitud incorrecta, base64 inválido)
es un error duro - los secretos a medio rotar hacen fallar el arranque,
en lugar de descartar en silencio un fallback.

### Procedimiento de rotación

```bash
# 1. Acuña una clave nueva.
NEW=$(suprnova key:generate --show)

# 2. Mueve la clave actual a APP_KEY_PREVIOUS, instala la nueva.
#    Edita tu .env o tu gestor de secretos:
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. Despliega. Las escrituras nuevas usan la clave nueva; las filas
#    existentes siguen descifrándose vía el fallback de clave
#    anterior. Los logs identifican las columnas que todavía están
#    en la clave antigua.

# 4. Ejecuta una pasada de recifrado. Para cada modelo con casts
#    cifrados:
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` siempre usa la clave actual, así que un
#    load-then-save sin cambios migra la fila.

# 5. Cuando las advertencias dejen de aparecer en los logs, elimina
#    APP_KEY_PREVIOUS y despliega de nuevo.
```

Todo el procedimiento es en caliente - en ningún punto hay una ventana
en la que las solicitudes nuevas fallen.

### Observar el llavero

Para dashboards de operaciones o health checks:

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

Los bytes de la clave en sí nunca son accesibles desde la API pública.
El impl de `Debug` de `EncryptionKey` imprime `"[REDACTED]"`, y no
existe ningún accessor que exponga una clave cruda fuera del crate.

## Integración con Eloquent - los casts `AsEncrypted*`

El cifrado a nivel de aplicación es más útil en el límite de la
columna. La familia de casts `AsEncrypted*` envuelve
`Crypt::encrypt_string` para que los campos de tu modelo se mantengan
como texto plano tipado en tiempo de ejecución y como texto cifrado en
reposo:

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| Cast | Tipo en tiempo de ejecución | Forma de almacenamiento |
|---|---|---|
| `AsEncrypted` | `String` | string cifrado |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → string cifrado |
| `AsEncryptedObject<T>` | `T` | JSON → string cifrado |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → string cifrado |

Los cuatro se encaminan a través de `CryptPurpose::Cast`. Un wire
acuñado por un cast cifrado es rechazado por cualquier código que
intente descifrarlo como una cookie o un cursor - aunque `APP_KEY` sea
la misma, la etiqueta AAD difiere.

Para la superficie completa de casts, la tabla de modos de fallo, y las
recetas de recifrado, consulta [API de Eloquent](eloquent.md). La
mecánica de cifrado es la misma que la de la fachada de arriba - el
cast es azúcar que ejecuta
`Crypt::encrypt_string(CryptPurpose::Cast, …)` en el límite de
almacenamiento.

### Cifrado frente a hashing - elige la herramienta correcta

`AsEncrypted` es **reversible**. El texto plano se puede recuperar con
`APP_KEY`. Úsalo para datos que tu aplicación necesita volver a leer:
tokens de API que muestras en una página de ajustes, secretos de
terceros que reenvías a servicios upstream, direcciones a las que
envías pedidos.

Para datos que tu aplicación solo necesita *verificar* - contraseñas,
prefijos de clave de API que comparas contra tokens entrantes - usa un
hash en su lugar. Los hashes son unidireccionales: no hay ningún texto
plano que filtrar aunque `APP_KEY` se vea comprometida. Consulta
[Hashing](hashing.md) para la fachada de Bcrypt / Argon2id y el cast
`AsHashed`.

## Dónde más se usa `Crypt` dentro del framework

No tienes que hacer nada para optar por esto - se cablean
automáticamente en cuanto `APP_KEY` está configurada.

- **Cookies cifradas** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted(...)` usan `CryptPurpose::Cookie`. La cookie
  de sesión, la cookie de "recuérdame", y la cookie de bypass del modo
  mantenimiento se apoyan todas en esto. Consulta
  [Respuestas](responses.md) y [Sesiones](session.md).
- **Paginación por cursor** - `CursorPaginator` codifica el cursor bajo
  `CryptPurpose::Cursor`, de modo que el valor `?cursor=…` en la red no
  puede falsificarse ni repetirse entre superficies. Consulta
  [API de Eloquent](eloquent.md#cursor-pagination).
- **Secretos de 2FA** - el secreto TOTP en base32 cifrado, en
  `two_factor_authentications.secret`, usa
  `CryptPurpose::TwoFactorSecret`; los códigos de recuperación usan
  `CryptPurpose::TwoFactorRecovery`. Propósitos distintos evitan la
  repetición de texto cifrado entre columnas dentro de la misma fila.
  Consulta [Flujos de autenticación](auth-flows.md).
- **Firma derivada de HMAC** - las URLs firmadas y los tokens de
  restablecimiento de contraseña derivan una clave HMAC a partir de
  `APP_KEY`, en lugar de cifrar bajo ella. Los bytes crudos de la clave
  no se exportan; la derivación vive dentro del framework. Consulta
  [Enrutamiento](routing.md#signed-urls).

## Pruebas con `Crypt`

La fachada `Crypt` está respaldada por `OnceLock`, así que el primer
instalador dentro de un binario de test gana. Los ayudantes de testing
se encargan del boilerplate:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // idempotente - seguro de llamar desde cada test

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

La clave de test es determinista, de modo que los tests pueden descifrar
fixtures estables y ejercitar la rotación con una clave conocida. Las cadenas
de texto cifrado no deben compararse por igualdad entre llamadas o
ejecuciones: cada cifrado sigue usando un nonce aleatorio nuevo.

Para tests de rotación, instala un llavero directamente y acuña texto
cifrado histórico con `_test_encrypt_with`:

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// Simula un valor escrito cuando `old` era la clave actual.
let legacy_wire = suprnova::crypto::_test_encrypt_with(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// El llavero actual lo descifra vía el fallback de clave anterior,
// emitiendo la línea de warn de rotación.
let plain = suprnova::Crypt::decrypt_string(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
```

Ambos ayudantes se compilan fuera de los binarios de producción cuando
la feature `testing` está desactivada (`default-features = false`).

## Modos de fallo - cómo se ven los errores

Cada llamada falible a `Crypt::*` devuelve `Result<_, FrameworkError>`.
Los cinco errores que puedes ver:

| Causa | Dónde | Emerge como |
|---|---|---|
| `Crypt` no inicializado | Cualquier llamada antes del arranque | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| El wire no es base64 válido | `decrypt_string`, `decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| El wire es demasiado corto (< 28 bytes) | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| Falla la comprobación del tag - clave incorrecta, AAD incorrecto, bytes manipulados | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| Falla el encode / decode de JSON | `encrypt`, `decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

No hay ningún fallback silencioso hacia basura. Una clave incorrecta
contra un texto cifrado existente siempre es un error duro, tanto a
nivel de fachada como a nivel de cast. Esto coincide con el
comportamiento del `Encrypter` de Laravel y es la propiedad que hace
segura la rotación: una columna olvidada emergería de inmediato, en
lugar de devolver un texto plano plausible pero incorrecto.

Cuando una clave anterior descifra con éxito un wire, la llamada sigue
devolviendo `Ok(...)` - pero la línea `tracing::warn!` se dispara junto
a ella, así que las alertas basadas en logs detectan la cola de la
rotación antes de que se elimine `APP_KEY_PREVIOUS`.

## Siguiente

- [Configuración](configuration.md) - `APP_KEY`, `APP_ENV`, y el resto
  del entorno de arranque.
- [API de Eloquent](eloquent.md) - los casts `AsEncrypted*`, la tabla
  completa de casts, y el procedimiento de rotación para columnas de
  modelo.
- [Hashing](hashing.md) - la alternativa unidireccional para cuando
  necesitas *verificar* y no *recuperar*; las fachadas de bcrypt y
  Argon2id más `AsHashed`.
- [Flujos de autenticación](auth-flows.md) - el almacenamiento del
  secreto de 2FA y de los códigos de recuperación, que se apoyan en
  `Crypt` bajo sus propios propósitos.
- [Sesiones](session.md) - la cookie de sesión, cifrada y firmada por
  `Crypt` vía `CryptPurpose::Cookie`.
