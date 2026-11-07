//! Line-range commands: `:s` (substitute), `:d`, `:y`, and `:g` (run a
//! command on every matching line).
//!
//! Each works on the range given before it (`:%s/a/b/`, `:5,10d`), or the
//! cursor's line when there isn't one (`:g`: the whole file).

use super::command_line::run_command_line;
use crate::commands::ex::{
    Bar, Flags, Global, Substitute, parse_global, parse_sort, parse_substitute, sort_lines,
};
use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::search::{CaseMode, SearchPattern};
use crate::core::{Position, Range, motion};
use crate::editor::{Message, Operator};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "substitute",
        "Replace matches in [range]: s/pattern/replacement/[g c i I n e]",
        substitute,
    )
    .bar(Bar::AfterPattern);
    r.alias("s", "substitute");
    r.register(
        "repeat_substitute",
        "Repeat the last :s on [range] or this line, without its flags (&, :&)",
        |cx| repeat_substitute(cx, false),
    );
    r.alias("&", "repeat_substitute");
    r.register(
        "substitute_confirm",
        "Answer a :s///c question: y, n, a (all), q (quit) or l (this one, then quit)",
        |cx| {
            let answer = cx.char_arg.ok_or("missing answer")?;
            if cx.editor.answer_confirm(answer) {
                cx.editor.request_char("substitute_confirm");
            }
            Ok(())
        },
    )
    .takes_char();
    r.register(
        "repeat_substitute_with_flags",
        "Repeat the last :s on [range] or this line, with its flags (:&&)",
        |cx| repeat_substitute(cx, true),
    );
    r.alias("&&", "repeat_substitute_with_flags");
    r.register(
        "repeat_substitute_everywhere",
        "Repeat the last :s on every line, with its flags and the last search (g&)",
        |cx| {
            cx.range = Some((0, cx.editor.document.buffer().line_count() - 1));
            let mut last = last_substitute(cx)?;
            last.pattern.clear();
            run_substitute(cx, last)
        },
    );

    r.register(
        "delete_range",
        "Delete the lines in [range] (:5,10d)",
        |cx| {
            let range = lines(cx);
            cx.editor.apply_operator(Operator::Delete, range, None)
        },
    );
    r.alias("d", "delete_range");
    r.alias("delete", "delete_range");

    r.register("yank_range", "Yank the lines in [range] (:%y)", |cx| {
        let range = lines(cx);
        let cursor = cx.editor.document.cursor();
        cx.editor.apply_operator(Operator::Yank, range, None)?;
        // `:y` doesn't move the cursor.
        cx.editor.document.set_cursor(cursor);
        Ok(())
    });
    r.alias("y", "yank_range");
    r.alias("yank", "yank_range");

    r.register(
        "sort",
        "Sort the lines in [range] (all by default): sort [!] [i] [n] [u] [r] [/pattern/]",
        sort,
    )
    .bar(Bar::Argument);
    r.alias("sor", "sort");

    r.register(
        "normal",
        "Run normal-mode keys on each line in [range], or at the cursor: normal A;",
        |cx| {
            if cx.arg_text.is_empty() {
                return Ok(());
            }
            match cx.range {
                Some((first, last)) => {
                    for row in first..=last {
                        cx.editor.queue_normal(cx.arg_text, Position::new(row, 0));
                    }
                }
                None => {
                    let cursor = cx.editor.document.cursor();
                    cx.editor.queue_normal(cx.arg_text, cursor);
                }
            }
            Ok(())
        },
    )
    .bar(Bar::Argument);
    r.alias("norm", "normal");

    r.register(
        "global",
        "Run a : command on each line in [range] matching a pattern: g/pattern/command (g! or v: not matching)",
        |cx| {
            let invert = cx.bang;
            global(cx, invert)
        },
    )
    .bar(Bar::Argument);
    r.alias("g", "global");
    r.register(
        "vglobal",
        "Run a : command on each line in [range] not matching a pattern: v/pattern/command",
        |cx| global(cx, true),
    )
    .bar(Bar::Argument);
    r.alias("v", "vglobal");
}

