[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$AppExe,

    [Parameter(Mandatory)]
    [string]$LicensesZip,

    [string]$Compiler = 'makensis.exe',

    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$inputExe = (Get-Item -LiteralPath $AppExe).FullName
$inputLicenses = (Get-Item -LiteralPath $LicensesZip).FullName
$makensis = (Get-Command $Compiler -CommandType Application -ErrorAction Stop).Source
$manifest = Get-Content -LiteralPath (Join-Path $project 'rust-app/Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"(?<value>\d+\.\d+\.\d+)"').Groups['value'].Value
if (-not $version) { throw 'The Cargo package version must be in major.minor.patch format.' }

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $project "dist/v$version"
}
$destination = (New-Item -Path $OutputDirectory -ItemType Directory -Force).FullName
$setup = Join-Path $destination 'AndroidDeviceDesk-Setup.exe'
& $makensis "/DAppVersion=$version" "/DAppExe=$inputExe" "/DLicensesZip=$inputLicenses" "/DSetupExe=$setup" (Join-Path $PSScriptRoot 'installer.nsi')
if ($LASTEXITCODE -ne 0) { throw "Installer build failed ($LASTEXITCODE)." }

$hash = Get-FileHash -LiteralPath $setup -Algorithm SHA256
"$($hash.Hash.ToLowerInvariant())  AndroidDeviceDesk-Setup.exe" |
    Set-Content -LiteralPath "$setup.sha256" -Encoding ascii
$hash
