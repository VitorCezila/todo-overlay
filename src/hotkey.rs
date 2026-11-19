//! Atalho global para mostrar/esconder (padrão `Ctrl+Alt+T`) e a thread de
//! fundo que também hospeda o ícone da bandeja. Marco 3.
//!
//! Com a janela invisível o eframe não pinta nem chama `ui`, então o atalho é
//! tratado numa thread própria que chama `ShowWindow` (Win32) e depois
//! `ctx.request_repaint()`. A thread fica bloqueada em `GetMessageW` — 0% de
//! CPU sem polling — e os handlers do `global-hotkey` e do `tray-icon` rodam
//! dentro do `DispatchMessageW` dela.
//!
//! A mesma thread tem uma janela oculta (`todo-overlay-sessao`) que recebe
//! `WM_ENDSESSION` no logoff/desligamento — o winit não trata essa mensagem e
//! o processo seria morto sem `on_exit`. Ela pede à UI que grave o que estiver
//! pendente (ver `Encerramento`) e espera a confirmação antes de devolver.
//!
//! Trocar o atalho (marco 4): o `RegisterHotKey` precisa rodar na thread dona
//! da janela oculta do `GlobalHotKeyManager`, então a UI só posta um pedido
//! (`WM_APP_ATALHO` para a janela de sessão) e lê a resposta depois, sem
//! bloquear (ver `Fundo::trocar_atalho`).

use std::cell::RefCell;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use global_hotkey::hotkey::{HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindowVisible,
    MSG, PostMessageW, PostThreadMessageW, RegisterClassW, SW_HIDE, SW_SHOW, SetForegroundWindow,
    ShowWindowAsync, TranslateMessage, WM_APP, WM_ENDSESSION, WM_QUERYENDSESSION, WM_QUIT,
    WNDCLASSW, WS_OVERLAPPED,
};

use crate::config::texto_atalho;
use crate::tray;

/// Pedido de troca de atalho, postado na janela de sessão.
const WM_APP_ATALHO: u32 = WM_APP + 1;

/// Resposta da thread de fundo a um pedido de troca: o atalho que passou a
/// valer, ou a mensagem de erro (o anterior continua valendo).
pub type ResultadoAtalho = Result<HotKey, String>;

/// Handle da janela principal guardado como inteiro (`HWND` é ponteiro e não
/// é `Send`). Só é convertido de volta na hora da chamada Win32.
#[derive(Clone, Copy, Debug)]
pub struct Janela(pub isize);

impl Janela {
    fn hwnd(self) -> HWND {
        self.0 as HWND
    }

    pub fn visivel(self) -> bool {
        // SAFETY: IsWindowVisible só lê o estado da janela e não tem
        // pré-condições. O HWND está sempre vivo aqui: `Fundo` (dono da
        // thread que chama isto) é dropado antes de o eframe destruir a janela.
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }

    /// Mostra, traz para a frente (para já poder digitar) e acorda o eframe,
    /// que não roda nada enquanto a janela está escondida.
    pub fn mostrar(self, ctx: &egui::Context) {
        // SAFETY: ShowWindowAsync/SetForegroundWindow não têm pré-condições
        // além de um HWND (válido, ver `visivel`). Usamos a versão *Async*
        // porque `ShowWindow` numa janela de outra thread espera a thread da
        // UI responder — e na saída ela fica parada no `join` do `Fundo`:
        // seria deadlock se o atalho fosse apertado nesse instante.
        unsafe {
            ShowWindowAsync(self.hwnd(), SW_SHOW);
            SetForegroundWindow(self.hwnd());
        }
        ctx.request_repaint();
    }

    pub fn esconder(self) {
        // SAFETY: idem `mostrar`; SW_HIDE só altera a visibilidade.
        unsafe {
            ShowWindowAsync(self.hwnd(), SW_HIDE);
        }
    }

    pub fn alternar(self, ctx: &egui::Context) {
        if self.visivel() {
            self.esconder();
        } else {
            self.mostrar(ctx);
        }
    }
}

/// Quanto a janela de sessão espera a UI gravar. O Windows começa a mostrar
/// "este app está impedindo o desligamento" por volta de 5 s.
const ESPERA_GRAVACAO: Duration = Duration::from_secs(3);

