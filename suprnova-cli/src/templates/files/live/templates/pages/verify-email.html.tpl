{% extends "live/layouts/guest.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% block content %}
{% call card::card("verify-email-card", "Verify your email", kind="section") %}
{{ notice|trusted_html }}
{% endcall %}
{% endblock %}
