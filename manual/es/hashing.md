# Hashing

El módulo `suprnova::hashing` es la superficie de hash de contraseñas del
framework, con tres drivers de primera clase - **bcrypt** (por defecto,
coincide con Laravel), **Argon2i** (memory-hard, resistente a canales
laterales) y **Argon2id** (recomendación de OWASP 2024). Úsalo para
almacenar contraseñas de usuario, hacer hash de los tokens verificadores
de "recuérdame", o en cualquier sitio donde la primitiva correcta sea
una función unidireccional. La selección de driver se controla por
variables de entorno, y la fachada conoce el algoritmo de punta a punta
(`info`, `is_hashed`, `needs_rehash`, `verify`), así que un hash bcrypt
ya almacenado sigue verificando después de cambiar a
`HASH_DRIVER=argon2id`.

## Panorama general

```rust
use suprnova::hashing;

// Async (preferible dentro de handlers de solicitud de Tokio - ejecuta
// el hash, que consume CPU, en spawn_blocking para que el hilo de
// worker quede libre):
let hashed = hashing::hash_async("my_password").await?;
let valid = hashing::verify_async("my_password", &hashed).await?;

// Sync (tests, herramientas de CLI, contextos no asíncronos):
let hashed = hashing::hash("my_password")?;
let valid = hashing::verify("my_password", &hashed)?;
```

La fachada de funciones libres lee el driver activo desde `HASH_DRIVER`
(o recurre a bcrypt si falta). Para llamadas con driver explícito,
construye el tipo del driver directamente y pásalo a `hash_with` /
`verify_with` / `needs_rehash_with`.

## Configuración

| Variable | Descripción | Por defecto | Rango |
|----------|-------------|---------|-------|
| `HASH_DRIVER` | Algoritmo activo | `bcrypt` | `bcrypt` \| `argon` \| `argon2i` \| `argon2id` |
| `HASH_ROUNDS` | Factor de costo de bcrypt | `12` | `4..=31` (solo bcrypt) |
| `HASH_MEMORY` | Costo de memoria de Argon en KiB | `65536` (64 MiB) | `>= 8` (solo argon) |
| `HASH_TIME` | Iteraciones de tiempo de Argon | `4` | `>= 1` (solo argon) |
| `HASH_THREADS` | Paralelismo / lanes de Argon | `1` | `>= 1` (solo argon) |
| `HASH_VERIFY` | Si es true, `verify()` rechaza hashes de otro algoritmo | `false` | `true` / `false` |

Una mala configuración (un valor incorrecto, un parámetro fuera de
rango) emerge como un `FrameworkError::param` en la primera llamada a
`hash` / `verify` / `needs_rehash` - no como un valor por defecto
silencioso.

### `.env` de ejemplo para argon2id

```env
HASH_DRIVER=argon2id
HASH_MEMORY=65536
HASH_TIME=4
HASH_THREADS=1
```

### Por qué los valores por defecto de Argon2 de Suprnova son más fuertes que los de Laravel

| Parámetro | Por defecto en Laravel | Por defecto en Suprnova | Fuente |
|-------|-----------------|------------------|--------|
| Memoria | 1 024 KiB (1 MiB) | 65 536 KiB (64 MiB) | OWASP 2024 |
| Tiempo | 2 iteraciones | 4 iteraciones | OWASP 2024 |
| Hilos | 2 | 1 | OWASP 2024 / alineado con libsodium |

Los valores por defecto de Laravel asumen el modelo de PHP de una
solicitud por proceso - un worker solo puede gastar cierto tiempo en
cada hash de contraseña antes de saturar la máquina. El
`spawn_blocking` de Tokio le permite a Suprnova entregar el hash a un
pool de hilos bloqueantes sin congelar el bucle de solicitudes, así que
las cifras de OWASP 2024 son realistas en hardware de producción real.

## Drivers

### Bcrypt (por defecto)

```rust
use suprnova::hashing::{BcryptHasher, BcryptOptions, hash_with, verify_with};

let driver = BcryptHasher::new(BcryptOptions { rounds: 14 });
let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);
```

Bcrypt tiene un **tope de tamaño de bloque de 72 bytes** en la entrada
de contraseña - la primitiva subyacente trunca en silencio las entradas
más largas, lo que significa que dos passphrases distintas que
comparten sus primeros 72 bytes hacen hash al mismo valor. Suprnova
rechaza por adelantado (la ruta bcrypt del framework falla en `hash()`
y devuelve `Ok(false)` en `verify()` para contraseñas sobredimensionadas,
manteniendo uniforme la respuesta de "credenciales inválidas" del flujo
de autenticación). Argon2 no tiene ese techo.

El tope de bcrypt se expone como
`suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES` (71 - el límite utilizable
después del terminador nulo de bcrypt).

### Argon2id (recomendación de OWASP 2024)

