# Supported Languages

Vera supports 65 languages and file formats. Each file is detected by extension (or filename for extensionless files like `Dockerfile` and `Makefile`).

Most languages with a tree-sitter grammar get symbol-level chunking: functions, classes, structs, and methods are extracted as discrete search results. reStructuredText is also parsed with tree-sitter, but chunked by section headings instead of code symbols. Languages without a grammar fall back to sliding-window text chunking.

## Systems & Low-Level

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| Rust | `.rs` | ✅ |
| C | `.c`, `.h` | ✅ |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` | ✅ |
| Objective-C | `.m`, `.mm` | ✅ |
| Zig | `.zig` | ✅ |
| D | `.d`, `.di` | ✅ |
| Fortran | `.f`, `.f90`, `.f95` | ✅ |

## JVM & .NET

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| Java | `.java` | ✅ |
| Kotlin | `.kt`, `.kts` | ✅ |
| Scala | `.scala`, `.sc` | ✅ |
| Groovy | `.groovy` | ✅ |
| C# | `.cs` | ✅ |
| F# | `.fs`, `.fsi`, `.fsx` | ✅ |
| Clojure | `.clj`, `.cljs`, `.cljc` | ✅ |

## Web & Frontend

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| TypeScript | `.ts`, `.tsx` | ✅ |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` | ✅ |
| HTML | `.html`, `.htm` | ✅ |
| CSS | `.css` | ✅ |
| SCSS | `.scss` | ✅ |
| Vue | `.vue` | ✅ |
| Svelte | `.svelte` | ✅ |
| Astro | `.astro` | ✅ |
| Elm | `.elm` | ✅ |
| GraphQL | `.graphql`, `.gql` | ✅ |

## Scripting

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| Python | `.py`, `.pyi` | ✅ |
| Ruby | `.rb` | ✅ |
| PHP | `.php` | ✅ |
| Lua | `.lua` | ✅ |
| Luau | `.luau` | ✅ |
| Perl | `.pl`, `.pm` | ✅ |
| R | `.r` | ✅ |
| Julia | `.jl` | ✅ |
| Matlab | `.mlx` | ✅ |
| Dart | `.dart` | ✅ |

## Functional

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| Go | `.go` | ✅ |
| Swift | `.swift` | ✅ |
| Haskell | `.hs` | Tier 0 |
| Elixir | `.ex`, `.exs` | ✅ |
| Erlang | `.erl`, `.hrl` | ✅ |
| OCaml | `.ml`, `.mli` | ✅ |
| Common Lisp | `.lisp`, `.cl`, `.lsp` | ✅ |
| Scheme | `.scm`, `.ss` | ✅ |
| Racket | `.rkt` | ✅ |
| Nix | `.nix` | ✅ |

## Shell

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| Bash | `.sh`, `.bash` | ✅ |
| Zsh | `.zsh` | ✅ |
| Fish | `.fish` | ✅ |
| PowerShell | `.ps1`, `.psm1` | ✅ |

## GPU Shaders

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| GLSL | `.glsl`, `.vert`, `.frag`, `.geom`, `.comp`, `.tesc`, `.tese` | ✅ |
| HLSL | `.hlsl`, `.hlsli`, `.fx` | ✅ |

## Infrastructure & Config

| Language | Extensions | Symbol extraction |
|----------|-----------|:-:|
| SQL | `.sql` | ✅ |
| HCL / Terraform | `.tf`, `.hcl` | ✅ |
| Protobuf | `.proto` | ✅ |
| Dockerfile | `Dockerfile` | ✅ |
| CMake | `.cmake`, `CMakeLists.txt` | ✅ |
| Makefile | `Makefile`, `GNUmakefile` | ✅ |
| Nginx | `.nginx`, `nginx.conf` | ✅ |
| Prisma | `.prisma` | ✅ |
| XML | `.xml`, `.xsl`, `.xsd`, `.svg` | ✅ |
| INI | `.ini`, `.cfg`, `.conf` | ✅ |

## Data & Markup

| Format | Extensions | Symbol extraction |
|--------|-----------|:-:|
| TOML | `.toml` | - |
| YAML | `.yaml`, `.yml` | - |
| JSON | `.json` | - |
| Markdown | `.md`, `.markdown` | - |
| reStructuredText | `.rst` | Section-level (tree-sitter) |

Files with unrecognized extensions are indexed using text chunking.
