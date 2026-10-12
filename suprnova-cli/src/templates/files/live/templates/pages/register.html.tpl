{% extends "live/layouts/guest.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% block content %}
{% call card::card("create-account", "Create an account", kind="section") %}
{{ form|trusted_html }}
{% endcall %}
<p class="kit-muted kit-center">Already registered? <a href="{{ frame.href("/login") }}">Sign in</a></p>
{% endblock %}
