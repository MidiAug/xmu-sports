#Requires -Version 5.1
<#
.SYNOPSIS
  Ensure src-tauri/resources/ocr has RapidOCR + captcha models for the installer.

.DESCRIPTION
  Prefer copying from %LOCALAPPDATA%\xmu-sports\; otherwise download with SHA256 check.
  Large .onnx files are gitignored — run this before `npm run tauri:build`.
#>
$ErrorActionPreference = "Stop"
if (-not (Get-Command Get-FileHash -ErrorAction SilentlyContinue)) {
  Import-Module (Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Utility') -ErrorAction Stop
}
$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root "src-tauri\resources\ocr"
$Local = Join-Path $env:LOCALAPPDATA "xmu-sports"

$Assets = @(
  @{
    Rel = "rapidocr\ch_PP-OCRv5_det_mobile.onnx"
    LocalRel = "rapidocr\ch_PP-OCRv5_det_mobile.onnx"
    Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx"
    Sha256 = "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae"
    Bytes = 4819576
  },
  @{
    Rel = "rapidocr\ch_PP-OCRv5_rec_mobile.onnx"
    LocalRel = "rapidocr\ch_PP-OCRv5_rec_mobile.onnx"
    Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx"
    Sha256 = "5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5"
    Bytes = 16631306
  },
  @{
    Rel = "rapidocr\ppocrv5_dict.txt"
    LocalRel = "rapidocr\ppocrv5_dict.txt"
    Url = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/paddle/PP-OCRv5/rec/ch_PP-OCRv5_rec_server/ppocrv5_dict.txt"
    Sha256 = "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b"
    Bytes = 74012
  },
  @{
    Rel = "captcha\common.onnx"
    LocalRel = "captcha\common.onnx"
    Url = "https://raw.githubusercontent.com/sml2h3/ddddocr/master/ddddocr/common.onnx"
    Sha256 = "33b5cd351ee94e73a6bf8fa18c415ed8b819b3ffd342e267c30d8ad8334e34e8"
    Bytes = 54088400
  }
)

function Test-Asset([string]$Path, [string]$Sha256, [long]$Bytes) {
  if (-not (Test-Path -LiteralPath $Path)) { return $false }
  $item = Get-Item -LiteralPath $Path
  if ($item.Length -ne $Bytes) { return $false }
  $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  return ($hash -eq $Sha256.ToLowerInvariant())
}

function Ensure-Asset($Asset) {
  $out = Join-Path $Dest $Asset.Rel
  $outDir = Split-Path -Parent $out
  if (-not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
  }
  if (Test-Asset $out $Asset.Sha256 $Asset.Bytes) {
    Write-Host "[ok] $($Asset.Rel)"
    return
  }
  $fromLocal = Join-Path $Local $Asset.LocalRel
  if (Test-Asset $fromLocal $Asset.Sha256 $Asset.Bytes) {
    Copy-Item -LiteralPath $fromLocal -Destination $out -Force
    Write-Host "[copy] $($Asset.Rel) <- LOCALAPPDATA"
    return
  }
  Write-Host "[download] $($Asset.Rel)"
  $tmp = "$out.download"
  try {
    Invoke-WebRequest -Uri $Asset.Url -OutFile $tmp -UseBasicParsing
    if (-not (Test-Asset $tmp $Asset.Sha256 $Asset.Bytes)) {
      throw "checksum/size mismatch for $($Asset.Rel)"
    }
    Move-Item -LiteralPath $tmp -Destination $out -Force
  }
  finally {
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
  }
  Write-Host "[ok] $($Asset.Rel) (downloaded)"
}

New-Item -ItemType Directory -Path $Dest -Force | Out-Null
foreach ($a in $Assets) { Ensure-Asset $a }
Write-Host "OCR bundle ready: $Dest"
