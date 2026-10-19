{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/button/button.html" as button %}
{#
  The approval page (AGENT-005): the client, the host it cannot choose
  freely, every permission it asks for, and where the user returns. Every
  value the client chose sits in <bdi>, so the direction marks it carries
  cannot reorder the sentence around it, and the page loads nothing the
  client named: no logo and no home page. The two forms are native and
  post the session's CSRF token. The page loads no Live runtime: its only
  scripts are the layout's component scripts, which register no tool.
-#}
{% block content %}
<h1 class="kit-page-title">Allow <bdi>{{ approval.client_name }}</bdi> to act for you?</h1>
<p class="kit-lead"><strong><bdi>{{ approval.client_name }}</bdi></strong>, from <strong><bdi>{{ approval.client_host }}</bdi></strong>, wants to use {{ frame.app_name }} as you.</p>
{% call card::card("approval-scopes", "It asks to", kind="section") %}
{% if approval.scopes.is_empty() %}
<p>It asks for no permissions: it can use only what needs none.</p>
{% else %}
<ul class="kit-stack">
{% for scope in approval.scopes %}
<li>{{ scope.description }} (<code>{{ scope.name }}</code>)</li>
{% endfor %}
</ul>
{% endif %}
<p class="kit-muted">If you allow it, you return to <bdi><code>{{ approval.redirect_uri }}</code></bdi>.</p>
<div class="kit-actions">
<form class="kit-inline-form" method="post" action="{{ frame.href("/oauth/approve") }}">
<input type="hidden" name="_token" value="{{ frame.csrf }}">
<input type="hidden" name="authorization" value="{{ approval.id }}">
<input type="hidden" name="decision" value="approve">
{% call button::button("Allow", kind="submit") %}{% endcall %}
</form>
<form class="kit-inline-form" method="post" action="{{ frame.href("/oauth/approve") }}">
<input type="hidden" name="_token" value="{{ frame.csrf }}">
<input type="hidden" name="authorization" value="{{ approval.id }}">
<input type="hidden" name="decision" value="deny">
{% call button::button("Deny", kind="submit", variant="secondary") %}{% endcall %}
</form>
</div>
{% endcall %}
{% endblock %}
