---
name: nova-config
description: Use quando o usuário quiser tornar algo configurável no todo-overlay (ex. "quero mudar a opacidade", "deixa eu escolher o atalho", "tamanho da fonte") — adiciona o campo em Config, valor padrão, controle na tela de configurações e persistência em config.json.
---

# Nova opção de configuração

## Quando usar
Para qualquer preferência do usuário que deva sobreviver a reinícios: opacidade, atalho, fonte, canto da tela, autostart, cores etc.

## Passo a passo
1. **Campo em `src/config.rs`** na struct `Config`, com `#[serde(default = "...")]` (ou `#[serde(default)]` na struct) para que um `config.json` antigo sem o campo continue carregando:
   ```rust
   #[serde(default = "default_opacity")]
   pub opacity: f32, // 0.2..=1.0
   fn default_opacity() -> f32 { 0.85 }
   ```
2. **Validar/limitar** o valor ao carregar (ex.: `opacity.clamp(0.2, 1.0)`) — nunca confie no arquivo.
3. **Aplicar o efeito** no lugar certo:
   - visual → `app.rs` (ex.: alpha do `Frame`/`Visuals`);
   - atalho → `hotkey.rs` (desregistrar o antigo, registrar o novo; se falhar, manter o antigo e avisar);
   - autostart → `autostart.rs`.
4. **Controle na UI** de configurações em `app.rs` (slider, checkbox, campo de texto). Aplicar ao vivo quando barato.
5. **Persistir** chamando o save de `config.json` só quando o valor muda (não a cada frame).
6. **Testes** em `config.rs`: (a) JSON sem o campo carrega com o padrão; (b) valor fora da faixa é corrigido; (c) roundtrip serializa/desserializa.
7. Validar: `cargo fmt; cargo clippy --all-targets -- -D warnings; cargo test` e `cargo run`.
8. Commit: `feat: permite configurar <opção>` (sem atribuição ao Claude).

## Convenções deste projeto
- Campo novo **nunca** quebra config antigo → sempre `serde(default)`. Renomear/remover campo é mudança de formato → use `migrar-dados`.
- Nomes de campo em inglês snake_case; rótulo da UI em português.

## Checklist final
- [ ] Padrão sensato definido e documentado no comentário do campo
- [ ] Valor validado ao carregar
- [ ] Config antigo continua carregando (teste)
- [ ] Salva só ao mudar
