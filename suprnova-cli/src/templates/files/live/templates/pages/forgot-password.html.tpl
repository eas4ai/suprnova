{% extends "live/layouts/guest.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% block content %}
{% call card::card("forgot-password", "Forgot your password?", kind="section") %}
<p class="kit-muted">Enter the address you registered with. If it is verified, we mail you a link to choose a new password.</p>
{{ form|trusted_html }}
{% endcall %}
<p class="kit-muted kit-center"><a href="{{ frame.href("/login") }}">Back to sign in</a></p>
{% endblock %}
