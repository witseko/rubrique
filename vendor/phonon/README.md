# Phonon dependency patches

The published CPU math and tokenizer crates used by `ptts` still depend
on `paste`, which is unmaintained. These copies use `pastey` 0.2.3 through
the existing `paste` dependency alias. Each crate has one manifest change.
The original Rust source and licenses come from the crates.io archives.
[RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2024-0436.html)

| Crate | Version | SHA-256 of the original archive |
| --- | --- | --- |
| pulp | 0.22.3 | `046aa45b989642ec2e4717c8e72d677b13edd831a4d3b6cf37d9a3e54912496a` |
| tokenizers | 0.23.2 | `7afbf6e88718afcc138bad01d6ccc3051dbbc3b2ce9793d8b8a3aeb610969cfc` |
| xn-gemm | 0.20.2 | `0f5bcb177032cca83eb7aae3de9d6fa9cc5cbb29a0090bb07ef8880cf82e3c7d` |
| xn-gemm-common | 0.20.2 | `c22f986c37636c5cbc0a6b4e120a5315ef8ba25615504c554a075e3b3286f642` |
| xn-gemm-c32 | 0.20.2 | `9137ec88a81f5510045a45b1d5fce27b67f9748b8d83400b39163a51ecae6f04` |
| xn-gemm-c64 | 0.20.2 | `e9ab9a0de939c339098e4b20da0aa863f66dfbf1a4b0396dff9a13645443d510` |
| xn-gemm-f16 | 0.20.2 | `c971a82caab9a5960a7c3a6c32bf0c5b6bf5b76492f403997825a4d308b7f233` |
| xn-gemm-f32 | 0.20.2 | `56b8b45a6a11b6d9fedcea8be60bb175c1278170484e7956b29bae507e3c3039` |
| xn-gemm-f64 | 0.20.2 | `2be9f43b557116575751164c7ce2c9e1a7a5bde9e944843bf87ca3314da7eb41` |

When `ptts` can resolve to published dependencies without `paste`, remove
the corresponding root Cargo patches and update `Cargo.lock`. Run
`cargo deny --locked --all-features check` and the offline playback test.

The Security workflow also checks these exact versions against their
crates.io advisory records. Cargo Deny skips such records for local path
dependencies, so the extra check adds the upstream source to a temporary
copy of Cargo metadata. Cargo still builds from the local copies.