/// Lado da UI do pedido de gravação no fim da sessão: `logic` consulta
/// `pedido()` e, depois de gravar, chama `confirmar()`.
pub struct Encerramento {
    pedido: Arc<AtomicBool>,
    ack: mpsc::SyncSender<()>,
}

impl Encerramento {
    /// `true` uma única vez por pedido.
    pub fn pedido(&self) -> bool {
        self.pedido.swap(false, Ordering::SeqCst)
    }

    pub fn confirmar(&self) {
        // Cheio = já há uma confirmação esperando; desconectado = a thread
        // de fundo acabou. Em ambos não há o que fazer.
        let _ = self.ack.try_send(());
    }
}

/// Lado da thread de fundo, usado pela `wndproc` da janela de sessão.
struct EstadoSessao {
    ctx: egui::Context,
    pedido: Arc<AtomicBool>,
    ack: mpsc::Receiver<()>,
}

/// Par ligado: a UI fica com o `Encerramento`, a thread de fundo com o estado.
fn canal_sessao(ctx: egui::Context) -> (Encerramento, EstadoSessao) {
    let pedido = Arc::new(AtomicBool::new(false));
    let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
    let ui = Encerramento {
        pedido: pedido.clone(),
        ack: ack_tx,
    };
    let fundo = EstadoSessao {
        ctx,
        pedido,
        ack: ack_rx,
    };
    (ui, fundo)
}

/// Atalho global, do lado da thread de fundo (dona do `GlobalHotKeyManager`).
struct EstadoAtalho {
    manager: GlobalHotKeyManager,
    atual: Option<HotKey>,
    /// Id do atalho atual (0 = nenhum), lido pelo handler de eventos.
    id: Arc<AtomicU32>,
    pedidos: mpsc::Receiver<HotKey>,
    respostas: mpsc::Sender<ResultadoAtalho>,
    ctx: egui::Context,
}

impl EstadoAtalho {
    /// `WM_APP_ATALHO`: atende só o pedido mais recente e acorda a UI.
    fn atender(&mut self) {
        let Some(novo) = self.pedidos.try_iter().last() else {
            return;
        };
        let r = self.trocar(novo);
        // Desconectado = a UI já foi embora; nada a fazer.
        let _ = self.respostas.send(r);
        self.ctx.request_repaint();
    }

    /// Registra o novo antes de soltar o antigo: se o novo estiver em uso por
    /// outro programa, o antigo continua valendo.
    fn trocar(&mut self, novo: HotKey) -> ResultadoAtalho {
        if self.atual == Some(novo) {
            return Ok(novo);
        }
        self.manager.register(novo).map_err(|e| {
            let texto = texto_atalho(&novo);
            match e {
                global_hotkey::Error::AlreadyRegistered(_) => {
                    format!("Atalho {texto} indisponível (já em uso por outro programa).")
                }
                outro => format!("Atalho {texto} indisponível: {outro}"),
            }
        })?;
        if let Some(antigo) = self.atual.replace(novo) {
            // Falhar aqui só deixaria o antigo registrado sem efeito: o
            // handler já passa a comparar com o id novo.
            let _ = self.manager.unregister(antigo);
        }
        self.id.store(novo.id(), Ordering::SeqCst);
        Ok(novo)
    }
}

thread_local! {
    /// Só é preenchido na thread "atalho-bandeja", onde a `wndproc` roda.
    static SESSAO: RefCell<Option<EstadoSessao>> = const { RefCell::new(None) };
    /// Idem; `None` se o `GlobalHotKeyManager` não pôde ser criado.
    static ATALHO: RefCell<Option<EstadoAtalho>> = const { RefCell::new(None) };
}

