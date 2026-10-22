#Requires -Version 7.0
<#
.SYNOPSIS
Launch Studio, the CLI or tests with an additional software Vulkan driver.
.DESCRIPTION
Without -Executable, launch the packaged Studio or the newest local Studio build.
Only the child process receives the setting. Windows and the original driver
manifest are unchanged. A CPU ICD must report DeviceType::Cpu to be accepted.
.EXAMPLE
.\with-vulkan-cpu.ps1
.EXAMPLE
.\with-vulkan-cpu.ps1 -Executable ..\target\debug\syfrack.exe -RenderArgs @('probe')
#>
[CmdletBinding()]
param(
    [string] $Executable,
    [string] $Icd = $env:SYFRACK_CPU_ICD,
    [string[]] $RenderArgs = @()
)

$ErrorActionPreference = 'Stop'
# The Windows Vulkan loader intentionally ignores driver environment overrides
# in elevated applications. Fail before launching a GPU-only export by mistake.
$exportIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
try {
    $exportPrincipal = [Security.Principal.WindowsPrincipal]::new($exportIdentity)
    if ($exportPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Le rendu CPU Vulkan est indisponible en mode administrateur : le chargeur ignore VK_ADD_DRIVER_FILES. Ouvrez PowerShell 7 sans « Exécuter en tant qu’administrateur », puis relancez ce script. Aucun programme n’a été lancé."
    }
} finally {
    $exportIdentity.Dispose()
}
$exportRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $exportPackaged = Join-Path $exportRoot 'syfrack-studio.exe'
    if (Test-Path -LiteralPath $exportPackaged -PathType Leaf) {
        $Executable = $exportPackaged
    } else {
        $exportBuilds = foreach ($exportRelative in @(
            'target/release/syfrack-studio.exe',
            'target/debug/syfrack-studio.exe',
            'target/x86_64-pc-windows-msvc/release/syfrack-studio.exe',
            'target/x86_64-pc-windows-msvc/debug/syfrack-studio.exe'
        )) {
            $exportCandidate = Join-Path $exportRoot $exportRelative
            if (Test-Path -LiteralPath $exportCandidate -PathType Leaf) {
                Get-Item -LiteralPath $exportCandidate
            }
        }
        $Executable = $exportBuilds | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $Executable) {
        throw "Studio introuvable. Depuis '$exportRoot', compilez-le avec 'cargo build --locked -p syfrack-app --bin syfrack-studio', puis relancez ce script. Vous pouvez aussi fournir -Executable avec le chemin d'un programme."
    }
}
$exportExecutable = (Resolve-Path -LiteralPath $Executable).ProviderPath
if (-not $Icd) {
    # Optional discovery of an already installed SwiftShader runtime. No
    # browser profile, settings or user data are read.
    $exportProviders = @(
        (Join-Path $env:ProgramFiles 'Google/Chrome/Application'),
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft/Edge/Application')
    )
    $exportCandidates = foreach ($exportProvider in $exportProviders) {
        if (Test-Path -LiteralPath $exportProvider -PathType Container) {
            foreach ($exportVersion in Get-ChildItem -LiteralPath $exportProvider -Directory) {
                $exportCandidate = Join-Path $exportVersion.FullName 'vk_swiftshader_icd.json'
                if (Test-Path -LiteralPath $exportCandidate -PathType Leaf) { $exportCandidate }
            }
        }
    }
    $Icd = $exportCandidates | Sort-Object -Descending | Select-Object -First 1
}
if (-not $Icd) { throw 'Supply -Icd with a Vulkan CPU driver manifest (lavapipe or SwiftShader).' }
$exportSource = (Resolve-Path -LiteralPath $Icd).ProviderPath
$exportManifest = Get-Content -LiteralPath $exportSource -Raw | ConvertFrom-Json
if (-not $exportManifest.ICD.library_path) { throw 'The manifest has no ICD.library_path.' }
$exportLibrary = [string] $exportManifest.ICD.library_path
if (-not [System.IO.Path]::IsPathRooted($exportLibrary)) {
    $exportLibrary = Join-Path (Split-Path -Parent $exportSource) $exportLibrary
}
$exportManifest.ICD.library_path = (Resolve-Path -LiteralPath $exportLibrary).ProviderPath
# Some Windows loaders reject a relative DLL path with error 87. Resolve it in
# a private manifest instead of changing the vendor's installed JSON.
$exportTemporary = Join-Path ([System.IO.Path]::GetTempPath()) ('syfrack-cpu-' + [guid]::NewGuid().ToString('N') + '.json')
$exportJson = $exportManifest | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($exportTemporary, $exportJson, [System.Text.UTF8Encoding]::new($false))

$exportPrevious = $env:VK_ADD_DRIVER_FILES
$exportExitCode = 1
try {
    $exportDriverList = @($exportPrevious, $exportTemporary) | Where-Object { $_ }
    Write-Host ('Executable: ' + $exportExecutable)
    Write-Host ('Vulkan CPU driver: ' + $exportManifest.ICD.library_path)
    $exportStart = [System.Diagnostics.ProcessStartInfo]::new()
    $exportStart.FileName = $exportExecutable
    $exportStart.WorkingDirectory = $exportRoot
    $exportStart.UseShellExecute = $false
    $exportStart.CreateNoWindow = $true
    $exportStart.Environment['VK_ADD_DRIVER_FILES'] = $exportDriverList -join [System.IO.Path]::PathSeparator
    # Explicit driver lists take precedence over VK_ADD_DRIVER_FILES. Preserve
    # the caller's selected drivers and add the CPU driver to that effective list.
    foreach ($exportOverride in @('VK_DRIVER_FILES', 'VK_ICD_FILENAMES')) {
        $exportExisting = [Environment]::GetEnvironmentVariable($exportOverride)
        if (-not [string]::IsNullOrWhiteSpace($exportExisting)) {
            $exportStart.Environment[$exportOverride] = @($exportExisting, $exportTemporary) -join [System.IO.Path]::PathSeparator
        } else {
            [void] $exportStart.Environment.Remove($exportOverride)
        }
    }
    $exportStart.RedirectStandardOutput = $true
    $exportStart.RedirectStandardError = $true
    foreach ($exportArgument in $RenderArgs) { $exportStart.ArgumentList.Add($exportArgument) }
    $exportProcess = [System.Diagnostics.Process]::Start($exportStart)
    try {
        $exportStdout = $exportProcess.StandardOutput.BaseStream.CopyToAsync([Console]::OpenStandardOutput())
        $exportStderr = $exportProcess.StandardError.BaseStream.CopyToAsync([Console]::OpenStandardError())
        $exportProcess.WaitForExit()
        [void] $exportStdout.GetAwaiter().GetResult()
        [void] $exportStderr.GetAwaiter().GetResult()
        $exportExitCode = $exportProcess.ExitCode
    } finally { $exportProcess.Dispose() }
} finally {
    Remove-Item -LiteralPath $exportTemporary -ErrorAction SilentlyContinue
}
exit $exportExitCode
