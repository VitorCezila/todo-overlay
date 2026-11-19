---
name: release-exe
description: Use ao fechar um marco do roadmap ou quando o usuário pedir "gera o exe"/"faz uma release" do todo-overlay — compila em release, mede tamanho e RAM ociosa contra as metas, cria tag e GitHub Release.
---

# Release do .exe

## Quando usar
Ao concluir um marco do roadmap, ou quando o usuário quiser instalar a versão nova no próprio PC.

## Passo a passo
1. Estar em `main` limpa e atualizada: `git switch main; git pull; git status`.
2. **Validar tudo**:
   ```powershell
   cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
   ```
3. **Versão**: atualizar `version` em `Cargo.toml` (SemVer: `0.x.0` para marcos do MVP, `0.x.y` para correções).
4. **Build**: `cargo build --release` (o perfil release já tem `lto`, `codegen-units = 1`, `strip`, `panic = "abort"`).
5. **Medir** e comparar com as metas do CLAUDE.md:
   ```powershell
   (Get-Item target\release\todo-overlay.exe).Length / 1MB      # meta: < 10 MB
   Start-Process target\release\todo-overlay.exe; Start-Sleep 5
   Get-Process todo-overlay | Select-Object WorkingSet64, PrivateMemorySize64, CPU   # meta: < 80 MB, CPU estável
   Start-Sleep 30; Get-Process todo-overlay | Select-Object CPU                        # CPU não deve subir ociosa
   Stop-Process -Name todo-overlay
   ```
   Se alguma meta estourar, **pare** e acione o agent `perf-auditor` antes de lançar.
6. **Smoke test manual** do `.exe` de release: abre transparente e no topo, atalho mostra/esconde, bandeja funciona, tarefas persistem após fechar/abrir.
7. **Commit e tag**:
   ```powershell
   git commit -am "chore: release v0.x.0"; git tag v0.x.0
   ```
8. **GitHub Release** (só se o usuário pedir push):
   ```powershell
   git push; git push --tags
   gh release create v0.x.0 target\release\todo-overlay.exe --title "v0.x.0" --notes "<o que mudou + medições de tamanho/RAM>"
   ```
9. Atualizar a instalação local: pedir ao usuário para fechar o app pelo "Sair" da bandeja (não usar `Stop-Process`: pode perder uma edição em andamento) e então:
   ```powershell
   $dest = "$env:LOCALAPPDATA\Programs\todo-overlay"
   New-Item -ItemType Directory -Force $dest | Out-Null
   Copy-Item target\release\todo-overlay.exe $dest -Force
   Start-Process "$dest\todo-overlay.exe"
   ```
   A chave `HKCU\...\Run\todo-overlay` deve apontar para `$dest\todo-overlay.exe`, nunca para `target\`.

## Convenções deste projeto
- Notas da release em português, listando features/fixes desde a última tag (`git log <ultima-tag>..HEAD --oneline`) e as medições.
- Nenhuma menção/atribuição ao Claude em commit, tag ou release.

## Checklist final
- [ ] fmt/clippy/test ok
- [ ] `.exe` < 10 MB, RAM ociosa < 80 MB, CPU ociosa ~0%
- [ ] Smoke test manual feito
- [ ] Versão no `Cargo.toml` = tag
