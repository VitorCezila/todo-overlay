//! Atalho global para mostrar/esconder (padrão `Ctrl+Alt+T`) e a thread de
//! fundo que também hospeda o ícone da bandeja. Marco 3.
//!
//! Com a janela invisível o eframe não pinta nem chama `ui`, então o atalho é
//! tratado numa thread própria que chama `ShowWindow` (Win32) e depois
//! `ctx.request_repaint()`. A thread fica bloqueada em `GetMessageW` — 0% de
//! CPU sem polling — e os handlers do `global-hotkey` e do `tray-icon` rodam
//! dentro do `DispatchMessageW` dela.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread::JoinHandle;

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, IsWindowVisible, MSG, PostThreadMessageW, SW_HIDE, SW_SHOW,
    SetForegroundWindow, ShowWindowAsync, TranslateMessage, WM_QUIT,
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

/// Thread de fundo (atalho + bandeja). Ao ser dropada encerra a thread, o que
/// remove o ícone da bandeja e libera o atalho.
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
}

/// O que a thread devolve depois de montar atalho e bandeja.
struct Pronto {
    thread_id: u32,
    avisos: Vec<String>,
    bandeja: bool,
}

pub fn iniciar(janela: Janela, ctx: egui::Context, saindo: Arc<AtomicBool>) -> Fundo {
    let (tx, rx) = mpsc::sync_channel::<Pronto>(1);
    let spawn = std::thread::Builder::new()
        .name("atalho-bandeja".into())
        .spawn(move || executar(janela, ctx, saindo, tx));
    let handle = match spawn {
        Ok(h) => h,
        Err(e) => {
            return Fundo {
                thread_id: 0,
                handle: None,
                avisos: vec![format!("Atalho e bandeja indisponíveis: {e}")],
                bandeja: false,
            };
        }
    };
    match rx.recv() {
        Ok(p) => Fundo {
            thread_id: p.thread_id,
            handle: Some(handle),
            avisos: p.avisos,
            bandeja: p.bandeja,
        },
        // A thread morreu antes de responder (pânico na inicialização).
        Err(_) => Fundo {
            thread_id: 0,
            handle: None,
            avisos: vec!["Atalho e bandeja indisponíveis (falha ao iniciar).".into()],
            bandeja: false,
        },
    }
}

fn executar(
    janela: Janela,
    ctx: egui::Context,
    saindo: Arc<AtomicBool>,
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

    let bandeja = icone.is_some();
    let enviado = tx.send(Pronto {
        thread_id,
        avisos,
        bandeja,
    });
    // Sem atalho nem bandeja não há mensagem para esperar: encerra já.
    if enviado.is_err() || (atalho.is_none() && icone.is_none()) {
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
