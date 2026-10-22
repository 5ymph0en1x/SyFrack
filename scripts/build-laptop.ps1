param(
    [ValidateRange(1,64)][int]$Jobs = 4,
    [switch]$BuildProfilers,
    [switch]$BuildComplexLab,
    [ValidatePattern('^syfrack-studio[a-z0-9-]*$')][string]$StudioName = 'syfrack-studio'
)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskOldFlags = $env:RUSTFLAGS
$taskOldEncoded = $env:CARGO_ENCODED_RUSTFLAGS
$taskOldTarget = $env:CARGO_TARGET_DIR
Push-Location $taskRoot
try {
    # Separate cache and opt-in profile: no change to portable release builds.
    $env:CARGO_ENCODED_RUSTFLAGS = $null
    $env:RUSTFLAGS = '-C target-cpu=native'
    $env:CARGO_TARGET_DIR = Join-Path $taskRoot 'target/laptop'
    # One invocation shares dependency features and prevents rebuilding the engine
    # between CLI and Studio. Optional studies use this same resolved GPU runtime.
    if ($StudioName -eq 'syfrack-studio') {
        $taskBuildArgs = @('build','--offline','--locked','--profile','laptop','--jobs',"$Jobs",'-p','syfrack-cli','-p','syfrack-app','--bin','syfrack','--bin','syfrack-studio')
        if ($BuildComplexLab) { $taskBuildArgs += @('-p','syfrack-render','--example','complex_lab') }
        & cargo @taskBuildArgs
    } else {
        cargo build --offline --locked --profile laptop --jobs $Jobs -p syfrack-cli -p syfrack-app --lib --bin syfrack
        if ($LASTEXITCODE -ne 0) { throw 'Laptop build failed.' }
        # Windows keeps an open executable locked. An alternate output lets
        # the user keep the running Studio and reopen the new build later.
        cargo rustc --offline --locked --profile laptop --jobs $Jobs -p syfrack-app --bin syfrack-studio -- -o (Join-Path $env:CARGO_TARGET_DIR "laptop/$StudioName.exe")
    }
    if ($LASTEXITCODE -ne 0) { throw 'Laptop Studio build failed.' }
    if ($BuildComplexLab -and $StudioName -ne 'syfrack-studio') {
        cargo build --offline --locked --profile laptop --jobs $Jobs -p syfrack-render --example complex_lab
        if ($LASTEXITCODE -ne 0) { throw 'Complex study build failed.' }
    }
    if ($BuildProfilers) {
        cargo build --offline --locked --profile laptop --jobs $Jobs -p syfrack-render --example denoise_profile --example workgroup_profile --example render_profile
        if ($LASTEXITCODE -ne 0) { throw 'Laptop profiler build failed.' }
    }
    & (Join-Path $env:CARGO_TARGET_DIR 'laptop/syfrack.exe') probe
    if ($LASTEXITCODE -ne 0) { throw 'GPU probe failed.' }
    $taskInfo = [ordered]@{
        cpu = (Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0').ProcessorNameString
        logicalProcessors = [int]$env:NUMBER_OF_PROCESSORS
        rustc = (rustc -Vv) -join "`n"
        rustflags = $env:RUSTFLAGS
        profile = 'laptop: release, ThinLTO, codegen-units=1'
        builtAt = (Get-Date).ToString('o')
        binaries = @('syfrack.exe', "$StudioName.exe" | ForEach-Object {
            $taskBinary = Join-Path $env:CARGO_TARGET_DIR "laptop/$_"
            $taskStream = [IO.File]::OpenRead($taskBinary)
            $taskHasher = [Security.Cryptography.SHA256]::Create()
            try {
                [ordered]@{ Path = $taskBinary; Hash = [BitConverter]::ToString($taskHasher.ComputeHash($taskStream)).Replace('-', '') }
            } finally { $taskStream.Dispose(); $taskHasher.Dispose() }
        })
    }
    $taskInfo | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $env:CARGO_TARGET_DIR 'build-info.json') -Encoding UTF8
    Write-Output "Studio: $env:CARGO_TARGET_DIR/laptop/$StudioName.exe"
    Write-Output 'Host-specific build: use the portable build for other computers.'
} finally {
    $env:RUSTFLAGS = $taskOldFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $taskOldEncoded
    $env:CARGO_TARGET_DIR = $taskOldTarget
    Pop-Location
}
