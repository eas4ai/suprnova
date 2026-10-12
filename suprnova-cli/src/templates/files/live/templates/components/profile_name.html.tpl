{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
{% call card::card("profile", "Display name", kind="section") %}
<form class="kit-form" live:submit.prevent="save">
<div class="kit-field"{% if errors.has("name") %} data-invalid{% endif %}>
{% call field::field("name", "Name", hint=errors.get("name"), required=true) %}{% call input::input("name", required=true, autocomplete="name", value=name) %}{% endcall %}{% endcall %}
</div>
<p class="kit-form-status" live:error.live.polite="save"></p>
{% call actions::form_actions() %}
{% call button::button("Save", kind="submit", action="save") %}{% endcall %}
{% endcall %}
</form>
{% endcall %}
