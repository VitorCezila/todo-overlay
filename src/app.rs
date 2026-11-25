//! Janela overlay e UI da lista (egui).
//!
//! Janela sem borda, transparente, sempre no topo e fora da taskbar;
//! posição e tamanho são lembrados pelo eframe (`persist_window`, em
//! `%APPDATA%\todo-overlay\data\app.ron`). Abas separam as categorias
//! (duplo clique ou botão direito renomeia/apaga). Enter adiciona na aba
//! ativa, duplo clique edita, a caixa conclui, a alça ⠿ arrasta para
//! reordenar, a lixeira (no hover) apaga e "Desfazer" devolve as últimas
//! apagadas; o olho da barra esconde as concluídas. `tasks.json` só é gravado
//! quando a lista muda.
//! O X e o Alt+F4 só escondem: sair é pela bandeja (ver `logic`).
//!
//! Submódulos: `ajustes` (tela de configurações) e `widgets` (desenhos).

mod ajustes;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, CursorIcon, Id, Key, Label, Layout, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, TextEdit, Vec2, ViewportCommand, pos2, vec2,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::hotkey::{self, Fundo, Janela};
use crate::model::{GERAL, ListaTarefas, NOME_GERAL, Tarefa};
use crate::storage;
use crate::tema::{self, cor};

use ajustes::Ajustes;
use widgets::{
    Botao, LARGURA_APAGAR, alca_arrastar, alcas_redimensionar, barra_titulo, botao_lixeira, caixa,
};

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
    /// Categoria da aba ativa (só em memória; começa na "Geral").
    aba: u64,
    /// Aba sendo renomeada (o `id` da [`Edicao`] é o da categoria).
    editando_aba: Option<Edicao>,
    /// Tarefa sendo arrastada pela alça ⠿.
    arrastando: Option<u64>,
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
    /// Tarefa, alvo e "depois do alvo" (ver [`ListaTarefas::mover`]).
    Mover(u64, u64, bool),
}

enum AcaoAba {
    Selecionar(u64),
    Criar,
    ComecarRenomear(u64),
    Renomear(u64, String),
    Apagar(u64),
}

/// Distância da borda da lista em que o arrasto passa a rolar.
const MARGEM_ROLAGEM: f32 = 24.0;

impl OverlayApp {
    fn carregar() -> Self {
        let mut app = Self {
            lista: ListaTarefas::default(),
            caminho: None,
            somente_leitura: false,
            novo_texto: String::new(),
            editando: None,
            aba: GERAL,
            editando_aba: None,
            arrastando: None,
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
        if let Some(ed) = self.editando_aba.take() {
            self.pendente_salvar |= self.lista.renomear_categoria(ed.id, &ed.texto);
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

        mudou |= self.abas(ui, editavel);

        let mut acoes = Vec::new();
        let mut aba_desfeita = None;
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
                if self.lista.adicionar(&self.novo_texto, self.aba).is_some() {
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
                        let id = tarefa.id;
                        let voltou = self.lista.restaurar(pos, tarefa);
                        debug_assert!(voltou, "ids são únicos e a pilha zera ao recarregar");
                        mudou |= voltou;
                        // Mostra a aba onde ela voltou, senão o desfazer parece não ter feito nada.
                        aba_desfeita = self
                            .lista
                            .tasks
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| t.categoria);
                    }
                });
            }

            let esconder = self.ajustes.config.esconder_concluidas;
            let aba = self.aba;
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut linhas: Vec<(u64, Rect)> = Vec::new();
                    let mut ocultas = 0;
                    let mut soltou = false;
                    for tarefa in self.lista.tasks.iter().filter(|t| t.categoria == aba) {
                        // A tarefa em edição fica visível: escondida, o TextEdit
                        // nunca perderia o foco e a edição ficaria sem gravar.
                        let em_edicao = self.editando.as_ref().is_some_and(|e| e.id == tarefa.id);
                        if esconder && tarefa.done && !em_edicao {
                            ocultas += 1;
                            continue;
                        }
                        let arrastada = self.arrastando == Some(tarefa.id);
                        let (rect, alca) =
                            linha(ui, tarefa, arrastada, &mut self.editando, &mut acoes);
                        if alca.drag_started() {
                            self.arrastando = Some(tarefa.id);
                        }
                        soltou |= alca.drag_stopped();
                        linhas.push((tarefa.id, rect));
                    }
                    if linhas.is_empty() {
                        ui.weak(if ocultas > 0 {
                            "Nenhuma tarefa pendente"
                        } else {
                            "Nenhuma tarefa ainda"
                        });
                    }
                    if ocultas > 0 {
                        ui.add_space(2.0);
                        ui.weak(if ocultas == 1 {
                            "1 concluída escondida".to_owned()
                        } else {
                            format!("{ocultas} concluídas escondidas")
                        });
                    }
                    if let Some(id) = self.arrastando
                        && let Arrasto::Soltou(alvo) =
                            soltar_ou_mostrar_destino(ui, id, &linhas, soltou)
                    {
                        if let Some((alvo, depois)) = alvo {
                            acoes.push(Acao::Mover(id, alvo, depois));
                        }
                        self.arrastando = None;
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
                Acao::Mover(id, alvo, depois) => self.lista.mover(id, alvo, depois),
            };
        }
        if let Some(aba) = aba_desfeita {
            self.trocar_aba(aba);
        }
        if mudou {
            self.salvar();
        }
    }
}

