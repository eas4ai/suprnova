# Third-party licenses

Suprnova Live is licensed under MIT. For Cargo, this generated inventory covers
the conservative dependency closure reachable from the four Live package roots
in the shared Suprnova resolution, plus the separately resolved fuzz and compile
fixture roots. Unrelated parent-workspace roots and their unreachable
dependencies are excluded. Regenerate it with
`rtk node scripts/generate-license-inventory.mjs`; the unattended gate uses
`--check` to reject lockfile or license drift.

Cargo feature unification is shared-workspace-wide, so this conservative closure
can include optional dependency edges enabled elsewhere in the workspace. A
`Workspace resolved` row records reachability in those resolved graphs; it does
not claim exact `cargo tree` use by every Live build.

For npm, usage is derived transitively from the exact root dependency graph.
Production runtime takes precedence over production build, test-only, and
development-tooling reachability. The production asset manifest and JavaScript
banner separately retain Idiomorph's name, version, and 0BSD license metadata.

| Ecosystem | Package | Version | Usage | License | Locked source |
|---|---|---:|---|---|---|
| Cargo | adler2 | 2.0.1 | Workspace resolved | 0BSD OR MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aead | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aes-gcm | 0.10.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aes | 0.8.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ahash | 0.7.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ahash | 0.8.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aho-corasick | 1.1.4 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aho-corasick | 1.1.5 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aliasable | 0.1.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | allocator-api2 | 0.2.21 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ammonia | 4.1.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | android_system_properties | 0.1.5 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anstream | 1.0.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anstyle-parse | 1.0.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anstyle-query | 1.1.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anstyle-wincon | 3.0.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anstyle | 1.0.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | anyhow | 1.0.103 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | apple | 0.3.1 | Workspace resolved | MIT | git+https://github.com/eas4ai/suprnova-apple-rs.git?tag=v0.3.1#bc969e97400663702c97381504b1d597d238b1a8 |
| Cargo | arbitrary | 1.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arc-swap | 1.9.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arcstr | 1.2.0 | Workspace resolved | Apache-2.0 OR MIT OR Zlib | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | argon2 | 0.5.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrayvec | 0.7.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-arith | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-array | 58.4.0 | Workspace resolved | Apache-2.0 AND MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-buffer | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-cast | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-data | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-ord | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-row | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-schema | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-select | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow-string | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | arrow | 58.4.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_derive | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_macros | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_parser | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | asn1-rs-derive | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | asn1-rs-impl | 0.2.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | asn1-rs | 0.6.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | assert-json-diff | 2.0.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | async-lock | 3.4.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | async-stream-impl | 0.3.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | async-stream | 0.3.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | async-trait | 0.1.92 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | atoi | 2.0.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | atomic-waker | 1.1.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | atomic | 0.6.1 | Workspace resolved | Apache-2.0/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | autocfg | 1.5.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-credential-types | 1.2.14 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-lc-rs | 1.17.0 | Workspace resolved | ISC AND (Apache-2.0 OR ISC) | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-lc-sys | 0.41.0 | Workspace resolved | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-sigv4 | 1.4.4 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-smithy-async | 1.2.14 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-smithy-http | 0.63.6 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-smithy-runtime-api-macros | 1.0.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-smithy-runtime-api | 1.12.1 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | aws-smithy-types | 1.4.8 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | axum-core | 0.4.5 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | axum | 0.7.9 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | backon | 1.6.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base16ct | 0.2.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base32 | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64-simd | 0.8.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64 | 0.21.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64 | 0.22.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64ct | 1.8.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64urlsafedata | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | basic-toml | 0.1.10 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bcrypt | 0.17.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bcrypt | 0.19.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bigdecimal | 0.4.10 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bincode | 1.3.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bit-set | 0.8.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bit-vec | 0.8.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bitflags | 1.3.2 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bitflags | 2.11.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bitflags | 2.13.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bitvec | 1.0.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | blake2 | 0.10.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | block-buffer | 0.10.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | block-buffer | 0.12.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | block-buffer | 0.12.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | blowfish | 0.10.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | blowfish | 0.9.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | borsh-derive | 1.6.1 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | borsh | 1.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bstr | 1.12.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bumpalo | 3.20.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytecheck_derive | 0.6.12 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytecheck | 0.6.12 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytemuck | 1.25.0 | Workspace resolved | Zlib OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | byteorder-lite | 0.1.0 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | byteorder | 1.5.0 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytes-utils | 0.1.4 | Workspace resolved | Apache-2.0/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytes | 1.11.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | calendrical_calculations | 0.2.4 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | caseless | 0.2.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cc | 1.2.62 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cc | 1.4.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cfb | 0.7.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cfg_aliases | 0.2.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cfg-if | 1.0.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | chacha20 | 0.10.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | chrono-tz-build | 0.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | chrono-tz | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | chrono | 0.4.44 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cipher | 0.4.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cipher | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | clap_builder | 4.6.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | clap_derive | 4.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | clap_lex | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | clap | 4.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cmake | 0.1.58 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cmov | 0.5.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cmov | 0.5.4 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | colorchoice | 1.0.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | combine | 4.6.7 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | compcol | 0.6.10 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | comrak | 0.52.0 | Workspace resolved | BSD-2-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | concurrent-queue | 2.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | const-oid | 0.10.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | const-oid | 0.9.6 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | const-random-macro | 0.1.16 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | const-random | 0.1.18 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | constant_time_eq | 0.3.1 | Workspace resolved | CC0-1.0 OR MIT-0 OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | core_maths | 0.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | core-foundation-sys | 0.8.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | core-foundation | 0.10.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | core-foundation | 0.9.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cpufeatures | 0.2.17 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cpufeatures | 0.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cpufeatures | 0.3.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crc-catalog | 2.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crc-fast | 1.10.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crc | 3.4.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crc32fast | 1.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crossbeam-channel | 0.5.15 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crossbeam-deque | 0.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crossbeam-epoch | 0.9.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crossbeam-queue | 0.3.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crossbeam-utils | 0.8.21 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crunchy | 0.2.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crypto-bigint | 0.5.5 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crypto-common | 0.1.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crypto-common | 0.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cssparser | 0.37.0 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ctr | 0.9.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ctutils | 0.4.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | darling_core | 0.20.11 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | darling_macro | 0.20.11 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | darling | 0.20.11 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dashmap | 5.5.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dashmap | 6.2.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | data-encoding | 2.11.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | deadpool-runtime | 0.1.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | deadpool | 0.12.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | defmt-macros | 1.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | defmt-parser | 1.0.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | defmt | 1.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | der-parser | 9.0.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | der | 0.7.10 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | deranged | 0.5.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive_builder_core | 0.20.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive_builder_macro | 0.20.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive_builder | 0.20.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive_more-impl | 2.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive_more | 2.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | derive-where | 1.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | deunicode | 1.6.2 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | digest | 0.10.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | digest | 0.11.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | displaydoc | 0.2.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dlv-list | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dotenvy | 0.15.7 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dtoa-short | 0.3.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dtoa | 1.0.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dummy | 0.12.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | dunce | 1.0.5 | Workspace resolved | CC0-1.0 OR MIT-0 OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ecdsa | 0.16.9 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ece | 2.3.1 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | either | 1.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | elliptic-curve | 0.13.8 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | email_address | 0.2.9 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | email-encoding | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | encoding_rs | 0.8.35 | Workspace resolved | (Apache-2.0 OR MIT) AND BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | entities | 1.0.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | envy | 0.4.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | equivalent | 1.0.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | errno | 0.3.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | etcetera | 0.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | event-listener-strategy | 0.5.4 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | event-listener | 5.4.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fake | 5.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fancy-regex | 0.16.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fastrand | 1.9.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fastrand | 2.4.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fastrand | 2.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fdeflate | 0.3.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | featureflag | 0.0.3 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ff | 0.13.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | filetime | 0.2.29 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | find-msvc-tools | 0.1.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | find-msvc-tools | 0.1.9 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | finl_unicode | 1.4.0 | Workspace resolved | (MIT OR Apache-2.0) AND Unicode-DFS-2016 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fixed_decimal | 0.7.2 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | flate2 | 1.1.9 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fluent-bundle | 0.16.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fluent-langneg | 0.13.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fluent-langneg | 0.14.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fluent-syntax | 0.12.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | flume | 0.11.1 | Workspace resolved | Apache-2.0/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | flume | 0.12.0 | Workspace resolved | Apache-2.0/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fnv | 1.0.7 | Workspace resolved | Apache-2.0 / MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | foldhash | 0.2.0 | Workspace resolved | Zlib | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | foreign-types-shared | 0.1.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | foreign-types | 0.3.2 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | form_urlencoded | 1.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fs_extra | 1.3.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fsevent-sys | 4.1.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | funty | 2.0.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-channel | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-core | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-executor | 0.3.32 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-intrusive | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-io | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-macro | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-sink | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-task | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-util | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures | 0.3.32 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | generic-array | 0.14.7 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | getrandom | 0.2.17 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | getrandom | 0.3.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | getrandom | 0.4.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ghash | 0.5.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | glob | 0.3.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | glob | 0.3.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | globset | 0.4.18 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | globwalk | 0.9.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | gloo-timers | 0.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | group | 0.13.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | h2 | 0.4.16 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | half | 2.7.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashbrown | 0.12.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashbrown | 0.14.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashbrown | 0.16.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashbrown | 0.17.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashlink | 0.11.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | heck | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | heck | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hermit-abi | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hex-literal | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hex | 0.4.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hkdf | 0.12.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hkdf | 0.13.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hmac | 0.12.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hmac | 0.13.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hostname | 0.4.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | html5ever | 0.39.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http-body-util | 0.1.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http-body | 0.4.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http-body | 1.0.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http | 0.2.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http | 1.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | httparse | 1.10.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | httpdate | 1.0.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | humansize | 2.1.3 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hybrid-array | 0.4.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hybrid-array | 0.4.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hyper-rustls | 0.27.9 | Workspace resolved | Apache-2.0 OR ISC OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hyper-timeout | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hyper-tungstenite | 0.20.0 | Workspace resolved | BSD-2-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hyper-util | 0.1.20 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hyper | 1.9.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | iana-time-zone-haiku | 0.1.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | iana-time-zone | 0.1.65 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_calendar_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_calendar | 2.2.1 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_casemap_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_casemap | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_collections | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_datetime_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_datetime | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_decimal_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_decimal | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_experimental_data | 0.5.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_experimental | 0.5.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_list_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_list | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_locale_core | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_locale_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_locale | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_locid | 1.5.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_normalizer_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_normalizer | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_pattern | 0.4.2 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_plurals_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_plurals | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_properties_data | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_properties | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_provider | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_time_data | 2.2.1 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | icu_time | 2.2.0 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ident_case | 1.0.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | idna_adapter | 1.2.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | idna | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ignore | 0.4.25 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | image | 0.25.10 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | imagesize | 0.15.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | indexmap | 1.9.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | indexmap | 2.14.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | indoc | 2.0.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | infer | 0.19.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | inotify-sys | 0.1.5 | Workspace resolved | ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | inotify | 0.9.6 | Workspace resolved | ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | inout | 0.1.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | inout | 0.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | instant | 0.1.13 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | intl_pluralrules | 7.0.2 | Workspace resolved | Apache-2.0/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | intl-memoizer | 0.5.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | inventory | 0.3.24 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ipnet | 2.12.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | is_terminal_polyfill | 1.70.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | iso_country | 0.1.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | iso_currency | 0.5.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | itertools | 0.14.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | itoa | 1.0.18 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ixdtf | 0.6.5 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jetscii | 0.5.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jiff-static | 0.2.32 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jiff-tzdb-platform | 0.1.3 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jiff-tzdb | 0.1.6 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jiff | 0.2.32 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jni-macros | 0.22.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jni-sys-macros | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jni-sys | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jni | 0.22.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jobserver | 0.1.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jobserver | 0.1.35 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | js-sys | 0.3.104 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | js-sys | 0.3.99 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jsonwebtoken | 9.3.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | kqueue-sys | 1.1.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | kqueue | 1.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lazy_static | 1.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lettre | 0.11.22 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-core | 1.0.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-parse-float | 1.0.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-parse-integer | 1.0.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-util | 1.0.7 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-write-float | 1.0.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lexical-write-integer | 1.0.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libc | 0.2.186 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libc | 0.2.189 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libfuzzer-sys | 0.4.10 | Workspace resolved | (MIT OR Apache-2.0) AND NCSA | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libm | 0.2.16 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libsqlite3-sys | 0.30.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libz-sys | 1.1.28 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | linked-hash-map | 0.5.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | linux-raw-sys | 0.12.1 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | litemap | 0.7.5 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | litemap | 0.8.2 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lock_api | 0.4.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | log | 0.4.33 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | log | 0.4.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lru-slab | 0.1.2 | Workspace resolved | MIT OR Apache-2.0 OR Zlib | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mac_address | 1.1.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | maplit | 1.0.2 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | markup5ever | 0.39.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | matchers | 0.2.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | matchit | 0.7.3 | Workspace resolved | MIT AND BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | matchit | 0.9.2 | Workspace resolved | MIT AND BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | md-5 | 0.10.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | md-5 | 0.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mea | 0.6.3 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | memchr | 2.8.0 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | memchr | 2.8.3 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | memoffset | 0.9.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mime_guess | 2.0.5 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mime | 0.3.17 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | minimal-lexical | 0.2.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | miniz_oxide | 0.8.9 | Workspace resolved | MIT OR Zlib OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mio | 0.8.11 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | mio | 1.2.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | moxcms | 0.8.1 | Workspace resolved | BSD-3-Clause OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | multer | 3.1.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | new_debug_unreachable | 1.0.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | nix | 0.29.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | nom | 7.1.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | nom | 8.0.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | notify | 6.1.1 | Workspace resolved | CC0-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | nu-ansi-term | 0.50.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num_cpus | 1.17.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num_enum_derive | 0.7.6 | Workspace resolved | BSD-3-Clause OR MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num_enum | 0.7.6 | Workspace resolved | BSD-3-Clause OR MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-bigint | 0.4.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-complex | 0.4.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-conv | 0.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-derive | 0.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-integer | 0.1.46 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-rational | 0.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-traits | 0.2.19 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oid-registry | 0.7.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | once_cell_polyfill | 1.70.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | once_cell | 1.21.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | onig_sys | 69.9.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | onig | 6.5.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | opaque-debug | 0.3.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | opendal-core | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-logging | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-observe-metrics-common | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-prometheus-client | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-retry | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-timeout | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-layer-tracing | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-service-fs | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal-service-s3 | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | opendal | 0.58.0 | Workspace resolved | Apache-2.0 | git+https://github.com/eas4ai/opendal.git?rev=88717391eb72c9839d3f8e79fccad9f22fc3a1b4#88717391eb72c9839d3f8e79fccad9f22fc3a1b4 |
| Cargo | openssl-macros | 0.1.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | openssl-probe | 0.2.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | openssl-sys | 0.9.116 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | openssl | 0.10.80 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ordered-float | 4.6.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ordered-multimap | 0.7.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ouroboros_macro | 0.18.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ouroboros | 0.18.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | outref | 0.5.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-bmp | 0.1.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-core | 0.1.34 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-gif | 0.0.11 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-image-filter | 0.1.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-mjpeg | 0.1.8 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-pixfmt | 0.1.7 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-png | 0.1.8 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-vp8 | 0.2.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | oxideav-webp | 0.2.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | p256 | 0.13.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parking_lot_core | 0.9.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parking_lot | 0.12.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parking | 2.2.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parse-zoneinfo | 0.3.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | password-hash | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pastey | 0.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pem-rfc7468 | 0.7.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pem | 3.0.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | percent-encoding | 2.3.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pest_derive | 2.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pest_generator | 2.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pest_meta | 2.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pest | 2.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pgvector | 0.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_codegen | 0.11.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_codegen | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_generator | 0.11.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_generator | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_shared | 0.11.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_shared | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf | 0.11.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pin-project-internal | 1.1.13 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pin-project-lite | 0.2.17 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pin-project | 1.1.13 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pin-utils | 0.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pkcs8 | 0.10.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pkg-config | 0.3.33 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | plist | 1.10.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pluralizer | 0.5.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | png | 0.18.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | polyval | 0.6.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | portable-atomic-util | 0.2.7 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | portable-atomic | 1.13.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | potential_utf | 0.1.5 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | powerfmt | 0.2.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ppv-lite86 | 0.2.21 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | precomputed-hash | 0.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | primeorder | 0.13.6 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro-crate | 3.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro-error-attr2 | 2.0.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro-error2 | 2.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro-hack | 0.5.20+deprecated | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro2-diagnostics | 0.10.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro2 | 1.0.106 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro2 | 1.0.107 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | prometheus-client-derive-encode | 0.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | prometheus-client | 0.25.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proptest | 1.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | prost-derive | 0.13.5 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | prost-types | 0.13.5 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | prost | 0.13.5 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ptr_meta_derive | 0.1.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ptr_meta | 0.1.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pxfm | 0.1.29 | Workspace resolved | BSD-3-Clause OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | qdrant-client | 1.18.0 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | qrcodegen-image | 1.5.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | qrcodegen | 1.8.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quick-error | 1.2.3 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quick-xml | 0.41.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quinn-proto | 0.11.15 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quinn-udp | 0.5.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quinn | 0.11.9 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quote | 1.0.45 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quote | 1.0.47 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quoted_printable | 0.5.2 | Workspace resolved | 0BSD | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | r-efi | 5.3.0 | Workspace resolved | MIT OR Apache-2.0 OR LGPL-2.1-or-later | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | r-efi | 6.0.0 | Workspace resolved | MIT OR Apache-2.0 OR LGPL-2.1-or-later | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | radium | 0.7.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_chacha | 0.3.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_chacha | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_core | 0.10.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_core | 0.6.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_core | 0.9.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_xorshift | 0.4.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand | 0.10.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand | 0.8.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand | 0.9.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rdkafka-sys | 4.10.0+2.12.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rdkafka | 0.36.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | redis | 0.25.5 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | redis | 1.2.1 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | redox_syscall | 0.5.18 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex-automata | 0.4.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex-automata | 0.4.18 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex-syntax | 0.8.10 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex-syntax | 0.8.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex | 1.12.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex | 1.13.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rend | 0.4.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | reqsign-aws-v4 | 3.0.2 | Workspace resolved | Apache-2.0 | git+https://github.com/apache/opendal-reqsign.git?rev=b49cd2996b9d2d9944e84481f8835ff55b188b97#b49cd2996b9d2d9944e84481f8835ff55b188b97 |
| Cargo | reqsign-core | 3.1.0 | Workspace resolved | Apache-2.0 | git+https://github.com/apache/opendal-reqsign.git?rev=b49cd2996b9d2d9944e84481f8835ff55b188b97#b49cd2996b9d2d9944e84481f8835ff55b188b97 |
| Cargo | reqsign-file-read-tokio | 3.0.2 | Workspace resolved | Apache-2.0 | git+https://github.com/apache/opendal-reqsign.git?rev=b49cd2996b9d2d9944e84481f8835ff55b188b97#b49cd2996b9d2d9944e84481f8835ff55b188b97 |
| Cargo | reqwest | 0.12.28 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | reqwest | 0.13.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rfc6979 | 0.4.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ring | 0.17.14 | Workspace resolved | Apache-2.0 AND ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rkyv_derive | 0.7.46 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rkyv | 0.7.46 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rust_decimal | 1.42.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rust-ini | 0.21.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustc_version | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustc-hash | 2.1.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustc-hash | 2.1.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rusticata-macros | 4.1.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustix | 1.1.4 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-native-certs | 0.8.3 | Workspace resolved | Apache-2.0 OR ISC OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-pemfile | 2.2.0 | Workspace resolved | Apache-2.0 OR ISC OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-pki-types | 1.14.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-platform-verifier-android | 0.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-platform-verifier | 0.7.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls-webpki | 0.103.13 | Workspace resolved | ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustls | 0.23.40 | Workspace resolved | Apache-2.0 OR ISC OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustversion | 1.0.22 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustversion | 1.0.23 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rusty-fork | 0.3.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ryu-js | 1.0.3 | Workspace resolved | Apache-2.0 OR BSL-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ryu | 1.0.23 | Workspace resolved | Apache-2.0 OR BSL-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | same-file | 1.0.6 | Workspace resolved | Unlicense/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | schannel | 0.1.29 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | scopeguard | 1.2.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-bae | 0.2.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-orm-arrow | 2.0.0-rc.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-orm-cli | 2.0.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-orm-macros | 2.0.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-orm-migration | 2.0.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-orm | 2.0.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-query-derive | 1.0.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-query-sqlx | 0.9.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-query | 1.0.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-schema-derive | 0.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-schema | 0.18.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-file | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-kafka | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-redis | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-runtime | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-socket | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-stdio | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer-types | 0.5.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sea-streamer | 0.5.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | seahash | 4.1.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sec1 | 0.7.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | secrecy | 0.10.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | security-framework-sys | 2.17.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | security-framework | 3.7.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | self_cell | 1.3.0 | Workspace resolved | Apache-2.0 OR GPL-2.0-only | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | semver | 1.0.28 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_bytes | 0.11.19 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_cbor_2 | 0.13.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_core | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_derive | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_json_canonicalizer | 0.3.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_json | 1.0.151 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_path_to_error | 0.1.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_spanned | 1.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_urlencoded | 0.7.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serial_test_derive | 2.0.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serial_test | 2.0.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha1_smol | 1.0.1 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha1 | 0.10.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha1 | 0.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha2 | 0.10.9 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha2 | 0.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sharded-slab | 0.1.7 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | shlex | 1.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | shlex | 2.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | signal-hook-registry | 1.4.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | signature | 2.2.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | simd_cesu8 | 1.1.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | simd-adler32 | 0.3.9 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | simdutf8 | 0.1.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | simple_asn1 | 0.6.4 | Workspace resolved | ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | siphasher | 1.0.3 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | slab | 0.4.12 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | slug | 0.1.6 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | smallvec | 1.15.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | smallvec | 1.15.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | socket2 | 0.5.10 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | socket2 | 0.6.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | spin | 0.10.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | spin | 0.9.8 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | spki | 0.7.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-core | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-macros-core | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-macros | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-mysql | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-postgres | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx-sqlite | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sqlx | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | stable_deref_trait | 1.2.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | static_assertions | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | string_cache_codegen | 0.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | string_cache | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | stringprep | 0.1.5 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | strsim | 0.11.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | strum_macros | 0.28.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | strum | 0.28.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | subtle | 2.6.1 | Workspace resolved | BSD-3-Clause | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 1.0.109 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 2.0.117 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 2.0.119 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 3.0.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 3.0.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sync_wrapper | 1.0.2 | Workspace resolved | Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | synstructure | 0.13.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syntect | 5.3.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | system-configuration-sys | 0.6.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | system-configuration | 0.7.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tap | 1.0.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | target-triple | 1.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tempfile | 3.27.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tendril | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tera | 1.20.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | termcolor | 1.4.1 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror-impl | 1.0.69 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror-impl | 2.0.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror | 1.0.69 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror | 2.0.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thread_local | 1.1.9 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time-core | 0.1.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time-macros | 0.2.27 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time | 0.3.47 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tiny-keccak | 2.0.2 | Workspace resolved | CC0-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tinystr | 0.7.6 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tinystr | 0.8.3 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tinyvec_macros | 0.1.1 | Workspace resolved | MIT OR Apache-2.0 OR Zlib | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tinyvec | 1.11.0 | Workspace resolved | Zlib OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-macros | 2.7.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-rustls | 0.26.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-stream | 0.1.18 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-tungstenite | 0.24.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-tungstenite | 0.29.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-util | 0.7.18 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio | 1.53.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_datetime | 1.1.1+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_edit | 0.25.11+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_parser | 1.1.2+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_writer | 1.1.1+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml | 1.1.2+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tonic | 0.12.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | totp-rs | 5.7.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tower-http | 0.6.11 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tower-layer | 0.3.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tower-service | 0.3.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tower | 0.4.13 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tower | 0.5.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-attributes | 0.1.31 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-core | 0.1.36 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-log | 0.2.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-serde | 0.2.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-subscriber | 0.3.23 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-test-macro | 0.2.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing-test | 0.2.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tracing | 0.1.44 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | try-lock | 0.2.5 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | trybuild | 1.0.120 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tungstenite | 0.24.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tungstenite | 0.29.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | type-map | 0.5.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | typed-arena | 2.0.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | typenum | 1.20.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | typenum | 1.20.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ucd-trie | 0.1.7 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unarray | 0.1.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unic-langid-impl | 0.9.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unic-langid-macros-impl | 0.9.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unic-langid-macros | 0.9.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unic-langid | 0.9.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicase | 2.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-bidi | 0.3.18 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-ident | 1.0.24 | Workspace resolved | (MIT OR Apache-2.0) AND Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-normalization | 0.1.25 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-properties | 0.1.4 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-segmentation | 1.13.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-xid | 0.2.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | universal-hash | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | untrusted | 0.9.0 | Workspace resolved | ISC | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | url | 2.5.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | urlencoding | 2.1.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | utf-8 | 0.7.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | utf8_iter | 1.0.4 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | utf8parse | 0.2.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | uuid | 1.23.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | validator_derive | 0.20.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | validator | 0.20.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | valuable | 0.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | vcpkg | 0.2.15 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | version_check | 0.9.5 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | vsimd | 0.8.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wait-timeout | 0.2.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | walkdir | 2.5.0 | Workspace resolved | Unlicense/MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | want | 0.3.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasi | 0.11.1+wasi-snapshot-preview1 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasip2 | 1.0.3+wasi-0.2.9 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-futures | 0.4.72 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro-support | 0.2.122 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro-support | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro | 0.2.122 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-shared | 0.2.122 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-shared | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen | 0.2.122 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-streams | 0.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | web_atoms | 0.2.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | web_atoms | 0.2.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | web-sys | 0.3.99 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | web-time | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webauthn-attestation-ca | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webauthn-authenticator-rs | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webauthn-rs-core | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webauthn-rs-proto | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webauthn-rs | 0.5.5 | Workspace resolved | MPL-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webpki-root-certs | 1.0.7 | Workspace resolved | CDLA-Permissive-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | webpki-roots | 1.0.7 | Workspace resolved | CDLA-Permissive-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | whoami | 2.1.3 | Workspace resolved | Apache-2.0 OR BSL-1.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winapi-i686-pc-windows-gnu | 0.4.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winapi-util | 0.1.11 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winapi-x86_64-pc-windows-gnu | 0.4.0 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winapi | 0.3.9 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_gnullvm | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_gnullvm | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_gnullvm | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_msvc | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_msvc | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_aarch64_msvc | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_gnu | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_gnu | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_gnu | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_gnullvm | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_gnullvm | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_msvc | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_msvc | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_i686_msvc | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnu | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnu | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnu | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnullvm | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnullvm | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_gnullvm | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_msvc | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_msvc | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows_x86_64_msvc | 0.53.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-core | 0.62.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-implement | 0.60.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-interface | 0.59.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-link | 0.2.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-registry | 0.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-result | 0.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-strings | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-sys | 0.48.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-sys | 0.52.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-sys | 0.60.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-sys | 0.61.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-targets | 0.48.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-targets | 0.52.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-targets | 0.53.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winnow | 1.0.3 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winnow | 1.0.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wiremock | 0.6.5 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wit-bindgen | 0.57.1 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | writeable | 0.5.5 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | writeable | 0.6.3 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wyz | 0.5.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | x509-parser | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | xattr | 1.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | xxhash-rust | 0.8.15 | Workspace resolved | BSL-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | yaml-rust | 0.4.5 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | yansi | 1.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | yoke-derive | 0.8.2 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | yoke | 0.8.2 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerocopy-derive | 0.8.48 | Workspace resolved | BSD-2-Clause OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerocopy | 0.8.48 | Workspace resolved | BSD-2-Clause OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerofrom-derive | 0.1.7 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerofrom | 0.1.8 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zeroize_derive | 1.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zeroize | 1.9.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerotrie | 0.2.4 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerovec-derive | 0.11.3 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerovec | 0.11.6 | Workspace resolved | Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zmij | 1.0.21 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zmij | 1.0.23 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| npm | @esbuild/aix-ppc64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/aix-ppc64/-/aix-ppc64-0.28.2.tgz |
| npm | @esbuild/android-arm | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/android-arm/-/android-arm-0.28.2.tgz |
| npm | @esbuild/android-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/android-arm64/-/android-arm64-0.28.2.tgz |
| npm | @esbuild/android-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/android-x64/-/android-x64-0.28.2.tgz |
| npm | @esbuild/darwin-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/darwin-arm64/-/darwin-arm64-0.28.2.tgz |
| npm | @esbuild/darwin-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/darwin-x64/-/darwin-x64-0.28.2.tgz |
| npm | @esbuild/freebsd-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/freebsd-arm64/-/freebsd-arm64-0.28.2.tgz |
| npm | @esbuild/freebsd-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/freebsd-x64/-/freebsd-x64-0.28.2.tgz |
| npm | @esbuild/linux-arm | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-arm/-/linux-arm-0.28.2.tgz |
| npm | @esbuild/linux-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-arm64/-/linux-arm64-0.28.2.tgz |
| npm | @esbuild/linux-ia32 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-ia32/-/linux-ia32-0.28.2.tgz |
| npm | @esbuild/linux-loong64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-loong64/-/linux-loong64-0.28.2.tgz |
| npm | @esbuild/linux-mips64el | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-mips64el/-/linux-mips64el-0.28.2.tgz |
| npm | @esbuild/linux-ppc64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-ppc64/-/linux-ppc64-0.28.2.tgz |
| npm | @esbuild/linux-riscv64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-riscv64/-/linux-riscv64-0.28.2.tgz |
| npm | @esbuild/linux-s390x | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-s390x/-/linux-s390x-0.28.2.tgz |
| npm | @esbuild/linux-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/linux-x64/-/linux-x64-0.28.2.tgz |
| npm | @esbuild/netbsd-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/netbsd-arm64/-/netbsd-arm64-0.28.2.tgz |
| npm | @esbuild/netbsd-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/netbsd-x64/-/netbsd-x64-0.28.2.tgz |
| npm | @esbuild/openbsd-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/openbsd-arm64/-/openbsd-arm64-0.28.2.tgz |
| npm | @esbuild/openbsd-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/openbsd-x64/-/openbsd-x64-0.28.2.tgz |
| npm | @esbuild/openharmony-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/openharmony-arm64/-/openharmony-arm64-0.28.2.tgz |
| npm | @esbuild/sunos-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/sunos-x64/-/sunos-x64-0.28.2.tgz |
| npm | @esbuild/win32-arm64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/win32-arm64/-/win32-arm64-0.28.2.tgz |
| npm | @esbuild/win32-ia32 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/win32-ia32/-/win32-ia32-0.28.2.tgz |
| npm | @esbuild/win32-x64 | 0.28.2 | Production build | MIT | https://registry.npmjs.org/@esbuild/win32-x64/-/win32-x64-0.28.2.tgz |
| npm | @eslint-community/eslint-utils | 4.10.1 | Development tooling | MIT | https://registry.npmjs.org/@eslint-community/eslint-utils/-/eslint-utils-4.10.1.tgz |
| npm | @eslint-community/regexpp | 4.12.2 | Development tooling | MIT | https://registry.npmjs.org/@eslint-community/regexpp/-/regexpp-4.12.2.tgz |
| npm | @eslint/config-array | 0.23.5 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@eslint/config-array/-/config-array-0.23.5.tgz |
| npm | @eslint/config-helpers | 0.7.0 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@eslint/config-helpers/-/config-helpers-0.7.0.tgz |
| npm | @eslint/core | 1.2.1 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@eslint/core/-/core-1.2.1.tgz |
| npm | @eslint/js | 10.0.1 | Development tooling | MIT | https://registry.npmjs.org/@eslint/js/-/js-10.0.1.tgz |
| npm | @eslint/object-schema | 3.0.5 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@eslint/object-schema/-/object-schema-3.0.5.tgz |
| npm | @eslint/plugin-kit | 0.7.2 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@eslint/plugin-kit/-/plugin-kit-0.7.2.tgz |
| npm | @hotwired/stimulus | 3.2.2 | Test only | MIT | https://registry.npmjs.org/@hotwired/stimulus/-/stimulus-3.2.2.tgz |
| npm | @humanfs/core | 0.19.2 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@humanfs/core/-/core-0.19.2.tgz |
| npm | @humanfs/node | 0.16.8 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@humanfs/node/-/node-0.16.8.tgz |
| npm | @humanfs/types | 0.15.0 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@humanfs/types/-/types-0.15.0.tgz |
| npm | @humanwhocodes/module-importer | 1.0.1 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@humanwhocodes/module-importer/-/module-importer-1.0.1.tgz |
| npm | @humanwhocodes/retry | 0.4.3 | Development tooling | Apache-2.0 | https://registry.npmjs.org/@humanwhocodes/retry/-/retry-0.4.3.tgz |
| npm | @jridgewell/gen-mapping | 0.3.13 | Production build | MIT | https://registry.npmjs.org/@jridgewell/gen-mapping/-/gen-mapping-0.3.13.tgz |
| npm | @jridgewell/resolve-uri | 3.1.2 | Production build | MIT | https://registry.npmjs.org/@jridgewell/resolve-uri/-/resolve-uri-3.1.2.tgz |
| npm | @jridgewell/source-map | 0.3.11 | Production build | MIT | https://registry.npmjs.org/@jridgewell/source-map/-/source-map-0.3.11.tgz |
| npm | @jridgewell/sourcemap-codec | 1.5.5 | Production build | MIT | https://registry.npmjs.org/@jridgewell/sourcemap-codec/-/sourcemap-codec-1.5.5.tgz |
| npm | @jridgewell/trace-mapping | 0.3.31 | Production build | MIT | https://registry.npmjs.org/@jridgewell/trace-mapping/-/trace-mapping-0.3.31.tgz |
| npm | @oxc-project/types | 0.146.0 | Test only | MIT | https://registry.npmjs.org/@oxc-project/types/-/types-0.146.0.tgz |
| npm | @playwright/test | 1.62.1 | Test only | Apache-2.0 | https://registry.npmjs.org/@playwright/test/-/test-1.62.1.tgz |
| npm | @rolldown/binding-android-arm-eabi | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-android-arm-eabi/-/binding-android-arm-eabi-1.2.5.tgz |
| npm | @rolldown/binding-android-arm64 | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-android-arm64/-/binding-android-arm64-1.2.5.tgz |
| npm | @rolldown/binding-darwin-arm64 | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-darwin-arm64/-/binding-darwin-arm64-1.2.5.tgz |
| npm | @rolldown/binding-darwin-x64 | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-darwin-x64/-/binding-darwin-x64-1.2.5.tgz |
| npm | @rolldown/binding-freebsd-x64 | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-freebsd-x64/-/binding-freebsd-x64-1.2.5.tgz |
| npm | @rolldown/binding-linux-arm-gnueabihf | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-arm-gnueabihf/-/binding-linux-arm-gnueabihf-1.2.5.tgz |
| npm | @rolldown/binding-linux-arm64-gnu | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-arm64-gnu/-/binding-linux-arm64-gnu-1.2.5.tgz |
| npm | @rolldown/binding-linux-arm64-musl | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-arm64-musl/-/binding-linux-arm64-musl-1.2.5.tgz |
| npm | @rolldown/binding-linux-ppc64-gnu | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-ppc64-gnu/-/binding-linux-ppc64-gnu-1.2.5.tgz |
| npm | @rolldown/binding-linux-s390x-gnu | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-s390x-gnu/-/binding-linux-s390x-gnu-1.2.5.tgz |
| npm | @rolldown/binding-linux-x64-gnu | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-x64-gnu/-/binding-linux-x64-gnu-1.2.5.tgz |
| npm | @rolldown/binding-linux-x64-musl | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-linux-x64-musl/-/binding-linux-x64-musl-1.2.5.tgz |
| npm | @rolldown/binding-openharmony-arm64 | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-openharmony-arm64/-/binding-openharmony-arm64-1.2.5.tgz |
| npm | @rolldown/binding-win32-arm64-msvc | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-win32-arm64-msvc/-/binding-win32-arm64-msvc-1.2.5.tgz |
| npm | @rolldown/binding-win32-x64-msvc | 1.2.5 | Test only | MIT | https://registry.npmjs.org/@rolldown/binding-win32-x64-msvc/-/binding-win32-x64-msvc-1.2.5.tgz |
| npm | @rolldown/pluginutils | 1.0.1 | Test only | MIT | https://registry.npmjs.org/@rolldown/pluginutils/-/pluginutils-1.0.1.tgz |
| npm | @standard-schema/spec | 1.1.0 | Test only | MIT | https://registry.npmjs.org/@standard-schema/spec/-/spec-1.1.0.tgz |
| npm | @types/chai | 5.2.3 | Test only | MIT | https://registry.npmjs.org/@types/chai/-/chai-5.2.3.tgz |
| npm | @types/deep-eql | 4.0.2 | Test only | MIT | https://registry.npmjs.org/@types/deep-eql/-/deep-eql-4.0.2.tgz |
| npm | @types/esrecurse | 4.3.1 | Development tooling | MIT | https://registry.npmjs.org/@types/esrecurse/-/esrecurse-4.3.1.tgz |
| npm | @types/estree | 1.0.9 | Test only | MIT | https://registry.npmjs.org/@types/estree/-/estree-1.0.9.tgz |
| npm | @types/json-schema | 7.0.15 | Development tooling | MIT | https://registry.npmjs.org/@types/json-schema/-/json-schema-7.0.15.tgz |
| npm | @types/node | 26.2.0 | Development tooling | MIT | https://registry.npmjs.org/@types/node/-/node-26.2.0.tgz |
| npm | @typescript-eslint/eslint-plugin | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/eslint-plugin/-/eslint-plugin-8.67.0.tgz |
| npm | @typescript-eslint/parser | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/parser/-/parser-8.67.0.tgz |
| npm | @typescript-eslint/project-service | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/project-service/-/project-service-8.67.0.tgz |
| npm | @typescript-eslint/scope-manager | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/scope-manager/-/scope-manager-8.67.0.tgz |
| npm | @typescript-eslint/tsconfig-utils | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/tsconfig-utils/-/tsconfig-utils-8.67.0.tgz |
| npm | @typescript-eslint/type-utils | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/type-utils/-/type-utils-8.67.0.tgz |
| npm | @typescript-eslint/types | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/types/-/types-8.67.0.tgz |
| npm | @typescript-eslint/typescript-estree | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/typescript-estree/-/typescript-estree-8.67.0.tgz |
| npm | @typescript-eslint/utils | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/utils/-/utils-8.67.0.tgz |
| npm | @typescript-eslint/visitor-keys | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/@typescript-eslint/visitor-keys/-/visitor-keys-8.67.0.tgz |
| npm | @vitest/expect | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/expect/-/expect-4.1.11.tgz |
| npm | @vitest/mocker | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/mocker/-/mocker-4.1.11.tgz |
| npm | @vitest/pretty-format | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/pretty-format/-/pretty-format-4.1.11.tgz |
| npm | @vitest/runner | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/runner/-/runner-4.1.11.tgz |
| npm | @vitest/snapshot | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/snapshot/-/snapshot-4.1.11.tgz |
| npm | @vitest/spy | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/spy/-/spy-4.1.11.tgz |
| npm | @vitest/utils | 4.1.11 | Test only | MIT | https://registry.npmjs.org/@vitest/utils/-/utils-4.1.11.tgz |
| npm | acorn-jsx | 5.3.2 | Development tooling | MIT | https://registry.npmjs.org/acorn-jsx/-/acorn-jsx-5.3.2.tgz |
| npm | acorn | 8.18.0 | Production build | MIT | https://registry.npmjs.org/acorn/-/acorn-8.18.0.tgz |
| npm | ajv | 6.15.0 | Development tooling | MIT | https://registry.npmjs.org/ajv/-/ajv-6.15.0.tgz |
| npm | assertion-error | 2.0.1 | Test only | MIT | https://registry.npmjs.org/assertion-error/-/assertion-error-2.0.1.tgz |
| npm | axe-core | 4.13.0 | Test only | MPL-2.0 | https://registry.npmjs.org/axe-core/-/axe-core-4.13.0.tgz |
| npm | balanced-match | 4.0.4 | Development tooling | MIT | https://registry.npmjs.org/balanced-match/-/balanced-match-4.0.4.tgz |
| npm | brace-expansion | 5.0.9 | Development tooling | MIT | https://registry.npmjs.org/brace-expansion/-/brace-expansion-5.0.9.tgz |
| npm | buffer-from | 1.1.2 | Production build | MIT | https://registry.npmjs.org/buffer-from/-/buffer-from-1.1.2.tgz |
| npm | chai | 6.2.2 | Test only | MIT | https://registry.npmjs.org/chai/-/chai-6.2.2.tgz |
| npm | commander | 2.20.3 | Production build | MIT | https://registry.npmjs.org/commander/-/commander-2.20.3.tgz |
| npm | convert-source-map | 2.0.0 | Test only | MIT | https://registry.npmjs.org/convert-source-map/-/convert-source-map-2.0.0.tgz |
| npm | cross-spawn | 7.0.6 | Development tooling | MIT | https://registry.npmjs.org/cross-spawn/-/cross-spawn-7.0.6.tgz |
| npm | debug | 4.4.3 | Development tooling | MIT | https://registry.npmjs.org/debug/-/debug-4.4.3.tgz |
| npm | deep-is | 0.1.4 | Development tooling | MIT | https://registry.npmjs.org/deep-is/-/deep-is-0.1.4.tgz |
| npm | detect-libc | 2.1.2 | Test only | Apache-2.0 | https://registry.npmjs.org/detect-libc/-/detect-libc-2.1.2.tgz |
| npm | es-module-lexer | 2.3.2 | Test only | MIT | https://registry.npmjs.org/es-module-lexer/-/es-module-lexer-2.3.2.tgz |
| npm | esbuild | 0.28.2 | Production build | MIT | https://registry.npmjs.org/esbuild/-/esbuild-0.28.2.tgz |
| npm | escape-string-regexp | 4.0.0 | Development tooling | MIT | https://registry.npmjs.org/escape-string-regexp/-/escape-string-regexp-4.0.0.tgz |
| npm | eslint-scope | 9.1.2 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/eslint-scope/-/eslint-scope-9.1.2.tgz |
| npm | eslint-visitor-keys | 3.4.3 | Development tooling | Apache-2.0 | https://registry.npmjs.org/eslint-visitor-keys/-/eslint-visitor-keys-3.4.3.tgz |
| npm | eslint-visitor-keys | 5.0.1 | Development tooling | Apache-2.0 | https://registry.npmjs.org/eslint-visitor-keys/-/eslint-visitor-keys-5.0.1.tgz |
| npm | eslint | 10.9.0 | Development tooling | MIT | https://registry.npmjs.org/eslint/-/eslint-10.9.0.tgz |
| npm | espree | 11.2.0 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/espree/-/espree-11.2.0.tgz |
| npm | esquery | 1.7.0 | Development tooling | BSD-3-Clause | https://registry.npmjs.org/esquery/-/esquery-1.7.0.tgz |
| npm | esrecurse | 4.3.0 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/esrecurse/-/esrecurse-4.3.0.tgz |
| npm | estraverse | 5.3.0 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/estraverse/-/estraverse-5.3.0.tgz |
| npm | estree-walker | 3.0.3 | Test only | MIT | https://registry.npmjs.org/estree-walker/-/estree-walker-3.0.3.tgz |
| npm | esutils | 2.0.3 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/esutils/-/esutils-2.0.3.tgz |
| npm | expect-type | 1.4.0 | Test only | Apache-2.0 | https://registry.npmjs.org/expect-type/-/expect-type-1.4.0.tgz |
| npm | fast-check | 4.9.0 | Test only | MIT | https://registry.npmjs.org/fast-check/-/fast-check-4.9.0.tgz |
| npm | fast-deep-equal | 3.1.3 | Development tooling | MIT | https://registry.npmjs.org/fast-deep-equal/-/fast-deep-equal-3.1.3.tgz |
| npm | fast-json-stable-stringify | 2.1.0 | Development tooling | MIT | https://registry.npmjs.org/fast-json-stable-stringify/-/fast-json-stable-stringify-2.1.0.tgz |
| npm | fast-levenshtein | 2.0.6 | Development tooling | MIT | https://registry.npmjs.org/fast-levenshtein/-/fast-levenshtein-2.0.6.tgz |
| npm | fdir | 6.5.0 | Test only | MIT | https://registry.npmjs.org/fdir/-/fdir-6.5.0.tgz |
| npm | file-entry-cache | 8.0.0 | Development tooling | MIT | https://registry.npmjs.org/file-entry-cache/-/file-entry-cache-8.0.0.tgz |
| npm | find-up | 5.0.0 | Development tooling | MIT | https://registry.npmjs.org/find-up/-/find-up-5.0.0.tgz |
| npm | flat-cache | 4.0.1 | Development tooling | MIT | https://registry.npmjs.org/flat-cache/-/flat-cache-4.0.1.tgz |
| npm | flatted | 3.4.4 | Development tooling | ISC | https://registry.npmjs.org/flatted/-/flatted-3.4.4.tgz |
| npm | fsevents | 2.3.2 | Test only | MIT | https://registry.npmjs.org/fsevents/-/fsevents-2.3.2.tgz |
| npm | fsevents | 2.3.3 | Test only | MIT | https://registry.npmjs.org/fsevents/-/fsevents-2.3.3.tgz |
| npm | glob-parent | 6.0.2 | Development tooling | ISC | https://registry.npmjs.org/glob-parent/-/glob-parent-6.0.2.tgz |
| npm | globals | 17.11.0 | Development tooling | MIT | https://registry.npmjs.org/globals/-/globals-17.11.0.tgz |
| npm | idiomorph | 0.7.4 | Production runtime | 0BSD | https://registry.npmjs.org/idiomorph/-/idiomorph-0.7.4.tgz |
| npm | ignore | 5.3.2 | Development tooling | MIT | https://registry.npmjs.org/ignore/-/ignore-5.3.2.tgz |
| npm | ignore | 7.0.6 | Development tooling | MIT | https://registry.npmjs.org/ignore/-/ignore-7.0.6.tgz |
| npm | imurmurhash | 0.1.4 | Development tooling | MIT | https://registry.npmjs.org/imurmurhash/-/imurmurhash-0.1.4.tgz |
| npm | is-extglob | 2.1.1 | Development tooling | MIT | https://registry.npmjs.org/is-extglob/-/is-extglob-2.1.1.tgz |
| npm | is-glob | 4.0.3 | Development tooling | MIT | https://registry.npmjs.org/is-glob/-/is-glob-4.0.3.tgz |
| npm | isexe | 2.0.0 | Development tooling | ISC | https://registry.npmjs.org/isexe/-/isexe-2.0.0.tgz |
| npm | json-buffer | 3.0.1 | Development tooling | MIT | https://registry.npmjs.org/json-buffer/-/json-buffer-3.0.1.tgz |
| npm | json-schema-traverse | 0.4.1 | Development tooling | MIT | https://registry.npmjs.org/json-schema-traverse/-/json-schema-traverse-0.4.1.tgz |
| npm | json-stable-stringify-without-jsonify | 1.0.1 | Development tooling | MIT | https://registry.npmjs.org/json-stable-stringify-without-jsonify/-/json-stable-stringify-without-jsonify-1.0.1.tgz |
| npm | keyv | 4.5.4 | Development tooling | MIT | https://registry.npmjs.org/keyv/-/keyv-4.5.4.tgz |
| npm | levn | 0.4.1 | Development tooling | MIT | https://registry.npmjs.org/levn/-/levn-0.4.1.tgz |
| npm | lightningcss-android-arm64 | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-android-arm64/-/lightningcss-android-arm64-1.33.0.tgz |
| npm | lightningcss-darwin-arm64 | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-darwin-arm64/-/lightningcss-darwin-arm64-1.33.0.tgz |
| npm | lightningcss-darwin-x64 | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-darwin-x64/-/lightningcss-darwin-x64-1.33.0.tgz |
| npm | lightningcss-freebsd-x64 | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-freebsd-x64/-/lightningcss-freebsd-x64-1.33.0.tgz |
| npm | lightningcss-linux-arm-gnueabihf | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-linux-arm-gnueabihf/-/lightningcss-linux-arm-gnueabihf-1.33.0.tgz |
| npm | lightningcss-linux-arm64-gnu | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-linux-arm64-gnu/-/lightningcss-linux-arm64-gnu-1.33.0.tgz |
| npm | lightningcss-linux-arm64-musl | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-linux-arm64-musl/-/lightningcss-linux-arm64-musl-1.33.0.tgz |
| npm | lightningcss-linux-x64-gnu | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-linux-x64-gnu/-/lightningcss-linux-x64-gnu-1.33.0.tgz |
| npm | lightningcss-linux-x64-musl | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-linux-x64-musl/-/lightningcss-linux-x64-musl-1.33.0.tgz |
| npm | lightningcss-win32-arm64-msvc | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-win32-arm64-msvc/-/lightningcss-win32-arm64-msvc-1.33.0.tgz |
| npm | lightningcss-win32-x64-msvc | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss-win32-x64-msvc/-/lightningcss-win32-x64-msvc-1.33.0.tgz |
| npm | lightningcss | 1.33.0 | Test only | MPL-2.0 | https://registry.npmjs.org/lightningcss/-/lightningcss-1.33.0.tgz |
| npm | locate-path | 6.0.0 | Development tooling | MIT | https://registry.npmjs.org/locate-path/-/locate-path-6.0.0.tgz |
| npm | magic-string | 0.30.21 | Test only | MIT | https://registry.npmjs.org/magic-string/-/magic-string-0.30.21.tgz |
| npm | minimatch | 10.2.6 | Development tooling | BlueOak-1.0.0 | https://registry.npmjs.org/minimatch/-/minimatch-10.2.6.tgz |
| npm | ms | 2.1.3 | Development tooling | MIT | https://registry.npmjs.org/ms/-/ms-2.1.3.tgz |
| npm | nanoid | 3.3.18 | Test only | MIT | https://registry.npmjs.org/nanoid/-/nanoid-3.3.18.tgz |
| npm | natural-compare | 1.4.0 | Development tooling | MIT | https://registry.npmjs.org/natural-compare/-/natural-compare-1.4.0.tgz |
| npm | obug | 2.1.4 | Test only | MIT | https://registry.npmjs.org/obug/-/obug-2.1.4.tgz |
| npm | optionator | 0.9.4 | Development tooling | MIT | https://registry.npmjs.org/optionator/-/optionator-0.9.4.tgz |
| npm | p-limit | 3.1.0 | Development tooling | MIT | https://registry.npmjs.org/p-limit/-/p-limit-3.1.0.tgz |
| npm | p-locate | 5.0.0 | Development tooling | MIT | https://registry.npmjs.org/p-locate/-/p-locate-5.0.0.tgz |
| npm | path-exists | 4.0.0 | Development tooling | MIT | https://registry.npmjs.org/path-exists/-/path-exists-4.0.0.tgz |
| npm | path-key | 3.1.1 | Development tooling | MIT | https://registry.npmjs.org/path-key/-/path-key-3.1.1.tgz |
| npm | pathe | 2.0.3 | Test only | MIT | https://registry.npmjs.org/pathe/-/pathe-2.0.3.tgz |
| npm | picocolors | 1.1.1 | Test only | ISC | https://registry.npmjs.org/picocolors/-/picocolors-1.1.1.tgz |
| npm | picomatch | 4.0.5 | Test only | MIT | https://registry.npmjs.org/picomatch/-/picomatch-4.0.5.tgz |
| npm | playwright-core | 1.62.1 | Test only | Apache-2.0 | https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz |
| npm | playwright | 1.62.1 | Test only | Apache-2.0 | https://registry.npmjs.org/playwright/-/playwright-1.62.1.tgz |
| npm | postcss | 8.5.26 | Test only | MIT | https://registry.npmjs.org/postcss/-/postcss-8.5.26.tgz |
| npm | prelude-ls | 1.2.1 | Development tooling | MIT | https://registry.npmjs.org/prelude-ls/-/prelude-ls-1.2.1.tgz |
| npm | prettier | 3.9.6 | Development tooling | MIT | https://registry.npmjs.org/prettier/-/prettier-3.9.6.tgz |
| npm | punycode | 2.3.1 | Development tooling | MIT | https://registry.npmjs.org/punycode/-/punycode-2.3.1.tgz |
| npm | pure-rand | 8.4.2 | Test only | MIT | https://registry.npmjs.org/pure-rand/-/pure-rand-8.4.2.tgz |
| npm | rolldown | 1.2.5 | Test only | MIT | https://registry.npmjs.org/rolldown/-/rolldown-1.2.5.tgz |
| npm | semver | 7.8.5 | Development tooling | ISC | https://registry.npmjs.org/semver/-/semver-7.8.5.tgz |
| npm | shebang-command | 2.0.0 | Development tooling | MIT | https://registry.npmjs.org/shebang-command/-/shebang-command-2.0.0.tgz |
| npm | shebang-regex | 3.0.0 | Development tooling | MIT | https://registry.npmjs.org/shebang-regex/-/shebang-regex-3.0.0.tgz |
| npm | siginfo | 2.0.0 | Test only | ISC | https://registry.npmjs.org/siginfo/-/siginfo-2.0.0.tgz |
| npm | source-map-js | 1.2.1 | Test only | BSD-3-Clause | https://registry.npmjs.org/source-map-js/-/source-map-js-1.2.1.tgz |
| npm | source-map-support | 0.5.21 | Production build | MIT | https://registry.npmjs.org/source-map-support/-/source-map-support-0.5.21.tgz |
| npm | source-map | 0.6.1 | Production build | BSD-3-Clause | https://registry.npmjs.org/source-map/-/source-map-0.6.1.tgz |
| npm | stackback | 0.0.2 | Test only | MIT | https://registry.npmjs.org/stackback/-/stackback-0.0.2.tgz |
| npm | std-env | 4.2.0 | Test only | MIT | https://registry.npmjs.org/std-env/-/std-env-4.2.0.tgz |
| npm | terser | 5.50.0 | Production build | BSD-2-Clause | https://registry.npmjs.org/terser/-/terser-5.50.0.tgz |
| npm | tinybench | 2.9.0 | Test only | MIT | https://registry.npmjs.org/tinybench/-/tinybench-2.9.0.tgz |
| npm | tinyexec | 1.3.0 | Test only | MIT | https://registry.npmjs.org/tinyexec/-/tinyexec-1.3.0.tgz |
| npm | tinyglobby | 0.2.17 | Test only | MIT | https://registry.npmjs.org/tinyglobby/-/tinyglobby-0.2.17.tgz |
| npm | tinyrainbow | 3.1.1 | Test only | MIT | https://registry.npmjs.org/tinyrainbow/-/tinyrainbow-3.1.1.tgz |
| npm | ts-api-utils | 2.5.0 | Development tooling | MIT | https://registry.npmjs.org/ts-api-utils/-/ts-api-utils-2.5.0.tgz |
| npm | type-check | 0.4.0 | Development tooling | MIT | https://registry.npmjs.org/type-check/-/type-check-0.4.0.tgz |
| npm | typescript-eslint | 8.67.0 | Development tooling | MIT | https://registry.npmjs.org/typescript-eslint/-/typescript-eslint-8.67.0.tgz |
| npm | typescript | 6.0.3 | Development tooling | Apache-2.0 | https://registry.npmjs.org/typescript/-/typescript-6.0.3.tgz |
| npm | undici-types | 8.3.0 | Development tooling | MIT | https://registry.npmjs.org/undici-types/-/undici-types-8.3.0.tgz |
| npm | uri-js | 4.4.1 | Development tooling | BSD-2-Clause | https://registry.npmjs.org/uri-js/-/uri-js-4.4.1.tgz |
| npm | vite | 8.2.2 | Test only | MIT | https://registry.npmjs.org/vite/-/vite-8.2.2.tgz |
| npm | vitest | 4.1.11 | Test only | MIT | https://registry.npmjs.org/vitest/-/vitest-4.1.11.tgz |
| npm | which | 2.0.2 | Development tooling | ISC | https://registry.npmjs.org/which/-/which-2.0.2.tgz |
| npm | why-is-node-running | 2.3.0 | Test only | MIT | https://registry.npmjs.org/why-is-node-running/-/why-is-node-running-2.3.0.tgz |
| npm | word-wrap | 1.2.5 | Development tooling | MIT | https://registry.npmjs.org/word-wrap/-/word-wrap-1.2.5.tgz |
| npm | yocto-queue | 0.1.0 | Development tooling | MIT | https://registry.npmjs.org/yocto-queue/-/yocto-queue-0.1.0.tgz |
