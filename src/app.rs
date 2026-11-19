//! Janela overlay e UI da lista (egui).
//!
//! Marco 1: janela sem borda, transparente, sempre no topo, fora da taskbar,
//! arrastável pela barra de título e redimensionável pelas bordas. Posição e
//! tamanho são lembrados pelo eframe (`persist_window`, em
//! `%APPDATA%\todo-overlay\data\app.ron`).
//! Marco 2: lista de tarefas — Enter no campo adiciona, duplo clique edita,
//! checkbox conclui, 🗑 (no hover) apaga. Grava `tasks.json` só quando a
//! lista muda (ver `storage`).
//! Marco 3: `Ctrl+Alt+T` (`hotkey`) e o ícone da bandeja (`tray`) mostram/escondem;
//! o X e o Alt+F4 só escondem — sair é pela bandeja (ver `logic`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, Align2, Button, Color32, CursorIcon, FontId, Id, Key, Label, Layout,
    PointerButton, Pos2, Rect, ResizeDirection, RichText, ScrollArea, Sense, Stroke, TextEdit,
    Vec2, ViewportCommand, pos2, vec2,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::hotkey::{self, Fundo, Janela};
use crate::model::{Task, TaskList};
use crate::storage;

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

    let mut app = OverlayApp::carregar();
    eframe::run_native(
        NOME_APP,
        options,
        Box::new(move |cc| {
            app.iniciar_fundo(cc);
            Ok(Box::new(app))
        }),
    )
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

/// Largura reservada para o botão de apagar (sempre reservada, para o texto
/// não "pular" quando o botão aparece no hover).
const LARGURA_APAGAR: f32 = 20.0;

struct OverlayApp {
    lista: TaskList,
    /// `None` se `%APPDATA%` não foi localizado: o app funciona, mas não grava.
    caminho: Option<PathBuf>,
    /// Não gravar por cima do arquivo (versão futura ou ilegível).
    somente_leitura: bool,
    novo_texto: String,
    editando: Option<Edicao>,
    /// Aviso do carregamento ou erro de gravação, mostrado na janela.
    mensagem: Option<String>,
    /// A última gravação falhou: tentar de novo na próxima mudança e ao sair.
    pendente_salvar: bool,
    /// O carregamento falhou por I/O (ex.: arquivo travado pelo OneDrive):
    /// oferecer "Tentar de novo" em vez de travar a sessão.
    pode_recarregar: bool,
    /// Janela principal (Win32), para esconder pelo X/Alt+F4.
    janela: Option<Janela>,
    /// Ligado pelo "Sair" da bandeja: deixa o próximo pedido de fechar passar.
    saindo: Arc<AtomicBool>,
    /// Thread do atalho + bandeja; o Drop (depois de `on_exit`) a encerra.
    fundo: Option<Fundo>,
    /// Avisos do atalho/bandeja (sobrevivem ao "Tentar de novo").
    aviso_fundo: Option<String>,
}

struct Edicao {
    id: u64,
    texto: String,
    /// Pedir foco no primeiro frame da edição.
    focar: bool,
}

enum Acao {
    Alternar(u64),
    Apagar(u64),
    Editar(u64, String),
}

impl OverlayApp {
    fn carregar() -> Self {
        let mut app = Self {
            lista: TaskList::default(),
            caminho: None,
            somente_leitura: false,
            novo_texto: String::new(),
            editando: None,
            mensagem: None,
            pendente_salvar: false,
            pode_recarregar: false,
            janela: None,
            saindo: Arc::new(AtomicBool::new(false)),
            fundo: None,
            aviso_fundo: None,
        };
        match storage::caminho_padrao() {
            Ok(caminho) => {
                match storage::load(&caminho) {
                    Ok(c) => {
                        app.lista = c.list;
                        app.somente_leitura = c.somente_leitura;
                        app.mensagem = c.aviso;
                    }
                    Err(e) => {
                        // Arquivo existe mas não pôde ser lido: nunca sobrescrever.
                        app.somente_leitura = true;
                        app.pode_recarregar = true;
                        app.mensagem = Some(format!("Erro ao carregar: {e:#}"));
                    }
                }
                app.caminho = Some(caminho);
            }
            Err(e) => app.mensagem = Some(format!("Tarefas não serão salvas: {e:#}")),
        }
        app
    }

