# Arquitetura

O executável Rust reúne transporte KVM, captura/emulação, coordenação autenticada e controle de webcam. A biblioteca Swift/AppKit pertence ao mesmo bundle do Mac e fornece interface, metadados de telas, captura USB e produtor CMIO. O Windows possui captura USB e câmera virtual Media Foundation na DLL nativa do projeto.

São três recursos independentes. A política do modo comum não consulta a entrada do monitor. No modo de monitor compartilhado, o Windows observa somente o monitor selecionado em um worker de DDC; a UI do Mac só reorganiza telas com identificação válida e amostra recente.

O controle de entrada usa o protocolo DTLS herdado do upstream. O estado de coordenação v3 usa HMAC-SHA256, sessão aleatória, sequência e validade; preferências do painel são transportadas nesse estado. O Mac controla as opções do par atual. Identidades privadas permanecem nos diretórios do usuário.

A câmera usa TLS 1.3 com certificado pareado fixado e prova HMAC por desafio. O listener de controle aguarda conexões sem polling; a captura física permanece desligada quando o compartilhamento não está em uso. A captura conserva o frame mais recente, os receptores têm pools limitados e a rede tem timeouts. Não há arquivos de frames, fila crescente ou captura de tela para detectar atividade.

A extensão CMIO é um componente de sistema instalado uma vez. O host original assinado é necessário para ativação, mas fica encerrado durante uso. O produtor escreve diretamente no sink público. [Componente](camera-component.md).

O estado desejado de política, o estado nativo dos hooks e a conexão DTLS são evidências distintas. Um status de captura ativa não prova que os comandos estejam chegando ao outro computador. O probe pareado verifica o transporte sem injetar entrada.
