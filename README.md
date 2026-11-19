# todo-overlay

Overlay de to-do list para Windows: janelinha semitransparente, sempre no topo, num canto da tela. Um atalho global (`Ctrl+Alt+T`) mostra/esconde. Feito em Rust + egui para ser muito leve.

## Pré-requisitos

1. [rustup](https://rustup.rs) (`winget install Rustlang.Rustup`)
2. Visual Studio Build Tools com a carga "Desenvolvimento para desktop com C++" (o rustup oferece instalar)

## Uso

```powershell
cargo run              # desenvolvimento
cargo build --release  # gera target\release\todo-overlay.exe
cargo test
```

Dados ficam em `%APPDATA%\todo-overlay\`.
