//! The agent's side of LSP queries: its answers, as text for it. The
//! screen's side is in `lsp.rs`; `lsp/` hands both the same answers and
//! never knows who asked (see docs/adr/0001-lsp-answers-are-structured.md).

use std::collections::BTreeSet;
use std::path::Path;

use super::Editor;
use super::agent::short_path;
use super::lsp::{
    ActionList, Answer, Applied, Busy, Calls, Edits, Location, LspFeature, LspQuery, LspRequest,
    NoAnswer, without_html,
};

impl Editor {
    /// The answer to the agent's tool call `call`, or why there's none.
    pub(super) fn agent_lsp_answered(
        &mut self,
        call: u64,
        request: &LspRequest,
        answer: Result<Answer, NoAnswer>,
    ) {
        if let LspQuery::MoveFile { to } = &request.query {
            self.agent_move(call, request.document, to, answer);
            return;
        }
        let text = match answer {
            Ok(Answer::Edits(edits)) if request.feature() == LspFeature::Format => {
                self.agent_format(call, request, edits);
                return;
            }
            Ok(Answer::Edits(edits)) => {
                self.agent_lsp_edits(call, request, edits);
                return;
            }
            // Only the screen asks for these.
            Ok(
                Answer::Completion { .. }
                | Answer::Signature(_)
                | Answer::InlayHints(_)
                | Answer::Highlights(_)
                | Answer::Outline(_),
            ) => return,
            Ok(Answer::Action { title, edits }) => {
                self.agent_made_action(call, &title, edits);
                return;
            }
            Ok(Answer::Actions(actions)) => {
                let file_wide = matches!(
                    request.query,
                    LspQuery::CodeActions {
                        file_wide: true,
                        ..
                    }
                );
                if actions.is_empty() {
                    Ok(if file_wide {
                        "No file-wide code actions for this file (organize imports and the like)"
                    } else {
                        "No code actions there"
                    }
                    .into())
                } else {
                    let list = self.keep_code_actions(request, actions);
                    let text = actions_text(&list);
                    self.keep_agent_code_actions(list);
                    Ok(text)
                }
            }
            Ok(Answer::Calls(calls)) => Ok(calls_text(&calls, request.feature())),
            Ok(Answer::Symbols { found, busy }) => {
                Ok(symbols_text(&found, request.feature(), busy.as_ref()))
            }
            Ok(Answer::Hover(markdown)) => Ok(hover_text(&markdown)),
            Ok(Answer::Locations { found, busy }) => {
                Ok(locations_text(found, request.feature(), busy.as_ref()))
            }
            // Nothing there isn't the call failing: it's the answer.
            Err(NoAnswer::NothingThere) => Ok(nothing_there(request.feature())),
            // A code action's command that failed (or whose server
            // stopped): what its edits changed so far is saved.
            Err(why) => match self.end_agent_change(call) {
                Some((saved, _)) => Err(format!(
                    "{}{saved}",
                    no_answer_text(request.feature(), &why)
                )),
                None => Err(no_answer_text(request.feature(), &why)),
            },
        };
        self.agent_lsp_answer(call, text);
    }

    /// The agent's file move, once its server's had its say: its edits for
    /// it (an import's path), then the move. With no server to ask (or
    /// one that doesn't update anything for moves), just the move; one
    /// that failed to answer is taken as having no edits.
    fn agent_move(
        &mut self,
        call: u64,
        document: super::DocumentId,
        to: &Path,
        answer: Result<Answer, NoAnswer>,
    ) {
        let (edits, asked) = match answer {
            Ok(Answer::Edits(edits)) => (Some(edits), true),
            Err(NoAnswer::NoServer | NoAnswer::Unsupported(_)) => (None, false),
            Err(NoAnswer::ServerError { .. }) => (None, true),
            Err(NoAnswer::Unusable(why)) => {
                self.agent_lsp_answer(call, Err(format!("{why}: nothing was moved")));
                return;
            }
            Err(why) => {
                let text = no_answer_text(LspFeature::MoveFile, &why);
                self.agent_lsp_answer(call, Err(text));
                return;
            }
            Ok(_) => (None, true),
        };
        match self.agent_move_document(document, to, edits, asked) {
            Ok((text, changed)) => self.agent_lsp_changed(call, text, changed),
            Err(error) => self.agent_lsp_answer(call, Err(error)),
        }
    }

