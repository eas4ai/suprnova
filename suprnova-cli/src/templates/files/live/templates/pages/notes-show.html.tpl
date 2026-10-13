{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/button/button.html" as button %}
{% block content %}
<article class="kit-note" aria-labelledby="note-title">
<h1 class="kit-page-title" id="note-title">{{ note.title }}</h1>
<p class="kit-muted">Written <time datetime="{{ note.created_at }}">{{ note.created_on }}</time></p>
{% if let Some(body) = note.body %}
<p class="kit-note-body">{{ body }}</p>
{% else %}
<p class="kit-muted">This note has a title and nothing else.</p>
{% endif %}
<div class="kit-actions">
{% call button::link_button("Back to your notes", frame.href("/notes")) %}{% endcall %}
</div>
</article>
{% endblock %}
