//! Janela overlay e UI da lista (egui).
//!
//! Janela sem borda, transparente, sempre no topo e fora da taskbar;
//! posição e tamanho são lembrados pelo eframe (`persist_window`, em
//! `%APPDATA%\todo-overlay\data\app.ron`). Enter adiciona, duplo clique
//! edita, a caixa conclui, a lixeira (no hover) apaga e "Desfazer" devolve
//! as últimas apagadas; `tasks.json` só é gravado quando a lista muda.
//! O X e o Alt+F4 só escondem: sair é pela bandeja (ver `logic`).
//!
//! Submódulos: `ajustes` (tela de configurações) e `widgets` (desenhos).

mod ajustes;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, Id, Key, Label, Layout, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit,
    Vec2, ViewportCommand, pos2, vec2,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::hotkey::{self, Fundo, Janela};
use crate::model::{ListaTarefas, Tarefa};
use crate::storage;
use crate::tema::{self, cor};

use ajustes::Ajustes;
use widgets::{Botao, LARGURA_APAGAR, alcas_redimensionar, barra_titulo, botao_lixeira, caixa};

const NOME_APP: &str = "todo-overlay";
const TAMANHO_INICIAL: Vec2 = Vec2::new(320.0, 420.0);
const TAMANHO_MINIMO: Vec2 = Vec2::new(200.0, 120.0);
/// Distância da janela às bordas da área de trabalho na primeira execução.
const MARGEM_CANTO: f32 = 16.0;

