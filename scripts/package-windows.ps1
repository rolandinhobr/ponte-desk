param([string]$Output=(Join-Path $PSScriptRoot '..\dist\PonteDesk-0.5.0-preview-windows.zip'))
$ErrorActionPreference='Stop'
$project=Split-Path $PSScriptRoot
$stage=Join-Path ([IO.Path]::GetTempPath()) ('PonteDesk-package-'+[guid]::NewGuid().ToString('N'))
$files=@{
 'target\release\ponte-desk.exe'='dist\windows\ponte-desk.exe'
 'native\windows\app\build\PonteDesk.exe'='dist\windows\PonteDesk.exe'
 'native\windows\webcam\build\PonteDeskWebcam.dll'='dist\windows\webcam\PonteDeskWebcam.dll'
 'native\windows\storage\build\PonteDeskStorage.dll'='dist\windows\storage\PonteDeskStorage.dll'
}
try {
 New-Item -Type Directory $stage|Out-Null
 foreach($entry in $files.GetEnumerator()){
  $source=Join-Path $project $entry.Key
  if(!(Test-Path $source)){throw ('Build the missing component first: '+$entry.Key)}
  $target=Join-Path $stage $entry.Value
  New-Item -Type Directory -Force (Split-Path $target)|Out-Null
  Copy-Item $source $target
 }
 $support=@('scripts\install-windows.ps1','scripts\start-windows-standard.ps1','native\windows\webcam\install.ps1','native\windows\storage\install.ps1','README.md','LICENSE','NOTICE','docs\setup.md','docs\options.md','docs\storage.md','docs\validation.md','docs\camera-component.md')
 $support+=Get-ChildItem (Join-Path $project 'docs') -Filter '*.md' -File|ForEach-Object {'docs\'+$_.Name}
 $support+=Get-ChildItem (Join-Path $project 'assets') -File|ForEach-Object {'assets\'+$_.Name}
 $support+=@('native\windows\webcam\NOTICE.txt')
 $support+=Get-ChildItem (Join-Path $project 'examples') -File|ForEach-Object {'examples\'+$_.Name}
 $support+=Get-ChildItem (Join-Path $project 'native\windows\webcam') -Filter '*LICENSE*' -File|ForEach-Object {'native\windows\webcam\'+$_.Name}
 foreach($relative in $support){$target=Join-Path $stage $relative;New-Item -Type Directory -Force (Split-Path $target)|Out-Null;Copy-Item (Join-Path $project $relative) $target}
 New-Item -Type Directory -Force (Split-Path $Output)|Out-Null
 Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $Output -Force
 Get-FileHash $Output|Select-Object Hash,Path|ConvertTo-Json -Compress
}finally{if(Test-Path $stage){Remove-Item -LiteralPath $stage -Recurse -Force}}
