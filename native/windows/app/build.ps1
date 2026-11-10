param([string]$Output=(Join-Path $PSScriptRoot 'build'))
$ErrorActionPreference='Stop'
$csc=Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if(!(Test-Path $csc)){throw 'Windows .NET Framework 4.8 compiler was not found'}
New-Item -Type Directory -Force $Output|Out-Null
$sources=Get-ChildItem $PSScriptRoot -Filter '*.cs'|ForEach-Object {$_.FullName}
$exe=Join-Path $Output 'PonteDesk.exe'
& $csc /nologo /target:winexe /platform:x64 /optimize+ /warnaserror+ /utf8output /codepage:65001 "/win32manifest:$PSScriptRoot\PonteDesk.manifest" "/out:$exe" /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll @sources
if($LASTEXITCODE -ne 0){throw 'Windows interface compilation failed'}
$worker=Join-Path $PSScriptRoot '..\..\..\tests\WindowsExitWorker.cs'
& $csc /nologo /target:exe /platform:x64 /optimize+ /warnaserror+ "/out:$Output\PonteDesk.TestWorker.exe" $worker
if($LASTEXITCODE -ne 0){throw 'Exit test fixture compilation failed'}
$test=Start-Process $exe -ArgumentList '--self-test' -Wait -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $Output 'self-test.txt') -RedirectStandardError (Join-Path $Output 'self-test-error.txt')
Get-Content (Join-Path $Output 'self-test.txt')
if($test.ExitCode -ne 0){Get-Content (Join-Path $Output 'self-test-error.txt');throw 'Windows interface checks failed'}
$uiTest=Start-Process $exe -ArgumentList '--ui-exit-test' -Wait -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $Output 'ui-exit-test.txt') -RedirectStandardError (Join-Path $Output 'ui-exit-test-error.txt')
Get-Content (Join-Path $Output 'ui-exit-test.txt')
if($uiTest.ExitCode -ne 0){Get-Content (Join-Path $Output 'ui-exit-test-error.txt');throw 'GUI close deadline failed'}
$icon=Join-Path $Output 'PonteDesk.ico'
$export=Start-Process $exe -ArgumentList @('--export-icon',('"'+$icon+'"')) -Wait -PassThru
if($export.ExitCode -ne 0){throw 'Application icon export failed'}
& $csc /nologo /target:winexe /platform:x64 /optimize+ /warnaserror+ /utf8output /codepage:65001 "/win32manifest:$PSScriptRoot\PonteDesk.manifest" "/win32icon:$icon" "/out:$exe" /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll @sources
if($LASTEXITCODE -ne 0){throw 'Windows application packaging failed'}
Get-FileHash $exe|Select-Object Hash,Path|ConvertTo-Json -Compress
