{#
  {namespace}.counter - the count and a button that runs the increment
  action. {namespace}-counter (counter.js) lets the plus key press the
  button while focus is inside the counter; without the script the button
  works the same.
-#}
<div class="{namespace}-counter">
<{namespace}-counter class="{namespace}-counter-body">
<output class="{namespace}-counter-value">{{ count }}</output>
<button class="{namespace}-counter-button" type="button" live:click="increment" aria-keyshortcuts="+">Add one</button>
</{namespace}-counter>
</div>
