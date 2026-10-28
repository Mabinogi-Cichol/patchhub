//! Bundled SF parser and cancellable child-process runtime.
pub mod model;
mod process;
pub mod production;
pub mod sf;
pub use model::WorkerError;
