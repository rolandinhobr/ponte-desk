# Opções

Os painéis do Mac e do Windows editam as mesmas preferências. O Mac mantém a cópia validada; o Windows a recebe pelo canal autenticado. As preferências do Mac ficam em `preferences.json`, separadas das identidades e da chave de pareamento. O painel aplica as mudanças sem um observador adicional. Opções inválidas pausam o compartilhamento até serem corrigidas.

| Opção | Padrão | Efeito |
|---|---|---|
| Destino do teclado e mouse | Passar pelas bordas | Escolha usar os periféricos dos dois no Mac ou no Windows, sem travessia e sem alterar telas. |
| Teclado e mouse | Ligado, após configurar o par | O interruptor principal pausa somente a entrada; a webcam tem controle próprio. |
| Proteger tela cheia no Windows | Ligado | No modo pelas bordas, suspende envio e recebimento de entrada nos dois computadores. |
| Proteger tela cheia no Mac | Ligado | No modo pelas bordas, impede que o cursor saia do Mac; não bloqueia receber o mouse do par. |
| Disponibilizar webcam | Ligado | Permite iniciar manualmente. Desligar encerra o compartilhamento. |
| Monitor compartilhado | Desligado | Sem ele, teclado/mouse não dependem de DDC. |
| Ajustar telas automaticamente | Desligado | Exige monitor compartilhado calibrado e tela integrada ativa no Mac. |

Nomes dos computadores e posição relativa podem ser alterados. Uma única configuração de par é suportada nesta versão.

## Identificar o monitor

Selecione a tela externa do Mac e o monitor correspondente do Windows. A identificação Windows inclui a saída e o dispositivo físico; a identificação Mac usa UUID. Não se escolhe o primeiro monitor com um modelo parecido.

Mude a entrada física para o Mac e clique em **Identificar entrada do Mac**. Depois mude para Windows e use **Identificar entrada do Windows**. Os dois valores precisam ser válidos e diferentes. Trocar o monitor selecionado invalida a calibração anterior. Outros monitores e drivers podem não informar a entrada por DDC; mantenha o modo independente nesse caso.

A reorganização de janelas é feita pelo próprio macOS. Não há garantia de restaurar a posição individual de cada janela ou Space. O recurso preserva a disposição, os modos e as frequências disponíveis e nunca desativa a última tela utilizável.

Autenticação, liberação de teclas, proteção em falha de rede e isolamento cooperativo não são opções desligáveis.

## Barra de menus e bandeja

O mesmo símbolo aparece no Mac e no Windows. O menu oferece abrir o painel, pausar/retomar teclado e mouse, ajustes, parar webcam e encerrar. Pausar em um lado pausa o par; retomar limpa as pausas manuais nos dois. No modo pelas bordas, continuam valendo o isolamento dos agentes e a proteção de tela cheia.

Use Recolher painel para ocultar a janela sem interromper uma sessão. Encerrar nunca espera autorizações, rede ou arquivos ocupados: cancela os comandos próprios e fecha o motor com prazo limitado. Não força a desmontagem de volumes. Uma limpeza não confirmada permanece registrada para conferência ao reabrir.

## Periféricos fixos

No cartão **Teclado e mouse** do painel, escolha **Usar os periféricos dos dois neste Mac** ou **Usar os periféricos dos dois no Windows**. A escolha pode ser feita de qualquer um dos painéis; no Windows, a opção correspondente diz **neste Windows**. Ela aplica o destino e retoma a entrada do par. **Passar pelas bordas** restaura a travessia configurada anteriormente.

O computador de origem envia apenas teclado e mouse físicos. O destino recebe a entrada e mantém seus próprios acessórios locais funcionando; não captura nem envia entrada de volta. O cursor permanece no destino, inclusive nas bordas e em tela cheia. Não há vídeo, troca de entrada, leitura DDC ou reorganização automática de monitores nesse modo. Webcam e cartão SD conservam seus controles independentes.

Agentes e programas no destino usam o cursor e o teclado do sistema normalmente, junto dos acessórios recebidos. Não há um segundo cursor no mesmo desktop. A lease confirma que não existe captura de retorno, sem pausar a recepção física. Se um agente opera a origem, sua lease suspende o envio para impedir que HID virtual seja confundido com acessório físico; eventos sintéticos identificáveis também são filtrados pelo backend.

O atalho de liberação indicado no painel devolve o controle local e pausa o par. **Pausar teclado e mouse** funciona nos dois painéis. Conexão não confirmada em dois segundos, falha no transporte e excesso de transições pendentes liberam a captura; o usuário retoma explicitamente pelo painel. A fila de início é limitada e agrega movimentos sem descartar a primeira tecla ou clique enquanto aguarda a confirmação do destino.
