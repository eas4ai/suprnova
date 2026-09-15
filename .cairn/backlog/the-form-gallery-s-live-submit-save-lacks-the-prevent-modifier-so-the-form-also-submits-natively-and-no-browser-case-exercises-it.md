# The form gallery's live:submit="save" lacks the prevent modifier, so the form also submits natively, and no browser case exercises it

Surfaced from: FORM-004
Captured: 2026-09-15T13:29:12.559Z

Found while building the datatable (data-display): live:submit without .prevent leaves the native GET submission alone, so every Live click also reloads the page from the form's own query. The datatable forms carry .prevent; the form gallery's save form does not, and no Playwright case drives it. Fix: add .prevent in app/templates/live/form-gallery.html and one browser case.
