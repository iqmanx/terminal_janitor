# Runs as a freshly created local account with its own profile, its own
# application data, and its own user PATH. Nothing here is elevated.
#
# `Start-Process -Credential` starts the process with the new account's token
# but hands it the *caller's* environment block, so TEMP, USERPROFILE, and the
# AppData variables still point into the build account's profile, which this
# account cannot read. The first run failed exactly there. Everything the
# environment could leak is therefore set explicitly, first thing.
#
# The result is written to a file because Start-Process -Credential cannot hand
# the inner exit code back to the caller.
$ErrorActionPreference = 'Stop'
$share = 'C:\tjshare'
$resultFile = Join-Path $share 'result.txt'

try {
    $profileRoot = 'C:\Users\tjfresh'
    $env:USERPROFILE = $profileRoot
    $env:HOMEDRIVE = 'C:'
    $env:HOMEPATH = '\Users\tjfresh'
    $env:LOCALAPPDATA = Join-Path $profileRoot 'AppData\Local'
    $env:APPDATA = Join-Path $profileRoot 'AppData\Roaming'
    $env:TEMP = Join-Path $profileRoot 'AppData\Local\Temp'
    $env:TMP = $env:TEMP
    foreach ($directory in @($env:LOCALAPPDATA, $env:APPDATA, $env:TEMP)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }

    Set-Location $profileRoot

    & (Join-Path $share 'install.ps1') -From (Join-Path $share 'terminal_janitor.exe')
    $tj = Join-Path $env:LOCALAPPDATA 'Programs\terminal_janitor\terminal_janitor.exe'
    if (-not (Test-Path $tj)) { throw 'the binary was not installed' }
    & $tj --version

    $projects = Join-Path $profileRoot 'projects'
    $workspace = Join-Path $projects 'api'
    New-Item -ItemType Directory -Path $workspace -Force | Out-Null
    '{"name":"api"}' | Set-Content (Join-Path $workspace 'package.json')
    'packages: []' | Set-Content (Join-Path $workspace 'pnpm-workspace.yaml')
    'lockfileVersion: 9' | Set-Content (Join-Path $workspace 'pnpm-lock.yaml')
    & $tj init --root $projects --json
    & $tj scan --json

    # Windows resolves configuration through the known-folder API rather than
    # the environment, so this asks where the file actually landed instead of
    # assuming. It must be somewhere inside this account's own profile.
    $config = Get-ChildItem -Path $profileRoot -Recurse -Filter 'config.toml' `
        -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -like '*terminal_janitor*' } |
        Select-Object -First 1
    if (-not $config) { throw 'no configuration was written inside the fresh profile' }

    # Installing must never schedule anything.
    schtasks /Query /TN terminal_janitor 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { throw 'installing enabled a schedule' }

    # Upgrading in place must not disturb configuration.
    $before = (Get-FileHash $config.FullName -Algorithm SHA256).Hash
    & (Join-Path $share 'install.ps1') -From (Join-Path $share 'terminal_janitor.exe')
    $after = (Get-FileHash $config.FullName -Algorithm SHA256).Hash
    if ($before -ne $after) { throw 'upgrade changed configuration' }

    & (Join-Path $share 'uninstall.ps1')
    if (Test-Path $tj) { throw 'the binary survived uninstall' }
    if (-not (Test-Path $config.FullName)) {
        throw 'uninstall removed state it was told to keep'
    }

    & (Join-Path $share 'uninstall.ps1') -Purge
    if (Test-Path $config.FullName) { throw 'purge left configuration behind' }

    'CLEAN_ACCOUNT_OK' | Set-Content $resultFile
}
catch {
    "CLEAN_ACCOUNT_FAILED: $_" | Set-Content $resultFile
}
