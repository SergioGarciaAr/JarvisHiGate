pub mod ollama;
mod session;

pub use ollama::{ChatMessage, ChatOutcome, ChatRequest, Finish, ModelInfo, OllamaClient, OllamaError, OllamaErrorInfo, OllamaHealth, Role};
pub use session::ChatSession;
