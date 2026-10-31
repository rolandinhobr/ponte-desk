# Cartão SD pela rede

O Ponte Desk pode publicar um cartão ou mídia removível montada em um computador e montar o compartilhamento SMB no outro. O acesso remoto é **somente leitura**. Os arquivos são lidos sob demanda; não há cópia integral prévia, redirecionamento USB, formatação ou alteração do conteúdo do cartão.

## Usar

1. Conecte o cartão e abra **Cartão SD** no painel do Mac.
2. Escolha onde o cartão está conectado e clique em **Atualizar**. O aplicativo também recebe notificações de montagem e remoção de volumes locais do Mac.
3. Selecione o cartão e clique em **Compartilhar cartão**. Autorize a criação do compartilhamento na máquina de origem.
4. Autentique a conexão SMB com uma conta habilitada na origem quando o sistema pedir. O Ponte Desk não recebe nem armazena essa senha.
5. Use **Abrir no destino** para abrir o Finder ou Explorer. Para terminar, use **Desmontar e parar**.

A desmontagem normal recusa arquivos ocupados; o app preserva o compartilhamento quando não consegue confirmar a desmontagem. Feche os arquivos e tente novamente. Ejetar ou desconectar fisicamente o SD durante uma leitura interrompe a operação: confirme a desmontagem antes de retirar o cartão.

O computador com o cartão precisa permanecer ligado e acessível. Aplicativos que exigem um dispositivo SD físico podem não aceitar um volume de rede. Esta versão usa o endereço IPv4 já pareado e uma sessão de cartão por par de computadores.

## Configuração do Mac como origem

**Configurar…** verifica o auxiliar e abre o Instalador do macOS quando ele está ausente ou precisa ser atualizado. Conclua a instalação de **PonteDeskStorage** com a autorização administrativa na janela do sistema e clique novamente em **Configurar…**. O pacote está incluído no aplicativo; não exige assinatura paga, download adicional ou outro aplicativo em execução.

O Instalador coloca o executável em `/Library/PrivilegedHelperTools/br.com.ponte-desk.storage-helper` e o registro em `/Library/LaunchDaemons/br.com.ponte-desk.storage-helper.plist`, ambos pertencentes a root. O cliente confere proprietário, permissões e SHA256 do executável e fixa a conexão XPC ao hash da assinatura do auxiliar incluído no aplicativo. O componente é iniciado sob demanda e encerra após ficar sem sessões. O app só informa que ele está pronto após uma resposta XPC real.

Versões anteriores usavam SMAppService com assinatura ad hoc. Essa combinação pode falhar na restrição de lançamento do macOS mesmo após a autorização em Itens de Início. Configurar remove somente esse registro antigo do Ponte Desk antes de abrir o instalador. Nenhuma permissão de outro aplicativo é modificada e a base de Itens de Início não é reiniciada. A instalação e a migração recusam sessões SD ainda compartilhadas.

Também é necessário ativar o SMB e habilitar uma conta em **Geral → Compartilhamento → Compartilhar Arquivos → Opções**. Antes de ativar, revise as pastas já cadastradas: elas pertencem à configuração de compartilhamento do macOS, e não ao Ponte Desk. O aplicativo não remove nem muda compartilhamentos existentes.

O auxiliar apenas cria compartilhamentos de volumes removíveis que ainda estejam presentes, com leitura exclusiva, convidado desativado e criptografia SMB exigida. As operações administrativas exigem autorização do sistema. O auxiliar é acionado sob demanda e libera a sessão quando seu controlador sai. Seu registro permite recuperar compartilhamentos próprios após interrupção do auxiliar.

## Configuração do Windows como origem

Instale `PonteDeskStorage.dll` em Program Files com o instalador incluído. O serviço principal continua como usuário comum. Uma sessão de publicação abre uma solicitação UAC; o auxiliar elevado permanece somente enquanto há uma sessão ativa.

A sessão recebe um nome aleatório e uma ACL de leitura para a identidade do controlador. Uma regra temporária permite TCP 445 somente a partir do IP pareado. Um ponto de montagem próprio vincula a sessão ao GUID do volume; a reutilização de uma letra de unidade não passa a compartilhar outra mídia. O encerramento remove o compartilhamento, a regra e o ponto de montagem próprios. Não muda a letra original nem os arquivos do SD.

## Autenticação e limites

O pareamento do Ponte Desk autentica os comandos de controle; o SMB autentica separadamente o acesso aos arquivos. As permissões da conta SMB e de outros compartilhamentos já existentes continuam sendo definidas pelo sistema operacional. Não habilite SMB1 nem acesso inseguro por convidados.

A velocidade depende do cartão, leitor e rede. Transferências podem disputar largura de banda com outros usos da rede. A captura de teclado/mouse não processa bytes do cartão.

Interrupções abruptas do auxiliar podem impedir confirmar a limpeza. Nesse caso, o aplicativo mostra erro e não declara a sessão encerrada; revise os compartilhamentos do sistema antes de retirar a mídia. Os compartilhamentos Windows são temporários e não persistem a uma reinicialização do servidor SMB.

## Encerrar e recuperar erros

Fechar a janela ou escolher **Encerrar Ponte Desk** sempre encerra o app, mesmo se a rede estiver indisponível, houver um pedido de autorização aberto ou o cartão estiver em erro. O encerramento cancela os comandos próprios com um prazo local limitado. Não força a desmontagem de volumes e não declara uma limpeza desconhecida como concluída.

O botão **Desmontar e parar** continua usando desmontagem normal e recusa arquivos ocupados. Se o controlador já saiu, o Mac confere a lista real de compartilhamentos para liberar uma sessão antiga. A atualização do auxiliar é vinculada à assinatura/conteúdo do componente; o app testa uma conexão real com ele antes de pedir autorização para publicar o cartão.
