//! Persistência de `%APPDATA%\todo-overlay\tasks.json`.
//!
//! Marco 2: load/save com gravação atômica (tmp + rename), campo `version`,
//! arquivo corrompido → `.bak` + lista vazia. Ver skill `migrar-dados`.
