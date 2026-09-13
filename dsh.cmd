@echo off
rem Dev-only shim: launch `dsh web` from a live checkout (args: web --no-open --port <n>).
rem Requires DSH_SRC to point at the checkout root. For normal use the shell
rem provisions its own packaged backend instead (see packaging/).
if "%DSH_SRC%"=="" (
  echo dsh.cmd: DSH_SRC is not set to a dsh checkout root. 1>&2
  exit /b 1
)
cd /d "%DSH_SRC%"
pnpm dsh %*
