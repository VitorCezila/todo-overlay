//! Configurações do usuário em `%APPDATA%\todo-overlay\config.json`.
//!
//! Formato: `{"version": 1, "hotkey": "Ctrl+Alt+T", "opacity": 0.8}`.
//! Mesmas regras do `tasks.json` (ver `storage`): gravação atômica, arquivo
//! corrompido vai para `.bak`, versão futura fica somente leitura.
//!
//! O "iniciar com o Windows" não fica aqui: a fonte da verdade é o registro
//! (`autostart`), que pode ser mudado por fora do app.
//!
//! Campo novo: `serde(default)` basta (skill `nova-config`). Renomear,
//! remover ou mudar tipo: skill `migrar-dados`.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Context;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::storage;

/// Versão atual do formato de `config.json`.
pub const VERSAO_ATUAL: u64 = 1;

const NOME_ARQUIVO: &str = "config.json";

/// Atalho global padrão para mostrar/esconder.
pub const ATALHO_PADRAO: &str = "Ctrl+Alt+T";
pub const OPACIDADE_PADRAO: f32 = 0.8;
pub const OPACIDADE_MIN: f32 = 0.2;
pub const OPACIDADE_MAX: f32 = 1.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Atalho global no formato de [`texto_atalho`] (ex.: `"Ctrl+Alt+T"`).
    /// Precisa de pelo menos um modificador. Padrão: [`ATALHO_PADRAO`].
    pub hotkey: String,
    /// Opacidade do fundo da janela, de [`OPACIDADE_MIN`] a [`OPACIDADE_MAX`].
    /// Padrão: [`OPACIDADE_PADRAO`].
    pub opacity: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: ATALHO_PADRAO.to_owned(),
            opacity: OPACIDADE_PADRAO,
        }
    }
}

#[derive(Serialize)]
struct Arquivo<'a> {
    version: u64,
    #[serde(flatten)]
    config: &'a Config,
}

/// Resultado de [`carregar`].
#[derive(Debug, Default)]
pub struct Carregado {
    pub config: Config,
    /// Mensagem para mostrar ao usuário (backup feito, valor corrigido...).
    pub aviso: Option<String>,
    /// `true` quando o arquivo não deve ser sobrescrito (versão futura).
    pub somente_leitura: bool,
}

impl Config {
    /// Corrige valores fora da faixa e atalho inválido. Devolve o aviso, se
    /// algo foi trocado pelo padrão.
    fn normalizar(&mut self) -> Option<String> {
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(OPACIDADE_MIN, OPACIDADE_MAX)
        } else {
            OPACIDADE_PADRAO
        };
        match atalho_de(&self.hotkey) {
            Some(hk) => {
                self.hotkey = texto_atalho(&hk);
                None
            }
            None => {
                let invalido = std::mem::replace(&mut self.hotkey, ATALHO_PADRAO.to_owned());
                Some(format!(
                    "Atalho inválido em config.json (\"{invalido}\"); usando {ATALHO_PADRAO}"
                ))
            }
        }
    }

    /// O atalho já validado (o padrão, se o campo foi mexido sem passar por
    /// `normalizar`).
    pub fn atalho(&self) -> HotKey {
        atalho_de(&self.hotkey).unwrap_or_else(atalho_padrao)
    }

    /// Alfa do fundo (0–255).
    pub fn alfa(&self) -> u8 {
        (self.opacity.clamp(OPACIDADE_MIN, OPACIDADE_MAX) * 255.0).round() as u8
    }
}

fn atalho_padrao() -> HotKey {
    HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT)
}

/// Interpreta um atalho e exige pelo menos um modificador (uma tecla solta
/// registrada como atalho global deixaria de funcionar em todo o Windows).
pub fn atalho_de(texto: &str) -> Option<HotKey> {
    let hk = HotKey::from_str(texto).ok()?;
    recusa(&hk).is_none().then_some(hk)
}

