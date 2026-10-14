{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% import "suprnova-ui/password-input/password-input.html" as password %}
{% import "suprnova-ui/checkbox/checkbox.html" as checkbox %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
<form class="kit-form" live:submit.prevent="login">
<div class="kit-field"{% if errors.has("email") %} data-invalid{% endif %}>
{% call field::field("email", "Email", hint=errors.get("email"), required=true) %}{% call input::input("email", kind="email", required=true, autocomplete="email", value=email) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("password") %} data-invalid{% endif %}>
{% call field::field("password", "Password", hint=errors.get("password"), required=true) %}{% call password::password_input("password", autocomplete="current-password", required=true) %}{% endcall %}{% endcall %}
</div>
{% call checkbox::checkbox("remember", "Remember me", checked=remember) %}{% endcall %}
<p class="kit-form-status" live:error.live.polite="login"></p>
{% call actions::form_actions() %}
<a class="kit-link" href="{{ suprnova::url::root() }}/forgot-password">Forgot your password?</a>
{% call button::button("Sign in", kind="submit", action="login") %}{% endcall %}
{% endcall %}
</form>
