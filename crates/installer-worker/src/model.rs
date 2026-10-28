//! Errors returned by the bundled worker runtime.
use std::fmt;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerError {
    pub stage: String,
    pub code: Option<u32>,
    pub message: String,
}
impl WorkerError {
    pub fn new(stage: impl Into<String>, code: Option<u32>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            code,
            message: message.into(),
        }
    }
}
impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "[{}] {} (OS error {})", self.stage, self.message, code),
            None => write!(f, "[{}] {}", self.stage, self.message),
        }
    }
}
impl std::error::Error for WorkerError {}
