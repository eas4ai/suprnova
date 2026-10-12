{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/button/button.html" as button %}
{% block content %}
<section class="kit-error" aria-labelledby="error-status">
<h1 class="kit-error-status" id="error-status">{{ status }}</h1>
<p class="kit-lead">{{ message }}</p>
{% if let Some(id) = request_id %}
<p class="kit-muted">{{ reference }} <code>{{ id }}</code></p>
{% endif %}
<div class="kit-actions">
{% call button::link_button(go_home, frame.href("/")) %}{% endcall %}
</div>
</section>
{% endblock %}
