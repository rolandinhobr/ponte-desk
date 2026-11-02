# Diagnóstico de cliques intermitentes

O campo `button_diagnostics` do `engine-status.json` mantém os últimos 256 registros de botões de cada motor. Ele serve para investigar cliques extras que cessam antes da observação manual. Isso não é uma correção da causa do clique duplo.

Arquivos locais:

- macOS: `~/.config/ponte-desk/engine-status.json`
- Windows: `%LOCALAPPDATA%\PonteDesk\engine-status.json`

A publicação ocorre junto ao status existente, normalmente a cada cinco segundos, por um worker de gravação. O histórico é circular, substituído conforme novos registros chegam e reiniciado ao encerrar o motor. Não há processo de monitoramento adicional. O buffer é pré-alocado; hooks nunca esperam pelo leitor do diagnóstico. Se houver disputa pelo buffer, só o registro de diagnóstico é descartado e `dropped_records` aumenta; a entrada segue seu caminho normal.

Cada registro contém horário em milissegundos, etapa e uma transição de botão ou estado sincronizado. Não contém teclas, posição ou deslocamento do cursor, texto, nome de aplicativo, título de janela ou imagem. Não há nova transmissão automática desse histórico pela rede. Os arquivos de status são dados de diagnóstico privados e não devem ser publicados no repositório.

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
