//! rust-analyzer: requests from the cursor and showing the results.
use super::*;

/// What an in-flight LSP request was for.
pub(super) enum Req {
    Init,
    Goto,
    Hover,
    /// A name from text (a comment, the PR's): where it's defined, by rust-analyzer's symbol
    /// search; then `then` (references, implementations, its hover) from there, if that's what
    /// was asked.
    Symbol { name: String, then: Option<(&'static str, Box<Req>)> },
}

impl App {
    pub(super) fn lsp_request(&mut self, method: &'static str, req: Req) {
        let row = &self.rows[self.head.row];
        // In text (a comment, the PR's): the name under the cursor, found by name.
        if matches!(row.line.kind, Kind::Comment | Kind::Thread) || row.file == INFO {
            return self.lsp_by_name(method, req);
        }
        let (Some(n), Some(f)) = (row.line.new, self.files.get(row.file)) else {
            self.msg = "not on a new-side line".into();
            return;
        };
        let (path, character) = (self.root.join(&f.path), lsp::utf16_col(&row.line.text, self.head.col));
        match &self.lsp {
            None => self.msg = "rust-analyzer is not running".into(),
            Some(_) if !self.lsp_ready => self.msg = "rust-analyzer is starting…".into(),
            Some(_) => self.ask_at(method, &path, n - 1, character, req, None),
        }
    }

    /// Ask rust-analyzer `method` at `path`'s `line`, `character` (0-based, UTF-16), for `req`;
    /// said till the answer comes (`of`: the name it's for).
    fn ask_at(&mut self, method: &'static str, path: &std::path::Path, line: u32, character: u32, req: Req, of: Option<&str>) {
        let Some(l) = &mut self.lsp else { return };
        let mut params = json!({
            "textDocument": { "uri": lsp::uri(path) },
            "position": { "line": line, "character": character },
        });
        if method.ends_with("references") {
            params["context"] = json!({ "includeDeclaration": true });
        }
        match l.request(method, params) {
            Ok(id) => {
                // Cleared when the answer comes (rust-analyzer can take a while on a big crate).
                let what = match method.rsplit('/').next() {
                    Some("definition") => "the definition",
                    Some("typeDefinition") => "the type",
                    Some("implementation") => "implementations",
                    Some("references") => "references",
                    _ => "",
                };
                self.msg = match (what, of) {
                    ("", _) => "asking rust-analyzer…".into(),
                    (w, Some(name)) => format!("finding {w} of {name}…"),
                    (w, None) => format!("finding {w}…"),
                };
                self.inflight.insert(id, req);
            }
            Err(e) => self.msg = format!("rust-analyzer: {e}"),
        }
    }

    /// `gd` / `gy` / `gi` / `gr` / `Space k` on a name in text: rust-analyzer's symbol search
    /// for it (its exact matches), then, for references / implementations / its hover, those of
    /// what it found. Without rust-analyzer (or while it starts), the definition by text
    /// search, as Enter finds it.
    fn lsp_by_name(&mut self, method: &'static str, req: Req) {
        let Some(name) = self.name_at_cursor() else { return self.msg = "no name under the cursor".into() };
        let then = match method.rsplit('/').next() {
            Some("references" | "implementation" | "hover") => Some((method, Box::new(req))),
            _ => None, // the definition (a type's is itself)
        };
        match &mut self.lsp {
            Some(l) if self.lsp_ready => match l.request("workspace/symbol", json!({ "query": name })) {
                Ok(id) => {
                    self.msg = format!("finding {name}…");
                    self.inflight.insert(id, Req::Symbol { name, then });
                }
                Err(e) => self.msg = format!("rust-analyzer: {e}"),
            },
            _ if then.is_none() => self.goto_definition(&name),
            None => self.msg = "rust-analyzer is not running (Enter finds the definition by text)".into(),
            Some(_) => self.msg = "rust-analyzer is starting… (Enter finds the definition by text meanwhile)".into(),
        }
    }

    /// rust-analyzer's news; gone (exited, crashed, never started): let go, said, so nothing
    /// waits on it for good (the text fallbacks then).
    pub(super) fn lsp_poll(&mut self) {
        let Some(l) = &self.lsp else { return };
        let mut msgs = vec![];
        let gone = loop {
            match l.rx.try_recv() {
                Ok(m) => msgs.push(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for m in msgs {
            self.lsp_msg(m);
        }
        if gone && self.lsp.is_some() {
            self.lsp_gone("rust-analyzer exited");
        }
    }

    fn lsp_gone(&mut self, why: &str) {
        (self.lsp, self.lsp_ready) = (None, false);
        self.inflight.clear();
        self.progress.clear();
        self.msg = why.into();
    }

    pub(super) fn lsp_msg(&mut self, m: Value) {
        let Some(l) = &mut self.lsp else { return };
        match (m.get("id"), m["method"].as_str()) {
            (Some(id), Some(_)) => _ = l.reply(id.clone()),
            (None, Some("$/progress")) => {
                let (tok, v) = (m["params"]["token"].to_string(), &m["params"]["value"]);
                if v["kind"] == "end" {
                    self.progress.remove(&tok);
                } else {
                    let e = self.progress.entry(tok).or_default();
                    if let Some(t) = v["title"].as_str() {
                        e.0 = t.to_owned();
                    }
                    e.1 = v["message"].as_str().unwrap_or("").to_owned();
                }
            }
            (Some(id), None) => {
                let Some(req) = id.as_u64().and_then(|id| self.inflight.remove(&id)) else { return };
                if let Some(err) = m.get("error") {
                    let why = format!("rust-analyzer: {}", err["message"].as_str().unwrap_or("error"));
                    // (Not started: let go, not "starting" for good.)
                    return match req {
                        Req::Init => self.lsp_gone(&why),
                        _ => self.msg = why,
                    };
                }
                let res = &m["result"];
                if !matches!(req, Req::Init) {
                    self.msg.clear(); // the "finding …" message
                }
                match req {
                    Req::Init => {
                        self.lsp_ready = true;
                        _ = l.notify("initialized", json!({}));
                    }
                    Req::Hover => match lsp::hover_text(res).trim().to_owned() {
                        t if t.is_empty() => self.msg = "no hover info".into(),
                        t => self.popup = Some((" hover  j/k scroll ".into(), self.hover_lines(&t), 0)),
                    },
                    Req::Goto => self.show_locations(lsp::locations(res)),
                    Req::Symbol { name, then } => {
                        // Its exact matches (the search is fuzzy), as locations.
                        let found: Vec<Value> = res.as_array().into_iter().flatten().filter(|s| s["name"] == name.as_str()).map(|s| s["location"].clone()).collect();
                        let locs = lsp::locations(&Value::Array(found));
                        match (then, locs.first()) {
                            // Not a symbol it knows: its definition by text; nothing else to
                            // go on (its references, its hover: not the definition instead).
                            (None, None) => self.goto_definition(&name),
                            (Some(_), None) => self.msg = format!("rust-analyzer doesn't know {name}"),
                            (None, _) => self.show_locations(locs),
                            (Some((method, req)), Some(at)) => {
                                let (path, line, character) = (at.path.clone(), at.line, at.character);
                                self.ask_at(method, &path, line, character, *req, Some(&name));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn show_locations(&mut self, locs: Vec<Loc>) {
        match locs.as_slice() {
            [] => self.msg = "no results".into(),
            [l] => self.jump_loc(l),
            _ => {
                let mut cache = HashMap::new();
                let items = locs
                    .into_iter()
                    .map(|l| {
                        let mark = if self.loc_row(&l).is_some() { "●" } else { " " };
                        let shown = l.path.strip_prefix(&self.root).unwrap_or(&l.path).display().to_string();
                        let text = disk_line(&mut cache, &l.path, l.line);
                        let label = format!("{mark} {shown}:{}  {}", l.line + 1, text.trim());
                        Item::new(label, Target::Loc(l))
                    })
                    .collect();
                self.picker = Some(Picker::new(" locations  ● = in diff ", items));
            }
        }
    }

    /// The diff row showing this location's line, if it is in the (expanded) diff.
    pub(super) fn loc_row(&self, l: &Loc) -> Option<usize> {
        let rel = l.path.strip_prefix(&self.root).ok()?.to_str()?;
        let fi = self.files.iter().position(|f| f.path == rel)?;
        self.rows.iter().position(|r| r.file == fi && r.line.new == Some(l.line + 1))
    }

    pub(super) fn jump_loc(&mut self, l: &Loc) {
        // In a diff file that's folded, or outside what the Claude / pending view shows: show it.
        let rel = l.path.strip_prefix(&self.root).ok().and_then(|p| p.to_str()).map(str::to_owned);
        if self.loc_row(l).is_none()
            && let Some(p) = rel.filter(|p| self.files.iter().any(|f| &f.path == p))
        {
            self.show_file(&p, |a| a.loc_row(l).is_some());
        }
        match self.loc_row(l) {
            Some(r) => {
                let col = lsp::char_col(&self.rows[r].line.text, l.character);
                self.jump(r, col);
            }
            None => {
                let col = lsp::char_col(&disk_line(&mut HashMap::new(), &l.path, l.line), l.character);
                self.action = Some(Action::Open(format!("{}:{}:{}", l.path.display(), l.line + 1, col + 1)));
            }
        }
    }
}
