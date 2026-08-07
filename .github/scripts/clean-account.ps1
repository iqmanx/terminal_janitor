# Runs as a freshly created local account with its own profile, its own
# AppData, and its own user PATH. Nothing here is elevated.
#
# The result is written to a file because Start-Process -Credential cannot
# hand the inner exit code back to the caller.
$ErrorActionPreference = 'Stop'
$share = 'C:\tjshare'
$result = Join-Path $share 'result.txt'

try {
    Set-Location $env:USERPROFILE

    & (Join-Path $share 'install.ps1') -From (Join-Path $share 'terminal_janitor.exe')
    $tj = Join-Path $env:LOCALAPPDATA 'Programs\terminal_janitor\terminal_janitor.exe'
    if (-not (Test-Path $tj)) { throw 'the binary was not installed' }
    & $tj --version

    $projects = Join-Path $env:USERPROFILE 'projects'
    $workspace = Join-Path $projects 'api'
    New-Item -ItemType Directory -Path $workspace -Force | Out-Null
    '{"name":"api"}' | Set-Content (Join-Path $workspace 'package.json')
    'packages: []' | Set-Content (Join-Path $workspace 'pnpm-workspace.yaml')
    'lockfileVersion: 9' | Set-Content (Join-Path $workspace 'pnpm-lock.yaml')
    & $tj init --root $projects --json
    & $tj scan --json

    $config = Join-Path $env:APPDATA 'terminal_janitor\config.toml'
    if (-not (Test-Path $config)) { throw 'no configuration was written' }

    # Installing must never schedule anything.
    schtasks /Query /TN terminal_janitor 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { throw 'installing enabled a schedule' }

    # Upgrading in place must not disturb configuration.
    $before = (Get-FileHash $config -Algorithm SHA256).Hash
    & (Join-Path $share 'install.ps1') -From (Join-Path $share 'terminal_janitor.exe')
    $after = (Get-FileHash $config -Algorithm SHA256).Hash
    if ($before -ne $after) { throw 'upgrade changed configuration' }

    & (Join-Path $share 'uninstall.ps1')
    if (Test-Path $tj) { throw 'the binary survived uninstall' }
    if (-not (Test-Path $config)) { throw 'uninstall removed state it was told to keep' }

    & (Join-Path $share 'uninstall.ps1') -Purge
    if (Test-Path $config) { throw 'purge left configuration behind' }

    'CLEAN_ACCOUNT_OK' | Set-Content $result
}
catch {
    "CLEAN_ACCOUNT_FAILED: $_" | Set-Content $result
}