    /// Sobe a thread do atalho e da bandeja, que precisam do HWND e do contexto.
    fn iniciar_fundo(&mut self, cc: &eframe::CreationContext<'_>) {
        let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => {
                self.aviso_fundo = Some("Atalho e bandeja indisponíveis (janela sem HWND).".into());
                return;
            }
        };
        let janela = Janela(hwnd);
        let fundo = hotkey::iniciar(janela, cc.egui_ctx.clone(), self.saindo.clone());
        if !fundo.avisos.is_empty() {
            self.aviso_fundo = Some(fundo.avisos.join("\n"));
        }
        self.janela = Some(janela);
        self.fundo = Some(fundo);
    }

    /// Recarrega `tasks.json` sem perder a janela, a bandeja e o atalho.
    fn recarregar(&mut self) {
        let mut novo = Self::carregar();
        novo.janela = self.janela;
        novo.saindo = self.saindo.clone();
        novo.fundo = self.fundo.take();
        novo.aviso_fundo = self.aviso_fundo.take();
        *self = novo;
    }

    /// Só dá para esconder em vez de fechar se a bandeja existe (é por ela
    /// que se reabre e se sai).
    fn pode_esconder(&self) -> bool {
        self.janela.is_some() && self.fundo.as_ref().is_some_and(|f| f.bandeja)
    }

    /// X da barra: esconde; sem bandeja, fecha de verdade (senão o app
    /// ficaria invisível sem como voltar nem sair).
    fn esconder_ou_fechar(&self, ctx: &egui::Context) {
        match self.janela {
            Some(j) if self.pode_esconder() => j.esconder(),
            _ => ctx.send_viewport_cmd(ViewportCommand::Close),
        }
    }

    fn salvar(&mut self) {
        let Some(caminho) = &self.caminho else {
            return;
        };
        if self.somente_leitura {
            return;
        }
        self.pendente_salvar = false;
        match storage::save(caminho, &self.lista) {
            // Limpa só um erro de gravação anterior, não avisos do carregamento.
            Ok(()) => {
                if self
                    .mensagem
                    .as_deref()
                    .is_some_and(|m| m.starts_with(PREFIXO_ERRO_SALVAR))
                {
                    self.mensagem = None;
                }
            }
            Err(e) => {
                self.pendente_salvar = true;
                self.mensagem = Some(format!("{PREFIXO_ERRO_SALVAR}{e:#}"));
            }
        }
    }

    /// Edição só é permitida quando dá para gravar: senão o usuário mexeria
    /// numa lista que seria descartada ao fechar.
    fn editavel(&self) -> bool {
        self.caminho.is_some() && !self.somente_leitura
    }

    fn conteudo(&mut self, ui: &mut egui::Ui) {
        let mut mudou = false;
        let editavel = self.editavel();

        for msg in [&self.aviso_fundo, &self.mensagem].into_iter().flatten() {
            ui.colored_label(ui.visuals().warn_fg_color, msg);
        }
        if self.pode_recarregar && ui.button("Tentar de novo").clicked() {
            self.recarregar();
            return;
        }

        let mut acoes = Vec::new();
        ui.add_enabled_ui(editavel, |ui| {
            let campo = ui.add(
                TextEdit::singleline(&mut self.novo_texto)
                    .hint_text("Nova tarefa…")
                    .desired_width(f32::INFINITY),
            );
            if campo.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                if self.lista.add(&self.novo_texto).is_some() {
                    mudou = true;
                    self.novo_texto.clear();
                }
                campo.request_focus(); // continua digitando a próxima
            }
            ui.add_space(4.0);

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.lista.tasks.is_empty() {
                        ui.weak("Nenhuma tarefa ainda");
                    }
                    for task in &self.lista.tasks {
                        linha(ui, task, &mut self.editando, &mut acoes);
                    }
                });
        });

        for acao in acoes {
            mudou |= match acao {
                Acao::Alternar(id) => self.lista.toggle(id),
                Acao::Apagar(id) => self.lista.remove(id),
                Acao::Editar(id, texto) => self.lista.edit(id, &texto),
            };
        }
        if mudou {
            self.salvar();
        }
    }
}

const PREFIXO_ERRO_SALVAR: &str = "Erro ao salvar: ";

