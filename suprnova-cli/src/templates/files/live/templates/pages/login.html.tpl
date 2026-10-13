{% extends "live/layouts/guest.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% block content %}
{% call card::card("sign-in", "Sign in", kind="section") %}
{{ form|trusted_html }}
{% endcall %}
<p class="kit-muted kit-center">No account yet? <a href="{{ frame.href("/register") }}">Register</a></p>
{% endblock %}
