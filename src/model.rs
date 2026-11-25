//! Tarefas, categorias (abas) e lista de tarefas — lógica pura, sem
//! dependência de UI.
//!
//! As mutações devolvem `bool` indicando se a lista mudou, para o app saber
//! quando precisa gravar em disco.
//!
//! A ordem de `tasks` é a ordem na tela: cada aba mostra as suas tarefas na
//! ordem em que aparecem no vetor.

use serde::{Deserialize, Serialize};

/// Categoria fixa "Geral": não fica em `categories`, não pode ser renomeada
/// nem apagada, e recebe as tarefas de uma categoria apagada.
pub const GERAL: u64 = 0;
pub const NOME_GERAL: &str = "Geral";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tarefa {
    pub id: u64,
    pub text: String,
    pub done: bool,
    /// Id da [`Categoria`]; [`GERAL`] quando ausente (arquivos v1).
    #[serde(rename = "category", default)]
    pub categoria: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Categoria {
    pub id: u64,
    #[serde(rename = "name")]
    pub nome: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListaTarefas {
    pub tasks: Vec<Tarefa>,
    #[serde(default)]
    next_id: u64,
    /// Abas criadas pelo usuário, na ordem da barra de abas (sem a "Geral").
    #[serde(rename = "categories", default)]
    categorias: Vec<Categoria>,
    #[serde(rename = "next_category_id", default)]
    proxima_categoria: u64,
}

impl ListaTarefas {
    /// Adiciona uma tarefa no fim da `categoria`. Texto vazio ou só com
    /// espaços é ignorado; categoria inexistente vira [`GERAL`].
    pub fn adicionar(&mut self, text: &str, categoria: u64) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // `normalizar` renumera antes de chegar perto do limite; isto é só a rede.
        let id = self.next_id.checked_add(1)?;
        self.next_id = id;
        let categoria = self.categoria_valida(categoria);
        self.tasks.push(Tarefa {
            id,
            text: text.to_owned(),
            done: false,
            categoria,
        });
        Some(id)
    }

    /// Troca o texto de uma tarefa. Texto vazio (após aparar) ou igual ao
    /// atual não muda nada — o texto antigo é mantido.
    pub fn editar(&mut self, id: u64, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) if t.text != text => {
                text.clone_into(&mut t.text);
                true
            }
            _ => false,
        }
    }

    /// Alterna concluída/pendente. Devolve `false` se o id não existe.
    pub fn alternar(&mut self, id: u64) -> bool {
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.done = !t.done;
                true
            }
            None => false,
        }
    }

    /// Apaga a tarefa (`None` se o id não existe) e devolve a posição que ela
    /// ocupava, para poder ser devolvida com [`ListaTarefas::restaurar`] ("Desfazer").
    pub fn tirar(&mut self, id: u64) -> Option<(usize, Tarefa)> {
        let pos = self.tasks.iter().position(|t| t.id == id)?;
        Some((pos, self.tasks.remove(pos)))
    }

    /// Devolve uma tarefa tirada por [`ListaTarefas::tirar`] na posição `pos`
    /// (ou no fim, se a lista encolheu). Recusa se o id já existe. `next_id`
    /// não muda: ele nunca diminui, então o id restaurado não será reutilizado.
    /// A categoria da tarefa pode ter sido apagada enquanto ela estava na
    /// pilha do "Desfazer": nesse caso ela volta para a [`GERAL`].
    pub fn restaurar(&mut self, pos: usize, mut tarefa: Tarefa) -> bool {
        if self.tasks.iter().any(|t| t.id == tarefa.id) {
            return false;
        }
        tarefa.categoria = self.categoria_valida(tarefa.categoria);
        let pos = pos.min(self.tasks.len());
        self.tasks.insert(pos, tarefa);
        true
    }

    /// Move a tarefa `id` para logo antes (ou depois, com `depois`) da tarefa
    /// `alvo`, passando para a categoria do alvo. A posição é relativa ao alvo
    /// e não a um índice da aba, porque a aba pode estar filtrada (concluídas
    /// escondidas). Devolve `false` se nada mudou ou algum id não existe.
    pub fn mover(&mut self, id: u64, alvo: u64, depois: bool) -> bool {
        if id == alvo {
            return false;
        }
        let Some(de) = self.tasks.iter().position(|t| t.id == id) else {
            return false;
        };
        let Some(para) = self.tasks.iter().position(|t| t.id == alvo) else {
            return false;
        };
        // Índice final que a tarefa ocuparia; igual ao atual → não mudou.
        let destino = match (para + usize::from(depois), de) {
            (d, de) if d > de => d - 1,
            (d, _) => d,
        };
        let categoria = self.tasks[para].categoria;
        if destino == de && self.tasks[de].categoria == categoria {
            return false;
        }
        let mut tarefa = self.tasks.remove(de);
        tarefa.categoria = categoria;
        self.tasks.insert(destino, tarefa);
        true
    }

    pub fn pendentes(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
    }

    /// Abas criadas pelo usuário, na ordem da barra (sem a [`GERAL`]).
    pub fn categorias(&self) -> &[Categoria] {
        &self.categorias
    }

    /// Nome para mostrar na aba; `None` se a categoria não existe.
    pub fn nome_categoria(&self, id: u64) -> Option<&str> {
        if id == GERAL {
            return Some(NOME_GERAL);
        }
        self.categorias
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.nome.as_str())
    }

    fn categoria_valida(&self, id: u64) -> u64 {
        if self.nome_categoria(id).is_some() {
            id
        } else {
            GERAL
        }
    }

    /// Cria uma aba no fim da barra. Nome vazio (após aparar) é ignorado.
    pub fn criar_categoria(&mut self, nome: &str) -> Option<u64> {
        let nome = nome.trim();
        if nome.is_empty() {
            return None;
        }
        let id = self.proxima_categoria.checked_add(1)?;
        self.proxima_categoria = id;
        self.categorias.push(Categoria {
            id,
            nome: nome.to_owned(),
        });
        Some(id)
    }

    /// Renomeia uma aba. Nome vazio ou igual ao atual não muda nada; a
    /// "Geral" não pode ser renomeada.
    pub fn renomear_categoria(&mut self, id: u64, nome: &str) -> bool {
        let nome = nome.trim();
        if nome.is_empty() {
            return false;
        }
        match self.categorias.iter_mut().find(|c| c.id == id) {
            Some(c) if c.nome != nome => {
                nome.clone_into(&mut c.nome);
                true
            }
            _ => false,
        }
    }

    /// Apaga a aba; as tarefas dela vão para o fim da [`GERAL`] (na mesma
    /// ordem), para nada se perder. A "Geral" não pode ser apagada.
    pub fn apagar_categoria(&mut self, id: u64) -> bool {
        let Some(pos) = self.categorias.iter().position(|c| c.id == id) else {
            return false;
        };
        self.categorias.remove(pos);
        let (mut movidas, ficam): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tasks)
            .into_iter()
            .partition(|t| t.categoria == id);
        self.tasks = ficam;
        for t in &mut movidas {
            t.categoria = GERAL;
        }
        self.tasks.extend(movidas);
        true
    }

    /// Corrige ids vindos de um arquivo editado à mão: `next_id` nunca reutiliza
    /// um id existente; ids duplicados ou perto de `u64::MAX` fazem a lista ser
    /// renumerada de 1 a N (os ids são internos, a ordem é preservada).
    /// O mesmo vale para as categorias; tarefa de categoria inexistente vai
    /// para a [`GERAL`].
    pub fn normalizar(&mut self) {
        self.normalizar_categorias();
        let validas: std::collections::HashSet<u64> = self
            .categorias
            .iter()
            .map(|c| c.id)
            .chain([GERAL])
            .collect();
        for t in &mut self.tasks {
            if !validas.contains(&t.categoria) {
                t.categoria = GERAL;
            }
        }
        self.normalizar_tarefas();
    }

    /// Ids de categoria duplicados, zerados (colidiriam com a "Geral") ou
    /// perto de `u64::MAX` → renumera de 1 a N e corrige as tarefas.
    fn normalizar_categorias(&mut self) {
        self.categorias.retain(|c| !c.nome.trim().is_empty());
        let mut vistos = std::collections::HashSet::with_capacity(self.categorias.len());
        let invalido = !self
            .categorias
            .iter()
            .all(|c| c.id != GERAL && vistos.insert(c.id));
        let maior = self.categorias.iter().map(|c| c.id).max().unwrap_or(0);
        if invalido || maior > u64::MAX / 2 || self.proxima_categoria > u64::MAX / 2 {
            // Com ids duplicados não dá para saber de qual aba cada tarefa
            // era: a primeira com o id fica com elas.
            let mut novo_id = std::collections::HashMap::new();
            for (c, id) in self.categorias.iter_mut().zip(1..) {
                // Uma aba com id 0 não leva as tarefas da "Geral" junto.
                if c.id != GERAL {
                    novo_id.entry(c.id).or_insert(id);
                }
                c.id = id;
            }
            for t in &mut self.tasks {
                t.categoria = novo_id.get(&t.categoria).copied().unwrap_or(GERAL);
            }
            self.proxima_categoria = self.categorias.len() as u64;
        } else {
            self.proxima_categoria = self.proxima_categoria.max(maior);
        }
    }

    fn normalizar_tarefas(&mut self) {
        let mut vistos = std::collections::HashSet::with_capacity(self.tasks.len());
        let duplicado = !self.tasks.iter().all(|t| vistos.insert(t.id));
        let maior = self.tasks.iter().map(|t| t.id).max().unwrap_or(0);
        if duplicado || maior > u64::MAX / 2 || self.next_id > u64::MAX / 2 {
            for (t, id) in self.tasks.iter_mut().zip(1..) {
                t.id = id;
            }
            self.next_id = self.tasks.len() as u64;
        } else {
            self.next_id = self.next_id.max(maior);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adicionar_ignora_texto_vazio() {
        let mut lista = ListaTarefas::default();
        assert_eq!(lista.adicionar("   ", GERAL), None);
        assert!(lista.tasks.is_empty());
    }

    #[test]
    fn adicionar_gera_ids_unicos_e_aparar_texto() {
        let mut lista = ListaTarefas::default();
        let a = lista.adicionar("  Reunião às 15h ✅ ", GERAL).unwrap();
        let b = lista.adicionar("Comprar café", GERAL).unwrap();
        assert_ne!(a, b);
        assert_eq!(lista.tasks[0].text, "Reunião às 15h ✅");
    }

    #[test]
    fn concluir_e_apagar() {
        let mut lista = ListaTarefas::default();
        let a = lista.adicionar("a", GERAL).unwrap();
        let b = lista.adicionar("b", GERAL).unwrap();
        assert!(lista.alternar(a));
        assert_eq!(lista.pendentes(), 1);
        assert!(lista.tirar(b).is_some());
        assert_eq!(lista.tasks.len(), 1);
        assert!(lista.tasks[0].done);
    }

    fn lista_com(textos: &[&str]) -> (ListaTarefas, Vec<u64>) {
        let mut lista = ListaTarefas::default();
        let ids = textos
            .iter()
            .map(|t| lista.adicionar(t, GERAL).unwrap())
            .collect();
        (lista, ids)
    }

    #[test]
    fn adicionar_preserva_ordem_de_insercao() {
        let (lista, ids) = lista_com(&["primeira", "segunda", "terceira"]);
        let textos: Vec<_> = lista.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(textos, ["primeira", "segunda", "terceira"]);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(lista.tasks.iter().all(|t| !t.done));
    }

    #[test]
    fn adicionar_ignora_string_vazia_e_quebras_de_linha() {
        let mut lista = ListaTarefas::default();
        assert_eq!(lista.adicionar("", GERAL), None);
        assert_eq!(lista.adicionar("\t\n  \r\n", GERAL), None);
        assert!(lista.tasks.is_empty());
    }

    #[test]
    fn ids_nao_sao_reutilizados_apos_apagar() {
        let (mut lista, ids) = lista_com(&["a", "b"]);
        assert!(lista.tirar(ids[1]).is_some());
        let c = lista.adicionar("c", GERAL).unwrap();
        assert!(!ids.contains(&c));
    }

    #[test]
    fn editar_apara_espacos() {
        let (mut lista, ids) = lista_com(&["antigo"]);
        assert!(lista.editar(ids[0], "   novo texto  "));
        assert_eq!(lista.tasks[0].text, "novo texto");
    }

    #[test]
    fn editar_com_texto_vazio_mantem_o_antigo() {
        let (mut lista, ids) = lista_com(&["antigo"]);
        assert!(!lista.editar(ids[0], ""));
        assert!(!lista.editar(ids[0], "   \t "));
        assert_eq!(lista.tasks[0].text, "antigo");
    }

    #[test]
    fn editar_com_texto_igual_devolve_false() {
        let (mut lista, ids) = lista_com(&["mesmo"]);
        assert!(!lista.editar(ids[0], "mesmo"));
        assert!(!lista.editar(ids[0], "  mesmo  "));
        assert_eq!(lista.tasks[0].text, "mesmo");
    }

    #[test]
    fn editar_id_inexistente_devolve_false() {
        let (mut lista, ids) = lista_com(&["a"]);
        let antes = lista.clone();
        assert!(!lista.editar(ids[0] + 100, "b"));
        assert_eq!(lista, antes);
    }

    #[test]
    fn editar_nao_altera_done_nem_outras_tarefas() {
        let (mut lista, ids) = lista_com(&["a", "b"]);
        lista.alternar(ids[0]);
        assert!(lista.editar(ids[0], "a2"));
        assert!(lista.tasks[0].done);
        assert_eq!(lista.tasks[1].text, "b");
    }

    #[test]
    fn alternar_conclui_e_reabre() {
        let (mut lista, ids) = lista_com(&["a"]);
        assert!(lista.alternar(ids[0]));
        assert!(lista.tasks[0].done);
        assert_eq!(lista.pendentes(), 0);
        assert!(lista.alternar(ids[0]));
        assert!(!lista.tasks[0].done);
        assert_eq!(lista.pendentes(), 1);
    }

    #[test]
    fn alternar_id_inexistente_devolve_false() {
        let (mut lista, ids) = lista_com(&["a"]);
        let antes = lista.clone();
        assert!(!lista.alternar(ids[0] + 1));
        assert!(!ListaTarefas::default().alternar(1));
        assert_eq!(lista, antes);
    }

    #[test]
    fn apagar_id_inexistente_devolve_false() {
        let (mut lista, ids) = lista_com(&["a"]);
        assert!(lista.tirar(ids[0] + 1).is_none());
        assert_eq!(lista.tasks.len(), 1);
        assert!(lista.tirar(ids[0]).is_some());
        assert!(
            lista.tirar(ids[0]).is_none(),
            "apagar duas vezes devolve None"
        );
    }

    #[test]
    fn apagar_do_meio_preserva_ordem() {
        let (mut lista, ids) = lista_com(&["a", "b", "c"]);
        assert!(lista.tirar(ids[1]).is_some());
        let textos: Vec<_> = lista.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(textos, ["a", "c"]);
    }

    fn textos(lista: &ListaTarefas) -> Vec<&str> {
        lista.tasks.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn tirar_e_restaurar_volta_a_mesma_ordem() {
        let (mut lista, ids) = lista_com(&["a", "b", "c"]);
        lista.alternar(ids[1]);
        let antes = lista.clone();
        let (pos, tarefa) = lista.tirar(ids[1]).unwrap();
        assert_eq!(pos, 1);
        assert_eq!(textos(&lista), ["a", "c"]);
        assert!(lista.restaurar(pos, tarefa));
        assert_eq!(lista, antes, "restaura texto, done e posição");
    }

    #[test]
    fn tirar_id_inexistente_devolve_none() {
        let (mut lista, ids) = lista_com(&["a"]);
        assert_eq!(lista.tirar(ids[0] + 1), None);
        assert_eq!(lista.tasks.len(), 1);
    }

    #[test]
    fn restaurar_alem_do_fim_insere_no_fim() {
        let (mut lista, ids) = lista_com(&["a", "b", "c"]);
        let (pos, tarefa) = lista.tirar(ids[2]).unwrap();
        assert!(lista.tirar(ids[1]).is_some());
        assert!(lista.restaurar(pos, tarefa));
        assert_eq!(textos(&lista), ["a", "c"]);
    }

    #[test]
    fn restaurar_id_existente_e_recusado() {
        let (mut lista, ids) = lista_com(&["a"]);
        let copia = lista.tasks[0].clone();
        let antes = lista.clone();
        assert!(!lista.restaurar(0, copia));
        assert_eq!(lista, antes);
        assert!(lista.tirar(ids[0]).is_some());
    }

    #[test]
    fn adicionar_depois_de_restaurar_nao_repete_id() {
        let (mut lista, ids) = lista_com(&["a", "b"]);
        let (pos, tarefa) = lista.tirar(ids[1]).unwrap();
        assert!(lista.restaurar(pos, tarefa));
        let novo = lista.adicionar("c", GERAL).unwrap();
        assert!(!ids.contains(&novo));
    }

    #[test]
    fn restaurar_varias_em_ordem_inversa_recompoe_a_lista() {
        let (mut lista, ids) = lista_com(&["a", "b", "c", "d"]);
        let antes = lista.clone();
        // Apagar em sequência, como num duplo clique que pega a linha de baixo.
        let mut pilha = Vec::new();
        for id in [ids[1], ids[2], ids[0]] {
            pilha.push(lista.tirar(id).unwrap());
        }
        assert_eq!(textos(&lista), ["d"]);
        while let Some((pos, tarefa)) = pilha.pop() {
            assert!(lista.restaurar(pos, tarefa));
        }
        assert_eq!(lista, antes);
    }

    #[test]
    fn normalizar_com_next_id_ausente_nao_repete_id() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[{"id":3,"text":"x","done":false},{"id":7,"text":"y","done":true}]}"#,
        )
        .unwrap();
        lista.normalizar();
        let novo = lista.adicionar("z", GERAL).unwrap();
        assert!(![3, 7].contains(&novo));
        assert!(novo > 7);
    }

    #[test]
    fn normalizar_com_next_id_defasado_nao_repete_id() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[{"id":3,"text":"x","done":false},{"id":7,"text":"y","done":false}],"next_id":2}"#,
        )
        .unwrap();
        lista.normalizar();
        assert_eq!(lista.adicionar("z", GERAL), Some(8));
    }

    #[test]
    fn normalizar_nao_diminui_next_id_maior() {
        let mut lista: ListaTarefas =
            serde_json::from_str(r#"{"tasks":[{"id":1,"text":"x","done":false}],"next_id":10}"#)
                .unwrap();
        lista.normalizar();
        assert_eq!(lista.adicionar("z", GERAL), Some(11));
    }

    #[test]
    fn normalizar_lista_vazia_comeca_do_um() {
        let mut lista = ListaTarefas::default();
        lista.normalizar();
        assert_eq!(lista.adicionar("a", GERAL), Some(1));
    }

    #[test]
    fn normalizar_renumera_ids_duplicados() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[{"id":5,"text":"a","done":false},{"id":5,"text":"b","done":true}],"next_id":5}"#,
        )
        .unwrap();
        lista.normalizar();
        let ids: Vec<_> = lista.tasks.iter().map(|t| t.id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(lista.tasks[1].text, "b");
        assert!(lista.tasks[1].done);
        assert!(lista.tirar(1).is_some());
        assert_eq!(
            lista.tasks.len(),
            1,
            "apagar não pode levar a duplicata junto"
        );
        assert_eq!(lista.adicionar("c", GERAL), Some(3));
    }

    #[test]
    fn adicionar_com_id_maximo_nao_da_panic() {
        let mut lista: ListaTarefas = serde_json::from_str(&format!(
            r#"{{"tasks":[{{"id":{},"text":"x","done":false}}]}}"#,
            u64::MAX
        ))
        .unwrap();
        lista.normalizar();
        let r = std::panic::catch_unwind(move || {
            let mut lista = lista;
            lista.adicionar("y", GERAL)
        });
        assert!(r.is_ok(), "add deu panic com id u64::MAX");
    }

    #[test]
    fn acentos_e_emoji_preservados() {
        let (mut lista, ids) = lista_com(&["Reunião às 15h ✅", "Café ☕ com açúcar"]);
        assert_eq!(lista.tasks[0].text, "Reunião às 15h ✅");
        assert_eq!(lista.tasks[1].text, "Café ☕ com açúcar");
        assert!(lista.editar(ids[0], "  Revisão ção 🚀 "));
        assert_eq!(lista.tasks[0].text, "Revisão ção 🚀");
    }

    #[test]
    fn edit_distingue_maiusculas_e_acentos() {
        let (mut lista, ids) = lista_com(&["cafe"]);
        assert!(lista.editar(ids[0], "café"));
        assert!(lista.editar(ids[0], "Café"));
        assert_eq!(lista.tasks[0].text, "Café");
    }

    // --- mover ---

    #[test]
    fn mover_para_cima_e_para_baixo() {
        let (mut lista, ids) = lista_com(&["a", "b", "c", "d"]);
        assert!(lista.mover(ids[3], ids[0], false));
        assert_eq!(textos(&lista), ["d", "a", "b", "c"]);
        assert!(lista.mover(ids[3], ids[2], true));
        assert_eq!(textos(&lista), ["a", "b", "c", "d"]);
        assert!(lista.mover(ids[0], ids[2], false));
        assert_eq!(textos(&lista), ["b", "a", "c", "d"]);
    }

    #[test]
    fn mover_para_o_mesmo_lugar_devolve_false() {
        let (mut lista, ids) = lista_com(&["a", "b", "c"]);
        let antes = lista.clone();
        assert!(!lista.mover(ids[1], ids[1], false));
        assert!(
            !lista.mover(ids[1], ids[0], true),
            "logo depois de a já está"
        );
        assert!(
            !lista.mover(ids[1], ids[2], false),
            "logo antes de c já está"
        );
        assert_eq!(lista, antes);
    }

    #[test]
    fn mover_id_inexistente_devolve_false() {
        let (mut lista, ids) = lista_com(&["a", "b"]);
        let antes = lista.clone();
        assert!(!lista.mover(99, ids[0], false));
        assert!(!lista.mover(ids[0], 99, true));
        assert_eq!(lista, antes);
    }

    #[test]
    fn mover_pula_tarefas_de_outras_abas_e_preserva_dados() {
        let mut lista = ListaTarefas::default();
        let cat = lista.criar_categoria("Trabalho").unwrap();
        let a = lista.adicionar("a", GERAL).unwrap();
        let x = lista.adicionar("x", cat).unwrap();
        let b = lista.adicionar("b", GERAL).unwrap();
        lista.alternar(b);
        // Na aba Geral só aparecem a e b: subir b para antes de a.
        assert!(lista.mover(b, a, false));
        assert_eq!(textos(&lista), ["b", "a", "x"]);
        assert!(lista.tasks[0].done);
        assert_eq!(lista.tasks[2].id, x);
        assert_eq!(lista.tasks[2].categoria, cat);
    }

    #[test]
    fn mover_assume_a_categoria_do_alvo() {
        let mut lista = ListaTarefas::default();
        let cat = lista.criar_categoria("Trabalho").unwrap();
        let a = lista.adicionar("a", GERAL).unwrap();
        let x = lista.adicionar("x", cat).unwrap();
        assert!(lista.mover(a, x, true));
        assert_eq!(textos(&lista), ["x", "a"]);
        assert_eq!(lista.tasks[1].categoria, cat);
    }

    // --- categorias ---

    #[test]
    fn criar_categoria_apara_e_ignora_vazio() {
        let mut lista = ListaTarefas::default();
        assert_eq!(lista.criar_categoria("  \t "), None);
        let a = lista.criar_categoria("  Trabalho ").unwrap();
        let b = lista.criar_categoria("Casa").unwrap();
        assert_ne!(a, GERAL);
        assert_ne!(a, b);
        assert_eq!(lista.nome_categoria(a), Some("Trabalho"));
        assert_eq!(lista.nome_categoria(GERAL), Some(NOME_GERAL));
        let nomes: Vec<_> = lista.categorias().iter().map(|c| c.nome.as_str()).collect();
        assert_eq!(nomes, ["Trabalho", "Casa"]);
    }

    #[test]
    fn adicionar_em_categoria_inexistente_vai_para_geral() {
        let mut lista = ListaTarefas::default();
        lista.adicionar("a", 42).unwrap();
        assert_eq!(lista.tasks[0].categoria, GERAL);
    }

    #[test]
    fn renomear_categoria() {
        let mut lista = ListaTarefas::default();
        let a = lista.criar_categoria("Trabalho").unwrap();
        assert!(lista.renomear_categoria(a, " Job "));
        assert_eq!(lista.nome_categoria(a), Some("Job"));
        assert!(!lista.renomear_categoria(a, "Job"), "igual não muda");
        assert!(!lista.renomear_categoria(a, "  "), "vazio mantém o antigo");
        assert!(!lista.renomear_categoria(GERAL, "Outra"), "Geral é fixa");
        assert!(!lista.renomear_categoria(99, "x"));
        assert_eq!(lista.nome_categoria(a), Some("Job"));
    }

    #[test]
    fn apagar_categoria_manda_tarefas_para_o_fim_da_geral() {
        let mut lista = ListaTarefas::default();
        let cat = lista.criar_categoria("Trabalho").unwrap();
        lista.adicionar("x1", cat).unwrap();
        lista.adicionar("g1", GERAL).unwrap();
        let x2 = lista.adicionar("x2", cat).unwrap();
        lista.adicionar("g2", GERAL).unwrap();
        lista.alternar(x2);
        assert!(lista.apagar_categoria(cat));
        assert_eq!(textos(&lista), ["g1", "g2", "x1", "x2"]);
        assert!(lista.tasks.iter().all(|t| t.categoria == GERAL));
        assert!(lista.tasks[3].done);
        assert!(lista.categorias().is_empty());
        assert_eq!(lista.nome_categoria(cat), None);
    }

    #[test]
    fn apagar_geral_ou_inexistente_e_recusado() {
        let (mut lista, _) = lista_com(&["a"]);
        let antes = lista.clone();
        assert!(!lista.apagar_categoria(GERAL));
        assert!(!lista.apagar_categoria(7));
        assert_eq!(lista, antes);
    }

    #[test]
    fn ids_de_categoria_nao_sao_reutilizados() {
        let mut lista = ListaTarefas::default();
        let a = lista.criar_categoria("a").unwrap();
        assert!(lista.apagar_categoria(a));
        let b = lista.criar_categoria("b").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn restaurar_tarefa_de_categoria_apagada_volta_para_geral() {
        let mut lista = ListaTarefas::default();
        let cat = lista.criar_categoria("Trabalho").unwrap();
        let x = lista.adicionar("x", cat).unwrap();
        let (pos, tarefa) = lista.tirar(x).unwrap();
        assert!(lista.apagar_categoria(cat));
        assert!(lista.restaurar(pos, tarefa));
        assert_eq!(lista.tasks[0].categoria, GERAL);
    }

    #[test]
    fn normalizar_manda_categoria_orfa_para_geral() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[{"id":1,"text":"x","done":false,"category":5}],
                "categories":[{"id":2,"name":"Casa"}],"next_category_id":2}"#,
        )
        .unwrap();
        lista.normalizar();
        assert_eq!(lista.tasks[0].categoria, GERAL);
        assert_eq!(lista.criar_categoria("nova"), Some(3));
    }

    #[test]
    fn normalizar_renumera_categorias_duplicadas_ou_zeradas() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[
                    {"id":1,"text":"g","done":false},
                    {"id":2,"text":"c","done":false,"category":4}],
                "categories":[{"id":0,"name":"Zero"},{"id":4,"name":"Casa"},{"id":4,"name":"Dup"}],
                "next_category_id":4}"#,
        )
        .unwrap();
        lista.normalizar();
        let ids: Vec<_> = lista.categorias().iter().map(|c| c.id).collect();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(
            lista.tasks[0].categoria, GERAL,
            "a Geral não vira a aba Zero"
        );
        assert_eq!(lista.tasks[1].categoria, 2, "fica com a primeira Casa");
        assert_eq!(lista.criar_categoria("nova"), Some(4));
    }

    #[test]
    fn normalizar_descarta_categoria_sem_nome() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[{"id":1,"text":"x","done":false,"category":3}],
                "categories":[{"id":3,"name":"  "}]}"#,
        )
        .unwrap();
        lista.normalizar();
        assert!(lista.categorias().is_empty());
        assert_eq!(lista.tasks[0].categoria, GERAL);
        assert_eq!(lista.tasks.len(), 1, "a tarefa nunca se perde");
    }

    #[test]
    fn normalizar_com_proxima_categoria_defasada_nao_repete_id() {
        let mut lista: ListaTarefas = serde_json::from_str(
            r#"{"tasks":[],"categories":[{"id":7,"name":"a"}],"next_category_id":1}"#,
        )
        .unwrap();
        lista.normalizar();
        assert_eq!(lista.criar_categoria("b"), Some(8));
    }
}
