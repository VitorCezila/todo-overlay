//! Atalho global para mostrar/esconder (padrão `Ctrl+Alt+T`). Marco 3.
//!
//! Com a janela invisível o eframe não chama `update`, então o evento do
//! atalho é tratado numa thread própria que chama `ShowWindow` (Win32) e
//! depois `ctx.request_repaint()`.
