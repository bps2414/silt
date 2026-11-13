<p align="center"><img src="docs/brand/wordmark.svg" height="72" alt="silt"></p>

See what fills your Windows drive, and get the space back without deleting what you use.

> **In development.** The core and `silt-cli` work and are measured below; the app window is
> being built.

Silt reads the NTFS Master File Table to scan a drive in about a second, sorts every byte into
categories, and offers three ways to get space back, from least to most destructive:

| Action | What it does | Undo |
|---|---|---|
| **Shrink** | Compresses folders with Windows' own transparent compression (the one `compact /exe` uses). Files open exactly as before; they just take less space. | Restore |
| **Clean** | Empties caches that rebuild themselves (browsers, GPU shaders, package managers, Windows Update downloads), after showing what and how much. | The cache comes back on its own |
| **Trash** | Sends any file or folder you pick to the Recycle Bin. | The Recycle Bin |

## Measured

**Scan** (MFT, administrator), on the author's machine:

| Drive | Entries | Time | Peak memory | Scanned bytes vs. used space |
|---|---|---|---|---|
| C: (151 GB used) | 790 k | 1.2 s | 151 MB | 99.98 % (34 MB of file system metadata) |
| D: (177 GB used) | 1.17 M | 8.9 s | 208 MB | 99.99 % |

**Shrink** (XPRESS8K, the default), estimate first, then compressed, checked, and restored. Every
file read back byte for byte identical after compressing and after restoring.

| Folder | Files | Size | Estimate | Saved | Estimate error |
|---|---|---|---|---|---|
| `node_modules` of a Vite + React game | 3,231 | 359 MB | 114 to 143 MB | 128 MB (36 %) | 0 % |
| Rust 1.99.1 toolchain | 132 | 627 MB | 329 to 398 MB | 352 MB (56 %) | 3 % |
| An Xbox PC game | 324 | 5.1 GB | 2.3 to 3.0 GB | 2.9 GB (56 %) | 8 % |
| Git for Windows | 3,343 | 396 MB | 177 to 207 MB | 196 MB (50 %) | 2 % |
| Screen recordings (MP4) | 0 | 0 B | nothing to gain | 0 B | |

Video, images, audio and archives are already compressed, so Shrink leaves them alone.

## Try the CLI

```
cargo build --release -p silt-cli
target\release\silt-cli report C:          # categories and what can be recovered
target\release\silt-cli largest C: --min-size 1GB
target\release\silt-cli estimate "C:\Program Files\Git"
```

Commands that change files (`shrink`, `restore`, `clean`, `trash`, `empty-bin`) only show what
they would do until you add `--yes`. Run as administrator for the fast MFT scan; without it Silt
lists folders instead (about 10x slower).

## License

MIT or Apache-2.0, at your option.
