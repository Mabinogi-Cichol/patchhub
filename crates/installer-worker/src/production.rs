//! Host-owned launcher for the bundled SF parser process.
use crate::{sf, WorkerError};
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Constructed by the trusted host with its bundled parser executable.
#[derive(Debug, Clone)]
pub struct WorkerRuntime {
    worker_exe: PathBuf,
}
#[derive(Debug)]
pub struct WorkerExecution {
    pub response: Value,
}
/// Analysis and extraction share the host's original input, held read-only.
pub struct WorkerSession {
    worker_exe: PathBuf,
    archive: PathBuf,
    _input_lock: File,
}
pub(crate) fn err(stage: &str, message: impl Into<String>) -> WorkerError {
    WorkerError::new(stage, None, message)
}
pub(crate) fn ioerr(stage: &str, e: std::io::Error) -> WorkerError {
    WorkerError::new(stage, e.raw_os_error().map(|n| n as u32), e.to_string())
}
impl WorkerRuntime {
    pub fn new(worker_exe: impl Into<PathBuf>) -> Result<Self, WorkerError> {
        let worker_exe = worker_exe.into();
        if !worker_exe.is_absolute() {
            return Err(err("path", "bundled worker path must be absolute"));
        }
        // Existence and execution permission are checked by the actual spawn.
        Ok(Self { worker_exe })
    }
    pub fn execute(
        &self,
        archive: &Path,
        descriptor: &[u8],
        cancel: &AtomicBool,
    ) -> Result<WorkerExecution, WorkerError> {
        cancelled(cancel)?;
        let parsed = sf::parse_descriptor(descriptor).map_err(|e| err("descriptor", e.0))?;
        let (hash, size) = match &parsed {
            sf::Descriptor::Analyze {
                exe_sha256,
                exe_size,
            }
            | sf::Descriptor::AnalyzeBehavior {
                exe_sha256,
                exe_size,
            } => (exe_sha256, *exe_size),
            sf::Descriptor::ExtractBehavior { .. } => {
                return Err(err("descriptor", "extraction requires a streaming session"))
            }
        };
        self.open_session(archive, hash, size, cancel)?
            .analyze(descriptor, cancel)
    }
    pub fn open_session(
        &self,
        archive: &Path,
        _exe_sha256: &str,
        exe_size: u64,
        cancel: &AtomicBool,
    ) -> Result<WorkerSession, WorkerError> {
        cancelled(cancel)?;
        if !archive.is_absolute() {
            return Err(err("input", "input path must be absolute"));
        }
        let input_lock = sf::open_input(archive).map_err(|e| err("input", e.0))?;
        if input_lock.metadata().map_err(|e| ioerr("input", e))?.len() != exe_size {
            return Err(err("input", "input size mismatch"));
        }
        // The parser checks the expected digest against this input for every action.
        Ok(WorkerSession {
            worker_exe: self.worker_exe.clone(),
            archive: archive.to_owned(),
            _input_lock: input_lock,
        })
    }
}
impl WorkerSession {
    pub fn analyze(
        &self,
        descriptor: &[u8],
        cancel: &AtomicBool,
    ) -> Result<WorkerExecution, WorkerError> {
        cancelled(cancel)?;
        let outcome =
            crate::process::run(&self.worker_exe, &self.archive, descriptor, cancel, None)?;
        cancelled(cancel)?;
        let response: StrictValue = serde_json::from_slice(&outcome.stdout).map_err(|e| {
            err(
                "response",
                format!(
                    "{e}; exit={}; stderr={}",
                    outcome.status,
                    String::from_utf8_lossy(&outcome.stderr)
                ),
            )
        })?;
        if !outcome.status.success() {
            return Err(err(
                "worker",
                response
                    .0
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "worker failed ({}): {}",
                            outcome.status,
                            String::from_utf8_lossy(&outcome.stderr).trim()
                        )
                    }),
            ));
        }
        validate_response(&response.0, descriptor)?;
        Ok(WorkerExecution {
            response: response.0,
        })
    }
    /// Feed stdout chunks to the caller without retaining extracted member bytes.
    pub fn extract(
        &self,
        descriptor: &[u8],
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<(), WorkerError>,
    ) -> Result<(), WorkerError> {
        cancelled(cancel)?;
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        std::thread::scope(|scope| {
            let launched = scope.spawn(|| {
                crate::process::run(
                    &self.worker_exe,
                    &self.archive,
                    descriptor,
                    cancel,
                    Some(tx),
                )
            });
            let mut consume_error = None;
            while let Ok(chunk) = rx.recv() {
                if let Err(e) = consume(&chunk) {
                    consume_error = Some(e);
                    break;
                }
            }
            drop(rx);
            let outcome = launched
                .join()
                .map_err(|_| err("worker", "worker supervisor panicked"))?;
            if let Some(e) = consume_error {
                return Err(e);
            }
            let outcome = outcome?;
            cancelled(cancel)?;
            if !outcome.status.success() {
                return Err(err(
                    "extract_worker",
                    format!(
                        "worker failed ({}): {}",
                        outcome.status,
                        String::from_utf8_lossy(&outcome.stderr).trim()
                    ),
                ));
            }
            Ok(())
        })
    }
}
pub(crate) fn cancelled(cancel: &AtomicBool) -> Result<(), WorkerError> {
    if cancel.load(Ordering::Acquire) {
        Err(err("cancelled", "worker cancellation requested"))
    } else {
        Ok(())
    }
}
fn validate_response(v: &Value, raw: &[u8]) -> Result<(), WorkerError> {
    let o = v
        .as_object()
        .ok_or_else(|| err("response", "invalid response object"))?;
    let d = sf::parse_descriptor(raw).map_err(|e| err("descriptor", e.0))?;
    let (action, hash) = match &d {
        sf::Descriptor::Analyze { exe_sha256, .. } => ("analyze", exe_sha256),
        sf::Descriptor::AnalyzeBehavior { exe_sha256, .. } => ("analyze_behavior", exe_sha256),
        sf::Descriptor::ExtractBehavior { .. } => {
            return Err(err("response", "extraction has no buffered response"))
        }
    };
    let keys = [
        "schema_version",
        "action",
        "exe_sha256",
        "report",
        "installable",
    ];
    if o.len() != keys.len()
        || keys.iter().any(|k| !o.contains_key(*k))
        || v["exe_sha256"].as_str() != Some(hash)
        || v["schema_version"] != 1
        || v["action"] != action
        || v["installable"] != false
        || !v["report"].is_object()
    {
        return Err(err(
            "response",
            "worker response fields/input binding mismatch",
        ));
    }
    Ok(())
}
/// serde_json::Value accepts duplicate object keys. IPC must reject them at
/// every depth rather than silently taking the last untrusted report binding.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite JSON"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(item) = seq.next_element::<StrictValue>()? {
                    v.push(item.0);
                }
                Ok(StrictValue(Value::Array(v)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut v = serde_json::Map::new();
                while let Some((key, item)) = map.next_entry::<String, StrictValue>()? {
                    if v.insert(key, item.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(StrictValue(Value::Object(v)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
#[cfg(test)]
mod production_tests {
    use super::*;
    #[test]
    fn ipc_rejects_duplicate_keys_at_all_depths_and_trailing_objects() {
        for raw in [
            br#"{"exe_sha256":"a","exe_sha256":"b"}"#.as_slice(),
            br#"{"report":{"binding":1,"binding":2}}"#.as_slice(),
            br#"{}{}"#.as_slice(),
        ] {
            assert!(serde_json::from_slice::<StrictValue>(raw).is_err());
        }
        assert!(serde_json::from_slice::<StrictValue>(br#"{"report":{"binding":1}}"#).is_ok());
    }
    #[test]
    fn response_cannot_authorize_or_change_action_and_input_binding() {
        let raw = serde_json::to_vec(
            &serde_json::json!({"action":"analyze","exe_size":1,"exe_sha256":"a".repeat(64)}),
        )
        .unwrap();
        let good = serde_json::json!({"schema_version":1,"action":"analyze","exe_sha256":"a".repeat(64),"report":{},"installable":false});
        assert!(validate_response(&good, &raw).is_ok());
        for (key, value) in [
            ("installable", serde_json::json!(true)),
            ("exe_sha256", serde_json::json!("b".repeat(64))),
            ("action", serde_json::json!("convert_behavior")),
            ("extra_path", serde_json::json!("/game")),
        ] {
            let mut changed = good.clone();
            changed[key] = value;
            assert!(validate_response(&changed, &raw).is_err());
        }
    }
}