/// Por que esta combinação não pode virar atalho global, se não puder. Um
/// atalho global é interceptado no Windows inteiro: Shift+letra deixaria de
/// digitar maiúscula, Alt+F4 deixaria de fechar janelas etc.
pub fn recusa(hk: &HotKey) -> Option<&'static str> {
    use Code::*;
    let m = hk.mods;
    let so = |x: Modifiers| m == x;
    let navegacao = matches!(
        hk.key,
        Insert
            | Delete
            | Home
            | End
            | PageUp
            | PageDown
            | ArrowUp
            | ArrowDown
            | ArrowLeft
            | ArrowRight
    ) || funcao(hk.key);

    if m.is_empty() {
        return Some("Use Ctrl, Alt ou Shift junto com a tecla.");
    }
    if !m.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META) && !navegacao {
        return Some("Shift sozinho só com F1–F24 ou teclas de navegação.");
    }
    let reservada = (so(Modifiers::ALT) && matches!(hk.key, F4 | Tab | Escape | Space))
        || (so(Modifiers::CONTROL)
            && matches!(
                hk.key,
                KeyA | KeyC | KeyV | KeyX | KeyZ | KeyY | KeyS | Backspace | Enter | Tab | Escape
            ));
    if reservada {
        return Some("Combinação reservada pelo Windows ou de uso comum.");
    }
    // Ctrl+Alt é o AltGr: no teclado ABNT2 estas teclas digitam / ? ° ₢
    // ¹ ² ³ £ ¢ ¬ § ª º, que deixariam de funcionar.
    let altgr = so(Modifiers::CONTROL | Modifiers::ALT)
        && matches!(
            hk.key,
            KeyQ | KeyW
                | KeyE
                | KeyC
                | Digit1
                | Digit2
                | Digit3
                | Digit4
                | Digit5
                | Digit6
                | Equal
                | BracketLeft
                | BracketRight
        );
    if altgr {
        return Some("No teclado ABNT2 essa combinação é o AltGr de um símbolo.");
    }
    None
}

fn funcao(c: Code) -> bool {
    use Code::*;
    matches!(
        c,
        F1 | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
    )
}

/// Texto legível e aceito de volta por [`atalho_de`]: `"Ctrl+Shift+F5"`.
/// (O `Display` do crate gera `"shift+control+F5"` e `"KeyT"`.)
pub fn texto_atalho(hk: &HotKey) -> String {
    let mut s = String::new();
    for (m, nome) in [
        (Modifiers::CONTROL, "Ctrl+"),
        (Modifiers::ALT, "Alt+"),
        (Modifiers::SHIFT, "Shift+"),
        (Modifiers::META, "Super+"),
    ] {
        if hk.mods.contains(m) {
            s.push_str(nome);
        }
    }
    let tecla = hk.key.to_string();
    // "KeyT" → "T", "Digit5" → "5"; o resto ("F5", "ArrowUp") fica como está.
    let curto = tecla
        .strip_prefix("Key")
        .or_else(|| tecla.strip_prefix("Digit"))
        .filter(|r| r.len() == 1)
        .unwrap_or(&tecla);
    s.push_str(curto);
    s
}

/// `%APPDATA%\todo-overlay\config.json`.
pub fn caminho_padrao() -> anyhow::Result<PathBuf> {
    storage::caminho_no_appdata(NOME_ARQUIVO)
}

/// Carrega as configurações. Arquivo inexistente → padrão. Só devolve `Err`
/// em falha real de I/O (ex.: permissão), sem mexer no arquivo.
pub fn carregar(path: &Path) -> anyhow::Result<Carregado> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Carregado::default()),
        Err(e) => return Err(e).with_context(|| format!("falha ao ler {}", path.display())),
    };

    let Ok(valor) = serde_json::from_slice::<Value>(&bytes) else {
        return corrompido(path);
    };
    let Some(versao) = storage::versao_de(&valor) else {
        return corrompido(path);
    };

    if versao > VERSAO_ATUAL {
        // De uma versão mais nova do app: usa o que der, nunca grava.
        let mut config = serde_json::from_value::<Config>(valor).unwrap_or_default();
        config.normalizar();
        return Ok(Carregado {
            config,
            aviso: Some(format!(
                "config.json é de uma versão mais nova (v{versao}); alterações não serão salvas"
            )),
            somente_leitura: true,
        });
    }

    match serde_json::from_value::<Config>(valor) {
        Ok(mut config) => {
            let aviso = config.normalizar();
            Ok(Carregado {
                config,
                aviso,
                somente_leitura: false,
            })
        }
        Err(_) => corrompido(path),
    }
}

