// Sem janela de console no build de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod hotkey;
mod model;
mod storage;
mod tema;
mod tray;

fn main() -> anyhow::Result<()> {
    app::run()
}
