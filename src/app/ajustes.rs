//! Tela de configurações (opacidade, atalho, iniciar com o Windows) e o
//! estado dela, incluindo o `config.json`.

use std::path::PathBuf;

use eframe::egui::{self, Event, Key, Slider};

use super::OverlayApp;
use crate::autostart;
use crate::config::{self, Config};
use crate::hotkey;

#[derive(Default)]
pub(super) struct Ajustes {
    pub(super) config: Config,
    /// `None` se `%APPDATA%` não foi localizado: vale só nesta execução.
    pub(super) caminho: Option<PathBuf>,
    /// Não gravar por cima (versão futura ou ilegível).
    pub(super) somente_leitura: bool,
    /// Tela de configurações no lugar da lista.
    pub(super) aberta: bool,
    /// Esperando a combinação de teclas do novo atalho.
    pub(super) capturando: bool,
    /// Troca de atalho pedida à thread de fundo, sem resposta ainda.
    pub(super) trocando: bool,
    /// Estado da chave `Run`, lido ao abrir a tela (`None` = erro ao ler).
    pub(super) autostart: Option<autostart::Estado>,
    /// Último conteúdo gravado/lido de `config.json`: não regravar igual
    /// (soltar o slider sem mudar, setas repetidas, clique no trilho).
    pub(super) salvo: Option<Config>,
    /// Aviso do carregamento do config ou de erro nesta tela.
    pub(super) aviso: Option<String>,
    /// Problema com o atalho (em uso etc.), mostrado também na lista.
    pub(super) aviso_atalho: Option<String>,
}

impl Ajustes {
    pub(super) fn carregar() -> Self {
        let mut a = Self::default();
        match config::caminho_padrao() {
            Ok(caminho) => {
                match config::carregar(&caminho) {
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
    pub(super) fn salvar(&mut self) {
        let Some(caminho) = &self.caminho else {
            return;
        };
        if self.somente_leitura || self.salvo.as_ref() == Some(&self.config) {
            return;
        }
        match config::salvar(caminho, &self.config) {
            Ok(()) => self.salvo = Some(self.config.clone()),
            Err(e) => self.aviso = Some(format!("Erro ao salvar as configurações: {e:#}")),
        }
    }

    pub(super) fn abrir(&mut self) {
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

    pub(super) fn fechar(&mut self) {
        self.aberta = false;
        self.capturando = false;
    }
}

impl OverlayApp {
    /// Tela de configurações (no lugar da lista).
    pub(super) fn tela_config(&mut self, ui: &mut egui::Ui) {
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
                "Use Ctrl ou Alt com outra tecla. Esc cancela. Se nada acontecer, a combinação já está em uso.",
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
    pub(super) fn atalho_trocado(&mut self, r: hotkey::ResultadoAtalho) {
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
