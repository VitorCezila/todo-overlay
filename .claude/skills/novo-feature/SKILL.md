---
name: novo-feature
description: Use ao começar qualquer item do roadmap ou melhoria do todo-overlay (ex. "implementa o atalho global", "adiciona edição de tarefa") — cria branch, implementa, valida com fmt/clippy/test, commita em Conventional Commits e abre PR.
---

# Nova feature no todo-overlay

## Quando usar
Sempre que for implementar algo novo ou corrigir um bug que mereça branch própria. Para adicionar só uma opção de configuração, prefira `nova-config`; para mudar o formato do JSON salvo, siga também `migrar-dados`.

## Passo a passo
1. **Entender o escopo**: identificar qual marco do roadmap (CLAUDE.md) a feature cobre e quais módulos de `src/` serão tocados. Se houver ambiguidade de comportamento visível ao usuário, pergunte antes.
2. **Branch** a partir de `main` atualizada:
   ```powershell
   git switch main; git pull; git switch -c feat/<nome-curto>
   ```
   (bug → `fix/<nome-curto>`).
3. **Implementar** respeitando a separação:
   - lógica pura em `model.rs` / `storage.rs` / `config.rs` (sem `egui`);
   - UI em `app.rs`; Win32/atalho em `hotkey.rs`; bandeja em `tray.rs`; registro em `autostart.rs`.
4. **Testes**: toda lógica nova em `model`/`storage`/`config` ganha teste em `#[cfg(test)] mod tests` no próprio arquivo. Para volume maior, delegue ao agent `test-writer`.
5. **Validar** (tudo precisa passar):
   ```powershell
   cargo fmt; cargo clippy --all-targets -- -D warnings; cargo test
   ```
6. **Testar manualmente** com `cargo run`: janela continua transparente, no topo, arrastável; a feature funciona; esconder/mostrar não quebrou.
7. **Leveza**: se a feature mexe em loop de UI, timers, threads ou dependências, peça ao agent `perf-auditor` uma checagem.
8. **Revisão**: delegue ao agent `code-reviewer` o diff (`git diff main...HEAD`) e corrija o que for apontado como bloqueante.
9. **Commit** em Conventional Commits, português, imperativo:
   ```powershell
   git add -A; git commit -m "feat: adiciona atalho global para mostrar/esconder"
   ```
   **Sem** `Co-Authored-By`, sem "Generated with Claude Code", sem qualquer menção ao Claude/Anthropic.
10. **PR** (só se o usuário pedir push/PR):
    ```powershell
    git push -u origin HEAD; gh pr create --fill --base main
    ```
    Corpo do PR: o que mudou, como testar manualmente. Sem rodapé de atribuição.
11. Se a feature mudou comandos, estrutura ou convenções, atualize o `CLAUDE.md` no mesmo PR.

## Convenções deste projeto
- Prefixos: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`.
- Um commit por mudança lógica; não misturar refactor com feature.
- Nova dependência → justificar no corpo do commit e usar `default-features = false` quando possível.

## Checklist final
- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` e `cargo test` passam
- [ ] Testado com `cargo run`
- [ ] Sem `unwrap` em I/O, sem `request_repaint` em loop
- [ ] Commit/PR sem qualquer atribuição ao Claude
- [ ] `CLAUDE.md` atualizado se necessário
