# todo-overlay

Overlay de to-do list para Windows: uma janelinha semitransparente, sem borda, sempre no topo, fixa num canto da tela, para consultar rapidamente o que está pendente. Um atalho global mostra/esconde. Dados 100% locais, sem rede.

**Requisito número 1: ser muito leve.** Meta: `.exe` < 10 MB, RAM ociosa < 80 MB (working set; ~55 MB disso é o driver OpenGL da NVIDIA, que o app não controla), ~0% de CPU quando ninguém está interagindo. Toda decisão técnica deve respeitar isso.

## Stack

- **Rust** (edição 2024, toolchain stable via rustup, target `x86_64-pc-windows-msvc`)
- **eframe/egui 0.37** com backend `glow` (OpenGL — mais leve que `wgpu`); `default-features = false`
- `global-hotkey` (atalho global), `tray-icon` (ícone na bandeja), `winreg` (iniciar com o Windows), `windows-sys` (chamadas Win32 pontuais, ex.: `ShowWindowAsync`)
- `serde` + `serde_json` (persistência), `directories` (caminho do `%APPDATA%`), `anyhow` (erros)
- `winresource` (build-dependency: ícone embutido no `.exe`)

## Estrutura

```
todo-overlay/
├─ Cargo.toml
├─ build.rs            # embute assets/icon.ico no .exe (se existir)
├─ .cargo/config.toml  # +crt-static: runtime C dentro do .exe (sem VC++ Redistributable)
├─ .github/
│  ├─ workflows/ci.yml       # push em main e PRs: fmt, clippy, test, build e metas do .exe
│  ├─ workflows/release.yml  # tag vX.Y.Z → valida, compila e publica a GitHub Release
│  └─ dependabot.yml         # atualizações mensais de crates e actions
├─ assets/             # icon.ico (gerado por gerar_icone.py; a bandeja o lê do recurso 1 do .exe), screenshot.png
├─ tests/fixtures/     # arquivos JSON de versões antigas, usados nos testes de migração
└─ src/
   ├─ main.rs          # ponto de entrada; sem console no build de release
   ├─ app.rs           # janela overlay + UI da lista (egui)
   ├─ app/ajustes.rs   # tela de configurações e estado do config.json
   ├─ app/widgets.rs   # barra de título, caixa, lixeira, engrenagem, alças de resize
   ├─ tema.rs          # paleta One Half Dark + fonte Cascadia Code (lida do sistema)
   ├─ model.rs         # Tarefa / ListaTarefas — lógica pura, SEM dependência de UI, com testes
   ├─ storage.rs       # carregar/salvar tasks.json com gravação atômica + migrações
   ├─ config.rs        # Config (atalho, opacidade) em config.json, com testes
   ├─ hotkey.rs        # thread de fundo: atalho global, bandeja e janela de sessão
   ├─ tray.rs          # ícone na bandeja: Mostrar/Esconder, Configurações, Sair
   └─ autostart.rs     # HKCU\Software\Microsoft\Windows\CurrentVersion\Run
```

