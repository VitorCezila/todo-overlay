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

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindowVisible,
    MSG, PostThreadMessageW, RegisterClassW, SW_HIDE, SW_SHOW, SetForegroundWindow,
    ShowWindowAsync, TranslateMessage, WM_ENDSESSION, WM_QUERYENDSESSION, WM_QUIT, WNDCLASSW,
    WS_OVERLAPPED,
};

use crate::tray;

/// Atalho fixo por enquanto; vira configuração no marco 4.
const ATALHO_MODS: Modifiers = Modifiers::CONTROL.union(Modifiers::ALT);
const ATALHO_TECLA: Code = Code::KeyT;
const ATALHO_TEXTO: &str = "Ctrl+Alt+T";

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

thread_local! {
    /// Só é preenchido na thread "atalho-bandeja", onde a `wndproc` roda.
    static SESSAO: RefCell<Option<EstadoSessao>> = const { RefCell::new(None) };
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
    /// O ícone da bandeja existe (logo, há como sair do app).
    pub bandeja: bool,
    /// Pedidos de gravação vindos do logoff/desligamento.
    pub encerramento: Option<Encerramento>,
}

/// O que a thread devolve depois de montar atalho e bandeja.
struct Pronto {
    thread_id: u32,
    avisos: Vec<String>,
    bandeja: bool,
}

pub fn iniciar(janela: Janela, ctx: egui::Context, saindo: Arc<AtomicBool>) -> Fundo {
    let (tx, rx) = mpsc::sync_channel::<Pronto>(1);
    let (encerramento, sessao) = canal_sessao(ctx.clone());
    let spawn = std::thread::Builder::new()
        .name("atalho-bandeja".into())
        .spawn(move || executar(janela, ctx, saindo, sessao, tx));
    let handle = match spawn {
        Ok(h) => h,
        Err(e) => {
            return Fundo {
                thread_id: 0,
                handle: None,
                avisos: vec![format!("Atalho e bandeja indisponíveis: {e}")],
                bandeja: false,
                encerramento: None,
            };
        }
    };
    match rx.recv() {
        Ok(p) => Fundo {
            thread_id: p.thread_id,
            handle: Some(handle),
            avisos: p.avisos,
            bandeja: p.bandeja,
            encerramento: Some(encerramento),
        },
        // A thread morreu antes de responder (pânico na inicialização).
        Err(_) => Fundo {
            thread_id: 0,
            handle: None,
            avisos: vec!["Atalho e bandeja indisponíveis (falha ao iniciar).".into()],
            bandeja: false,
            encerramento: None,
        },
    }
}

fn executar(
    janela: Janela,
    ctx: egui::Context,
    saindo: Arc<AtomicBool>,
    sessao: EstadoSessao,
    tx: mpsc::SyncSender<Pronto>,
) {
    // SAFETY: sem argumentos nem pré-condições.
    let thread_id = unsafe { GetCurrentThreadId() };
    let mut avisos = Vec::new();

    // Os dois precisam viver até o fim do loop: o Drop desregistra o atalho
    // e remove o ícone.
    let atalho = match registrar_atalho(janela, ctx.clone()) {
        Ok(m) => Some(m),
        Err(e) => {
            avisos.push(e);
            None
        }
    };
    let icone = match tray::criar(janela, ctx, saindo) {
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
        bandeja,
    });
    // Sem nada que receba mensagens não há o que esperar: encerra já.
    let nada = atalho.is_none() && icone.is_none() && janela_sessao.is_none();
    if enviado.is_err() || nada {
        destruir_janela_sessao(janela_sessao);
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

fn registrar_atalho(janela: Janela, ctx: egui::Context) -> Result<GlobalHotKeyManager, String> {
    let manager = GlobalHotKeyManager::new()
        .map_err(|e| format!("Atalho {ATALHO_TEXTO} indisponível: {e}"))?;
    let atalho = HotKey::new(Some(ATALHO_MODS), ATALHO_TECLA);
    manager.register(atalho).map_err(|_| {
        format!("Atalho {ATALHO_TEXTO} indisponível (já em uso por outro programa).")
    })?;
    let id = atalho.id();
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        if ev.id == id && ev.state == HotKeyState::Pressed {
            janela.alternar(&ctx);
        }
    }));
    Ok(manager)
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

    #[test]
    fn ui_encerrada_nao_segura_o_desligamento() {
        let (ui, fundo) = canal_sessao(egui::Context::default());
        drop(ui); // como no Drop de Fundo
        let inicio = Instant::now();
        assert!(!fundo.encerrar());
        assert!(inicio.elapsed() < Duration::from_millis(500));
    }
}
