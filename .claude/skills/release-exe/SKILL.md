---
name: release-exe
description: Use quando houver mudanças acumuladas em main que valem uma versão ou quando o usuário pedir "gera o exe"/"faz uma release" do todo-overlay — compila em release, mede tamanho e RAM ociosa contra as metas, cria tag e GitHub Release.
---

# Release do .exe

## Quando usar
Quando há mudanças em main que valem uma versão, ou quando o usuário quiser instalar a versão nova no próprio PC.

## Passo a passo
1. Estar em `main` limpa e atualizada: `git switch main; git pull; git status`.
2. **Validar tudo**:
   ```powershell
   cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
   ```
3. **Versão**: atualizar `version` em `Cargo.toml` (SemVer: `0.x.0` para funcionalidades novas, `0.x.y` só para correções).
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
7. **Commit** da versão (via PR, como qualquer mudança) e, depois do merge, **tag anotada na `main`**. A mensagem da tag vira as notas da release:
   ```powershell
   git switch main; git pull
   git tag -a v0.x.0 --cleanup=verbatim -F <arquivo-com-as-notas.md>   # verbatim: sem isso o git apaga as linhas "## ..."
   ```
8. **GitHub Release pelo CI** (só se o usuário pedir push): `git push origin v0.x.0`. O workflow `.github/workflows/release.yml` roda fmt/clippy/test, confere tag = `Cargo.toml`, compila, barra `.exe` ≥ 10 MB ou dependente de `VCRUNTIME140.dll` e publica a release com o `.exe`, o tamanho e o SHA-256. Acompanhe com `gh run watch`. Não publique com `gh release create` local: o `.exe` oficial é o do CI.
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
- Sem trailers automáticos em commit, tag ou release.

## Checklist final
- [ ] fmt/clippy/test ok
- [ ] `.exe` < 10 MB, RAM ociosa < 80 MB, CPU ociosa ~0%
- [ ] Smoke test manual feito
- [ ] Versão no `Cargo.toml` = tag
