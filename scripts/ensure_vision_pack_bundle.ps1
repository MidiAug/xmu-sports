#Requires -Version 5.1
<#
.SYNOPSIS
  Ensure src-tauri/resources/vision-pack is a valid installable staging pack.

.DESCRIPTION
  Writes page_rules + health.ok + placeholder-signed manifest for the NSIS bundle.
  Tiny files are committed; re-run when version or rules change.
#>
$ErrorActionPreference = "Stop"
if (-not (Get-Command Get-FileHash -ErrorAction SilentlyContinue)) {
  Import-Module (Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Utility') -ErrorAction Stop
}
$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root "src-tauri\resources\vision-pack"
$Pkg = Get-Content (Join-Path $Root "package.json") -Raw | ConvertFrom-Json
$Version = [string]$Pkg.version
if ([string]::IsNullOrWhiteSpace($Version)) { $Version = "0.1.0" }
$Triple = "x86_64-pc-windows-msvc"
$KeyId = "bundled-installer-key"

New-Item -ItemType Directory -Force -Path (Join-Path $Dest "templates") | Out-Null

$RulesPath = Join-Path $Dest "templates\page_rules.json"
$RulesJson = @'
{
  "ok": true,
  "pages": ["select_slot", "confirm_order", "home", "venue_sport_list"],
  "shadow": {
    "min_confidence": 0.55,
    "never_guess_buttons": true,
    "ocr_backend_preference": ["pack_onnx_download", "windows_media_system"],
    "note": "OCR models ship in the main installer (resources/ocr); this pack holds page rules only"
  }
}
'@
$Utf8NoBom = New-Object System.Text.UTF8Encoding $false
function Write-Utf8NoBom([string]$Path, [string]$Text) {
  [System.IO.File]::WriteAllText($Path, $Text, $Utf8NoBom)
}

Write-Utf8NoBom $RulesPath ($RulesJson.Trim() + "`n")

$HealthPath = Join-Path $Dest "health.ok"
Write-Utf8NoBom $HealthPath "ok"

function Sha256Hex([string]$Path) {
  return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

$files = @(
  @{ path = "templates/page_rules.json"; size = [int64](Get-Item -LiteralPath $RulesPath).Length; sha256 = (Sha256Hex $RulesPath) },
  @{ path = "health.ok"; size = [int64](Get-Item -LiteralPath $HealthPath).Length; sha256 = (Sha256Hex $HealthPath) }
)

# Must match vision-pack::compute_placeholder_signature
$enc = [System.Text.Encoding]::UTF8
$bytes = New-Object System.Collections.Generic.List[byte]
foreach ($b in $enc.GetBytes($Version)) { [void]$bytes.Add($b) }
foreach ($b in $enc.GetBytes($Triple)) { [void]$bytes.Add($b) }
foreach ($b in $enc.GetBytes($KeyId)) { [void]$bytes.Add($b) }
foreach ($f in $files) {
  foreach ($b in $enc.GetBytes($f.path)) { [void]$bytes.Add($b) }
  foreach ($b in $enc.GetBytes($f.sha256)) { [void]$bytes.Add($b) }
}
$sha = [System.Security.Cryptography.SHA256]::Create()
$sig = ($sha.ComputeHash($bytes.ToArray()) | ForEach-Object { $_.ToString("x2") }) -join ""

$manifestObj = [ordered]@{
  pack_version           = $Version
  protocol_version       = 1
  minimum_app_version    = "0.1.0"
  target_triple          = $Triple
  ocr_backend            = "installer_bundled_onnx"
  files                  = $files
  model_versions         = @{}
  required_capabilities  = @("ocr", "captcha")
  signature              = $sig
  key_id                 = $KeyId
}
$manifestPath = Join-Path $Dest "manifest.json"
$manifestJson = $manifestObj | ConvertTo-Json -Depth 6
Write-Utf8NoBom $manifestPath ($manifestJson + "`n")

Write-Host "Wrote vision-pack staging: $Dest"
Write-Host "pack_version=$Version signature=$sig"
