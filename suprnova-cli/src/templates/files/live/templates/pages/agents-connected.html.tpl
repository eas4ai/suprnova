{% extends "live/layouts/app.html" %}
{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/list-group/list-group.html" as list %}
{% import "suprnova-ui/empty-state/empty-state.html" as empty %}
{#
  The connected-agents page (AGENT-010): every MCP client and agent that
  acts for the signed-in user, each with a native form that revokes it.
  A client's name and host are the client's own words, so each sits in
  <bdi>, the name on its revoke button too. An agent's provider is named by
  the application's trust list.
-#}
{% block content %}
<h1 class="kit-page-title">Connected agents</h1>
<p class="kit-lead">These apps and agents can use {{ frame.app_name }} as you. Revoking one ends its access at once.</p>
<div class="kit-columns">
{% call card::card("connected-apps", "Apps", kind="section") %}
{% if clients.is_empty() %}
{% call empty::empty_state("connected-apps-empty", "empty", "No apps can act for you.") %}{% endcall %}
{% else %}
{% call list::list_group("connected-apps-list", "Apps that can act for you") %}
{% for client in clients %}
{% call list::list_item(client.id) %}
<h3 class="kit-subtitle"><bdi>{{ client.client_name }}</bdi></h3>
<p class="kit-muted">From <bdi>{{ client.client_host }}</bdi>. Allowed {{ client.allowed_on }}, last used {{ client.last_used }}.</p>
{% if client.scopes.is_empty() %}
<p>No permissions.</p>
{% else %}
<ul>
{% for scope in client.scopes %}
<li>{{ scope.description }} (<code>{{ scope.name }}</code>)</li>
{% endfor %}
</ul>
{% endif %}
<form class="kit-inline-form" method="post" action="{{ frame.root }}/connected-agents/clients/{{ client.id }}/revoke">
<input type="hidden" name="_token" value="{{ frame.csrf }}">
<button class="sn-button" type="submit" data-sn-variant="danger">Revoke <bdi>{{ client.client_name }}</bdi></button>
</form>
{% endcall %}
{% endfor %}
{% endcall %}
{% endif %}
{% endcall %}
{% call card::card("connected-agent-accounts", "Agents", kind="section") %}
{% if agents.is_empty() %}
{% call empty::empty_state("connected-agents-empty", "empty", "No agents can act for you.") %}{% endcall %}
{% else %}
{% call list::list_group("connected-agents-list", "Agents that can act for you") %}
{% for agent in agents %}
{% call list::list_item(agent.id) %}
<h3 class="kit-subtitle">{% if let Some(provider) = agent.provider %}Agent through <bdi>{{ provider }}</bdi>{% else %}Agent{% endif %}</h3>
<p class="kit-muted">{{ agent.how }}. Registered {{ agent.registered_on }}{% if let Some(claimed) = agent.claimed_on %}, claimed {{ claimed }}{% endif %}.</p>
{% if agent.scopes.is_empty() %}
<p>No permissions.</p>
{% else %}
<ul>
{% for scope in agent.scopes %}
<li>{{ scope.description }} (<code>{{ scope.name }}</code>)</li>
{% endfor %}
</ul>
{% endif %}
<form class="kit-inline-form" method="post" action="{{ frame.root }}/connected-agents/agents/{{ agent.id }}/revoke">
<input type="hidden" name="_token" value="{{ frame.csrf }}">
<button class="sn-button" type="submit" data-sn-variant="danger">Revoke this agent</button>
</form>
{% endcall %}
{% endfor %}
{% endcall %}
{% endif %}
{% endcall %}
</div>
{% endblock %}
