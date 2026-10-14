# Inline-code-span parity across the whole manual -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-08
Promoted: 2026-09-08
Target domain: `conventions.md`

## What it is

Inline-code-span parity SHALL hold across the whole manual, so that the six
mirrors agree with the English chapter for every chapter rather than for a
listed subset. The rule's tokenizer SHALL treat a quoted backtick as quoted
text, so that only real drift is reported, and every real drift SHALL be fixed
by translation rather than by narrowing the rule. The ratchet list SHALL be
retired once every source is covered.

`scripts/check-manual-structure.py` already compares inline code spans between
each English chapter and its six mirrors, but only over the seven RenderCache
and documentation chapters named in `SPAN_CHECKED_SOURCES`. Run over the whole
manual, the same rule reports 1,591 problems over 198 files across 73 chapters.
Part of that count is a tokenizer artefact from chapters that quote backticks
as literal text, of which `seeding.md` alone contributes 958. The rest is real
drift: `manual/from-laravel.md:233` carries `TwoFactor` where the German mirror
carries no such span, and `manual/cli.md:139` writes "workflow +
workflow_steps" as prose while `manual/de/cli.md:149` sets both names in
backticks. Ruling R37 shipped the ratchet over the seven chapters and raised
the whole-manual audit as a capture candidate.

## Acceptance criteria

- The rule's tokenizer SHALL treat quoted backticks so that only real drift is
  reported.
- Every real drift SHALL be fixed by translation.
- The ratchet list in `check-manual-structure.py` SHALL be retired because
  every source is covered.
- The translation lock SHALL be restamped for every chapter that changed.
