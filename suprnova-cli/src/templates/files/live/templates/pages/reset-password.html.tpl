{% extends "live/layouts/guest.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% block content %}
{% call card::card("reset-password", "Choose a new password", kind="section") %}
{{ form|trusted_html }}
{% endcall %}
{% endblock %}
