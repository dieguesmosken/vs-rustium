# VS Rustium

Editor de código nativo, pequeno e de código aberto, escrito em Rust. A interface usa
`winit`, `wgpu` e `glyphon`; o texto é armazenado em uma rope e o realce de sintaxe Rust
é feito com Tree-sitter.

> O projeto está em estágio inicial. Ele já permite editar e salvar um arquivo, mas ainda
> não é um ambiente de desenvolvimento completo: não há abas, terminal integrado,
> conclusão de código, diagnóstico do compilador ou diálogos de abrir/salvar.

## Recursos atuais

- Janela nativa e renderização de texto acelerada por GPU.
- Realce de sintaxe para código Rust com Tree-sitter.
- Numeração de linhas, cursor visível, seleção com mouse ou teclado e rolagem vertical por roda do mouse.
- Barra de status com linha, coluna, linguagem e estado do documento.
- Edição de texto, indentação automática ao pressionar Enter e tabulação de quatro espaços.
- Desfazer/refazer com histórico limitado a 256 estados.
- Abertura de um arquivo existente pela linha de comando e salvamento com `Ctrl+S`.
- Indicador de alterações não salvas no título da janela.
- Interface do site em português, inglês e espanhol.

## Requisitos

- Rust estável e Cargo. Instale usando [rustup](https://rustup.rs/).
- Um ambiente gráfico com suporte a wgpu/Vulkan, Direct3D ou Metal.
- No Linux, bibliotecas de desenvolvimento do sistema para winit e Vulkan. O workflow de
  release instala `libxkbcommon-dev`, `libwayland-dev`, bibliotecas X11/XCB e `libvulkan-dev`.

## Executar

Clone o repositório e entre na pasta:

```sh
git clone https://github.com/dieguesmosken/vs-rustium.git
cd vs-rustium
```

Abra um arquivo:

```sh
cargo run -- caminho/para/arquivo.rs
```

O caminho pode apontar para um arquivo que ainda não existe: o editor abre um buffer vazio
e `Ctrl+S` cria o arquivo nesse local. Sem um argumento, o editor inicia com um exemplo,
mas ainda não oferece “Salvar como”; para salvar suas alterações, inicie o editor passando
o caminho de um arquivo.

Para compilar a versão otimizada:

```sh
cargo build --locked --release
```

O executável fica em `target/release/` (`vs-rustium.exe` no Windows e `vs-rustium` em
Linux/macOS).

## Atalhos e edição

| Tecla | Ação |
| --- | --- |
| Setas | Mover o cursor |
| `Shift` + setas | Estender a seleção |
| `Home` / `End` | Início / fim da linha |
| `Page Up` / `Page Down` | Mover aproximadamente uma tela |
| `Ctrl` + seta esquerda/direita | Mover o cursor por palavras |
| `Ctrl+A` | Selecionar todo o texto |
| `Backspace` / `Delete` | Apagar caractere ou seleção |
| `Enter` | Nova linha com a indentação atual |
| `Tab` | Inserir quatro espaços ou substituir a seleção por quatro espaços |
| `Shift+Tab` | Remover indentação no início da linha atual |
| `Ctrl+S` | Salvar no caminho informado ao iniciar |
| `Ctrl+Z` | Desfazer |
| `Ctrl+Shift+Z` ou `Ctrl+Y` | Refazer |

Clique para posicionar o cursor e arraste para selecionar; a seleção também pode ser
estendida usando Shift com as setas, Home, End e Page Up/Page Down. O buffer acompanha
verticalmente o cursor quando ele é movido pelo teclado; a roda do mouse rola o documento
independentemente do cursor. Linhas longas não quebram automaticamente e podem ficar fora
da área visível.

## Próximos recursos planejados

1. Localizar e substituir texto dentro do arquivo.
2. Copiar, recortar e colar usando a área de transferência do sistema.
3. Abrir e salvar arquivos por diálogos nativos, incluindo “Salvar como”.

Esses itens ainda não estão implementados. A ordem é uma proposta para orientar próximas
contribuições e pode mudar conforme o feedback.

## Testes e formatação

Execute os testes unitários e verifique a formatação:

```sh
cargo test --locked
cargo fmt -- --check
```

## Site e releases

O site está em [`docs/`](docs/) e é publicado pelo GitHub Pages quando há alterações nessa
pasta na branch `master`. Os arquivos de release são compilados em Windows, Linux e macOS
quando uma tag `v*` é enviada. Os nomes dos pacotes publicados são:

- `vs-rustium-windows-x86_64.zip`
- `vs-rustium-Linux.tar.gz`
- `vs-rustium-macOS.tar.gz`

## Contribuir

Relatos de bugs, sugestões, documentação e pull requests são bem-vindos. Abra uma issue
com os passos para reproduzir o problema, o sistema operacional e, quando aplicável, a
saída de erro do terminal. Mantenha as mudanças focadas e execute os testes antes de enviar
um pull request.

## Licença

Consulte a licença aplicável ao repositório antes de redistribuir ou modificar o projeto.
