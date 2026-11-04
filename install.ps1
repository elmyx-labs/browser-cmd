# Installs the browser-cmd binary from GitHub Releases on Windows, checked against the release's SHA256SUMS,
# and its agent skill for every coding agent found on this machine (Claude Code, Codex, Pi).
param([switch]$NoSkills)
$ErrorActionPreference = 'Stop'

$repo = 'elmyx-labs/browser-cmd'
$binDir = if ($env:BROWSER_CMD_INSTALL_DIR) { $env:BROWSER_CMD_INSTALL_DIR } else { "$env:LOCALAPPDATA\browser-cmd\bin" }
$version = if ($env:BROWSER_CMD_INSTALL_VERSION) { $env:BROWSER_CMD_INSTALL_VERSION } else { 'latest' }
$target = 'x86_64-pc-windows-msvc'
$asset = "browser-cmd-$target.zip"
$base = if ($version -eq 'latest') { "https://github.com/$repo/releases/latest/download" } else { "https://github.com/$repo/releases/download/$version" }

function Put-Skill($skill, $dir) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item $skill "$dir\SKILL.md" -Force
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) "browser-cmd-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Invoke-WebRequest -Uri "$base/$asset" -OutFile "$tmp\$asset"
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS"
    $expected = (Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match "\s$([regex]::Escape($asset))$" }) -replace '\s.*$', ''
    $actual = (Get-FileHash "$tmp\$asset" -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) { throw "checksum mismatch for $asset" }
    Expand-Archive "$tmp\$asset" -DestinationPath $tmp
    $unpacked = "$tmp\browser-cmd-$target"

    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    Copy-Item "$unpacked\browser-cmd.exe" "$binDir\browser-cmd.exe" -Force
    Write-Host "Installed $(& "$binDir\browser-cmd.exe" --version) to $binDir"
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $binDir) {
        [Environment]::SetEnvironmentVariable('Path', "$binDir;$userPath", 'User')
        $env:Path = "$binDir;$env:Path"
        Write-Host "Added $binDir to your PATH; terminals opened from now on see it."
    }

    # The skill goes where each agent on this machine reads skills: Claude Code in ~\.claude\skills,
    # Codex and Pi in ~\.agents\skills. An agent whose directory is absent is not installed here.
    if (-not $NoSkills) {
        $skill = "$unpacked\skills\browser-cmd\SKILL.md"
        $installed = @()
        if (Test-Path "$HOME\.claude") {
            Put-Skill $skill "$HOME\.claude\skills\browser-cmd"
            $installed += 'Claude Code (/browser-cmd)'
        }
        if ((Test-Path "$HOME\.codex") -or (Test-Path "$HOME\.pi") -or (Test-Path "$HOME\.agents")) {
            Put-Skill $skill "$HOME\.agents\skills\browser-cmd"
            if (Test-Path "$HOME\.codex") { $installed += 'Codex ($browser-cmd)' }
            if (Test-Path "$HOME\.pi") { $installed += 'Pi (/skill:browser-cmd)' }
        }
        if ($installed) {
            Write-Host "Installed the browser-cmd skill for: $($installed -join ' ')"
            Write-Host 'A new session sees it; a running one may need /reload-skills (Claude Code) or /reload (Pi).'
        } else {
            Write-Host 'No coding agent found (~\.claude, ~\.codex, ~\.pi); the skill is skills\browser-cmd\SKILL.md in the release archive.'
        }
    }
} finally {
    Remove-Item -Recurse -Force $tmp
}

# browser-cmd finds a stable Google Chrome itself; this only warns early when there is none.
$chrome = @("$env:ProgramFiles\Google\Chrome\Application\chrome.exe", "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe", "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path $_ }
if (-not $chrome) {
    Write-Host "Google Chrome (stable) was not found. browser-cmd drives an installed Chrome: install it from https://www.google.com/chrome/, or set BROWSER_CMD_EXECUTABLE to a Chrome binary, or BROWSER_CMD_CDP to the DevTools endpoint of a running one (see https://github.com/$repo/blob/main/docs/your-chrome.md)."
}
