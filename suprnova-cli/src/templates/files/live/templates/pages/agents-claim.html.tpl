{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/button/button.html" as button %}
{#
  The claim page (AGENT-008, AGENT-009): who the agent will act as,
  through which provider, with which permissions, and the form that takes
  the code the agent gave the user. The provider's name is the
  application's trust list entry, never one the provider or the agent
  sent. Every value someone other than the application chose sits in
  <bdi>. The form is native and posts the session's CSRF token.
-#}
{% block content %}
<h1 class="kit-page-title">Let an agent act for you?</h1>
<p class="kit-lead">{% if let Some(provider) = claim.provider %}An agent signed in through <strong><bdi>{{ provider }}</bdi></strong> asks to act as you.{% else if claim.registration_type == "service_auth" %}An agent that named your email address asks to act as you.{% else %}An agent that registered on its own asks to act as you.{% endif %}</p>
<p>If you allow it, the agent will use {{ frame.app_name }} as you, with the permissions below, until you revoke it.</p>
<p>You are signed in as <strong><bdi>{{ claim.signed_in_as }}</bdi></strong>.{% if let Some(email) = claim.email %} The agent named <strong><bdi>{{ email }}</bdi></strong>.{% endif %}</p>
{% if claim.first_time_account %}
<p class="kit-form-error" role="alert"><strong>Warning:</strong> no agent has acted for your account before. Allow it only if you asked an agent to connect.</p>
{% endif %}
{% if claim.first_time_provider %}
<p class="kit-form-error" role="alert"><strong>Warning:</strong> no agent from this provider has acted for you before.</p>
{% endif %}
{% call card::card("claim-scopes", "It will be able to", kind="section") %}
{% if claim.scopes.is_empty() %}
<p>It asks for no permissions: it can use only what needs none.</p>
{% else %}
<ul class="kit-stack">
{% for scope in claim.scopes %}
<li>{{ scope.description }} (<code>{{ scope.name }}</code>)</li>
{% endfor %}
</ul>
{% endif %}
{% endcall %}
{% call card::card("claim-code", "Enter the code", kind="section") %}
<p>Enter the code the agent gave you. It works for about {{ minutes }} more minutes, {{ claim.attempts_remaining }} wrong entries allowed.</p>
<form class="kit-form" method="post" action="{{ frame.href("/agent/claim") }}">
<input type="hidden" name="_token" value="{{ frame.csrf }}">
<input type="hidden" name="claim_attempt_token" value="{{ claim_attempt_token }}">
<div class="kit-field"{% if !code_error.is_empty() %} data-invalid{% endif %}>
{% call field::field("user_code", "Code", hint=code_error, required=true) %}<input class="sn-input" id="user_code" name="user_code" type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="9" aria-describedby="user_code-hint user_code-error" required>{% endcall %}
</div>
{% call button::button("Allow", kind="submit") %}{% endcall %}
</form>
{% endcall %}
<p class="kit-muted">If you did not ask an agent to connect, close this page.</p>
{% endblock %}
