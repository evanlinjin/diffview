//! rust-analyzer: requests from the cursor and showing the results.
use super::*;

impl App {
    pub(super) fn lsp_request(&mut self, method: &str, req: Req) {
        let row = &self.rows[self.head.row];
        let (Some(n), Some(f)) = (row.line.new, self.files.get(row.file)) else {
            self.msg = "not on a new-side line".into();
            return;
        };
        let mut params = json!({
            "textDocument": { "uri": lsp::uri(&self.root.join(&f.path)) },
            "position": { "line": n - 1, "character": lsp::utf16_col(&row.line.text, self.head.col) },
        });
        if method.ends_with("references") {
            params["context"] = json!({ "includeDeclaration": true });
        }
        match &mut self.lsp {
            None => self.msg = "rust-analyzer is not running".into(),
            Some(_) if !self.lsp_ready => self.msg = "rust-analyzer is starting…".into(),
            Some(l) => match l.request(method, params) {
                Ok(id) => {
                    // Cleared when the answer comes (rust-analyzer can take a while on a big crate).
                    self.msg = match method.rsplit('/').next() {
                        Some("definition") => "finding the definition…",
                        Some("typeDefinition") => "finding the type…",
                        Some("implementation") => "finding implementations…",
                        Some("references") => "finding references…",
                        _ => "asking rust-analyzer…",
                    }
                    .into();
                    self.inflight.insert(id, req);
                }
                Err(e) => self.msg = format!("rust-analyzer: {e}"),
            },
        }
    }

    pub(super) fn lsp_poll(&mut self) {
        let msgs: Vec<Value> = self.lsp.as_ref().map(|l| l.rx.try_iter().collect()).unwrap_or_default();
        for m in msgs {
            self.lsp_msg(m);
        }
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
                    self.msg = format!("rust-analyzer: {}", err["message"].as_str().unwrap_or("error"));
                    return;
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
                self.picker = Some(Picker { title: " locations  ● = in diff ", items, query: String::new(), sel: 0 });
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
        if self.loc_row(l).is_none() && rel.as_ref().is_some_and(|p| self.files.iter().any(|f| &f.path == p)) {
            let p = rel.unwrap_or_default();
            if self.collapsed.remove(&p) || self.view != View::All {
                self.view = View::All;
                let keep = self.cursor_key();
                self.rebuild(keep);
            }
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
