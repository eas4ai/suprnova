{% extends "live/layouts/app.html" %}
{% block content %}
<h1 class="kit-page-title">Dashboard</h1>
{% if let Some(user) = frame.user %}
<p class="kit-lead">Welcome, {{ user.name }}! <span class="kit-muted">{{ user.email }}</span></p>
{% endif %}
<div class="kit-columns">
{{ stats|trusted_html }}
{{ profile|trusted_html }}
</div>
{% endblock %}