    /// The agent's formatting, made as an agent change if the text is as
    /// it was when asked.
    fn agent_format(&mut self, call: u64, request: &LspRequest, edits: Edits) {
        let document = request.document;
        let seq = self.document_by_id(document).map(|d| d.buffer().edit_seq());
        if seq != Some(request.edit_seq) {
            let text = "The text changed while formatting: try again";
            self.agent_lsp_answer(call, Err(text.into()));
            return;
        }
        let name = self.agent_names(&[document]);
        let changes: usize = edits.documents.iter().map(|(_, e)| e.len()).sum();
        if changes == 0 {
            self.agent_lsp_answer(call, Ok(format!("{name} is already formatted")));
            return;
        }
        let ((), saved, changed) = self.agent_change(&[], |e| {
            e.apply_lsp_edits(edits);
        });
        let changes = if changes == 1 {
            "1 change".to_string()
        } else {
            format!("{changes} changes")
        };
        let text = format!("Formatted {name} ({changes}){saved}");
        if changed.is_empty() {
            self.agent_lsp_answer(call, Ok(text));
        } else {
            self.agent_lsp_changed(call, text, changed);
        }
    }

    /// A code action made for the agent: its edits as an agent change
    /// (with those its command's server sent as it ran, if any).
    fn agent_made_action(&mut self, call: u64, title: &str, edits: Option<Edits>) {
        let opened = edits.as_ref().map(|e| e.opened.clone()).unwrap_or_default();
        self.begin_agent_change(call, &opened);
        if let Some(edits) = edits {
            self.apply_lsp_edits(edits);
        }
        let (saved, changed) = self.end_agent_change(call).unwrap_or_default();
        let text = if changed.is_empty() {
            format!("Made {title}{saved}")
        } else {
            format!("Made {title} in {}{saved}", self.agent_names(&changed))
        };
        if changed.is_empty() {
            self.agent_lsp_answer(call, Ok(text));
        } else {
            self.agent_lsp_changed(call, text, changed);
        }
    }

    /// A server's edits for the agent (a rename's): made as an agent
    /// change, so the files it changes are saved unless the user has
    /// unsaved work there, and the agent hears of the problems after.
    fn agent_lsp_edits(&mut self, call: u64, request: &LspRequest, edits: Edits) {
        let opened = edits.opened.clone();
        let (applied, saved, changed) = self.agent_change(&opened, |e| e.apply_lsp_edits(edits));
        if applied.changes == 0 {
            self.agent_lsp_answer(call, Err("Nothing to rename there".into()));
            return;
        }
        let name = match &request.query {
            LspQuery::Rename { name } => name.as_str(),
            _ => "",
        };
        let text = format!("Renamed to {name}: {}{saved}", saved_describe(applied));
        if changed.is_empty() {
            self.agent_lsp_answer(call, Ok(text));
        } else {
            self.agent_lsp_changed(call, text, changed);
        }
    }
}

/// What edits did, for the agent: other files aren't left for `:wa`, as
/// the agent change saves them.
fn saved_describe(applied: Applied) -> String {
    Applied {
        others: 0,
        ..applied
    }
    .describe()
}