Dados do usuário: `%APPDATA%\todo-overlay\tasks.json` e `%APPDATA%\todo-overlay\config.json` (mesmas regras: gravação atômica, corrompido → `.bak`, versão futura → somente leitura).
"Iniciar com o Windows" não fica no `config.json`: a fonte da verdade é o valor `todo-overlay` na chave `Run` do registro.
Instalação local: `%LOCALAPPDATA%\Programs\todo-overlay\todo-overlay.exe` (a chave `Run` aponta para lá, nunca para `target\`). Atualizar = Sair pela bandeja, copiar o `.exe` novo por cima, abrir de novo.
Posição/tamanho da janela: salvos pelo próprio eframe (`persist_window`) em `%APPDATA%\todo-overlay\data\app.ron`.

## Comandos

```powershell
cargo run                                   # rodar em debug
cargo build --release                       # .exe final em target\release\todo-overlay.exe
cargo fmt                                   # formatar
cargo clippy --all-targets -- -D warnings   # lint (deve passar sem avisos)
cargo test                                  # testes (model, storage, config)
```

Antes de qualquer commit: `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test` precisam passar (o CI roda o mesmo em cada PR).

## Convenções de código

- `model.rs` e `storage.rs` não importam `egui`/`eframe` — a lógica é testável sem janela.
- Nada de `unwrap()`/`expect()` em caminhos que dependem de I/O ou entrada do usuário; use `anyhow::Result` e mostre/registre o erro. `expect` só para invariantes de inicialização impossíveis de falhar.
- **Nunca perder dados**: gravação de JSON é atômica (escreve `*.tmp` e renomeia). Arquivo corrompido → renomeia para `*.bak` e começa vazio, nunca sobrescreve silenciosamente.
- Mudou o formato do JSON → campo `version` + migração + teste (skill `migrar-dados`).
- Performance: não chamar `ctx.request_repaint()` em loop; nada de alocação/clonagem pesada por frame; I/O fora do `ui`/`logic` por frame (salvar só quando a lista muda).
- `unsafe` só em `hotkey.rs`/`app.rs` para Win32, sempre com comentário `// SAFETY:`.
- **Idioma**: textos da interface, comentários e identificadores em português. Exceção: campos que espelham o JSON salvo (`text`, `done`, `tasks`, `next_id`, `hotkey`, `opacity`, `version`) mantêm o nome do arquivo, para não exigir migração; nomes vindos de crates (egui, Win32) ficam como são.
- Comentários explicam o *porquê* (restrição do Windows/eframe, risco de perder dados), em poucas linhas. Nada de histórico ("marco 3", "antes era..."): isso vai no commit.
- Dependência nova só se justificar o peso; prefira `default-features = false`.

## Git e commits

- Branch principal `main`; features em `feat/<nome-curto>`, correções em `fix/<nome-curto>`, o resto em `docs/`, `chore/`, `refactor/`.
- Mensagens no padrão **Conventional Commits** (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`), em português, no imperativo. Ex.: `feat: adiciona atalho global para mostrar/esconder`. `fix:` só para corrigir comportamento errado; funcionalidade nova é `feat:`.
- PRs via `gh pr create`, mesclados com **squash merge** (um commit por PR em `main`, com o título do PR).
- Sem trailers automáticos (`Co-Authored-By`, "Generated with ...") em commits, PRs, tags e releases. O uso do Claude Code é declarado uma vez, no README (seção "Como foi feito"), e não em cada commit.

## Estado e próximos passos

MVP concluído na v0.5.0: janela overlay, lista com persistência, atalho global + bandeja, configurações e iniciar com o Windows, release pelo CI. Ideias para depois (não prometidas): reordenar tarefas. O README existe em português (`README.md`) e inglês (`README.en.md`): mudou um, atualize o outro no mesmo PR.

## Restrições técnicas conhecidas

- **Janela escondida não roda `ui`.** O atalho global é tratado na thread de fundo (`hotkey.rs`), que mostra/esconde via `ShowWindowAsync` e chama `ctx.request_repaint()`. `ShowWindow` síncrono daria deadlock se o atalho fosse apertado durante a saída.
- **`App::logic` roda mesmo escondido** (eframe 0.37, passo só de lógica após `request_repaint`). É lá que X/Alt+F4 viram "esconder" (`CancelClose`); o "Sair" da bandeja liga a flag `saindo` + `ViewportCommand::Close`, o que mantém o `on_exit` funcionando.
- **Uma thread de fundo só**, com `GetMessageW`, para atalho, bandeja e janela de sessão; encerrada no `Drop` de `Fundo` para não deixar ícone fantasma.
- **Logoff/desligamento**: o winit não trata `WM_ENDSESSION`. A janela oculta `todo-overlay-sessao` recebe a mensagem, pede a gravação à UI (`hotkey::Encerramento`, atendido em `App::logic` → `gravar_pendencias`) e espera a confirmação por até 3 s.
- **Trocar o atalho**: `RegisterHotKey` precisa rodar na thread do `GlobalHotKeyManager`. A UI posta `WM_APP_ATALHO` para a janela `todo-overlay-sessao` (não `PostThreadMessageW`, que se perde com o menu da bandeja aberto) e lê a resposta em `App::logic`, sem bloquear. O novo é registrado antes de soltar o antigo.
- **Captura do atalho**: combinação já registrada por outro programa é interceptada pelo Windows e nem chega ao egui. A captura usa `physical_key` (no ABNT2, Ctrl+Alt+Q chega como "/" na tecla lógica, por ser AltGr) e `config::recusa` barra combinações que quebrariam o Windows (Shift+letra, Alt+F4, Ctrl+C, AltGr do ABNT2...).
- **Ids do egui**: os automáticos dependem da posição; `TextEdit` que precisa manter o foco usa id explícito.
- O overlay não aparece sobre jogos em tela cheia exclusiva.

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
