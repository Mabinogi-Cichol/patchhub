//! Wall-clock helper for read-only projection.
use std::{
    io,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid_system_clock"))
}
