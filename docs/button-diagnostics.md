# Diagnóstico de cliques intermitentes

O campo `button_diagnostics` do `engine-status.json` mantém os últimos 256 registros de botões de cada motor. No Mac, `posted_entries` guarda separadamente os últimos 256 eventos preparados para Quartz, e `modified_posted_entries` os últimos 64 desses eventos com modificadores. Assim o tráfego periódico de snapshots não apaga imediatamente os cliques e cliques comuns não apagam o histórico de modificadores. Ele serve para investigar cliques extras que cessam antes da observação manual. Isso não é uma correção da causa do clique duplo.

Arquivos locais:

- macOS: `~/.config/ponte-desk/engine-status.json`
- Windows: `%LOCALAPPDATA%\PonteDesk\engine-status.json`

A publicação ocorre junto ao status existente, normalmente a cada cinco segundos, por um worker de gravação. O histórico é circular, substituído conforme novos registros chegam e reiniciado ao encerrar o motor. Não há processo de monitoramento adicional. O buffer é pré-alocado; hooks nunca esperam pelo leitor do diagnóstico. Se houver disputa pelo buffer, só o registro de diagnóstico é descartado e `dropped_records` aumenta; a entrada segue seu caminho normal.

Cada registro contém horário em milissegundos, etapa e uma transição de botão ou estado sincronizado. As máscaras `remote_modifier_flags` e `local_modifier_flags` distinguem o estado remoto da amostra de modificadores locais usada naquele clique; nas versões anteriores, a amostra local vinha da tabela global do Quartz e podia incluir eventos sintéticos. Agora ela vem de transições locais de modificadores com origem de hardware (PID de origem zero); HID virtual pode ter a mesma classificação. Não contém teclas comuns, posição ou deslocamento do cursor, texto, nome de aplicativo, título de janela ou imagem. Não há nova transmissão automática desse histórico pela rede. Os arquivos de status são dados de diagnóstico privados e não devem ser publicados no repositório.

Etapas:

| Etapa | Significado |
| --- | --- |
| `physical` | Botão classificado como físico pelo hook do Windows; HID virtual também pode aparecer como físico. |
| `captured` | Transição retirada da fila de captura do Windows. |
| `repair` | Soltura sintetizada pelo reparo da fila de captura do Windows. |
| `send` | Tentativa de envio de transição/estado pelo motor de origem. |
| `receive` | Estado de botões aceito pelo receptor após autorização e verificação de sequência. |
| `emulated` | Transição aceita pela deduplicação para chamar o backend nativo. |
| `suppressed` | Pressionamento repetido ignorado pela deduplicação. Solturas redundantes do snapshot não ocupam o histórico. |
| `cleanup` | Liberação explícita da rota ou de botões mantidos pelo motor. |
| `posted` | Evento de botão preparado para publicação via Quartz no Mac, com seu `click_count`. O campo opcional `modifier_flags` registra a máscara de modificadores desse evento de mouse; ele não contém códigos de teclas nem conteúdo digitado. Não confirma o consumo pelo aplicativo. |

Botões são índices: 0 esquerdo, 1 direito, 2 central, 3 voltar, 4 avançar. `down` distingue pressionar e soltar. Snapshots trazem `serial` e uma máscara dos cinco botões; retransmissões com o mesmo serial não enchem o histórico de envio/recepção. Os relógios das máquinas podem diferir: compare sequências e intervalos de cada lado, sem presumir sincronismo perfeito entre horários absolutos.

Ao ocorrer a falha, preserve os dois arquivos de status antes de reiniciar ou produzir muitos novos cliques. Compare o caminho completo: pares extras já em `physical`, diferença entre captura/envio e recepção, liberação por `repair`/`cleanup`, ou `click_count` inesperado em `posted`. Nenhuma dessas observações isolada identifica hardware, rede ou sistema como causa. Registros descartados ou já substituídos impedem usar a ausência de uma entrada como prova.

## Tratamento de modificadores no macOS

Modificadores momentâneos usam eventos `FlagsChanged` com o código da tecla que mudou e os bits do lado correspondente. Eles não entram na tarefa de repetição de letras. Soltar um lado preserva o outro lado pressionado, e a sincronização absoluta libera cada lado conhecido. Os testes de regressão exercitam mil sequências de atalhos, soltura durante repetição, dois lados e reset absoluto, sem publicar entrada no sistema.

Essa correção cobre um defeito reproduzido em teste no caminho de modificadores. O histórico de uma recorrência mostrou cliques preparados com contagem 1 e Command, mas isso por si só não prova a causa de todo comportamento descrito como clique duplo. Não pressupõe uma dependência de aplicativos de ditado. Preserve os dois status para distinguir uma nova origem.

## Validação desta revisão

- Os cinco testes novos de transição de modificadores falharam com o despacho anterior e passaram com a correção; a construção nativa também confere tipo, identidade e flags de soltura sem publicar eventos.
- 68 testes no macOS e 49 no Windows passaram; Clippy nos dois sistemas e compilações de produção passaram.
- Testes isolados do coordenador confirmaram roteamento, modos fixos, pausa por tela cheia/agente, peer expirado e resposta com I/O bloqueado. Esses testes usam backends dummy e não demonstram ausência do sintoma intermitente no uso real.
- Os núcleos atualizados foram instalados preservando configuração e pareamento; as duas permissões existentes do Mac foram renovadas, e a sessão de webcam foi restaurada em 1920×1080. O histórico ampliado está publicado pelo motor instalado. O comportamento intermitente permanece sujeito à análise de novas ocorrências; uma melhora momentânea não é considerada comprovação da solução.

## Estado local independente de eventos sintéticos

Uma recorrência posterior foi preservada durante o sintoma: cliques de contagem 1 tinham `remote_modifier_flags=0` e `local_modifier_flags=0x100000` (Command). Isso localizou a contaminação na leitura global de teclas do Mac, não no estado remoto. As tabelas de estado do Quartz acumulam estado de fontes que publicam eventos; a fonte privada usada para publicar a ponte não torna uma consulta à tabela HID uma leitura física independente.

O receptor agora mantém um observador passivo só de `FlagsChanged`, aceitando origem de hardware e atualizando apenas o lado da tecla identificado pelo evento. Um Shift físico com flags de Command contaminadas não pode pressionar Command nesse estado. Eventos de programas (inclusive os publicados pela própria ponte) não alteram o estado local. Cada movimento, botão e rolagem lê uma máscara atômica, sem consultar a tabela global, esperar locks, gravar arquivos ou enfileirar entrada. Os modificadores remotos continuam separados e são combinados para preservar atalhos usando teclados dos dois lados.

O observador nunca consome ou modifica eventos. Na desativação do tap ele limpa os modificadores; timeout permite reativação. Se não puder iniciar, os modificadores locais ficam neutros e há um aviso no log, sem voltar à consulta contaminável. Ao ativar o receptor com um modificador já segurado, é necessário soltá-lo e pressioná-lo novamente: não se importa estado global no início. O observador termina junto do receptor; ele não captura texto, cliques, posições ou áudio. Desconexões de um teclado sem evento de soltura e dispositivos HID virtuais permanecem limitações da classificação de eventos.

Referência de estado do Quartz: <https://developer.apple.com/documentation/coregraphics/cgeventsourcestateid>.