/// Why there's no answer, for the agent: what to do about it, too.
fn no_answer_text(feature: LspFeature, why: &NoAnswer) -> String {
    match why {
        NoAnswer::NoServer => "No language server for this file".into(),
        NoAnswer::Starting(_) if feature == LspFeature::MoveFile => {
            "The language server is still starting (it'd update the imports): try again in a moment"
                .into()
        }
        NoAnswer::Starting(_) => {
            "The language server is still starting (waited 30 s): try again in a moment".into()
        }
        NoAnswer::Unsupported(name) => format!("{name} doesn't offer {}", feature.noun()),
        NoAnswer::Stopped(name) => format!("{name} stopped before answering"),
        NoAnswer::ServerError {
            server, message, ..
        } if matches!(
            feature,
            LspFeature::ApplyCodeAction | LspFeature::ResolveCodeAction
        ) =>
        {
            format!(
                "{server}: {message} (the file may have changed since: run lsp_code_actions again)"
            )
        }
        NoAnswer::ServerError {
            server, message, ..
        } => format!("{server}: {message}"),
        NoAnswer::NothingThere => nothing_there(feature),
        NoAnswer::Unusable(why) => why.clone(),
    }
}

/// There's nothing at the place asked about.
fn nothing_there(feature: LspFeature) -> String {
    match feature {
        LspFeature::Callers | LspFeature::Callees => {
            "Nothing there to find calls of (put it on a function or method's name)".into()
        }
        _ => "Nothing found".into(),
    }
}

/// A hover's Markdown, without its HTML.
fn hover_text(markdown: &str) -> String {
    let markdown = without_html(markdown);
    if markdown.trim().is_empty() {
        "No hover information there".into()
    } else {
        markdown
    }
}

/// The most places listed.
const PLACES_LISTED: usize = 200;