impl OverlayApp {
    /// Barra de abas: "Geral" fixa, as criadas pelo usuário e o "+".
    /// Devolve `true` se a lista mudou (aba criada, renomeada ou apagada).
    fn abas(&mut self, ui: &mut egui::Ui, editavel: bool) -> bool {
        let mut acoes = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let geral = std::iter::once((GERAL, NOME_GERAL));
            let criadas = self
                .lista
                .categorias()
                .iter()
                .map(|c| (c.id, c.nome.as_str()));
            for (id, nome) in geral.chain(criadas) {
                if let Some(ed) = self.editando_aba.as_mut().filter(|e| e.id == id) {
                    // Id explícito, como na edição de tarefa: o automático
                    // mudaria com a largura das abas e tiraria o foco.
                    let resp = ui.add(
                        TextEdit::singleline(&mut ed.texto)
                            .id(Id::unique(("renomear-aba", id)))
                            .hint_text(nome)
                            .desired_width(90.0),
                    );
                    if ed.focar {
                        resp.request_focus();
                        ed.focar = false;
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.editando_aba = None;
                    } else if resp.lost_focus()
                        && let Some(ed) = self.editando_aba.take()
                    {
                        acoes.push(AcaoAba::Renomear(ed.id, ed.texto));
                    }
                    continue;
                }
                let resp = ui.selectable_label(self.aba == id, nome);
                if resp.clicked() {
                    acoes.push(AcaoAba::Selecionar(id));
                }
                if id == GERAL || !editavel {
                    continue;
                }
                if resp.double_clicked() {
                    acoes.push(AcaoAba::ComecarRenomear(id));
                }
                resp.context_menu(|ui| {
                    if ui.button("Renomear").clicked() {
                        acoes.push(AcaoAba::ComecarRenomear(id));
                        ui.close();
                    }
                    if ui
                        .button("Apagar aba")
                        .on_hover_text("As tarefas dela vão para Geral")
                        .clicked()
                    {
                        acoes.push(AcaoAba::Apagar(id));
                        ui.close();
                    }
                });
            }
            if editavel && ui.small_button("+").on_hover_text("Nova aba").clicked() {
                acoes.push(AcaoAba::Criar);
            }
        });
        ui.add_space(4.0);

        let mut mudou = false;
        for acao in acoes {
            match acao {
                AcaoAba::Selecionar(id) => self.trocar_aba(id),
                AcaoAba::Criar => {
                    mudou |= self.confirmar_renomear_aba();
                    if let Some(id) = self.lista.criar_categoria("Nova aba") {
                        mudou = true;
                        self.trocar_aba(id);
                        self.editando_aba = Some(Edicao {
                            id,
                            texto: String::new(),
                            focar: true,
                        });
                    }
                }
                AcaoAba::ComecarRenomear(id) => {
                    mudou |= self.confirmar_renomear_aba();
                    let texto = self.lista.nome_categoria(id).unwrap_or_default().to_owned();
                    self.editando_aba = Some(Edicao {
                        id,
                        texto,
                        focar: true,
                    });
                }
                AcaoAba::Renomear(id, nome) => mudou |= self.lista.renomear_categoria(id, &nome),
                AcaoAba::Apagar(id) => {
                    if self.lista.apagar_categoria(id) {
                        mudou = true;
                        if self.aba == id {
                            self.trocar_aba(GERAL);
                        }
                    }
                }
            }
        }
        mudou
    }

    /// Grava uma renomeação de aba em andamento (antes de começar outra, que
    /// tomaria o lugar dela antes de o TextEdit avisar a perda de foco).
    fn confirmar_renomear_aba(&mut self) -> bool {
        self.editando_aba
            .take()
            .is_some_and(|ed| self.lista.renomear_categoria(ed.id, &ed.texto))
    }

    /// Uma edição de tarefa em andamento é confirmada antes: a linha some
    /// com a troca de aba e o TextEdit não perderia o foco.
    fn trocar_aba(&mut self, id: u64) {
        if self.aba == id {
            return;
        }
        if let Some(ed) = self.editando.take()
            && self.lista.editar(ed.id, &ed.texto)
        {
            self.salvar();
        }
        self.arrastando = None;
        self.aba = id;
    }
}