/// Uma tarefa: checkbox, texto (duplo clique edita) e botão de apagar no hover.
fn linha(ui: &mut egui::Ui, task: &Task, editando: &mut Option<Edicao>, acoes: &mut Vec<Acao>) {
    ui.horizontal(|ui| {
        let mut done = task.done;
        if ui.checkbox(&mut done, "").changed() {
            acoes.push(Acao::Alternar(task.id));
        }

        let espaco = ui.spacing().item_spacing.x;
        let largura_texto = (ui.available_width() - LARGURA_APAGAR - espaco).max(0.0);

        match editando.as_mut().filter(|e| e.id == task.id) {
            Some(ed) => {
                let resp = ui.add(TextEdit::singleline(&mut ed.texto).desired_width(largura_texto));
                if ed.focar {
                    resp.request_focus();
                    ed.focar = false;
                }
                // Esc também tira o foco; checar antes para cancelar em vez de salvar.
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    *editando = None;
                } else if resp.lost_focus()
                    && let Some(ed) = editando.take()
                {
                    acoes.push(Acao::Editar(ed.id, ed.texto));
                }
            }
            None => {
                let mut texto = RichText::new(&task.text);
                if task.done {
                    texto = texto.strikethrough().weak();
                }
                let resp = ui
                    .allocate_ui_with_layout(
                        vec2(largura_texto, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_width(largura_texto);
                            ui.add(Label::new(texto).wrap().sense(Sense::click()))
                        },
                    )
                    .inner;
                if resp.double_clicked() {
                    *editando = Some(Edicao {
                        id: task.id,
                        texto: task.text.clone(),
                        focar: true,
                    });
                }
            }
        }

        // Área da linha até aqui, estendida até a borda direita.
        let area = Rect::from_min_max(
            ui.min_rect().min,
            pos2(ui.max_rect().right(), ui.min_rect().bottom()),
        );
        let visivel = ui.rect_contains_pointer(area);
        let apagar = ui
            .add_visible(visivel, Button::new("🗑").frame(false))
            .on_hover_text("Apagar");
        if apagar.clicked() {
            acoes.push(Acao::Apagar(task.id));
        }
    });
}

impl eframe::App for OverlayApp {
    /// Roda também com a janela escondida (o eframe faz um passo só de lógica
    /// quando alguém chama `request_repaint`), por isso o Sair da bandeja
    /// funciona sem mostrar a janela.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let fechar = ctx.input(|i| i.viewport().close_requested());
        if fechar && !self.saindo.load(Ordering::SeqCst) && self.pode_esconder() {
            // Alt+F4 (ou outro pedido do sistema): só esconde. Sair é pela bandeja.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if let Some(j) = self.janela {
                j.esconder();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let janela = ui.max_rect();

        egui::Frame::new()
            .fill(Color32::from_black_alpha(OPACIDADE))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if barra_titulo(ui, self.lista.pending_count()) {
                    self.esconder_ou_fechar(ui.ctx());
                }
                ui.separator();
                self.conteudo(ui);
            });

        // Por último, para ficar por cima do conteúdo nas bordas.
        alcas_redimensionar(ui, janela);
    }

    /// Última chance para uma gravação que falhou (ex.: arquivo travado por
    /// antivírus) e para uma edição em andamento ao sair pela bandeja.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(ed) = self.editando.take() {
            self.pendente_salvar |= self.lista.edit(ed.id, &ed.texto);
        }
        if self.pendente_salvar {
            self.salvar();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // fundo totalmente transparente; o Frame desenha o fundo translúcido
    }
}

/// Barra própria: arrastar move a janela. Devolve `true` se o X foi clicado.
fn barra_titulo(ui: &mut egui::Ui, pendentes: usize) -> bool {
    let (barra, arrasto) =
        ui.allocate_exact_size(vec2(ui.available_width(), ALTURA_BARRA), Sense::drag());
    if arrasto.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let titulo = if pendentes == 0 {
        "Tarefas".to_owned()
    } else {
        format!("Tarefas ({pendentes})")
    };
    ui.painter().text(
        barra.left_center(),
        Align2::LEFT_CENTER,
        titulo,
        FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
    );

    let botao = Rect::from_min_size(
        barra.right_top() - vec2(ALTURA_BARRA, 0.0),
        Vec2::splat(ALTURA_BARRA),
    );
    let fechar = ui
        .interact(botao, Id::unique("fechar"), Sense::click())
        .on_hover_text("Esconder");
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
    fechar.clicked()
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
