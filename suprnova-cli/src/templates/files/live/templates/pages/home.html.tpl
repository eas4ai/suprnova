{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/button/button.html" as button %}
{% block content %}
<section class="kit-hero">
<h1 class="kit-page-title">{{ welcome }}</h1>
<p class="kit-lead">Your Live app is ready. Every page is a server-rendered Live document, with no Node toolchain to install.</p>
<p class="kit-muted">Edit <code>templates/live/pages/home.html</code> to get started.</p>
<div class="kit-actions">
{% if frame.user.is_some() %}
{% call button::link_button("Go to your dashboard", frame.href("/dashboard"), variant="primary") %}{% endcall %}
{% else %}
{% call button::link_button("Register", frame.href("/register"), variant="primary") %}{% endcall %}
{% call button::link_button("Sign in", frame.href("/login")) %}{% endcall %}
{% endif %}
</div>
</section>
{% endblock %}
