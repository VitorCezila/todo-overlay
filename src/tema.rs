//! Tema visual: paleta One Half Dark e fonte Cascadia Code.
//!
//! A fonte é lida de `C:\Windows\Fonts` (vem com o Windows 11) em vez de
//! embutida, para não engordar o `.exe`; sem ela, ficam as fontes padrão do egui.

use std::path::PathBuf;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
    vec2,
};

/// Paleta One Half Dark (com dois tons intermediários do One Dark para
/// bordas e texto fraco).
pub mod cor {
    use eframe::egui::Color32;

    pub const FUNDO: Color32 = Color32::from_rgb(0x28, 0x2c, 0x34);
    /// Um tom abaixo do fundo: campos de texto e dicas.
    pub const FUNDO_ESCURO: Color32 = Color32::from_rgb(0x21, 0x25, 0x2b);
    /// Um tom acima do fundo: botões e hover.
    pub const FUNDO_CLARO: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3a);
    pub const LINHA: Color32 = Color32::from_rgb(0x3e, 0x44, 0x51);
    pub const SELECAO: Color32 = Color32::from_rgb(0x47, 0x4e, 0x5d);
    pub const TEXTO: Color32 = Color32::from_rgb(0xdc, 0xdf, 0xe4);
    pub const TEXTO_MEDIO: Color32 = Color32::from_rgb(0xab, 0xb2, 0xbf);
    pub const TEXTO_FRACO: Color32 = Color32::from_rgb(0x7f, 0x84, 0x8e);
    pub const CINZA: Color32 = Color32::from_rgb(0x5a, 0x63, 0x74);
    pub const VERMELHO: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x75);
    pub const VERDE: Color32 = Color32::from_rgb(0x98, 0xc3, 0x79);
    pub const AMARELO: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b);
    pub const AZUL: Color32 = Color32::from_rgb(0x61, 0xaf, 0xef);
}

/// Fundo da janela com a opacidade escolhida nas configurações.
pub fn fundo(alfa: u8) -> Color32 {
    let [r, g, b, _] = cor::FUNDO.to_array();
    Color32::from_rgba_unmultiplied(r, g, b, alfa)
}

/// Aplica fonte e cores. Chamado uma vez, na criação da janela.
pub fn aplicar(ctx: &egui::Context) {
    if let Some(bytes) = ler_fonte() {
        ctx.add_font(FontInsert::new(
            "cascadia",
            FontData::from_owned(bytes),
            [FontFamily::Proportional, FontFamily::Monospace]
                .into_iter()
                .map(|family| InsertFontFamily {
                    family,
                    priority: FontPriority::Highest,
                })
                .collect(),
        ));
    }
    // Sempre escuro, independente do tema do Windows.
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, estilo);
}

/// Cascadia Code (ou Mono, se só ela existir), do sistema ou instalada pelo usuário.
fn ler_fonte() -> Option<Vec<u8>> {
    caminhos_fonte()
        .into_iter()
        .find_map(|caminho| std::fs::read(caminho).ok())
}

fn caminhos_fonte() -> Vec<PathBuf> {
    let pastas = [
        std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")),
        std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join(r"Microsoft\Windows\Fonts")),
    ];
    let mut caminhos = Vec::new();
    for arquivo in ["CascadiaCode.ttf", "CascadiaMono.ttf"] {
        for pasta in pastas.iter().flatten() {
            caminhos.push(pasta.join(arquivo));
        }
    }
    caminhos
}

fn estilo(s: &mut egui::Style) {
    use cor::*;

    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::proportional(15.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();

    s.spacing.item_spacing = vec2(8.0, 5.0);
    s.spacing.button_padding = vec2(6.0, 2.0);

    let v = &mut s.visuals;
    v.dark_mode = true;
    v.panel_fill = FUNDO;
    v.window_fill = FUNDO_ESCURO;
    v.window_stroke = Stroke::new(1.0, LINHA);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.extreme_bg_color = FUNDO_ESCURO;
    v.text_edit_bg_color = Some(FUNDO_ESCURO);
    v.faint_bg_color = FUNDO_CLARO;
    v.code_bg_color = FUNDO_CLARO;
    v.weak_text_color = Some(TEXTO_FRACO);
    v.hyperlink_color = AZUL;
    v.warn_fg_color = AMARELO;
    v.error_fg_color = VERMELHO;
    v.selection.bg_fill = SELECAO;
    v.selection.stroke = Stroke::new(1.0, AZUL);
    v.text_cursor.stroke = Stroke::new(2.0, AZUL);
    // Piscar o cursor redesenha a janela sem parar enquanto o campo tem foco.
    v.text_cursor.blink = false;
    v.slider_trailing_fill = true;

    let w = &mut v.widgets;
    let raio = CornerRadius::same(3);
    w.noninteractive.bg_fill = FUNDO;
    w.noninteractive.weak_bg_fill = FUNDO;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINHA); // separadores
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXTO); // texto normal
    w.noninteractive.corner_radius = raio;

    w.inactive.bg_fill = FUNDO_CLARO;
    w.inactive.weak_bg_fill = FUNDO_CLARO;
    w.inactive.bg_stroke = Stroke::new(1.0, LINHA);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXTO_MEDIO);
    w.inactive.corner_radius = raio;

    w.hovered.bg_fill = LINHA;
    w.hovered.weak_bg_fill = LINHA;
    w.hovered.bg_stroke = Stroke::new(1.0, CINZA);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXTO);
    w.hovered.corner_radius = raio;

    w.active.bg_fill = SELECAO;
    w.active.weak_bg_fill = SELECAO;
    w.active.bg_stroke = Stroke::new(1.0, AZUL);
    w.active.fg_stroke = Stroke::new(1.0, TEXTO);
    w.active.corner_radius = raio;

    w.open.bg_fill = FUNDO_CLARO;
    w.open.weak_bg_fill = FUNDO_CLARO;
    w.open.bg_stroke = Stroke::new(1.0, LINHA);
    w.open.fg_stroke = Stroke::new(1.0, TEXTO);
    w.open.corner_radius = raio;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fundo_mantem_a_cor_e_aplica_o_alfa() {
        let c = fundo(128);
        assert_eq!(c.a(), 128);
        assert_eq!(fundo(255), cor::FUNDO);
    }

    #[test]
    fn prefere_cascadia_code_a_mono() {
        let caminhos = caminhos_fonte();
        let primeira_mono = caminhos
            .iter()
            .position(|c| c.ends_with("CascadiaMono.ttf"));
        let ultima_code = caminhos
            .iter()
            .rposition(|c| c.ends_with("CascadiaCode.ttf"));
        if let (Some(m), Some(c)) = (primeira_mono, ultima_code) {
            assert!(c < m);
        }
    }
}
