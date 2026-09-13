<#
.SYNOPSIS
  Provision the packaged DSH backend: pristine upstream clone + generated
  launcher/manifest/updater, installed and built, preflighted.
.DESCRIPTION
  One-shot installer step (re-run refuses when a manifest already exists;
  updates go through the shell's Settings → Upgrade dsh). Fails loud on
  any step; the shell's own preflight independently refuses to spawn into
  an unbuilt backend.
.PARAMETER Remote
  Backend source. Default: official upstream (clone-only, never edited).
.PARAMETER Branch
  Branch to track. Default: master.
.PARAMETER Dest
  Target dir. Default: %LOCALAPPDATA%\dsh-desktop\dsh-src.
.EXAMPLE
  .\packaging\install-backend.ps1
#>
param(
  [string]$Remote = "https://github.com/deepseek-ai/deepseek-harness.git",
  [string]$Branch = "master",
  [string]$Dest = (Join-Path $env:LOCALAPPDATA "dsh-desktop\dsh-src")
)

$ErrorActionPreference = "Continue"
# Deliberately NOT Stop: git/pnpm narrate on stderr even on success, and
# newer PowerShell hosts turn those lines into terminating errors under
# Stop (measured: the script died on git's own "Cloning into..." line).
# Every native step below checks $LASTEXITCODE explicitly instead.

function Need($name) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
    throw "prerequisite missing on PATH: $name"
  }
}
Need git; Need node; Need pnpm

if ((Test-Path $Dest)) {
  if ((Test-Path (Join-Path $Dest "dsh-manifest.json"))) {
    throw "already provisioned at $Dest (dsh-manifest.json exists); updates go through Settings → Upgrade dsh"
  }
  # Same policy as the shell's provisioner: residue without a manifest is a
  # failed attempt (clone/install/build died midway) — clear it so retry
  # heals instead of refusing.
  if (((Get-ChildItem $Dest -Force | Measure-Object).Count) -gt 0) {
    Write-Host "clearing incomplete previous attempt at $Dest"
    Remove-Item -Recurse -Force $Dest
  }
}

git clone --branch $Branch --single-branch $Remote $Dest
if ($LASTEXITCODE -ne 0) { throw "git clone failed (rc=$LASTEXITCODE)" }
$sha = (git -C $Dest rev-parse HEAD).Trim()

$manifest = @{ remote = $Remote; branch = $Branch; commit = $sha } | ConvertTo-Json
[System.IO.File]::WriteAllText((Join-Path $Dest "dsh-manifest.json"), $manifest)
$launcher = "@echo off`r`nrem Generated at install: launch the packaged dsh backend (no checkout needed).`r`ncd /d %~dp0`r`nnode --import tsx/esm apps/cli/src/bin.ts %*`r`n"
[System.IO.File]::WriteAllText((Join-Path $Dest "dsh-packaged.cmd"), $launcher)
Copy-Item (Join-Path $PSScriptRoot "updater.cmd.template") (Join-Path $Dest "updater.cmd")
Add-Content (Join-Path $Dest ".git\info\exclude") "`r`ndsh-manifest.json`r`ndsh-packaged.cmd`r`nupdater.cmd"

Push-Location $Dest
try {
  pnpm install
  if ($LASTEXITCODE -ne 0) { throw "pnpm install failed (rc=$LASTEXITCODE)" }
  pnpm run build
  if ($LASTEXITCODE -ne 0) { throw "pnpm run build failed (rc=$LASTEXITCODE)" }
} finally {
  Pop-Location
}

if (-not (Test-Path (Join-Path $Dest "node_modules") -PathType Container)) { throw "preflight: node_modules missing" }
if (-not (Test-Path (Join-Path $Dest "apps\cli\lib\bin.js") -PathType Leaf)) { throw "preflight: apps/cli/lib/bin.js missing (build incomplete)" }
Write-Host "backend ready at $Dest ($sha)"
