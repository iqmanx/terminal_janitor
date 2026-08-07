<#
.SYNOPSIS
    terminal_janitor installer for Windows.

.DESCRIPTION
    Three rules this script will not break:

      1. It never asks for administrator rights. Everything lands under the
         invoking user's own profile and only the user's own PATH is touched.
      2. It never enables scheduling. Automatic operation is opt-in and the
         user turns it on themselves with `terminal_janitor enable`.
      3. It never touches configuration or state, so upgrading over an
         existing install preserves thresholds, approved roots, and
         protection.

.EXAMPLE
    .\install.ps1
    .\install.ps1 -Version v0.1.0
    .\install.ps1 -From .\target\release\terminal_janitor.exe
#>
[CmdletBinding()]
param(
    [string] $Version = 'latest',
    [string] $From = '',
    [string] $Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\terminal_janitor')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = 'iqmanx/terminal_janitor'
$name = 'terminal_janitor.exe'

# Only x86_64 Windows is a supported target; anything else is an error rather
# than a guess at a binary that would not run.
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw "unsupported architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("terminal_janitor-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
    $staged = Join-Path $tmp $name

    if ($From) {
        if (-not (Test-Path -LiteralPath $From)) { throw "no such file: $From" }
        Copy-Item -LiteralPath $From -Destination $staged
    }
    else {
        $base = if ($Version -eq 'latest') {
            "https://github.com/$repo/releases/latest/download"
        } else {
            "https://github.com/$repo/releases/download/$Version"
        }
        $archive = 'terminal_janitor-x86_64-pc-windows-msvc.zip'
        $archivePath = Join-Path $tmp $archive
        $sumsPath = Join-Path $tmp 'SHA256SUMS'

        Write-Host "install: downloading $archive"
        Invoke-WebRequest -Uri "$base/$archive" -OutFile $archivePath -UseBasicParsing
        Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $sumsPath -UseBasicParsing

        # A release artefact is verified before it is ever unpacked.
        $expected = $null
        foreach ($line in Get-Content -LiteralPath $sumsPath) {
            $fields = $line -split '\s+', 2
            if ($fields.Count -eq 2 -and $fields[1].TrimStart('*') -eq $archive) {
                $expected = $fields[0]
            }
        }
        if (-not $expected) { throw "$archive is not listed in SHA256SUMS" }
        $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
        if ($expected -ne $actual) { throw "checksum mismatch for $archive" }
        Write-Host 'install: checksum verified'

        Expand-Archive -LiteralPath $archivePath -DestinationPath $tmp -Force
        if (-not (Test-Path -LiteralPath $staged)) { throw "$archive did not contain $name" }
    }

    New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
    $target = Join-Path $Prefix $name
    # Replace atomically so a running or scheduled binary is never half-written.
    $incoming = Join-Path $Prefix ($name + '.new')
    Copy-Item -LiteralPath $staged -Destination $incoming -Force
    Move-Item -LiteralPath $incoming -Destination $target -Force

    $installed = & $target --version
    if (-not $installed) { throw 'the installed binary did not run' }
    Write-Host ""
    Write-Host "install: $installed -> $target"

    # User-scope PATH only. This needs no administrator rights and cannot
    # affect any other account on the machine.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }
    $entries = $userPath -split ';' | Where-Object { $_ -ne '' }
    if ($entries -notcontains $Prefix) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $Prefix) -join ';'), 'User')
        Write-Host "install: added $Prefix to your user PATH; open a new terminal to use it by name"
    }

    Write-Host ""
    Write-Host "Scheduling is NOT enabled. Nothing runs automatically until you ask:"
    Write-Host ""
    Write-Host "    terminal_janitor init --root C:\path\to\your\projects"
    Write-Host "    terminal_janitor enable      # hourly, per-user, no administrator rights"
    Write-Host "    terminal_janitor disable     # stops it again"
    Write-Host ""
    Write-Host "Your configuration and protection state were not touched by this install."
}
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
