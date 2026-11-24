# Installs to the current runner user's fixed NSIS location and requires a clean user profile.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $Installer,
    [Parameter(Mandatory)][string] $Stage,
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $Target
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\reSearch'
$uninstall = Join-Path $installDir 'uninstall.exe'
$registryKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\reSearch'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'reSearch.lnk'
if ((Test-Path $installDir) -or (Test-Path $registryKey) -or (Test-Path $shortcut)) {
    throw 'The installed-app test needs a user profile without an existing reSearch installation.'
}

function Get-LoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        $listener.LocalEndpoint.Port
    } finally {
        $listener.Stop()
    }
}

function Test-InstalledWorkbenchControls([int] $AppProcessId) {
    # Windows PowerShell runs UI Automation in STA. Time-limit this optional
    # path because the hosted desktop may not expose a WebView accessibility tree.
    $probe = @'
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$button = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button
)
$known = @('New research', 'Canvas', 'Connect', 'Settings')
$found = [Collections.Generic.HashSet[string]]::new()
$buttonCount = 0
$deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $app = Get-Process -Id __APP_PROCESS_ID__
    if ($app.MainWindowHandle -ne [IntPtr]::Zero) {
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($app.MainWindowHandle)
        $buttons = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $button)
        $buttonCount = $buttons.Count
        foreach ($item in $buttons) {
            $name = $item.Current.Name
            if ($name -like '*Test numerical convergence*') { exit 0 }
            if ($name -in $known) { [void] $found.Add($name) }
        }
    }
    Start-Sleep -Milliseconds 500
}
Write-Host "UI Automation saw $buttonCount buttons (known controls: $($found -join ', '))."
exit 1
'@
    $probe = $probe.Replace('__APP_PROCESS_ID__', [string]$AppProcessId)
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probe))
    $legacy = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $uia = Start-Process -FilePath $legacy -ArgumentList '-NoProfile', '-NonInteractive', '-STA', '-EncodedCommand', $encoded -NoNewWindow -PassThru
    try {
        if (-not $uia.WaitForExit(24000)) {
            $uia.Kill()
            $uia.WaitForExit()
            return $false
        }
        return $uia.ExitCode -eq 0
    } finally {
        $uia.Dispose()
    }
}