/// `:g/pattern/command`: finds the matching lines first, then runs
/// `command` with the cursor on each in turn, skipping lines an earlier run
/// deleted. With `invert`, the lines that don't match. With no command, it
/// reports how many lines match and goes to the last. One undo step.
fn global(cx: &mut Context, invert: bool) -> CommandResult {
    if cx.editor.document.is_tracking_rows() {
        return Err("Cannot use :g inside :g".into());
    }
    let Global { pattern, command } = parse_global(cx.arg_text)?;
    let case = cx.editor.options.search.case.into();
    let pattern = if pattern.is_empty() {
        let last = cx
            .editor
            .last_search_pattern()
            .ok_or("No previous search pattern")?;
        SearchPattern::new(last.source(), case)?
    } else {
        SearchPattern::new(&pattern, case)?
    };

    let doc = &cx.editor.document;
    let (first, last) = cx.range.unwrap_or((0, doc.buffer().line_count() - 1));
    let rows: Vec<usize> = (first..=last)
        .filter(|&row| {
            let line = doc.buffer().line(row);
            let matched = pattern.is_match(&line);
            matched != invert
        })
        .collect();
    let source = pattern.source().to_string();
    cx.editor.set_last_search(pattern);
    let Some(&last_row) = rows.last() else {
        return Err(format!("Pattern not found: {source}"));
    };
    if command.is_empty() {
        let doc = &mut cx.editor.document;
        let col = motion::first_non_blank(doc.buffer(), last_row);
        doc.set_cursor(Position::new(last_row, col));
        let lines = if rows.len() == 1 { "line" } else { "lines" };
        cx.editor
            .set_message(Message::info(format!("{} matching {lines}", rows.len())));
        return Ok(());
    }

    let lines_before = cx.editor.document.buffer().line_count();
    let count = rows.len();
    cx.editor.document.track_rows(rows);
    let mut result = Ok(());
    for index in 0..count {
        let Some(row) = cx.editor.document.tracked_row(index) else {
            continue;
        };
        let doc = &mut cx.editor.document;
        doc.set_cursor(Position::new(row, 0));
        if let Err(error) = run_command_line(cx, &command) {
            result = Err(error);
            break;
        }
    }
    cx.editor.document.stop_tracking_rows();
    result?;

    let lines_after = cx.editor.document.buffer().line_count();
    if lines_after < lines_before {
        let fewer = lines_before - lines_after;
        let lines = if fewer == 1 { "line" } else { "lines" };
        cx.editor
            .set_message(Message::info(format!("{fewer} fewer {lines}")));
    }
    Ok(())
}

/// The command's range as whole lines, or the cursor's line.
fn lines(cx: &Context) -> Range {
    let row = cx.editor.document.cursor().row;
    let (first, last) = cx.range.unwrap_or((row, row));
    Range::Lines { first, last }
}

/// `:s/pattern/replacement/flags`, or with no arguments, `:&`.
fn substitute(cx: &mut Context) -> CommandResult {
    if cx.arg_text.trim().is_empty() {
        return repeat_substitute(cx, false);
    }
    let previous = cx.editor.last_replacement.clone();
    let parsed = parse_substitute(cx.arg_text, previous.as_deref())?;
    cx.editor.last_substitute = Some(cx.arg_text.to_string());
    cx.editor.last_replacement = Some(parsed.replacement.clone());
    run_substitute(cx, parsed)
}

/// The last `:s`, parsed again, with the replacement it actually used (so
/// a `~` in it isn't expanded a second time).
fn last_substitute(cx: &Context) -> Result<Substitute, String> {
    let text = cx
        .editor
        .last_substitute
        .as_deref()
        .ok_or("No previous substitute regular expression")?;
    let mut parsed = parse_substitute(text, None)?;
    if let Some(replacement) = &cx.editor.last_replacement {
        parsed.replacement = replacement.clone();
    }
    Ok(parsed)
}

/// `&` / `:&` (no flags) and `:&&` (same flags): the last `:s` again.
fn repeat_substitute(cx: &mut Context, keep_flags: bool) -> CommandResult {
    let mut last = last_substitute(cx)?;
    if !keep_flags {
        last.flags = Flags::default();
    }
    run_substitute(cx, last)
}

