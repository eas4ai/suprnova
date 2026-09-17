# The combobox hides server results whose text does not contain the typed query

Surfaced from: FORM-008
Captured: 2026-09-17T11:53:47.897Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/combobox/combobox.js filters options with textContent.toLowerCase().includes(query) even when the listbox's data-sn-query equals the input, so a server search that is accent-insensitive, matches synonyms or codes, or is fuzzy has its results hidden: "sao" answered with "São Tomé and Príncipe" shows no option. Today that case also reaches the freeze recorded for a query that matches no option.
