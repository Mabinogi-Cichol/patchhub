# OpenCC character tables

`STCharacters.txt` and `TSCharacters.txt` are copied verbatim from
[OpenCC v1.1.9](https://github.com/BYVoid/OpenCC/tree/ver.1.1.9/data/dictionary)
(original origin: `apps/decky/backend/vendor/opencc/`). They are licensed under
the bundled Apache-2.0 [`LICENSE`](LICENSE).

`crates/core/src/sources/identity.rs` embeds these files with `include_str!` and
uses each character's first listed conversion to expand search terms. It is not
a complete OpenCC phrase-conversion implementation, and converted terms never
establish game identity. Bundling here keeps the core crate independent of the
Decky tree at build time.
