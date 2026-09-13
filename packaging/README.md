# dsh-desktop packaging

First-run backend provisioning. The shell never bundles the DSH backend;
it provisions a pristine upstream clone on first launch (or via this
script) and self-updates it afterwards (Settings → Upgrade dsh).

## Layout produced

`%LOCALAPPDATA%\dsh-desktop\dsh-src\` (default):

- upstream clone (`master`, single branch) — never edited
- `dsh-manifest.json` — `{remote, branch, commit}`
- `dsh-packaged.cmd` — `node --import tsx/esm apps/cli/src/bin.ts` in that dir
- `updater.cmd` — detached updater (copy of `updater.cmd.template`)
- `.git/info/exclude` gains the three generated files so merges ignore them

## Manual provision (until the NSIS installer runs this)

```powershell
.\packaging\install-backend.ps1
```

Prerequisites on PATH: `git`, `node` (>=22), `pnpm`. The script clones,
installs, **builds** (workspace `lib/` output is required at boot — a
fresh clone without it fails with `ERR_MODULE_NOT_FOUND`), then
preflights (`node_modules` + `apps/cli/lib/bin.js`). Any step fails loud;
nothing is left half-provisioned (the shell's own preflight refuses to
spawn into it, with the exact fix).

Parameters: `-Remote`, `-Branch`, `-Dest`. Defaults track upstream
`master` into the standard dir. A re-run against an existing manifest
refuses — updates go through the in-app Upgrade flow, never reinstall.
