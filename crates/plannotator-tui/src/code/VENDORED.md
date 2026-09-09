# Vendored syntax definitions

`default-syntaxes` ships 75 syntaxes and includes neither HCL/Terraform nor TOML, which are two of
the languages this reviewer sees most (terragrunt and infra work). Both are embedded with
`include_str!` and added to the syntax set at startup by `code/highlight.rs`.

| File | Upstream | Licence |
|---|---|---|
| `TOML.sublime-syntax` | [sublimehq/Packages](https://github.com/sublimehq/Packages) `TOML/TOML.sublime-syntax`, fetched 2026-09-08 | Permissive: "Permission to copy, use, modify, sell and distribute this software is granted... provided as is without express or implied warranty." (repository `LICENSE`) |
| `Terraform.sublime-syntax` | [alexlouden/Terraform.tmLanguage](https://github.com/alexlouden/Terraform.tmLanguage) `Terraform.sublime-syntax`, fetched 2026-09-08 | MIT |

Both are unmodified. The Terraform file declares Oniguruma regexes; they were checked to compile
under `fancy-regex`, which is the engine this crate uses, because `regex-onig` would pull a C
dependency. `hcl` is not among Terraform's declared extensions, so `highlight.rs` maps that token
onto this syntax itself rather than editing a vendored file.
