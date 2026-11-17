# Vendored crates

`zune-jpeg` is crates.io release 0.5.15, unchanged, from source commit `31d81fed7551c8ccea456d9d8e2b1fd8bebb6995` (see its `.cargo_vcs_info.json`). plaingram decodes photos from strangers with it, built without its `x86` and `neon` SIMD features so the crate is `#![forbid(unsafe_code)]`. GPUI and resvg depend on the crates.io copy with those features on, and Cargo merges features within one copy, so plaingram keeps its own: nothing else can depend on this one.