enum Arrasto {
    Continua,
    /// Soltou; `None` se não há onde soltar.
    Soltou(Option<(u64, bool)>),
}

/// Durante o arrasto: desenha a linha onde a tarefa vai cair e rola a lista
/// perto das bordas. Ao soltar, devolve o alvo para [`ListaTarefas::mover`].
fn soltar_ou_mostrar_destino(
    ui: &mut egui::Ui,
    id: u64,
    linhas: &[(u64, Rect)],
    soltou: bool,
) -> Arrasto {
    // A linha arrastada pode ter sumido (ex.: concluída com o filtro ligado)
    // sem o egui avisar o fim do arrasto.
    let segurando = ui.input(|i| i.pointer.primary_down());
    let presente = linhas.iter().any(|(l, _)| *l == id);
    let Some(p) = ui.ctx().pointer_interact_pos().filter(|_| presente) else {
        return Arrasto::Soltou(None);
    };
    // Primeira linha cujo meio está abaixo do ponteiro → antes dela; senão,
    // depois da última.
    let alvo = match linhas.iter().find(|(_, r)| p.y < r.center().y) {
        Some(&(alvo, r)) => Some((alvo, false, r.top())),
        None => linhas.last().map(|&(alvo, r)| (alvo, true, r.bottom())),
    };
    if soltou {
        return Arrasto::Soltou(alvo.map(|(a, depois, _)| (a, depois)));
    }
    // Botão solto sem o egui ver o fim do arrasto (janela escondida no meio)
    // ou Esc: cancela, em vez de soltar onde o ponteiro estiver.
    if !segurando || ui.input(|i| i.key_pressed(Key::Escape)) {
        return Arrasto::Soltou(None);
    }

    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    if let Some((_, depois, y)) = alvo {
        // No meio do espaço entre as linhas.
        let meio = ui.spacing().item_spacing.y / 2.0;
        let y = if depois { y + meio } else { y - meio };
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(2.0, cor::AZUL));
    }
    let visivel = ui.clip_rect();
    let passo = if p.y < visivel.top() + MARGEM_ROLAGEM {
        8.0
    } else if p.y > visivel.bottom() - MARGEM_ROLAGEM {
        -8.0
    } else {
        0.0
    };
    if passo != 0.0 {
        ui.scroll_with_delta(vec2(0.0, passo));
        // Só enquanto o botão está pressionado perto da borda.
        ui.ctx().request_repaint();
    }
    Arrasto::Continua
}

const PREFIXO_ERRO_SALVAR: &str = "Erro ao salvar: ";

/// Uma tarefa: alça de arrastar, checkbox, texto (duplo clique edita) e
/// botão de apagar no hover. Devolve a área da linha e a resposta da alça.
fn linha(
    ui: &mut egui::Ui,
    tarefa: &Tarefa,
    arrastada: bool,
    editando: &mut Option<Edicao>,
    acoes: &mut Vec<Acao>,
) -> (Rect, egui::Response) {
    let id_alca = Id::unique(("arrastar", tarefa.id));
    // O hover da linha inteira só se sabe no fim dela, depois da alça: usa
    // a área da linha no frame anterior, guardada pelo id da alça.
    let area_antes: Option<Rect> = ui.data(|d| d.get_temp(id_alca));
    let hover_antes = area_antes.is_some_and(|r| ui.rect_contains_pointer(r));
    // Topo: alça, caixa e lixeira alinhadas à primeira linha de um texto quebrado.
    let r = ui.horizontal_top(|ui| {
        let alca = alca_arrastar(ui, id_alca, arrastada || hover_antes);
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
        let visivel = ui.rect_contains_pointer(area) && !arrastada;
        if botao_lixeira(ui, visivel) {
            acoes.push(Acao::Apagar(tarefa.id));
        }
        alca
    });
    let area = r.response.rect;
    if area_antes != Some(area) {
        ui.data_mut(|d| d.insert_temp(id_alca, area));
    }
    (area, r.inner)
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
                let esconder = self.ajustes.config.esconder_concluidas;
                match barra_titulo(ui, self.lista.pendentes(), aberta, esconder) {
                    Some(Botao::Fechar) => self.esconder_ou_fechar(ui.ctx()),
                    Some(Botao::Filtro) => {
                        self.ajustes.config.esconder_concluidas = !esconder;
                        self.ajustes.salvar();
                    }
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
