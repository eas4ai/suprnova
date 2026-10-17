{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% import "suprnova-ui/password-input/password-input.html" as password %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
<form class="kit-form" live:submit.prevent="register">
<div class="kit-field"{% if errors.has("name") %} data-invalid{% endif %}>
{% call field::field("name", "Name", hint=errors.get("name"), required=true) %}{% call input::input("name", required=true, autocomplete="name", value=component.name) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("email") %} data-invalid{% endif %}>
{% call field::field("email", "Email", hint=errors.get("email"), required=true) %}{% call input::input("email", kind="email", required=true, autocomplete="email", value=email) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("password") %} data-invalid{% endif %}>
{% call field::field("password", "Password", hint=errors.get("password"), required=true) %}{% call password::password_input("password", autocomplete="new-password", required=true) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("password_confirmation") %} data-invalid{% endif %}>
{% call field::field("password_confirmation", "Confirm password", hint=errors.get("password_confirmation"), required=true) %}{% call password::password_input("password_confirmation", autocomplete="new-password", required=true) %}{% endcall %}{% endcall %}
</div>
<p class="kit-form-status" live:error.live.polite="register"></p>
{% call actions::form_actions() %}
{% call button::button("Register", kind="submit", action="register") %}{% endcall %}
{% endcall %}
</form>
