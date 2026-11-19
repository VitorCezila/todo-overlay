//! Janela overlay e UI da lista (egui).
//!
//! Marco 1: janela sem borda, transparente, sempre no topo, fora da taskbar,
//! arrastável pela barra de título e redimensionável pelas bordas. Posição e
//! tamanho são lembrados pelo eframe (`persist_window`, em
//! `%APPDATA%\todo-overlay\data\app.ron`).
//! Marco 2: UI para adicionar/editar/concluir/apagar tarefas.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, PointerButton, Pos2, Rect, ResizeDirection,
    Sense, Stroke, Vec2, ViewportCommand, pos2, vec2,
};

const NOME_APP: &str = "todo-overlay";
const TAMANHO_INICIAL: Vec2 = Vec2::new(320.0, 420.0);
const TAMANHO_MINIMO: Vec2 = Vec2::new(200.0, 120.0);
/// Distância da janela às bordas da área de trabalho na primeira execução.
const MARGEM_CANTO: f32 = 16.0;
/// Opacidade do fundo (0–255). Vira configuração no marco 4.
const OPACIDADE: u8 = 200;
const ALTURA_BARRA: f32 = 24.0;
/// Espessura das alças invisíveis de redimensionamento nas bordas.
const BORDA_RESIZE: f32 = 6.0;

pub fn run() -> anyhow::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(NOME_APP)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(true)
        .with_inner_size(TAMANHO_INICIAL)
        .with_min_inner_size(TAMANHO_MINIMO);

    // Só vale na primeira execução: havendo estado salvo, o eframe o aplica por cima.
    if let Some(area) = area_de_trabalho() {
        viewport = viewport.with_position(posicao_canto(area, TAMANHO_INICIAL));
    }

    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(NOME_APP, options, Box::new(|_cc| Ok(Box::new(OverlayApp))))
        .map_err(|e| anyhow::anyhow!("falha ao abrir a janela: {e}"))
}

/// Área de trabalho do monitor principal (sem a taskbar) em pontos lógicos:
/// `[esquerda, topo, direita, base]`.
///
/// Divide pelo DPI do sistema para não depender de o processo já ser DPI-aware
/// (sem awareness o Windows devolve 96 e coordenadas já escaladas; com
/// awareness, o DPI real e pixels físicos — o resultado é o mesmo).
fn area_de_trabalho() -> Option<[f32; 4]> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW};

    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: SPI_GETWORKAREA escreve um RECT em `pvparam`; passamos um ponteiro
    // válido e exclusivo para um RECT local que vive durante toda a chamada.
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut r).cast(), 0) };
    if ok == 0 {
        return None;
    }
    // SAFETY: GetDpiForSystem não recebe argumentos nem tem pré-condições.
    let dpi = unsafe { GetDpiForSystem() };
    let escala = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    Some([r.left, r.top, r.right, r.bottom].map(|v| v as f32 / escala))
}

/// Posição do canto superior direito da área, com margem, sem sair pela esquerda/topo.
fn posicao_canto(area: [f32; 4], tamanho: Vec2) -> Pos2 {
    let [esquerda, topo, direita, _] = area;
    let x = (direita - tamanho.x - MARGEM_CANTO).max(esquerda);
    pos2(x, topo + MARGEM_CANTO)
}

struct OverlayApp;

impl eframe::App for OverlayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let janela = ui.max_rect();

        egui::Frame::new()
            .fill(Color32::from_black_alpha(OPACIDADE))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                barra_titulo(ui);
                ui.separator();
                ui.weak("Nenhuma tarefa ainda");
            });

        // Por último, para ficar por cima do conteúdo nas bordas.
        alcas_redimensionar(ui, janela);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // fundo totalmente transparente; o Frame desenha o fundo translúcido
    }
}

/// Barra própria: arrastar move a janela; o botão fecha o app.
fn barra_titulo(ui: &mut egui::Ui) {
    let (barra, arrasto) =
        ui.allocate_exact_size(vec2(ui.available_width(), ALTURA_BARRA), Sense::drag());
    if arrasto.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    ui.painter().text(
        barra.left_center(),
        Align2::LEFT_CENTER,
        "Tarefas",
        FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
    );

    let botao = Rect::from_min_size(
        barra.right_top() - vec2(ALTURA_BARRA, 0.0),
        Vec2::splat(ALTURA_BARRA),
    );
    let fechar = ui
        .interact(botao, Id::unique("fechar"), Sense::click())
        .on_hover_text("Fechar");
    let cor = if fechar.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let x = botao.shrink(7.0);
    let traco = Stroke::new(1.5, cor);
    ui.painter()
        .line_segment([x.left_top(), x.right_bottom()], traco);
    ui.painter()
        .line_segment([x.right_top(), x.left_bottom()], traco);
    if fechar.clicked() {
        // TODO(marco 3): com a bandeja, esconder em vez de encerrar o processo.
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
}

/// Alças invisíveis nas 4 bordas e 4 cantos (a janela sem borda não tem resize nativo).
fn alcas_redimensionar(ui: &egui::Ui, r: Rect) {
    let b = BORDA_RESIZE;
    let (l, t, rt, bt) = (r.left(), r.top(), r.right(), r.bottom());
    let alcas = [
        (
            ResizeDirection::North,
            Rect::from_min_max(pos2(l + b, t), pos2(rt - b, t + b)),
            CursorIcon::ResizeNorth,
        ),
        (
            ResizeDirection::South,
            Rect::from_min_max(pos2(l + b, bt - b), pos2(rt - b, bt)),
            CursorIcon::ResizeSouth,
        ),
        (
            ResizeDirection::West,
            Rect::from_min_max(pos2(l, t + b), pos2(l + b, bt - b)),
            CursorIcon::ResizeWest,
        ),
        (
            ResizeDirection::East,
            Rect::from_min_max(pos2(rt - b, t + b), pos2(rt, bt - b)),
            CursorIcon::ResizeEast,
        ),
        (
            ResizeDirection::NorthWest,
            Rect::from_min_max(pos2(l, t), pos2(l + b, t + b)),
            CursorIcon::ResizeNorthWest,
        ),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_max(pos2(rt - b, t), pos2(rt, t + b)),
            CursorIcon::ResizeNorthEast,
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_max(pos2(l, bt - b), pos2(l + b, bt)),
            CursorIcon::ResizeSouthWest,
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_max(pos2(rt - b, bt - b), pos2(rt, bt)),
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (i, (direcao, rect, cursor)) in alcas.into_iter().enumerate() {
        let resp = ui
            .interact(rect, Id::unique(("alca_resize", i)), Sense::drag())
            .on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direcao));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canto_superior_direito_com_margem() {
        let p = posicao_canto([0.0, 0.0, 1920.0, 1040.0], vec2(320.0, 420.0));
        assert_eq!(p, pos2(1920.0 - 320.0 - MARGEM_CANTO, MARGEM_CANTO));
    }

    #[test]
    fn canto_respeita_offset_da_area() {
        // Taskbar no topo/esquerda desloca a área de trabalho.
        let p = posicao_canto([100.0, 40.0, 1920.0, 1080.0], vec2(320.0, 420.0));
        assert_eq!(p, pos2(1920.0 - 320.0 - MARGEM_CANTO, 40.0 + MARGEM_CANTO));
    }

    #[test]
    fn canto_nao_sai_pela_esquerda_em_tela_estreita() {
        let p = posicao_canto([0.0, 0.0, 300.0, 600.0], vec2(320.0, 420.0));
        assert_eq!(p.x, 0.0);
    }
}