/// Thread de fundo (atalho + bandeja + janela de sessão). Ao ser dropada
/// encerra a thread, o que remove o ícone da bandeja e libera o atalho.
///
/// Fica dentro do `OverlayApp`, que o eframe dropa *antes* de destruir a
/// janela (campo `app` vem antes de `glutin` em `GlowWinitRunning`): por isso
/// a thread nunca usa um HWND já destruído ou reaproveitado.
pub struct Fundo {
    thread_id: u32,
    handle: Option<JoinHandle<()>>,
    /// Problemas de inicialização para mostrar na janela.
    pub avisos: Vec<String>,
    /// Problema com o atalho inicial (em uso, gerenciador indisponível);
    /// separado de `avisos` para sumir quando uma troca der certo.
    pub aviso_atalho: Option<String>,
    /// O ícone da bandeja existe (logo, há como sair do app).
    pub bandeja: bool,
    /// Pedidos de gravação vindos do logoff/desligamento.
    pub encerramento: Option<Encerramento>,
    /// Janela de sessão (`HWND` como inteiro; 0 = não existe), destino dos
    /// pedidos de troca de atalho.
    janela_sessao: isize,
    /// `None` se não há como trocar o atalho (gerenciador não criado).
    pedidos_atalho: Option<mpsc::Sender<HotKey>>,
    respostas_atalho: Option<mpsc::Receiver<ResultadoAtalho>>,
    /// Ligado pelo item "Configurações" da bandeja.
    abrir_config: Arc<AtomicBool>,
}

/// O que a thread devolve depois de montar atalho e bandeja.
struct Pronto {
    thread_id: u32,
    avisos: Vec<String>,
    aviso_atalho: Option<String>,
    bandeja: bool,
    atalho: bool,
    janela_sessao: isize,
}

pub fn iniciar(
    janela: Janela,
    ctx: egui::Context,
    saindo: Arc<AtomicBool>,
    atalho: HotKey,
) -> Fundo {
    let (tx, rx) = mpsc::sync_channel::<Pronto>(1);
    let (encerramento, sessao) = canal_sessao(ctx.clone());
    let (pedidos_tx, pedidos_rx) = mpsc::channel::<HotKey>();
    let (respostas_tx, respostas_rx) = mpsc::channel::<ResultadoAtalho>();
    let abrir_config = Arc::new(AtomicBool::new(false));
    let canais = CanaisFundo {
        saindo,
        abrir_config: abrir_config.clone(),
        sessao,
        atalho,
        pedidos: pedidos_rx,
        respostas: respostas_tx,
    };
    let spawn = std::thread::Builder::new()
        .name("atalho-bandeja".into())
        .spawn(move || executar(janela, ctx, canais, tx));
    let indisponivel = |aviso: String| Fundo {
        thread_id: 0,
        handle: None,
        avisos: vec![aviso],
        aviso_atalho: None,
        bandeja: false,
        encerramento: None,
        janela_sessao: 0,
        pedidos_atalho: None,
        respostas_atalho: None,
        abrir_config: Arc::new(AtomicBool::new(false)),
    };
    let handle = match spawn {
        Ok(h) => h,
        Err(e) => return indisponivel(format!("Atalho e bandeja indisponíveis: {e}")),
    };
    match rx.recv() {
        Ok(p) => Fundo {
            thread_id: p.thread_id,
            handle: Some(handle),
            avisos: p.avisos,
            aviso_atalho: p.aviso_atalho,
            bandeja: p.bandeja,
            encerramento: Some(encerramento),
            janela_sessao: p.janela_sessao,
            pedidos_atalho: (p.atalho && p.janela_sessao != 0).then_some(pedidos_tx),
            respostas_atalho: Some(respostas_rx),
            abrir_config,
        },
        // A thread morreu antes de responder (pânico na inicialização).
        Err(_) => indisponivel("Atalho e bandeja indisponíveis (falha ao iniciar).".into()),
    }
}

impl Fundo {
    /// Pede à thread de fundo para trocar o atalho. Não espera: o resultado
    /// chega por [`Fundo::resultado_atalho`] (a thread acorda a UI).
    pub fn trocar_atalho(&self, novo: HotKey) -> Result<(), String> {
        const ERRO: &str = "Troca de atalho indisponível.";
        let tx = self.pedidos_atalho.as_ref().ok_or(ERRO)?;
        tx.send(novo).map_err(|_| ERRO)?;
        // PostMessageW para a janela, e não PostThreadMessageW: mensagem de
        // thread se perde enquanto o menu da bandeja (loop modal) está aberto.
        // SAFETY: só põe uma mensagem na fila; com HWND já destruído a
        // chamada falha e devolve 0 (a janela só é destruída no fim da thread,
        // que acontece no Drop de `Fundo`).
        let ok = unsafe { PostMessageW(self.janela_sessao as HWND, WM_APP_ATALHO, 0, 0) };
        if ok == 0 {
            return Err(ERRO.into());
        }
        Ok(())
    }

