//! In-memory conversation context. Nothing is persisted to disk.

use super::ollama::{ChatMessage, Role};

pub const SYSTEM_PROMPT: &str = "Eres Jarvis, un asistente local que se ejecuta en el Mac del usuario mediante Ollama. \
Responde en el idioma del usuario, de forma clara y concisa. No afirmes haber ejecutado acciones en el sistema: \
en esta versión solo puedes conversar.";

pub struct ChatSession {
    messages: Vec<ChatMessage>,
    max_turns: usize,
}

impl Default for ChatSession {
    fn default() -> Self {
        Self::new(20)
    }
}

impl ChatSession {
    /// Keeps at most `max_turns` user/assistant pairs besides the system prompt.
    pub fn new(max_turns: usize) -> Self {
        Self { messages: Vec::new(), max_turns }
    }

    /// Messages to send for a new user turn (history + the new message).
    pub fn request_messages(&self, user_text: &str) -> Vec<ChatMessage> {
        let mut out = Vec::with_capacity(self.messages.len() + 2);
        out.push(ChatMessage::new(Role::System, SYSTEM_PROMPT));
        out.extend(self.messages.iter().cloned());
        out.push(ChatMessage::new(Role::User, user_text));
        out
    }

    /// Records a completed (or partially cancelled) turn.
    pub fn record_turn(&mut self, user_text: &str, assistant_text: &str) {
        self.messages.push(ChatMessage::new(Role::User, user_text));
        self.messages.push(ChatMessage::new(Role::Assistant, assistant_text));
        let max = self.max_turns * 2;
        if self.messages.len() > max {
            let excess = self.messages.len() - max;
            self.messages.drain(..excess);
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
