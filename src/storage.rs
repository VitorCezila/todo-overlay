//! Persistência de `%APPDATA%\todo-overlay\tasks.json`.
//!
//! Formato: `{"version": 1, "tasks": [...], "next_id": N}`.
//! - Gravação atômica: escreve `tasks.json.tmp` e renomeia por cima.
//! - Arquivo corrompido (JSON inválido ou formato inesperado) → renomeado
//!   para `.bak` (sem nunca sobrescrever um backup existente) e a lista
//!   começa vazia.
//! - Arquivo de versão futura → carregado em modo somente leitura; o app não
//!   grava por cima.
//!
//! Mudanças de formato: ver skill `migrar-dados`.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::model::ListaTarefas;

/// Versão atual do formato de `tasks.json`.
pub const VERSAO_ATUAL: u64 = 1;

const NOME_APP: &str = "todo-overlay";
const NOME_ARQUIVO: &str = "tasks.json";

/// Resultado de [`carregar`].
#[derive(Debug, Default)]
pub struct Carregado {
    pub lista: ListaTarefas,
    /// Mensagem para mostrar ao usuário (backup feito, versão futura...).
    pub aviso: Option<String>,
    /// `true` quando o arquivo não deve ser sobrescrito (versão futura).
    pub somente_leitura: bool,
}

#[derive(Serialize)]
struct Arquivo<'a> {
    version: u64,
    #[serde(flatten)]
    lista: &'a ListaTarefas,
}

/// `%APPDATA%\todo-overlay\tasks.json`.
pub fn caminho_padrao() -> anyhow::Result<PathBuf> {
    caminho_no_appdata(NOME_ARQUIVO)
}

/// `%APPDATA%\todo-overlay\<nome>`.
pub(crate) fn caminho_no_appdata(nome: &str) -> anyhow::Result<PathBuf> {
    let base =
        directories::BaseDirs::new().context("não foi possível localizar a pasta %APPDATA%")?;
    Ok(base.config_dir().join(NOME_APP).join(nome))
}

/// Carrega a lista. Arquivo inexistente → lista vazia. Só devolve `Err` em
/// falha real de I/O (ex.: permissão), sem mexer no arquivo.
pub fn carregar(path: &Path) -> anyhow::Result<Carregado> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Carregado::default()),
        Err(e) => return Err(e).with_context(|| format!("falha ao ler {}", path.display())),
    };

    let Ok(valor) = serde_json::from_slice::<Value>(&bytes) else {
        return corrompido(path);
    };
    let Some(versao) = versao_de(&valor) else {
        return corrompido(path);
    };

    if versao > VERSAO_ATUAL {
        // Gerado por uma versão mais nova do app: lê o que der, nunca grava.
        let mut lista = serde_json::from_value::<ListaTarefas>(valor).unwrap_or_default();
        lista.normalizar();
        return Ok(Carregado {
            lista,
            aviso: Some(format!(
                "tasks.json é de uma versão mais nova (v{versao}); alterações não serão salvas"
            )),
            somente_leitura: true,
        });
    }

    let valor = migrar(valor, versao);
    match serde_json::from_value::<ListaTarefas>(valor) {
        Ok(mut lista) => {
            lista.normalizar();
            Ok(Carregado {
                lista,
                ..Default::default()
            })
        }
        Err(_) => corrompido(path),
    }
}

/// Grava de forma atômica: `*.tmp` + `sync_all` + rename por cima.
pub fn salvar(path: &Path, lista: &ListaTarefas) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(&Arquivo {
        version: VERSAO_ATUAL,
        lista,
    })
    .context("falha ao serializar as tarefas")?;
    gravar_atomico(path, &json)
}

