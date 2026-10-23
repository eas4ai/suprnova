{% extends "live/layouts/app.html" %}
{% block content %}
<h1 class="kit-page-title">Notes</h1>
<div class="kit-columns">
{{ form|trusted_html }}
{{ list|trusted_html }}
</div>
{% endblock %}
