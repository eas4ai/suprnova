/*
 * The Live kit's page layout. The library components style themselves
 * (their stylesheets sit beside their views under templates/suprnova-ui/);
 * this file only places them on the page, with the same --sn- tokens the
 * base stylesheet defines, so a token you change restyles the kit too.
 * Served from public/ by the router's static-file fallback.
 */

body {
  margin: 0;
  background: var(--sn-color-bg);
  color: var(--sn-color-text);
  font-family: var(--sn-font-sans);
  line-height: var(--sn-font-leading-base);
}

a {
  color: var(--sn-color-link);
}

a:hover {
  color: var(--sn-color-link-hover);
}

.kit-muted {
  color: var(--sn-color-text-muted);
}

.kit-lead {
  font-size: var(--sn-font-size-lg);
}

.kit-center {
  text-align: center;
}

.kit-page-title {
  margin: 0 0 var(--sn-space-4);
  font-size: var(--sn-font-size-2xl);
  font-weight: var(--sn-font-weight-semibold);
  line-height: var(--sn-font-leading-tight);
}

.kit-subtitle {
  margin: var(--sn-space-4) 0 var(--sn-space-2);
  font-size: var(--sn-font-size-base);
  font-weight: var(--sn-font-weight-semibold);
}

.kit-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sn-space-2);
  margin-block-start: var(--sn-space-4);
}

.kit-stack > * + * {
  margin-block-start: var(--sn-space-3);
}

/* The application layout. */

.kit-topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sn-space-4);
  padding-inline-end: var(--sn-space-4);
  border-block-end: var(--sn-state-border-width) solid var(--sn-color-border);
  background: var(--sn-color-surface);
}

.kit-topbar .sn-header-bar {
  flex: 1;
  border: 0;
}

.kit-account {
  display: flex;
  align-items: center;
  gap: var(--sn-space-2);
}

.kit-app-main {
  max-inline-size: 72rem;
  margin: 0 auto;
  padding: var(--sn-space-6) var(--sn-space-4);
}

.kit-columns {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
  gap: var(--sn-space-6);
  align-items: start;
}

/* The guest layout. */

.kit-guest-main {
  max-inline-size: 28rem;
  margin: 0 auto;
  padding: var(--sn-space-8) var(--sn-space-4);
}

.kit-brand {
  margin: 0 0 var(--sn-space-6);
  font-size: var(--sn-font-size-xl);
  text-align: center;
}

.kit-brand a {
  color: inherit;
  text-decoration: none;
}

.kit-guest-links {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: var(--sn-space-4);
  margin-block-start: var(--sn-space-6);
}

.kit-inline-form {
  display: inline;
}

.kit-link-button {
  padding: 0;
  border: 0;
  background: none;
  color: var(--sn-color-link);
  font: inherit;
  text-decoration: underline;
  cursor: pointer;
}

/* Forms. */

.kit-form > * + * {
  margin-block-start: var(--sn-space-4);
}

.kit-field[data-invalid] .sn-input,
.kit-field[data-invalid] .sn-textarea {
  border-color: var(--sn-color-danger);
}

.kit-field[data-invalid] .sn-hint {
  color: var(--sn-color-danger);
}

.kit-form-status:empty {
  display: none;
}

.kit-form-status,
.kit-form-error {
  color: var(--sn-color-danger);
}

.kit-form .sn-form-actions {
  align-items: center;
  justify-content: space-between;
}

.kit-search {
  display: flex;
  align-items: flex-end;
  gap: var(--sn-space-2);
  margin-block-end: var(--sn-space-4);
}

.kit-search .sn-field {
  flex: 1;
}

/* The dashboard and the notes pages. */

.kit-stats {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--sn-space-4);
}

.kit-note-row {
  display: flex;
  justify-content: space-between;
  gap: var(--sn-space-4);
  padding-block: var(--sn-space-2);
  border-block-end: var(--sn-state-border-width) solid var(--sn-color-border);
}

.kit-note-body {
  white-space: pre-wrap;
}

.kit-error {
  text-align: center;
}

.kit-error .kit-actions {
  justify-content: center;
}

.kit-error-status {
  margin: 0;
  font-size: var(--sn-font-size-4xl);
  font-weight: var(--sn-font-weight-bold);
}