/// Cria a pasta se preciso, grava `*.tmp`, faz `sync_all` e renomeia por cima.
/// Também usado pelo `config`.
pub(crate) fn gravar_atomico(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("falha ao criar a pasta {}", dir.display()))?;
    }
    let tmp = com_sufixo(path, ".tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("falha ao criar {}", tmp.display()))?;
        f.write_all(bytes)
            .and_then(|()| f.sync_all())
            .with_context(|| format!("falha ao gravar {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("falha ao substituir {}", path.display()))?;
    Ok(())
}

/// `version` do arquivo; ausente → 1. Presente mas não numérico → `None`.
/// Raiz que não é objeto → `None` (o serde aceitaria struct como array, e um
/// `[[]]` viraria lista vazia gravada por cima do arquivo).
pub(crate) fn versao_de(valor: &Value) -> Option<u64> {
    match valor.as_object()?.get("version") {
        None => Some(1),
        Some(v) => v.as_u64(),
    }
}

/// Aplica as migrações em cadeia de `versao` até [`VERSAO_ATUAL`].
/// Ainda não há nenhuma: v1 é o primeiro formato.
fn migrar(valor: Value, _versao: u64) -> Value {
    valor
}

/// Move o arquivo ilegível para um `.bak` e começa com a lista vazia.
fn corrompido(path: &Path) -> anyhow::Result<Carregado> {
    let nome = mover_para_backup(path)?;
    Ok(Carregado {
        aviso: Some(format!(
            "tasks.json estava corrompido; cópia salva em {nome}"
        )),
        ..Default::default()
    })
}

/// Renomeia o arquivo para o primeiro `.bak` livre e devolve o nome do
/// backup (para o aviso). Também usado pelo `config`.
pub(crate) fn mover_para_backup(path: &Path) -> anyhow::Result<String> {
    let bak = caminho_backup(path);
    fs::rename(path, &bak).with_context(|| {
        format!(
            "{} está corrompido e não foi possível movê-lo para {}",
            path.display(),
            bak.display()
        )
    })?;
    Ok(bak.file_name().map_or_else(
        || bak.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    ))
}

/// Primeiro nome livre entre `tasks.json.bak`, `tasks.json.<ts>.bak`,
/// `tasks.json.<ts>-2.bak`... — nunca sobrescreve um backup existente.
fn caminho_backup(path: &Path) -> PathBuf {
    let simples = com_sufixo(path, ".bak");
    if !simples.exists() {
        return simples;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut candidato = com_sufixo(path, &format!(".{ts}.bak"));
    let mut n = 2;
    while candidato.exists() {
        candidato = com_sufixo(path, &format!(".{ts}-{n}.bak"));
        n += 1;
    }
    candidato
}

fn com_sufixo(path: &Path, sufixo: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(sufixo);
    PathBuf::from(s)
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

    fn lista(textos: &[&str]) -> ListaTarefas {
        let mut lista = ListaTarefas::default();
        for t in textos {
            lista.adicionar(t).unwrap();
        }
        lista
    }

    /// Nomes dos arquivos dentro do diretório, ordenados.
    fn arquivos(dir: &TempDir) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn bak(path: &Path) -> PathBuf {
        com_sufixo(path, ".bak")
    }

    /// Grava `conteudo`, carrega e confere o fluxo de arquivo corrompido.
    fn assert_vira_bak(conteudo: &[u8]) {
        let (dir, path) = tmp();
        fs::write(&path, conteudo).unwrap();
        let c = carregar(&path).unwrap();
        assert!(c.lista.tasks.is_empty());
        assert!(c.aviso.is_some(), "deveria avisar o usuário");
        assert!(!c.somente_leitura);
        assert!(!path.exists(), "original deveria ter sido movido");
        assert_eq!(fs::read(bak(&path)).unwrap(), conteudo);
        assert_eq!(arquivos(&dir), ["tasks.json.bak"]);
    }

    #[test]
    fn salvar_e_carregar_ida_e_volta() {
        let (_dir, path) = tmp();
        let mut lista = lista(&["Reunião às 15h ✅", "Café ☕", "Terceira"]);
        let id = lista.tasks[1].id;
        lista.alternar(id);
        salvar(&path, &lista).unwrap();

        let c = carregar(&path).unwrap();
        assert_eq!(c.lista, lista);
        assert!(c.aviso.is_none());
        assert!(!c.somente_leitura);
        let textos: Vec<_> = c.lista.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(textos, ["Reunião às 15h ✅", "Café ☕", "Terceira"]);
        let feitas: Vec<_> = c.lista.tasks.iter().map(|t| t.done).collect();
        assert_eq!(feitas, [false, true, false]);
    }

    #[test]
    fn ida_e_volta_preserva_next_id() {
        let (_dir, path) = tmp();
        let mut lista = lista(&["a", "b", "c"]);
        let ultimo = lista.tasks[2].id;
        assert!(lista.tirar(ultimo).is_some());
        salvar(&path, &lista).unwrap();
        let mut c = carregar(&path).unwrap();
        let novo = c.lista.adicionar("d").unwrap();
        assert!(
            novo > ultimo,
            "id {novo} reutilizaria o id apagado {ultimo}"
        );
    }

    #[test]
    fn json_gravado_tem_versao_atual() {
        let (_dir, path) = tmp();
        salvar(&path, &lista(&["a"])).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["version"], VERSAO_ATUAL);
        assert_eq!(v["version"], 1);
        assert!(v["tasks"].is_array());
    }

    #[test]
    fn json_gravado_mantem_unicode_legivel() {
        let (_dir, path) = tmp();
        salvar(&path, &lista(&["Reunião às 15h ✅"])).unwrap();
        let s = fs::read_to_string(&path).unwrap();
        assert!(s.contains("Reunião às 15h ✅"));
    }

    #[test]
    fn arquivo_ausente_vira_lista_vazia_sem_criar_nada() {
        let (dir, path) = tmp();
        let c = carregar(&path).unwrap();
        assert!(c.lista.tasks.is_empty());
        assert!(c.aviso.is_none());
        assert!(!c.somente_leitura);
        assert!(!path.exists());
        assert!(arquivos(&dir).is_empty());
    }

    #[test]
    fn arquivo_ausente_em_pasta_ausente_nao_cria_pasta() {
        let (dir, _) = tmp();
        let path = dir.path().join("nao").join("existe").join(NOME_ARQUIVO);
        let c = carregar(&path).unwrap();
        assert!(c.lista.tasks.is_empty());
        assert!(!dir.path().join("nao").exists());
    }

    #[test]
    fn json_invalido_vira_bak() {
        assert_vira_bak(b"{\"version\": 1, \"tasks\": [");
    }

    #[test]
    fn arquivo_vazio_vira_bak() {
        assert_vira_bak(b"");
    }

    #[test]
    fn bytes_nao_utf8_viram_bak() {
        assert_vira_bak(&[0xff, 0xfe, 0x00, 0x7b, 0x80]);
    }

    #[test]
    fn json_array_vira_bak() {
        assert_vira_bak(b"[1,2]");
    }

    #[test]
    fn tasks_com_tipo_errado_vira_bak() {
        assert_vira_bak(br#"{"tasks": "x"}"#);
    }

    #[test]
    fn versao_nao_numerica_vira_bak() {
        assert_vira_bak(br#"{"version": "1", "tasks": []}"#);
    }

    #[test]
    fn versao_negativa_vira_bak() {
        assert_vira_bak(br#"{"version": -1, "tasks": []}"#);
    }

    #[test]
    fn json_null_vira_bak() {
        assert_vira_bak(b"null");
    }

    #[test]
    fn tarefa_sem_campo_obrigatorio_vira_bak() {
        assert_vira_bak(br#"{"version": 1, "tasks": [{"id": 1, "text": "sem done"}]}"#);
    }

    #[test]
    fn bak_existente_nao_e_sobrescrito() {
        let (dir, path) = tmp();
        let primeiro = b"primeiro corrompido {".as_slice();
        let segundo = b"segundo corrompido [".as_slice();

        fs::write(&path, primeiro).unwrap();
        carregar(&path).unwrap();
        fs::write(&path, segundo).unwrap();
        let c = carregar(&path).unwrap();

        assert!(c.lista.tasks.is_empty());
        assert!(!path.exists());
        assert_eq!(fs::read(bak(&path)).unwrap(), primeiro);

        let nomes = arquivos(&dir);
        assert_eq!(nomes.len(), 2, "{nomes:?}");
        let outro = nomes.iter().find(|n| *n != "tasks.json.bak").unwrap();
        assert!(outro.starts_with("tasks.json.") && outro.ends_with(".bak"));
        assert_eq!(fs::read(dir.path().join(outro)).unwrap(), segundo);
        assert!(c.aviso.unwrap().contains(outro.as_str()));
    }

    #[test]
    fn varios_corrompidos_seguidos_nao_sobrescrevem_backups() {
        let (dir, path) = tmp();
        let conteudos: Vec<Vec<u8>> = (0..4).map(|i| format!("lixo {i}").into_bytes()).collect();
        for c in &conteudos {
            fs::write(&path, c).unwrap();
            carregar(&path).unwrap();
        }
        let nomes = arquivos(&dir);
        assert_eq!(nomes.len(), 4, "{nomes:?}");
        let mut lidos: Vec<Vec<u8>> = nomes
            .iter()
            .map(|n| fs::read(dir.path().join(n)).unwrap())
            .collect();
        lidos.sort();
        assert_eq!(lidos, conteudos);
    }

    #[test]
    fn sem_versao_e_tratado_como_v1() {
        let (_dir, path) = tmp();
        fs::write(
            &path,
            r#"{"tasks":[{"id":3,"text":"Reunião","done":true},{"id":7,"text":"b","done":false}]}"#,
        )
        .unwrap();
        let mut c = carregar(&path).unwrap();
        assert!(c.aviso.is_none());
        assert!(!c.somente_leitura);
        assert_eq!(c.lista.tasks.len(), 2);
        assert_eq!(c.lista.tasks[0].text, "Reunião");
        assert!(c.lista.tasks[0].done);
        // sem next_id: normalizar evita repetir ids
        assert_eq!(c.lista.adicionar("novo"), Some(8));
        assert!(path.exists(), "carregar não deve mover o arquivo");
    }

    #[test]
    fn versao_futura_fica_somente_leitura_e_intacta() {
        let (dir, path) = tmp();
        let conteudo = format!(
            r#"{{"version":{},"tasks":[{{"id":1,"text":"do futuro","done":false}}],"campo_novo":true}}"#,
            VERSAO_ATUAL + 1
        );
        fs::write(&path, &conteudo).unwrap();
        let c = carregar(&path).unwrap();
        assert!(c.somente_leitura);
        assert!(c.aviso.is_some());
        assert_eq!(c.lista.tasks.len(), 1);
        assert_eq!(c.lista.tasks[0].text, "do futuro");
        assert_eq!(fs::read(&path).unwrap(), conteudo.as_bytes());
        assert_eq!(arquivos(&dir), ["tasks.json"], "não deve criar .bak");
    }

    #[test]
    fn versao_futura_ilegivel_nao_vira_bak() {
        let (dir, path) = tmp();
        let conteudo = br#"{"version": 99, "itens": {"formato": "novo"}}"#;
        fs::write(&path, conteudo).unwrap();
        let c = carregar(&path).unwrap();
        assert!(c.somente_leitura);
        assert!(c.aviso.is_some());
        assert!(c.lista.tasks.is_empty());
        assert_eq!(fs::read(&path).unwrap(), conteudo);
        assert_eq!(arquivos(&dir), ["tasks.json"]);
    }

    #[test]
    fn salvar_nao_deixa_tmp() {
        let (dir, path) = tmp();
        salvar(&path, &lista(&["a"])).unwrap();
        salvar(&path, &lista(&["a", "b"])).unwrap();
        assert!(!com_sufixo(&path, ".tmp").exists());
        assert_eq!(arquivos(&dir), ["tasks.json"]);
    }

    #[test]
    fn salvar_sobrescreve_arquivo_existente() {
        let (_dir, path) = tmp();
        salvar(&path, &lista(&["velha 1", "velha 2", "velha 3"])).unwrap();
        salvar(&path, &lista(&["nova"])).unwrap();
        let c = carregar(&path).unwrap();
        assert_eq!(c.lista.tasks.len(), 1);
        assert_eq!(c.lista.tasks[0].text, "nova");
    }

    #[test]
    fn salvar_cria_diretorios_intermediarios() {
        let (dir, _) = tmp();
        let path = dir.path().join("a").join("b").join("c").join(NOME_ARQUIVO);
        salvar(&path, &lista(&["x"])).unwrap();
        assert_eq!(carregar(&path).unwrap().lista.tasks[0].text, "x");
    }

    #[test]
    fn salvar_sobrescreve_tmp_antigo_de_crash() {
        let (dir, path) = tmp();
        let tmp_antigo = com_sufixo(&path, ".tmp");
        fs::write(
            &tmp_antigo,
            "lixo de um crash anterior, bem maior que o novo conteúdo",
        )
        .unwrap();
        salvar(&path, &lista(&["ok"])).unwrap();
        assert!(!tmp_antigo.exists());
        assert_eq!(arquivos(&dir), ["tasks.json"]);
        let c = carregar(&path).unwrap();
        assert!(c.aviso.is_none());
        assert_eq!(c.lista.tasks[0].text, "ok");
    }

    #[test]
    fn salvar_lista_vazia_carrega_vazia() {
        let (_dir, path) = tmp();
        salvar(&path, &ListaTarefas::default()).unwrap();
        let c = carregar(&path).unwrap();
        assert!(c.lista.tasks.is_empty());
        assert!(c.aviso.is_none());
    }

    #[test]
    fn caminho_padrao_termina_em_todo_overlay_tasks_json() {
        let p = caminho_padrao().unwrap();
        assert!(
            p.ends_with(Path::new("todo-overlay").join("tasks.json")),
            "{}",
            p.display()
        );
        assert!(p.is_absolute());
    }

    #[test]
    fn fixture_v1_carrega_todas_as_tarefas() {
        let (_dir, path) = tmp();
        fs::write(&path, include_str!("../tests/fixtures/tasks_v1.json")).unwrap();
        let mut c = carregar(&path).unwrap();
        assert!(c.aviso.is_none());
        assert!(!c.somente_leitura);

        let esperado = [
            (1, "Reunião às 15h ✅", false),
            (2, "Comprar pão e café", true),
            (4, "Revisar relatório de manutenção", false),
            (5, "Ligar para o médico 📞", false),
        ];
        let obtido: Vec<_> = c
            .lista
            .tasks
            .iter()
            .map(|t| (t.id, t.text.as_str(), t.done))
            .collect();
        assert_eq!(obtido, esperado);
        assert_eq!(c.lista.adicionar("nova"), Some(7), "next_id da fixture é 6");
    }

    #[test]
    fn fixture_v1_ida_e_volta_mantem_dados() {
        let (_dir, path) = tmp();
        fs::write(&path, include_str!("../tests/fixtures/tasks_v1.json")).unwrap();
        let original = carregar(&path).unwrap().lista;
        salvar(&path, &original).unwrap();
        assert_eq!(carregar(&path).unwrap().lista, original);
    }

    #[test]
    fn array_com_formato_de_struct_vira_bak() {
        // serde aceita structs como sequência: `[[]]` vira ListaTarefas vazia
        // silenciosamente em vez de ir para o .bak.
        assert_vira_bak(b"[[]]");
    }
}