    /// Resposta pendente de um [`Fundo::trocar_atalho`], se já chegou.
    pub fn resultado_atalho(&self) -> Option<ResultadoAtalho> {
        match self.respostas_atalho.as_ref()?.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            // A thread de fundo acabou com um pedido pendente: não deixar a
            // tela presa em "Registrando…".
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Troca de atalho indisponível.".into()))
            }
        }
    }

    /// `true` uma vez depois do clique em "Configurações" na bandeja.
    pub fn pedido_config(&self) -> bool {
        self.abrir_config.swap(false, Ordering::SeqCst)
    }
}

/// O que `executar` leva para a thread de fundo (além da janela e do ctx).
struct CanaisFundo {
    saindo: Arc<AtomicBool>,
    abrir_config: Arc<AtomicBool>,
    sessao: EstadoSessao,
    atalho: HotKey,
    pedidos: mpsc::Receiver<HotKey>,
    respostas: mpsc::Sender<ResultadoAtalho>,
}

fn executar(janela: Janela, ctx: egui::Context, c: CanaisFundo, tx: mpsc::SyncSender<Pronto>) {
    // SAFETY: sem argumentos nem pré-condições.
    let thread_id = unsafe { GetCurrentThreadId() };
    let mut avisos = Vec::new();
    let CanaisFundo {
        saindo,
        abrir_config,
        sessao,
        atalho,
        pedidos,
        respostas,
    } = c;

    // Atalho e ícone precisam viver até o fim do loop: o Drop desregistra o
    // atalho e remove o ícone.
    let mut aviso_atalho = None;
    let estado = preparar_atalho(
        janela,
        ctx.clone(),
        atalho,
        pedidos,
        respostas,
        &mut aviso_atalho,
    );
    let tem_atalho = estado.is_some();
    ATALHO.set(estado);
    let icone = match tray::criar(janela, ctx, saindo, abrir_config) {
        Ok(t) => Some(t),
        Err(e) => {
            avisos.push(format!("Ícone na bandeja indisponível: {e:#}"));
            None
        }
    };

    SESSAO.set(Some(sessao));
    let janela_sessao = match criar_janela_sessao() {
        Ok(h) => Some(h),
        Err(e) => {
            avisos.push(e);
            None
        }
    };

    let bandeja = icone.is_some();
    let enviado = tx.send(Pronto {
        thread_id,
        avisos,
        aviso_atalho,
        bandeja,
        atalho: tem_atalho,
        janela_sessao: janela_sessao.map_or(0, |h| h as isize),
    });
    // Sem nada que receba mensagens não há o que esperar: encerra já.
    let nada = !tem_atalho && icone.is_none() && janela_sessao.is_none();
    if enviado.is_err() || nada {
        destruir_janela_sessao(janela_sessao);
        ATALHO.take();
        return;
    }

    // SAFETY: MSG é uma struct POD; zeros é um valor válido para ela.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `msg` é um ponteiro válido e exclusivo durante a chamada;
        // HWND nulo = mensagens de qualquer janela desta thread + de thread.
        let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        // 0 = WM_QUIT; -1 = erro (não há o que fazer além de encerrar).
        if r == 0 || r == -1 {
            break;
        }
        // SAFETY: `msg` acabou de ser preenchida por GetMessageW.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    destruir_janela_sessao(janela_sessao);
    // Desregistra o atalho e destrói a janela do gerenciador nesta thread.
    ATALHO.take();
}

