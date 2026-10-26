# Code-signing hook. Tauri runs this for every file it bundles (tauri.conf.json >
# bundle.windows.signCommand). Which signer runs depends on SHIFTDROP_SIGN:
#
#   (unset)    Don't sign. Prints a warning; the build still succeeds.
#   cert       A certificate in the Windows certificate store (OV cert on a USB token
#              or cloud HSM). Needs SHIFTDROP_CERT_THUMBPRINT. Uses signtool.exe.
#   azure      Azure Artifact Signing (UK organisations, US/CA individuals). Needs the
#              Azure dlib + metadata.json described in RELEASING.md. Uses signtool.exe.
#
# SignPath (free for open-source projects) signs in their CI, not here; see RELEASING.md.

param([Parameter(Mandatory = $true)][string]$File)
$ErrorActionPreference = "Stop"

$mode = $env:SHIFTDROP_SIGN
if (-not $mode) {
    Write-Host "sign.ps1: NOT signing $([IO.Path]::GetFileName($File)) (SHIFTDROP_SIGN not set)"
    exit 0
}

function Find-SignTool {
    if ($env:SIGNTOOL -and (Test-Path $env:SIGNTOOL)) { return $env:SIGNTOOL }
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    $found = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\x64\\" } | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $found) { throw "signtool.exe not found: install the Windows SDK or set SIGNTOOL" }
    return $found.FullName
}

$timestamp = if ($env:SHIFTDROP_TIMESTAMP_URL) { $env:SHIFTDROP_TIMESTAMP_URL } else { "http://timestamp.digicert.com" }

switch ($mode) {
    "cert" {
        if (-not $env:SHIFTDROP_CERT_THUMBPRINT) { throw "set SHIFTDROP_CERT_THUMBPRINT" }
        & (Find-SignTool) sign /sha1 $env:SHIFTDROP_CERT_THUMBPRINT /fd sha256 /tr $timestamp /td sha256 $File
    }
    "azure" {
        if (-not $env:SHIFTDROP_AZURE_DLIB -or -not $env:SHIFTDROP_AZURE_METADATA) {
            throw "set SHIFTDROP_AZURE_DLIB and SHIFTDROP_AZURE_METADATA (see RELEASING.md)"
        }
        & (Find-SignTool) sign /v /fd SHA256 /tr "http://timestamp.acs.microsoft.com" /td SHA256 `
            /dlib $env:SHIFTDROP_AZURE_DLIB /dmdf $env:SHIFTDROP_AZURE_METADATA $File
    }
    default { throw "unknown SHIFTDROP_SIGN '$mode' (use cert or azure)" }
}
if ($LASTEXITCODE -ne 0) { throw "signing failed for $File" }
Write-Host "sign.ps1: signed $([IO.Path]::GetFileName($File)) ($mode)"
