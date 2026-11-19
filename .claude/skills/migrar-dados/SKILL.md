---
name: migrar-dados
description: Use sempre que for alterar a struct Tarefa/ListaTarefas/Config de forma que mude o JSON salvo (renomear, remover ou mudar tipo de campo, reestruturar) no todo-overlay — garante versão, migração e teste para não perder as tarefas do usuário.
---

# Migrar formato de dados

## Quando usar
Qualquer mudança em `src/model.rs` ou `src/config.rs` que altere o que está em `%APPDATA%\todo-overlay\tasks.json` / `config.json` de forma **incompatível**. Adicionar um campo opcional com `serde(default)` **não** precisa de migração.

## Passo a passo
1. **Guardar uma amostra do formato atual** antes de mexer: crie `tests/fixtures/tasks_v<N>.json` com um exemplo realista do formato vigente (versão `N`).
2. **Subir a versão**: o arquivo tem `"version": N` no topo. Incremente `VERSAO_ATUAL` em `src/storage.rs` para `N+1`.
3. **Migração em `src/storage.rs`** operando sobre `serde_json::Value` (não sobre a struct nova), uma função por passo:
   ```rust
   fn migrate_v1_to_v2(mut v: serde_json::Value) -> anyhow::Result<serde_json::Value> { /* ... */ }
   ```
   O `carregar` lê o `Value`, aplica as migrações em cadeia de `version` até `VERSAO_ATUAL` e só então desserializa para a struct.
4. **Backup antes de gravar**: ao migrar, copie o arquivo original para `tasks.v<N>.bak` antes de salvar o novo formato.
5. **Versão futura** (arquivo com `version` > `VERSAO_ATUAL`): não sobrescrever; abrir em modo somente leitura ou avisar o usuário.
6. **Testes** em `src/storage.rs`:
   - fixture `v<N>` carrega e vira o formato atual sem perder nenhuma tarefa (texto, concluída, ordem);
   - arquivo sem `version` é tratado como v1;
   - JSON corrompido → renomeia para `.bak` e retorna lista vazia, sem panic.
   Delegue ao agent `test-writer` se houver muitas combinações.
7. Validar: `cargo fmt; cargo clippy --all-targets -- -D warnings; cargo test`.
8. Teste manual: copie um `tasks.json` antigo real para `%APPDATA%\todo-overlay\`, rode `cargo run` e confira a lista.
9. Commit: `feat!: migra tasks.json para v<N+1>` ou `refactor:` conforme o caso.

## Convenções deste projeto
- Gravação sempre atômica: escreve `tasks.json.tmp` e renomeia por cima.
- Migrações nunca são removidas do código — um usuário pode pular várias versões.

## Checklist final
- [ ] Fixture da versão antiga em `tests/fixtures/`
- [ ] `VERSAO_ATUAL` incrementado e migração encadeada
- [ ] Backup `.bak` antes de gravar o formato novo
- [ ] Testes de migração, sem-versão e corrompido passando
