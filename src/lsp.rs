//! Minimal LSP client for rust-analyzer: JSON-RPC over stdio, messages read on a thread.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

pub struct Lsp {
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
    pub rx: Receiver<Value>,
}

/// A location from the server. `line` is 0-based, `character` is in UTF-16 units.
#[derive(Clone, Debug)]
pub struct Loc {
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
}

impl Lsp {
    /// Spawn rust-analyzer and send `initialize`; returns the request id to wait on.
    pub fn start(root: &Path) -> Result<(Self, u64)> {
        let mut child = Command::new("rust-analyzer")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting rust-analyzer")?;
        let stdin = child.stdin.take().context("rust-analyzer stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("rust-analyzer stdout")?);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(v) = read_msg(&mut stdout) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let mut lsp = Lsp { child, stdin, next_id: 0, rx };
        let id = lsp.request("initialize", json!({
            "processId": std::process::id(),
            "rootUri": uri(root),
            "capabilities": {
                "window": { "workDoneProgress": true },
                "textDocument": { "hover": { "contentFormat": ["markdown", "plaintext"] } },
            },
            // We never send didChange, so let the server watch the disk itself.
            "initializationOptions": { "files": { "watcher": "server" } },
        }))?;
        Ok((lsp, id))
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params }))?;
        Ok(self.next_id)
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Acknowledge a server→client request (progress tokens, registrations) with a null result.
    pub fn reply(&mut self, id: Value) -> Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": null }))
    }

    fn send(&mut self, v: Value) -> Result<()> {
        let s = v.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{s}", s.len())?;
        self.stdin.flush()?;
        Ok(())
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_msg(r: &mut impl BufRead) -> Option<Value> {
    let mut len = 0;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length: ") {
            len = n.parse().ok()?;
        }
    }
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).ok()?;
    Some(serde_json::from_slice(&buf).unwrap_or(Value::Null))
}

/// Parse a definition/references result: Location, Location[], or LocationLink[].
pub fn locations(v: &Value) -> Vec<Loc> {
    let items: &[Value] = match v {
        Value::Array(a) => a,
        Value::Null => &[],
        o => std::slice::from_ref(o),
    };
    items
        .iter()
        .filter_map(|l| {
            let uri = l.get("uri").or(l.get("targetUri"))?.as_str()?;
            let start = &l.get("range").or(l.get("targetSelectionRange"))?["start"];
            Some(Loc {
                path: path(uri),
                line: start["line"].as_u64()? as u32,
                character: start["character"].as_u64()? as u32,
            })
        })
        .collect()
}

pub fn hover_text(v: &Value) -> String {
    let one = |c: &Value| c.as_str().or(c.get("value").and_then(Value::as_str)).unwrap_or("").to_owned();
    match &v["contents"] {
        Value::Array(a) => a.iter().map(one).collect::<Vec<_>>().join("\n\n"),
        c => one(c),
    }
}

pub fn uri(p: &Path) -> String {
    let mut s = String::from("file://");
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn path(uri: &str) -> PathBuf {
    let s = uri.strip_prefix("file://").unwrap_or(uri).as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < s.len() {
        let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (s[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

/// Char index → UTF-16 offset (LSP's default position encoding).
pub fn utf16_col(text: &str, col: usize) -> u32 {
    text.chars().take(col).map(|c| c.len_utf16() as u32).sum()
}

/// UTF-16 offset → char index.
pub fn char_col(text: &str, u: u32) -> usize {
    let mut acc = 0;
    for (i, c) in text.chars().enumerate() {
        if acc >= u {
            return i;
        }
        acc += c.len_utf16() as u32;
    }
    text.chars().count()
}

#[test]
fn conversions() {
    let p = Path::new("/a b/ü.rs");
    assert_eq!(uri(p), "file:///a%20b/%C3%BC.rs");
    assert_eq!(path(&uri(p)), p);
    let t = "🦀x = 1";
    assert_eq!(utf16_col(t, 1), 2);
    assert_eq!(char_col(t, 2), 1);
    let hits = locations(&json!([{ "uri": "file:///x.rs", "range": { "start": { "line": 3, "character": 4 } } }]));
    assert_eq!((hits[0].line, hits[0].character), (3, 4));
}
