# Opções

Os painéis do Mac e do Windows editam as mesmas preferências. O Mac mantém a cópia validada; o Windows a recebe pelo canal autenticado. As preferências do Mac ficam em `preferences.json`, separadas das identidades e da chave de pareamento. O painel aplica as mudanças sem um observador adicional. Opções inválidas pausam o compartilhamento até serem corrigidas.

| Opção | Padrão | Efeito |
|---|---|---|
| Teclado e mouse | Ligado, após configurar o par | O interruptor principal pausa somente a entrada; a webcam tem controle próprio. |
| Proteger tela cheia no Windows | Ligado | Suspende envio e recebimento de entrada nos dois computadores. |
| Proteger tela cheia no Mac | Ligado | Impede que o cursor saia do Mac; não bloqueia receber o mouse do par. |
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

O mesmo símbolo aparece no Mac e no Windows. O menu oferece abrir o painel, pausar/retomar teclado e mouse, ajustes, parar webcam e encerrar. Pausar em um lado pausa o par; retomar limpa as pausas manuais nos dois. Isso não remove o isolamento de agentes nem ignora tela cheia.

No Windows, a janela pode ficar fechada sem interromper uma sessão. Encerrar desmonta o cartão normalmente e aguarda a limpeza antes de fechar o motor. Se houver arquivos ocupados ou uma autorização pendente, o app preserva a sessão e informa a pendência.
