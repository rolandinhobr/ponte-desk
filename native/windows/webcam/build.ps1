param([string]$SourceRoot=$PSScriptRoot,[string]$DeveloperCommand='', [switch]$PrepareOnly)
$ErrorActionPreference='Stop'
if($env:VSCMD_ARG_TGT_ARCH -ne 'x64' -or !(Get-Command cl.exe -ErrorAction SilentlyContinue)){
 if(!$DeveloperCommand){
  $vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if(Test-Path $vswhere){
   $installation=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1
   if($installation){$DeveloperCommand=Join-Path $installation 'Common7\Tools\VsDevCmd.bat'}
  }
  if(!$DeveloperCommand -and $env:VSINSTALLDIR){$DeveloperCommand=Join-Path $env:VSINSTALLDIR 'Common7\Tools\VsDevCmd.bat'}
  if(!$DeveloperCommand -and (Test-Path 'C:\BuildTools\PonteDesk\Common7\Tools\VsDevCmd.bat')){$DeveloperCommand='C:\BuildTools\PonteDesk\Common7\Tools\VsDevCmd.bat'}
 }
 if(!$DeveloperCommand -or !(Test-Path $DeveloperCommand)){throw 'Install MSVC x64 Build Tools and Windows 11 SDK 26100, or pass -DeveloperCommand with the path to VsDevCmd.bat'}
 $environment=& cmd.exe /d /s /c "`"$DeveloperCommand`" -arch=x64 -host_arch=x64 >nul && set"
 if($LASTEXITCODE -ne 0){throw 'MSVC developer environment initialization failed'}
 foreach($line in $environment){if($line -match '^([^=]+)=(.*)$'){[Environment]::SetEnvironmentVariable($matches[1],$matches[2],'Process')}}
}
$deps=Join-Path $SourceRoot 'deps';New-Item -Type Directory -Force $deps|Out-Null
foreach($package in @(@('microsoft.windows.cppwinrt','3.0.260818.1'),@('microsoft.windows.implementationlibrary','1.0.260126.7'))){
 $name=$package[0];$version=$package[1];$dest=Join-Path $deps "$name.$version"
 if(!(Test-Path $dest)){
  $zip=Join-Path $deps "$name.$version.zip"
  Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/$name/$version/$name.$version.nupkg" -OutFile $zip
  $expected=@{
   'microsoft.windows.cppwinrt'='A8993608AE9263A8288E1A1D9EE2684A5632F4ACC59E1E89DDD86B3B889A7367'
   'microsoft.windows.implementationlibrary'='7D67BC71DD0EDC342DB44EA0334AC9B6A1776514F81694B0B5B3C7AC9D608E42'
  }[$name]
  if((Get-FileHash $zip -Algorithm SHA256).Hash -ne $expected){throw "Pinned dependency hash mismatch: $name"}
  Expand-Archive $zip -DestinationPath $dest
  Get-FileHash $zip -Algorithm SHA256|Select-Object Hash,Path|ConvertTo-Json -Compress
 }
}
if($PrepareOnly){return}
$out=Join-Path $SourceRoot 'build';New-Item -Type Directory -Force $out|Out-Null
$src=Join-Path $SourceRoot 'source'
$wil=Join-Path $deps 'microsoft.windows.implementationlibrary.1.0.260126.7\include'
$winrt=Join-Path $deps 'microsoft.windows.cppwinrt.3.0.260818.1\include'
Set-Location $out
$units=@('Activator','dllmain','EnumNames','FrameGenerator','MediaSource','MediaStream','MFTools','Tools','WinTrace','Controller','Capture')|ForEach-Object {Join-Path $src "$_.cpp"}
& cl.exe /nologo /LD /MT /std:c++20 /EHsc /O2 /W3 /DUNICODE /D_UNICODE /DNDEBUG /DWINVER=0x0A00 /D_WIN32_WINNT=0x0A00 "/I$wil" "/I$winrt" "/I$src" @units /link /OUT:PonteDeskWebcam.dll "/DEF:$src\VCamSampleSource.def" mf.lib mfplat.lib mfuuid.lib mfsensorgroup.lib ole32.lib oleaut32.lib advapi32.lib propsys.lib windowscodecs.lib d2d1.lib dwrite.lib d3d11.lib dxgi.lib shlwapi.lib runtimeobject.lib strmiids.lib user32.lib mfreadwrite.lib setupapi.lib cfgmgr32.lib
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
Get-FileHash (Join-Path $out 'PonteDeskWebcam.dll') -Algorithm SHA256|Select-Object Hash,Path|ConvertTo-Json -Compress
