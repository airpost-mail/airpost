# tern-mime test corpus

- `golden/`: handcrafted messages, one or more per rendering, file and header rule of plan
  section 6 "Bodies". The `harness-*` messages come from `apps/engine/test/harness/mailgen.py`.
- `mailcore/`: messages and expected outputs copied from mailcore2's `unittest/data`
  (`parser/` and `summary/`), under mailcore2's BSD licence (`mailcore/LICENSE`).
  `parser/` holds the jwz and simple mbox messages under 24 KB plus `messages/`; `summary/`
  holds the summary messages under 100 KB. Inputs from `parser/input/mbox/*` are prefixed with
  their directory name (`jwz-1`).
- `expected/<group>/`: the reviewed parse results (`.json`, `.body`, `.text`) that
  `tests/corpus.rs` compares against. Regenerate with
  `TERN_MIME_BLESS=1 cargo test -p tern-mime --test corpus` and review the diff.

`tests/mailcore_parser.rs` and `tests/mailcore_summary.rs` compare the port with mailcore's own
expected outputs; `tests/rules.rs` asserts the rules on the golden inputs directly.
