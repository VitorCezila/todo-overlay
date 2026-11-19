//! Iniciar com o Windows via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Marco 4.
//!
//! O registro é a fonte da verdade (não há campo no `config.json`): o valor
//! pode ser apagado por fora, por exemplo por um limpador de inicialização.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};

const CHAVE_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NOME_VALOR: &str = "todo-overlay";

/// Estado do valor na chave `Run`.
#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    Desligado,
    /// Aponta para este `.exe`.
    Ligado,
    /// Existe, mas aponta para outro caminho (ex.: ligado num `cargo run`,
    /// ou o `.exe` foi movido). Guarda o valor gravado.
    OutroCaminho(String),
}

pub fn estado() -> anyhow::Result<Estado> {
    let Some(valor) = valor()? else {
        return Ok(Estado::Desligado);
    };
    let exe = std::env::current_exe().context("não foi possível localizar o .exe")?;
    Ok(if valor.eq_ignore_ascii_case(&comando(&exe)) {
        Estado::Ligado
    } else {
        Estado::OutroCaminho(valor)
    })
}

/// Texto do valor `todo-overlay` na chave `Run`, se existir.
fn valor() -> anyhow::Result<Option<String>> {
    if !existe()? {
        return Ok(None);
    }
    let run = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(CHAVE_RUN, KEY_QUERY_VALUE)
        .context("falha ao ler a inicialização do Windows")?;
    match run.get_value::<String, _>(NOME_VALOR) {
        Ok(v) => Ok(Some(v)),
        // Existe mas não é texto (REG_BINARY etc.): mostrar como "outro".
        Err(_) => Ok(Some(String::from("(valor não é texto)"))),
    }
}

/// O valor existe na chave `Run`.
fn existe() -> anyhow::Result<bool> {
    let run = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(CHAVE_RUN, KEY_QUERY_VALUE)
    {
        Ok(k) => k,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).context("falha ao ler a inicialização do Windows"),
    };
    match run.get_raw_value(NOME_VALOR) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context("falha ao ler a inicialização do Windows"),
    }
}

/// Grava o caminho do `.exe` atual na chave `Run`.
pub fn ativar() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("não foi possível localizar o .exe")?;
    let (run, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(CHAVE_RUN)
        .context("falha ao abrir a inicialização do Windows")?;
    run.set_value(NOME_VALOR, &comando(&exe))
        .context("falha ao gravar a inicialização do Windows")
}

/// Remove o valor; já ausente conta como sucesso.
pub fn desativar() -> anyhow::Result<()> {
    let run =
        match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(CHAVE_RUN, KEY_SET_VALUE) {
            Ok(k) => k,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).context("falha ao abrir a inicialização do Windows"),
        };
    match run.delete_value(NOME_VALOR) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("falha ao remover da inicialização do Windows"),
    }
}

/// Linha de comando do valor: o caminho entre aspas (pode ter espaços).
fn comando(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comando_poe_aspas_no_caminho_com_espacos() {
        let exe = Path::new(r"C:\Program Files\todo overlay\todo-overlay.exe");
        assert_eq!(
            comando(exe),
            r#""C:\Program Files\todo overlay\todo-overlay.exe""#
        );
    }
}
