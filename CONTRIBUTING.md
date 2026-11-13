# Contributing to Silt

Thanks for helping. Silt deletes and compresses files on people's drives, so changes are held to
one rule above the others: nothing the person would miss is ever lost. Shrink is reversible, Large
files goes to the Recycle Bin, and Clean only touches what its rules name.

## Build

Silt is Windows only (10 and 11, x64). The toolchain is pinned in `rust-toolchain.toml`, so
`rustup` picks the right one on the first build.

```
cargo build --release            # target\release\silt.exe and silt-cli.exe
cargo test
```

| Folder | What it holds |
|---|---|
| `crates/silt-core` | Scanning (MFT and folder walk), categories, rules, Shrink, Clean, the journal |
| `crates/silt-cli` | `silt-cli report`, `largest`, `estimate`: the core without a window |
| `crates/silt-app` | The window, built with Rune |
| `rules/` | Cleanup rules, compiled into the binary |
| `docs/brand` | Mark, wordmark, colours and the bar's spec |

## Before you open a pull request

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Keep a pull request to one change. Say what it does and how you checked it; for anything that
touches files on disk, say on which drive and how much it moved.

## Adding a cleanup rule

Rules live in `rules/<group>.toml`. A rule earns its place only if the app it belongs to rebuilds
the files on its own and nothing personal lives there (sign-ins, history, saves, settings).

```toml
[[rule]]
id = "example-cache"                 # stable, kebab-case; the journal records it
name = "Example app cache"
group = "browsers"                   # system, browsers, gpu or developer
paths = ["%LOCALAPPDATA%\\Example\\Cache"]
safety = "safe"                      # or "caution" when rebuilding costs something
note = "What happens after cleaning, in one sentence."
note-pt-BR = "A mesma frase em português."
```

Use `caution` when rebuilding has a cost the person should know about: a sign-in, a long
download, a slow first launch. Check the rule against a real machine with
`silt-cli report C:` before and after, and include both numbers in the pull request.

## Translating

Every word the window shows is in `crates/silt-app/locales/app/<locale>.json`. Keys are checked at
compile time against `en.json`, so a missing or misspelt key fails the build. Plurals are
`key.one` and `key.other` (plus `few` or `many` where a language needs them), and `{{name}}` is a
value filled in at run time. To add a language, add it to `locales/locales.json` and copy
`en.json`.

Write the way the English does: plain verbs, sentence case, say what happens.

## Changing the window

The app draws every screen and state from a fixed made-up drive, so changes can be checked
without scanning anything:

```
target\debug\silt.exe --list-scenes
target\debug\silt.exe --snap out.png --scene shrink --size 1120x760 --scale 2 --theme dark
```

Attach before and after pictures, light and dark, for any visual change. The bar's rules (cell
pitch, dither thresholds, colours) are in `docs/brand/README.md`; keep to them.

## License

Silt is licensed under either of MIT or Apache 2.0, at your option. Unless you say otherwise, any
contribution you submit is licensed the same way, without extra terms.
