param([string]$DeveloperCommand='')
$ErrorActionPreference='Stop'
$storageRoot=$PSScriptRoot
$webcamRoot=Join-Path $storageRoot '..\webcam'
& (Join-Path $webcamRoot 'build.ps1') -DeveloperCommand $DeveloperCommand -PrepareOnly
$include=Join-Path $webcamRoot 'deps\microsoft.windows.cppwinrt.3.0.260818.1\include'
$out=Join-Path $storageRoot 'build';New-Item -Type Directory -Force $out|Out-Null
Push-Location $out
try {
 & cl.exe /nologo /LD /MT /std:c++20 /EHsc /O2 /W3 /DUNICODE /D_UNICODE "/I$include" (Join-Path $storageRoot 'Storage.cpp') /link /OUT:PonteDeskStorage.dll netapi32.lib mpr.lib shell32.lib ole32.lib oleaut32.lib advapi32.lib ws2_32.lib runtimeobject.lib uuid.lib
 if($LASTEXITCODE -ne 0){throw 'Storage component build failed'}
 Get-FileHash (Join-Path $out 'PonteDeskStorage.dll') -Algorithm SHA256|Select-Object Hash,Path|ConvertTo-Json -Compress
}finally{Pop-Location}