/// Grava de forma atômica (`*.tmp` + rename).
pub fn salvar(path: &Path, config: &Config) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(&Arquivo {
        version: VERSAO_ATUAL,
        config,
    })
    .context("falha ao serializar as configurações")?;
    storage::gravar_atomico(path, &json)
}

fn corrompido(path: &Path) -> anyhow::Result<Carregado> {
    let nome = storage::mover_para_backup(path)?;
    Ok(Carregado {
        aviso: Some(format!(
            "config.json estava corrompido; cópia salva em {nome}"
        )),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NOME_ARQUIVO);
        (dir, path)
    }

    fn nomes(dir: &TempDir) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn carregar_texto(conteudo: &str) -> (TempDir, PathBuf, Carregado) {
        let (dir, path) = tmp();
        fs::write(&path, conteudo).unwrap();
        let c = carregar(&path).unwrap();
        (dir, path, c)
    }

    #[test]
    fn arquivo_ausente_usa_padrao_sem_criar_nada() {
        let (dir, path) = tmp();
        let c = carregar(&path).unwrap();
        assert_eq!(c.config, Config::default());
        assert!(c.aviso.is_none());
        assert!(!c.somente_leitura);
        assert!(nomes(&dir).is_empty());
    }

    #[test]
    fn json_sem_campos_usa_padroes() {
        let (_d, _p, c) = carregar_texto(r#"{"version": 1}"#);
        assert_eq!(c.config, Config::default());
        assert!(c.aviso.is_none());
    }

    #[test]
    fn json_sem_versao_e_tratado_como_v1() {
        let (_d, _p, c) = carregar_texto(r#"{"opacity": 0.5}"#);
        assert_eq!(c.config.opacity, 0.5);
        assert_eq!(c.config.hotkey, ATALHO_PADRAO);
        assert!(!c.somente_leitura);
    }

    #[test]
    fn opacidade_fora_da_faixa_e_corrigida() {
        let (_d, _p, c) = carregar_texto(r#"{"version": 1, "opacity": 3.0}"#);
        assert_eq!(c.config.opacity, OPACIDADE_MAX);
        let (_d, _p, c) = carregar_texto(r#"{"version": 1, "opacity": -1}"#);
        assert_eq!(c.config.opacity, OPACIDADE_MIN);
    }

    #[test]
    fn atalho_invalido_volta_ao_padrao_com_aviso() {
        for ruim in [
            "",
            "Ctrl+",
            "Ctrl+Alt+Banana",
            "T",
            "Ctrl+T+Alt",
            "Shift+A",
            "Alt+F4",
            "Ctrl+Alt+Q",
        ] {
            let json = format!(r#"{{"version": 1, "hotkey": "{ruim}", "opacity": 0.5}}"#);
            let (_d, _p, c) = carregar_texto(&json);
            assert_eq!(c.config.hotkey, ATALHO_PADRAO, "{ruim:?}");
            assert_eq!(c.config.opacity, 0.5, "o resto do arquivo vale");
            assert!(c.aviso.is_some(), "{ruim:?}");
        }
    }

    #[test]
    fn atalho_e_normalizado_para_o_texto_legivel() {
        let (_d, _p, c) = carregar_texto(r#"{"hotkey": "shift+control+KeyY"}"#);
        assert_eq!(c.config.hotkey, "Ctrl+Shift+Y");
        assert!(c.aviso.is_none());
    }

    #[test]
    fn ida_e_volta() {
        let (_dir, path) = tmp();
        let config = Config {
            hotkey: "Ctrl+Shift+F5".into(),
            opacity: 0.35,
        };
        salvar(&path, &config).unwrap();
        let c = carregar(&path).unwrap();
        assert_eq!(c.config, config);
        assert!(c.aviso.is_none());
        let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["version"], VERSAO_ATUAL);
    }

    #[test]
    fn salvar_nao_deixa_tmp() {
        let (dir, path) = tmp();
        salvar(&path, &Config::default()).unwrap();
        salvar(&path, &Config::default()).unwrap();
        assert_eq!(nomes(&dir), [NOME_ARQUIVO]);
    }

    #[test]
    fn corrompido_vira_bak_e_usa_padrao() {
        for ruim in [
            "{",
            "",
            "[1]",
            "null",
            r#"{"opacity": "alta"}"#,
            r#"{"version": "1"}"#,
        ] {
            let (dir, path, c) = carregar_texto(ruim);
            assert_eq!(c.config, Config::default(), "{ruim:?}");
            assert!(c.aviso.is_some());
            assert!(!path.exists());
            assert_eq!(nomes(&dir), ["config.json.bak"], "{ruim:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("config.json.bak")).unwrap(),
                ruim
            );
        }
    }

    #[test]
    fn versao_futura_fica_somente_leitura_e_intacta() {
        let conteudo = format!(
            r#"{{"version": {}, "opacity": 0.4, "novo": true}}"#,
            VERSAO_ATUAL + 1
        );
        let (dir, path, c) = carregar_texto(&conteudo);
        assert!(c.somente_leitura);
        assert!(c.aviso.is_some());
        assert_eq!(c.config.opacity, 0.4);
        assert_eq!(fs::read_to_string(&path).unwrap(), conteudo);
        assert_eq!(nomes(&dir), [NOME_ARQUIVO]);
    }

    #[test]
    fn texto_atalho_ida_e_volta() {
        for texto in [
            "Ctrl+Alt+T",
            "Ctrl+Shift+5",
            "Alt+F12",
            "Ctrl+Alt+Shift+ArrowUp",
            "Shift+F7",
            "Ctrl+PageDown",
            "Alt+Super+Comma",
        ] {
            let hk = atalho_de(texto).unwrap_or_else(|| panic!("{texto} não interpretou"));
            assert_eq!(texto_atalho(&hk), texto);
        }
    }

    #[test]
    fn atalho_sem_modificador_e_recusado() {
        assert!(atalho_de("T").is_none());
        assert!(atalho_de("F5").is_none());
        assert!(atalho_de("Ctrl+T").is_some());
    }

    #[test]
    fn combinacoes_que_quebrariam_o_windows_sao_recusadas() {
        for ruim in [
            "Shift+A",
            "Shift+Space",
            "Shift+5",
            "Alt+F4",
            "Alt+Tab",
            "Ctrl+Escape",
            "Ctrl+C",
            "Ctrl+V",
            "Ctrl+Z",
            "Ctrl+Enter",
            "Ctrl+Alt+Q",
            "Ctrl+Alt+2",
            "Ctrl+Alt+Equal",
        ] {
            let hk = HotKey::from_str(ruim).unwrap();
            assert!(recusa(&hk).is_some(), "{ruim} deveria ser recusado");
            assert!(atalho_de(ruim).is_none(), "{ruim}");
        }
    }

    #[test]
    fn combinacoes_comuns_sao_aceitas() {
        for bom in [
            "Ctrl+Alt+T",
            "Ctrl+Shift+Y",
            "Ctrl+Shift+C",
            "Ctrl+Alt+Shift+Q",
            "Shift+F7",
            "Shift+PageUp",
            "Alt+F12",
            "Ctrl+F4",
        ] {
            assert!(atalho_de(bom).is_some(), "{bom} deveria ser aceito");
        }
    }

    #[test]
    fn alfa_segue_a_opacidade() {
        let com = |opacity| Config {
            opacity,
            ..Default::default()
        };
        assert_eq!(com(1.0).alfa(), 255);
        assert_eq!(com(0.2).alfa(), 51);
        assert_eq!(com(9.0).alfa(), 255, "mexido sem normalizar");
    }

    #[test]
    fn atalho_padrao_bate_com_o_texto() {
        assert_eq!(texto_atalho(&atalho_padrao()), ATALHO_PADRAO);
        assert_eq!(Config::default().atalho(), atalho_padrao());
    }

    #[test]
    fn caminho_padrao_termina_em_config_json() {
        let p = caminho_padrao().unwrap();
        assert!(p.ends_with(Path::new("todo-overlay").join("config.json")));
    }
}
