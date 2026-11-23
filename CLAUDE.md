## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues for this repo (JonWatkins/ferra), via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Architecture

`docs/ARCHITECTURE.md` is the map: one entry per module (what it owns, its interface and seams, the ADRs behind it). Read it first to find where something lives; how a part works is in the `//!` docs of the file that implements it. Plans and known gaps are GitHub issues.