```rust
use suprnova::hashing::{Argon2idHasher, Argon2Options, hash_with, verify_with};

let driver = Argon2idHasher::new(Argon2Options {
    memory: 65_536,  // 64 MiB
    time: 4,
    threads: 1,
})?;

let hashed = hash_with(&driver, "my_password")?;
assert!(verify_with(&driver, "my_password", &hashed)?);

// Argon2 acepta passphrases de longitud arbitraria - el tope de 72
// bytes de bcrypt no aplica.
let long = "x".repeat(500);
let h = hash_with(&driver, &long)?;
assert!(verify_with(&driver, &long, &h)?);
```

### Argon2i

Misma forma que Argon2id; `Argon2iHasher::new(opts)`. Usa Argon2id para
proyectos nuevos - Argon2i se mantiene por paridad, pero Argon2id es la
recomendación moderna.

## Bcrypt con un costo explícito (`hash_with_cost`)

`hash_with_cost(password, cost)` y
`hash_with_cost_async(password, cost)` acuñan un hash bcrypt a un factor
de costo proporcionado por quien llama, sin importar `HASH_DRIVER`.
Úsalos cuando una política o una configuración por tenant hacen fluir
un costo hacia el sitio de la llamada en lugar de hacia el entorno del
proceso - por ejemplo, una clase de cuenta de alta seguridad que usa el
costo 14 mientras el resto de la app corre en el 12 por defecto.

```rust
use suprnova::hashing::{hash_with_cost, hash_with_cost_async};

// Sync - tests, herramientas de CLI.
let h = hash_with_cost("my_password", 14)?;

// Async - dentro de handlers de solicitud de Tokio.
let h = hash_with_cost_async("my_password", 14).await?;
```

Ambos puntos de entrada rechazan un `cost` fuera de
`MIN_BCRYPT_COST..=MAX_BCRYPT_COST` (`4..=31`) con
`FrameworkError::param`, reflejando la misma validación que
`HASH_ROUNDS` aplica del lado del entorno:

```rust
use suprnova::hashing::{hash_with_cost, MIN_BCRYPT_COST, MAX_BCRYPT_COST};

assert!(hash_with_cost("pw", MIN_BCRYPT_COST - 1).is_err()); // < 4
assert!(hash_with_cost("pw", MAX_BCRYPT_COST + 1).is_err()); // > 31
```

La comprobación de límites importa porque cada incremento de costo
duplica el tiempo de CPU. A costo 31, un solo hash bcrypt tarda horas en
hardware de consumo - comprobar los límites dentro del framework evita
que una errata de política o configuración inmovilice por accidente un
hilo de worker durante el resto del día. La variante async pasa por
`spawn_blocking`, así que ni siquiera un costo legítimamente alto
congela el bucle de solicitudes.

## `needs_rehash` con conocimiento del algoritmo

`needs_rehash` devuelve `true` cuando el hash almacenado debería volver
a hacerse bajo el driver activo. Cubre tres casos:

1. **Desajuste de algoritmo** - un hash bcrypt almacenado mientras
   `HASH_DRIVER=argon2id` (o viceversa). Dispara una rotación en la
   siguiente verificación con éxito.
2. **Debilidad de parámetros** - un costo de bcrypt por debajo de
   `HASH_ROUNDS`, o `m`/`t`/`p` de argon por debajo de
   `HASH_MEMORY`/`HASH_TIME`/`HASH_THREADS`.
3. **Variantes heredadas de bcrypt** - `$2a$`, `$2x$`, `$2y$` rotan al
   `$2b$` canónico incluso al costo configurado.

```rust
if hashing::needs_rehash(&stored_hash) {
    let fresh = hashing::hash_async("plaintext_at_login").await?;
    // Persiste `fresh`. Es el patrón estándar de Laravel "rehacer el
    // hash en un login exitoso"; funciona entre algoritmos.
}
```

Una entrada malformada devuelve `true` - quien llama rota de forma
natural cualquier cosa que no pueda analizar.

## Inspección de hashes (`info` + `is_hashed`)

```rust
use suprnova::hashing::{info, is_hashed};

let h = hashing::hash_async("my_password").await?;
let i = info(&h);
println!("algo: {}", i.algo.as_str());
println!("bcrypt cost: {:?}", i.rounds);
println!("argon memory KiB: {:?}", i.memory);

// True para cualquier hash de algoritmo reconocido; false para
// texto plano / basura.
assert!(is_hashed(&h));
assert!(!is_hashed("plaintext"));
```

`info().algo` es uno de: `Bcrypt`, `Argon2i`, `Argon2id`, `Argon2d`
(reconocido pero nunca acuñado), `Unknown`.

