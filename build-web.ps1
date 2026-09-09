# Build the WebAssembly engine and drop it next to the web front end.
#
#   pwsh -File build-web.ps1
#
# The web build needs the wasm32-unknown-unknown standard library. On a normal
# Rust install that is one command:
#
#   rustup target add wasm32-unknown-unknown
#
# If RUSTUP_HOME is set (for example to a toolchain installed inside this
# checkout because the user profile is not writable), this script uses it.

[CmdletBinding()]
param(
    [switch]$Dev
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$profile_ = if ($Dev) { "dev" } else { "wasm-release" }

Push-Location $root
try {
    $target = "wasm32-unknown-unknown"
    $installed = (rustup target list --installed) -split "`n" | ForEach-Object { $_.Trim() }
    if ($installed -notcontains $target) {
        Write-Host "adding the $target standard library..." -ForegroundColor Cyan
        rustup target add $target
    }

    Write-Host "building tapa-wasm ($profile_)..." -ForegroundColor Cyan
    cargo build -p tapa-wasm --profile $profile_ --target $target --offline
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $built = Join-Path $root "target\$target\$profile_\tapa_wasm.wasm"
    if (-not (Test-Path $built)) { throw "expected $built" }
    Copy-Item $built (Join-Path $root "web\tapa.wasm") -Force

    $size = [math]::Round((Get-Item (Join-Path $root "web\tapa.wasm")).Length / 1KB)
    Write-Host "web/tapa.wasm  ($size KiB)" -ForegroundColor Green

    Write-Host "checking the engine without a browser..." -ForegroundColor Cyan
    node (Join-Path $root "tools\wasm-smoke.cjs")

    Write-Host ""
    Write-Host "Serve the page over HTTP (WebAssembly cannot be fetched from file://):" -ForegroundColor Cyan
    Write-Host "  node tools/serve.cjs" -ForegroundColor White
    Write-Host "  then open http://127.0.0.1:8080/" -ForegroundColor White
}
finally {
    Pop-Location
}
