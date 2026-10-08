//! Bounded static Setup Factory parsing. No scripts or installer code are executed.
pub mod audit;
pub mod behavior;
pub mod graph;
pub mod lua;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(format!("io_error:{e}"))
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn require(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(code.to_owned()))
    }
}
