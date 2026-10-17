{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
<form class="kit-form" live:submit.prevent="send">
<div class="kit-field"{% if errors.has("email") %} data-invalid{% endif %}>
{% call field::field("email", "Email", hint=errors.get("email"), required=true) %}{% call input::input("email", kind="email", required=true, autocomplete="email", value=email) %}{% endcall %}{% endcall %}
</div>
<p class="kit-form-status" live:error.live.polite="send"></p>
{% call actions::form_actions() %}
{% call button::button("Email a reset link", kind="submit", action="send") %}{% endcall %}
{% endcall %}
</form>
