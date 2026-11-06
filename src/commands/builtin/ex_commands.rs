//! Line-range commands: `:s` (substitute), `:d` and `:y`.
//!
//! Each works on the range given before it (`:%s/a/b/`, `:5,10d`), or the
//! cursor's line when there isn't one.

use crate::commands::ex::{Substitute, parse_substitute};
use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::search::{CaseMode, SearchPattern};
use crate::core::{Position, Range, motion};
use crate::editor::{Message, Operator};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "substitute",
        "Replace matches in [range]: s/pattern/replacement/[g i I n e]",
        substitute,
    );
    r.alias("s", "substitute");

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
}

/// The command's range as whole lines, or the cursor's line.
fn lines(cx: &Context) -> Range {
    let row = cx.editor.document.cursor().row;
    let (first, last) = cx.range.unwrap_or((row, row));
    Range::Lines { first, last }
}

fn substitute(cx: &mut Context) -> CommandResult {
    let Substitute {
        pattern,
        replacement,
        flags,
    } = parse_substitute(cx.arg_text)?;

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
