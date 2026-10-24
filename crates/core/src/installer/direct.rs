//! Host-side consumer of the worker member stream for direct EXE extraction.
//!
//! The worker emits one JSON line, exactly N raw bytes, one JSON trailer per
//! member, then a final done line. This module writes those bytes straight to
//! the resolved game targets; it never creates a staging copy or archive.
use crate::{
    install::{self, DirectTarget},
    installer::exe_report::PayloadEntry,
    operation,
};
use patchhub_installer_worker::IsolationError;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

fn err(message: impl Into<String>) -> IsolationError {
    IsolationError::new("extract", None, message)
}

struct Current {
    index: usize,
    file: BufWriter<File>,
}

enum Parse {
    Line,
    Data { remaining: u64 },
}

pub(crate) struct DirectWriter<'a> {
    expected: &'a [PayloadEntry],
    targets: &'a [DirectTarget],
    line: Vec<u8>,
    parse: Parse,
    current: Option<Current>,
    next: usize,
    started: bool,
    done: bool,
    installed: Vec<Option<String>>,
    parents: BTreeSet<PathBuf>,
}

impl<'a> DirectWriter<'a> {
    pub fn new(expected: &'a [PayloadEntry], targets: &'a [DirectTarget]) -> Self {
        Self {
            expected,
            targets,
            line: Vec::new(),
            parse: Parse::Line,
            current: None,
            next: 0,
            started: false,
            done: false,
            installed: vec![None; expected.len()],
            parents: BTreeSet::new(),
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), IsolationError> {
        operation::check().map_err(|e| IsolationError::new("cancelled", None, e.to_string()))?;
        if self.done {
            return Err(err("member data after done"));
        }
        let mut at = 0usize;
        while at < chunk.len() {
            match self.parse {
                Parse::Data { remaining } => {
                    let count = remaining.min((chunk.len() - at) as u64) as usize;
                    let current = self.current.as_mut().ok_or_else(|| err("missing member"))?;
                    current
                        .file
                        .write_all(&chunk[at..at + count])
                        .map_err(|e| err(format!("game write failed: {e}")))?;
                    operation::advance(count as u64);
                    at += count;
                    self.parse = if remaining as usize == count {
                        Parse::Line
                    } else {
                        Parse::Data {
                            remaining: remaining - count as u64,
                        }
                    };
                }
                Parse::Line => {
                    let rest = &chunk[at..];
                    let Some(newline) = rest.iter().position(|b| *b == b'\n') else {
                        self.line.extend_from_slice(rest);
                        if self.line.len() > 64 * 1024 {
                            return Err(err("member header too large"));
                        }
                        return Ok(());
                    };
                    self.line.extend_from_slice(&rest[..newline]);
                    at += newline + 1;
                    let line = std::mem::take(&mut self.line);
                    self.object(&line)?;
                }
            }
        }
        Ok(())
    }

    fn object(&mut self, line: &[u8]) -> Result<(), IsolationError> {
        let value: Value =
            serde_json::from_slice(line).map_err(|e| err(format!("bad frame: {e}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| err("frame is not an object"))?;
        if let Some(extract) = object.get("extract") {
            if self.started || extract.as_u64() != Some(1) {
                return Err(err("bad extract header"));
            }
            let members = object
                .get("members")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("missing member count"))?;
            if members != self.targets.len() as u64 {
                return Err(err("member count changed"));
            }
            self.started = true;
            return Ok(());
        }
        if object.get("done").is_some() {
            if object.get("done").and_then(Value::as_bool) != Some(true)
                || !self.started
                || self.next != self.expected.len()
                || self.current.is_some()
            {
                return Err(err("incomplete extract stream"));
            }
            self.done = true;
            return Ok(());
        }
        if let Some(index) = object.get("index").and_then(Value::as_u64) {
            let index = index as usize;
            if index != self.next {
                return Err(err("member order changed"));
            }
            if let Some(sha) = object.get("sha256") {
                return self.trailer(index, object, sha);
            }
            return self.header(index, object);
        }
        Err(err("unknown extract frame"))
    }

    fn header(
        &mut self,
        index: usize,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), IsolationError> {
        if self.current.is_some() {
            return Err(err("overlapping members"));
        }
        let expected = self
            .expected
            .get(index)
            .ok_or_else(|| err("unexpected member"))?;
        let target = self
            .targets
            .get(index)
            .ok_or_else(|| err("missing target"))?;
        if object.get("path").and_then(Value::as_str) != Some(expected.path.as_str())
            || object.get("size").and_then(Value::as_u64) != Some(expected.size)
            || target.path != expected.path
        {
            return Err(err("member identity changed"));
        }
        operation::file(index, &expected.path);
        let file = install::open_direct_target(&target.file)
            .map_err(|e| err(format!("game write failed: {e}")))?;
        self.current = Some(Current {
            index,
            file: BufWriter::new(file),
        });
        self.parse = Parse::Data {
            remaining: expected.size,
        };
        Ok(())
    }

    fn trailer(
        &mut self,
        index: usize,
        object: &serde_json::Map<String, Value>,
        sha: &Value,
    ) -> Result<(), IsolationError> {
        let expected = self
            .expected
            .get(index)
            .ok_or_else(|| err("unexpected member"))?;
        let sha = sha.as_str().ok_or_else(|| err("missing digest"))?;
        if object.get("size").and_then(Value::as_u64) != Some(expected.size)
            || object.get("crc32").and_then(Value::as_u64) != Some(expected.crc32 as u64)
            || sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(err("member content changed"));
        }
        let current = self.current.take().ok_or_else(|| err("missing member"))?;
        if current.index != index {
            return Err(err("member order changed"));
        }
        let file = current
            .file
            .into_inner()
            .map_err(|e| err(format!("game write failed: {e}")))?;
        file.sync_all()
            .map_err(|e| err(format!("game sync failed: {e}")))?;
        if let Some(parent) = self.targets[index].file.parent() {
            self.parents.insert(parent.to_path_buf());
        }
        self.installed[index] = Some(sha.to_owned());
        self.next += 1;
        operation::finish_file(self.next);
        Ok(())
    }

    pub fn opened_any(&self) -> bool {
        self.current.is_some() || self.next > 0
    }

    pub fn finish(self) -> Result<Vec<(String, String)>, IsolationError> {
        if !self.started || !self.done || self.current.is_some() || self.next != self.expected.len()
        {
            return Err(err("incomplete extract stream"));
        }
        let mut out = Vec::with_capacity(self.expected.len());
        for (index, sha) in self.installed.into_iter().enumerate() {
            out.push((
                self.expected[index].path.clone(),
                sha.ok_or_else(|| err("missing member digest"))?,
            ));
        }
        for parent in self.parents {
            install::sync_directory(&parent)
                .map_err(|e| err(format!("directory sync failed: {e}")))?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn expected() -> Vec<PayloadEntry> {
        vec![PayloadEntry {
            path: "a/b.bin".into(),
            size: 5,
            crc32: 907060870,
            payload_offset: 0,
            packed_size: 0,
            compressed: false,
        }]
    }

    fn frame() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"{\"extract\":1,\"members\":1}\n");
        out.extend_from_slice(b"{\"index\":0,\"path\":\"a/b.bin\",\"size\":5}\n");
        out.extend_from_slice(b"hello");
        out.extend_from_slice(
            b"{\"index\":0,\"size\":5,\"crc32\":907060870,\"sha256\":\"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\"}\n",
        );
        out.extend_from_slice(b"{\"done\":true}\n");
        out
    }

    #[test]
    fn frames_can_split_anywhere_and_write_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let expected = expected();
        let targets = vec![DirectTarget {
            path: expected[0].path.clone(),
            file: temp.path().join(&expected[0].path),
        }];
        let mut writer = DirectWriter::new(&expected, &targets);
        for chunk in frame().chunks(3) {
            writer.feed(chunk).unwrap_or_else(|e| panic!("{e}"));
        }
        let installed = writer.finish().unwrap();
        assert_eq!(installed[0].1.len(), 64);
        assert_eq!(fs::read(&targets[0].file).unwrap(), b"hello");
    }

    #[test]
    fn wrong_crc_is_rejected_and_partial_output_stays() {
        let temp = tempfile::tempdir().unwrap();
        let expected = expected();
        let targets = vec![DirectTarget {
            path: expected[0].path.clone(),
            file: temp.path().join(&expected[0].path),
        }];
        let mut writer = DirectWriter::new(&expected, &targets);
        let mut data = frame();
        let at = data.windows(9).position(|w| w == b"907060870").unwrap();
        data[at..at + 9].copy_from_slice(b"907060871");
        assert!(writer.feed(&data).is_err());
        drop(writer);
        assert_eq!(fs::read(&targets[0].file).unwrap(), b"hello");
    }
}
