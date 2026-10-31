param([string]$Executable=(Join-Path $PSScriptRoot '..\dist\windows\ponte-desk.exe'),[string]$Application=(Join-Path $PSScriptRoot '..\dist\windows\PonteDesk.exe'))
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
$Application=(Resolve-Path $Application).Path
$root=Join-Path $env:LOCALAPPDATA 'PonteDesk'
$program=Join-Path $env:LOCALAPPDATA 'Programs\PonteDesk'
New-Item -ItemType Directory -Force $root,$program | Out-Null
if (!(Test-Path (Join-Path $root 'observer.json')) -or !(Test-Path (Join-Path $root 'config.toml'))) {
 throw "Configure observer.json e config.toml em $root conforme README antes da instalação."
}
# Never replace a running application or interrupt an active media session.
$running=Get-Process PonteDesk -ErrorAction SilentlyContinue|Where-Object {$_.Path -eq (Join-Path $program 'PonteDesk.exe')}
if($running){throw 'Encerre Ponte Desk pelo menu da bandeja antes de atualizar.'}
$cameraPath=Join-Path $root 'webcam-status.json'
if(Test-Path $cameraPath){$camera=Get-Content $cameraPath -Raw|ConvertFrom-Json;if($camera.mode -eq 'sending' -or $camera.mode -eq 'receiving'){throw 'Pare a webcam antes de atualizar.'}}
foreach($name in @('storage-host.json','storage-mount.json')){if(Test-Path (Join-Path $root $name)){$state=Get-Content (Join-Path $root $name)-Raw|ConvertFrom-Json;if($state.active -or $state.phase -in @('preparing','mounting','unmounting')){throw 'Desmonte e pare o cartão antes de atualizar.'}}}
$pause=Join-Path $root 'manual-pause';$ownedPause=!(Test-Path $pause);$pauseToken='install-'+[guid]::NewGuid().ToString('N')
try {
 if($ownedPause){Set-Content -NoNewline -LiteralPath $pause -Value $pauseToken}
 $engine=Join-Path $root 'engine-status.json'
 if(Test-Path $engine){
  $previous=Get-Content $engine -Raw|ConvertFrom-Json
  $old=Get-Process -Id $previous.pid -ErrorAction SilentlyContinue
  if($old -and $old.Path -eq (Join-Path $program 'ponte-desk.exe')){
   $safe=$false
   for($i=0;$i -lt 50;$i++){$state=Get-Content $engine -Raw|ConvertFrom-Json;if(!$state.capture_installed -and !$state.emulation_enabled -and ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()-$state.timestamp_ms) -lt 6000){$safe=$true;break};Start-Sleep -Milliseconds 200}
   if(!$safe){throw 'O controle nativo não confirmou a pausa. Atualização cancelada.'}
   Stop-Process -Id $old.Id;if(!$old.WaitForExit(8000)){throw 'O motor anterior ainda está em execução.'}
  }
 }
 Copy-Item $Executable (Join-Path $program 'ponte-desk.exe') -Force
 Copy-Item $Application (Join-Path $program 'PonteDesk.exe') -Force
} finally {if($ownedPause -and (Test-Path $pause) -and (Get-Content $pause -Raw) -eq $pauseToken){Remove-Item -LiteralPath $pause}}
$applicationExe=Join-Path $program 'PonteDesk.exe'
$exe=Join-Path $program 'ponte-desk.exe'
$shell=New-Object -ComObject WScript.Shell
$link=$shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Startup')) 'Ponte Desk.lnk'))
$link.TargetPath=$applicationExe;$link.Arguments='--background';$link.WorkingDirectory=$program;$link.IconLocation=$applicationExe;$link.Save()
$menu=$shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Ponte Desk.lnk'));$menu.TargetPath=$applicationExe;$menu.WorkingDirectory=$program;$menu.IconLocation=$applicationExe;$menu.Save()
$accountSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
& icacls $root /inheritance:r /grant:r "*${accountSid}:(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' | Out-Null
if($LASTEXITCODE -ne 0){throw 'Não foi possível proteger o diretório'}
& icacls (Join-Path $root '*') /reset /T /C | Out-Null
if($LASTEXITCODE -ne 0){throw 'Não foi possível aplicar as permissões aos arquivos'}
# Execute as administrator only for the narrowly scoped inbound rule.
$cfg=Get-Content (Join-Path $root 'observer.json') -Raw | ConvertFrom-Json
if(!(Get-NetFirewallRule -DisplayName 'Ponte Desk - rede pareada' -ErrorAction SilentlyContinue)) {
 New-NetFirewallRule -DisplayName 'Ponte Desk - rede pareada' -Direction Inbound -Action Allow -Program $exe -Protocol UDP -LocalPort 4242,4243 -RemoteAddress $cfg.peers -Profile Any | Out-Null
}

$cameraDll=Join-Path $PSScriptRoot '..\dist\windows\webcam\PonteDeskWebcam.dll'
if(Test-Path $cameraDll){& (Join-Path $PSScriptRoot '..\native\windows\webcam\install.ps1') -Dll $cameraDll}
if(!(Get-NetFirewallRule -DisplayName 'Ponte Desk Webcam - rede pareada' -ErrorAction SilentlyContinue)) {
 New-NetFirewallRule -DisplayName 'Ponte Desk Webcam - rede pareada' -Direction Inbound -Action Allow -Program $exe -Protocol TCP -LocalPort 4244 -RemoteAddress $cfg.peers -Profile Any | Out-Null
}

$storageDll=Join-Path $PSScriptRoot '..\dist\windows\storage\PonteDeskStorage.dll'
if(Test-Path $storageDll){& (Join-Path $PSScriptRoot '..\native\windows\storage\install.ps1') -Dll $storageDll}

$daemonPid=& (Join-Path $PSScriptRoot 'start-windows-standard.ps1') -Exe $applicationExe -Arguments '--background'
if([PonteDeskStandardBootstrap]::Elevated([uint32]$daemonPid)) {
 Stop-Process -Id $daemonPid -Force -ErrorAction SilentlyContinue
 throw 'O aplicativo não pode permanecer executando como administrador.'
}
Write-Output "Instalado em $program; inicialização automática configurada."
