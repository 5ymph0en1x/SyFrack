param(
    [ValidateSet('release', 'dev')]
    [string]$Profile = 'release',
    [int]$Jobs = 0
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $taskRoot
try {
    $buildArgs = @('build', '--locked', '--workspace', '--bins', '--profile', $Profile)
    if ($Jobs -gt 0) { $buildArgs += @('--jobs', $Jobs) }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    $outputFolder = if ($Profile -eq 'dev') { 'debug' } else { $Profile }
    Write-Host "Built Studio and CLI in target/$outputFolder."
} finally {
    Pop-Location
}