/// Places, a line each: `path:line:column  text`; references by file and
/// place (servers give them in their own order). Nothing found in the
/// project by a server at work says it may know more in a moment.
fn locations_text(mut found: Vec<Location>, feature: LspFeature, busy: Option<&Busy>) -> String {
    if found.is_empty() {
        let project_wide = matches!(feature, LspFeature::References | LspFeature::Implementation);
        return match busy {
            Some(Busy { server, work }) if project_wide => format!(
                "Nothing found, but {server} is busy ({work}): it may know more in a moment"
            ),
            _ => "Nothing found".into(),
        };
    }
    if feature == LspFeature::References {
        found.sort_by(|a, b| (&a.path, a.position).cmp(&(&b.path, b.position)));
    }
    found
        .iter()
        .take(PLACES_LISTED)
        .map(|l| {
            let path = short_path(&l.path.to_string_lossy());
            format!(
                "{path}:{}:{}  {}",
                l.position.row + 1,
                l.position.col + 1,
                l.line.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Code actions for the agent: numbered, with how to make one.
fn actions_text(list: &ActionList) -> String {
    let lines: Vec<String> = list
        .actions
        .iter()
        .enumerate()
        .map(|(i, a)| match a.kind.as_str() {
            "" => format!("{}. {}", i + 1, a.title),
            kind => format!("{}. {} ({kind})", i + 1, a.title),
        })
        .collect();
    format!(
        "{}\nTo make one: lsp_apply_code_action with list {}, its number and this path.",
        lines.join("\n"),
        list.number
    )
}

/// Symbols for the agent, a line each: `name (what)  path:line:column
/// text`. None found in the project (by `WorkspaceSymbols`) by a server
/// at work says it may know more in a moment.
fn symbols_text(
    found: &[(String, String, Location)],
    feature: LspFeature,
    busy: Option<&Busy>,
) -> String {
    if found.is_empty() {
        return match busy {
            Some(Busy { server, work }) if feature == LspFeature::WorkspaceSymbols => format!(
                "No symbols found, but {server} is busy ({work}): it may know more in a moment"
            ),
            _ => "No symbols found".into(),
        };
    }
    found
        .iter()
        .take(PLACES_LISTED)
        .map(|(name, detail, l)| {
            let path = short_path(&l.path.to_string_lossy());
            format!(
                "{name} ({detail})  {path}:{}:{}  {}",
                l.position.row + 1,
                l.position.col + 1,
                l.line.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The most functions a call hierarchy answer lists.
const CALLS_LISTED: usize = 50;
/// The most of a function's detail (its signature) shown.
const DETAIL_CHARS: usize = 60;

/// The calls, a function each: `name (kind, detail)  path:line`, and the
/// lines the calls are on; the project's own first, sorted, those outside
/// it named on one line.
fn calls_text(calls: &Calls, feature: LspFeature) -> String {
    let callers = feature == LspFeature::Callers;
    // Each in the project: its path as shown, its line, and its text.
    let mut ours: Vec<(String, usize, String)> = Vec::new();
    for call in &calls.ours {
        let path = short_path(&call.path.to_string_lossy());
        let at: Vec<String> = call.at.iter().map(usize::to_string).collect();
        // A caller's calls are in its own file; the callees are called
        // from the function asked about.
        let place = match (callers, at.len()) {
            (_, 0) => String::new(),
            (true, 1) => format!(" (calls it on line {})", at[0]),
            (true, _) => format!(" (calls it on lines {})", at.join(", ")),
            (false, 1) => format!(" (called on line {})", at[0]),
            (false, _) => format!(" (called on lines {})", at.join(", ")),
        };
        // The server's detail (often the signature), cut short.
        let what = match &call.detail {
            Some(detail) => format!("{}, {}", call.kind, short(detail)),
            None => call.kind.to_string(),
        };
        // Same-named ones: what each is in (`impl Clipboard for X`), found
        // in its file, tells them apart.
        let twins = calls.ours.iter().filter(|c| c.name == call.name).count() > 1;
        let within = twins
            .then(|| container_of(&call.path, call.line))
            .flatten()
            .map(|c| format!(" in {c}"))
            .unwrap_or_default();
        let (name, line) = (&call.name, call.line);
        ours.push((
            path.clone(),
            line,
            format!("{name} ({what}){within}  {path}:{line}{place}"),
        ));
    }
    let outside = &calls.outside;
    if ours.is_empty() && outside.is_empty() {
        return if callers {
            "No callers found".into()
        } else {
            "It calls nothing the server knows of".into()
        };
    }
    ours.sort();
    let count = ours.len();
    let head = match (callers, count) {
        (true, 1) => "1 function calls it:".to_string(),
        (true, n) => format!("{n} functions call it:"),
        (false, 0) => "It calls nothing in the project".to_string(),
        (false, 1) => "It calls 1 function in the project:".to_string(),
        (false, n) => format!("It calls {n} functions in the project:"),
    };
    let mut lines = vec![head];
    let files_left: BTreeSet<&str> = ours
        .iter()
        .skip(CALLS_LISTED)
        .map(|(path, ..)| path.as_str())
        .collect();
    lines.extend(
        ours.iter()
            .take(CALLS_LISTED)
            .map(|(_, _, text)| text.clone()),
    );
    if count > CALLS_LISTED {
        let files = files_left.len();
        let per_file: Vec<String> = files_left
            .iter()
            .take(15)
            .map(|file| {
                let n = ours
                    .iter()
                    .skip(CALLS_LISTED)
                    .filter(|(path, ..)| path == file)
                    .count();
                format!("{file} ({n})")
            })
            .collect();
        let more_files = if files > 15 {
            format!(", … {} more files", files - 15)
        } else {
            String::new()
        };
        lines.push(format!(
            "… and {} more in {files} file{}: {}{more_files}",
            count - CALLS_LISTED,
            if files == 1 { "" } else { "s" },
            per_file.join(", ")
        ));
    }
    if !outside.is_empty() {
        let shown: Vec<&str> = outside.iter().take(20).map(String::as_str).collect();
        let more = outside.len().saturating_sub(20);
        lines.push(format!(
            "And {} outside the project (the standard library, dependencies): {}{}",
            outside.len(),
            shown.join(", "),
            if more > 0 {
                format!(", … {more} more")
            } else {
                String::new()
            }
        ));
    }
    lines.join("\n")
}

/// What the function on `line` (from 1) of `file` is in: the nearest line
/// above it, less indented, that opens an `impl`, `trait`, `class` or
/// `interface` (`impl Clipboard for TerminalClipboard`).
fn container_of(file: &Path, line: usize) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let own = lines.get(line.checked_sub(1)?)?;
    let indent = |l: &str| l.len() - l.trim_start().len();
    let depth = indent(own);
    let opens = |l: &str| {
        let word = l
            .split_whitespace()
            .find(|w| {
                !matches!(
                    *w,
                    "pub" | "pub(crate)" | "export" | "default" | "abstract" | "unsafe"
                )
            })
            .unwrap_or("");
        let word = word.split('<').next().unwrap_or("");
        matches!(word, "impl" | "trait" | "class" | "interface")
    };
    // The nearest line that's less indented is what it's in: a container,
    // or something else (a function, an object literal), then none.
    let outer = lines[..line - 1]
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty() && indent(l) < depth)?;
    opens(outer.trim()).then(|| {
        let l = outer
            .trim()
            .trim_end_matches('{')
            .trim_end_matches(':')
            .trim_end();
        short(l.strip_prefix("pub ").unwrap_or(l))
    })
}

/// `text` on one line (a wrapped signature's line breaks and indents as
/// single spaces), cut to `DETAIL_CHARS` with `…`.
fn short(text: &str) -> String {
    let one = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(",)", ")");
    if one.chars().count() <= DETAIL_CHARS {
        return one;
    }
    let cut: String = one.chars().take(DETAIL_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::lsp::Call;

    #[test]
    fn the_agent_is_told_why_and_what_to_do() {
        let starting = NoAnswer::Starting("rust-analyzer".into());
        assert!(no_answer_text(LspFeature::Hover, &starting).contains("try again"));
        assert!(no_answer_text(LspFeature::MoveFile, &starting).contains("imports"));
        assert_eq!(
            no_answer_text(LspFeature::Callers, &NoAnswer::Unsupported("x".into())),
            "x doesn't offer callers"
        );
        assert_eq!(
            no_answer_text(LspFeature::Hover, &NoAnswer::Stopped("x".into())),
            "x stopped before answering"
        );
    }

    fn call(name: &str, path: &Path, line: usize, detail: &str) -> Call {
        Call {
            name: name.into(),
            kind: "function",
            detail: Some(detail.into()),
            path: path.to_path_buf(),
            line,
            at: vec![line],
        }
    }

    #[test]
    fn many_callers_are_cut_with_how_many_more() {
        let ours = (0..60)
            .map(|n| {
                let file = if n < 55 { "/p/a.rs" } else { "/p/b.rs" };
                call(&format!("f{n:02}"), Path::new(file), n + 1, "fn f()")
            })
            .collect();
        let calls = Calls {
            ours,
            outside: Vec::new(),
        };
        let text = calls_text(&calls, LspFeature::Callers);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "60 functions call it:");
        assert_eq!(lines.len(), 1 + CALLS_LISTED + 1);
        assert_eq!(
            lines.last().unwrap(),
            &"… and 10 more in 2 files: /p/a.rs (5), /p/b.rs (5)"
        );
    }

    #[test]
    fn same_named_functions_say_what_they_are_in() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.rs");
        std::fs::write(
            &file,
            "pub trait Clipboard {\n    fn get(&mut self) -> String;\n}\n\n\
             impl Clipboard for TerminalClipboard {\n    fn get(&mut self) -> String {\n        \
             String::new()\n    }\n}\n\nfn alone() {}\n",
        )
        .unwrap();
        assert_eq!(container_of(&file, 2).as_deref(), Some("trait Clipboard"));
        assert_eq!(
            container_of(&file, 6).as_deref(),
            Some("impl Clipboard for TerminalClipboard")
        );
        assert_eq!(container_of(&file, 11), None, "at the top");
        // In something that isn't a container (a function, an object): none.
        let ts = dir.path().join("a.ts");
        std::fs::write(
            &ts,
            "export class Cat {\n  speak() {}\n}\nexport const obj = {\n  speak() {},\n};\n",
        )
        .unwrap();
        assert_eq!(container_of(&ts, 2).as_deref(), Some("export class Cat"));
        assert_eq!(container_of(&ts, 5), None, "obj isn't a class");
        // A wrapped signature, on one line.
        assert_eq!(
            short("fn go(\n    a: u8,\n    b: u8,\n) -> u32\nwhere\n    Self: Sized,"),
            "fn go(a: u8, b: u8) -> u32 where Self: Sized,"
        );

        let long = "fn alone(with: a, very: long, list: of, parameters: that, goes: on) -> X";
        let calls = Calls {
            ours: vec![
                call("get", &file, 2, "fn get(&mut self) -> String"),
                call("get", &file, 6, "fn get(&mut self) -> String"),
                call("alone", &file, 11, long),
            ],
            outside: Vec::new(),
        };
        let text = calls_text(&calls, LspFeature::Callees);
        assert!(
            text.contains("get (function, fn get(&mut self) -> String) in trait Clipboard  "),
            "{text}"
        );
        assert!(
            text.contains(") in impl Clipboard for TerminalClipboard  "),
            "{text}"
        );
        // Only twins get it; a long detail is cut, not dropped.
        assert!(
            text.contains(
                "alone (function, fn alone(with: a, very: long, list: of, parameters: that, g…)  "
            ),
            "{text}"
        );
    }

    #[test]
    fn places_for_the_agent() {
        use crate::core::Position;
        let place = |path: &str, row, col| Location {
            path: path.into(),
            position: Position::new(row, col),
            line: "  let x = bad;  ".into(),
        };
        let found = vec![
            place("/p/b.rs", 0, 0),
            place("/p/a.rs", 4, 8),
            place("/p/a.rs", 1, 2),
        ];
        assert_eq!(
            locations_text(found.clone(), LspFeature::References, None),
            "/p/a.rs:2:3  let x = bad;\n/p/a.rs:5:9  let x = bad;\n/p/b.rs:1:1  let x = bad;",
            "references by file and place"
        );
        assert!(
            locations_text(found, LspFeature::Definition, None).starts_with("/p/b.rs:1:1"),
            "others as the server gave them"
        );
        let busy = Busy {
            server: "rust-analyzer".into(),
            work: "Indexing".into(),
        };
        assert_eq!(
            locations_text(Vec::new(), LspFeature::References, Some(&busy)),
            "Nothing found, but rust-analyzer is busy (Indexing): it may know more in a moment"
        );
        assert_eq!(
            locations_text(Vec::new(), LspFeature::Definition, Some(&busy)),
            "Nothing found",
            "only the project-wide ones wait on indexing"
        );
        assert_eq!(hover_text("<b></b>  "), "No hover information there");
    }

    #[test]
    fn symbols_for_the_agent() {
        use crate::core::Position;
        let found = vec![(
            "parse".to_string(),
            "function".to_string(),
            Location {
                path: "/p/a.rs".into(),
                position: Position::new(2, 3),
                line: "  fn parse() {}".into(),
            },
        )];
        assert_eq!(
            symbols_text(&found, LspFeature::DocumentSymbols, None),
            "parse (function)  /p/a.rs:3:4  fn parse() {}"
        );
        let busy = Busy {
            server: "rust-analyzer".into(),
            work: "Indexing".into(),
        };
        assert!(
            symbols_text(&[], LspFeature::WorkspaceSymbols, Some(&busy))
                .ends_with("is busy (Indexing): it may know more in a moment")
        );
        assert_eq!(
            symbols_text(&[], LspFeature::DocumentSymbols, Some(&busy)),
            "No symbols found",
            "a file's symbols don't wait on the project's indexing"
        );
    }

    #[test]
    fn nothing_there_is_an_answer_not_a_failure() {
        let calls = nothing_there(LspFeature::Callers);
        assert!(
            calls.starts_with("Nothing there to find calls of"),
            "{calls}"
        );
    }
}