/// Janela top-level invisível só para receber `WM_QUERYENDSESSION` e
/// `WM_ENDSESSION` (janelas `HWND_MESSAGE` não recebem essas mensagens).
fn criar_janela_sessao() -> Result<HWND, String> {
    const ERRO: &str = "Gravação no logoff/desligamento indisponível.";
    let classe: Vec<u16> = "todo-overlay-sessao\0".encode_utf16().collect();
    // SAFETY: nome nulo = módulo do próprio .exe; não há pré-condições.
    let instancia = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: WNDCLASSW é POD; zeros (ponteiros nulos, sem ícone/cursor) é
    // válido, e os campos obrigatórios são preenchidos logo abaixo.
    let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
    wc.lpfnWndProc = Some(wndproc_sessao);
    wc.hInstance = instancia;
    wc.lpszClassName = classe.as_ptr();
    // SAFETY: `wc` e `classe` vivem durante a chamada; o Windows copia o nome.
    // A classe nunca é desregistrada: `iniciar` roda uma vez por processo
    // (`recarregar` reaproveita o `Fundo`).
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err(ERRO.into());
    }
    // SAFETY: classe registrada acima; sem WS_VISIBLE a janela nunca aparece.
    // Pai, menu e parâmetro nulos são permitidos para janela top-level.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            classe.as_ptr(),
            classe.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instancia,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(ERRO.into());
    }
    Ok(hwnd)
}

fn destruir_janela_sessao(hwnd: Option<HWND>) {
    if let Some(h) = hwnd {
        // SAFETY: `h` foi criada por esta mesma thread e ainda não destruída.
        unsafe {
            DestroyWindow(h);
        }
    }
}

/// Chamada só pelo Windows, na thread dona da janela de sessão. Nada aqui
/// pode entrar em pânico: pânico não atravessa `extern "system"` (aborta).
unsafe extern "system" fn wndproc_sessao(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_QUERYENDSESSION => {
            // Já pede a gravação aqui, sem esperar: o Windows pode encerrar o
            // processo assim que *outra* janela nossa (a do winit) devolver o
            // WM_ENDSESSION, antes de a nossa receber o dela.
            SESSAO.with_borrow(|s| {
                if let Some(s) = s {
                    s.consultar();
                }
            });
            1 // pode encerrar
        }
        WM_ENDSESSION => {
            // wParam != 0: a sessão vai mesmo acabar e o processo morre logo
            // depois que devolvermos. Garantir que gravou.
            if wparam != 0 {
                SESSAO.with_borrow(|s| {
                    if let Some(s) = s {
                        s.encerrar();
                    }
                });
            }
            0
        }
        WM_APP_ATALHO => {
            // `try_with`: não entra em pânico se chegar durante a destruição
            // da thread (os thread-locals já podem ter sido liberados).
            // `try_borrow_mut`: numa reentrância (hoje impossível: nada em
            // `atender` bombeia mensagens) ignora em vez de abortar.
            let _ = ATALHO.try_with(|a| {
                if let Ok(mut a) = a.try_borrow_mut()
                    && let Some(a) = a.as_mut()
                {
                    a.atender();
                }
            });
            0
        }
        // SAFETY: repassa sem alterar os argumentos recebidos do Windows.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl EstadoSessao {
    fn pedir(&self) {
        self.pedido.store(true, Ordering::SeqCst);
        self.ctx.request_repaint();
    }

    /// `WM_QUERYENDSESSION`: descarta confirmações antigas e pede a gravação
    /// sem esperar (não atrasa a resposta ao Windows).
    fn consultar(&self) {
        while self.ack.try_recv().is_ok() {}
        self.pedir();
    }

    /// `WM_ENDSESSION`: pede de novo (caso não tenha havido consulta) e espera
    /// uma confirmação, com limite. Se a UI já confirmou o pedido da consulta,
    /// volta na hora (o pedido extra só regrava o que já está gravado, sem
    /// efeito). Não bombeia mensagens
    /// enquanto espera, então não há reentrada em `SESSAO`.
    fn encerrar(&self) -> bool {
        self.pedir();
        self.ack.recv_timeout(ESPERA_GRAVACAO).is_ok()
    }
}

