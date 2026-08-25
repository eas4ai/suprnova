# Third-party licenses

Suprnova Live is licensed under MIT. This generated inventory covers every
resolved third-party package in the root, fuzz, compile-fixture, and npm
lockfiles. Regenerate it with
`rtk node scripts/generate-license-inventory.mjs`; the unattended gate uses
`--check` to reject lockfile or license drift.

For npm, usage is derived transitively from the exact root dependency graph.
Production runtime takes precedence over production build, test-only, and
development-tooling reachability. The production asset manifest and JavaScript
banner separately retain Idiomorph's name, version, and 0BSD license metadata.

| Ecosystem | Package | Version | Usage | License | Locked source |
|---|---|---:|---|---|---|
| Cargo | arbitrary | 1.4.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_derive | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_macros | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama_parser | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | askama | 0.16.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | async-trait | 0.1.92 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | autocfg | 1.5.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | base64 | 0.22.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | basic-toml | 0.1.10 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bit-set | 0.8.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bit-vec | 0.8.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bitflags | 2.13.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | block-buffer | 0.12.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bumpalo | 3.20.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | bytes | 1.11.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cc | 1.4.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cfg-if | 1.0.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cmov | 0.5.4 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | const-oid | 0.10.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | cpufeatures | 0.3.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | crypto-common | 0.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ctutils | 0.4.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | deranged | 0.5.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | digest | 0.11.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | equivalent | 1.0.2 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | errno | 0.3.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fastrand | 2.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | find-msvc-tools | 0.1.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | fnv | 1.0.7 | Workspace resolved | Apache-2.0 / MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-core | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-task | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | futures-util | 0.3.34 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | getrandom | 0.3.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | getrandom | 0.4.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | glob | 0.3.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hashbrown | 0.17.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hex-literal | 1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hkdf | 0.13.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hmac | 0.13.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | html5ever | 0.39.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | http | 1.4.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | hybrid-array | 0.4.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | imagesize | 0.15.0 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | indexmap | 2.14.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | itoa | 1.0.18 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | jobserver | 0.1.35 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | js-sys | 0.3.104 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libc | 0.2.189 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | libfuzzer-sys | 0.4.10 | Workspace resolved | (MIT OR Apache-2.0) AND NCSA | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | linux-raw-sys | 0.12.1 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | lock_api | 0.4.14 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | log | 0.4.33 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | markup5ever | 0.39.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | memchr | 2.8.3 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | new_debug_unreachable | 1.0.6 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-conv | 0.2.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | num-traits | 0.2.19 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | once_cell | 1.21.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parking_lot_core | 0.9.12 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | parking_lot | 0.12.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | percent-encoding | 2.3.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_codegen | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_generator | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf_shared | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | phf | 0.13.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | pin-project-lite | 0.2.17 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | powerfmt | 0.2.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ppv-lite86 | 0.2.21 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | precomputed-hash | 0.1.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro2 | 1.0.106 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proc-macro2 | 1.0.107 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | proptest | 1.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quick-error | 1.2.3 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quote | 1.0.45 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | quote | 1.0.47 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | r-efi | 5.3.0 | Workspace resolved | MIT OR Apache-2.0 OR LGPL-2.1-or-later | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | r-efi | 6.0.0 | Workspace resolved | MIT OR Apache-2.0 OR LGPL-2.1-or-later | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_chacha | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_core | 0.9.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand_xorshift | 0.4.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rand | 0.9.5 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | redox_syscall | 0.5.18 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | regex-syntax | 0.8.11 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustc-hash | 2.1.3 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustix | 1.1.4 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rustversion | 1.0.23 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | rusty-fork | 0.3.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | ryu-js | 1.0.3 | Workspace resolved | Apache-2.0 OR BSL-1.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | scopeguard | 1.2.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_core | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_derive | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_json_canonicalizer | 0.3.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_json | 1.0.151 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde_spanned | 1.1.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | serde | 1.0.229 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | sha2 | 0.11.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | shlex | 2.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | siphasher | 1.0.3 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | slab | 0.4.12 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | smallvec | 1.15.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | string_cache_codegen | 0.6.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | string_cache | 0.9.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 2.0.117 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 2.0.119 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | syn | 3.0.3 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | target-triple | 1.0.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tempfile | 3.27.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tendril | 0.5.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | termcolor | 1.4.1 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror-impl | 2.0.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | thiserror | 2.0.20 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time-core | 0.1.8 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time-macros | 0.2.27 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | time | 0.3.47 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio-macros | 2.7.2 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | tokio | 1.53.1 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_datetime | 1.1.1+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_parser | 1.1.3+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml_writer | 1.1.2+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | toml | 1.1.4+spec-1.1.0 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | trybuild | 1.0.120 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | typenum | 1.20.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unarray | 0.1.4 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | unicode-ident | 1.0.24 | Workspace resolved | (MIT OR Apache-2.0) AND Unicode-3.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | uuid | 1.23.1 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wait-timeout | 0.2.1 | Workspace resolved | MIT/Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasip2 | 1.0.4+wasi-0.2.12 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro-support | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-macro | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen-shared | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wasm-bindgen | 0.2.127 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | web_atoms | 0.2.6 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winapi-util | 0.1.11 | Workspace resolved | Unlicense OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-link | 0.2.1 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | windows-sys | 0.61.2 | Workspace resolved | MIT OR Apache-2.0 | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | winnow | 1.0.4 | Workspace resolved | MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | wit-bindgen | 0.57.1 | Workspace resolved | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerocopy-derive | 0.8.56 | Workspace resolved | BSD-2-Clause OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zerocopy | 0.8.56 | Workspace resolved | BSD-2-Clause OR Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zeroize_derive | 1.5.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
| Cargo | zeroize | 1.9.0 | Workspace resolved | Apache-2.0 OR MIT | registry+https://github.com/rust-lang/crates.io-index |
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
