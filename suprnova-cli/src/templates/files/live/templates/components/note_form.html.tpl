{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% import "suprnova-ui/textarea/textarea.html" as textarea %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
{% call card::card("note-form", "Write a note", kind="section") %}
<form class="kit-form" live:submit.prevent="save">
<div class="kit-field"{% if errors.has("title") %} data-invalid{% endif %}>
{% call field::field("title", "Title", hint=errors.get("title"), required=true) %}{% call input::input("title", required=true, value=title) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("body") %} data-invalid{% endif %}>
{% call field::field("body", "Text", hint=errors.get("body")) %}{% call textarea::textarea("body", rows=5, value=body) %}{% endcall %}{% endcall %}
</div>
<p class="kit-form-status" live:error.live.polite="save"></p>
{% call actions::form_actions() %}
{% call button::button("Save note", kind="submit", action="save") %}{% endcall %}
{% endcall %}
</form>
{% endcall %}
