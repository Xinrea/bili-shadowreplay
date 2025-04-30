bili-shadowreplay v1.0.0 (/Users/xinreasuper/Desktop/Projects/bili-shadowreplay/src-tauri)
├── async-ffmpeg-sidecar v0.0.1
│   ├── anyhow v1.0.98
│   ├── async_zip v0.0.17
│   │   ├── async-compression v0.4.23
│   │   │   ├── bzip2 v0.5.2
│   │   │   │   └── bzip2-sys v0.1.13+1.0.8
│   │   │   │       [build-dependencies]
│   │   │   │       ├── cc v1.2.20
│   │   │   │       │   ├── jobserver v0.1.33
│   │   │   │       │   │   └── libc v0.2.172
│   │   │   │       │   ├── libc v0.2.172
│   │   │   │       │   └── shlex v1.3.0
│   │   │   │       └── pkg-config v0.3.32
│   │   │   ├── deflate64 v0.1.9
│   │   │   ├── flate2 v1.1.1
│   │   │   │   ├── crc32fast v1.4.2
│   │   │   │   │   └── cfg-if v1.0.0
│   │   │   │   ├── libz-sys v1.1.22
│   │   │   │   │   [build-dependencies]
│   │   │   │   │   ├── cc v1.2.20 (*)
│   │   │   │   │   ├── pkg-config v0.3.32
│   │   │   │   │   └── vcpkg v0.2.15
│   │   │   │   └── miniz_oxide v0.8.8
│   │   │   │       ├── adler2 v2.0.0
│   │   │   │       └── simd-adler32 v0.3.7
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-io v0.3.31
│   │   │   ├── liblzma v0.4.1
│   │   │   │   └── liblzma-sys v0.4.3
│   │   │   │       └── libc v0.2.172
│   │   │   │       [build-dependencies]
│   │   │   │       ├── cc v1.2.20 (*)
│   │   │   │       └── pkg-config v0.3.32
│   │   │   ├── memchr v2.7.4
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   ├── zstd v0.13.3
│   │   │   │   └── zstd-safe v7.2.4
│   │   │   │       └── zstd-sys v2.0.15+zstd.1.5.7
│   │   │   │           [build-dependencies]
│   │   │   │           ├── cc v1.2.20 (*)
│   │   │   │           └── pkg-config v0.3.32
│   │   │   └── zstd-safe v7.2.4 (*)
│   │   ├── chrono v0.4.41
│   │   │   ├── iana-time-zone v0.1.63
│   │   │   │   └── core-foundation-sys v0.8.7
│   │   │   ├── num-traits v0.2.19
│   │   │   │   [build-dependencies]
│   │   │   │   └── autocfg v1.4.0
│   │   │   └── serde v1.0.219
│   │   │       └── serde_derive v1.0.219 (proc-macro)
│   │   │           ├── proc-macro2 v1.0.95
│   │   │           │   └── unicode-ident v1.0.18
│   │   │           ├── quote v1.0.40
│   │   │           │   └── proc-macro2 v1.0.95 (*)
│   │   │           └── syn v2.0.101
│   │   │               ├── proc-macro2 v1.0.95 (*)
│   │   │               ├── quote v1.0.40 (*)
│   │   │               └── unicode-ident v1.0.18
│   │   ├── crc32fast v1.4.2 (*)
│   │   ├── futures-lite v2.6.0
│   │   │   ├── fastrand v2.3.0
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-io v0.3.31
│   │   │   ├── parking v2.2.1
│   │   │   └── pin-project-lite v0.2.16
│   │   ├── pin-project v1.1.10
│   │   │   └── pin-project-internal v1.1.10 (proc-macro)
│   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │       ├── quote v1.0.40 (*)
│   │   │       └── syn v2.0.101 (*)
│   │   ├── thiserror v1.0.69
│   │   │   └── thiserror-impl v1.0.69 (proc-macro)
│   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │       ├── quote v1.0.40 (*)
│   │   │       └── syn v2.0.101 (*)
│   │   ├── tokio v1.44.2
│   │   │   ├── bytes v1.10.1
│   │   │   ├── libc v0.2.172
│   │   │   ├── mio v1.0.3
│   │   │   │   └── libc v0.2.172
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   ├── signal-hook-registry v1.4.5
│   │   │   │   └── libc v0.2.172
│   │   │   ├── socket2 v0.5.9
│   │   │   │   └── libc v0.2.172
│   │   │   └── tokio-macros v2.5.0 (proc-macro)
│   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │       ├── quote v1.0.40 (*)
│   │   │       └── syn v2.0.101 (*)
│   │   └── tokio-util v0.7.15
│   │       ├── bytes v1.10.1
│   │       ├── futures-core v0.3.31
│   │       ├── futures-io v0.3.31
│   │       ├── futures-sink v0.3.31
│   │       ├── pin-project-lite v0.2.16
│   │       └── tokio v1.44.2 (*)
│   ├── futures v0.3.31
│   │   ├── futures-channel v0.3.31
│   │   │   ├── futures-core v0.3.31
│   │   │   └── futures-sink v0.3.31
│   │   ├── futures-core v0.3.31
│   │   ├── futures-executor v0.3.31
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-task v0.3.31
│   │   │   └── futures-util v0.3.31
│   │   │       ├── futures-channel v0.3.31 (*)
│   │   │       ├── futures-core v0.3.31
│   │   │       ├── futures-io v0.3.31
│   │   │       ├── futures-macro v0.3.31 (proc-macro)
│   │   │       │   ├── proc-macro2 v1.0.95 (*)
│   │   │       │   ├── quote v1.0.40 (*)
│   │   │       │   └── syn v2.0.101 (*)
│   │   │       ├── futures-sink v0.3.31
│   │   │       ├── futures-task v0.3.31
│   │   │       ├── memchr v2.7.4
│   │   │       ├── pin-project-lite v0.2.16
│   │   │       ├── pin-utils v0.1.0
│   │   │       └── slab v0.4.9
│   │   │           [build-dependencies]
│   │   │           └── autocfg v1.4.0
│   │   ├── futures-io v0.3.31
│   │   ├── futures-sink v0.3.31
│   │   ├── futures-task v0.3.31
│   │   └── futures-util v0.3.31 (*)
│   ├── futures-util v0.3.31 (*)
│   ├── reqwest v0.12.15
│   │   ├── base64 v0.22.1
│   │   ├── bytes v1.10.1
│   │   ├── cookie v0.18.1
│   │   │   ├── percent-encoding v2.3.1
│   │   │   └── time v0.3.41
│   │   │       ├── deranged v0.4.0
│   │   │       │   ├── powerfmt v0.2.0
│   │   │       │   └── serde v1.0.219 (*)
│   │   │       ├── itoa v1.0.15
│   │   │       ├── libc v0.2.172
│   │   │       ├── num-conv v0.1.0
│   │   │       ├── num_threads v0.1.7
│   │   │       │   └── libc v0.2.172
│   │   │       ├── powerfmt v0.2.0
│   │   │       ├── serde v1.0.219 (*)
│   │   │       ├── time-core v0.1.4
│   │   │       └── time-macros v0.2.22 (proc-macro)
│   │   │           ├── num-conv v0.1.0
│   │   │           └── time-core v0.1.4
│   │   │   [build-dependencies]
│   │   │   └── version_check v0.9.5
│   │   ├── cookie_store v0.21.1
│   │   │   ├── cookie v0.18.1 (*)
│   │   │   ├── document-features v0.2.11 (proc-macro)
│   │   │   │   └── litrs v0.4.1
│   │   │   ├── idna v1.0.3
│   │   │   │   ├── idna_adapter v1.2.0
│   │   │   │   │   ├── icu_normalizer v1.5.0
│   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro)
│   │   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   │   ├── icu_collections v1.5.0
│   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   ├── yoke v0.7.5
│   │   │   │   │   │   │   │   ├── stable_deref_trait v1.2.0
│   │   │   │   │   │   │   │   ├── yoke-derive v0.7.5 (proc-macro)
│   │   │   │   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   │   │   ├── syn v2.0.101 (*)
│   │   │   │   │   │   │   │   │   └── synstructure v0.13.1
│   │   │   │   │   │   │   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   │   │       ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   │   │       └── syn v2.0.101 (*)
│   │   │   │   │   │   │   │   └── zerofrom v0.1.6
│   │   │   │   │   │   │   │       └── zerofrom-derive v0.1.6 (proc-macro)
│   │   │   │   │   │   │   │           ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   │           ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   │           ├── syn v2.0.101 (*)
│   │   │   │   │   │   │   │           └── synstructure v0.13.1 (*)
│   │   │   │   │   │   │   ├── zerofrom v0.1.6 (*)
│   │   │   │   │   │   │   └── zerovec v0.10.4
│   │   │   │   │   │   │       ├── yoke v0.7.5 (*)
│   │   │   │   │   │   │       ├── zerofrom v0.1.6 (*)
│   │   │   │   │   │   │       └── zerovec-derive v0.10.3 (proc-macro)
│   │   │   │   │   │   │           ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │           ├── quote v1.0.40 (*)
│   │   │   │   │   │   │           └── syn v2.0.101 (*)
│   │   │   │   │   │   ├── icu_normalizer_data v1.5.1
│   │   │   │   │   │   ├── icu_properties v1.5.1
│   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   ├── icu_collections v1.5.0 (*)
│   │   │   │   │   │   │   ├── icu_locid_transform v1.5.0
│   │   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   │   ├── icu_locid v1.5.0
│   │   │   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   │   │   ├── litemap v0.7.5
│   │   │   │   │   │   │   │   │   ├── tinystr v0.7.6
│   │   │   │   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   │   │   │   │   ├── writeable v0.5.5
│   │   │   │   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   │   │   │   ├── icu_locid_transform_data v1.5.1
│   │   │   │   │   │   │   │   ├── icu_provider v1.5.0
│   │   │   │   │   │   │   │   │   ├── displaydoc v0.2.5 (proc-macro) (*)
│   │   │   │   │   │   │   │   │   ├── icu_locid v1.5.0 (*)
│   │   │   │   │   │   │   │   │   ├── icu_provider_macros v1.5.0 (proc-macro)
│   │   │   │   │   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   │   │   │   │   ├── stable_deref_trait v1.2.0
│   │   │   │   │   │   │   │   │   ├── tinystr v0.7.6 (*)
│   │   │   │   │   │   │   │   │   ├── writeable v0.5.5
│   │   │   │   │   │   │   │   │   ├── yoke v0.7.5 (*)
│   │   │   │   │   │   │   │   │   ├── zerofrom v0.1.6 (*)
│   │   │   │   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   │   │   │   ├── tinystr v0.7.6 (*)
│   │   │   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   │   │   ├── icu_properties_data v1.5.1
│   │   │   │   │   │   │   ├── icu_provider v1.5.0 (*)
│   │   │   │   │   │   │   ├── tinystr v0.7.6 (*)
│   │   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   │   ├── icu_provider v1.5.0 (*)
│   │   │   │   │   │   ├── smallvec v1.15.0
│   │   │   │   │   │   ├── utf16_iter v1.0.5
│   │   │   │   │   │   ├── utf8_iter v1.0.4
│   │   │   │   │   │   ├── write16 v1.0.0
│   │   │   │   │   │   └── zerovec v0.10.4 (*)
│   │   │   │   │   └── icu_properties v1.5.1 (*)
│   │   │   │   ├── smallvec v1.15.0
│   │   │   │   └── utf8_iter v1.0.4
│   │   │   ├── log v0.4.27
│   │   │   │   └── value-bag v1.11.1
│   │   │   ├── publicsuffix v2.3.0
│   │   │   │   ├── idna v1.0.3 (*)
│   │   │   │   └── psl-types v2.0.11
│   │   │   ├── serde v1.0.219 (*)
│   │   │   ├── serde_derive v1.0.219 (proc-macro) (*)
│   │   │   ├── serde_json v1.0.140
│   │   │   │   ├── itoa v1.0.15
│   │   │   │   ├── memchr v2.7.4
│   │   │   │   ├── ryu v1.0.20
│   │   │   │   └── serde v1.0.219 (*)
│   │   │   ├── time v0.3.41 (*)
│   │   │   └── url v2.5.4
│   │   │       ├── form_urlencoded v1.2.1
│   │   │       │   └── percent-encoding v2.3.1
│   │   │       ├── idna v1.0.3 (*)
│   │   │       ├── percent-encoding v2.3.1
│   │   │       └── serde v1.0.219 (*)
│   │   ├── encoding_rs v0.8.35
│   │   │   └── cfg-if v1.0.0
│   │   ├── futures-core v0.3.31
│   │   ├── futures-util v0.3.31 (*)
│   │   ├── h2 v0.4.9
│   │   │   ├── atomic-waker v1.1.2
│   │   │   ├── bytes v1.10.1
│   │   │   ├── fnv v1.0.7
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-sink v0.3.31
│   │   │   ├── http v1.3.1
│   │   │   │   ├── bytes v1.10.1
│   │   │   │   ├── fnv v1.0.7
│   │   │   │   └── itoa v1.0.15
│   │   │   ├── indexmap v2.9.0
│   │   │   │   ├── equivalent v1.0.2
│   │   │   │   ├── hashbrown v0.15.2
│   │   │   │   │   ├── allocator-api2 v0.2.21
│   │   │   │   │   ├── equivalent v1.0.2
│   │   │   │   │   └── foldhash v0.1.5
│   │   │   │   └── serde v1.0.219 (*)
│   │   │   ├── slab v0.4.9 (*)
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   ├── tokio-util v0.7.15 (*)
│   │   │   └── tracing v0.1.41
│   │   │       ├── log v0.4.27 (*)
│   │   │       ├── pin-project-lite v0.2.16
│   │   │       ├── tracing-attributes v0.1.28 (proc-macro)
│   │   │       │   ├── proc-macro2 v1.0.95 (*)
│   │   │       │   ├── quote v1.0.40 (*)
│   │   │       │   └── syn v2.0.101 (*)
│   │   │       └── tracing-core v0.1.33
│   │   │           └── once_cell v1.21.3
│   │   ├── http v1.3.1 (*)
│   │   ├── http-body v1.0.1
│   │   │   ├── bytes v1.10.1
│   │   │   └── http v1.3.1 (*)
│   │   ├── http-body-util v0.1.3
│   │   │   ├── bytes v1.10.1
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── http v1.3.1 (*)
│   │   │   ├── http-body v1.0.1 (*)
│   │   │   └── pin-project-lite v0.2.16
│   │   ├── hyper v1.6.0
│   │   │   ├── bytes v1.10.1
│   │   │   ├── futures-channel v0.3.31 (*)
│   │   │   ├── futures-util v0.3.31 (*)
│   │   │   ├── h2 v0.4.9 (*)
│   │   │   ├── http v1.3.1 (*)
│   │   │   ├── http-body v1.0.1 (*)
│   │   │   ├── httparse v1.10.1
│   │   │   ├── httpdate v1.0.3
│   │   │   ├── itoa v1.0.15
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   ├── smallvec v1.15.0
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   └── want v0.3.1
│   │   │       └── try-lock v0.2.5
│   │   ├── hyper-rustls v0.27.5
│   │   │   ├── futures-util v0.3.31 (*)
│   │   │   ├── http v1.3.1 (*)
│   │   │   ├── hyper v1.6.0 (*)
│   │   │   ├── hyper-util v0.1.11
│   │   │   │   ├── bytes v1.10.1
│   │   │   │   ├── futures-channel v0.3.31 (*)
│   │   │   │   ├── futures-util v0.3.31 (*)
│   │   │   │   ├── http v1.3.1 (*)
│   │   │   │   ├── http-body v1.0.1 (*)
│   │   │   │   ├── hyper v1.6.0 (*)
│   │   │   │   ├── libc v0.2.172
│   │   │   │   ├── pin-project-lite v0.2.16
│   │   │   │   ├── socket2 v0.5.9 (*)
│   │   │   │   ├── tokio v1.44.2 (*)
│   │   │   │   ├── tower-service v0.3.3
│   │   │   │   └── tracing v0.1.41 (*)
│   │   │   ├── rustls v0.23.26
│   │   │   │   ├── once_cell v1.21.3
│   │   │   │   ├── ring v0.17.14
│   │   │   │   │   ├── cfg-if v1.0.0
│   │   │   │   │   ├── getrandom v0.2.16
│   │   │   │   │   │   ├── cfg-if v1.0.0
│   │   │   │   │   │   └── libc v0.2.172
│   │   │   │   │   ├── libc v0.2.172
│   │   │   │   │   └── untrusted v0.9.0
│   │   │   │   │   [build-dependencies]
│   │   │   │   │   └── cc v1.2.20 (*)
│   │   │   │   ├── rustls-pki-types v1.11.0
│   │   │   │   ├── rustls-webpki v0.103.1
│   │   │   │   │   ├── ring v0.17.14 (*)
│   │   │   │   │   ├── rustls-pki-types v1.11.0
│   │   │   │   │   └── untrusted v0.9.0
│   │   │   │   ├── subtle v2.6.1
│   │   │   │   └── zeroize v1.8.1
│   │   │   ├── rustls-native-certs v0.8.1
│   │   │   │   ├── rustls-pki-types v1.11.0
│   │   │   │   └── security-framework v3.2.0
│   │   │   │       ├── bitflags v2.9.0
│   │   │   │       │   └── serde v1.0.219 (*)
│   │   │   │       ├── core-foundation v0.10.0
│   │   │   │       │   ├── core-foundation-sys v0.8.7
│   │   │   │       │   └── libc v0.2.172
│   │   │   │       ├── core-foundation-sys v0.8.7
│   │   │   │       ├── libc v0.2.172
│   │   │   │       └── security-framework-sys v2.14.0
│   │   │   │           ├── core-foundation-sys v0.8.7
│   │   │   │           └── libc v0.2.172
│   │   │   ├── rustls-pki-types v1.11.0
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   ├── tokio-rustls v0.26.2
│   │   │   │   ├── rustls v0.23.26 (*)
│   │   │   │   └── tokio v1.44.2 (*)
│   │   │   ├── tower-service v0.3.3
│   │   │   └── webpki-roots v0.26.9
│   │   │       └── rustls-pki-types v1.11.0
│   │   ├── hyper-util v0.1.11 (*)
│   │   ├── ipnet v2.11.0
│   │   ├── log v0.4.27 (*)
│   │   ├── mime v0.3.17
│   │   ├── once_cell v1.21.3
│   │   ├── percent-encoding v2.3.1
│   │   ├── pin-project-lite v0.2.16
│   │   ├── rustls v0.23.26 (*)
│   │   ├── rustls-native-certs v0.8.1 (*)
│   │   ├── rustls-pemfile v2.2.0
│   │   │   └── rustls-pki-types v1.11.0
│   │   ├── rustls-pki-types v1.11.0
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_urlencoded v0.7.1
│   │   │   ├── form_urlencoded v1.2.1 (*)
│   │   │   ├── itoa v1.0.15
│   │   │   ├── ryu v1.0.20
│   │   │   └── serde v1.0.219 (*)
│   │   ├── sync_wrapper v1.0.2
│   │   │   └── futures-core v0.3.31
│   │   ├── system-configuration v0.6.1
│   │   │   ├── bitflags v2.9.0 (*)
│   │   │   ├── core-foundation v0.9.4
│   │   │   │   ├── core-foundation-sys v0.8.7
│   │   │   │   └── libc v0.2.172
│   │   │   └── system-configuration-sys v0.6.0
│   │   │       ├── core-foundation-sys v0.8.7
│   │   │       └── libc v0.2.172
│   │   ├── tokio v1.44.2 (*)
│   │   ├── tokio-rustls v0.26.2 (*)
│   │   ├── tokio-util v0.7.15 (*)
│   │   ├── tower v0.5.2
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-util v0.3.31 (*)
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   ├── sync_wrapper v1.0.2 (*)
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   ├── tower-layer v0.3.3
│   │   │   ├── tower-service v0.3.3
│   │   │   └── tracing v0.1.41 (*)
│   │   ├── tower-service v0.3.3
│   │   ├── url v2.5.4 (*)
│   │   └── webpki-roots v0.26.9 (*)
│   ├── sanitize-filename v0.6.0
│   │   └── regex v1.11.1
│   │       ├── aho-corasick v1.1.3
│   │       │   └── memchr v2.7.4
│   │       ├── memchr v2.7.4
│   │       ├── regex-automata v0.4.9
│   │       │   ├── aho-corasick v1.1.3 (*)
│   │       │   ├── memchr v2.7.4
│   │       │   └── regex-syntax v0.8.5
│   │       └── regex-syntax v0.8.5
│   ├── tokio v1.44.2 (*)
│   └── tokio-util v0.7.15 (*)
├── async-std v1.13.1
│   ├── async-channel v1.9.0
│   │   ├── concurrent-queue v2.5.0
│   │   │   └── crossbeam-utils v0.8.21
│   │   ├── event-listener v2.5.3
│   │   └── futures-core v0.3.31
│   ├── async-global-executor v2.4.1
│   │   ├── async-channel v2.3.1
│   │   │   ├── concurrent-queue v2.5.0 (*)
│   │   │   ├── event-listener-strategy v0.5.4
│   │   │   │   ├── event-listener v5.4.0
│   │   │   │   │   ├── concurrent-queue v2.5.0 (*)
│   │   │   │   │   ├── parking v2.2.1
│   │   │   │   │   └── pin-project-lite v0.2.16
│   │   │   │   └── pin-project-lite v0.2.16
│   │   │   ├── futures-core v0.3.31
│   │   │   └── pin-project-lite v0.2.16
│   │   ├── async-executor v1.13.2
│   │   │   ├── async-task v4.7.1
│   │   │   ├── concurrent-queue v2.5.0 (*)
│   │   │   ├── fastrand v2.3.0
│   │   │   ├── futures-lite v2.6.0 (*)
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   └── slab v0.4.9 (*)
│   │   ├── async-io v2.4.0
│   │   │   ├── async-lock v3.4.0
│   │   │   │   ├── event-listener v5.4.0 (*)
│   │   │   │   ├── event-listener-strategy v0.5.4 (*)
│   │   │   │   └── pin-project-lite v0.2.16
│   │   │   ├── cfg-if v1.0.0
│   │   │   ├── concurrent-queue v2.5.0 (*)
│   │   │   ├── futures-io v0.3.31
│   │   │   ├── futures-lite v2.6.0 (*)
│   │   │   ├── parking v2.2.1
│   │   │   ├── polling v3.7.4
│   │   │   │   ├── cfg-if v1.0.0
│   │   │   │   ├── rustix v0.38.44
│   │   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   │   ├── errno v0.3.11
│   │   │   │   │   │   └── libc v0.2.172
│   │   │   │   │   └── libc v0.2.172
│   │   │   │   └── tracing v0.1.41 (*)
│   │   │   ├── rustix v0.38.44 (*)
│   │   │   ├── slab v0.4.9 (*)
│   │   │   └── tracing v0.1.41 (*)
│   │   ├── async-lock v3.4.0 (*)
│   │   ├── blocking v1.6.1
│   │   │   ├── async-channel v2.3.1 (*)
│   │   │   ├── async-task v4.7.1
│   │   │   ├── futures-io v0.3.31
│   │   │   ├── futures-lite v2.6.0 (*)
│   │   │   └── piper v0.2.4
│   │   │       ├── atomic-waker v1.1.2
│   │   │       ├── fastrand v2.3.0
│   │   │       └── futures-io v0.3.31
│   │   ├── futures-lite v2.6.0 (*)
│   │   └── once_cell v1.21.3
│   ├── async-io v2.4.0 (*)
│   ├── async-lock v3.4.0 (*)
│   ├── crossbeam-utils v0.8.21
│   ├── futures-core v0.3.31
│   ├── futures-io v0.3.31
│   ├── futures-lite v2.6.0 (*)
│   ├── kv-log-macro v1.0.7
│   │   └── log v0.4.27 (*)
│   ├── log v0.4.27 (*)
│   ├── memchr v2.7.4
│   ├── once_cell v1.21.3
│   ├── pin-project-lite v0.2.16
│   ├── pin-utils v0.1.0
│   └── slab v0.4.9 (*)
├── async-trait v0.1.88 (proc-macro)
│   ├── proc-macro2 v1.0.95 (*)
│   ├── quote v1.0.40 (*)
│   └── syn v2.0.101 (*)
├── axum v0.7.9
│   ├── async-trait v0.1.88 (proc-macro) (*)
│   ├── axum-core v0.4.5
│   │   ├── async-trait v0.1.88 (proc-macro) (*)
│   │   ├── bytes v1.10.1
│   │   ├── futures-util v0.3.31 (*)
│   │   ├── http v1.3.1 (*)
│   │   ├── http-body v1.0.1 (*)
│   │   ├── http-body-util v0.1.3 (*)
│   │   ├── mime v0.3.17
│   │   ├── pin-project-lite v0.2.16
│   │   ├── rustversion v1.0.20 (proc-macro)
│   │   ├── sync_wrapper v1.0.2 (*)
│   │   ├── tower-layer v0.3.3
│   │   ├── tower-service v0.3.3
│   │   └── tracing v0.1.41 (*)
│   ├── axum-macros v0.4.2 (proc-macro)
│   │   ├── proc-macro2 v1.0.95 (*)
│   │   ├── quote v1.0.40 (*)
│   │   └── syn v2.0.101 (*)
│   ├── bytes v1.10.1
│   ├── futures-util v0.3.31 (*)
│   ├── http v1.3.1 (*)
│   ├── http-body v1.0.1 (*)
│   ├── http-body-util v0.1.3 (*)
│   ├── hyper v1.6.0 (*)
│   ├── hyper-util v0.1.11 (*)
│   ├── itoa v1.0.15
│   ├── matchit v0.7.3
│   ├── memchr v2.7.4
│   ├── mime v0.3.17
│   ├── percent-encoding v2.3.1
│   ├── pin-project-lite v0.2.16
│   ├── rustversion v1.0.20 (proc-macro)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── serde_path_to_error v0.1.17
│   │   ├── itoa v1.0.15
│   │   └── serde v1.0.219 (*)
│   ├── serde_urlencoded v0.7.1 (*)
│   ├── sync_wrapper v1.0.2 (*)
│   ├── tokio v1.44.2 (*)
│   ├── tower v0.5.2 (*)
│   ├── tower-layer v0.3.3
│   ├── tower-service v0.3.3
│   └── tracing v0.1.41 (*)
├── base64 v0.21.7
├── chrono v0.4.41 (*)
├── clap v4.5.37
│   ├── clap_builder v4.5.37
│   │   ├── anstream v0.6.18
│   │   │   ├── anstyle v1.0.10
│   │   │   ├── anstyle-parse v0.2.6
│   │   │   │   └── utf8parse v0.2.2
│   │   │   ├── anstyle-query v1.1.2
│   │   │   ├── colorchoice v1.0.3
│   │   │   ├── is_terminal_polyfill v1.70.1
│   │   │   └── utf8parse v0.2.2
│   │   ├── anstyle v1.0.10
│   │   ├── clap_lex v0.7.4
│   │   └── strsim v0.11.1
│   └── clap_derive v4.5.32 (proc-macro)
│       ├── heck v0.5.0
│       ├── proc-macro2 v1.0.95 (*)
│       ├── quote v1.0.40 (*)
│       └── syn v2.0.101 (*)
├── custom_error v1.9.2
├── dashmap v6.1.0
│   ├── cfg-if v1.0.0
│   ├── crossbeam-utils v0.8.21
│   ├── hashbrown v0.14.5
│   ├── lock_api v0.4.12
│   │   └── scopeguard v1.2.0
│   │   [build-dependencies]
│   │   └── autocfg v1.4.0
│   ├── once_cell v1.21.3
│   └── parking_lot_core v0.9.10
│       ├── cfg-if v1.0.0
│       ├── libc v0.2.172
│       └── smallvec v1.15.0
├── felgens v0.3.1 (https://github.com/Xinrea/felgens.git?tag=v0.4.2#02925575)
│   ├── brotli v3.5.0
│   │   ├── alloc-no-stdlib v2.0.4
│   │   ├── alloc-stdlib v0.2.2
│   │   │   └── alloc-no-stdlib v2.0.4
│   │   └── brotli-decompressor v2.5.1
│   │       ├── alloc-no-stdlib v2.0.4
│   │       └── alloc-stdlib v0.2.2 (*)
│   ├── flate2 v1.1.1 (*)
│   ├── futures-util v0.3.31 (*)
│   ├── log v0.4.27 (*)
│   ├── reqwest v0.11.27
│   │   ├── base64 v0.21.7
│   │   ├── bytes v1.10.1
│   │   ├── encoding_rs v0.8.35 (*)
│   │   ├── futures-core v0.3.31
│   │   ├── futures-util v0.3.31 (*)
│   │   ├── h2 v0.3.26
│   │   │   ├── bytes v1.10.1
│   │   │   ├── fnv v1.0.7
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-sink v0.3.31
│   │   │   ├── futures-util v0.3.31 (*)
│   │   │   ├── http v0.2.12
│   │   │   │   ├── bytes v1.10.1
│   │   │   │   ├── fnv v1.0.7
│   │   │   │   └── itoa v1.0.15
│   │   │   ├── indexmap v2.9.0 (*)
│   │   │   ├── slab v0.4.9 (*)
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   ├── tokio-util v0.7.15 (*)
│   │   │   └── tracing v0.1.41 (*)
│   │   ├── http v0.2.12 (*)
│   │   ├── http-body v0.4.6
│   │   │   ├── bytes v1.10.1
│   │   │   ├── http v0.2.12 (*)
│   │   │   └── pin-project-lite v0.2.16
│   │   ├── hyper v0.14.32
│   │   │   ├── bytes v1.10.1
│   │   │   ├── futures-channel v0.3.31 (*)
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── futures-util v0.3.31 (*)
│   │   │   ├── h2 v0.3.26 (*)
│   │   │   ├── http v0.2.12 (*)
│   │   │   ├── http-body v0.4.6 (*)
│   │   │   ├── httparse v1.10.1
│   │   │   ├── httpdate v1.0.3
│   │   │   ├── itoa v1.0.15
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   ├── socket2 v0.5.9 (*)
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   ├── tower-service v0.3.3
│   │   │   ├── tracing v0.1.41 (*)
│   │   │   └── want v0.3.1 (*)
│   │   ├── hyper-tls v0.5.0
│   │   │   ├── bytes v1.10.1
│   │   │   ├── hyper v0.14.32 (*)
│   │   │   ├── native-tls v0.2.14
│   │   │   │   ├── libc v0.2.172
│   │   │   │   ├── security-framework v2.11.1
│   │   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   │   ├── core-foundation v0.9.4 (*)
│   │   │   │   │   ├── core-foundation-sys v0.8.7
│   │   │   │   │   ├── libc v0.2.172
│   │   │   │   │   └── security-framework-sys v2.14.0 (*)
│   │   │   │   ├── security-framework-sys v2.14.0 (*)
│   │   │   │   └── tempfile v3.19.1
│   │   │   │       ├── fastrand v2.3.0
│   │   │   │       ├── getrandom v0.3.2
│   │   │   │       │   ├── cfg-if v1.0.0
│   │   │   │       │   └── libc v0.2.172
│   │   │   │       ├── once_cell v1.21.3
│   │   │   │       └── rustix v1.0.5
│   │   │   │           ├── bitflags v2.9.0 (*)
│   │   │   │           ├── errno v0.3.11 (*)
│   │   │   │           └── libc v0.2.172
│   │   │   ├── tokio v1.44.2 (*)
│   │   │   └── tokio-native-tls v0.3.1
│   │   │       ├── native-tls v0.2.14 (*)
│   │   │       └── tokio v1.44.2 (*)
│   │   ├── ipnet v2.11.0
│   │   ├── log v0.4.27 (*)
│   │   ├── mime v0.3.17
│   │   ├── native-tls v0.2.14 (*)
│   │   ├── once_cell v1.21.3
│   │   ├── percent-encoding v2.3.1
│   │   ├── pin-project-lite v0.2.16
│   │   ├── rustls-pemfile v1.0.4
│   │   │   └── base64 v0.21.7
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   ├── serde_urlencoded v0.7.1 (*)
│   │   ├── sync_wrapper v0.1.2
│   │   ├── system-configuration v0.5.1
│   │   │   ├── bitflags v1.3.2
│   │   │   ├── core-foundation v0.9.4 (*)
│   │   │   └── system-configuration-sys v0.5.0
│   │   │       ├── core-foundation-sys v0.8.7
│   │   │       └── libc v0.2.172
│   │   ├── tokio v1.44.2 (*)
│   │   ├── tokio-native-tls v0.3.1 (*)
│   │   ├── tower-service v0.3.3
│   │   └── url v2.5.4 (*)
│   ├── scroll v0.11.0
│   ├── scroll_derive v0.11.1 (proc-macro)
│   │   ├── proc-macro2 v1.0.95 (*)
│   │   ├── quote v1.0.40 (*)
│   │   └── syn v2.0.101 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── thiserror v1.0.69 (*)
│   ├── tokio v1.44.2 (*)
│   ├── tokio-tungstenite v0.19.0
│   │   ├── futures-util v0.3.31 (*)
│   │   ├── log v0.4.27 (*)
│   │   ├── native-tls v0.2.14 (*)
│   │   ├── tokio v1.44.2 (*)
│   │   ├── tokio-native-tls v0.3.1 (*)
│   │   └── tungstenite v0.19.0
│   │       ├── byteorder v1.5.0
│   │       ├── bytes v1.10.1
│   │       ├── data-encoding v2.9.0
│   │       ├── http v0.2.12 (*)
│   │       ├── httparse v1.10.1
│   │       ├── log v0.4.27 (*)
│   │       ├── native-tls v0.2.14 (*)
│   │       ├── rand v0.8.5
│   │       │   ├── libc v0.2.172
│   │       │   ├── rand_chacha v0.3.1
│   │       │   │   ├── ppv-lite86 v0.2.21
│   │       │   │   │   └── zerocopy v0.8.25
│   │       │   │   └── rand_core v0.6.4
│   │       │   │       └── getrandom v0.2.16 (*)
│   │       │   └── rand_core v0.6.4 (*)
│   │       ├── sha1 v0.10.6
│   │       │   ├── cfg-if v1.0.0
│   │       │   ├── cpufeatures v0.2.17
│   │       │   │   └── libc v0.2.172
│   │       │   └── digest v0.10.7
│   │       │       ├── block-buffer v0.10.4
│   │       │       │   └── generic-array v0.14.7
│   │       │       │       └── typenum v1.18.0
│   │       │       │       [build-dependencies]
│   │       │       │       └── version_check v0.9.5
│   │       │       └── crypto-common v0.1.6
│   │       │           ├── generic-array v0.14.7 (*)
│   │       │           └── typenum v1.18.0
│   │       ├── thiserror v1.0.69 (*)
│   │       ├── url v2.5.4 (*)
│   │       └── utf-8 v0.7.6
│   └── url v2.5.4 (*)
├── fix-path-env v0.0.0 (https://github.com/tauri-apps/fix-path-env-rs#0e479e28)
│   ├── home v0.5.11
│   ├── strip-ansi-escapes v0.2.1
│   │   └── vte v0.14.1
│   │       └── memchr v2.7.4
│   └── thiserror v1.0.69 (*)
├── futures v0.3.31 (*)
├── futures-core v0.3.31
├── hound v3.5.1
├── hyper v0.14.32 (*)
├── log v0.4.27 (*)
├── m3u8-rs v5.0.5
│   ├── chrono v0.4.41 (*)
│   └── nom v7.1.3
│       ├── memchr v2.7.4
│       └── minimal-lexical v0.2.1
├── md5 v0.7.0
├── mime_guess v2.0.5
│   ├── mime v0.3.17
│   └── unicase v2.8.1
│   [build-dependencies]
│   └── unicase v2.8.1
├── pct-str v1.2.0
│   └── utf8-decode v1.0.1
├── platform-dirs v0.3.0
│   └── dirs-next v1.0.2
│       ├── cfg-if v1.0.0
│       └── dirs-sys-next v0.1.2
│           └── libc v0.2.172
├── rand v0.8.5 (*)
├── regex v1.11.1 (*)
├── reqwest v0.11.27 (*)
├── serde v1.0.219 (*)
├── serde_derive v1.0.219 (proc-macro) (*)
├── serde_json v1.0.140 (*)
├── simplelog v0.12.2
│   ├── log v0.4.27 (*)
│   ├── termcolor v1.4.1
│   └── time v0.3.41 (*)
├── sqlx v0.8.5
│   ├── sqlx-core v0.8.5
│   │   ├── base64 v0.22.1
│   │   ├── bytes v1.10.1
│   │   ├── crc v3.2.1
│   │   │   └── crc-catalog v2.4.0
│   │   ├── crossbeam-queue v0.3.12
│   │   │   └── crossbeam-utils v0.8.21
│   │   ├── either v1.15.0
│   │   │   └── serde v1.0.219 (*)
│   │   ├── event-listener v5.4.0 (*)
│   │   ├── futures-core v0.3.31
│   │   ├── futures-intrusive v0.5.0
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── lock_api v0.4.12 (*)
│   │   │   └── parking_lot v0.12.3
│   │   │       ├── lock_api v0.4.12 (*)
│   │   │       └── parking_lot_core v0.9.10 (*)
│   │   ├── futures-io v0.3.31
│   │   ├── futures-util v0.3.31 (*)
│   │   ├── hashbrown v0.15.2 (*)
│   │   ├── hashlink v0.10.0
│   │   │   └── hashbrown v0.15.2 (*)
│   │   ├── indexmap v2.9.0 (*)
│   │   ├── log v0.4.27 (*)
│   │   ├── memchr v2.7.4
│   │   ├── once_cell v1.21.3
│   │   ├── percent-encoding v2.3.1
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   ├── sha2 v0.10.8
│   │   │   ├── cfg-if v1.0.0
│   │   │   ├── cpufeatures v0.2.17 (*)
│   │   │   └── digest v0.10.7 (*)
│   │   ├── smallvec v1.15.0
│   │   ├── thiserror v2.0.12
│   │   │   └── thiserror-impl v2.0.12 (proc-macro)
│   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │       ├── quote v1.0.40 (*)
│   │   │       └── syn v2.0.101 (*)
│   │   ├── time v0.3.41 (*)
│   │   ├── tokio v1.44.2 (*)
│   │   ├── tokio-stream v0.1.17
│   │   │   ├── futures-core v0.3.31
│   │   │   ├── pin-project-lite v0.2.16
│   │   │   └── tokio v1.44.2 (*)
│   │   ├── tracing v0.1.41 (*)
│   │   └── url v2.5.4 (*)
│   ├── sqlx-macros v0.8.5 (proc-macro)
│   │   ├── proc-macro2 v1.0.95 (*)
│   │   ├── quote v1.0.40 (*)
│   │   ├── sqlx-core v0.8.5 (*)
│   │   ├── sqlx-macros-core v0.8.5
│   │   │   ├── dotenvy v0.15.7
│   │   │   ├── either v1.15.0 (*)
│   │   │   ├── heck v0.5.0
│   │   │   ├── hex v0.4.3
│   │   │   ├── once_cell v1.21.3
│   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   ├── quote v1.0.40 (*)
│   │   │   ├── serde v1.0.219 (*)
│   │   │   ├── serde_json v1.0.140
│   │   │   │   ├── itoa v1.0.15
│   │   │   │   ├── memchr v2.7.4
│   │   │   │   ├── ryu v1.0.20
│   │   │   │   └── serde v1.0.219 (*)
│   │   │   ├── sha2 v0.10.8
│   │   │   │   ├── cfg-if v1.0.0
│   │   │   │   ├── cpufeatures v0.2.17 (*)
│   │   │   │   └── digest v0.10.7 (*)
│   │   │   ├── sqlx-core v0.8.5 (*)
│   │   │   ├── sqlx-sqlite v0.8.5
│   │   │   │   ├── atoi v2.0.0
│   │   │   │   │   └── num-traits v0.2.19 (*)
│   │   │   │   ├── flume v0.11.1
│   │   │   │   │   ├── futures-core v0.3.31
│   │   │   │   │   ├── futures-sink v0.3.31
│   │   │   │   │   └── spin v0.9.8
│   │   │   │   │       └── lock_api v0.4.12 (*)
│   │   │   │   ├── futures-channel v0.3.31
│   │   │   │   │   ├── futures-core v0.3.31
│   │   │   │   │   └── futures-sink v0.3.31
│   │   │   │   ├── futures-core v0.3.31
│   │   │   │   ├── futures-executor v0.3.31 (*)
│   │   │   │   ├── futures-intrusive v0.5.0 (*)
│   │   │   │   ├── futures-util v0.3.31
│   │   │   │   │   ├── futures-core v0.3.31
│   │   │   │   │   ├── futures-io v0.3.31
│   │   │   │   │   ├── futures-sink v0.3.31
│   │   │   │   │   ├── futures-task v0.3.31
│   │   │   │   │   ├── memchr v2.7.4
│   │   │   │   │   ├── pin-project-lite v0.2.16
│   │   │   │   │   ├── pin-utils v0.1.0
│   │   │   │   │   └── slab v0.4.9 (*)
│   │   │   │   ├── libsqlite3-sys v0.30.1
│   │   │   │   │   [build-dependencies]
│   │   │   │   │   ├── cc v1.2.20 (*)
│   │   │   │   │   ├── pkg-config v0.3.32
│   │   │   │   │   └── vcpkg v0.2.15
│   │   │   │   ├── log v0.4.27
│   │   │   │   ├── percent-encoding v2.3.1
│   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   ├── serde_urlencoded v0.7.1 (*)
│   │   │   │   ├── sqlx-core v0.8.5 (*)
│   │   │   │   ├── thiserror v2.0.12 (*)
│   │   │   │   ├── time v0.3.41
│   │   │   │   │   ├── deranged v0.4.0
│   │   │   │   │   │   └── powerfmt v0.2.0
│   │   │   │   │   ├── itoa v1.0.15
│   │   │   │   │   ├── num-conv v0.1.0
│   │   │   │   │   ├── powerfmt v0.2.0
│   │   │   │   │   ├── time-core v0.1.4
│   │   │   │   │   └── time-macros v0.2.22 (proc-macro) (*)
│   │   │   │   ├── tracing v0.1.41 (*)
│   │   │   │   └── url v2.5.4 (*)
│   │   │   ├── syn v2.0.101 (*)
│   │   │   ├── tempfile v3.19.1 (*)
│   │   │   ├── tokio v1.44.2
│   │   │   │   ├── bytes v1.10.1
│   │   │   │   ├── libc v0.2.172
│   │   │   │   ├── mio v1.0.3 (*)
│   │   │   │   ├── pin-project-lite v0.2.16
│   │   │   │   └── socket2 v0.5.9 (*)
│   │   │   └── url v2.5.4 (*)
│   │   └── syn v2.0.101 (*)
│   └── sqlx-sqlite v0.8.5
│       ├── atoi v2.0.0 (*)
│       ├── flume v0.11.1 (*)
│       ├── futures-channel v0.3.31 (*)
│       ├── futures-core v0.3.31
│       ├── futures-executor v0.3.31 (*)
│       ├── futures-intrusive v0.5.0 (*)
│       ├── futures-util v0.3.31 (*)
│       ├── libsqlite3-sys v0.30.1 (*)
│       ├── log v0.4.27 (*)
│       ├── percent-encoding v2.3.1
│       ├── serde v1.0.219 (*)
│       ├── serde_urlencoded v0.7.1 (*)
│       ├── sqlx-core v0.8.5 (*)
│       ├── thiserror v2.0.12 (*)
│       ├── time v0.3.41 (*)
│       ├── tracing v0.1.41 (*)
│       └── url v2.5.4 (*)
├── sysinfo v0.32.1
│   ├── core-foundation-sys v0.8.7
│   ├── libc v0.2.172
│   ├── memchr v2.7.4
│   └── rayon v1.10.0
│       ├── either v1.15.0 (*)
│       └── rayon-core v1.12.1
│           ├── crossbeam-deque v0.8.6
│           │   ├── crossbeam-epoch v0.9.18
│           │   │   └── crossbeam-utils v0.8.21
│           │   └── crossbeam-utils v0.8.21
│           └── crossbeam-utils v0.8.21
├── tauri v2.5.1
│   ├── anyhow v1.0.98
│   ├── dirs v6.0.0
│   │   └── dirs-sys v0.5.0
│   │       ├── libc v0.2.172
│   │       └── option-ext v0.2.0
│   ├── dunce v1.0.5
│   ├── embed_plist v1.2.2
│   ├── futures-util v0.3.31 (*)
│   ├── getrandom v0.2.16 (*)
│   ├── glob v0.3.2
│   ├── heck v0.5.0
│   ├── http v1.3.1 (*)
│   ├── http-range v0.1.5
│   ├── log v0.4.27 (*)
│   ├── mime v0.3.17
│   ├── muda v0.16.1
│   │   ├── crossbeam-channel v0.5.15
│   │   │   └── crossbeam-utils v0.8.21
│   │   ├── dpi v0.1.1
│   │   │   └── serde v1.0.219 (*)
│   │   ├── keyboard-types v0.7.0
│   │   │   ├── bitflags v2.9.0 (*)
│   │   │   ├── serde v1.0.219 (*)
│   │   │   └── unicode-segmentation v1.12.0
│   │   ├── objc2 v0.6.1
│   │   │   ├── objc2-encode v4.1.0
│   │   │   └── objc2-exception-helper v0.1.1
│   │   │       [build-dependencies]
│   │   │       └── cc v1.2.20 (*)
│   │   ├── objc2-app-kit v0.3.1
│   │   │   ├── bitflags v2.9.0 (*)
│   │   │   ├── block2 v0.6.1
│   │   │   │   └── objc2 v0.6.1 (*)
│   │   │   ├── libc v0.2.172
│   │   │   ├── objc2 v0.6.1 (*)
│   │   │   ├── objc2-cloud-kit v0.3.1
│   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   ├── objc2 v0.6.1 (*)
│   │   │   │   └── objc2-foundation v0.3.1
│   │   │   │       ├── bitflags v2.9.0 (*)
│   │   │   │       ├── block2 v0.6.1 (*)
│   │   │   │       ├── libc v0.2.172
│   │   │   │       ├── objc2 v0.6.1 (*)
│   │   │   │       └── objc2-core-foundation v0.3.1
│   │   │   │           ├── bitflags v2.9.0 (*)
│   │   │   │           └── objc2 v0.6.1 (*)
│   │   │   ├── objc2-core-data v0.3.1
│   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   ├── objc2 v0.6.1 (*)
│   │   │   │   └── objc2-foundation v0.3.1 (*)
│   │   │   ├── objc2-core-foundation v0.3.1 (*)
│   │   │   ├── objc2-core-graphics v0.3.1
│   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   ├── objc2 v0.6.1 (*)
│   │   │   │   └── objc2-core-foundation v0.3.1 (*)
│   │   │   ├── objc2-core-image v0.3.1
│   │   │   │   ├── objc2 v0.6.1 (*)
│   │   │   │   └── objc2-foundation v0.3.1 (*)
│   │   │   ├── objc2-foundation v0.3.1 (*)
│   │   │   └── objc2-quartz-core v0.3.1
│   │   │       ├── bitflags v2.9.0 (*)
│   │   │       ├── objc2 v0.6.1 (*)
│   │   │       └── objc2-foundation v0.3.1 (*)
│   │   ├── objc2-core-foundation v0.3.1 (*)
│   │   ├── objc2-foundation v0.3.1 (*)
│   │   ├── once_cell v1.21.3
│   │   ├── png v0.17.16
│   │   │   ├── bitflags v1.3.2
│   │   │   ├── crc32fast v1.4.2 (*)
│   │   │   ├── fdeflate v0.3.7
│   │   │   │   └── simd-adler32 v0.3.7
│   │   │   ├── flate2 v1.1.1 (*)
│   │   │   └── miniz_oxide v0.8.8 (*)
│   │   ├── serde v1.0.219 (*)
│   │   └── thiserror v2.0.12 (*)
│   ├── objc2 v0.6.1 (*)
│   ├── objc2-app-kit v0.3.1 (*)
│   ├── objc2-foundation v0.3.1 (*)
│   ├── percent-encoding v2.3.1
│   ├── plist v1.7.1
│   │   ├── base64 v0.22.1
│   │   ├── indexmap v2.9.0 (*)
│   │   ├── quick-xml v0.32.0
│   │   │   └── memchr v2.7.4
│   │   ├── serde v1.0.219 (*)
│   │   └── time v0.3.41 (*)
│   ├── raw-window-handle v0.6.2
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── serde_repr v0.1.20 (proc-macro)
│   │   ├── proc-macro2 v1.0.95 (*)
│   │   ├── quote v1.0.40 (*)
│   │   └── syn v2.0.101 (*)
│   ├── serialize-to-javascript v0.1.1
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   └── serialize-to-javascript-impl v0.1.1 (proc-macro)
│   │       ├── proc-macro2 v1.0.95 (*)
│   │       ├── quote v1.0.40 (*)
│   │       └── syn v1.0.109
│   │           ├── proc-macro2 v1.0.95 (*)
│   │           ├── quote v1.0.40 (*)
│   │           └── unicode-ident v1.0.18
│   ├── tauri-macros v2.2.0 (proc-macro)
│   │   ├── heck v0.5.0
│   │   ├── proc-macro2 v1.0.95 (*)
│   │   ├── quote v1.0.40 (*)
│   │   ├── syn v2.0.101 (*)
│   │   ├── tauri-codegen v2.2.0
│   │   │   ├── base64 v0.22.1
│   │   │   ├── brotli v7.0.0
│   │   │   │   ├── alloc-no-stdlib v2.0.4
│   │   │   │   ├── alloc-stdlib v0.2.2 (*)
│   │   │   │   └── brotli-decompressor v4.0.3
│   │   │   │       ├── alloc-no-stdlib v2.0.4
│   │   │   │       └── alloc-stdlib v0.2.2 (*)
│   │   │   ├── ico v0.4.0
│   │   │   │   ├── byteorder v1.5.0
│   │   │   │   └── png v0.17.16 (*)
│   │   │   ├── json-patch v3.0.1
│   │   │   │   ├── jsonptr v0.6.3
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   └── serde_json v1.0.140 (*)
│   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   ├── serde_json v1.0.140 (*)
│   │   │   │   └── thiserror v1.0.69 (*)
│   │   │   ├── plist v1.7.1 (*)
│   │   │   ├── png v0.17.16 (*)
│   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   ├── quote v1.0.40 (*)
│   │   │   ├── semver v1.0.26
│   │   │   │   └── serde v1.0.219 (*)
│   │   │   ├── serde v1.0.219 (*)
│   │   │   ├── serde_json v1.0.140 (*)
│   │   │   ├── sha2 v0.10.8 (*)
│   │   │   ├── syn v2.0.101 (*)
│   │   │   ├── tauri-utils v2.4.0
│   │   │   │   ├── anyhow v1.0.98
│   │   │   │   ├── brotli v7.0.0 (*)
│   │   │   │   ├── cargo_metadata v0.19.2
│   │   │   │   │   ├── camino v1.1.9
│   │   │   │   │   │   └── serde v1.0.219 (*)
│   │   │   │   │   ├── cargo-platform v0.1.9
│   │   │   │   │   │   └── serde v1.0.219 (*)
│   │   │   │   │   ├── semver v1.0.26 (*)
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   ├── serde_json v1.0.140 (*)
│   │   │   │   │   └── thiserror v2.0.12 (*)
│   │   │   │   ├── ctor v0.2.9 (proc-macro)
│   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   ├── dunce v1.0.5
│   │   │   │   ├── glob v0.3.2
│   │   │   │   ├── html5ever v0.26.0
│   │   │   │   │   ├── log v0.4.27
│   │   │   │   │   ├── mac v0.1.1
│   │   │   │   │   └── markup5ever v0.11.0
│   │   │   │   │       ├── log v0.4.27
│   │   │   │   │       ├── phf v0.10.1
│   │   │   │   │       │   └── phf_shared v0.10.0
│   │   │   │   │       │       └── siphasher v0.3.11
│   │   │   │   │       ├── string_cache v0.8.9
│   │   │   │   │       │   ├── new_debug_unreachable v1.0.6
│   │   │   │   │       │   ├── parking_lot v0.12.3 (*)
│   │   │   │   │       │   ├── phf_shared v0.11.3
│   │   │   │   │       │   │   └── siphasher v1.0.1
│   │   │   │   │       │   ├── precomputed-hash v0.1.1
│   │   │   │   │       │   └── serde v1.0.219 (*)
│   │   │   │   │       └── tendril v0.4.3
│   │   │   │   │           ├── futf v0.1.5
│   │   │   │   │           │   ├── mac v0.1.1
│   │   │   │   │           │   └── new_debug_unreachable v1.0.6
│   │   │   │   │           ├── mac v0.1.1
│   │   │   │   │           └── utf-8 v0.7.6
│   │   │   │   │       [build-dependencies]
│   │   │   │   │       ├── phf_codegen v0.10.0
│   │   │   │   │       │   ├── phf_generator v0.10.0
│   │   │   │   │       │   │   ├── phf_shared v0.10.0 (*)
│   │   │   │   │       │   │   └── rand v0.8.5
│   │   │   │   │       │   │       ├── libc v0.2.172
│   │   │   │   │       │   │       ├── rand_chacha v0.3.1 (*)
│   │   │   │   │       │   │       └── rand_core v0.6.4 (*)
│   │   │   │   │       │   └── phf_shared v0.10.0 (*)
│   │   │   │   │       └── string_cache_codegen v0.5.4
│   │   │   │   │           ├── phf_generator v0.11.3
│   │   │   │   │           │   ├── phf_shared v0.11.3 (*)
│   │   │   │   │           │   └── rand v0.8.5 (*)
│   │   │   │   │           ├── phf_shared v0.11.3 (*)
│   │   │   │   │           ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │           └── quote v1.0.40 (*)
│   │   │   │   │   [build-dependencies]
│   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   └── syn v1.0.109 (*)
│   │   │   │   ├── http v1.3.1 (*)
│   │   │   │   ├── infer v0.19.0
│   │   │   │   │   └── cfb v0.7.3
│   │   │   │   │       ├── byteorder v1.5.0
│   │   │   │   │       ├── fnv v1.0.7
│   │   │   │   │       └── uuid v1.16.0
│   │   │   │   │           ├── getrandom v0.3.2 (*)
│   │   │   │   │           └── serde v1.0.219 (*)
│   │   │   │   ├── json-patch v3.0.1 (*)
│   │   │   │   ├── kuchikiki v0.8.2
│   │   │   │   │   ├── cssparser v0.27.2
│   │   │   │   │   │   ├── cssparser-macros v0.6.1 (proc-macro)
│   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   │   ├── dtoa-short v0.3.5
│   │   │   │   │   │   │   └── dtoa v1.0.10
│   │   │   │   │   │   ├── itoa v0.4.8
│   │   │   │   │   │   ├── matches v0.1.10
│   │   │   │   │   │   ├── phf v0.8.0
│   │   │   │   │   │   │   ├── phf_macros v0.8.0 (proc-macro)
│   │   │   │   │   │   │   │   ├── phf_generator v0.8.0
│   │   │   │   │   │   │   │   │   ├── phf_shared v0.8.0
│   │   │   │   │   │   │   │   │   │   └── siphasher v0.3.11
│   │   │   │   │   │   │   │   │   └── rand v0.7.3
│   │   │   │   │   │   │   │   │       ├── getrandom v0.1.16
│   │   │   │   │   │   │   │   │       │   ├── cfg-if v1.0.0
│   │   │   │   │   │   │   │   │       │   └── libc v0.2.172
│   │   │   │   │   │   │   │   │       ├── libc v0.2.172
│   │   │   │   │   │   │   │   │       ├── rand_chacha v0.2.2
│   │   │   │   │   │   │   │   │       │   ├── ppv-lite86 v0.2.21 (*)
│   │   │   │   │   │   │   │   │       │   └── rand_core v0.5.1
│   │   │   │   │   │   │   │   │       │       └── getrandom v0.1.16 (*)
│   │   │   │   │   │   │   │   │       ├── rand_core v0.5.1 (*)
│   │   │   │   │   │   │   │   │       └── rand_pcg v0.2.1
│   │   │   │   │   │   │   │   │           └── rand_core v0.5.1 (*)
│   │   │   │   │   │   │   │   ├── phf_shared v0.8.0 (*)
│   │   │   │   │   │   │   │   ├── proc-macro-hack v0.5.20+deprecated (proc-macro)
│   │   │   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   │   └── syn v1.0.109 (*)
│   │   │   │   │   │   │   ├── phf_shared v0.8.0 (*)
│   │   │   │   │   │   │   └── proc-macro-hack v0.5.20+deprecated (proc-macro)
│   │   │   │   │   │   └── smallvec v1.15.0
│   │   │   │   │   │   [build-dependencies]
│   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   └── syn v1.0.109 (*)
│   │   │   │   │   ├── html5ever v0.26.0 (*)
│   │   │   │   │   ├── indexmap v1.9.3
│   │   │   │   │   │   ├── hashbrown v0.12.3
│   │   │   │   │   │   └── serde v1.0.219 (*)
│   │   │   │   │   │   [build-dependencies]
│   │   │   │   │   │   └── autocfg v1.4.0
│   │   │   │   │   ├── matches v0.1.10
│   │   │   │   │   └── selectors v0.22.0
│   │   │   │   │       ├── bitflags v1.3.2
│   │   │   │   │       ├── cssparser v0.27.2 (*)
│   │   │   │   │       ├── derive_more v0.99.20 (proc-macro)
│   │   │   │   │       │   ├── convert_case v0.4.0
│   │   │   │   │       │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │       │   ├── quote v1.0.40 (*)
│   │   │   │   │       │   └── syn v2.0.101 (*)
│   │   │   │   │       │   [build-dependencies]
│   │   │   │   │       │   └── rustc_version v0.4.1
│   │   │   │   │       │       └── semver v1.0.26 (*)
│   │   │   │   │       ├── fxhash v0.2.1
│   │   │   │   │       │   └── byteorder v1.5.0
│   │   │   │   │       ├── log v0.4.27
│   │   │   │   │       ├── matches v0.1.10
│   │   │   │   │       ├── phf v0.8.0 (*)
│   │   │   │   │       ├── precomputed-hash v0.1.1
│   │   │   │   │       ├── servo_arc v0.1.1
│   │   │   │   │       │   ├── nodrop v0.1.14
│   │   │   │   │       │   └── stable_deref_trait v1.2.0
│   │   │   │   │       ├── smallvec v1.15.0
│   │   │   │   │       └── thin-slice v0.1.1
│   │   │   │   │       [build-dependencies]
│   │   │   │   │       └── phf_codegen v0.8.0
│   │   │   │   │           ├── phf_generator v0.8.0 (*)
│   │   │   │   │           └── phf_shared v0.8.0 (*)
│   │   │   │   ├── log v0.4.27
│   │   │   │   ├── memchr v2.7.4
│   │   │   │   ├── phf v0.11.3
│   │   │   │   │   ├── phf_macros v0.11.3 (proc-macro)
│   │   │   │   │   │   ├── phf_generator v0.11.3 (*)
│   │   │   │   │   │   ├── phf_shared v0.11.3 (*)
│   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   └── phf_shared v0.11.3 (*)
│   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   ├── regex v1.11.1 (*)
│   │   │   │   ├── schemars v0.8.22
│   │   │   │   │   ├── dyn-clone v1.0.19
│   │   │   │   │   ├── indexmap v1.9.3 (*)
│   │   │   │   │   ├── schemars_derive v0.8.22 (proc-macro)
│   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   ├── serde_derive_internals v0.29.1
│   │   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   ├── serde_json v1.0.140 (*)
│   │   │   │   │   ├── url v2.5.4 (*)
│   │   │   │   │   └── uuid v1.16.0 (*)
│   │   │   │   ├── semver v1.0.26 (*)
│   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   ├── serde-untagged v0.1.7
│   │   │   │   │   ├── erased-serde v0.4.6
│   │   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   │   └── typeid v1.0.3
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   └── typeid v1.0.3
│   │   │   │   ├── serde_json v1.0.140 (*)
│   │   │   │   ├── serde_with v3.12.0
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   ├── serde_derive v1.0.219 (proc-macro) (*)
│   │   │   │   │   └── serde_with_macros v3.12.0 (proc-macro)
│   │   │   │   │       ├── darling v0.20.11
│   │   │   │   │       │   ├── darling_core v0.20.11
│   │   │   │   │       │   │   ├── fnv v1.0.7
│   │   │   │   │       │   │   ├── ident_case v1.0.1
│   │   │   │   │       │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │       │   │   ├── quote v1.0.40 (*)
│   │   │   │   │       │   │   ├── strsim v0.11.1
│   │   │   │   │       │   │   └── syn v2.0.101 (*)
│   │   │   │   │       │   └── darling_macro v0.20.11 (proc-macro)
│   │   │   │   │       │       ├── darling_core v0.20.11 (*)
│   │   │   │   │       │       ├── quote v1.0.40 (*)
│   │   │   │   │       │       └── syn v2.0.101 (*)
│   │   │   │   │       ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │       ├── quote v1.0.40 (*)
│   │   │   │   │       └── syn v2.0.101 (*)
│   │   │   │   ├── swift-rs v1.0.7
│   │   │   │   │   ├── base64 v0.21.7
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   └── serde_json v1.0.140 (*)
│   │   │   │   │   [build-dependencies]
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   └── serde_json v1.0.140 (*)
│   │   │   │   ├── thiserror v2.0.12 (*)
│   │   │   │   ├── toml v0.8.22
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   ├── serde_spanned v0.6.8
│   │   │   │   │   │   └── serde v1.0.219 (*)
│   │   │   │   │   ├── toml_datetime v0.6.9
│   │   │   │   │   │   └── serde v1.0.219 (*)
│   │   │   │   │   └── toml_edit v0.22.26
│   │   │   │   │       ├── indexmap v2.9.0
│   │   │   │   │       │   ├── equivalent v1.0.2
│   │   │   │   │       │   └── hashbrown v0.15.2 (*)
│   │   │   │   │       ├── serde v1.0.219 (*)
│   │   │   │   │       ├── serde_spanned v0.6.8 (*)
│   │   │   │   │       ├── toml_datetime v0.6.9 (*)
│   │   │   │   │       ├── toml_write v0.1.1
│   │   │   │   │       └── winnow v0.7.7
│   │   │   │   ├── url v2.5.4 (*)
│   │   │   │   ├── urlpattern v0.3.0
│   │   │   │   │   ├── regex v1.11.1 (*)
│   │   │   │   │   ├── serde v1.0.219 (*)
│   │   │   │   │   ├── unic-ucd-ident v0.9.0
│   │   │   │   │   │   ├── unic-char-property v0.9.0
│   │   │   │   │   │   │   └── unic-char-range v0.9.0
│   │   │   │   │   │   ├── unic-char-range v0.9.0
│   │   │   │   │   │   └── unic-ucd-version v0.9.0
│   │   │   │   │   │       └── unic-common v0.9.0
│   │   │   │   │   └── url v2.5.4 (*)
│   │   │   │   ├── uuid v1.16.0 (*)
│   │   │   │   └── walkdir v2.5.0
│   │   │   │       └── same-file v1.0.6
│   │   │   ├── thiserror v2.0.12 (*)
│   │   │   ├── time v0.3.41 (*)
│   │   │   ├── url v2.5.4 (*)
│   │   │   ├── uuid v1.16.0 (*)
│   │   │   └── walkdir v2.5.0 (*)
│   │   └── tauri-utils v2.4.0 (*)
│   ├── tauri-runtime v2.6.0
│   │   ├── cookie v0.18.1 (*)
│   │   ├── dpi v0.1.1 (*)
│   │   ├── http v1.3.1 (*)
│   │   ├── raw-window-handle v0.6.2
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   ├── tauri-utils v2.4.0
│   │   │   ├── anyhow v1.0.98
│   │   │   ├── brotli v7.0.0 (*)
│   │   │   ├── ctor v0.2.9 (proc-macro) (*)
│   │   │   ├── dunce v1.0.5
│   │   │   ├── glob v0.3.2
│   │   │   ├── html5ever v0.26.0 (*)
│   │   │   ├── http v1.3.1 (*)
│   │   │   ├── infer v0.19.0 (*)
│   │   │   ├── json-patch v3.0.1 (*)
│   │   │   ├── kuchikiki v0.8.2 (*)
│   │   │   ├── log v0.4.27 (*)
│   │   │   ├── memchr v2.7.4
│   │   │   ├── phf v0.11.3 (*)
│   │   │   ├── regex v1.11.1 (*)
│   │   │   ├── semver v1.0.26
│   │   │   ├── serde v1.0.219 (*)
│   │   │   ├── serde-untagged v0.1.7 (*)
│   │   │   ├── serde_json v1.0.140 (*)
│   │   │   ├── serde_with v3.12.0 (*)
│   │   │   ├── thiserror v2.0.12 (*)
│   │   │   ├── toml v0.8.22 (*)
│   │   │   ├── url v2.5.4 (*)
│   │   │   ├── urlpattern v0.3.0 (*)
│   │   │   ├── uuid v1.16.0 (*)
│   │   │   └── walkdir v2.5.0 (*)
│   │   ├── thiserror v2.0.12 (*)
│   │   └── url v2.5.4 (*)
│   ├── tauri-runtime-wry v2.6.0
│   │   ├── http v1.3.1 (*)
│   │   ├── log v0.4.27 (*)
│   │   ├── objc2 v0.6.1 (*)
│   │   ├── objc2-app-kit v0.3.1 (*)
│   │   ├── objc2-foundation v0.3.1 (*)
│   │   ├── raw-window-handle v0.6.2
│   │   ├── tao v0.33.0
│   │   │   ├── bitflags v2.9.0 (*)
│   │   │   ├── core-foundation v0.10.0 (*)
│   │   │   ├── core-graphics v0.24.0
│   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   ├── core-foundation v0.10.0 (*)
│   │   │   │   ├── core-graphics-types v0.2.0
│   │   │   │   │   ├── bitflags v2.9.0 (*)
│   │   │   │   │   ├── core-foundation v0.10.0 (*)
│   │   │   │   │   └── libc v0.2.172
│   │   │   │   ├── foreign-types v0.5.0
│   │   │   │   │   ├── foreign-types-macros v0.2.3 (proc-macro)
│   │   │   │   │   │   ├── proc-macro2 v1.0.95 (*)
│   │   │   │   │   │   ├── quote v1.0.40 (*)
│   │   │   │   │   │   └── syn v2.0.101 (*)
│   │   │   │   │   └── foreign-types-shared v0.3.1
│   │   │   │   └── libc v0.2.172
│   │   │   ├── crossbeam-channel v0.5.15 (*)
│   │   │   ├── dispatch v0.2.0
│   │   │   ├── dpi v0.1.1 (*)
│   │   │   ├── lazy_static v1.5.0
│   │   │   ├── libc v0.2.172
│   │   │   ├── log v0.4.27 (*)
│   │   │   ├── objc2 v0.6.1 (*)
│   │   │   ├── objc2-app-kit v0.3.1 (*)
│   │   │   ├── objc2-foundation v0.3.1 (*)
│   │   │   ├── raw-window-handle v0.6.2
│   │   │   ├── scopeguard v1.2.0
│   │   │   └── url v2.5.4 (*)
│   │   ├── tauri-runtime v2.6.0 (*)
│   │   ├── tauri-utils v2.4.0 (*)
│   │   ├── url v2.5.4 (*)
│   │   └── wry v0.51.2
│   │       ├── block2 v0.6.1 (*)
│   │       ├── cookie v0.18.1 (*)
│   │       ├── dpi v0.1.1 (*)
│   │       ├── http v1.3.1 (*)
│   │       ├── objc2 v0.6.1 (*)
│   │       ├── objc2-app-kit v0.3.1 (*)
│   │       ├── objc2-core-foundation v0.3.1 (*)
│   │       ├── objc2-foundation v0.3.1 (*)
│   │       ├── objc2-web-kit v0.3.1
│   │       │   ├── bitflags v2.9.0 (*)
│   │       │   ├── block2 v0.6.1 (*)
│   │       │   ├── objc2 v0.6.1 (*)
│   │       │   ├── objc2-app-kit v0.3.1 (*)
│   │       │   ├── objc2-core-foundation v0.3.1 (*)
│   │       │   └── objc2-foundation v0.3.1 (*)
│   │       ├── once_cell v1.21.3
│   │       ├── raw-window-handle v0.6.2
│   │       ├── thiserror v2.0.12 (*)
│   │       └── url v2.5.4 (*)
│   ├── tauri-utils v2.4.0 (*)
│   ├── thiserror v2.0.12 (*)
│   ├── tokio v1.44.2 (*)
│   ├── tray-icon v0.20.1
│   │   ├── crossbeam-channel v0.5.15 (*)
│   │   ├── muda v0.16.1 (*)
│   │   ├── objc2 v0.6.1 (*)
│   │   ├── objc2-app-kit v0.3.1 (*)
│   │   ├── objc2-core-foundation v0.3.1 (*)
│   │   ├── objc2-core-graphics v0.3.1 (*)
│   │   ├── objc2-foundation v0.3.1 (*)
│   │   ├── once_cell v1.21.3
│   │   ├── png v0.17.16 (*)
│   │   ├── serde v1.0.219 (*)
│   │   └── thiserror v2.0.12 (*)
│   ├── url v2.5.4 (*)
│   ├── urlpattern v0.3.0 (*)
│   └── window-vibrancy v0.6.0
│       ├── objc2 v0.6.1 (*)
│       ├── objc2-app-kit v0.3.1 (*)
│       ├── objc2-core-foundation v0.3.1 (*)
│       ├── objc2-foundation v0.3.1 (*)
│       └── raw-window-handle v0.6.2
│   [build-dependencies]
│   ├── glob v0.3.2
│   ├── heck v0.5.0
│   ├── tauri-build v2.2.0
│   │   ├── anyhow v1.0.98
│   │   ├── cargo_toml v0.22.1
│   │   │   ├── serde v1.0.219 (*)
│   │   │   └── toml v0.8.22 (*)
│   │   ├── dirs v6.0.0 (*)
│   │   ├── glob v0.3.2
│   │   ├── heck v0.5.0
│   │   ├── json-patch v3.0.1 (*)
│   │   ├── schemars v0.8.22 (*)
│   │   ├── semver v1.0.26 (*)
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   ├── tauri-utils v2.4.0 (*)
│   │   ├── tauri-winres v0.3.1
│   │   │   ├── embed-resource v3.0.2
│   │   │   │   ├── cc v1.2.20 (*)
│   │   │   │   ├── memchr v2.7.4
│   │   │   │   ├── rustc_version v0.4.1 (*)
│   │   │   │   └── toml v0.8.22 (*)
│   │   │   ├── indexmap v2.9.0 (*)
│   │   │   └── toml v0.8.22 (*)
│   │   ├── toml v0.8.22 (*)
│   │   └── walkdir v2.5.0 (*)
│   └── tauri-utils v2.4.0 (*)
├── tauri-plugin-dialog v2.2.1
│   ├── log v0.4.27 (*)
│   ├── raw-window-handle v0.6.2
│   ├── rfd v0.15.3
│   │   ├── block2 v0.6.1 (*)
│   │   ├── dispatch2 v0.2.0
│   │   │   ├── bitflags v2.9.0 (*)
│   │   │   ├── block2 v0.6.1 (*)
│   │   │   ├── libc v0.2.172
│   │   │   └── objc2 v0.6.1 (*)
│   │   ├── log v0.4.27 (*)
│   │   ├── objc2 v0.6.1 (*)
│   │   ├── objc2-app-kit v0.3.1 (*)
│   │   ├── objc2-core-foundation v0.3.1 (*)
│   │   ├── objc2-foundation v0.3.1 (*)
│   │   └── raw-window-handle v0.6.2
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── tauri v2.5.1 (*)
│   ├── tauri-plugin-fs v2.2.1
│   │   ├── anyhow v1.0.98
│   │   ├── dunce v1.0.5
│   │   ├── glob v0.3.2
│   │   ├── percent-encoding v2.3.1
│   │   ├── serde v1.0.219 (*)
│   │   ├── serde_json v1.0.140 (*)
│   │   ├── serde_repr v0.1.20 (proc-macro) (*)
│   │   ├── tauri v2.5.1 (*)
│   │   ├── thiserror v2.0.12 (*)
│   │   ├── url v2.5.4 (*)
│   │   └── uuid v1.16.0 (*)
│   │   [build-dependencies]
│   │   ├── schemars v0.8.22 (*)
│   │   ├── serde v1.0.219 (*)
│   │   ├── tauri-plugin v2.2.0
│   │   │   ├── anyhow v1.0.98
│   │   │   ├── glob v0.3.2
│   │   │   ├── plist v1.7.1 (*)
│   │   │   ├── schemars v0.8.22 (*)
│   │   │   ├── serde v1.0.219 (*)
│   │   │   ├── serde_json v1.0.140 (*)
│   │   │   ├── tauri-utils v2.4.0 (*)
│   │   │   ├── toml v0.8.22 (*)
│   │   │   └── walkdir v2.5.0 (*)
│   │   ├── tauri-utils v2.4.0 (*)
│   │   └── toml v0.8.22 (*)
│   ├── thiserror v2.0.12 (*)
│   └── url v2.5.4 (*)
│   [build-dependencies]
│   └── tauri-plugin v2.2.0 (*)
├── tauri-plugin-fs v2.2.1 (*)
├── tauri-plugin-http v2.4.3
│   ├── bytes v1.10.1
│   ├── cookie_store v0.21.1 (*)
│   ├── data-url v0.3.1
│   ├── http v1.3.1 (*)
│   ├── regex v1.11.1 (*)
│   ├── reqwest v0.12.15 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── tauri v2.5.1 (*)
│   ├── tauri-plugin-fs v2.2.1 (*)
│   ├── thiserror v2.0.12 (*)
│   ├── tokio v1.44.2 (*)
│   ├── url v2.5.4 (*)
│   └── urlpattern v0.3.0 (*)
│   [build-dependencies]
│   ├── regex v1.11.1 (*)
│   ├── schemars v0.8.22 (*)
│   ├── serde v1.0.219 (*)
│   ├── tauri-plugin v2.2.0 (*)
│   ├── url v2.5.4 (*)
│   └── urlpattern v0.3.0 (*)
├── tauri-plugin-notification v2.2.2
│   ├── log v0.4.27 (*)
│   ├── notify-rust v4.11.7
│   │   ├── futures-lite v2.6.0 (*)
│   │   └── mac-notification-sys v0.6.4
│   │       ├── objc2 v0.6.1 (*)
│   │       ├── objc2-foundation v0.3.1 (*)
│   │       └── time v0.3.41 (*)
│   │       [build-dependencies]
│   │       └── cc v1.2.20 (*)
│   ├── rand v0.8.5 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── serde_repr v0.1.20 (proc-macro) (*)
│   ├── tauri v2.5.1 (*)
│   ├── thiserror v2.0.12 (*)
│   ├── time v0.3.41 (*)
│   └── url v2.5.4 (*)
│   [build-dependencies]
│   └── tauri-plugin v2.2.0 (*)
├── tauri-plugin-os v2.2.1
│   ├── gethostname v1.0.1
│   │   └── rustix v1.0.5 (*)
│   ├── log v0.4.27 (*)
│   ├── os_info v3.10.0
│   │   ├── log v0.4.27 (*)
│   │   └── serde v1.0.219 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── serialize-to-javascript v0.1.1 (*)
│   ├── sys-locale v0.3.2
│   ├── tauri v2.5.1 (*)
│   └── thiserror v2.0.12 (*)
│   [build-dependencies]
│   └── tauri-plugin v2.2.0 (*)
├── tauri-plugin-shell v2.2.1
│   ├── encoding_rs v0.8.35 (*)
│   ├── log v0.4.27 (*)
│   ├── open v5.3.2
│   │   ├── libc v0.2.172
│   │   └── pathdiff v0.2.3
│   ├── os_pipe v1.2.1
│   │   └── libc v0.2.172
│   ├── regex v1.11.1 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── shared_child v1.0.1
│   │   └── libc v0.2.172
│   ├── tauri v2.5.1 (*)
│   ├── thiserror v2.0.12 (*)
│   └── tokio v1.44.2 (*)
│   [build-dependencies]
│   ├── schemars v0.8.22 (*)
│   ├── serde v1.0.219 (*)
│   └── tauri-plugin v2.2.0 (*)
├── tauri-plugin-single-instance v2.2.3
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── tauri v2.5.1 (*)
│   ├── thiserror v2.0.12 (*)
│   └── tracing v0.1.41 (*)
├── tauri-plugin-sql v2.2.0
│   ├── futures-core v0.3.31
│   ├── indexmap v2.9.0 (*)
│   ├── log v0.4.27 (*)
│   ├── serde v1.0.219 (*)
│   ├── serde_json v1.0.140 (*)
│   ├── sqlx v0.8.5 (*)
│   ├── tauri v2.5.1 (*)
│   ├── thiserror v2.0.12 (*)
│   ├── time v0.3.41 (*)
│   └── tokio v1.44.2 (*)
│   [build-dependencies]
│   └── tauri-plugin v2.2.0 (*)
├── tauri-utils v2.4.0 (*)
├── tokio v1.44.2 (*)
├── tokio-util v0.7.15 (*)
├── toml v0.7.8
│   ├── serde v1.0.219 (*)
│   ├── serde_spanned v0.6.8 (*)
│   ├── toml_datetime v0.6.9 (*)
│   └── toml_edit v0.19.15
│       ├── indexmap v2.9.0 (*)
│       ├── serde v1.0.219 (*)
│       ├── serde_spanned v0.6.8 (*)
│       ├── toml_datetime v0.6.9 (*)
│       └── winnow v0.5.40
├── tower-http v0.5.2
│   ├── bitflags v2.9.0 (*)
│   ├── bytes v1.10.1
│   ├── futures-util v0.3.31 (*)
│   ├── http v1.3.1 (*)
│   ├── http-body v1.0.1 (*)
│   ├── http-body-util v0.1.3 (*)
│   ├── http-range-header v0.4.2
│   ├── httpdate v1.0.3
│   ├── mime v0.3.17
│   ├── mime_guess v2.0.5 (*)
│   ├── percent-encoding v2.3.1
│   ├── pin-project-lite v0.2.16
│   ├── tokio v1.44.2 (*)
│   ├── tokio-util v0.7.15 (*)
│   ├── tower-layer v0.3.3
│   ├── tower-service v0.3.3
│   └── tracing v0.1.41 (*)
├── url v2.5.4 (*)
├── urlencoding v2.1.3
├── uuid v1.16.0 (*)
└── whisper-rs v0.14.2
    └── whisper-rs-sys v0.12.1
        [build-dependencies]
        ├── bindgen v0.71.1
        │   ├── bitflags v2.9.0
        │   ├── cexpr v0.6.0
        │   │   └── nom v7.1.3
        │   │       ├── memchr v2.7.4
        │   │       └── minimal-lexical v0.2.1
        │   ├── clang-sys v1.8.1
        │   │   ├── glob v0.3.2
        │   │   ├── libc v0.2.172
        │   │   └── libloading v0.8.6
        │   │       └── cfg-if v1.0.0
        │   │   [build-dependencies]
        │   │   └── glob v0.3.2
        │   ├── itertools v0.13.0
        │   │   └── either v1.15.0 (*)
        │   ├── log v0.4.27
        │   ├── prettyplease v0.2.32
        │   │   ├── proc-macro2 v1.0.95 (*)
        │   │   └── syn v2.0.101 (*)
        │   ├── proc-macro2 v1.0.95 (*)
        │   ├── quote v1.0.40 (*)
        │   ├── regex v1.11.1 (*)
        │   ├── rustc-hash v2.1.1
        │   ├── shlex v1.3.0
        │   └── syn v2.0.101 (*)
        ├── cfg-if v1.0.0
        ├── cmake v0.1.54
        │   └── cc v1.2.20 (*)
        └── fs_extra v1.3.0
[build-dependencies]
└── tauri-build v2.2.0 (*)
