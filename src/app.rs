//! Janela overlay e UI da lista (egui).
//!
//! Marco 1: janela sem borda, transparente, sempre no topo, fora da taskbar,
//! arrastável pela barra de título e redimensionável pelas bordas. Posição e
//! tamanho são lembrados pelo eframe (`persist_window`, em
//! `%APPDATA%\todo-overlay\data\app.ron`).
//! Marco 2: lista de tarefas — Enter no campo adiciona, duplo clique edita,
//! checkbox conclui, a lixeira (no hover) apaga e "Desfazer" devolve as
//! últimas apagadas. Grava `tasks.json` só quando a lista muda (ver `storage`).
//! Marco 3: `Ctrl+Alt+T` (`hotkey`) e o ícone da bandeja (`tray`) mostram/escondem;
//! o X e o Alt+F4 só escondem — sair é pela bandeja (ver `logic`).
//! Marco 4: a engrenagem da barra (ou "Configurações" na bandeja) troca a
//! lista pela tela de configurações: opacidade, atalho e iniciar com o Windows.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Event, FontId, Id, Key, Label, Layout, PointerButton,
    Pos2, Rect, ResizeDirection, RichText, ScrollArea, Sense, Slider, Stroke, TextEdit, Vec2,
    ViewportCommand, pos2, vec2,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::autostart;
use crate::config::{self, Config};
use crate::hotkey::{self, Fundo, Janela};
use crate::model::{Task, TaskList};
use crate::storage;

const NOME_APP: &str = "todo-overlay";
const TAMANHO_INICIAL: Vec2 = Vec2::new(320.0, 420.0);
const TAMANHO_MINIMO: Vec2 = Vec2::new(200.0, 120.0);
/// Distância da janela às bordas da área de trabalho na primeira execução.
const MARGEM_CANTO: f32 = 16.0;
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
    app.ajustes = Ajustes::carregar();
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
/// Quantas tarefas apagadas o "Desfazer" lembra (só em memória).
const MAX_DESFAZER: usize = 20;

struct OverlayApp {
    lista: TaskList,
    /// `None` se `%APPDATA%` não foi localizado: o app funciona, mas não grava.
    caminho: Option<PathBuf>,
    /// Não gravar por cima do arquivo (versão futura ou ilegível).
    somente_leitura: bool,
    novo_texto: String,
    editando: Option<Edicao>,
    /// Pilha do "Desfazer": (posição, tarefa) das últimas apagadas. Zera ao
    /// recarregar, porque `normalizar` pode ter renumerado os ids.
    apagadas: Vec<(usize, Task)>,
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

#[derive(Default)]
struct Ajustes {
    config: Config,
    /// `None` se `%APPDATA%` não foi localizado: vale só nesta execução.
    caminho: Option<PathBuf>,
    /// Não gravar por cima (versão futura ou ilegível).
    somente_leitura: bool,
    /// Tela de configurações no lugar da lista.
    aberta: bool,
    /// Esperando a combinação de teclas do novo atalho.
    capturando: bool,
    /// Troca de atalho pedida à thread de fundo, sem resposta ainda.
    trocando: bool,
    /// Estado da chave `Run`, lido ao abrir a tela (`None` = erro ao ler).
    autostart: Option<autostart::Estado>,
    /// Último conteúdo gravado/lido de `config.json`: não regravar igual
    /// (soltar o slider sem mudar, setas repetidas, clique no trilho).
    salvo: Option<Config>,
    /// Aviso do carregamento do config ou de erro nesta tela.
    aviso: Option<String>,
    /// Problema com o atalho (em uso etc.), mostrado também na lista.
    aviso_atalho: Option<String>,
}

impl Ajustes {
    fn carregar() -> Self {
        let mut a = Self::default();
        match config::caminho_padrao() {
            Ok(caminho) => {
                match config::load(&caminho) {
                    Ok(c) => {
                        a.salvo = Some(c.config.clone());
                        a.config = c.config;
                        a.somente_leitura = c.somente_leitura;
                        a.aviso = c.aviso;
                    }
                    Err(e) => {
                        a.somente_leitura = true;
                        a.aviso = Some(format!("Erro ao carregar as configurações: {e:#}"));
                    }
                }
                a.caminho = Some(caminho);
            }
            Err(e) => a.aviso = Some(format!("Configurações não serão salvas: {e:#}")),
        }
        a
    }

