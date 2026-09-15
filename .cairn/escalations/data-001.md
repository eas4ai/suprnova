DECISION

Question:   Agree DATA-001 to DATA-005 in docs/spec/component-library-data-display.md as the data-display commitment (separator, scroll area, aspect image, card, badge, avatar and group, list group, description list, stat card; the chart through charts-rs; the datatable last, one island per table with sort, filter and page as #[url] fields)? You ruled on 2026-09-14 at 23:35 that every remaining commitment is agreed; this records that ruling on the five requirements so the loop can name them.
Recommend:  Answer ok: agree the five as one set, with charts-rs 1.0.0 (Apache-2.0, default features off, no image codec) entering the Live crate and the interactive data grid staying out.
Because:    Every requirement rests on vocabulary the library already has (keyed morph through live_key, reflected URL intent, live:submit model proposals, trusted markup with a bounded reason), and the one new dependency is the chart renderer spec 25 already names.
If wrong:   A component ships against a falsifier you did not mean, or charts-rs enters the workspace against your intent; each is a spec edit superseded with the cause named, and the dependency is one line to remove.
Instead:    Agree DATA-001 to DATA-003 and DATA-005 now and hold the chart (DATA-004) for a later commitment, or name a different chart renderer.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: DATA-001
Status: open
Raised: 2026-09-15T05:08:42.669Z
Raised after: DATA-001=0
Answer: ok
Answered: 2026-09-15T13:05:15.766Z
Answered after: DATA-001=0
Answered order: 7
