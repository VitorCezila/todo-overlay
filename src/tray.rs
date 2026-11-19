//! Ícone na bandeja: Mostrar/Esconder, Configurações, Sair. Marco 3.
//!
//! Criado na thread de fundo de `hotkey` (o `tray-icon` precisa de uma thread
//! com loop de mensagens Win32). Os handlers só mexem na janela via
//! [`Janela`] e no eframe via `egui::Context` — ambos seguros entre threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context as _;
use eframe::egui::{self, ViewportCommand};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::hotkey::Janela;

const ID_ALTERNAR: &str = "alternar";
const ID_CONFIG: &str = "config";
const ID_SAIR: &str = "sair";

/// Id do `assets/icon.ico` embutido pelo `build.rs` (`winresource` usa 1).
const ID_RECURSO_ICONE: u16 = 1;

pub fn criar(
    janela: Janela,
    ctx: egui::Context,
    saindo: Arc<AtomicBool>,
    abrir_config: Arc<AtomicBool>,
) -> anyhow::Result<TrayIcon> {
    let alternar = MenuItem::with_id(ID_ALTERNAR, "Mostrar/Esconder", true, None);
    let config = MenuItem::with_id(ID_CONFIG, "Configurações", true, None);
    let sair = MenuItem::with_id(ID_SAIR, "Sair", true, None);
    let menu = Menu::with_items(&[&alternar, &config, &PredefinedMenuItem::separator(), &sair])
        .context("falha ao montar o menu")?;

    // Tamanho padrão do sistema; o Windows escolhe a melhor imagem do .ico.
    let icone = Icon::from_resource(ID_RECURSO_ICONE, None)
        .context("ícone não encontrado nos recursos do .exe")?;

    let ctx_menu = ctx.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| match ev.id.0.as_str() {
        ID_ALTERNAR => janela.alternar(&ctx_menu),
        ID_CONFIG => {
            // `logic` vê a flag e troca para a tela de configurações.
            abrir_config.store(true, Ordering::SeqCst);
            janela.mostrar(&ctx_menu);
        }
        ID_SAIR => {
            // `logic` deixa este Close passar (não cancela) e o eframe roda
            // `on_exit`, inclusive com a janela escondida.
            saindo.store(true, Ordering::SeqCst);
            // (send_viewport_cmd já pede repaint e acorda o eframe.)
            ctx_menu.send_viewport_cmd(ViewportCommand::Close);
        }
        _ => {}
    }));

    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = ev
        {
            janela.alternar(&ctx);
        }
    }));

    TrayIconBuilder::new()
        .with_tooltip("todo-overlay")
        .with_icon(icone)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .context("falha ao criar o ícone")
}
