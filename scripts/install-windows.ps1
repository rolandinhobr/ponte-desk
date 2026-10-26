param([string]$Executable=(Join-Path $PSScriptRoot '..\dist\windows\ponte-desk.exe'))
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
$root=Join-Path $env:LOCALAPPDATA 'PonteDesk'
$program=Join-Path $env:LOCALAPPDATA 'Programs\PonteDesk'
New-Item -ItemType Directory -Force $root,$program | Out-Null
if (!(Test-Path (Join-Path $root 'observer.json')) -or !(Test-Path (Join-Path $root 'config.toml'))) {
 throw "Configure observer.json e config.toml em $root conforme README antes da instalação."
}
Copy-Item $Executable (Join-Path $program 'ponte-desk.exe') -Force
$exe=Join-Path $program 'ponte-desk.exe'
$shell=New-Object -ComObject WScript.Shell
$link=$shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Startup')) 'Ponte Desk.lnk'))
$link.TargetPath=$exe;$link.Arguments='--capture-backend windows --emulation-backend windows daemon';$link.WorkingDirectory=$program;$link.Save()
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

$daemonPid=& (Join-Path $PSScriptRoot 'start-windows-standard.ps1') -Exe $exe
if([PonteDeskStandardBootstrap]::Elevated([uint32]$daemonPid)) {
 Stop-Process -Id $daemonPid -Force -ErrorAction SilentlyContinue
 throw 'O aplicativo não pode permanecer executando como administrador.'
}
Write-Output "Instalado em $program; inicialização automática configurada."