fn run_substitute(cx: &mut Context, parsed: Substitute) -> CommandResult {
    let Substitute {
        pattern,
        replacement,
        flags,
    } = parsed;

    let case = match flags.ignore_case {
        Some(true) => CaseMode::Insensitive,
        Some(false) => CaseMode::Sensitive,
        None => cx.editor.options.search.case.into(),
    };
    let pattern = if pattern.is_empty() {
        let last = cx
            .editor
            .last_search_pattern()
            .ok_or("No previous search pattern")?;
        SearchPattern::new(last.source(), case)?
    } else {
        SearchPattern::new(&pattern, case)?
    };

    let Range::Lines { first, last } = lines(cx) else {
        unreachable!("lines() always gives a line range")
    };
    if flags.confirm && !flags.count_only {
        cx.editor
            .begin_confirm(pattern, replacement, flags.global, (first, last))?;
        cx.editor.request_char("substitute_confirm");
        return Ok(());
    }
    let doc = &mut cx.editor.document;
    let mut substitutions = 0;
    let mut changed_lines = 0;
    let mut last_changed = None;

    // Bottom-up, so a replacement that adds line breaks doesn't move the
    // rows still to be processed.
    for row in (first..=last).rev() {
        let line = doc.buffer().line(row).into_owned();
        let (replaced, count) = pattern.replace(&line, &replacement, flags.global);
        if count == 0 {
            continue;
        }
        substitutions += count;
        changed_lines += 1;
        last_changed.get_or_insert(row);
        if !flags.count_only && replaced != line {
            doc.delete(Position::new(row, 0), doc.buffer().line_end(row));
            doc.insert(Position::new(row, 0), &replaced);
        }
    }

    let source = pattern.source().to_string();
    cx.editor.set_last_search(pattern);
    let Some(row) = last_changed else {
        if flags.no_error {
            return Ok(());
        }
        return Err(format!("Pattern not found: {source}"));
    };

    let doc = &mut cx.editor.document;
    let row = row.min(doc.buffer().line_count() - 1);
    let col = motion::first_non_blank(doc.buffer(), row);
    doc.set_cursor(Position::new(row, col));

    let verb = if flags.count_only {
        "match"
    } else {
        "substitution"
    };
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let verb_plural = if flags.count_only && substitutions != 1 {
        format!("{substitutions} matches")
    } else {
        plural(substitutions, verb)
    };
    cx.editor.set_message(Message::info(format!(
        "{verb_plural} on {}",
        plural(changed_lines, "line")
    )));
    Ok(())
}