/// Cria o gerenciador e tenta registrar o atalho inicial. Se o registro
/// falhar (atalho em uso), o gerenciador fica mesmo assim, para a tela de
/// configurações poder escolher outro.
fn preparar_atalho(
    janela: Janela,
    ctx: egui::Context,
    inicial: HotKey,
    pedidos: mpsc::Receiver<HotKey>,
    respostas: mpsc::Sender<ResultadoAtalho>,
    aviso: &mut Option<String>,
) -> Option<EstadoAtalho> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            *aviso = Some(format!("Atalho global indisponível: {e}"));
            return None;
        }
    };
    let id = Arc::new(AtomicU32::new(0));
    let id_handler = id.clone();
    let ctx_handler = ctx.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        if ev.state == HotKeyState::Pressed && ev.id == id_handler.load(Ordering::SeqCst) {
            janela.alternar(&ctx_handler);
        }
    }));
    let mut estado = EstadoAtalho {
        manager,
        atual: None,
        id,
        pedidos,
        respostas,
        ctx,
    };
    if let Err(e) = estado.trocar(inicial) {
        *aviso = Some(e);
    }
    Some(estado)
}

/// Atalho a partir de uma tecla apertada na tela de configurações. Exige
/// Ctrl, Alt ou Shift (tecla solta como atalho global atrapalharia o
/// Windows todo); Esc é "cancelar" e não vira atalho. A tecla Win não entra:
/// o egui não a reporta de forma confiável no Windows.
pub fn atalho_capturado(mods: egui::Modifiers, tecla: egui::Key) -> Option<HotKey> {
    use egui::Key;
    let mut m = Modifiers::empty();
    m.set(Modifiers::CONTROL, mods.ctrl);
    m.set(Modifiers::ALT, mods.alt);
    m.set(Modifiers::SHIFT, mods.shift);
    if m.is_empty() {
        return None;
    }
    // Lista fechada: o egui 0.37 também manda `Event::Key` para Ctrl/Shift/
    // Alt sozinhos (ControlLeft...), teclas de mídia, Caps Lock etc., que o
    // parser do global-hotkey aceitaria. Nomes diferentes nos dois crates
    // são traduzidos aqui.
    let nome = match tecla {
        Key::Equals => "Equal",
        Key::OpenBracket => "BracketLeft",
        Key::CloseBracket => "BracketRight",
        Key::Backtick => "Backquote",
        Key::ArrowDown
        | Key::ArrowLeft
        | Key::ArrowRight
        | Key::ArrowUp
        | Key::Insert
        | Key::Delete
        | Key::Home
        | Key::End
        | Key::PageUp
        | Key::PageDown
        | Key::Space
        | Key::Enter
        | Key::Tab
        | Key::Backspace
        | Key::Comma
        | Key::Minus
        | Key::Period
        | Key::Semicolon
        | Key::Quote
        | Key::Slash
        | Key::Backslash => tecla.name(),
        // Letras e dígitos ("A", "5") e F1–F24.
        _ => {
            let nome = tecla.name();
            let funcao = nome
                .strip_prefix('F')
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=24).contains(&n));
            if nome.len() == 1 || funcao {
                nome
            } else {
                return None;
            }
        }
    };
    let codigo = HotKey::from_str(nome).ok()?.key;
    Some(HotKey::new(Some(m), codigo))
}

