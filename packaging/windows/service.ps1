<#
  Ferrent service management for Windows, wrapped with NSSM.

  The daemon is a console program that handles Ctrl-C (that is where it saves
  fastresume), so the wrapper must stop it with a console event rather than a
  kill: NSSM's console stop method does exactly that.

  Run from an elevated PowerShell:

    .\service.ps1 install     # data dir, token, config, service, start
    .\service.ps1 status
    .\service.ps1 stop
    .\service.ps1 start
    .\service.ps1 uninstall   # removes the service, keeps %ProgramData%\ferrent
#>
param(
    [Parameter(Position = 0)]
    [ValidateSet('install', 'uninstall', 'start', 'stop', 'status', 'restart')]
    [string]$Action = 'status',

    [string]$BinDir = $PSScriptRoot,
    [string]$DataDir = (Join-Path $env:ProgramData 'ferrent')
)

$ErrorActionPreference = 'Stop'
$serviceName = 'Ferrent'
$nssm = Join-Path $BinDir 'nssm.exe'
$daemon = Join-Path $BinDir 'ferrentd.exe'
$configPath = Join-Path $DataDir 'config.toml'
$tokenPath = Join-Path $DataDir 'token'
$logDir = Join-Path $DataDir 'logs'

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Run this script from an elevated PowerShell (Run as administrator).'
    }
}

function ConvertTo-TomlPath([string]$path) {
    return $path.Replace('\', '/')
}

function New-Token([string]$path) {
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    [IO.File]::WriteAllText($path, [Convert]::ToBase64String($bytes), [Text.Encoding]::ASCII)
}

# %ProgramData%\ferrent inherits an ACL that lets every user read, so the files
# that hold secrets get the Linux package's 0640 root:ferrent equivalent:
# SYSTEM and Administrators full, nobody else, no inheritance. The SIDs are
# spelled out because a Hungarian Windows calls the group "Rendszergazdák" and
# icacls would not resolve the English name.
function Protect-File([string]$path) {
    & icacls $path /inheritance:r /grant:r '*S-1-5-18:(F)' '*S-1-5-32-544:(F)' | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "icacls failed on $path (exit $LASTEXITCODE); the file stays readable by Users"
    }
}

function Invoke-Nssm {
    & $nssm @args
    if ($LASTEXITCODE -ne 0) { throw "nssm $args failed with exit code $LASTEXITCODE" }
}

Assert-Admin
if (-not (Test-Path $nssm)) { throw "nssm.exe not found next to the script ($nssm)" }
if (-not (Test-Path $daemon)) { throw "ferrentd.exe not found next to the script ($daemon)" }

switch ($Action) {
    'install' {
        New-Item -ItemType Directory -Force -Path $DataDir, (Join-Path $DataDir 'data'), $logDir | Out-Null
        if (-not (Test-Path $tokenPath)) { New-Token $tokenPath }
        Protect-File $tokenPath
        if (-not (Test-Path $configPath)) {
            $toml = @"
# Written by service.ps1.
data_dir = "$(ConvertTo-TomlPath (Join-Path $DataDir 'data'))"
bind = "127.0.0.1:50051"
token_file = "$(ConvertTo-TomlPath $tokenPath)"
"@
            Set-Content -Path $configPath -Value $toml -Encoding ASCII
        }
        Protect-File $configPath

        Invoke-Nssm install $serviceName $daemon
        Invoke-Nssm set $serviceName AppDirectory $BinDir
        Invoke-Nssm set $serviceName AppEnvironmentExtra "FERRENT_CONFIG=$configPath"
        # Ctrl-C first, then the process tree; 30 s covers the fastresume save
        # (the daemon's own deadline is 30 s).
        Invoke-Nssm set $serviceName AppStopMethodConsole 30000
        Invoke-Nssm set $serviceName AppStdout (Join-Path $logDir 'ferrentd.out.log')
        Invoke-Nssm set $serviceName AppStderr (Join-Path $logDir 'ferrentd.err.log')
        Invoke-Nssm set $serviceName AppRotateFiles 1
        Invoke-Nssm set $serviceName Start SERVICE_AUTO_START
        Invoke-Nssm start $serviceName
        Write-Host "Ferrent installed. Config: $configPath"
        Write-Host "Token (give it to ferrent-gui): $(Get-Content $tokenPath)"
    }
    'uninstall' {
        # A stopped service must not turn the uninstall into a failure.
        & $nssm stop $serviceName | Out-Null
        Invoke-Nssm remove $serviceName confirm
        Write-Host "Service removed; $DataDir kept (delete it by hand if you want the data gone)."
    }
    'start'   { Invoke-Nssm start $serviceName }
    'stop'    { Invoke-Nssm stop $serviceName }
    'restart' { Invoke-Nssm restart $serviceName }
    'status'  {
        & $nssm status $serviceName
        Get-Service -Name $serviceName -ErrorAction SilentlyContinue | Format-List Name, Status, StartType
    }
}
