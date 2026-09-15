DECISION

Question:   Agree FORM-005 to FORM-008, FDB-005 and NAV-005 as the live-native commitment (upload widget, input OTP, date picker, combobox, live feed and notification bell, account menu as a stitch slot)? You ruled on 2026-09-14 at 23:35 that every remaining commitment is agreed; this records that ruling on the six requirements so the loop can name them, and records one design choice: the custom-element base.
Recommend:  Answer ok: agree the six as one set, with the three enhancements as plain light-DOM HTMLElement subclasses using ElementInternals, defined by their own vendored file, and no JavaScript helper library added.
Because:    The glossary's Draft entry names an Elena-class helper, but the tree holds none, Live spec 20 keeps component JavaScript on Live primitives, and the password input already ships a working plain element; three elements do not justify a dependency.
If wrong:   The enhancements carry more boilerplate than a helper would give, and adopting a helper later is a rewrite of three small files; a spec edit superseded with the cause named records it.
Instead:    Vendor ElenaJS as a shared library base under __live/assets and build the three enhancements on it.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: FORM-005
Status: open
Raised: 2026-09-15T13:30:00.196Z
Raised after: FORM-005=0
Answer: ok
Answered: 2026-09-15T13:30:07.194Z
Answered after: FORM-005=0
Answered order: 8
