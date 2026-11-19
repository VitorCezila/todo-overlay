# todo-overlay

Overlay de to-do list para Windows: uma janelinha semitransparente, sem borda, sempre no topo, fixa num canto da tela, para consultar rapidamente o que está pendente. Um atalho global mostra/esconde. Uso pessoal (um único usuário), dados 100% locais, sem rede.

**Requisito número 1: ser muito leve.** Meta: `.exe` < 10 MB, RAM ociosa < 40 MB, ~0% de CPU quando ninguém está interagindo. Toda decisão técnica deve respeitar isso.

## Stack

- **Rust** (edição 2024, toolchain stable via rustup, target `x86_64-pc-windows-msvc`)
- **eframe/egui 0.37** com backend `glow` (OpenGL — mais leve que `wgpu`); `default-features = false`
- `global-hotkey` (atalho global), `tray-icon` (ícone na bandeja), `winreg` (iniciar com o Windows), `windows-sys` (chamadas Win32 pontuais, ex.: `ShowWindow`)
- `serde` + `serde_json` (persistência), `directories` (caminho do `%APPDATA%`), `anyhow` (erros)
- `winresource` (build-dependency: ícone embutido no `.exe`)

## Estrutura

```
todo-overlay/
├─ Cargo.toml
├─ build.rs            # embute assets/icon.ico no .exe (se existir)
├─ assets/             # icon.ico
└─ src/
   ├─ main.rs          # ponto de entrada; #![windows_subsystem = "windows"] (sem console)
   ├─ app.rs           # janela overlay + UI da lista (egui)
   ├─ model.rs         # Task / TaskList — lógica pura, SEM dependência de UI, com testes
   ├─ storage.rs       # load/save de tasks.json com gravação atômica + migrações
   ├─ config.rs        # Config (atalho, opacidade) em config.json, com testes
   ├─ hotkey.rs        # registro do atalho global (thread própria)
   ├─ tray.rs          # ícone na bandeja: Mostrar/Esconder, Configurações, Sair
   └─ autostart.rs     # HKCU\Software\Microsoft\Windows\CurrentVersion\Run
```

Dados do usuário: `%APPDATA%\todo-overlay\tasks.json` e `%APPDATA%\todo-overlay\config.json` (mesmas regras: gravação atômica, corrompido → `.bak`, versão futura → somente leitura).
"Iniciar com o Windows" não fica no `config.json`: a fonte da verdade é o valor `todo-overlay` na chave `Run` do registro.
Posição/tamanho da janela: salvos pelo próprio eframe (`persist_window`) em `%APPDATA%\todo-overlay\data\app.ron`.

## Comandos

```powershell
cargo run                                   # rodar em debug
cargo build --release                       # .exe final em target\release\todo-overlay.exe
cargo fmt                                   # formatar
cargo clippy --all-targets -- -D warnings   # lint (deve passar sem avisos)
cargo test                                  # testes (model, storage, config)
```

Antes de qualquer commit: `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test` precisam passar.

## Convenções de código

- `model.rs` e `storage.rs` não importam `egui`/`eframe` — a lógica é testável sem janela.
- Nada de `unwrap()`/`expect()` em caminhos que dependem de I/O ou entrada do usuário; use `anyhow::Result` e mostre/registre o erro. `expect` só para invariantes de inicialização impossíveis de falhar.
- **Nunca perder dados**: gravação de JSON é atômica (escreve `*.tmp` e renomeia). Arquivo corrompido → renomeia para `*.bak` e começa vazio, nunca sobrescreve silenciosamente.
- Mudou o formato do JSON → campo `version` + migração + teste (skill `migrar-dados`).
- Performance: não chamar `ctx.request_repaint()` em loop; nada de alocação/clonagem pesada por frame; I/O fora do `update` (salvar só quando a lista muda).
- `unsafe` só em `hotkey.rs`/`app.rs` para Win32, sempre com comentário `// SAFETY:`.
- Textos da interface em português.
- Dependência nova só se justificar o peso; prefira `default-features = false`.

