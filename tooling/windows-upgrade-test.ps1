# Upgrades a published KChess Windows installation with a newly built installer, the way
# electron-updater runs it (`--updated /S`), and checks what the upgrade leaves behind.
# Run by .github/workflows/windows-upgrade.yml on a disposable windows-latest runner: it installs
# software, writes to the registry and does not clean up.
param(
  [Parameter(Mandatory)][ValidateSet('default', 'custom', 'allusers')][string]$Mode,
  [Parameter(Mandatory)][string]$OldInstaller,
  [Parameter(Mandatory)][string]$NewInstaller,
  [Parameter(Mandatory)][string]$NewVersion
)

$ErrorActionPreference = 'Stop'
$customFolder = 'C:\Custom Apps\KChess'
$uninstallRoots = @(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
)
$failures = [System.Collections.Generic.List[string]]::new()
$notes = [System.Collections.Generic.List[string]]::new()

function Fail([string]$message) {
  $failures.Add($message)
  Write-Host "::error title=Windows upgrade ($Mode)::$message"
}

function Get-KChessEntries {
  foreach ($root in $uninstallRoots) {
    if (-not (Test-Path $root)) { continue }
    foreach ($key in Get-ChildItem $root) {
      $entry = Get-ItemProperty $key.PSPath
      if ($entry.DisplayName -notlike 'KChess*') { continue }
      $uninstaller = if ($entry.UninstallString -match '^"([^"]+)"') { $Matches[1] } else { $null }
      [pscustomobject]@{
        Hive = $root.Split(':')[0]
        Key = $key.PSChildName
        DisplayName = $entry.DisplayName
        DisplayVersion = $entry.DisplayVersion
        Folder = if ($uninstaller) { Split-Path $uninstaller -Parent } else { $null }
      }
    }
  }
}

function Get-ExeVersion([string]$exe) {
  if (-not (Test-Path $exe)) { return $null }
  # Windows reports the package version 1.2.3 as 1.2.3.0.
  (Get-Item $exe).VersionInfo.ProductVersion -replace '^(\d+\.\d+\.\d+)\.0$', '$1'
}

function Get-Shortcuts {
  $shell = New-Object -ComObject WScript.Shell
  $folders = [ordered]@{
    'Start menu (user)' = [Environment]::GetFolderPath('Programs')
    'Start menu (all users)' = [Environment]::GetFolderPath('CommonPrograms')
    'Desktop (user)' = [Environment]::GetFolderPath('Desktop')
    'Desktop (all users)' = [Environment]::GetFolderPath('CommonDesktopDirectory')
  }
  foreach ($place in $folders.Keys) {
    $link = Join-Path $folders[$place] 'KChess.lnk'
    if (-not (Test-Path $link)) { continue }
    $target = $shell.CreateShortcut($link).TargetPath
    [pscustomobject]@{
      Place = $place
      Link = $link
      Target = $target
      TargetVersion = Get-ExeVersion $target
    }
  }
}

function Invoke-Installer([string]$installer, [string]$arguments) {
  Write-Host "> $installer $arguments"
  $process = Start-Process -FilePath $installer -ArgumentList $arguments -PassThru
  if (-not $process.WaitForExit(600000)) { throw "$installer did not finish within 10 minutes." }
  # An elevated or relaunched installer can outlive the process that started it.
  $name = [IO.Path]::GetFileNameWithoutExtension($installer)
  $deadline = (Get-Date).AddMinutes(10)
  while (Get-Process -Name $name -ErrorAction SilentlyContinue) {
    if ((Get-Date) -gt $deadline) { throw "$name is still running after 10 minutes." }
    Start-Sleep -Seconds 2
  }
  Get-Process -Name KChess -ErrorAction SilentlyContinue | Stop-Process -Force
  Write-Host "  exit code $($process.ExitCode)"
  $process.ExitCode
}

function Show-State([string]$title) {
  Write-Host "`n== $title"
  Get-KChessEntries | Format-Table Hive, DisplayVersion, Folder, Key -AutoSize | Out-String -Width 400 | Write-Host
  Get-Shortcuts | Format-Table Place, Target, TargetVersion -AutoSize | Out-String -Width 400 | Write-Host
}

# 1. Install the published release.
$oldArguments = switch ($Mode) {
  'default' { '/S' }
  'allusers' { '/S /allusers' }
  # /D= must come last and unquoted, even with spaces.
  'custom' { "/S /D=$customFolder" }
}
$exit = Invoke-Installer $OldInstaller $oldArguments
if ($exit -ne 0) { throw "The published installer failed with exit code $exit." }
Show-State 'Published release installed'

