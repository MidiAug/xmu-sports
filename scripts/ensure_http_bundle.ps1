#Requires -Version 5.1
param([switch]$UseInstalledDependencies)
$ErrorActionPreference = 'Stop'
if (-not (Get-Command Get-FileHash -ErrorAction SilentlyContinue)) {
    Import-Module (Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Utility') -ErrorAction Stop
}
$TaskRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$SourceRoot = Join-Path $TaskRoot 'src-tauri\sidecars\http-booking'
$Destination = Join-Path $TaskRoot 'src-tauri\resources\http-booking'
$Interpreter = Join-Path $SourceRoot '.venv\Scripts\python.exe'
$BuildRoot = Join-Path $TaskRoot 'artifacts\http-desktop-build'
$Stamp = Join-Path $Destination '.source.sha256'
$SourceFiles = @($PSCommandPath, (Join-Path $SourceRoot 'desktop.py'), (Join-Path $SourceRoot 'requirements.txt'))
$SourceFiles += @(Get-ChildItem -LiteralPath (Join-Path $SourceRoot 'xmu_http') -Filter '*.py' | Sort-Object Name | ForEach-Object { $_.FullName })
$SourceFiles += @(Get-ChildItem -LiteralPath (Join-Path $SourceRoot 'xmu_http\captcha_assets') -File | Sort-Object Name | ForEach-Object { $_.FullName })
$DigestInput = ($SourceFiles | ForEach-Object { (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash }) -join ''
$Digest = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($DigestInput))).Replace('-', '')
if ((Test-Path -LiteralPath (Join-Path $Destination 'xmu-http-booking.exe')) -and
    (Test-Path -LiteralPath $Stamp) -and ((Get-Content -LiteralPath $Stamp -Raw).Trim() -eq $Digest)) {
    Write-Host '[ok] HTTP worker is current'
    exit 0
}
if (-not (Test-Path -LiteralPath $Interpreter)) {
    & python -m venv (Join-Path $SourceRoot '.venv')
    if ($LASTEXITCODE -ne 0) { throw 'HTTP build environment creation failed' }
}
if ($UseInstalledDependencies) {
    & $Interpreter -c "import requests, ccl_chromium_reader, onnxruntime, numpy, PIL, PyInstaller"
    if ($LASTEXITCODE -ne 0) { throw 'Installed HTTP build dependencies are incomplete' }
} else {
    & $Interpreter -m pip install -r (Join-Path $SourceRoot 'requirements.txt') 'pyinstaller==6.22.3'
    if ($LASTEXITCODE -ne 0) { throw 'HTTP build dependencies failed' }
}
$OrtRoot = (& $Interpreter -c "import onnxruntime; from pathlib import Path; print(Path(onnxruntime.__file__).parent)").Trim()
if ($LASTEXITCODE -ne 0) { throw 'ONNX Runtime location unavailable' }
# Include licenses, but not ORT's sample models, mobile tool data or benchmarks.
& $Interpreter -m PyInstaller --noconfirm --clean --onedir --name xmu-http-booking --distpath (Join-Path $BuildRoot 'dist') --workpath (Join-Path $BuildRoot 'work') --specpath $BuildRoot --collect-all ccl_chromium_reader --collect-binaries onnxruntime --hidden-import onnxruntime.capi.onnxruntime_pybind11_state --exclude-module frida --exclude-module websockets --exclude-module cv2 --exclude-module ddddocr --exclude-module xmu_http.runtime_entry --exclude-module xmu_http.runtime_probe --exclude-module xmu_http.runtime_wire --exclude-module tkinter --exclude-module matplotlib --add-data "$(Join-Path $OrtRoot 'LICENSE');onnxruntime" --add-data "$(Join-Path $OrtRoot 'Privacy.md');onnxruntime" --add-data "$(Join-Path $OrtRoot 'ThirdPartyNotices.txt');onnxruntime" --add-data "$(Join-Path $SourceRoot 'xmu_http\captcha_assets');xmu_http/captcha_assets" (Join-Path $SourceRoot 'desktop.py')
if ($LASTEXITCODE -ne 0) { throw 'HTTP worker build failed' }
# Replace generated resources so legacy DLLs cannot survive an overlay copy.
$ExpectedDestination = [IO.Path]::GetFullPath((Join-Path $TaskRoot 'src-tauri\resources\http-booking'))
if ([IO.Path]::GetFullPath($Destination) -ne $ExpectedDestination) { throw 'Unsafe HTTP resource destination' }
if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Recurse -Force }
New-Item -ItemType Directory -Path $Destination -Force | Out-Null
Copy-Item -Path (Join-Path $BuildRoot 'dist\xmu-http-booking\*') -Destination $Destination -Recurse -Force
[IO.File]::WriteAllText($Stamp, $Digest, [Text.Encoding]::UTF8)
Write-Host '[ok] HTTP worker bundled; end users do not need Python'
