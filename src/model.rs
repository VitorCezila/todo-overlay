//! Tarefas e lista de tarefas — lógica pura, sem dependência de UI.

#![allow(dead_code)] // remover quando a UI usar o modelo (marco 2)

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
    next_id: u64,
}

impl TaskList {
    /// Adiciona uma tarefa. Texto vazio ou só com espaços é ignorado.
    pub fn add(&mut self, text: &str) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.tasks.push(Task {
            id,
            text: text.to_owned(),
            done: false,
        });
        Some(id)
    }

    pub fn toggle(&mut self, id: u64) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.done = !t.done;
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.tasks.retain(|t| t.id != id);
    }

    pub fn pending_count(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
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
        list.toggle(a);
        assert_eq!(list.pending_count(), 1);
        list.remove(b);
        assert_eq!(list.tasks.len(), 1);
        assert!(list.tasks[0].done);
    }
}