$before = @(Get-KChessEntries)
if ($before.Count -ne 1) { throw "Expected one KChess installation before the upgrade, found $($before.Count)." }
$old = $before[0]
$oldExe = Join-Path $old.Folder 'KChess.exe'
$expectedHive = if ($Mode -eq 'allusers') { 'HKLM' } else { 'HKCU' }
if ($old.Hive -ne $expectedHive) { throw "The published release registered in $($old.Hive), not $expectedHive." }
if ($Mode -eq 'custom' -and $old.Folder -ne $customFolder) {
  throw "The published release installed in $($old.Folder), not $customFolder."
}
if (-not (Test-Path $oldExe)) { throw "The published release has no $oldExe." }
if ($old.DisplayVersion -eq $NewVersion) { throw "The published release already has version $NewVersion." }

# 2. Keep something in the user data folder.
$userData = Join-Path $env:APPDATA 'KChess'
$marker = Join-Path $userData 'upgrade-test-marker.txt'
$markerText = [guid]::NewGuid().ToString()
New-Item -ItemType Directory -Force $userData | Out-Null
Set-Content -Path $marker -Value $markerText -NoNewline

# 3. Upgrade as electron-updater does for a silent install.
$exit = Invoke-Installer $NewInstaller '--updated /S'
if ($exit -ne 0) { Fail "The new installer exited with code $exit." }
Show-State 'After the upgrade'

# 4. Check the result.
$after = @(Get-KChessEntries)
if ($after.Count -ne 1) {
  $listed = ($after | ForEach-Object { "$($_.Hive) $($_.DisplayVersion) in $($_.Folder)" }) -join '; '
  Fail "Expected one KChess entry in Apps & features, found $($after.Count): $listed."
}
$current = $after | Where-Object DisplayVersion -eq $NewVersion | Select-Object -First 1
if (-not $current) {
  Fail "No Apps & features entry has version $NewVersion."
} else {
  $newExe = Join-Path $current.Folder 'KChess.exe'
  $actual = Get-ExeVersion $newExe
  if ($actual -ne $NewVersion) { Fail "$newExe has version '$actual', expected $NewVersion." }
  if ($Mode -ne 'allusers' -and $current.Folder -ne $old.Folder) {
    Fail "The upgrade moved KChess from $($old.Folder) to $($current.Folder)."
  }
  $notes.Add("New version $NewVersion registered in $($current.Hive), installed in ``$($current.Folder)``.")
}

if (Test-Path $oldExe) {
  $leftover = Get-ExeVersion $oldExe
  if ($leftover -ne $NewVersion) {
    Fail "The previous installation is still present: $oldExe (version $leftover)."
  }
} else {
  $notes.Add("Previous installation in ``$($old.Folder)`` was removed.")
}

$shortcuts = @(Get-Shortcuts)
foreach ($kind in 'Start menu', 'Desktop') {
  if (-not ($shortcuts | Where-Object { $_.Place -like "$kind*" })) { Fail "No $kind shortcut to KChess." }
}
foreach ($shortcut in $shortcuts) {
  if ($shortcut.TargetVersion -ne $NewVersion) {
    $found = if ($shortcut.TargetVersion) { "version $($shortcut.TargetVersion)" } else { 'a missing file' }
    Fail "$($shortcut.Place) shortcut $($shortcut.Link) points at $($shortcut.Target), $found."
  }
}

if (-not (Test-Path $marker) -or (Get-Content -Raw $marker) -ne $markerText) {
  Fail "User data was not kept: $marker is missing or changed."
} else {
  $notes.Add('User data in `%APPDATA%\KChess` was kept.')
}

# 5. Report.
$summary = [System.Text.StringBuilder]::new()
$status = if ($failures.Count) { 'failed' } else { 'passed' }
[void]$summary.AppendLine("### Upgrade from $($old.DisplayVersion) ($Mode install): $status")
[void]$summary.AppendLine()
[void]$summary.AppendLine("Published release: $($old.Hive), ``$($old.Folder)``, installed with ``$oldArguments``.")
[void]$summary.AppendLine()
foreach ($failure in $failures) { [void]$summary.AppendLine("- :x: $failure") }
foreach ($note in $notes) { [void]$summary.AppendLine("- $note") }
if ($Mode -eq 'allusers' -and $failures.Count) {
  [void]$summary.AppendLine()
  [void]$summary.AppendLine('The one-click installer is per-user: it looks for an earlier installation only in HKCU, so an all-users installation in Program Files and its all-users shortcuts are left in place next to the new per-user copy.')
}
$summary.ToString() | Write-Host
if ($env:GITHUB_STEP_SUMMARY) { $summary.ToString() | Add-Content -Path $env:GITHUB_STEP_SUMMARY }
if ($failures.Count) { exit 1 }
