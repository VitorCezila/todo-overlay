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

/// Lado do ícone provisório (o `.ico` de verdade chega no marco 5).
const LADO_ICONE: u32 = 32;

pub fn criar(
    janela: Janela,
    ctx: egui::Context,
    saindo: Arc<AtomicBool>,
) -> anyhow::Result<TrayIcon> {
    let alternar = MenuItem::with_id(ID_ALTERNAR, "Mostrar/Esconder", true, None);
    // Tela de configurações é do marco 4.
    let config = MenuItem::with_id(ID_CONFIG, "Configurações", false, None);
    let sair = MenuItem::with_id(ID_SAIR, "Sair", true, None);
    let menu = Menu::with_items(&[&alternar, &config, &PredefinedMenuItem::separator(), &sair])
        .context("falha ao montar o menu")?;

    let icone = Icon::from_rgba(icone_rgba(), LADO_ICONE, LADO_ICONE).context("ícone inválido")?;

    let ctx_menu = ctx.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| match ev.id.0.as_str() {
        ID_ALTERNAR => janela.alternar(&ctx_menu),
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

/// Ícone provisório: quadrado arredondado azul com um ✓ branco, RGBA.
fn icone_rgba() -> Vec<u8> {
    let n = LADO_ICONE as usize;
    let lado = LADO_ICONE as f32;
    let raio = lado * 0.22;
    // Traço do ✓ em coordenadas do ícone.
    let (a, b, c) = (
        (lado * 0.26, lado * 0.52),
        (lado * 0.43, lado * 0.69),
        (lado * 0.75, lado * 0.33),
    );
    let espessura = lado * 0.09;

    let mut px = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let rgba = if !dentro_arredondado(p, lado, raio) {
                [0, 0, 0, 0]
            } else if dist_segmento(p, a, b).min(dist_segmento(p, b, c)) <= espessura {
                [255, 255, 255, 255]
            } else {
                [52, 120, 220, 255]
            };
            px.extend_from_slice(&rgba);
        }
    }
    px
}

fn dentro_arredondado((x, y): (f32, f32), lado: f32, raio: f32) -> bool {
    let cx = x.clamp(raio, lado - raio);
    let cy = y.clamp(raio, lado - raio);
    (x - cx).powi(2) + (y - cy).powi(2) <= raio * raio
}

fn dist_segmento(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icone_tem_tamanho_rgba_certo() {
        assert_eq!(icone_rgba().len(), (LADO_ICONE * LADO_ICONE * 4) as usize);
    }

    #[test]
    fn icone_tem_cantos_transparentes_e_centro_opaco() {
        let px = icone_rgba();
        let alfa = |x: u32, y: u32| px[((y * LADO_ICONE + x) * 4 + 3) as usize];
        assert_eq!(alfa(0, 0), 0);
        assert_eq!(alfa(LADO_ICONE - 1, LADO_ICONE - 1), 0);
        assert_eq!(alfa(LADO_ICONE / 2, LADO_ICONE / 2), 255);
    }
}
