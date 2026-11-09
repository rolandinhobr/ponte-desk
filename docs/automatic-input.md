# Destino automático

Escolha **Automático — acompanhar meu uso** em **Destino do teclado e mouse**, no painel do Mac ou Windows. Os dois computadores precisam estar atualizados. O modo usa o mesmo transporte autenticado, captura e emulação dos modos fixos. Não compartilha imagem da tela e não inicia webcam, áudio ou cartão SD.

## Regras

- O Mac coordena uma única decisão e a publica ao Windows pelo canal autenticado. O painel mostra destino, motivo e espera por soltura.
- Ao iniciar, usa o computador preferido. Uma entrada de monitor válida e estável pode indicar outro destino; a primeira leitura não substitui uma indicação humana local recente.
- Atividade física local no Mac, incluindo teclado e trackpad, assume o Mac quando a opção está ligada. O primeiro evento fica no Mac mesmo se ele estivesse enviando entrada ao Windows: o hook deixa o evento local antes de a decisão de rede terminar.
- O movimento do mouse ou teclado do PC já enviado ao Mac conta como atividade humana para a sessão, mas não escolhe o Windows. Isso evita oscilações ao usar um periférico conectado à outra máquina.
- No Windows, o retorno pela entrada do monitor ou por desbloqueio com atividade humana seleciona Windows. Atividade física no Windows atualiza a contagem de uso, mas não disputa a prioridade de uma retomada local no Mac.
- Desbloqueio precisa de atividade física próxima do evento. Existe uma janela de cinco segundos para o primeiro evento após desbloquear; desbloqueio sem entrada física não escolhe destino. No Mac, a informação vem dos metadados da sessão; no Windows, do estado de bloqueio da sessão WTS. Diálogos de UAC não são tratados como desbloqueio.
- A entrada do monitor precisa de calibração, leitura com menos de 12 segundos e estabilidade de 1,2 segundo. Só uma mudança dispara a escolha: a entrada que continua no Windows não toma repetidamente o controle do MacBook. DDC roda no worker existente, separado da entrada.
- Durante bloqueio/desbloqueio, o controle fica local até a intenção humana ser confirmada e as teclas/botões serem soltos. Isso mantém o primeiro clique no desktop desbloqueado.
- Uma mudança espera ambos os lados soltarem teclas e botões. Não há soltura forçada por prazo para aplicar automação. Uma intenção pendente expira após 15 segundos e pode ser renovada por novo sinal.
- Existe um intervalo mínimo de 1,2 segundo entre mudanças de destino. Após 120 segundos sem entrada humana, a próxima indicação é tratada como chegada. Se a indicação for ambígua, o destino permanece.
- Pausa manual sempre prevalece. Falha de rede ou observador indisponível preserva controle local. A inversão espera a captura anterior sair antes de instalar a nova captura.
- As proteções de tela cheia do Windows continuam valendo no automático quando o monitor está nele (ou quando não há monitor compartilhado). Em modo fixo manual continua valendo o comportamento anterior.

Agentes não escolhem destino. Eventos sintéticos identificáveis são ignorados, e leases de agentes bloqueiam novas decisões. Agente no destino pode continuar trabalhando ali como nos modos fixos; agente na origem suspende o envio. HID virtual pode se apresentar como físico, portanto a lease continua necessária para automação de desktop.

## Ajustes nos dois painéis

- **Atividade local assume o controle** (`auto_local_activity`, padrão ligado). Se quiser enviar também os acessórios locais do Mac ao Windows sem que eles retomem o Mac, desligue essa regra ou use o modo fixo manual.
- **Acompanhar a entrada do monitor compartilhado** (`auto_monitor`, padrão ligado). Só atua quando **Monitor compartilhado** está ligado e calibrado. Sem monitor compartilhado, o automático funciona pelos demais sinais.
- **Acompanhar desbloqueio com atividade humana** (`auto_unlock`, padrão ligado).
- **Computador preferido ao iniciar** (`auto_preferred`, `mac` ou `pc`; padrão `mac`).
- `auto_idle_seconds` define a chegada após inatividade (15–3600, padrão 120), disponível também pelo comando de preferências.

**Ajustar telas do Mac automaticamente** é independente da escolha de destino. Quando ligado e calibrado, acompanha a entrada física do monitor também em modo automático, mesmo se o teclado/mouse continuarem no MacBook. O recurso não muda a entrada física do monitor.

Os modos pelas bordas e fixos continuam disponíveis. A escolha de automático não altera a calibração nem ativa reorganização de telas por conta própria.

## Carga e privacidade

Os observadores são internos ao processo e passivos. No Mac observam eventos com PID de origem zero; no Windows usam Raw Input de dispositivos, com reforço da contagem durante captura física para que a ordem dos hooks não esconda eventos. Eventos injetados identificáveis não entram nessa contagem. Só há contadores, idade da última atividade e indicação de algum botão/tecla pressionado no estado compartilhado. Não são registrados nem enviados texto, códigos de teclas, coordenadas, nomes de apps, áudio ou imagens. O bitmap temporário de teclas atualmente pressionadas fica apenas na memória e é limpo na suspensão/bloqueio.

Callbacks fazem operações atômicas e retornam; não fazem rede, disco, DDC ou filas de eventos. A decisão e os metadados usam o coordenador já existente, normalmente a cada 500 ms. Fora do modo automático, a coleta fica desativada.

Referências das APIs: [observação passiva do macOS](https://developer.apple.com/documentation/coregraphics/cgeventtapoptions/listenonly), [sessão do macOS](https://developer.apple.com/documentation/coregraphics/cgsessioncopycurrentdictionary()), [limites de hooks no Windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelmouseproc), [estado de bloqueio da sessão Windows](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoex_level1_w).

## Validação da versão 0.7

Passaram 87 testes de biblioteca no macOS, 63 no Windows, Clippy nos dois sistemas, a compilação das interfaces e 19 verificações da interface Windows. Os testes cobrem origem emprestada sem retorno automático, prioridade local sobre a primeira leitura do monitor, estabilidade/expiração, agentes, espera por teclas/botões, inversão da captura e controle local durante desbloqueio. Os testes isolados do coordenador também passaram com backends dummy e I/O bloqueado. As interfaces e motores foram instalados preservando configuração, pareamento e componentes assinados; a leitura real da entrada do monitor respondeu. A ativação da entrada nativa continua condicionada às permissões do sistema. Esses resultados não substituem uma validação física prolongada de todas as combinações de dispositivos, bloqueio e tela cheia.
