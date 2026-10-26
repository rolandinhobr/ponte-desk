# Aplicativo nativo do Windows

WinForms sobre o .NET Framework incluído no Windows 11, sem WebView, navegador, npm ou serviço adicional. `PonteDesk.exe` é o ponto de entrada do aplicativo e gerencia `ponte-desk.exe`, o motor instalado ao lado. As DLLs nativas permanecem nos locais protegidos definidos pelos instaladores existentes.

Compile com `./native/windows/app/build.ps1`. O script usa o compilador do .NET Framework do Windows, trata avisos como erros, executa verificações de argumentos/sessões/ícone e incorpora o mesmo símbolo da barra do Mac no executável. A saída fica em `native/windows/app/build/PonteDesk.exe`. Os binários não são versionados.

O aplicativo mantém um único ícone por usuário, possui menu de bandeja e painel, recolhe a janela ao fechar e inicia sem tomar o foco quando recebe `--background`. Instale com o script principal depois do pareamento. `--preview` abre uma interface sem iniciar o motor ou enviar comandos; `--render-preview CAMINHO.png` renderiza uma fixture fora da área visível, sem usar dados reais.

As operações de rede e autenticação rodam fora da thread da interface. Credenciais SMB são solicitadas pelo sistema; o painel não possui campo nem arquivo de senha. Opções compartilhadas exigem 0.5 nos dois sistemas. Operações que exigem administrador ficam nos componentes específicos, nunca na interface residente.
