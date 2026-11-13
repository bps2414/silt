# r/webdev (draft, not posted; Showoff Saturday thread if the sub requires it)

**Title:** node_modules, pnpm store, Playwright browsers, npm cache... I made a Windows tool that shows how much they take and gets it back

**Body:**

On my dev machine the biggest "categories" on C: were all tooling: package caches, a few
`node_modules`, Playwright's browsers, Hugging Face models, VS Code caches.

Silt (https://github.com/bps2414/silt, free, MIT/Apache) scans the drive in a couple of seconds and
has a **Developer** category for exactly that. Two ways to get space back:

- **Clean** empties caches that come back on their own: npm, Yarn, Bun, pnpm store, uv, pip,
  Cargo, Gradle, NuGet, Go, Electron/node-gyp, VS Code. It finds them through the env vars too
  (`npm_config_cache`, `PNPM_STORE_DIR`, `UV_CACHE_DIR`, ...), so relocated caches count.
  The ones that cost a re-download (pnpm store, Playwright, HF models) are unticked by default.
- **Shrink** compresses folders you keep with Windows' transparent compression. A Vite + React
  project's `node_modules`: 359 MB → saved 128 MB, nothing deleted, `npm run dev` doesn't notice.
  Restore undoes it.

Every rule is a few lines of TOML in `rules/`, so if your stack's cache is missing, a PR is about
five lines. The README has an "Is it safe?" section listing exactly what it can delete.

There's also `silt-cli` with JSON output if you'd rather script it.
