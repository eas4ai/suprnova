{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/password-input/password-input.html" as password %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/form-actions/form-actions.html" as actions %}
<form class="kit-form" live:submit.prevent="reset">
{% if errors.has("token") %}
<p class="kit-form-error" role="alert">{{ errors.get("token") }} <a href="{{ suprnova::url::root() }}/forgot-password">Request a new link</a></p>
{% endif %}
<div class="kit-field"{% if errors.has("password") %} data-invalid{% endif %}>
{% call field::field("password", "New password", hint=errors.get("password"), required=true) %}{% call password::password_input("password", autocomplete="new-password", required=true) %}{% endcall %}{% endcall %}
</div>
<div class="kit-field"{% if errors.has("password_confirmation") %} data-invalid{% endif %}>
{% call field::field("password_confirmation", "Confirm new password", hint=errors.get("password_confirmation"), required=true) %}{% call password::password_input("password_confirmation", autocomplete="new-password", required=true) %}{% endcall %}{% endcall %}
</div>
<p class="kit-form-status" live:error.live.polite="reset"></p>
{% call actions::form_actions() %}
{% call button::button("Reset password", kind="submit", action="reset") %}{% endcall %}
{% endcall %}
</form>
