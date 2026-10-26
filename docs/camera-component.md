# Componente de câmera do macOS

O nome de uso é **Ponte Desk Camera**. O Ponte Desk envia seus frames diretamente ao sink CMIO de um componente aberto e assinado, sem precisar manter outro controlador de câmera em execução.

A distribuição inclui o host original completo de **LiveLoop 1.3.4**, preservando assinatura, notarização e licença GPL-3.0. Ele é usado somente para solicitar a ativação ou remoção da extensão. Seu código correspondente acompanha `native/macos/camera-extension/vendor-source/`; commit `67dc46bc3dfb76cd88730bb6337966dbabc5e30b`.

- Origem: <https://github.com/adrbn/liveloop/releases/tag/v1.3.4>
- SHA256 do DMG: `91850c858e83804128f191d9607f56cb8e2d3ac658c2ad0e6dad3270589c6946`.
- Team ID do release verificado: `2TWQF4T93E`.
- Extensão: `com.adrbn.LiveLoop.Extension`.
- Device UID: `5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C`.

## Personalização

O provedor oferece propriedades CMIO de nome (`llcn`, CFString) e imagem de espera (`plch`, JPEG em Base64/CFString). O Ponte usa essas propriedades públicas para identificar o dispositivo e fornecer sua própria imagem de espera, sem alterar arquivos assinados.

A renomeação exige que nenhum aplicativo esteja consumindo a câmera e nenhum produtor esteja conectado. Feche o uso da câmera e tente a configuração novamente. O dispositivo é republicado; os IDs numéricos CMIO devem ser descobertos outra vez.

A saída declarada desse componente é **1920×1080 BGRA a 30 fps**. Captura/rede 720p continua disponível; o receptor a adapta para essa saída. No Windows o componente do projeto oferece 720p/1080p com NV12/RGB32.

## Instalação e limites

A assinatura original identifica o fornecedor nos detalhes técnicos e na aprovação do macOS; créditos, identificadores e avisos de licença são preservados. Não se promete apagar essa identidade do sistema. O nome do dispositivo escolhido nos aplicativos é personalizado para Ponte Desk.

O host original deve permanecer **fechado durante o uso**. Depois da ativação, o Ponte o encerra antes de preparar o dispositivo. Não autorize câmera física para o instalador: quem escolhe e captura a webcam externa é o Ponte Desk. A extensão apenas recebe buffers e gera imagem de espera.

Se o mesmo componente já existir por causa de outro aplicativo, o Ponte pede confirmação antes de alterar seu nome ou imagem: há um único UID global, não uma segunda câmera independente.

Não habilite modo de desenvolvedor de extensões, não desative SIP/Gatekeeper e não use plugins legados para contornar a aprovação. O pacote distribuído pelo projeto de origem é assinado e notarizado; o instalador precisa somente da aprovação normal da extensão.
