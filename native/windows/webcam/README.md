# Componente nativo do Windows

DLL de captura USB e câmera virtual Media Foundation usada pelo próprio Ponte Desk. Requer Windows 11 e não precisa de outro aplicativo residente. O serviço principal e os produtores executam como usuário comum; o registro inicial da DLL em Program Files exige administrador.

## Compilar

Instale MSVC x64 Build Tools e Windows 11 SDK 26100. A partir da raiz do projeto:

```powershell
./native/windows/webcam/build.ps1
```

O script usa um terminal MSVC x64 já preparado ou encontra Visual Studio/Build Tools com `vswhere`. Para um local personalizado, passe `-DeveloperCommand 'C:\caminho\Common7\Tools\VsDevCmd.bat'`. WIL e C++/WinRT são baixados do NuGet em versões e SHA256 fixados. As licenças MIT acompanham o componente.

A saída fica em `native/windows/webcam/build/PonteDeskWebcam.dll`. Copie para `dist/windows/webcam/PonteDeskWebcam.dll` ao preparar a distribuição. O pacote de desenvolvimento também inclui uma cópia verificada em `native/windows/webcam/dist/`.

## Instalar e usar

O instalador principal `scripts/install-windows.ps1` registra o componente e inicia o serviço sem elevação. As configurações de pareamento devem existir antes, conforme o README principal. `install.ps1 -Dll CAMINHO` registra apenas a DLL, sem ligar câmera.

Somente o ID exato de uma webcam USB externa presente pode iniciar captura. O formato nativo 720p/1080p é selecionado e confirmado antes de configurar a conversão do SourceReader; isso evita anunciar Full HD enquanto a origem permanece em 640×480. O receptor valida JPEGs de até 2 MiB e adapta 720p/1080p ao tipo escolhido pelo consumidor: NV12 ou RGB32.

Os quadros passam por até dois slots de memória anônima de 2 MiB, com ACL para o usuário instalado, SYSTEM e LocalService. O slot Global atende FrameServer; o Local atende consumidores Media Foundation comuns sem privilégio de criar memória Global. Não há arquivos de imagem, fila crescente, serviço Python residente ou fallback de câmera integrada. Ao parar, o dispositivo temporário é removido e as amostras antigas expiram para preto.

As fontes derivadas de VCamSample preservam a licença MIT e os créditos em `NOTICE.txt` e `LICENSE-VCamSample.txt`; WIL e C++/WinRT têm arquivos de licença próprios. As evidências reais e sintéticas estão em `VALIDATION.json` e na pasta `validation/` do projeto principal; matrizes sintéticas não equivalem a testar todos os aplicativos de chamada.
