# Installs the newest Sage on Windows:
#
#   irm https://raw.githubusercontent.com/Aakshar-D/dots/master/install.ps1 | iex
#
# It reads the newest release's `latest.json`, picks the installer for this PC's processor,
# downloads it from this repository's releases and runs it. A file fetched this way is not
# marked as coming from the internet, so Windows does not ask about it first.
#
# Set before running, when wanted:
#   $env:SAGE_VERSION = '0.5.4'   installs that version, not the newest
#   $env:SAGE_SILENT  = '1'       installs without the installer's windows
#   $env:SAGE_DRY_RUN = '1'       says what it would download, and stops

# In a block of its own, so that a run through `iex` leaves nothing behind in the PowerShell
# it was typed into: no variables, and no changed error setting.
& {
    $ErrorActionPreference = 'Stop'

    # GitHub answers only TLS 1.2 and newer, which an older Windows PowerShell does not offer
    # unless asked. Left alone where Windows itself chooses (the value 0).
    $tls = [Net.ServicePointManager]::SecurityProtocol
    if ([int]$tls -ne 0 -and -not ($tls -band [Net.SecurityProtocolType]::Tls12)) {
        [Net.ServicePointManager]::SecurityProtocol = $tls -bor [Net.SecurityProtocolType]::Tls12
    }

    $repo = 'Aakshar-D/dots'
    $releases = "https://github.com/$repo/releases"

    # The release to install: the newest, or the one asked for.
    $base = "$releases/latest/download"
    if ($env:SAGE_VERSION) {
        $base = "$releases/download/v$($env:SAGE_VERSION.TrimStart('v'))"
    }

    # A manifest on disk is for the release's own check (tools/release/install.test.mjs).
    if ($env:SAGE_MANIFEST_FILE) {
        $manifest = Get-Content -Raw $env:SAGE_MANIFEST_FILE | ConvertFrom-Json
    } else {
        try {
            $manifest = Invoke-RestMethod -UseBasicParsing "$base/latest.json"
        } catch {
            throw "Sage's release could not be read from $base/latest.json. Check your connection, or the version asked for."
        }
    }

    # An ARM64 PC gets the ARM64 installer when the release has one; every other PC, and a
    # release without it, gets x64, which runs on both.
    # The processor of the PC itself, not of this PowerShell: an x64 PowerShell on an ARM64 PC
    # is told x64 by its own environment and by .NET, so Windows' own record is read first.
    # `SAGE_ARCH` (`x64` or `arm64`) is for the release's own check.
    $arch = $env:SAGE_ARCH
    if (-not $arch) {
        try {
            $key = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
            $arch = [string](Get-ItemProperty -Path $key -Name PROCESSOR_ARCHITECTURE).PROCESSOR_ARCHITECTURE
        } catch {
            $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
        }
    }
    $arm = $arch -match '^(arm64|aarch64)$'
    $entry = $null
    if ($arm) { $entry = $manifest.platforms.'windows-aarch64' }
    if (-not $entry) { $entry = $manifest.platforms.'windows-x86_64' }
    if (-not $entry -or -not $entry.url) {
        throw "Sage $($manifest.version) has no Windows installer."
    }

    # Only a file of this repository's releases is ever run.
    $url = [string]$entry.url
    if (-not $url.StartsWith("$releases/download/")) {
        throw "The release names a download outside $releases, so nothing was installed."
    }

    Write-Host "Sage $($manifest.version): $url"
    if ($env:SAGE_DRY_RUN) { return }

    $file = Join-Path $env:TEMP "Sage-$($manifest.version)-setup.exe"
    $progress = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'   # the progress bar slows the download many times over
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $file
    } finally {
        $ProgressPreference = $progress
    }

    Write-Host 'Installing...'
    $arguments = @()
    if ($env:SAGE_SILENT) { $arguments += '/S' }
    if ($arguments.Count -gt 0) {
        $installer = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru
    } else {
        $installer = Start-Process -FilePath $file -Wait -PassThru
    }
    Remove-Item $file -ErrorAction SilentlyContinue
    if ($installer.ExitCode -ne 0) {
        throw "The installer ended with code $($installer.ExitCode). Sage may not be installed."
    }
    Write-Host "Sage $($manifest.version) is installed. Find it in the Start menu."
}
