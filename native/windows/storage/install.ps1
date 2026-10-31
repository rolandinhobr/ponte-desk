param([string]$Dll=(Join-Path $PSScriptRoot 'build\PonteDeskStorage.dll'))
$ErrorActionPreference='Stop'
$target=Join-Path $env:ProgramFiles 'PonteDesk\Storage'
New-Item -Type Directory -Force $target|Out-Null
& icacls $target /inheritance:e /grant '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX'|Out-Null
if($LASTEXITCODE -ne 0){throw 'Cannot protect storage component'}
Copy-Item $Dll (Join-Path $target 'PonteDeskStorage.dll') -Force
Write-Output 'Storage component installed. No card shared and no SMB service changed.'
