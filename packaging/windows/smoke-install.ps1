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

function Test-AuthenticatedWebView($WebSocketUrl) {
    # Only a boolean leaves the page. The ephemeral app token remains inside its WebView.
    $expression = @'
(async () => {
  if (!document.getElementById('app-shell') || !document.getElementById('message-stream')) return false;
  const token = sessionStorage.getItem('re-search-token');
  if (!token) return false;
  const headers = { 'X-ReSearch-Token': token };
  const sessions = await fetch('/api/sessions', { headers });
  const connection = await fetch('/api/connections', { headers });
  if (!sessions.ok || !connection.ok) return false;
  const status = await connection.json();
  return status.capabilities?.sympy_worker === 'configured';
})()
'@
    $client = [Net.WebSockets.ClientWebSocket]::new()
    $timeout = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds(5))
    try {
        $uri = [Uri] $WebSocketUrl
        if ($uri.Scheme -ne 'ws' -or $uri.Host -ne '127.0.0.1') { throw 'Unexpected WebView debugging endpoint.' }
        [void] $client.ConnectAsync($uri, $timeout.Token).GetAwaiter().GetResult()
        $request = @{
            id = 1
            method = 'Runtime.evaluate'
            params = @{ expression = $expression; awaitPromise = $true; returnByValue = $true }
        } | ConvertTo-Json -Depth 5 -Compress
        $requestBytes = [ArraySegment[byte]]::new([Text.Encoding]::UTF8.GetBytes($request))
        [void] $client.SendAsync($requestBytes, [Net.WebSockets.WebSocketMessageType]::Text, $true, $timeout.Token).GetAwaiter().GetResult()
        $buffer = [ArraySegment[byte]]::new([byte[]]::new(16384))
        while ($client.State -eq [Net.WebSockets.WebSocketState]::Open) {
            $reply = $client.ReceiveAsync($buffer, $timeout.Token).GetAwaiter().GetResult()
            if ($reply.MessageType -ne [Net.WebSockets.WebSocketMessageType]::Text -or -not $reply.EndOfMessage) { return $false }
            $message = [Text.Encoding]::UTF8.GetString($buffer.Array, $buffer.Offset, $reply.Count) | ConvertFrom-Json
            if ($message.id -eq 1) { return $message.result.result.value -eq $true }
        }
        return $false
    } finally {
        $client.Dispose()
        $timeout.Dispose()
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
    & (Join-Path $installDir 're-search.exe') sessions --data-dir $smokeData

    $port = Get-LoopbackPort
    do { $debugPort = Get-LoopbackPort } while ($debugPort -eq $port)
    $origin = "http://127.0.0.1:$port"
    $debugEndpoint = "http://127.0.0.1:$debugPort/json/list"
    $debugArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    try {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-address=127.0.0.1 --remote-debugging-port=$debugPort"
        $app = Start-Process -FilePath $appPath -ArgumentList @('--data-dir', "`"$smokeData`"", '--port', "$port") -PassThru
    } finally {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $debugArgs
    }

    $ready = $false
    $serverReady = $false
    $webviewFound = $false
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(75)
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        $app.Refresh()
        if ($app.HasExited) { throw "Installed WebView2 app exited with code $($app.ExitCode)." }
        try {
            $health = Invoke-RestMethod -Uri "$origin/api/health" -TimeoutSec 2
            $serverReady = $health.ok -and $health.version -ceq $Version
            $pages = Invoke-RestMethod -Uri $debugEndpoint -TimeoutSec 2
            $page = $pages | Where-Object { $_.type -eq 'page' -and $_.url -eq "$origin/" } | Select-Object -First 1
            $webviewFound = [bool]$page.webSocketDebuggerUrl
            if ($serverReady -and $webviewFound -and (Test-AuthenticatedWebView $page.webSocketDebuggerUrl)) {
                $ready = $true
                break
            }
        } catch {
            # The server, WebView and authenticated app boot asynchronously.
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "Installed app did not load the authenticated workbench with bundled Python configured (server ready: $serverReady, WebView page found: $webviewFound)." }
    Write-Host 'Installed WebView2 authenticated workbench detected bundled Python.'

    $python = Join-Path $installDir 'runtime\python.exe'
    & $python -c "import sys, sympy, mpmath, numpy; assert sys.maxsize > 2**32"
    $savedPython = $env:RE_SEARCH_PYTHON
    try {
        $env:RE_SEARCH_PYTHON = $python
        cargo test --locked --release --target $Target -p worker --test appcontainer_sandbox
    } finally {
        $env:RE_SEARCH_PYTHON = $savedPython
    }
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
