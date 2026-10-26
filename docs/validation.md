# Validação

A suíte cobre autenticação e proteção contra replay, rejeição de par incorreto, formatos limitados de vídeo, estado inválido, modo sem DDC, suspensão por agente e liberação de teclas/botões. Os testes de scroll do Mac inspecionam eventos Quartz em memória e não publicam entrada no desktop.

O smoke test cria configuração temporária, backends dummy, UDP/IPC de loopback e permanece no mesmo processo. Também verifica mudanças de opções, pausa manual, modo independente sem entrada DDC e falha segura em configuração inválida.

A configuração de monitor precisa de validação física no hardware de cada instalação. Testes dummy não certificam jogos, anticheats ou HID virtual. Compatibilidade com todos os aplicativos de chamada não é presumida.

Os testes locais de desenvolvimento anteriores validaram webcam USB física em 1080p nos dois sentidos, em consumidores Media Foundation e CMIO. A versão pública mantém testes sintéticos separados dos testes físicos e não distribui logs pessoais, imagens privadas, nomes de máquinas ou configurações de desenvolvimento.

Na preparação da versão 0.3, passaram 22 testes no Mac e 20 no Windows, além de formatação, Clippy, compilação nativa e smoke test do coordenador. O novo componente CMIO recebeu padrões sintéticos vermelho/azul em entrada 1080p e 720p, com saída 1920×1080 BGRA. O controle negativo rejeitou os 60 frames da imagem de espera como vídeo ao vivo; nenhum frame de câmera física foi aberto ou salvo nesses testes.
