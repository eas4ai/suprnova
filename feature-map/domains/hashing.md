# Feature map: `manual/hashing.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 47 checked.

## Rust API: suprnova

### `suprnova::hashing::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::HashConfig` · framework/src/hashing/config.rs:55 (also `suprnova::hashing::HashConfig`)
  - Public fields: `driver`, `rounds`, `memory`, `time`, `threads`, `verify_algorithm`
  - [ ] fn `suprnova::HashConfig::from_env` · framework/src/hashing/config.rs:96
- [ ] enum `suprnova::HashAlgorithm` · framework/src/hashing/config.rs:15 (also `suprnova::hashing::Algorithm`)
  - Variants: `Bcrypt`, `Argon2i`, `Argon2id`
  - [ ] fn `suprnova::HashAlgorithm::as_str` · framework/src/hashing/config.rs:31
  - [ ] fn `suprnova::HashAlgorithm::parse` · framework/src/hashing/config.rs:42

### `suprnova::hashing::driver` (private module; items are public through re-exports)

- [ ] struct `suprnova::Argon2idHasher` · framework/src/hashing/driver.rs:287 (also `suprnova::hashing::Argon2idHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::Argon2idHasher::new` · framework/src/hashing/driver.rs:296
  - [ ] fn `suprnova::Argon2idHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:307
- [ ] struct `suprnova::Argon2iHasher` · framework/src/hashing/driver.rs:226 (also `suprnova::hashing::Argon2iHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::Argon2iHasher::new` · framework/src/hashing/driver.rs:235
  - [ ] fn `suprnova::Argon2iHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:247
- [ ] struct `suprnova::Argon2Options` · framework/src/hashing/driver.rs:205 (also `suprnova::hashing::Argon2Options`)
  - Public fields: `memory`, `time`, `threads`
- [ ] struct `suprnova::BcryptHasher` · framework/src/hashing/driver.rs:98 (also `suprnova::hashing::BcryptHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::BcryptHasher::new` · framework/src/hashing/driver.rs:106
  - [ ] fn `suprnova::BcryptHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:114
- [ ] struct `suprnova::BcryptOptions` · framework/src/hashing/driver.rs:80 (also `suprnova::hashing::BcryptOptions`)
  - Public fields: `rounds`
- [ ] trait `suprnova::Hasher` · framework/src/hashing/driver.rs:18 (also `suprnova::hashing::Hasher`)
  - Implemented here by: `Argon2iHasher`, `Argon2idHasher`, `BcryptHasher`
  - [ ] fn `suprnova::Hasher::algorithm` · framework/src/hashing/driver.rs:22 (required)
  - [ ] fn `suprnova::Hasher::hash` · framework/src/hashing/driver.rs:29 (required)
  - [ ] fn `suprnova::Hasher::verify` · framework/src/hashing/driver.rs:43 (required)
  - [ ] fn `suprnova::Hasher::needs_rehash` · framework/src/hashing/driver.rs:51 (required)
  - [ ] fn `suprnova::Hasher::verify_algorithm` · framework/src/hashing/driver.rs:56 (provided)

### `suprnova::hashing::info` (private module; items are public through re-exports)

- [ ] fn `suprnova::is_hashed` · framework/src/hashing/info.rs:106 (also `suprnova::hashing::is_hashed`)
- [ ] fn `suprnova::hashing::parse` · framework/src/hashing/info.rs:84
- [ ] struct `suprnova::HashInfo` · framework/src/hashing/info.rs:20 (also `suprnova::hashing::HashInfo`)
  - Public fields: `algo`, `rounds`, `memory`, `time`, `threads`, `bcrypt_variant`
- [ ] enum `suprnova::hashing::AlgoName` · framework/src/hashing/info.rs:41
  - Variants: `Bcrypt`, `Argon2i`, `Argon2id`, `Argon2d`, `Unknown`
  - [ ] fn `suprnova::hashing::AlgoName::supported` · framework/src/hashing/info.rs:59
  - [ ] fn `suprnova::hashing::AlgoName::as_str` · framework/src/hashing/info.rs:69

### `suprnova::hashing`

- [ ] fn `suprnova::hashing::default_driver` · framework/src/hashing/mod.rs:133
- [ ] fn `suprnova::hash` · framework/src/hashing/mod.rs:171 (also `suprnova::hashing::hash`)
- [ ] fn `suprnova::hashing::hash_async` · framework/src/hashing/mod.rs:325
- [ ] fn `suprnova::hash_info` · framework/src/hashing/mod.rs:389 (also `suprnova::hashing::info`)
- [ ] fn `suprnova::hashing::hash_with` · framework/src/hashing/mod.rs:178
- [ ] fn `suprnova::hashing::hash_with_cost` · framework/src/hashing/mod.rs:195
- [ ] fn `suprnova::hashing::hash_with_cost_async` · framework/src/hashing/mod.rs:333
- [ ] fn `suprnova::needs_rehash` · framework/src/hashing/mod.rs:369 (also `suprnova::hashing::needs_rehash`)
- [ ] fn `suprnova::hashing::needs_rehash_with` · framework/src/hashing/mod.rs:378
- [ ] fn `suprnova::hashing::set_default_driver` · framework/src/hashing/mod.rs:154
- [ ] fn `suprnova::verify` · framework/src/hashing/mod.rs:227 (also `suprnova::hashing::verify`)
- [ ] fn `suprnova::hashing::verify_async` · framework/src/hashing/mod.rs:341
- [ ] fn `suprnova::hashing::verify_with` · framework/src/hashing/mod.rs:237
- [ ] const `suprnova::HASH_DEFAULT_COST` · framework/src/hashing/mod.rs:93 (also `suprnova::hashing::DEFAULT_COST`)
- [ ] const `suprnova::HASH_DEFAULT_ROUNDS` · framework/src/hashing/mod.rs:96 (also `suprnova::hashing::DEFAULT_ROUNDS`)
- [ ] const `suprnova::hashing::MAX_BCRYPT_COST` · framework/src/hashing/mod.rs:108
- [ ] const `suprnova::MAX_BCRYPT_PASSWORD_BYTES` · framework/src/hashing/mod.rs:119 (also `suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES`)
- [ ] const `suprnova::hashing::MAX_PASSWORD_BYTES` · framework/src/hashing/mod.rs:123
- [ ] const `suprnova::hashing::MIN_BCRYPT_COST` · framework/src/hashing/mod.rs:100
