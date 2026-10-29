# Smoke-run the console on a Windows desktop (the CI runner's): the sign-in
# window, then the stress pass on the synthetic plant (no server needed),
# each captured as a PNG of the primary screen.
# Usage: native/scripts/smoke-windows.ps1 -Exe <voltrus-native.exe> -Out <dir>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$Exe = (Resolve-Path $Exe).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

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

# The sign-in window: it must come up and stay up.
$console = Start-Process -FilePath $Exe -PassThru `
    -RedirectStandardOutput (Join-Path $Out 'sign-in.out.log') `
    -RedirectStandardError (Join-Path $Out 'sign-in.err.log')
Start-Sleep -Seconds 10
Save-Screen 'sign-in'
if ($console.HasExited) {
    throw "the console exited during the sign-in smoke run (code $($console.ExitCode))"
}
Stop-Process -Id $console.Id -Force
$console.WaitForExit()

# The stress pass: 20 s of the synthetic plant through the live store path,
# switching pages; it writes its report and quits on its own.
$report = Join-Path $Out 'stress-report.txt'
$stress = Start-Process -FilePath $Exe -PassThru `
    -ArgumentList @('--stress', '--stress-seconds', '20', '--stress-report', $report, '--hud') `
    -RedirectStandardOutput (Join-Path $Out 'stress.out.log') `
    -RedirectStandardError (Join-Path $Out 'stress.err.log')
Start-Sleep -Seconds 12
Save-Screen 'stress'
if (-not $stress.WaitForExit(120000)) {
    Stop-Process -Id $stress.Id -Force
    throw 'the stress pass did not finish within 120 s'
}
if ($stress.ExitCode -ne 0) { throw "the stress pass exited with code $($stress.ExitCode)" }
if (-not (Test-Path $report)) { throw 'the stress pass wrote no report' }
Get-Content $report
