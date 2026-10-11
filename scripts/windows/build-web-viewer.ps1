[CmdletBinding()]
param(
    # Folder for the static site; must not exist or be empty.
    [Parameter(Mandatory = $true)][string]$OutputDir,
    # Optional .ketchup-view copied next to the Viewer as model.ketchup-view.
    [string]$Model
)

# Builds the web Kečup Viewer as static files: index.html, main.js, the
# WebAssembly module and embed-test.html. Nothing is uploaded or published.
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$bindgenVersion = "0.2.126"

if ((Test-Path $OutputDir) -and (Get-ChildItem $OutputDir -Force | Select-Object -First 1)) {
    throw "Output folder is not empty; refusing to overwrite: $OutputDir"
}
$bindgen = Get-Command wasm-bindgen -ErrorAction SilentlyContinue
if (-not $bindgen -or ((& wasm-bindgen --version) -notmatch [regex]::Escape($bindgenVersion))) {
    throw "wasm-bindgen $bindgenVersion is required: cargo install wasm-bindgen-cli --version =$bindgenVersion --locked"
}

$env:CARGO_TARGET_DIR = Join-Path $root "target\wasm"
& rustup target add wasm32-unknown-unknown | Out-Null
& cargo build -p ketchup-viewer-web --target wasm32-unknown-unknown --release --manifest-path (Join-Path $root "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "WebAssembly build failed" }

[void](New-Item $OutputDir -ItemType Directory -Force)
$wasm = Join-Path $env:CARGO_TARGET_DIR "wasm32-unknown-unknown\release\ketchup_viewer_web.wasm"
& wasm-bindgen --target web --no-typescript --out-dir $OutputDir $wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }
Copy-Item (Join-Path $root "viewer\web\static\*") $OutputDir
if ($Model) {
    Copy-Item $Model (Join-Path $OutputDir "model.ketchup-view")
}
Write-Host "Web Viewer: $OutputDir (serve over HTTPS; see viewer\web\HOSTING.md)"
