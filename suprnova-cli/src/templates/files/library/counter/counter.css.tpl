/* {namespace}.counter - the count beside its button. The digits are
   fixed-width figures, so the button stays put as the count grows. */
.{namespace}-counter-body {
  display: inline-flex;
  align-items: center;
  gap: 0.75rem;
}

.{namespace}-counter-value {
  min-inline-size: 3ch;
  font-variant-numeric: tabular-nums;
  text-align: end;
}
