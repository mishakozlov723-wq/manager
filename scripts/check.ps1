# The build machine's check. The manager installs itself through its window (it walks through the screens by
# itself: RM_SETUP_AUTO), installs and removes the real Retocker Voice and Retocker Snap, updates itself from a
# local test manifest, then is removed through its window, and once more without windows.
# Results: check.txt; screenshots: shots/*.png (both end up in the ci-shots branch). A failed line fails the build.
$ErrorActionPreference = 'Continue'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force shots | Out-Null
$script:fails = 0
function L($s) { $s | Tee-Object -FilePath check.txt -Append }
function Check($name, $ok) { if ($ok) { L "ok    $name" } else { L "FAIL  $name"; $script:fails++ } }
function Shot($n) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.X, $b.Y, 0, 0, $b.Size)
  $bmp.Save("shots/$n.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}
function Running { @(Get-Process rt-manager -ErrorAction SilentlyContinue).Count }
function Reg($key) { Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$key" -ErrorAction SilentlyContinue }

$exe = (Resolve-Path 'src-tauri/target/release/rt-manager.exe').Path
$tmp = $env:RUNNER_TEMP
$setup = Join-Path $tmp 'Retocker-Manager-Setup.exe'
Copy-Item $exe $setup -Force
# like a file from the browser: the «downloaded from the internet» mark (the installed copy must not keep it)
Set-Content -Path $setup -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3"
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Retocker Manager'
$app = Join-Path $dir 'rt-manager.exe'
$menu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Retocker Manager.lnk'
$desk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Retocker Manager.lnk'
$data = Join-Path $env:LOCALAPPDATA 'com.retocker.manager'
$logf = Join-Path $data 'logs\rt-manager.log'
L "build: $exe ($((Get-Item $exe).Length) bytes), version $env:VER"

# 1. the setup through its window, screenshots on the way
$env:RM_SETUP_AUTO = '1'
$p = Start-Process $setup -PassThru
$i = 0
while (-not $p.HasExited -and $i -lt 14) { Start-Sleep 2; $i++; Shot ('setup-{0:d2}' -f $i) }
$p.WaitForExit(20000) | Out-Null
Remove-Item Env:RM_SETUP_AUTO
Check 'setup window: walked through and closed' $p.HasExited
$k = Reg 'RetockerManager'
Check "setup: program in $dir" (Test-Path $app)
Check "setup: version in Windows is $env:VER (got $($k.DisplayVersion))" ($k.DisplayVersion -eq $env:VER)
Check 'setup: Start menu and desktop shortcuts' ((Test-Path $menu) -and (Test-Path $desk))
Check 'setup: no internet mark on the installed exe' (-not (Get-Item $app -Stream Zone.Identifier -ErrorAction SilentlyContinue))

# 2. the window of the manager
$env:RM_DEBUG = '1'
Start-Process $app
Start-Sleep 14
Shot 'app-01-start'
Check 'app: running after start' ((Running) -eq 1)

# 3. real programs, without a window: rt-manager --do <what> --id <program> --out <file>
function Mgr($what, $id, $more) {
  $out = Join-Path $tmp 'do.json'
  Remove-Item $out -ErrorAction SilentlyContinue
  $a = @('--do', $what, '--out', "`"$out`"")
  if ($id) { $a += @('--id', $id) }
  if ($more) { $a += $more }
  $q = Start-Process $exe -ArgumentList $a -PassThru
  if (-not $q.WaitForExit(300000)) { $q.Kill() }
  $raw = if (Test-Path $out) { Get-Content $out -Raw -Encoding utf8 } else { '{"ok":false,"error":"no answer"}' }
  L "  --do $what $id -> $raw"
  $raw | ConvertFrom-Json
}
$ver = @{}
foreach ($id in 'voice', 'snap', 'zapret', 'translate') {
  $r = Mgr 'remote' $id
  $ver[$id] = $r.result.version
  Check "remote: $id is at version '$($ver[$id])'" ($r.ok -and $ver[$id])
}
foreach ($pr in @(@('voice', 'RetockerVoice', 'retocker-voice'), @('snap', 'RetockerSnap', 'retocker-snap'))) {
  $id = $pr[0]
  $r = Mgr 'install' $id
  Check "install ${id}: version $($r.result.version)" ($r.ok -and $r.result.installed -and $r.result.version -eq $ver[$id])
  $loc = (Reg $pr[1]).InstallLocation
  Check "install ${id}: exe in '$loc'" ($loc -and (Test-Path (Join-Path $loc "$($pr[2]).exe")))
  # their installers close every running retocker*.exe: the manager must not be one of them
  Check "install ${id}: the manager is still running" ((Running) -eq 1)
}
# a fresh start of the window shows both as installed
Get-Process rt-manager -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Process $app
Start-Sleep 12
Shot 'app-02-two-installed'
$st = Mgr 'status'
L "  firefox on this machine: $(($st.result | Where-Object id -eq 'translate').firefox)"
foreach ($pr in @(@('voice', 'RetockerVoice'), @('snap', 'RetockerSnap'))) {
  $id = $pr[0]
  $r = Mgr 'uninstall' $id @('--wipe')
  Check "uninstall ${id}" ($r.ok -and -not $r.result.installed -and -not (Reg $pr[1]))
}
Get-Process retocker-voice, retocker-snap -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Process rt-manager -ErrorAction SilentlyContinue | Stop-Process -Force

# 4. the manager updates itself: a manifest (version 9.9.9) that points at a copy of this build
$pay = Join-Path $tmp 'payload.exe'
Copy-Item $exe $pay -Force
$man = Join-Path $tmp 'latest-test.json'
$env:NOTES = 'test'
node scripts/manifest.mjs $pay 9.9.9 $pay $man
Remove-Item Env:NOTES
$env:RM_UPDATE_URL = $man
Start-Process $app
Start-Sleep 10
Start-Process $app -ArgumentList '--test-update'
Start-Sleep 16
Shot 'update-after'
Check "update: version in Windows is 9.9.9 (got $((Reg 'RetockerManager').DisplayVersion))" ((Reg 'RetockerManager').DisplayVersion -eq '9.9.9')
Check 'update: the new copy is running' ((Running) -eq 1)
Check 'update: the old copy is cleaned up' (-not (Test-Path (Join-Path $dir 'rt-manager.old.exe')))
Get-Process rt-manager -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item Env:RM_UPDATE_URL
Remove-Item Env:RM_DEBUG
Copy-Item $logf shots/app-log.txt -ErrorAction SilentlyContinue
Start-Sleep 1

# 5. uninstall through its window
$env:RM_SETUP_AUTO = '1'
$u = Start-Process $app -ArgumentList '--uninstall' -PassThru
$i = 0
while (-not $u.HasExited -and $i -lt 10) { Start-Sleep 2; $i++; Shot ('uninstall-{0:d2}' -f $i) }
$u.WaitForExit(20000) | Out-Null
Remove-Item Env:RM_SETUP_AUTO
Start-Sleep 6
Check 'uninstall window: walked through and closed' $u.HasExited
Check 'uninstall: exe, folder and the entry in Windows are gone' (-not (Test-Path $app) -and -not (Test-Path $dir) -and -not (Reg 'RetockerManager'))
Check 'uninstall: shortcuts are gone' (-not (Test-Path $menu) -and -not (Test-Path $desk))
Check 'uninstall: its data is gone' (-not (Test-Path $data))

# 6. once more without windows
$p = Start-Process $setup -ArgumentList '--silent', '--no-desktop' -PassThru -Wait
Check "quiet install: code $($p.ExitCode), no desktop shortcut" ($p.ExitCode -eq 0 -and (Test-Path $app) -and (Test-Path $menu) -and -not (Test-Path $desk))
$p = Start-Process $app -ArgumentList '--uninstall', '--silent' -PassThru -Wait
Start-Sleep 6
Check "quiet uninstall: code $($p.ExitCode)" ($p.ExitCode -eq 0 -and -not (Test-Path $app) -and -not (Test-Path $dir) -and -not (Reg 'RetockerManager'))

L "failed checks: $script:fails"
if ($script:fails -gt 0) { exit 1 }