function Test-InstalledWorkbenchApi([string] $CliPath, [string] $DataDir) {
    # Check default worker discovery through the installed CLI's headless launch.
    # Keep credentials in memory and out of the log.
    $start = [Diagnostics.ProcessStartInfo]::new($CliPath)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.WorkingDirectory = $installDir
    foreach ($argument in 'ui', '--no-open', 'true', '--print-url-json', 'true', '--data-dir', $DataDir) {
        $start.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $line = $process.StandardOutput.ReadLineAsync().WaitAsync([TimeSpan]::FromSeconds(20)).GetAwaiter().GetResult()
        try {
            $launch = $line | ConvertFrom-Json
            $origin = [uri]$launch.base_url
            if ($origin.Scheme -cne 'http' -or $origin.Host -cne '127.0.0.1' -or $origin.AbsolutePath -ne '/' -or
                $origin.UserInfo -or $origin.Query -or $origin.Fragment -or $launch.token -cnotmatch '^[0-9a-f]{64}$') {
                throw 'Invalid local launch info.'
            }
        } catch {
            throw 'Installed CLI did not report valid local launch information.'
        }
        $headers = @{ 'X-ReSearch-Token' = $launch.token }
        $base = $origin.GetLeftPart([UriPartial]::Authority)
        $sessions = Invoke-RestMethod -Uri "$base/api/sessions" -Headers $headers -TimeoutSec 4 -NoProxy -MaximumRedirection 0
        if ($null -eq $sessions.sessions) { throw 'Installed CLI did not return the authenticated session list.' }
        $connections = Invoke-RestMethod -Uri "$base/api/connections" -Headers $headers -TimeoutSec 4 -NoProxy -MaximumRedirection 0
        if ($connections.capabilities.sympy_worker -cne 'configured') {
            throw 'Installed CLI did not discover the bundled Python worker.'
        }
    } finally {
        if (-not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        $process.Dispose()
    }
}

$smokeRoot = New-Item -ItemType Directory -Path (Join-Path $env:RUNNER_TEMP 'reSearch install smoke')
$smokeData = Join-Path $smokeRoot.FullName 'ledger'
$defaultData = Join-Path $env:LOCALAPPDATA 're-search'
$marker = Join-Path $defaultData 'installer-smoke-preserved.txt'
$app = $null
try {
    New-Item -ItemType Directory -Path $defaultData -Force | Out-Null
    New-Item -ItemType File -Path $marker | Out-Null

    $setup = Start-Process -FilePath $Installer -ArgumentList '/S', '/ACCEPT_PYTHON_LICENSE' -Wait -PassThru
    if ($setup.ExitCode -ne 0) { throw "Installer returned $($setup.ExitCode)." }

    $installRecord = Get-ItemProperty -LiteralPath $registryKey
    if ($installRecord.DisplayVersion -cne $Version) { throw 'The installed version does not match the release tag.' }
    if (-not (Test-Path -LiteralPath $shortcut) -or -not (Test-Path -LiteralPath $uninstall)) {
        throw 'The installer did not register its shortcut and uninstaller.'
    }
    $shortcutTarget = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    $appPath = Join-Path $installDir 're-search-app.exe'
    if ($shortcutTarget -ne $appPath) { throw 'The Start menu shortcut does not launch the installed app.' }

    foreach ($file in Get-ChildItem -LiteralPath $Stage -File -Recurse -Force) {
        $relative = [IO.Path]::GetRelativePath($Stage, $file.FullName)
        $installed = Join-Path $installDir $relative
        if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) { throw "Missing installed payload: $relative" }
        $stagedHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        $installedHash = (Get-FileHash -LiteralPath $installed -Algorithm SHA256).Hash
        if ($stagedHash -ne $installedHash) { throw "Installed payload differs from the staged file: $relative" }
    }
    $failures = [Collections.Generic.List[string]]::new()
    $nativeUiVerified = $false
    try {
        $port = Get-LoopbackPort
        $origin = "http://127.0.0.1:$port"
        $nativeLog = Join-Path $smokeRoot.FullName 'desktop-stderr.txt'
        $app = Start-Process -FilePath $appPath -WorkingDirectory $installDir -RedirectStandardError $nativeLog -ArgumentList @('--data-dir', "`"$smokeData`"", '--port', "$port") -PassThru
        $serverReady = $false
        $webviewStatus = $null
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds(55)
        while ([DateTimeOffset]::UtcNow -lt $deadline) {
            $app.Refresh()
            if ($app.HasExited) { throw "Installed WebView2 app exited with code $($app.ExitCode)." }
            try {
                $health = Invoke-RestMethod -Uri "$origin/api/health" -TimeoutSec 2 -NoProxy
                $serverReady = $health.ok -and $health.version -ceq $Version
            } catch {
                # The loopback server may still be starting.
            }
            $webviewStatus = Get-Content -LiteralPath $nativeLog -Tail 10 -ErrorAction SilentlyContinue |
                Where-Object { $_ -like 're-search-app: window authenticated via *' -or $_ -like 're-search-app: could not hand the session token*' } |
                Select-Object -Last 1
            if ($webviewStatus -like 're-search-app: could not hand the session token*') { throw $webviewStatus }
            if ($serverReady -and $webviewStatus -like 're-search-app: window authenticated via *') { break }
            Start-Sleep -Milliseconds 500
        }
        if ($serverReady) {
            try {
                $nativeUiVerified = Test-InstalledWorkbenchControls $app.Id
            } catch {
                Write-Host "Windows UI Automation unavailable: $($_.Exception.Message)"
            }
        }
        if ($nativeUiVerified) {
            Write-Host 'Installed WebView2 workbench exposes the Test numerical convergence button.'
        } elseif ($serverReady -and $webviewStatus -like 're-search-app: window authenticated via *') {
            Write-Host 'Windows UI Automation did not expose the workbench control. The native WebView reported successful authentication.'
        } else {
            $app.Refresh()
            if ($app.HasExited) { throw "Installed WebView2 app exited with code $($app.ExitCode)." }
            $knownTitle = $app.MainWindowTitle
            if ($knownTitle -notin '', 'reSearch', 'reSearch (not connected)') { $knownTitle = '(other)' }
            throw "Installed WebView2 authentication was not confirmed (app alive: True, server ready: $serverReady, window handle: $($app.MainWindowHandle), title: '$knownTitle', native auth log: $([bool]$webviewStatus))."
        }
    } catch {
        $failures.Add("Native WebView2: $($_.Exception.Message)")
    }

    try {
        Test-InstalledWorkbenchApi (Join-Path $installDir 're-search.exe') (Join-Path $smokeRoot.FullName 'cli-ledger')
        Write-Host 'Installed CLI workbench authenticated and detected bundled Python.'
    } catch {
        $failures.Add("Installed CLI workbench: $($_.Exception.Message)")
    }

    $savedPython = $env:RE_SEARCH_PYTHON
    $savedNativeError = $PSNativeCommandUseErrorActionPreference
    try {
        $python = Join-Path $installDir 'runtime\python.exe'
        & $python -c "import sys, sympy, mpmath, numpy; assert sys.maxsize > 2**32"
        $env:RE_SEARCH_PYTHON = $python
        $PSNativeCommandUseErrorActionPreference = $false
        $testOutput = @(& cargo test --locked --release --target $Target -p worker --test appcontainer_sandbox 2>&1)
        $testCode = $LASTEXITCODE
        $testOutput | ForEach-Object { Write-Host $_ }
        if ($testCode -ne 0) {
            $tail = ($testOutput | Select-Object -Last 20) -join "`n"
            throw "Cargo exited with code ${testCode}.`n$tail"
        }
    } catch {
        $failures.Add("Installed AppContainer worker: $($_.Exception.Message)")
    } finally {
        $PSNativeCommandUseErrorActionPreference = $savedNativeError
        $env:RE_SEARCH_PYTHON = $savedPython
    }
    if ($failures.Count) { throw ($failures -join "`n") }
} finally {
    if ($app -and -not $app.HasExited) {
        if (-not $app.CloseMainWindow() -or -not $app.WaitForExit(10000)) {
            Stop-Process -Id $app.Id -Force
            $app.WaitForExit()
        }
        $app.Dispose()
    }
    try {
        if (Test-Path -LiteralPath $uninstall) {
            $removed = Start-Process -FilePath $uninstall -ArgumentList '/S' -Wait -PassThru
            if ($removed.ExitCode -ne 0 -or (Test-Path -LiteralPath $appPath) -or (Test-Path $registryKey) -or (Test-Path $shortcut)) {
                throw 'Silent uninstall did not remove the application and registrations.'
            }
            if (-not (Test-Path -LiteralPath $marker)) { throw 'Uninstall removed the research data directory.' }
        }
    } finally {
        Remove-Item -LiteralPath $marker -ErrorAction SilentlyContinue
    }
}
