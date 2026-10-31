param(
    [ValidateRange(1, 86400)][int]$PhaseSeconds = 300,
    [string]$BaseUrl = 'http://127.0.0.1:1437/',
    [ValidateSet('foreground', 'background', 'minimized', 'hidden', 'idle')][string[]]$Phases = @('foreground', 'background', 'minimized', 'hidden')
)
$ErrorActionPreference = 'Stop'
if ($Phases -contains 'idle' -and $Phases.Count -ne 1) { throw 'Run idle separately from pressure phases' }
$repo = Split-Path $PSScriptRoot -Parent
$run = Join-Path $repo ('artifacts\window-soak-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run | Out-Null
$build = & cargo test --manifest-path (Join-Path $repo 'src-tauri\Cargo.toml') -p xmu-sports --lib --features window-soak --no-run --message-format=json
if ($LASTEXITCODE -ne 0) { throw 'Native soak compilation failed' }
$exe = $build | ForEach-Object { try { $_ | ConvertFrom-Json } catch {} } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.executable } |
    Select-Object -Last 1 -ExpandProperty executable
if (!$exe) { throw 'Test executable was not reported by Cargo' }
# Rust's test harness lacks the application's Common Controls v6 manifest.
# Embed it in an isolated copy so normal cargo test artifacts stay untouched.
$target = Join-Path $run 'window-soak.exe'
Copy-Item -LiteralPath $exe -Destination $target
$manifest = Join-Path $run 'window-soak.manifest'
@'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" /></dependentAssembly></dependency>
<application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings><dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware></windowsSettings></application>
</assembly>
'@ | Set-Content -LiteralPath $manifest -Encoding utf8
$sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$mt = Get-ChildItem -LiteralPath $sdk -Directory | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\mt.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (!$mt) { throw 'Windows SDK mt.exe is required' }
& $mt -nologo "-manifest" $manifest "-outputresource:$target;#1"
if ($LASTEXITCODE -ne 0) { throw 'Embedding the native test manifest failed' }
$env:XMU_SOAK_URL = $BaseUrl
$env:XMU_SPORTS_DATA = Join-Path $run 'data'
$env:XMU_SOAK_PHASE_SECONDS = "$PhaseSeconds"
$env:XMU_SOAK_PHASES = $Phases -join ','
Write-Output "ISOLATED_SOAK_OUTPUT=$run"
$ErrorActionPreference = 'Continue'
& $target window_soak::native_window_soak --ignored --exact --nocapture 2>&1 | Tee-Object -FilePath (Join-Path $run 'console.log')
$ErrorActionPreference = 'Stop'
if ($LASTEXITCODE -ne 0) { throw "Native soak failed; see $run" }
$samples = Get-Content -LiteralPath (Join-Path $env:XMU_SPORTS_DATA 'samples.jsonl') | ForEach-Object { $_ | ConvertFrom-Json }
$finishedPhases = @($samples | Where-Object { $_.complete -eq $true })
if ($finishedPhases.Count -ne $Phases.Count -or @($samples | Where-Object { $_.failure }).Count) { throw 'Soak did not finish all requested phases' }
Write-Output 'Requested native soak phases completed. See samples.jsonl for measured window states and response times.'
