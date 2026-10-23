# 生成本地假识别资源包（占位签名），供 Pack 安装流程验收。
param(
    [string]$OutDir = "src-tauri/vision-runtime/fixtures/fake-pack/staging",
    [string]$Version = "0.0.1-dev",
    [string]$Triple = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not (Test-Path (Join-Path $root "src-tauri"))) {
    $root = (Get-Location).Path
}
$staging = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $root $OutDir }
New-Item -ItemType Directory -Force -Path (Join-Path $staging "templates") | Out-Null
$rules = Join-Path $staging "templates/page_rules.json"
Set-Content -Path $rules -Value '{"ok":true,"pages":["select_slot","confirm_order"]}' -Encoding utf8NoBOM
$health = Join-Path $staging "health.ok"
Set-Content -Path $health -Value "ok" -Encoding ascii -NoNewline

function Sha256Hex([string]$Path) {
    $hash = Get-FileHash -Algorithm SHA256 -Path $Path
    return $hash.Hash.ToLowerInvariant()
}

$files = @(
    @{ path = "templates/page_rules.json"; size = (Get-Item $rules).Length; sha256 = (Sha256Hex $rules) },
    @{ path = "health.ok"; size = (Get-Item $health).Length; sha256 = (Sha256Hex $health) }
)

# Must match vision-pack::compute_placeholder_signature
$sha = [System.Security.Cryptography.SHA256]::Create()
$enc = [System.Text.Encoding]::UTF8
$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $ms
# Rust hasher.update concatenates bytes; replicate field order
$bytes = New-Object System.Collections.Generic.List[byte]
foreach ($b in $enc.GetBytes($Version)) { [void]$bytes.Add($b) }
foreach ($b in $enc.GetBytes($Triple)) { [void]$bytes.Add($b) }
foreach ($b in $enc.GetBytes("dev-test-key")) { [void]$bytes.Add($b) }
foreach ($f in $files) {
    foreach ($b in $enc.GetBytes($f.path)) { [void]$bytes.Add($b) }
    foreach ($b in $enc.GetBytes($f.sha256)) { [void]$bytes.Add($b) }
}
$sig = ($sha.ComputeHash($bytes.ToArray()) | ForEach-Object { $_.ToString("x2") }) -join ""

$manifest = @{
    pack_version = $Version
    protocol_version = 1
    minimum_app_version = "0.1.0"
    target_triple = $Triple
    ocr_backend = "download_onnx_baseline"
    files = $files
    model_versions = @{}
    required_capabilities = @("ocr", "captcha")
    signature = $sig
    key_id = "dev-test-key"
} | ConvertTo-Json -Depth 6

Set-Content -Path (Join-Path $staging "manifest.json") -Value $manifest -Encoding utf8NoBOM
Write-Host "Wrote fake pack staging: $staging"
Write-Host "signature=$sig"
