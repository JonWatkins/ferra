# VS Code's keys are the default, over the modal engine

`keymap = "vscode"` is the default; Vim's keys (`keymap = "vim"`) are the other preset. Both run on the same modal engine: with VS Code's keys, insert mode is where the editor rests (asking for normal mode gives insert mode), a selection is visual mode with an exclusive end that typing replaces, there are no counts or registers, and commands other than typing end an undo step.

We chose one modal engine with a modeless preset over two editing engines because every command, motion and operator then works for both, and a feature written for one keymap is there for the other. It surprises readers of the code: features that show "when resting in normal mode" (hover, blame, references, watching files) have to ask `Editor::resting` instead of checking the mode.
