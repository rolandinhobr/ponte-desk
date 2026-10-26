param([string]$Dll=(Join-Path $PSScriptRoot 'dist\PonteDeskWebcam.dll'))
$ErrorActionPreference='Stop'
$target=Join-Path $env:ProgramFiles 'PonteDesk\Webcam'
New-Item -Type Directory -Force $target|Out-Null
Copy-Item $Dll (Join-Path $target 'PonteDeskWebcam.dll') -Force
# Protected Program Files code, readable by FrameServer LocalService/System.
& icacls $target /inheritance:e /grant '*S-1-5-19:(OI)(CI)RX' '*S-1-5-18:(OI)(CI)RX'|Out-Null
if($LASTEXITCODE -ne 0){throw 'Cannot assign FrameServer DLL read access'}
$p=Start-Process "$env:WINDIR\System32\regsvr32.exe" -ArgumentList '/s',('"'+(Join-Path $target 'PonteDeskWebcam.dll')+'"') -Wait -PassThru
if($p.ExitCode -ne 0){throw "Virtual camera source registration failed: $($p.ExitCode)"}
@{registered=$true;path=(Join-Path $target 'PonteDeskWebcam.dll');camera_started=$false}|ConvertTo-Json -Compress
