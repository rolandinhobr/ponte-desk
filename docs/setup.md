# Instalação e pareamento

O pareamento ainda é manual. Configure os dois computadores antes de habilitar os recursos do aplicativo.

1. No Mac, use `~/.config/ponte-desk`. No Windows, `%LOCALAPPDATA%\PonteDesk`. `PONTE_DESK_HOME` permite outro diretório.
2. Copie `examples/config.mac.toml` ou `examples/config.windows.toml` para `config.toml` no diretório correspondente. Substitua o endereço reservado pelo IP real do outro computador e escolha a posição relativa. As posições nos dois lados são opostas.
3. Abra o aplicativo no Mac; no Windows execute `ponte-desk.exe --capture-backend windows --emulation-backend windows daemon`. A identidade `ponte-desk.pem` é criada na primeira inicialização. Para imprimir somente o fingerprint público, use `python3 scripts/certificate-fingerprint.py CAMINHO_DO_PEM`. Cadastre o fingerprint do outro computador em `authorized_fingerprints`. Reinicie o serviço depois do pareamento.
4. Copie `examples/observer.example.json` para `observer.json`. Defina `side` como `mac` ou `pc` e `peers` com o IP real do outro computador. Gere uma chave aleatória de 32 bytes, por exemplo com `openssl rand -hex 32`, e coloque o mesmo valor `key` nas duas configurações. A chave não é o fingerprint e não deve ser publicada.
5. Copie `examples/preferences.example.json` para `preferences.json` no Mac ou deixe o aplicativo criar suas preferências padrão. As opções gerais são controladas no painel do Mac e transmitidas ao par autenticado.
6. No firewall, libere **UDP 4242** (entrada), **UDP 4243** (coordenação) e **TCP 4244** (controle/webcam) apenas entre os IPs pareados. Não exponha essas portas à internet.
7. Depois de concluir todos os arquivos e o pareamento, encerre e reabra o Ponte Desk no Mac e reinicie o serviço Windows. O coordenador carrega a configuração de pareamento na inicialização.

O exemplo usa endereços reservados para documentação; eles não localizam seu computador. Nenhuma configuração real, certificado ou chave faz parte do projeto.

## Permissões do Mac

Ative **Ponte Desk** em Controle do Dispositivo e Acesso a Dados/Acessibilidade e em Monitoração de Entrada. São permissões distintas. Autorize Câmera somente quando iniciar o envio da webcam USB. Uma atualização assinada localmente pode exigir renovar as permissões existentes e reabrir o aplicativo.

Para receber vídeo, escolha **Configurar câmera…** no painel. O componente incluído pede sua aprovação em Extensões de Câmera. Depois da ativação, o instalador é encerrado e o dispositivo recebe o nome **Ponte Desk Camera**. Não conceda acesso à câmera física ao instalador; quem captura é o Ponte Desk.

## Serviço Windows

Copie `target/release/ponte-desk.exe` para `dist/windows/ponte-desk.exe` e a DLL criada em `native/windows/webcam/build/` para `dist/windows/webcam/`. Com o pareamento pronto, execute o instalador principal. O registro inicial da DLL exige administrador; o processo residente usa o token do usuário interativo, sem elevação.

Certificados e configurações devem ser preservados ao atualizar. Não compartilhe o PEM nem copie a chave privada para outra pessoa.
