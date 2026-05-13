# Install the minitodo skill into ~/.claude/skills/minitodo/ on Windows.
#
# Run via PowerShell:
#     .\install.ps1

$ErrorActionPreference = "Stop"

$SrcDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$DestDir = Join-Path $env:USERPROFILE ".claude\skills\minitodo"

Write-Host ">> installing minitodo skill into $DestDir"
New-Item -ItemType Directory -Path $DestDir -Force | Out-Null

Copy-Item -Force (Join-Path $SrcDir "SKILL.md")            (Join-Path $DestDir "SKILL.md")
Copy-Item -Force (Join-Path $SrcDir "minitodo.py")         (Join-Path $DestDir "minitodo.py")
Copy-Item -Force (Join-Path $SrcDir "config.example.toml") (Join-Path $DestDir "config.example.toml")

$ConfigPath = Join-Path $DestDir "config.toml"
if (-not (Test-Path $ConfigPath)) {
    Copy-Item (Join-Path $SrcDir "config.example.toml") $ConfigPath
    Write-Host "!! $ConfigPath created from example. Please edit it to fill in"
    Write-Host "   'endpoint' (e.g. https://minitodo.example.com) and 'api_key'."
} else {
    Write-Host ">> $ConfigPath already exists, kept untouched."
}

# Sanity check: Python + requests
$Py = $null
foreach ($cmd in @("python", "python3")) {
    $found = Get-Command $cmd -ErrorAction SilentlyContinue
    if ($found) { $Py = $cmd; break }
}
if (-not $Py) {
    Write-Warning "Python 3 not found in PATH. Install Python 3.10+ to use the skill."
    exit 0
}

& $Py -c "import requests" 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Warning "'requests' not installed for $Py. Run:"
    Write-Host  "    $Py -m pip install requests"
}

# tomli needed on <3.11
& $Py -c "import sys; sys.exit(0 if sys.version_info >= (3,11) else 1)" 2>$null
if ($LASTEXITCODE -ne 0) {
    & $Py -c "import tomli" 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Python <3.11 detected; install tomli:"
        Write-Host  "    $Py -m pip install tomli"
    }
}

Write-Host ">> done. Test with:"
Write-Host  "    $Py $DestDir\minitodo.py health --json"