`is_hashed` es lo que usa el cast eloquent `AsHashed` para saltarse
volver a hacer el hash de una columna que ya lo tiene - funciona en los
tres drivers, así que cambiar `HASH_DRIVER` a mitad de proyecto no
provoca un bucle de hash-sobre-hash en el siguiente guardado.

## Compuerta de verificación entre algoritmos (`HASH_VERIFY`)

Por defecto, `verify()` comprueba la contraseña contra el hash sin
importar qué algoritmo produjo ese hash - esto es lo que permite que los
hashes bcrypt heredados sigan verificando después de cambiar a
`HASH_DRIVER=argon2id` (para poder rotarlos en el login). Establece
`HASH_VERIFY=true` una vez que cada usuario esté rotado, para exigir el
algoritmo activo de forma estricta:

```env
HASH_VERIFY=true
```

Con la compuerta activada, `verify()` devuelve `Ok(false)` para
cualquier hash cuyo algoritmo difiera del driver activo - la misma
forma que el `RuntimeException` de Laravel, pero Suprnova devuelve
false en lugar de lanzar una excepción, porque quien llama desde el
flujo de autenticación espera un `Result<bool>` en cualquier caso.

## Async frente a sync

Tanto bcrypt a costo 12 (~250 ms) como Argon2id con memory=64 MiB
(~80 ms) son deliberadamente intensivos en CPU - ese es todo el
sentido de un hash lento. Llamar directamente a `hash` / `verify` en su
forma sync desde un handler de solicitud de Tokio bloquea el hilo de
worker durante toda la duración del hash, dejando en inanición a otras
solicitudes en ese mismo worker.

Usa las variantes `*_async` dentro de handlers `async fn`. Envuelven la
llamada intensiva en CPU en `tokio::task::spawn_blocking`, así que el
worker queda libre para otras solicitudes:

```rust
// BIEN - dentro de un handler async
let hashed = hashing::hash_async(&form.password).await?;

// MAL - bloquea el worker durante ~250 ms
let hashed = hashing::hash(&form.password)?;
```

Las variantes sync son para tests, herramientas de CLI y otros
contextos no asíncronos donde bloquear no tiene costo.

## Integración con Eloquent: el cast `AsHashed`

El cast eloquent `#[cast(AsHashed)]` hace el hash de un campo en texto
plano al escribir, usando el driver activo, y es **idempotente en
todos los drivers** - guardar un modelo cuya columna `password` ya
contiene un hash reconocido (bcrypt o argon) deja el valor sin cambios.
Sin esta salvaguarda, `User::find(id).await?.save().await?` haría el
hash del hash existente en cada guardado, rompiendo la autenticación.

```rust
use suprnova::eloquent::casts::AsHashed;

#[suprnova::model]
struct User {
    #[cast(AsHashed)]
    pub password: String,
    // ...
}
```

La comprobación de idempotencia usa `hashing::is_hashed`, así que
cambiar `HASH_DRIVER` a mitad de proyecto es seguro - tanto los hashes
bcrypt heredados como los hashes argon2id nuevos se reconocen y se
saltan al volver a guardar.

## Uso con `Auth::attempt`

`Auth::attempt(&credentials)` llama a
`UserProvider::validate_credentials`, que a su vez llama a
`hashing::verify_async` contra el hash almacenado del usuario. Verify
despacha según el algoritmo del hash *almacenado*, no según el driver
configurado - así que después de cambiar a `HASH_DRIVER=argon2id`, cada
hash bcrypt existente sigue verificando, y `needs_rehash` devuelve
`true`, de modo que el patrón estándar de rotar-en-el-login lleva a la
base de usuarios hacia el nuevo algoritmo un login a la vez.

## Sobrescribir el driver en tests

`set_default_driver(Box<dyn Hasher>)` instala un driver de forma
programática para tests y herramientas de CLI embebidas que construyen
el driver sin pasar por `HASH_DRIVER`. Es de una sola vez - la primera
llamada gana, y una segunda llamada devuelve `FrameworkError::internal`
en lugar de reemplazar el driver a mitad de proceso. Úsalo al arrancar
la suite, antes de que ninguna ruta de código resuelva el valor por
defecto.

## Siguiente

- [Autenticación](authentication.md) - `Auth::attempt`, el trait de
  proveedor de usuario, y cómo se integra el hashing con el login
- [Flujos de autenticación](auth-flows.md) - `PasswordReset::complete`
  rota el hash de contraseña almacenado a través del driver activo; los
  tokens de "recuérdame" se hashean antes de almacenarse vía
  `hash_async`
- [Eloquent](eloquent.md) - referencia de `#[cast(AsHashed)]` y la
  superficie más amplia de casts
- [Cifrado](encryption.md) - cifrado autenticado bidireccional para
  datos en reposo; el complemento del hashing unidireccional
- [Modelo de errores](error-model.md) - cómo se ve un
  `FrameworkError::param` cuando se rechaza un valor de configuración
  de hashing
