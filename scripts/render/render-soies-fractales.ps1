param(
    [ValidateRange(64,16384)][int]$Width = 2330,
    [ValidateRange(64,16384)][int]$Height = 1440,
    [ValidateRange(1,4096)][int]$Samples = 16,
    [string]$Output = 'renders/studies/soies-fractales-20261013'
)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Push-Location $taskRoot
try {
    python scripts/compose/compose-soies-fractales.py
    if ($LASTEXITCODE -ne 0) { throw 'Scene composition failed.' }
    & target/laptop/laptop/examples/complex_lab.exe scenes/studies/soies-fractales-20261013 $Output $Width $Height $Samples
    if ($LASTEXITCODE -ne 0) { throw 'Native rendering failed.' }
} finally {
    Pop-Location
}
