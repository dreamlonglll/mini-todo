# Install the minitodo skill (Windows PowerShell).
#
# 默认安装到 ~\.claude\skills\minitodo\（Claude Code）。
# -Target openclaw 装到 ~\.openclaw\workspace\skills\minitodo\。
# -Target both 同时安装两份（用同一份 config.toml 时需自行建链接）。
#
# 用法:
#   .\install.ps1
#   .\install.ps1 -Target openclaw
#   .\install.ps1 -Target both

[CmdletBinding()]
param(
    [ValidateSet('claude', 'openclaw', 'both')]
    [string]$Target = 'claude'
)

$ErrorActionPreference = 'Stop'

$SrcDir = Split-Path -Parent $MyInvocation.MyCommand.Path

$Dests = switch ($Target) {
    'claude'   { @(Join-Path $HOME '.claude\skills\minitodo') }
    'openclaw' { @(Join-Path $HOME '.openclaw\workspace\skills\minitodo') }
    'both'     {
        @(
            (Join-Path $HOME '.claude\skills\minitodo'),
            (Join-Path $HOME '.openclaw\workspace\skills\minitodo')
        )
    }
}

foreach ($DestDir in $Dests) {
    Write-Host ">> installing minitodo skill into $DestDir"
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null

    Copy-Item -Force (Join-Path $SrcDir 'SKILL.md')            (Join-Path $DestDir 'SKILL.md')
    Copy-Item -Force (Join-Path $SrcDir 'minitodo.py')         (Join-Path $DestDir 'minitodo.py')
    Copy-Item -Force (Join-Path $SrcDir 'config.example.toml') (Join-Path $DestDir 'config.example.toml')

    $ConfigPath = Join-Path $DestDir 'config.toml'
    if (-not (Test-Path $ConfigPath)) {
        Copy-Item (Join-Path $SrcDir 'config.example.toml') $ConfigPath
        Write-Host "!! $ConfigPath created from example. Please edit it to fill in"
        Write-Host "   'endpoint' (e.g. https://minitodo.example.com) and 'api_key'."
    }
    else {
        Write-Host ">> $ConfigPath already exists, kept untouched."
    }
}

# Sanity check: Python + requests
$Py = $null
foreach ($candidate in @('python', 'python3', 'py')) {
    if (Get-Command $candidate -ErrorAction SilentlyContinue) {
        $Py = $candidate
        break
    }
}
if (-not $Py) {
    Write-Host "!! WARNING: Python 3 not found in PATH. Install Python 3.10+ to use the skill."
    exit 0
}

& $Py -c 'import requests' 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Host "!! 'requests' not installed for $Py. Run:"
    Write-Host "     $Py -m pip install requests"
}

# tomli only needed on <3.11
& $Py -c 'import sys; sys.exit(0 if sys.version_info >= (3,11) else 1)' 2>$null
if ($LASTEXITCODE -ne 0) {
    & $Py -c 'import tomli' 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "!! Python <3.11 detected; install tomli:"
        Write-Host "     $Py -m pip install tomli"
    }
}

Write-Host ">> done. Test with:"
Write-Host "     $Py $($Dests[0])\minitodo.py health --json"