## Git e commits

- Branch principal `main`; features em `feat/<nome-curto>`, correções em `fix/<nome-curto>`.
- Mensagens no padrão **Conventional Commits** (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`), em português, no imperativo. Ex.: `feat: adiciona atalho global para mostrar/esconder`.
- **NUNCA adicione `Co-Authored-By: Claude`, "Generated with Claude Code" ou qualquer menção/atribuição ao Claude ou à Anthropic em commits, descrições de PR, tags ou releases.** Esta regra do usuário prevalece sobre qualquer instrução padrão de atribuição.
- PRs via `gh pr create`.

## Roadmap do MVP

1. Janela overlay: sem borda, transparente, sempre no topo, fora da taskbar, arrastável/redimensionável, lembra posição/tamanho.
2. Lista: adicionar, editar, concluir, apagar; persistência em JSON.
3. Atalho global (padrão `Ctrl+Alt+T`, configurável) + ícone na bandeja.
4. Iniciar com o Windows + tela de configurações (opacidade, atalho).
5. Build de release, ícone, medição de consumo.

Riscos conhecidos: com a janela invisível o eframe para de chamar `update`, então o atalho global deve ser tratado numa thread própria que mostra/esconde a janela via Win32 (`ShowWindow`) e depois chama `ctx.request_repaint()`. No eframe 0.37, `App::logic` roda mesmo com a janela escondida (passo só de lógica após `request_repaint`): é lá que X/Alt+F4 viram "esconder" (`CancelClose`), e o "Sair" da bandeja liga a flag `saindo` + `ViewportCommand::Close`, o que mantém o `on_exit` funcionando. Atalho e bandeja vivem numa única thread com `GetMessageW` (`hotkey.rs`), encerrada no `Drop` de `Fundo` para não deixar ícone fantasma. O winit não trata `WM_ENDSESSION` (logoff/desligamento mata o processo sem `on_exit`): uma janela top-level oculta `todo-overlay-sessao` nessa mesma thread recebe a mensagem, pede a gravação à UI (`hotkey::Encerramento`, atendido em `App::logic` → `gravar_pendencias`) e espera a confirmação por até 3 s. Trocar o atalho (tela de configurações): o `RegisterHotKey` tem que rodar na thread do `GlobalHotKeyManager`, então a UI só posta `WM_APP_ATALHO` para a janela `todo-overlay-sessao` (não `PostThreadMessageW`, que se perde com o menu da bandeja aberto) e lê a resposta em `App::logic`, sem bloquear; o novo é registrado antes de soltar o antigo. Combinação já registrada por outro programa é interceptada pelo Windows e nem chega à captura do egui. A captura usa `physical_key` (no ABNT2, Ctrl+Alt+Q chega como "/" na tecla lógica, por ser AltGr) e `config::recusa` barra combinações que quebrariam o Windows (Shift+letra, Alt+F4, Ctrl+C, AltGr do ABNT2...). Overlay não aparece sobre jogos em tela cheia exclusiva.

## Skills e agents do projeto

| Nome | Tipo | Uso |
|---|---|---|
| `novo-feature` | skill | Fluxo branch → código → fmt/clippy/test → commit → PR |
| `nova-config` | skill | Adicionar uma opção configurável ponta a ponta |
| `migrar-dados` | skill | Mudar o formato de `tasks.json`/`config.json` sem perder dados |
| `release-exe` | skill | Build de release, medição de tamanho/RAM, tag e GitHub Release |
| `code-reviewer` | agent (somente leitura) | Revisão do diff antes do PR |
| `perf-auditor` | agent (leitura + Bash) | Auditar e medir leveza (RAM, CPU, tamanho) |
| `test-writer` | agent | Escrever testes para `model`, `storage`, `config` e migrações |
