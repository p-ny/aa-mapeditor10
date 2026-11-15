# Writes the saved cell files (save_folder\overlay) into the client's game_pak, with an undo log per install.
param([ValidateSet('install','undo')][string]$Action='install')
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$cfg=@{}
foreach($line in Get-Content (Join-Path $root 'editor-config.txt')){
  $t=$line.Trim(); if(!$t -or $t.StartsWith('#')){continue}
  $k,$v=$t -split '=',2; if($v -ne $null){$cfg[$k.Trim()]=$v.Trim()}
}
$pak=Join-Path $cfg['client_folder'] 'game_pak'
if(!(Test-Path $pak)){throw "No game_pak in '$($cfg['client_folder'])'. Check client_folder in editor-config.txt"}
$save=$cfg['save_folder']; if(!$save){$save='saves'}
if(![IO.Path]::IsPathRooted($save)){$save=Join-Path $root $save}
$overlay=Join-Path $save 'overlay'
$undoDir=Join-Path $save 'installs'
New-Item $undoDir -ItemType Directory -Force | Out-Null
$tool=Join-Path $root 'pak-patch.exe'
if(Get-Process archeage,x2game -ErrorAction SilentlyContinue){throw 'Close the game client first.'}
if($Action -eq 'undo'){
  $logs=@(Get-ChildItem $undoDir -Filter '*.undo' | Sort-Object Name -Descending)
  if(!$logs){Write-Host 'Nothing to undo.'; exit 0}
  $last=$logs[0].BaseName -replace '-\d{3}$',''
  foreach($u in $logs | Where-Object {$_.BaseName -like "$last-*"}){
    & $tool undo $pak $u.FullName; if($LASTEXITCODE){throw "undo failed: $($u.Name)"}
    Move-Item $u.FullName ($u.FullName+'.done')
  }
  Write-Host "Undid install $last."
  exit 0
}
if(!(Test-Path $overlay)){Write-Host 'No saved edits yet (save with Ctrl+S in the editor).'; exit 0}
$files=@(Get-ChildItem $overlay -Recurse -File)
if(!$files){Write-Host 'No saved edits yet.'; exit 0}
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
$pairs=foreach($f in $files){
  $rel=$f.FullName.Substring($overlay.Length+1).Replace([char]92,'/')
  "game/$($rel.ToLower())=$($f.FullName)"
}
# one call per batch that fits the Windows command line; usually a single batch
$batches=@(); $cur=@(); $len=0
foreach($p in $pairs){ if($cur.Count -and $len+$p.Length -gt 24000){$batches+=,$cur; $cur=@(); $len=0}; $cur+=$p; $len+=$p.Length+3 }
if($cur.Count){$batches+=,$cur}
$n=0
foreach($b in $batches){
  $n++
  $undo=Join-Path $undoDir ('{0}-{1:000}.undo' -f $stamp,$n)
  & $tool apply $pak $undo @b
  if($LASTEXITCODE){throw "install failed. Run 'Undo Last Install.cmd' to roll back what was written."}
}
Write-Host "Installed $($files.Count) files into $pak (undo: Undo Last Install.cmd)."
