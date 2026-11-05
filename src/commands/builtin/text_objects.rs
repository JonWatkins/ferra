//! Text-object commands (`iw`, `a"`, `i(`, ...), used after an operator.
//!
//! Outside operator-pending mode they do nothing. Visual mode will give them
//! a second use later.

use super::motions::multiply_counts;
use crate::commands::CommandRegistry;
use crate::core::{Buffer, Position, Range, text_object};
use crate::editor::Mode;

type SelectFn = fn(&Buffer, Position, usize) -> Option<Range>;

struct TextObject {
    name: &'static str,
    description: &'static str,
    select: SelectFn,
}

const fn object(name: &'static str, description: &'static str, select: SelectFn) -> TextObject {
    TextObject {
        name,
        description,
        select,
    }
}

macro_rules! pair_objects {
    ($inner:literal, $around:literal, $what:literal, $open:literal, $close:literal) => {
        [
            object($inner, concat!("Inside ", $what), |b, p, _| {
                text_object::bracketed(b, p, $open, $close, false)
            }),
            object(
                $around,
                concat!("Around ", $what, ", including them"),
                |b, p, _| text_object::bracketed(b, p, $open, $close, true),
            ),
        ]
    };
}

macro_rules! quote_objects {
    ($inner:literal, $around:literal, $what:literal, $quote:literal) => {
        [
            object($inner, concat!("Inside ", $what), |b, p, _| {
                text_object::quoted(b, p, $quote, false)
            }),
            object(
                $around,
                concat!("Around ", $what, ", including them"),
                |b, p, _| text_object::quoted(b, p, $quote, true),
            ),
        ]
    };
}

const WORD: [TextObject; 2] = [
    object("inner_word", "The word under the cursor", |b, p, _| {
        text_object::inner_word(b, p)
    }),
    object(
        "around_word",
        "The word under the cursor plus surrounding space",
        |b, p, _| text_object::a_word(b, p),
    ),
];
const LINE: TextObject = object(
    "current_line",
    "[count] lines from the cursor (dd, cc, yy)",
    |b, p, n| {
        Some(Range::Lines {
            first: p.row,
            last: (p.row + n - 1).min(b.line_count() - 1),
        })
    },
);
const PARENS: [TextObject; 2] =
    pair_objects!("inner_paren", "around_paren", "parentheses", '(', ')');
const BRACES: [TextObject; 2] = pair_objects!("inner_brace", "around_brace", "braces", '{', '}');
const BRACKETS: [TextObject; 2] =
    pair_objects!("inner_bracket", "around_bracket", "brackets", '[', ']');
const ANGLES: [TextObject; 2] =
    pair_objects!("inner_angle", "around_angle", "angle brackets", '<', '>');
const DOUBLE_QUOTES: [TextObject; 2] = quote_objects!(
    "inner_double_quote",
    "around_double_quote",
    "double quotes",
    '"'
);
const SINGLE_QUOTES: [TextObject; 2] = quote_objects!(
    "inner_single_quote",
    "around_single_quote",
    "single quotes",
    '\''
);
const BACKTICKS: [TextObject; 2] =
    quote_objects!("inner_backtick", "around_backtick", "backticks", '`');

pub fn register(r: &mut CommandRegistry) {
    let groups: [&'static [TextObject]; 9] = [
        &WORD,
        std::slice::from_ref(&LINE),
        &PARENS,
        &BRACES,
        &BRACKETS,
        &ANGLES,
        &DOUBLE_QUOTES,
        &SINGLE_QUOTES,
        &BACKTICKS,
    ];
    for object in groups.into_iter().flatten() {
        r.register(object.name, object.description, move |cx| {
            if cx.editor.mode() == Mode::Visual {
                let doc = &cx.editor.document;
                if let Some(range) = (object.select)(doc.buffer(), doc.cursor(), cx.count()) {
                    cx.editor.select_range(range);
                }
                return Ok(());
            }
            let Some(pending) = cx.editor.take_pending_operator() else {
                return Ok(());
            };
            let count = multiply_counts(pending.count, cx.count).unwrap_or(1);
            let doc = &cx.editor.document;
            match (object.select)(doc.buffer(), doc.cursor(), count) {
                Some(range) => {
                    let register = pending.register.or(cx.register);
                    cx.editor.apply_operator(pending.operator, range, register)
                }
                // No such object here: the operator is simply cancelled.
                None => Ok(()),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::{Mode, Operator};

    fn operate(text: &str, col: usize, operator: Operator, object: &str) -> (String, Mode) {
        let mut e = editor(text, Position::new(0, col), Mode::Normal);
        e.begin_operator(operator, None, None);
        run(&mut e, object, None);
        (e.document.buffer().text(), e.mode())
    }

    #[test]
    fn delete_inner_word() {
        assert_eq!(
            operate("foo bar baz", 5, Operator::Delete, "inner_word").0,
            "foo  baz"
        );
    }

    #[test]
    fn change_inside_quotes_enters_insert() {
        let (text, mode) = operate(r#"x = "abc";"#, 6, Operator::Change, "inner_double_quote");
        assert_eq!(text, r#"x = "";"#);
        assert_eq!(mode, Mode::Insert);
    }

    #[test]
    fn delete_around_parens() {
        assert_eq!(
            operate("f(a, b) + 1", 3, Operator::Delete, "around_paren").0,
            "f + 1"
        );
    }

    #[test]
    fn missing_object_cancels_operator() {
        let (text, mode) = operate("no parens", 3, Operator::Delete, "inner_paren");
        assert_eq!(text, "no parens");
        assert_eq!(mode, Mode::Normal);
    }

    #[test]
    fn current_line_respects_count() {
        let mut e = editor("a\nb\nc", Position::new(0, 0), Mode::Normal);
        e.begin_operator(Operator::Delete, Some(2), None);
        run(&mut e, "current_line", None);
        assert_eq!(e.document.buffer().text(), "c");
    }

    #[test]
    fn selects_object_in_visual_mode() {
        let mut e = editor("call(a, b)", Position::new(0, 6), Mode::Normal);
        e.toggle_visual(crate::editor::SelectionKind::Char);
        run(&mut e, "inner_paren", None);
        assert_eq!(
            e.selection_range(),
            Some(crate::core::Range::Chars {
                start: Position::new(0, 5),
                end: Position::new(0, 9)
            })
        );
        assert_eq!(e.mode(), Mode::Visual);
    }

    #[test]
    fn without_operator_does_nothing() {
        let mut e = editor("abc", Position::new(0, 1), Mode::Normal);
        run(&mut e, "inner_word", None);
        assert_eq!(e.document.buffer().text(), "abc");
        assert_eq!(e.mode(), Mode::Normal);
    }
}