impl Drop for Fundo {
    fn drop(&mut self) {
        // Solta o lado da UI antes do join: se a thread estiver esperando uma
        // confirmação de fim de sessão, o `recv_timeout` volta na hora
        // (`on_exit` já gravou) em vez de segurar a saída por 3 s.
        drop(self.encerramento.take());
        let Some(handle) = self.handle.take() else {
            return;
        };
        // SAFETY: só posta uma mensagem na fila da thread; com id inválido falha.
        let ok = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        // Sem a mensagem a thread nunca sairia do GetMessageW: não esperar por
        // ela (o processo vai encerrar de qualquer jeito).
        if ok != 0 {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Simula a UI: atende pedidos como o `logic` faz, até `parar`.
    fn ui_falsa(ui: Encerramento, parar: Arc<AtomicBool>) -> std::thread::JoinHandle<usize> {
        std::thread::spawn(move || {
            let mut gravacoes = 0;
            while !parar.load(Ordering::SeqCst) {
                if ui.pedido() {
                    gravacoes += 1;
                    ui.confirmar();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            gravacoes
        })
    }

    #[test]
    fn encerrar_espera_a_ui_gravar() {
        let (ui, fundo) = canal_sessao(egui::Context::default());
        let parar = Arc::new(AtomicBool::new(false));
        let t = ui_falsa(ui, parar.clone());
        let inicio = Instant::now();
        assert!(fundo.encerrar());
        assert!(inicio.elapsed() < ESPERA_GRAVACAO);
        parar.store(true, Ordering::SeqCst);
        assert_eq!(t.join().unwrap(), 1);
    }

    #[test]
    fn consulta_e_encerramento_gravam() {
        let (ui, fundo) = canal_sessao(egui::Context::default());
        let parar = Arc::new(AtomicBool::new(false));
        let t = ui_falsa(ui, parar.clone());
        fundo.consultar();
        assert!(fundo.encerrar());
        parar.store(true, Ordering::SeqCst);
        assert!(t.join().unwrap() >= 1);
    }

    #[test]
    fn consulta_descarta_confirmacao_antiga() {
        let (ui, fundo) = canal_sessao(egui::Context::default());
        ui.confirmar(); // sobra de um desligamento cancelado
        fundo.consultar();
        assert!(fundo.ack.try_recv().is_err());
        assert!(ui.pedido());
        assert!(!ui.pedido(), "o pedido vale uma vez só");
    }

    fn mods(ctrl: bool, alt: bool, shift: bool) -> egui::Modifiers {
        egui::Modifiers {
            ctrl,
            alt,
            shift,
            command: ctrl,
            ..Default::default()
        }
    }

    fn capturado(m: egui::Modifiers, k: egui::Key) -> Option<String> {
        atalho_capturado(m, k).map(|hk| texto_atalho(&hk))
    }

    #[test]
    fn captura_monta_o_atalho() {
        use egui::Key;
        let ca = mods(true, true, false);
        assert_eq!(capturado(ca, Key::T).as_deref(), Some("Ctrl+Alt+T"));
        assert_eq!(capturado(ca, Key::Num5).as_deref(), Some("Ctrl+Alt+5"));
        assert_eq!(capturado(ca, Key::F12).as_deref(), Some("Ctrl+Alt+F12"));
        assert_eq!(
            capturado(ca, Key::ArrowUp).as_deref(),
            Some("Ctrl+Alt+ArrowUp")
        );
        assert_eq!(
            capturado(ca, Key::Equals).as_deref(),
            Some("Ctrl+Alt+Equal")
        );
        assert_eq!(
            capturado(mods(false, false, true), Key::Space).as_deref(),
            Some("Shift+Space")
        );
    }

    #[test]
    fn captura_recusa_sem_modificador_esc_e_teclas_estranhas() {
        use egui::Key;
        assert_eq!(capturado(egui::Modifiers::NONE, Key::T), None);
        assert_eq!(capturado(mods(true, false, false), Key::Escape), None);
        assert_eq!(capturado(mods(true, false, false), Key::Copy), None);
        assert_eq!(capturado(mods(true, false, false), Key::Pipe), None);
        // O egui manda o próprio modificador como tecla ao apertá-lo.
        for k in [
            Key::ControlLeft,
            Key::ControlRight,
            Key::ShiftLeft,
            Key::AltLeft,
            Key::AltRight,
            Key::SuperLeft,
            Key::CapsLock,
            Key::MediaPlayPause,
            Key::F25,
        ] {
            assert_eq!(capturado(mods(true, true, true), k), None, "{k:?}");
        }
    }

    #[test]
    fn captura_volta_pelo_config() {
        // O texto gravado no config.json precisa ser lido de volta igual.
        use egui::Key;
        for k in [
            Key::T,
            Key::Num0,
            Key::F5,
            Key::PageDown,
            Key::Comma,
            Key::Backtick,
        ] {
            let hk = atalho_capturado(mods(true, false, true), k).unwrap();
            assert_eq!(
                crate::config::atalho_de(&texto_atalho(&hk)),
                Some(hk),
                "{k:?}"
            );
        }
    }

    #[test]
    fn ui_encerrada_nao_segura_o_desligamento() {
        let (ui, fundo) = canal_sessao(egui::Context::default());
        drop(ui); // como no Drop de Fundo
        let inicio = Instant::now();
        assert!(!fundo.encerrar());
        assert!(inicio.elapsed() < Duration::from_millis(500));
    }
}