pub fn executar() -> anyhow::Result<()> {
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
    app.ajustes = Ajustes::carregar();
    eframe::run_native(
        NOME_APP,
        options,
        Box::new(move |cc| {
            tema::aplicar(&cc.egui_ctx);
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

/// Quantas tarefas apagadas o "Desfazer" lembra (só em memória).
const MAX_DESFAZER: usize = 20;

struct OverlayApp {
    lista: ListaTarefas,
    /// `None` se `%APPDATA%` não foi localizado: o app funciona, mas não grava.
    caminho: Option<PathBuf>,
    /// Não gravar por cima do arquivo (versão futura ou ilegível).
    somente_leitura: bool,
    novo_texto: String,
    editando: Option<Edicao>,
    /// Pilha do "Desfazer": (posição, tarefa) das últimas apagadas. Zera ao
    /// recarregar, porque `normalizar` pode ter renumerado os ids.
    apagadas: Vec<(usize, Tarefa)>,
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
    /// `config.json` e o estado da tela de configurações.
    ajustes: Ajustes,
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
            lista: ListaTarefas::default(),
            caminho: None,
            somente_leitura: false,
            novo_texto: String::new(),
            editando: None,
            apagadas: Vec::new(),
            mensagem: None,
            pendente_salvar: false,
            pode_recarregar: false,
            janela: None,
            saindo: Arc::new(AtomicBool::new(false)),
            fundo: None,
            aviso_fundo: None,
            ajustes: Ajustes::default(),
        };
        match storage::caminho_padrao() {
            Ok(caminho) => {
                match storage::carregar(&caminho) {
                    Ok(c) => {
                        app.lista = c.lista;
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
        let mut fundo = hotkey::iniciar(
            janela,
            cc.egui_ctx.clone(),
            self.saindo.clone(),
            self.ajustes.config.atalho(),
        );
        if !fundo.avisos.is_empty() {
            self.aviso_fundo = Some(fundo.avisos.join("\n"));
        }
        self.ajustes.aviso_atalho = fundo.aviso_atalho.take();
        self.janela = Some(janela);
        self.fundo = Some(fundo);
    }

    /// Recarrega `tasks.json` sem perder a janela, a bandeja, o atalho e as
    /// configurações.
    fn recarregar(&mut self) {
        let mut novo = Self::carregar();
        novo.janela = self.janela;
        novo.saindo = self.saindo.clone();
        novo.fundo = self.fundo.take();
        novo.aviso_fundo = self.aviso_fundo.take();
        novo.ajustes = std::mem::take(&mut self.ajustes);
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
        match storage::salvar(caminho, &self.lista) {
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

    /// Última chance para uma gravação que falhou (ex.: arquivo travado por
    /// antivírus) e para uma edição em andamento: confirma a edição e grava.
    fn gravar_pendencias(&mut self) {
        if let Some(ed) = self.editando.take() {
            self.pendente_salvar |= self.lista.editar(ed.id, &ed.texto);
        }
        if self.pendente_salvar {
            self.salvar();
        }
    }

    /// Troca a lista pela tela de configurações. Uma edição em andamento é
    /// confirmada antes: o TextEdit some com a lista e não perderia o foco.
    fn abrir_config(&mut self) {
        self.gravar_pendencias();
        self.ajustes.abrir();
    }

    /// Edição só é permitida quando dá para gravar: senão o usuário mexeria
    /// numa lista que seria descartada ao fechar.
    fn editavel(&self) -> bool {
        self.caminho.is_some() && !self.somente_leitura
    }

    fn conteudo(&mut self, ui: &mut egui::Ui) {
        let mut mudou = false;
        let editavel = self.editavel();

        let avisos = [
            &self.aviso_fundo,
            &self.ajustes.aviso_atalho,
            &self.mensagem,
        ];
        for msg in avisos.into_iter().flatten() {
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
                    // Id explícito: avisos acima (ex.: o do atalho) aparecem
                    // e somem e mudariam o id automático, tirando o foco.
                    .id(Id::unique("nova-tarefa"))
                    .prefix(RichText::new("❯").color(cor::VERDE))
                    .hint_text("nova tarefa…")
                    .desired_width(f32::INFINITY),
            );
            if campo.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                if self.lista.adicionar(&self.novo_texto).is_some() {
                    mudou = true;
                    self.novo_texto.clear();
                }
                campo.request_focus(); // continua digitando a próxima
            }
            ui.add_space(4.0);

            // Longe do botão de apagar e da barra de rolagem, para não acertar
            // um no lugar do outro.
            let n = self.apagadas.len();
            if n > 0 {
                ui.horizontal(|ui| {
                    if n == 1 {
                        ui.weak("Tarefa apagada");
                    } else {
                        ui.weak(format!("{n} tarefas apagadas"));
                    }
                    if ui.small_button("Desfazer").clicked()
                        && let Some((pos, tarefa)) = self.apagadas.pop()
                    {
                        let voltou = self.lista.restaurar(pos, tarefa);
                        debug_assert!(voltou, "ids são únicos e a pilha zera ao recarregar");
                        mudou |= voltou;
                    }
                });
            }

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.lista.tasks.is_empty() {
                        ui.weak("Nenhuma tarefa ainda");
                    }
                    for tarefa in &self.lista.tasks {
                        linha(ui, tarefa, &mut self.editando, &mut acoes);
                    }
                });
        });

        for acao in acoes {
            mudou |= match acao {
                Acao::Alternar(id) => self.lista.alternar(id),
                Acao::Apagar(id) => match self.lista.tirar(id) {
                    Some(apagada) => {
                        if self.apagadas.len() == MAX_DESFAZER {
                            self.apagadas.remove(0);
                        }
                        self.apagadas.push(apagada);
                        true
                    }
                    None => false,
                },
                Acao::Editar(id, texto) => self.lista.editar(id, &texto),
            };
        }
        if mudou {
            self.salvar();
        }
    }
}

const PREFIXO_ERRO_SALVAR: &str = "Erro ao salvar: ";

/// Uma tarefa: checkbox, texto (duplo clique edita) e botão de apagar no hover.
fn linha(ui: &mut egui::Ui, tarefa: &Tarefa, editando: &mut Option<Edicao>, acoes: &mut Vec<Acao>) {
    // Topo: caixa e lixeira alinhadas à primeira linha de um texto quebrado.
    ui.horizontal_top(|ui| {
        if caixa(ui, tarefa.done).clicked() {
            acoes.push(Acao::Alternar(tarefa.id));
        }

        let espaco = ui.spacing().item_spacing.x;
        let largura_texto = (ui.available_width() - LARGURA_APAGAR - espaco).max(0.0);

        match editando.as_mut().filter(|e| e.id == tarefa.id) {
            Some(ed) => {
                // Id explícito: os automáticos dependem da posição na tela, que
                // muda quando a faixa do "Desfazer" aparece/some ou uma tarefa
                // volta; com outro id a edição perderia o foco sem ser gravada.
                let resp = ui.add(
                    TextEdit::singleline(&mut ed.texto)
                        .id(Id::unique(("editar", tarefa.id)))
                        .desired_width(largura_texto),
                );
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
                let mut texto = RichText::new(&tarefa.text);
                if tarefa.done {
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
                        id: tarefa.id,
                        texto: tarefa.text.clone(),
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
        if botao_lixeira(ui, visivel) {
            acoes.push(Acao::Apagar(tarefa.id));
        }
    });
}

impl eframe::App for OverlayApp {
    /// Roda também com a janela escondida (o eframe faz um passo só de lógica
    /// quando alguém chama `request_repaint`), por isso o Sair da bandeja
    /// funciona sem mostrar a janela.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Logoff/desligamento (ver `hotkey::Encerramento`): o `on_exit` não
        // vai rodar, então grava aqui e libera a thread de fundo.
        let pedido = self
            .fundo
            .as_ref()
            .and_then(|f| f.encerramento.as_ref())
            .is_some_and(|e| e.pedido());
        if pedido {
            self.gravar_pendencias();
            if let Some(e) = self.fundo.as_ref().and_then(|f| f.encerramento.as_ref()) {
                e.confirmar();
            }
        }

        // A thread de fundo acorda o eframe (request_repaint) nos dois casos.
        if let Some(r) = self.fundo.as_ref().and_then(Fundo::resultado_atalho) {
            self.atalho_trocado(r);
        }
        if self.fundo.as_ref().is_some_and(Fundo::pedido_config) {
            self.abrir_config();
        }

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

        let alfa = self.ajustes.config.alfa();
        egui::Frame::new()
            .fill(tema::fundo(alfa))
            .stroke(Stroke::new(1.0, cor::LINHA.gamma_multiply_u8(alfa)))
            .corner_radius(6)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let aberta = self.ajustes.aberta;
                match barra_titulo(ui, self.lista.pendentes(), aberta) {
                    Some(Botao::Fechar) => self.esconder_ou_fechar(ui.ctx()),
                    Some(Botao::Config) if aberta => self.ajustes.fechar(),
                    Some(Botao::Config) => self.abrir_config(),
                    None => {}
                }
                ui.separator();
                if self.ajustes.aberta {
                    // Janela baixa: rola em vez de cortar avisos e botões.
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.tela_config(ui));
                } else {
                    self.conteudo(ui);
                }
            });

        // Por último, para ficar por cima do conteúdo nas bordas.
        alcas_redimensionar(ui, janela);
    }

    /// Ao sair pela bandeja (o logoff/desligamento passa por `logic`).
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.gravar_pendencias();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // fundo totalmente transparente; o Frame desenha o fundo translúcido
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
