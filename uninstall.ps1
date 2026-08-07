<#
.SYNOPSIS
    terminal_janitor uninstaller for Windows.

.DESCRIPTION
    `PLANS.md` lists install.sh, install.ps1, and uninstall.sh. This fourth
    script exists because `ACCEPTANCE.md` section A requires uninstall to work
    from a clean user account on every supported platform, and Windows cannot
    run uninstall.sh.

    Order matters. The scheduled task is removed *before* the binary, because a
    task left pointing at a deleted binary fails every hour forever.

    Configuration and protection state are kept by default: they record which
    roots a user approved and which workspaces they protected, and deleting
    that silently would be the one irreversible thing an uninstaller could do.
    Pass -Purge to remove them deliberately.

.EXAMPLE
    .\uninstall.ps1
    .\uninstall.ps1 -Purge
#>
[CmdletBinding()]
param(
    [switch] $Purge,
    [string] $Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\terminal_janitor')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$name = 'terminal_janitor.exe'
$binary = Join-Path $Prefix $name

if (Test-Path -LiteralPath $binary) {
    # Disable is idempotent and succeeds when nothing is installed, so this is
    # safe whether or not the user ever enabled a schedule.
    Write-Host 'uninstall: removing any installed schedule'
    try {
        & $binary disable | Out-Null
    }
    catch {
        Write-Warning 'uninstall: the binary could not remove its schedule; check it by hand'
    }
    Remove-Item -LiteralPath $binary -Force
    Write-Host "uninstall: removed $binary"
}
else {
    Write-Host "uninstall: no binary at $binary"
}

# Leave the directory only if this product put nothing else there.
if ((Test-Path -LiteralPath $Prefix) -and
    -not (Get-ChildItem -LiteralPath $Prefix -Force)) {
    Remove-Item -LiteralPath $Prefix -Force
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $entries = $userPath -split ';' | Where-Object { $_ -ne '' -and $_ -ne $Prefix }
    if ($entries.Count -ne ($userPath -split ';' | Where-Object { $_ -ne '' }).Count) {
        [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
        Write-Host "uninstall: removed $Prefix from your user PATH"
    }
}

if ($Purge) {
    $roaming = Join-Path $env:APPDATA 'terminal_janitor'
    $local = Join-Path $env:LOCALAPPDATA 'terminal_janitor'
    foreach ($directory in @($roaming, $local)) {
        if (Test-Path -LiteralPath $directory) {
            Remove-Item -LiteralPath $directory -Recurse -Force
            Write-Host "uninstall: purged $directory"
        }
    }
}
else {
    Write-Host 'uninstall: configuration and protection state were kept; -Purge removes them'
}