    /// Chamado só quando um valor muda (nunca por frame).
    fn salvar(&mut self) {
        let Some(caminho) = &self.caminho else {
            return;
        };
        if self.somente_leitura || self.salvo.as_ref() == Some(&self.config) {
            return;
        }
        match config::save(caminho, &self.config) {
            Ok(()) => self.salvo = Some(self.config.clone()),
            Err(e) => self.aviso = Some(format!("Erro ao salvar as configurações: {e:#}")),
        }
    }

    fn abrir(&mut self) {
        self.aberta = true;
        self.capturando = false;
        // Uma leitura do registro por abertura, fora do caminho de cada frame.
        self.autostart = match autostart::estado() {
            Ok(v) => Some(v),
            Err(e) => {
                self.aviso = Some(format!("{e:#}"));
                None
            }
        };
    }

    fn fechar(&mut self) {
        self.aberta = false;
        self.capturando = false;
    }
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

    /// Última chance para uma gravação que falhou (ex.: arquivo travado por
    /// antivírus) e para uma edição em andamento: confirma a edição e grava.
    fn gravar_pendencias(&mut self) {
        if let Some(ed) = self.editando.take() {
            self.pendente_salvar |= self.lista.edit(ed.id, &ed.texto);
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
                        && let Some((pos, task)) = self.apagadas.pop()
                    {
                        let voltou = self.lista.restore(pos, task);
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
                    for task in &self.lista.tasks {
                        linha(ui, task, &mut self.editando, &mut acoes);
                    }
                });
        });

