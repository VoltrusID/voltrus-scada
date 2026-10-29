# End-to-end check of the Windows installer as a local station, on a
# disposable machine (the CI runner): installs silently, checks that the
# VoltrusSCADA service runs and answers on 127.0.0.1 with the station's
# credential, starts the installed console with no arguments (local station
# mode signs in by itself) and captures the screen, then uninstalls silently
# and checks that the service is gone and the plant data kept.
# Usage: native/scripts/station-windows.ps1 -Installer <setup.exe> -Out <dir>
param(
    [Parameter(Mandatory)] [string] $Installer,
    [Parameter(Mandatory)] [string] $Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$Installer = (Resolve-Path $Installer).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$app = Join-Path $env:ProgramFiles 'Voltrus SCADA'
$voltrus = Join-Path $app 'voltrus.exe'
$console = Join-Path $app 'voltrus-native.exe'
$station = Join-Path $env:ProgramData 'Voltrus SCADA'
$serviceName = 'VoltrusSCADA'

function Save-Screen([string] $Name) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $path = Join-Path $Out "$Name.png"
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Write-Host "captured $Name ($($bounds.Width)x$($bounds.Height))"
}

# The station's logs, copied next to the screenshots (readable by
# administrators only on the machine).
function Save-StationLogs {
    $logs = Join-Path $station 'logs'
    if (Test-Path $logs) {
        Copy-Item -Recurse -Force $logs (Join-Path $Out 'station-logs')
    }
}

function Wait-Until([scriptblock] $Condition, [int] $Seconds, [string] $What) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 500
    }
    throw "timed out after $Seconds s waiting for $What"
}

# A file another process is still writing (the console's redirected stderr).
function Read-Shared([string] $Path) {
    if (-not (Test-Path $Path)) { return '' }
    $stream = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
    try { return [System.IO.StreamReader]::new($stream).ReadToEnd() } finally { $stream.Dispose() }
}

try {
    # Install, as a fleet tool would.
    $setup = Start-Process -FilePath $Installer -PassThru -Wait -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$(Join-Path $Out 'setup.log')`""
    )
    if ($setup.ExitCode -ne 0) { throw "the installer exited with code $($setup.ExitCode)" }
    foreach ($file in $voltrus, $console, (Join-Path $app 'LICENSE')) {
        if (-not (Test-Path $file)) { throw "the installer did not install $file" }
    }

    # The service, as Windows and the server report it.
    Wait-Until {
        (Get-Service -Name $serviceName -ErrorAction SilentlyContinue).Status -eq 'Running'
    } 30 "the $serviceName service to run"
    & $voltrus service status
    if ($LASTEXITCODE -ne 0) { throw "voltrus service status exited with code $LASTEXITCODE (0 is running)" }
    Write-Host "service $serviceName is running"

    # The console's credential, and the server's config locked away from Users.
    $credential = @{}
    foreach ($line in Get-Content (Join-Path $station 'station.toml')) {
        if ($line -match '^\s*(\w+)\s*=\s*"(.*)"\s*$') { $credential[$Matches[1]] = $Matches[2] }
    }
    foreach ($key in 'url', 'username', 'password') {
        if (-not $credential[$key]) { throw "station.toml has no $key" }
    }
    $url = $credential.url.TrimEnd('/')
    if (([uri] $url).Host -ne '127.0.0.1') { throw "the station answers on $url, not on 127.0.0.1" }
    $users = [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-545')
    function Test-UsersCanRead([string] $Path) {
        foreach ($rule in (Get-Acl $Path).Access) {
            $sid = $rule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier])
            if ($sid -eq $users -and $rule.AccessControlType -eq 'Allow' -and
                ([int] ($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::ReadData)) -ne 0) {
                return $true
            }
        }
        return $false
    }
    if (Test-UsersCanRead (Join-Path $station 'config.yaml')) { throw 'Users can read config.yaml' }
    if (-not (Test-UsersCanRead (Join-Path $station 'station.toml'))) { throw 'Users cannot read station.toml' }
    Write-Host 'config.yaml is closed to Users; station.toml is readable'

    # The server: health, then the console's credential.
    Wait-Until {
        try { (Invoke-WebRequest -Uri "$url/api/v1/health" -TimeoutSec 5).StatusCode -eq 200 } catch { $false }
    } 60 "$url/api/v1/health"
    Write-Host "$url/api/v1/health answers"
    $body = @{ username = $credential.username; password = $credential.password } | ConvertTo-Json
    $login = Invoke-RestMethod -Method Post -Uri "$url/api/v1/auth/login" -ContentType 'application/json' -Body $body
    if (-not $login.ok) { throw 'the station credential did not sign in' }
    if ($login.role -ne 'admin') { throw "the station user's role is $($login.role), not admin" }
    if ($login.must_change_password) { throw 'the station user must change its password' }
    Write-Host "signed in to $url as $($credential.username)"

    # The installed console with no arguments: local station mode signs in by
    # itself.
    $stderr = Join-Path $Out 'station.err.log'
    $running = Start-Process -FilePath $console -PassThru -WorkingDirectory $app `
        -RedirectStandardOutput (Join-Path $Out 'station.out.log') -RedirectStandardError $stderr
    try {
        Wait-Until {
            if ($running.HasExited) { throw "the console exited with code $($running.ExitCode)" }
            (Read-Shared $stderr).Contains('station: signed in to')
        } 60 'the console to sign in to the station'
        Write-Host 'the console signed in to the station'
        # The first page's data and layout.
        Start-Sleep -Seconds 8
        Save-Screen 'station'
        if ($running.HasExited) { throw "the console exited with code $($running.ExitCode)" }
    } finally {
        if (-not $running.HasExited) { Stop-Process -Id $running.Id -Force }
        $running.WaitForExit()
    }

    # Uninstall. The uninstaller hands over to a copy of itself in %TEMP%,
    # so its first process ends early: wait for the result instead.
    $uninstaller = Join-Path $app 'unins000.exe'
    $remove = Start-Process -FilePath $uninstaller -PassThru -Wait -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$(Join-Path $Out 'uninstall.log')`""
    )
    if ($remove.ExitCode -ne 0) { throw "the uninstaller exited with code $($remove.ExitCode)" }
    Wait-Until { -not (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) } 120 "the $serviceName service to go"
    Wait-Until { -not (Test-Path $voltrus) -and -not (Test-Path $console) } 120 'the uninstaller to remove the binaries'
    Write-Host "service $serviceName is gone"
    foreach ($kept in 'config.yaml', 'data.db', 'station.toml') {
        if (-not (Test-Path (Join-Path $station $kept))) { throw "the uninstaller removed the plant data ($kept)" }
    }
    Write-Host "the plant data stays in $station"
} finally {
    Save-StationLogs
}
