[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $root 'vendor'
$engine = Join-Path $vendor 'scrcpy-win64-v5.0.1.zip'
$url = 'https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-win64-v5.0.1.zip'
$sha256 = 'B12A2C4EE8BE317422451FC7DCF8EE20A71B5EA7EF9AD73DDD825A25316227A5'
$manifest = Join-Path $root 'rust-app/Cargo.toml'

New-Item -Type Directory -Force -Path $vendor | Out-Null
if (-not (Test-Path -LiteralPath $engine)) {
    $download = "$engine.download"
    Invoke-WebRequest -Uri $url -OutFile $download
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $sha256) {
        throw 'The downloaded scrcpy archive did not match the published SHA-256.'
    }
    Move-Item -LiteralPath $download -Destination $engine
}
if ((Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash -ne $sha256) {
    throw "The engine archive has an unexpected SHA-256. Replace it with the official archive at $url."
}

Push-Location $root
try {
    & cargo build --locked --release --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed ($LASTEXITCODE)." }
    $metadata = & cargo metadata --no-deps --format-version 1 --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed ($LASTEXITCODE)." }
    $target = ($metadata | ConvertFrom-Json).target_directory
    $dist = Join-Path $root 'dist'
    $program = Join-Path $dist 'AndroidDeviceDesk.exe'
    New-Item -Type Directory -Force -Path $dist | Out-Null
    Copy-Item -LiteralPath (Join-Path $target 'release/device-desk.exe') -Destination $program -Force
    Get-FileHash -LiteralPath $program -Algorithm SHA256
} finally {
    Pop-Location
}