/// `:sort`: sorts the lines in the range, or the whole file (see
/// [`parse_sort`]), as one change.
fn sort(cx: &mut Context) -> CommandResult {
    let options = parse_sort(cx.arg_text, cx.bang)?;
    let pattern = match options.pattern.as_deref() {
        None => None,
        Some("") => Some(
            cx.editor
                .last_search_pattern()
                .cloned()
                .ok_or("No previous search pattern")?,
        ),
        Some(text) => Some(SearchPattern::new(
            text,
            CaseMode::from(cx.editor.options.search.case),
        )?),
    };
    let buffer = cx.editor.document.buffer();
    let (first, last) = cx.range.unwrap_or((0, buffer.line_count() - 1));
    let lines: Vec<String> = (first..=last)
        .map(|row| buffer.line(row).into_owned())
        .collect();
    let sorted = sort_lines(&lines, &options, pattern.as_ref());
    if sorted != lines {
        let doc = &mut cx.editor.document;
        let end = doc.buffer().line_end(last);
        doc.delete(Position::new(first, 0), end);
        doc.insert(Position::new(first, 0), &sorted.join("\n"));
    }
    let col = motion::first_non_blank(cx.editor.document.buffer(), first);
    cx.editor.document.set_cursor(Position::new(first, col));
    let removed = lines.len() - sorted.len();
    if removed > 0 {
        let s = if removed == 1 { "" } else { "s" };
        cx.editor
            .set_message(Message::info(format!("{removed} fewer line{s}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::Invocation;
    use crate::commands::builtin::test_util::{editor, run_with};
    use crate::core::Position;
    use crate::editor::{Editor, Mode};

    fn run(e: &mut Editor, name: &str, text: &str, range: Option<(usize, usize)>) {
        let invocation = Invocation {
            arg_text: text,
            range,
            ..Invocation::default()
        };
        run_with(e, name, invocation);
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    fn message(e: &Editor) -> String {
        e.message().map(|m| m.text.clone()).unwrap_or_default()
    }

    #[test]
    fn substitutes_on_the_current_line_by_default() {
        let mut e = editor("a a\na a", Position::new(1, 0), Mode::Normal);
        run(&mut e, "substitute", "/a/b/", None);
        assert_eq!(text(&e), "a a\nb a");
        assert_eq!(message(&e), "1 substitution on 1 line");
    }

    #[test]
    fn global_flag_and_ranges() {
        let mut e = editor("a a\na a\na a", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", "/a/b/g", Some((0, 1)));
        assert_eq!(text(&e), "b b\nb b\na a");
        assert_eq!(message(&e), "4 substitutions on 2 lines");
        assert_eq!(
            e.document.cursor(),
            Position::new(1, 0),
            "on the last changed line"
        );
    }

    #[test]
    fn captures_whole_match_and_line_breaks() {
        let mut e = editor("key=value", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", r"/(\w+)=(\w+)/\2=\1 [&]/", None);
        assert_eq!(text(&e), "value=key [key=value]");

        let mut e = editor("a,b\nc,d", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", r"/,/\n/g", Some((0, 1)));
        assert_eq!(text(&e), "a\nb\nc\nd", "line breaks split lines");
    }

    #[test]
    fn whole_substitution_undoes_in_one_step() {
        let mut e = editor("x\nx\nx", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", "/x/y/", Some((0, 2)));
        assert_eq!(text(&e), "y\ny\ny");
        e.document.undo();
        assert_eq!(text(&e), "x\nx\nx");
    }

    #[test]
    fn case_flags() {
        let mut e = editor("Foo foo", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", "/foo/x/gI", None);
        assert_eq!(text(&e), "Foo x");
        run(&mut e, "substitute", "/FOO/y/i", None);
        assert_eq!(text(&e), "y x");
    }

    #[test]
    fn count_only_and_no_error() {
        let mut e = editor("a a", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", "/a/b/gn", None);
        assert_eq!(text(&e), "a a", "nothing changed");
        assert_eq!(message(&e), "2 matches on 1 line");

        run(&mut e, "substitute", "/zzz/b/", None);
        assert_eq!(message(&e), "Pattern not found: zzz");
        e.clear_message();
        run(&mut e, "substitute", "/zzz/b/e", None);
        assert_eq!(message(&e), "");
    }

    #[test]
    fn empty_pattern_reuses_the_last_search_and_sets_it() {
        let mut e = editor("cat dog", Position::new(0, 0), Mode::Normal);
        run(&mut e, "substitute", "//x/", None);
        assert_eq!(message(&e), "No previous search pattern");

        run(&mut e, "substitute", "/dog/DOG/", None);
        // The empty pattern reuses `dog`, which (lowercase, so smart case
        // ignores case) matches the `DOG` it just wrote.
        run(&mut e, "substitute", "//wolf/", None);
        assert_eq!(text(&e), "cat wolf");
        assert!(e.search_highlight().is_some(), "and it's highlighted");
    }

    #[test]
    fn delete_and_yank_ranges() {
        let mut e = editor("1\n2\n3\n4", Position::new(3, 0), Mode::Normal);
        run(&mut e, "yank_range", "", Some((0, 1)));
        assert_eq!(e.registers.get(None).unwrap().text, "1\n2");
        assert_eq!(e.document.cursor(), Position::new(3, 0), ":y doesn't move");
        run(&mut e, "delete_range", "", Some((1, 2)));
        assert_eq!(text(&e), "1\n4");
        run(&mut e, "delete_range", "", None);
        assert_eq!(text(&e), "1", "the cursor's line by default");
    }
}
