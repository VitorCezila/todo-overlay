//! Tarefas e lista de tarefas — lógica pura, sem dependência de UI.
//!
//! As mutações devolvem `bool` indicando se a lista mudou, para o app saber
//! quando precisa gravar em disco.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    #[serde(default)]
    next_id: u64,
}

impl TaskList {
    /// Adiciona uma tarefa. Texto vazio ou só com espaços é ignorado.
    pub fn add(&mut self, text: &str) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // `normalizar` renumera antes de chegar perto do limite; isto é só a rede.
        let id = self.next_id.checked_add(1)?;
        self.next_id = id;
        self.tasks.push(Task {
            id,
            text: text.to_owned(),
            done: false,
        });
        Some(id)
    }

    /// Troca o texto de uma tarefa. Texto vazio (após aparar) ou igual ao
    /// atual não muda nada — o texto antigo é mantido.
    pub fn edit(&mut self, id: u64, text: &str) -> bool {
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
    pub fn toggle(&mut self, id: u64) -> bool {
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.done = !t.done;
                true
            }
            None => false,
        }
    }

    /// Apaga a tarefa (`None` se o id não existe) e devolve a posição que ela
    /// ocupava, para poder ser devolvida com [`TaskList::restore`] ("Desfazer").
    pub fn take(&mut self, id: u64) -> Option<(usize, Task)> {
        let pos = self.tasks.iter().position(|t| t.id == id)?;
        Some((pos, self.tasks.remove(pos)))
    }

    /// Devolve uma tarefa tirada por [`TaskList::take`] na posição `pos`
    /// (ou no fim, se a lista encolheu). Recusa se o id já existe. `next_id`
    /// não muda: ele nunca diminui, então o id restaurado não será reutilizado.
    pub fn restore(&mut self, pos: usize, task: Task) -> bool {
        if self.tasks.iter().any(|t| t.id == task.id) {
            return false;
        }
        let pos = pos.min(self.tasks.len());
        self.tasks.insert(pos, task);
        true
    }

    pub fn pending_count(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
    }

    /// Corrige ids vindos de um arquivo editado à mão: `next_id` nunca reutiliza
    /// um id existente; ids duplicados ou perto de `u64::MAX` fazem a lista ser
    /// renumerada de 1 a N (os ids são internos, a ordem é preservada).
    pub fn normalizar(&mut self) {
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
        let mut list = TaskList::default();
        assert_eq!(list.add("   "), None);
        assert!(list.tasks.is_empty());
    }

    #[test]
    fn adicionar_gera_ids_unicos_e_aparar_texto() {
        let mut list = TaskList::default();
        let a = list.add("  Reunião às 15h ✅ ").unwrap();
        let b = list.add("Comprar café").unwrap();
        assert_ne!(a, b);
        assert_eq!(list.tasks[0].text, "Reunião às 15h ✅");
    }

    #[test]
    fn concluir_e_apagar() {
        let mut list = TaskList::default();
        let a = list.add("a").unwrap();
        let b = list.add("b").unwrap();
        assert!(list.toggle(a));
        assert_eq!(list.pending_count(), 1);
        assert!(list.take(b).is_some());
        assert_eq!(list.tasks.len(), 1);
        assert!(list.tasks[0].done);
    }

    fn lista_com(textos: &[&str]) -> (TaskList, Vec<u64>) {
        let mut list = TaskList::default();
        let ids = textos.iter().map(|t| list.add(t).unwrap()).collect();
        (list, ids)
    }

    #[test]
    fn adicionar_preserva_ordem_de_insercao() {
        let (list, ids) = lista_com(&["primeira", "segunda", "terceira"]);
        let textos: Vec<_> = list.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(textos, ["primeira", "segunda", "terceira"]);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(list.tasks.iter().all(|t| !t.done));
    }

    #[test]
    fn adicionar_ignora_string_vazia_e_quebras_de_linha() {
        let mut list = TaskList::default();
        assert_eq!(list.add(""), None);
        assert_eq!(list.add("\t\n  \r\n"), None);
        assert!(list.tasks.is_empty());
    }

    #[test]
    fn ids_nao_sao_reutilizados_apos_apagar() {
        let (mut list, ids) = lista_com(&["a", "b"]);
        assert!(list.take(ids[1]).is_some());
        let c = list.add("c").unwrap();
        assert!(!ids.contains(&c));
    }

    #[test]
    fn editar_apara_espacos() {
        let (mut list, ids) = lista_com(&["antigo"]);
        assert!(list.edit(ids[0], "   novo texto  "));
        assert_eq!(list.tasks[0].text, "novo texto");
    }

    #[test]
    fn editar_com_texto_vazio_mantem_o_antigo() {
        let (mut list, ids) = lista_com(&["antigo"]);
        assert!(!list.edit(ids[0], ""));
        assert!(!list.edit(ids[0], "   \t "));
        assert_eq!(list.tasks[0].text, "antigo");
    }

    #[test]
    fn editar_com_texto_igual_devolve_false() {
        let (mut list, ids) = lista_com(&["mesmo"]);
        assert!(!list.edit(ids[0], "mesmo"));
        assert!(!list.edit(ids[0], "  mesmo  "));
        assert_eq!(list.tasks[0].text, "mesmo");
    }

    #[test]
    fn editar_id_inexistente_devolve_false() {
        let (mut list, ids) = lista_com(&["a"]);
        let antes = list.clone();
        assert!(!list.edit(ids[0] + 100, "b"));
        assert_eq!(list, antes);
    }

    #[test]
    fn editar_nao_altera_done_nem_outras_tarefas() {
        let (mut list, ids) = lista_com(&["a", "b"]);
        list.toggle(ids[0]);
        assert!(list.edit(ids[0], "a2"));
        assert!(list.tasks[0].done);
        assert_eq!(list.tasks[1].text, "b");
    }

    #[test]
    fn alternar_conclui_e_reabre() {
        let (mut list, ids) = lista_com(&["a"]);
        assert!(list.toggle(ids[0]));
        assert!(list.tasks[0].done);
        assert_eq!(list.pending_count(), 0);
        assert!(list.toggle(ids[0]));
        assert!(!list.tasks[0].done);
        assert_eq!(list.pending_count(), 1);
    }

    #[test]
    fn alternar_id_inexistente_devolve_false() {
        let (mut list, ids) = lista_com(&["a"]);
        let antes = list.clone();
        assert!(!list.toggle(ids[0] + 1));
        assert!(!TaskList::default().toggle(1));
        assert_eq!(list, antes);
    }

    #[test]
    fn apagar_id_inexistente_devolve_false() {
        let (mut list, ids) = lista_com(&["a"]);
        assert!(list.take(ids[0] + 1).is_none());
        assert_eq!(list.tasks.len(), 1);
        assert!(list.take(ids[0]).is_some());
        assert!(
            list.take(ids[0]).is_none(),
            "apagar duas vezes devolve None"
        );
    }

    #[test]
    fn apagar_do_meio_preserva_ordem() {
        let (mut list, ids) = lista_com(&["a", "b", "c"]);
        assert!(list.take(ids[1]).is_some());
        let textos: Vec<_> = list.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(textos, ["a", "c"]);
    }

    fn textos(list: &TaskList) -> Vec<&str> {
        list.tasks.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn tirar_e_restaurar_volta_a_mesma_ordem() {
        let (mut list, ids) = lista_com(&["a", "b", "c"]);
        list.toggle(ids[1]);
        let antes = list.clone();
        let (pos, task) = list.take(ids[1]).unwrap();
        assert_eq!(pos, 1);
        assert_eq!(textos(&list), ["a", "c"]);
        assert!(list.restore(pos, task));
        assert_eq!(list, antes, "restaura texto, done e posição");
    }

    #[test]
    fn tirar_id_inexistente_devolve_none() {
        let (mut list, ids) = lista_com(&["a"]);
        assert_eq!(list.take(ids[0] + 1), None);
        assert_eq!(list.tasks.len(), 1);
    }

    #[test]
    fn restaurar_alem_do_fim_insere_no_fim() {
        let (mut list, ids) = lista_com(&["a", "b", "c"]);
        let (pos, task) = list.take(ids[2]).unwrap();
        assert!(list.take(ids[1]).is_some());
        assert!(list.restore(pos, task));
        assert_eq!(textos(&list), ["a", "c"]);
    }

    #[test]
    fn restaurar_id_existente_e_recusado() {
        let (mut list, ids) = lista_com(&["a"]);
        let copia = list.tasks[0].clone();
        let antes = list.clone();
        assert!(!list.restore(0, copia));
        assert_eq!(list, antes);
        assert!(list.take(ids[0]).is_some());
    }

    #[test]
    fn adicionar_depois_de_restaurar_nao_repete_id() {
        let (mut list, ids) = lista_com(&["a", "b"]);
        let (pos, task) = list.take(ids[1]).unwrap();
        assert!(list.restore(pos, task));
        let novo = list.add("c").unwrap();
        assert!(!ids.contains(&novo));
    }

    #[test]
    fn restaurar_varias_em_ordem_inversa_recompoe_a_lista() {
        let (mut list, ids) = lista_com(&["a", "b", "c", "d"]);
        let antes = list.clone();
        // Apagar em sequência, como num duplo clique que pega a linha de baixo.
        let mut pilha = Vec::new();
        for id in [ids[1], ids[2], ids[0]] {
            pilha.push(list.take(id).unwrap());
        }
        assert_eq!(textos(&list), ["d"]);
        while let Some((pos, task)) = pilha.pop() {
            assert!(list.restore(pos, task));
        }
        assert_eq!(list, antes);
    }

    #[test]
    fn normalizar_com_next_id_ausente_nao_repete_id() {
        let mut list: TaskList = serde_json::from_str(
            r#"{"tasks":[{"id":3,"text":"x","done":false},{"id":7,"text":"y","done":true}]}"#,
        )
        .unwrap();
        list.normalizar();
        let novo = list.add("z").unwrap();
        assert!(![3, 7].contains(&novo));
        assert!(novo > 7);
    }

    #[test]
    fn normalizar_com_next_id_defasado_nao_repete_id() {
        let mut list: TaskList = serde_json::from_str(
            r#"{"tasks":[{"id":3,"text":"x","done":false},{"id":7,"text":"y","done":false}],"next_id":2}"#,
        )
        .unwrap();
        list.normalizar();
        assert_eq!(list.add("z"), Some(8));
    }

    #[test]
    fn normalizar_nao_diminui_next_id_maior() {
        let mut list: TaskList =
            serde_json::from_str(r#"{"tasks":[{"id":1,"text":"x","done":false}],"next_id":10}"#)
                .unwrap();
        list.normalizar();
        assert_eq!(list.add("z"), Some(11));
    }

    #[test]
    fn normalizar_lista_vazia_comeca_do_um() {
        let mut list = TaskList::default();
        list.normalizar();
        assert_eq!(list.add("a"), Some(1));
    }

    #[test]
    fn normalizar_renumera_ids_duplicados() {
        let mut list: TaskList = serde_json::from_str(
            r#"{"tasks":[{"id":5,"text":"a","done":false},{"id":5,"text":"b","done":true}],"next_id":5}"#,
        )
        .unwrap();
        list.normalizar();
        let ids: Vec<_> = list.tasks.iter().map(|t| t.id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(list.tasks[1].text, "b");
        assert!(list.tasks[1].done);
        assert!(list.take(1).is_some());
        assert_eq!(
            list.tasks.len(),
            1,
            "apagar não pode levar a duplicata junto"
        );
        assert_eq!(list.add("c"), Some(3));
    }

    #[test]
    fn adicionar_com_id_maximo_nao_da_panic() {
        let mut list: TaskList = serde_json::from_str(&format!(
            r#"{{"tasks":[{{"id":{},"text":"x","done":false}}]}}"#,
            u64::MAX
        ))
        .unwrap();
        list.normalizar();
        let r = std::panic::catch_unwind(move || {
            let mut list = list;
            list.add("y")
        });
        assert!(r.is_ok(), "add deu panic com id u64::MAX");
    }

    #[test]
    fn acentos_e_emoji_preservados() {
        let (mut list, ids) = lista_com(&["Reunião às 15h ✅", "Café ☕ com açúcar"]);
        assert_eq!(list.tasks[0].text, "Reunião às 15h ✅");
        assert_eq!(list.tasks[1].text, "Café ☕ com açúcar");
        assert!(list.edit(ids[0], "  Revisão ção 🚀 "));
        assert_eq!(list.tasks[0].text, "Revisão ção 🚀");
    }

    #[test]
    fn edit_distingue_maiusculas_e_acentos() {
        let (mut list, ids) = lista_com(&["cafe"]);
        assert!(list.edit(ids[0], "café"));
        assert!(list.edit(ids[0], "Café"));
        assert_eq!(list.tasks[0].text, "Café");
    }
}