        for acao in acoes {
            mudou |= match acao {
                Acao::Alternar(id) => self.lista.toggle(id),
                Acao::Apagar(id) => match self.lista.take(id) {
                    Some(apagada) => {
                        if self.apagadas.len() == MAX_DESFAZER {
                            self.apagadas.remove(0);
                        }
                        self.apagadas.push(apagada);
                        true
                    }
                    None => false,
                },
                Acao::Editar(id, texto) => self.lista.edit(id, &texto),
            };
        }
        if mudou {
            self.salvar();
        }
    }

    /// Tela de configurações (no lugar da lista).
    fn tela_config(&mut self, ui: &mut egui::Ui) {
        let a = &mut self.ajustes;
        // Janela escondida (atalho, X) ou sem foco: cancela a captura, senão
        // a primeira tecla ao voltar viraria o atalho.
        if a.capturando && !ui.input(|i| i.focused) {
            a.capturando = false;
        }

        ui.label("Opacidade do fundo");
        let resp = ui.add(
            Slider::new(
                &mut a.config.opacity,
                config::OPACIDADE_MIN..=config::OPACIDADE_MAX,
            )
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| {
                let s = s.trim().trim_end_matches('%').trim().replace(',', ".");
                // "80" é 80 %; "0.8" (até 1) já é a fração.
                s.parse::<f64>()
                    .ok()
                    .map(|v| if v > 1.0 { v / 100.0 } else { v })
            }),
        );
        // Aplica ao vivo (o Frame lê a opacidade a cada frame), mas só grava
        // ao soltar o arrasto ou numa mudança pelo teclado.
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            a.salvar();
        }
        ui.add_space(8.0);

        ui.label("Atalho para mostrar/esconder");
        ui.horizontal(|ui| {
            if a.capturando {
                ui.strong("Pressione a combinação…");
                if ui.small_button("Cancelar").clicked() {
                    a.capturando = false;
                }
            } else if a.trocando {
                ui.weak("Registrando…");
            } else {
                ui.strong(&a.config.hotkey);
                if ui.button("Alterar…").clicked() {
                    a.capturando = true;
                    a.aviso = None;
                }
            }
        });
        if a.capturando {
            // Combinação registrada por outro programa (inclusive o atalho
            // atual, que esconde a janela) é interceptada pelo Windows e nem
            // chega aqui: daí a segunda frase.
            ui.weak(
                "Use Ctrl ou Alt com outra tecla. Esc cancela.                  Se nada acontecer, a combinação já está em uso.",
            );
            let tecla = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    // Tecla física: no ABNT2, Ctrl+Alt+Q chega como "/" (AltGr)
                    // na lógica, e o RegisterHotKey usa a posição da letra.
                    Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    } => Some((physical_key.unwrap_or(*key), *modifiers)),
                    _ => None,
                })
            });
            match tecla {
                Some((Key::Escape, _)) => a.capturando = false,
                Some((k, m)) => {
                    // Sem modificador ou tecla não suportada (inclusive o
                    // próprio Ctrl/Alt): continua esperando.
                    if let Some(novo) = hotkey::atalho_capturado(m, k) {
                        if let Some(motivo) = config::recusa(&novo) {
                            a.aviso = Some(format!(
                                "{} não pode ser usado: {motivo}",
                                config::texto_atalho(&novo)
                            ));
                        } else {
                            a.capturando = false;
                            a.aviso = None;
                            match self.fundo.as_ref().map(|f| f.trocar_atalho(novo)) {
                                Some(Ok(())) => a.trocando = true,
                                Some(Err(e)) => a.aviso = Some(e),
                                None => {
                                    a.aviso = Some("Troca de atalho indisponível.".into());
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        // Logo abaixo do atalho (e não no topo): um aviso que aparece acima do
        // slider mudaria o id automático dele no meio da digitação.
        for msg in [&a.aviso, &a.aviso_atalho].into_iter().flatten() {
            ui.colored_label(ui.visuals().warn_fg_color, msg);
        }
        ui.add_space(8.0);

        match a.autostart.clone() {
            Some(estado) => {
                let mut ligado = estado != autostart::Estado::Desligado;
                let mut mudar = None;
                if ui.checkbox(&mut ligado, "Iniciar com o Windows").changed() {
                    mudar = Some(ligado);
                }
                if let autostart::Estado::OutroCaminho(v) = &estado {
                    ui.weak(format!("Aponta para outro programa: {v}"));
                    if ui.small_button("Usar este .exe").clicked() {
                        mudar = Some(true);
                    }
                }
                if let Some(ligar) = mudar {
                    let r = if ligar {
                        autostart::ativar()
                    } else {
                        autostart::desativar()
                    };
                    if let Err(e) = r {
                        a.aviso = Some(format!("{e:#}"));
                    }
                    // Relê: o checkbox mostra o que ficou no registro.
                    a.autostart = autostart::estado().ok();
                }
            }
            None => {
                ui.weak("Iniciar com o Windows: indisponível");
            }
        }
        ui.add_space(12.0);

        if ui.button("Voltar").clicked() {
            a.fechar();
        }
    }

    /// Resposta da thread de fundo a uma troca de atalho.
    fn atalho_trocado(&mut self, r: hotkey::ResultadoAtalho) {
        let a = &mut self.ajustes;
        a.trocando = false;
        match r {
            Ok(hk) => {
                a.aviso_atalho = None;
                let texto = config::texto_atalho(&hk);
                if texto != a.config.hotkey {
                    a.config.hotkey = texto;
                    a.salvar();
                }
            }
            Err(e) => a.aviso = Some(e),
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
                // Id explícito: os automáticos dependem da posição na tela, que
                // muda quando a faixa do "Desfazer" aparece/some ou uma tarefa
                // volta; com outro id a edição perderia o foco sem ser gravada.
                let resp = ui.add(
                    TextEdit::singleline(&mut ed.texto)
                        .id(Id::unique(("editar", task.id)))
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
        if botao_lixeira(ui, visivel) {
            acoes.push(Acao::Apagar(task.id));
        }
    });
}

/// Lixeira desenhada com o painter (o glifo 🗑 não existe nas fontes padrão
/// do egui). Invisível, só reserva o espaço (sem desenho nem dica). Devolve `true`
/// no clique quando visível.
fn botao_lixeira(ui: &mut egui::Ui, visivel: bool) -> bool {
    // Sempre a mesma alocação (visível ou não), para os ids automáticos dos
    // widgets seguintes não mudarem com o hover e tirarem o foco de uma edição.
    let tamanho = vec2(LARGURA_APAGAR, ui.spacing().interact_size.y);
    // Sem FOCUSABLE: uma lixeira (mesmo invisível) não pode ser parada do Tab.
    let (r, resp) = ui.allocate_exact_size(tamanho, Sense::CLICK);
    if !visivel {
        return false;
    }
    let resp = resp.on_hover_text("Apagar");
    let cor = if resp.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let traco = Stroke::new(1.5, cor);
    let p = ui.painter();
    let c = r.center();
    // Ícone de ~12 px: tampa, alça, corpo afunilado e dois frisos.
    let tampa = c.y - 4.0;
    p.line_segment([pos2(c.x - 6.0, tampa), pos2(c.x + 6.0, tampa)], traco);
    p.line_segment(
        [pos2(c.x - 2.0, tampa - 2.0), pos2(c.x + 2.0, tampa - 2.0)],
        traco,
    );
    let (topo, base) = (tampa + 1.5, c.y + 6.0);
    p.add(egui::Shape::closed_line(
        vec![
            pos2(c.x - 4.5, topo),
            pos2(c.x + 4.5, topo),
            pos2(c.x + 3.5, base),
            pos2(c.x - 3.5, base),
        ],
        traco,
    ));
    for dx in [-1.5, 1.5] {
        p.line_segment(
            [pos2(c.x + dx, topo + 2.0), pos2(c.x + dx, base - 2.0)],
            Stroke::new(1.0, cor),
        );
    }
    resp.clicked()
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

        egui::Frame::new()
            .fill(Color32::from_black_alpha(self.ajustes.config.alfa()))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let aberta = self.ajustes.aberta;
                match barra_titulo(ui, self.lista.pending_count(), aberta) {
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

enum Botao {
    Fechar,
    Config,
}

/// Barra própria: arrastar move a janela. Devolve o botão clicado.
fn barra_titulo(ui: &mut egui::Ui, pendentes: usize, config_aberta: bool) -> Option<Botao> {
    let (barra, arrasto) =
        ui.allocate_exact_size(vec2(ui.available_width(), ALTURA_BARRA), Sense::DRAG);
    if arrasto.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let titulo = if config_aberta {
        "Configurações".to_owned()
    } else if pendentes == 0 {
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
        .interact(botao, Id::unique("fechar"), Sense::CLICK)
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

    let engrenagem = botao.translate(vec2(-ALTURA_BARRA, 0.0));
    let dica = if config_aberta {
        "Voltar à lista"
    } else {
        "Configurações"
    };
    // Sense::CLICK (sem FOCUSABLE): não vira parada do Tab.
    let config = ui
        .interact(engrenagem, Id::unique("config"), Sense::CLICK)
        .on_hover_text(dica);
    desenhar_engrenagem(ui, engrenagem.center(), config.hovered() || config_aberta);

    if fechar.clicked() {
        Some(Botao::Fechar)
    } else if config.clicked() {
        Some(Botao::Config)
    } else {
        None
    }
}

/// Engrenagem de ~14 px desenhada com o painter (sem depender de glifo).
fn desenhar_engrenagem(ui: &egui::Ui, c: Pos2, destaque: bool) {
    let cor = if destaque {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let p = ui.painter();
    p.circle_stroke(c, 3.5, Stroke::new(1.5, cor));
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::FRAC_PI_4;
        let dir = vec2(ang.cos(), ang.sin());
        p.line_segment([c + dir * 4.5, c + dir * 7.0], Stroke::new(2.0, cor));
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
