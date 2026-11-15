param([string]$World='main_world',[string]$Position='13534,17409')
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$cfg=@{}
foreach($line in Get-Content (Join-Path $root 'editor-config.txt')){
  $t=$line.Trim()
  if(!$t -or $t.StartsWith('#')){continue}
  $k,$v=$t -split '=',2
  if($v -ne $null){$cfg[$k.Trim()]=$v.Trim()}
}
function Need($key){ if(!$cfg[$key]){throw "Set '$key' in editor-config.txt"}; $cfg[$key] }
$client=Need 'client_folder'
$pak=Join-Path $client 'game_pak'
if(!(Test-Path $pak)){throw "No game_pak in $client. Check client_folder in editor-config.txt"}
$db=Need 'database'
if(!(Test-Path $db)){throw "Database not found: $db. Check database in editor-config.txt"}
$save=$cfg['save_folder']; if(!$save){$save='saves'}
if(![IO.Path]::IsPathRooted($save)){$save=Join-Path $root $save}
foreach($d in 'overlay','spawner-overlay',"doodad-overlay\$World",'housing-overlay','level-doodad-overlay',"Worlds\$World",'logs'){
  New-Item (Join-Path $save $d) -ItemType Directory -Force | Out-Null
}
$env:AA_PAK=$pak
$env:AA_PAK_OVERLAY=Join-Path $save 'overlay'
$env:AA_PRIORITY='normal'
$env:AA_PRESENT='vsync'
$env:AA_LOG_FILE=Join-Path $save 'logs\editor.log'
$a=@('--tenx','--publish-target','none','--read-only-db','--db',$db,'--world',$World,'--size','1600x900',
  '--overlay',$env:AA_PAK_OVERLAY,'--spawn-dir',(Join-Path $save "Worlds\$World"),
  '--spawner-overlay',(Join-Path $save 'spawner-overlay'),'--doodad-overlay',(Join-Path $save "doodad-overlay\$World"),
  '--housing-overlay',(Join-Path $save 'housing-overlay'),'--ground-catalog',(Join-Path $root 'data/ground_catalog.json'),'--level-doodad-overlay',(Join-Path $save 'level-doodad-overlay'))
if($cfg['server_worlds']){$a+=@('--world-spawn-dir',(Join-Path $cfg['server_worlds'] $World))}
if($Position){$a+=@('--pos',$Position)}
Push-Location $save  # relative editor files (teleport bookmarks) go into the save folder
try { & (Join-Path $root 'aa-editor.exe') @a } finally { Pop-Location }
exit $LASTEXITCODE
