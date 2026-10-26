# Validação

A suíte cobre autenticação e proteção contra replay, rejeição de par incorreto, formatos limitados de vídeo, estado inválido, modo sem DDC, suspensão por agente e liberação de teclas/botões. Os testes de scroll do Mac inspecionam eventos Quartz em memória e não publicam entrada no desktop.

O smoke test cria configuração temporária, backends dummy, UDP/IPC de loopback e permanece no mesmo processo. Também verifica mudanças de opções, pausa manual, modo independente sem entrada DDC e falha segura em configuração inválida.

A configuração de monitor precisa de validação física no hardware de cada instalação. Testes dummy não certificam jogos, anticheats ou HID virtual. Compatibilidade com todos os aplicativos de chamada não é presumida.

Os testes locais de desenvolvimento anteriores validaram webcam USB física em 1080p nos dois sentidos, em consumidores Media Foundation e CMIO. A versão pública mantém testes sintéticos separados dos testes físicos e não distribui logs pessoais, imagens privadas, nomes de máquinas ou configurações de desenvolvimento.

Na preparação da versão 0.3, passaram 22 testes no Mac e 20 no Windows, além de formatação, Clippy, compilação nativa e smoke test do coordenador. O novo componente CMIO recebeu padrões sintéticos vermelho/azul em entrada 1080p e 720p, com saída 1920×1080 BGRA. O controle negativo rejeitou os 60 frames da imagem de espera como vídeo ao vivo; nenhum frame de câmera física foi aberto ou salvo nesses testes.

Após a instalação local da 0.3, uma webcam USB externa no Windows entregou 60 amostras ao consumidor CMIO do Mac em 1920×1080; 46 amostras de espera foram rejeitadas. A captura foi encerrada após o teste, sem salvar imagens. O recebimento no Windows também precisa ser conferido com o consumidor de destino de cada instalação.

O dispositivo Windows com o nome Ponte Desk Camera também passou no consumidor Media Foundation: 64 quadros, padrões vermelho e azul, saída NV12 1920×1080, produtor sem elevação e zero dispositivos temporários restantes após parar. Os probes pareados completaram cinco trocas Ping/Pong em cada direção, sem enviar entrada.

Na preparação da 0.5, passaram 25 testes do motor no Mac e 23 no Windows, Clippy e os testes de coordenação e de ciclo de vida do armazenamento. A interface Windows compilou com avisos tratados como erros e passou em 15 verificações de argumentos pela API Windows, identificação de sessões, JSON e ícone. A fixture visual usa dados vazios de prévia e não acessa câmera ou cartão reais.

Na instalação de desenvolvimento, o comando do painel do Mac pausou os dois lados. Também passaram pausa/retomada e leitura/atualização sem mudança das opções a partir do Windows. Após a renovação de permissões, os snapshots recentes confirmaram captura e emulação ativas nos dois computadores. Uma instância de interface e uma de motor foram observadas no Windows, com interface sem elevação.

O teste físico de SD Mac → Windows leu um arquivo existente, conferiu o SHA256 na origem e no destino, negou uma abertura para escrita sem gravar/truncar bytes e recusou desmontar enquanto havia um handle de leitura aberto. Depois de fechar o handle, a desmontagem e a remoção do compartilhamento foram confirmadas. O teste físico Windows → Mac continua pendente; a implementação desse sentido não equivale a uma validação física concluída.

A 0.5.1 adiciona regressão do delegado real do Mac: fechar uma NSWindow com um comando bloqueado de 30 segundos e estado de SD desconhecido encerra em menos de 1,3 segundo, cancela os processos próprios e preserva a indicação de limpeza não confirmada. A fixture não usa teclado, mouse, câmera ou SMB reais. No Windows, verificações de cancelamento cobrem um comando bloqueado e outro na fila, e uma fixture de Form.Close exercita a saída enquanto a interface está ocupada. O botão vermelho foi também conferido na instalação real do Mac com o outro computador indisponível; GUI e motor encerraram.

O erro de SD observado após a atualização era acompanhado de um auxiliar registrado na versão anterior e falha EX_CONFIG ao iniciar. A sessão antiga foi liberada somente após confirmar ausência de compartilhamento no SMB do sistema. Mesmo com o registro atualizado e autorização renovada, os logs mostraram Launch Constraint Violation para o auxiliar ad hoc, com restrição de cdhash vazia. A instalação do Mac passou a incluir um pacote nativo que instala o auxiliar como root fora do bundle; o cliente verifica seu SHA256 e fixa o hash de código na conexão XPC. A nova validação física de montagem depende da instalação autorizada desse componente e não se presume concluída pela compilação ou pelo registro.
